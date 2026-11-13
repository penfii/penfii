//! Workspace search on OpenSearch.
//!
//! OpenSearch finds candidates. Every candidate is then checked against PostgreSQL by the shared
//! visibility service, and every displayed field is read from PostgreSQL, so a stale or wrong index
//! can lose a result but never show one the person may not see.

pub mod client;
pub mod docs;
pub mod index;
pub mod mapping;
pub mod query;

use std::collections::HashSet;
use std::sync::Arc;

use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::visibility;
use client::{OsClient, OsError};
use index::Indexer;
use query::{Invalid, Parsed};

const MARK_OPEN: char = '\u{E000}';
const MARK_CLOSE: char = '\u{E001}';
const MAX_ROUNDS: usize = 5;
const MAX_CANDIDATES: usize = 500;

#[derive(Clone)]
pub struct SearchEngine {
    pub indexer: Arc<Indexer>,
}

impl SearchEngine {
    pub fn new(pool: PgPool, client: OsClient) -> Self {
        Self {
            indexer: Arc::new(Indexer::new(pool, client)),
        }
    }

    /// Same engine with the operator's limits for Docs.
    pub fn with_limits(pool: PgPool, client: OsClient, limits: docs::BuildLimits) -> Self {
        Self {
            indexer: Arc::new(Indexer::new(pool, client).with_limits(limits)),
        }
    }
}

/// Why a search did not produce results.
#[derive(Debug)]
pub enum SearchError {
    Invalid(Invalid),
    Ambiguous {
        token: String,
        candidates: Vec<(Uuid, String)>,
    },
    Unavailable,
    Core(CoreError),
}

impl From<CoreError> for SearchError {
    fn from(error: CoreError) -> Self {
        SearchError::Core(error)
    }
}

impl From<sqlx::Error> for SearchError {
    fn from(error: sqlx::Error) -> Self {
        SearchError::Core(error.into())
    }
}

impl From<OsError> for SearchError {
    fn from(_: OsError) -> Self {
        SearchError::Unavailable
    }
}

impl From<Invalid> for SearchError {
    fn from(value: Invalid) -> Self {
        SearchError::Invalid(value)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub q: String,
    /// all | message | ticket | file | space | view | doc
    pub kind: String,
    pub state_ids: Vec<Uuid>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    pub tz: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Segment {
    pub text: String,
    pub hit: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub snippet: Vec<Segment>,
    pub created_at: Option<String>,
    pub fields: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub ready: bool,
    pub unavailable_kinds: Vec<String>,
    pub stale: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: String,
    pub normalized: Value,
    pub results: Vec<SearchResult>,
    pub next_cursor: Option<String>,
    pub status: IndexStatus,
}

pub fn kinds_for_tab(tab: &str) -> Option<Vec<&'static str>> {
    match tab {
        "" | "all" => Some(mapping::KINDS.to_vec()),
        "message" => Some(vec!["message"]),
        "ticket" => Some(vec!["ticket"]),
        "file" => Some(vec!["file"]),
        "space" => Some(vec!["space"]),
        "view" => Some(vec!["view"]),
        "doc" => Some(vec!["doc"]),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Resolving the names a person typed into ids they are allowed to mean
// ---------------------------------------------------------------------------------------------

#[derive(Default, Debug)]
struct Resolved {
    channel_ids: Option<Vec<Uuid>>,
    from_ids: Option<Vec<Uuid>>,
    assignee_ids: Option<Vec<Uuid>>,
    assignee_none: bool,
    project_ids: Option<Vec<Uuid>>,
    team_ids: Option<Vec<Uuid>>,
    type_ids: Option<Vec<Uuid>>,
    label_ids: Option<Vec<Uuid>>,
    categories: Option<Vec<&'static str>>,
    before: Option<String>,
    after: Option<String>,
    has_file: bool,
    mentions_me: bool,
}

fn parse_uuid(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value.trim()).ok()
}

async fn resolve_users(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: Uuid,
    key: &str,
    values: &[&str],
) -> Result<(Vec<Uuid>, bool), SearchError> {
    let mut ids = Vec::new();
    let mut none = false;
    for value in values {
        if value.eq_ignore_ascii_case("me") {
            ids.push(actor);
            continue;
        }
        if key == "assignee" && value.eq_ignore_ascii_case("none") {
            none = true;
            continue;
        }
        let rows = sqlx::query(
            "SELECT u.id, u.display_name FROM penfii.users u
             JOIN penfii.workspace_members wm ON wm.user_id=u.id AND wm.workspace_id=$1 AND wm.state='ACTIVE'
             WHERE u.id = $3 OR lower(u.username_key) = lower($2) OR lower(u.display_name) = lower($2)",
        )
        .bind(workspace_id)
        .bind(value)
        .bind(parse_uuid(value).unwrap_or(Uuid::nil()))
        .fetch_all(&mut *conn)
        .await?;
        match rows.len() {
            0 => {
                return Err(Invalid::new(
                    "unknown_filter_value",
                    format!("{key}:{value}"),
                    "no member matches this name",
                )
                .into());
            }
            1 => ids.push(rows[0].try_get("id")?),
            _ => {
                return Err(SearchError::Ambiguous {
                    token: format!("{key}:{value}"),
                    candidates: rows
                        .iter()
                        .map(|r| Ok((r.try_get("id")?, r.try_get("display_name")?)))
                        .collect::<Result<_, sqlx::Error>>()?,
                });
            }
        }
    }
    Ok((ids, none))
}

async fn resolve_projects(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    values: &[&str],
) -> Result<Vec<Uuid>, SearchError> {
    let mut ids = Vec::new();
    for value in values {
        let rows = sqlx::query(
            "SELECT id, name FROM penfii.projects
             WHERE workspace_id=$1 AND (id=$3 OR lower(ticket_prefix)=lower($2) OR lower(name)=lower($2))",
        )
        .bind(workspace_id)
        .bind(value)
        .bind(parse_uuid(value).unwrap_or(Uuid::nil()))
        .fetch_all(&mut *conn)
        .await?;
        match rows.len() {
            0 => {
                return Err(Invalid::new(
                    "unknown_filter_value",
                    format!("project:{value}"),
                    "no Project matches",
                )
                .into());
            }
            1 => ids.push(rows[0].try_get("id")?),
            _ => {
                return Err(SearchError::Ambiguous {
                    token: format!("project:{value}"),
                    candidates: rows
                        .iter()
                        .map(|r| Ok((r.try_get("id")?, r.try_get("name")?)))
                        .collect::<Result<_, sqlx::Error>>()?,
                });
            }
        }
    }
    Ok(ids)
}

async fn resolve(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: Uuid,
    parsed: &Parsed,
    tz: &str,
) -> Result<Resolved, SearchError> {
    let mut out = Resolved::default();
    let channels = parsed.values("in");
    if !channels.is_empty() {
        let mut ids = Vec::new();
        for value in channels {
            let name = value.trim_start_matches('#');
            let rows = sqlx::query(
                "SELECT ch.id, ch.name FROM penfii.channels ch JOIN penfii.conversations c ON c.id=ch.id
                 WHERE c.workspace_id=$1 AND (ch.id=$3 OR lower(ch.name)=lower($2))",
            )
            .bind(workspace_id)
            .bind(name)
            .bind(parse_uuid(name).unwrap_or(Uuid::nil()))
            .fetch_all(&mut *conn)
            .await?;
            let candidate_ids: Vec<Uuid> = rows
                .iter()
                .map(|r| r.try_get("id"))
                .collect::<Result<_, sqlx::Error>>()?;
            let readable = visibility::channels(conn, actor, &candidate_ids).await?;
            let mut matches: Vec<&sqlx::postgres::PgRow> = Vec::new();
            for r in &rows {
                if readable.contains(&r.try_get::<Uuid, _>("id")?) {
                    matches.push(r);
                }
            }
            match matches.len() {
                0 => {
                    return Err(Invalid::new(
                        "unknown_filter_value",
                        format!("in:{value}"),
                        "no channel you can see matches",
                    )
                    .into());
                }
                1 => ids.push(matches[0].try_get("id")?),
                _ => {
                    return Err(SearchError::Ambiguous {
                        token: format!("in:{value}"),
                        candidates: matches
                            .iter()
                            .map(|r| -> Result<_, sqlx::Error> {
                                Ok((r.try_get("id")?, r.try_get("name")?))
                            })
                            .collect::<Result<Vec<_>, sqlx::Error>>()?,
                    });
                }
            }
        }
        out.channel_ids = Some(ids);
    }
    let from = parsed.values("from");
    if !from.is_empty() {
        out.from_ids = Some(
            resolve_users(conn, workspace_id, actor, "from", &from)
                .await?
                .0,
        );
    }
    let assignee = parsed.values("assignee");
    if !assignee.is_empty() {
        let (ids, none) = resolve_users(conn, workspace_id, actor, "assignee", &assignee).await?;
        out.assignee_ids = Some(ids);
        out.assignee_none = none;
    }
    let projects = parsed.values("project");
    if !projects.is_empty() {
        out.project_ids = Some(resolve_projects(conn, workspace_id, &projects).await?);
    }
    let teams = parsed.values("team");
    if !teams.is_empty() {
        let mut ids = Vec::new();
        for value in teams {
            let rows = sqlx::query(
                "SELECT id, name FROM penfii.teams WHERE workspace_id=$1 AND (id=$3 OR lower(name)=lower($2))",
            )
            .bind(workspace_id)
            .bind(value)
            .bind(parse_uuid(value).unwrap_or(Uuid::nil()))
            .fetch_all(&mut *conn)
            .await?;
            match rows.len() {
                0 => {
                    return Err(Invalid::new(
                        "unknown_filter_value",
                        format!("team:{value}"),
                        "no Team matches",
                    )
                    .into());
                }
                1 => ids.push(rows[0].try_get("id")?),
                _ => {
                    return Err(SearchError::Ambiguous {
                        token: format!("team:{value}"),
                        candidates: rows
                            .iter()
                            .map(|r| -> Result<_, sqlx::Error> {
                                Ok((r.try_get("id")?, r.try_get("name")?))
                            })
                            .collect::<Result<Vec<_>, sqlx::Error>>()?,
                    });
                }
            }
        }
        out.team_ids = Some(ids);
    }
    let types = parsed.values("type");
    if !types.is_empty() {
        let mut ids = Vec::new();
        for value in types {
            let rows: Vec<Uuid> = sqlx::query_scalar(
                "SELECT t.id FROM penfii.ticket_types t JOIN penfii.projects p ON p.id=t.project_id
                 WHERE p.workspace_id=$1 AND (t.id=$3 OR lower(t.name)=lower($2))",
            )
            .bind(workspace_id)
            .bind(value)
            .bind(parse_uuid(value).unwrap_or(Uuid::nil()))
            .fetch_all(&mut *conn)
            .await?;
            if rows.is_empty() {
                return Err(Invalid::new(
                    "unknown_filter_value",
                    format!("type:{value}"),
                    "no Ticket type matches",
                )
                .into());
            }
            ids.extend(rows);
        }
        out.type_ids = Some(ids);
    }
    let labels = parsed.values("label");
    if !labels.is_empty() {
        let mut ids = Vec::new();
        for value in labels {
            let rows: Vec<Uuid> = sqlx::query_scalar(
                "SELECT l.id FROM penfii.labels l JOIN penfii.projects p ON p.id=l.project_id
                 WHERE p.workspace_id=$1 AND (l.id=$3 OR lower(l.name)=lower($2))",
            )
            .bind(workspace_id)
            .bind(value)
            .bind(parse_uuid(value).unwrap_or(Uuid::nil()))
            .fetch_all(&mut *conn)
            .await?;
            if rows.is_empty() {
                return Err(Invalid::new(
                    "unknown_filter_value",
                    format!("label:{value}"),
                    "no label matches",
                )
                .into());
            }
            ids.extend(rows);
        }
        out.label_ids = Some(ids);
    }
    let statuses = parsed.values("status");
    if !statuses.is_empty() {
        out.categories = Some(
            statuses
                .iter()
                .filter_map(|s| query::status_category(s))
                .collect(),
        );
    }
    for (key, slot) in [("before", &mut out.before), ("after", &mut out.after)] {
        if let Some(day) = parsed.values(key).first() {
            // The day boundary is midnight in the requester's time zone.
            let at: Option<chrono::DateTime<chrono::Utc>> =
                sqlx::query_scalar("SELECT (($1::date)::timestamp AT TIME ZONE $2)")
                    .bind(day)
                    .bind(tz)
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(|_| {
                        Invalid::new("invalid_value", format!("tz={tz}"), "unknown time zone")
                    })?;
            *slot = at.map(|t| t.to_rfc3339());
        }
    }
    out.has_file = !parsed.values("has").is_empty();
    out.mentions_me = !parsed.values("mentions").is_empty();
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Building the OpenSearch query
// ---------------------------------------------------------------------------------------------

fn text_fields(kind: &str) -> Vec<&'static str> {
    match kind {
        "message" => vec!["text^2", "text.ngram", "text.raw^2"],
        "ticket" => vec![
            "title^6",
            "title.ngram^2",
            "title.raw^4",
            "description^2",
            "description.ngram",
            "comments",
            "comments.ngram^0.5",
            "key.raw^8",
        ],
        "space" => vec![
            "name^6",
            "name.ngram^2",
            "name.raw^4",
            "description",
            "key.raw^8",
        ],
        "file" => vec!["filename^6", "filename.ngram^2", "filename.raw^6"],
        "view" => vec!["name^6", "name.ngram^2", "name.raw^4"],
        "doc" => vec![
            "title^6",
            "title.ngram^2",
            "title.raw^4",
            "body^2",
            "body.ngram",
            "body.raw",
            "comments",
            "comments.ngram^0.5",
        ],
        _ => vec![],
    }
}

fn phrase_fields(kind: &str) -> Vec<&'static str> {
    match kind {
        "message" => vec!["text"],
        "ticket" => vec!["title^3", "description", "comments"],
        "space" => vec!["name^3", "description"],
        "file" => vec!["filename"],
        "view" => vec!["name"],
        "doc" => vec!["title^3", "body", "comments"],
        _ => vec![],
    }
}

fn terms(field: &str, values: &[Uuid]) -> Value {
    json!({"terms": {field: values}})
}

fn kind_query(
    kind: &str,
    workspace_id: Uuid,
    actor: Uuid,
    parsed: &Parsed,
    r: &Resolved,
    state_ids: &[Uuid],
) -> Option<Value> {
    let mut filter = vec![
        json!({"term": {"workspaceId": workspace_id}}),
        json!({"term": {"kind": kind}}),
    ];
    let mut must = Vec::new();
    // `project:` / `team:` mean different fields in different kinds.
    match kind {
        "ticket" => {
            if let Some(p) = &r.project_ids {
                filter.push(terms("projectId", p));
            }
            if let Some(c) = &r.channel_ids {
                filter.push(terms("channelId", c));
            }
            if let Some(f) = &r.from_ids {
                filter.push(terms("creatorId", f));
            }
            if let Some(a) = &r.assignee_ids {
                if r.assignee_none {
                    filter.push(json!({"bool": {"should": [terms("assigneeId", a), {"bool": {"must_not": {"exists": {"field": "assigneeId"}}}}], "minimum_should_match": 1}}));
                } else {
                    filter.push(terms("assigneeId", a));
                }
            } else if r.assignee_none {
                filter.push(json!({"bool": {"must_not": {"exists": {"field": "assigneeId"}}}}));
            }
            if let Some(t) = &r.type_ids {
                filter.push(terms("typeId", t));
            }
            if let Some(l) = &r.label_ids {
                filter.push(terms("labelIds", l));
            }
            if let Some(c) = &r.categories {
                filter.push(json!({"terms": {"statusCategory": c}}));
            }
            if !state_ids.is_empty() {
                filter.push(terms("statusId", state_ids));
            }
        }
        "message" => {
            if let Some(c) = &r.channel_ids {
                filter.push(terms("conversationId", c));
            }
            if let Some(f) = &r.from_ids {
                filter.push(terms("authorId", f));
            }
            if r.has_file {
                filter.push(json!({"term": {"hasFile": true}}));
            }
            if r.mentions_me {
                filter.push(json!({"term": {"mentionedUserIds": actor}}));
            }
            // A Project's messages are the messages of its channels, resolved by the caller
            // into `channel_ids` when `project:` is used with messages.
        }
        "space" => {
            if let Some(p) = &r.project_ids {
                filter.push(json!({"bool": {"should": [terms("projectId", p), terms("id", p)], "minimum_should_match": 1}}));
            }
            if let Some(t) = &r.team_ids {
                filter.push(json!({"bool": {"should": [terms("teamId", t), terms("id", t)], "minimum_should_match": 1}}));
            }
            filter.push(json!({"term": {"archived": false}}));
        }
        "file" => {
            if let Some(c) = &r.channel_ids {
                filter.push(terms("conversationIds", c));
            }
            if let Some(f) = &r.from_ids {
                filter.push(terms("uploaderId", f));
            }
        }
        "view" => {
            if let Some(p) = &r.project_ids {
                filter.push(terms("projectId", p));
            }
            if let Some(t) = &r.team_ids {
                filter.push(terms("teamId", t));
            }
            if let Some(f) = &r.from_ids {
                filter.push(terms("ownerId", f));
            }
        }
        "doc" => {
            if let Some(p) = &r.project_ids {
                filter.push(terms("projectId", p));
            }
            if let Some(f) = &r.from_ids {
                filter.push(terms("creatorId", f));
            }
        }
        _ => return None,
    }
    if r.before.is_some() || r.after.is_some() {
        let mut range = serde_json::Map::new();
        if let Some(b) = &r.before {
            range.insert("lt".into(), json!(b));
        }
        if let Some(a) = &r.after {
            range.insert("gte".into(), json!(a));
        }
        // Docs change in place: their date is when they last changed.
        let field = if kind == "doc" {
            "updatedAt"
        } else {
            "createdAt"
        };
        filter.push(json!({"range": {field: range}}));
    }
    for word in &parsed.keywords {
        let text = json!({"multi_match": {"query": word, "fields": text_fields(kind), "type": "best_fields", "operator": "and"}});
        if kind == "ticket" && query::looks_like_ticket_key(word) {
            must.push(json!({"bool": {"should": [text, {"term": {"key": {"value": word.to_lowercase(), "boost": 50}}}], "minimum_should_match": 1}}));
        } else {
            must.push(text);
        }
    }
    for phrase in &parsed.phrases {
        must.push(json!({"multi_match": {"query": phrase, "fields": phrase_fields(kind), "type": "phrase"}}));
    }
    Some(json!({"bool": {"filter": filter, "must": must}}))
}

fn hash_of(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(value.to_string().as_bytes())[..8])
}

fn encode_cursor(hash: &str, sort: &Value) -> String {
    hex::encode(json!({"h": hash, "s": sort}).to_string())
}

fn decode_cursor(cursor: &str, hash: &str) -> Result<Value, SearchError> {
    let bad = || {
        Invalid::new(
            "invalid_cursor",
            "cursor",
            "this cursor does not belong to this query",
        )
    };
    let bytes = hex::decode(cursor).map_err(|_| bad())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    if value["h"].as_str() != Some(hash) {
        return Err(bad().into());
    }
    Ok(value["s"].clone())
}

fn segments(fragment: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut hit = false;
    let mut current = String::new();
    for ch in fragment.chars() {
        if ch == MARK_OPEN || ch == MARK_CLOSE {
            if !current.is_empty() {
                out.push(Segment {
                    text: std::mem::take(&mut current),
                    hit,
                });
            }
            hit = ch == MARK_OPEN;
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push(Segment { text: current, hit });
    }
    out
}

fn excerpt(text: &str, limit: usize) -> Vec<Segment> {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = flat.chars().take(limit).collect();
    if cut.is_empty() {
        return Vec::new();
    }
    vec![Segment {
        text: if flat.chars().count() > limit {
            format!("{cut}…")
        } else {
            cut
        },
        hit: false,
    }]
}

struct Candidate {
    kind: String,
    id: Uuid,
    space_kind: Option<String>,
    source_version: i64,
    sort: Value,
    highlight: Option<String>,
}

fn candidates_from(response: &Value) -> Vec<Candidate> {
    response["hits"]["hits"]
        .as_array()
        .map(|hits| {
            hits.iter()
                .filter_map(|hit| {
                    let source = &hit["_source"];
                    let id = Uuid::parse_str(source["id"].as_str()?).ok()?;
                    let highlight = hit["highlight"]
                        .as_object()
                        .and_then(|h| h.values().next())
                        .and_then(|v| v.get(0))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    Some(Candidate {
                        kind: source["kind"].as_str()?.to_owned(),
                        id,
                        space_kind: source["spaceKind"].as_str().map(str::to_owned),
                        source_version: source["sourceVersion"].as_i64().unwrap_or(0),
                        sort: hit["sort"].clone(),
                        highlight,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------------------------

/// What a search request asks for, once it has been validated. No database access and no
/// permission decision: the caller still checks membership and every result is still re-checked
/// for visibility after the index answers.
struct SearchPlan {
    parsed: Parsed,
    /// The result kinds to search: the tab's kinds that support every filter given.
    requested: Vec<&'static str>,
    limit: usize,
    /// The query as it is echoed back, and hashed into the paging cursor.
    normalized: Value,
    hash: String,
    /// The `search_after` position decoded from the cursor.
    after: Option<Value>,
}

fn plan_search(params: &SearchParams) -> Result<SearchPlan, SearchError> {
    let parsed = query::parse(&params.q)?;
    let tab_kinds = kinds_for_tab(&params.kind).ok_or_else(|| {
        Invalid::new(
            "invalid_value",
            format!("kind={}", params.kind),
            "unknown result type",
        )
    })?;
    let limit = params.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(Invalid::new("invalid_value", "limit", "limit is 1 to 50").into());
    }
    let limit = limit as usize;
    // A kind that lacks a filtered field is left out.
    let requested: Vec<&'static str> = tab_kinds
        .into_iter()
        .filter(|kind| {
            parsed
                .filters
                .iter()
                .all(|(key, _)| query::kind_supports(kind, key))
                && (params.state_ids.is_empty() || *kind == "ticket")
        })
        .collect();
    let normalized = {
        let mut n = parsed.normalized();
        n["stateIds"] = json!(params.state_ids);
        n["kind"] = json!(if params.kind.is_empty() {
            "all"
        } else {
            &params.kind
        });
        n
    };
    let hash = hash_of(&normalized);
    let after = match &params.cursor {
        Some(cursor) => Some(decode_cursor(cursor, &hash)?),
        None => None,
    };

    Ok(SearchPlan {
        parsed,
        requested,
        limit,
        normalized,
        hash,
        after,
    })
}

/// `project:` and `team:` narrow what is searched: messages through the channels of those Projects,
/// Tickets and Spaces through the Projects of those Teams. This only narrows the query; what the
/// actor may see is decided by the query's own filters and the visibility re-check after it.
async fn narrow_by_project_and_team(
    conn: &mut PgConnection,
    requested: &[&'static str],
    resolved: &mut Resolved,
) -> Result<(), SearchError> {
    // `project:` and `team:` narrow messages through the channels of those Projects.
    if requested.contains(&"message")
        && (resolved.project_ids.is_some() || resolved.team_ids.is_some())
    {
        let channels: Vec<Uuid> = sqlx::query_scalar(
            "SELECT ch.id FROM penfii.channels ch
             WHERE ($1::uuid[] IS NOT NULL AND ch.project_id = ANY($1))
                OR ($2::uuid[] IS NOT NULL AND (ch.team_id = ANY($2)
                    OR ch.project_id IN (SELECT project_id FROM penfii.project_teams WHERE team_id = ANY($2))))",
        )
        .bind(resolved.project_ids.as_deref())
        .bind(resolved.team_ids.as_deref())
        .fetch_all(&mut *conn)
        .await?;
        resolved.channel_ids = Some(match resolved.channel_ids.take() {
            Some(existing) => existing
                .into_iter()
                .filter(|c| channels.contains(c))
                .collect(),
            None => channels,
        });
    }
    // `team:` for Tickets, Spaces and Views means the Projects of that Team.
    if let Some(teams) = resolved.team_ids.clone() {
        let projects: Vec<Uuid> = sqlx::query_scalar(
            "SELECT project_id FROM penfii.project_teams WHERE team_id = ANY($1)",
        )
        .bind(&teams)
        .fetch_all(&mut *conn)
        .await?;
        if requested.contains(&"ticket") || requested.contains(&"doc") {
            resolved.project_ids = Some(match resolved.project_ids.take() {
                Some(existing) => existing
                    .into_iter()
                    .filter(|p| projects.contains(p))
                    .collect(),
                None => projects,
            });
        }
    }
    Ok(())
}

pub async fn search(
    pool: &PgPool,
    engine: Option<&SearchEngine>,
    actor: Uuid,
    workspace_id: Uuid,
    params: SearchParams,
) -> Result<SearchResponse, SearchError> {
    // Everything in the request that can be refused before the database is touched.
    let SearchPlan {
        parsed,
        requested,
        limit,
        normalized,
        hash,
        after,
    } = plan_search(&params)?;

    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let tz = params.tz.clone().unwrap_or_else(|| "UTC".into());
    let mut resolved = resolve(&mut conn, workspace_id, actor, &parsed, &tz).await?;
    narrow_by_project_and_team(&mut conn, &requested, &mut resolved).await?;
    drop(conn);

    // The directory kinds (people) are served elsewhere; this function is the index path.
    let Some(engine) = engine else {
        return Err(SearchError::Unavailable);
    };
    let states = engine.indexer.states().await?;
    let mut unavailable = Vec::new();
    let mut aliases = Vec::new();
    let mut should = Vec::new();
    for kind in &requested {
        let ready = states
            .iter()
            .any(|s| s.kind == *kind && s.live_index.is_some());
        if !ready {
            unavailable.push((*kind).to_owned());
            continue;
        }
        aliases.push(engine.indexer.client.alias(kind));
        if let Some(q) = kind_query(
            kind,
            workspace_id,
            actor,
            &parsed,
            &resolved,
            &params.state_ids,
        ) {
            should.push(q);
        }
    }
    let status_of = |unavailable: Vec<String>, stale: bool| IndexStatus {
        ready: unavailable.is_empty(),
        unavailable_kinds: unavailable,
        stale,
    };
    if should.is_empty() {
        if !unavailable.is_empty() && requested.len() == unavailable.len() && !requested.is_empty()
        {
            // Everything asked for is still being indexed.
        }
        return Ok(SearchResponse {
            query: params.q,
            normalized,
            results: Vec::new(),
            next_cursor: None,
            status: status_of(unavailable, false),
        });
    }
    // Nothing typed at all: no results (never "everything").
    if parsed.is_empty() && params.state_ids.is_empty() {
        return Ok(SearchResponse {
            query: params.q,
            normalized,
            results: Vec::new(),
            next_cursor: None,
            status: status_of(unavailable, false),
        });
    }

    let has_text = !parsed.keywords.is_empty() || !parsed.phrases.is_empty();
    let sort = if has_text {
        json!([{"_score": "desc"}, {"createdAt": "desc"}, {"id": "asc"}])
    } else {
        json!([{"createdAt": "desc"}, {"id": "asc"}])
    };
    let size = (limit * 3 + 5).min(100);
    let mut cursor_sort = after;
    let mut collected: Vec<(Candidate, Option<String>)> = Vec::new();
    let mut examined = 0usize;
    let mut exhausted = false;
    let mut stopped_at: Option<Value> = None;

    for _ in 0..MAX_ROUNDS {
        let mut body = json!({
            "size": size,
            "query": {"bool": {"should": should, "minimum_should_match": 1}},
            "sort": sort,
            "_source": ["id", "kind", "spaceKind", "sourceVersion"],
            "highlight": {
                "pre_tags": [MARK_OPEN.to_string()], "post_tags": [MARK_CLOSE.to_string()],
                "fragment_size": 140, "number_of_fragments": 1,
                "fields": {"text": {}, "title": {}, "description": {}, "comments": {}, "name": {}, "filename": {}, "body": {}, "databaseName": {}},
                "require_field_match": true
            }
        });
        if let Some(s) = &cursor_sort {
            body["search_after"] = s.clone();
        }
        let path = format!("/{}/_search?ignore_unavailable=true", aliases.join(","));
        let (status, response) = engine.indexer.client.search_request(&path, &body).await?;
        if status >= 400 {
            return Err(SearchError::Unavailable);
        }
        let batch = candidates_from(&response);
        if batch.is_empty() {
            exhausted = true;
            break;
        }
        let full = batch.len() >= size;
        let last_sort = batch.last().map(|c| c.sort.clone());
        examined += batch.len();
        let visible = visible_candidates(pool, actor, &batch).await?;
        let mut stop = false;
        for candidate in batch {
            if !visible.contains(&(candidate.kind.clone(), candidate.id)) {
                continue;
            }
            let sort_here = candidate.sort.clone();
            let highlight = candidate.highlight.clone();
            collected.push((candidate, highlight));
            if collected.len() == limit {
                stopped_at = Some(sort_here);
                stop = true;
                break;
            }
        }
        if stop {
            break;
        }
        cursor_sort = last_sort;
        if !full {
            exhausted = true;
            break;
        }
        if examined >= MAX_CANDIDATES {
            break;
        }
    }
    let next_cursor = if let Some(sort) = stopped_at {
        Some(encode_cursor(&hash, &sort))
    } else if !exhausted {
        cursor_sort.map(|sort| encode_cursor(&hash, &sort))
    } else {
        None
    };
    let results = display(pool, actor, collected).await?;
    let stale = {
        let oldest: Option<i64> = sqlx::query_scalar(
            "SELECT extract(epoch FROM now() - min(updated_at))::bigint FROM penfii.consumer_receipts
             WHERE consumer='search' AND state IN ('PENDING','RUNNING')",
        )
        .fetch_one(pool)
        .await?;
        oldest.is_some_and(|age| age > 60)
    };
    Ok(SearchResponse {
        query: params.q,
        normalized,
        results,
        next_cursor,
        status: status_of(unavailable, stale),
    })
}

/// The shared visibility service decides, per kind, which candidates may be shown.
async fn visible_candidates(
    pool: &PgPool,
    actor: Uuid,
    batch: &[Candidate],
) -> Result<HashSet<(String, Uuid)>, SearchError> {
    let mut conn = pool.acquire().await?;
    let ids_of = |kind: &str, space: Option<&str>| -> Vec<Uuid> {
        batch
            .iter()
            .filter(|c| c.kind == kind && (space.is_none() || c.space_kind.as_deref() == space))
            .map(|c| c.id)
            .collect()
    };
    let mut visible: HashSet<(String, Uuid)> = HashSet::new();
    for id in visibility::messages(&mut conn, actor, &ids_of("message", None)).await? {
        visible.insert(("message".into(), id));
    }
    for id in visibility::tickets(&mut conn, actor, &ids_of("ticket", None)).await? {
        visible.insert(("ticket".into(), id));
    }
    for id in visibility::files(&mut conn, actor, &ids_of("file", None)).await? {
        visible.insert(("file".into(), id));
    }
    for id in visibility::views(&mut conn, actor, &ids_of("view", None)).await? {
        visible.insert(("view".into(), id));
    }
    for id in visibility::docs(&mut conn, actor, &ids_of("doc", None)).await? {
        visible.insert(("doc".into(), id));
    }
    for id in visibility::channels(&mut conn, actor, &ids_of("space", Some("CHANNEL"))).await? {
        visible.insert(("space".into(), id));
    }
    for id in visibility::projects(&mut conn, actor, &ids_of("space", Some("PROJECT"))).await? {
        visible.insert(("space".into(), id));
    }
    for id in visibility::teams(&mut conn, actor, &ids_of("space", Some("TEAM"))).await? {
        visible.insert(("space".into(), id));
    }
    Ok(visible)
}

/// Reads what is shown from PostgreSQL. Nothing shown comes from the index except the highlight,
/// and the highlight is dropped when the index is behind the current version.
async fn display(
    pool: &PgPool,
    actor: Uuid,
    collected: Vec<(Candidate, Option<String>)>,
) -> Result<Vec<SearchResult>, SearchError> {
    let mut conn = pool.acquire().await?;
    let mut out = Vec::new();
    for (candidate, highlight) in collected {
        let snippet_of = |current_version: i64, text: &str| -> Vec<Segment> {
            match &highlight {
                Some(fragment) if candidate.source_version >= current_version => segments(fragment),
                _ => excerpt(text, 160),
            }
        };
        let result = match candidate.kind.as_str() {
            "message" => {
                let Some(row) = sqlx::query(
                    "SELECT m.conversation_id, m.thread_root_id, m.version, m.body, m.created_at, u.display_name,
                            ch.name AS channel_name, c.kind AS conv_kind
                     FROM penfii.messages m JOIN penfii.users u ON u.id=m.author_id
                     JOIN penfii.conversations c ON c.id=m.conversation_id
                     LEFT JOIN penfii.channels ch ON ch.id=c.id WHERE m.id=$1",
                )
                .bind(candidate.id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    continue;
                };
                let body: Value = row.try_get("body")?;
                let text = crate::work_history::description_text(&body);
                let version: i64 = row.try_get("version")?;
                let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
                SearchResult {
                    kind: "message".into(),
                    id: candidate.id,
                    title: row.try_get("display_name")?,
                    subtitle: row.try_get::<Option<String>, _>("channel_name")?,
                    snippet: snippet_of(version, &text),
                    created_at: Some(created.to_rfc3339()),
                    fields: json!({
                        "conversationId": row.try_get::<Uuid, _>("conversation_id")?,
                        "threadRootId": row.try_get::<Option<Uuid>, _>("thread_root_id")?,
                        "conversationKind": row.try_get::<String, _>("conv_kind")?,
                    }),
                }
            }
            "ticket" => {
                let Some(row) = sqlx::query(
                    "SELECT t.public_key, t.title, t.description, t.revision_no, t.project_id, t.created_at, p.name AS project_name,
                            ws.name AS status_name, ws.category, au.display_name AS assignee_name
                     FROM penfii.tickets t JOIN penfii.projects p ON p.id=t.project_id
                     JOIN penfii.workflow_statuses ws ON ws.id=t.status_id
                     LEFT JOIN penfii.users au ON au.id=t.assignee_id WHERE t.id=$1",
                )
                .bind(candidate.id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    continue;
                };
                let description =
                    crate::work_history::description_text(&row.try_get::<Value, _>("description")?);
                let version: i64 = row.try_get("revision_no")?;
                let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
                SearchResult {
                    kind: "ticket".into(),
                    id: candidate.id,
                    title: row.try_get("title")?,
                    subtitle: Some(format!(
                        "{} · {}",
                        row.try_get::<String, _>("public_key")?,
                        row.try_get::<String, _>("project_name")?
                    )),
                    snippet: snippet_of(version, &description),
                    created_at: Some(created.to_rfc3339()),
                    fields: json!({
                        "key": row.try_get::<String, _>("public_key")?,
                        "projectId": row.try_get::<Uuid, _>("project_id")?,
                        "statusName": row.try_get::<String, _>("status_name")?,
                        "statusCategory": row.try_get::<String, _>("category")?,
                        "assigneeName": row.try_get::<Option<String>, _>("assignee_name")?,
                    }),
                }
            }
            "space" => {
                let kind = candidate.space_kind.clone().unwrap_or_default();
                let row = match kind.as_str() {
                    "PROJECT" => sqlx::query("SELECT name, NULL::text AS description, id AS project_id, NULL::uuid AS team_id FROM penfii.projects WHERE id=$1"),
                    "TEAM" => sqlx::query("SELECT name, NULL::text AS description, NULL::uuid AS project_id, id AS team_id FROM penfii.teams WHERE id=$1"),
                    _ => sqlx::query("SELECT name, description, project_id, team_id FROM penfii.channels WHERE id=$1"),
                }
                .bind(candidate.id)
                .fetch_optional(&mut *conn)
                .await?;
                let Some(row) = row else { continue };
                let description: Option<String> = row.try_get("description")?;
                SearchResult {
                    kind: "space".into(),
                    id: candidate.id,
                    title: row.try_get("name")?,
                    subtitle: Some(kind.to_lowercase()),
                    snippet: excerpt(description.as_deref().unwrap_or(""), 160),
                    created_at: None,
                    fields: json!({
                        "spaceKind": kind,
                        "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
                        "teamId": row.try_get::<Option<Uuid>, _>("team_id")?,
                    }),
                }
            }
            "file" => {
                let Some(row) = sqlx::query(
                    "SELECT f.filename, f.byte_size, f.detected_mime, f.created_at, u.display_name FROM penfii.files f JOIN penfii.users u ON u.id=f.created_by WHERE f.id=$1",
                )
                .bind(candidate.id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    continue;
                };
                // Link to a message of this file that the person can read.
                let attachments = sqlx::query(
                    "SELECT m.id AS message_id, m.conversation_id FROM penfii.message_attachments a
                     JOIN penfii.messages m ON m.id=a.message_id AND m.deleted_at IS NULL WHERE a.file_id=$1 ORDER BY m.created_at",
                )
                .bind(candidate.id)
                .fetch_all(&mut *conn)
                .await?;
                let mut link = None;
                for attachment in &attachments {
                    let conversation: Uuid = attachment.try_get("conversation_id")?;
                    if access::load_conversation(&mut conn, actor, conversation)
                        .await?
                        .is_some_and(|a| a.can_read())
                    {
                        link = Some((attachment.try_get::<Uuid, _>("message_id")?, conversation));
                        break;
                    }
                }
                // Otherwise the live Ticket it is attached to (the filter has already checked access).
                let ticket: Option<(Uuid, Uuid)> = if link.is_none() {
                    sqlx::query_as(
                        "SELECT a.ticket_id, t.project_id FROM penfii.ticket_attachments a
                         JOIN penfii.tickets t ON t.id=a.ticket_id AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
                         WHERE a.file_id=$1",
                    )
                    .bind(candidate.id)
                    .fetch_optional(&mut *conn)
                    .await?
                } else {
                    None
                };
                let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
                SearchResult {
                    kind: "file".into(),
                    id: candidate.id,
                    title: row.try_get("filename")?,
                    subtitle: Some(row.try_get::<String, _>("display_name")?),
                    snippet: Vec::new(),
                    created_at: Some(created.to_rfc3339()),
                    fields: json!({
                        "byteSize": row.try_get::<i64, _>("byte_size")?,
                        "mime": row.try_get::<Option<String>, _>("detected_mime")?,
                        "messageId": link.map(|l| l.0),
                        "conversationId": link.map(|l| l.1),
                        "ticketId": ticket.map(|t| t.0),
                        "projectId": ticket.map(|t| t.1),
                    }),
                }
            }
            "doc" => {
                let Some(row) = sqlx::query(
                    "SELECT d.title, d.content, d.revision_no, d.project_id, d.parent_id, d.created_at, d.updated_at, u.display_name
                     FROM penfii.doc_pages d JOIN penfii.users u ON u.id = d.created_by
                     WHERE d.id = $1 AND d.deleted_at IS NULL",
                )
                .bind(candidate.id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    continue;
                };
                let content: Value = row.try_get("content")?;
                let (body, _) = crate::docs::content::plain_text(&content, 4096);
                let version: i64 = row.try_get("revision_no")?;
                let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
                let updated: chrono::DateTime<chrono::Utc> = row.try_get("updated_at")?;
                SearchResult {
                    kind: "doc".into(),
                    id: candidate.id,
                    title: row.try_get("title")?,
                    subtitle: Some(row.try_get::<String, _>("display_name")?),
                    snippet: snippet_of(version, &body),
                    created_at: Some(created.to_rfc3339()),
                    fields: json!({
                        "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
                        "parentId": row.try_get::<Option<Uuid>, _>("parent_id")?,
                        "updatedAt": updated.to_rfc3339(),
                        "authorName": row.try_get::<String, _>("display_name")?,
                    }),
                }
            }
            "view" => {
                let Some(row) = sqlx::query(
                    "SELECT name, scope, target_kind, project_id, team_id, created_at FROM penfii.project_views WHERE id=$1 AND deleted_at IS NULL",
                )
                .bind(candidate.id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    continue;
                };
                let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
                SearchResult {
                    kind: "view".into(),
                    id: candidate.id,
                    title: row.try_get("name")?,
                    subtitle: Some(row.try_get::<String, _>("scope")?),
                    snippet: Vec::new(),
                    created_at: Some(created.to_rfc3339()),
                    fields: json!({
                        "targetKind": row.try_get::<String, _>("target_kind")?,
                        "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
                        "teamId": row.try_get::<Option<Uuid>, _>("team_id")?,
                    }),
                }
            }
            _ => continue,
        };
        out.push(result);
    }
    Ok(out)
}

/// Used by the admin CLI and tests: waits for the index to show the latest writes.
pub async fn refresh_all(engine: &SearchEngine) -> Result<(), CoreError> {
    let states = engine.indexer.states().await?;
    for state in states {
        for index in state.live_index.iter().chain(state.building_index.iter()) {
            engine
                .indexer
                .client
                .admin(Method::POST, &format!("/{index}/_refresh"), None)
                .await
                .map_err(index::map_os)?;
        }
    }
    Ok(())
}

pub fn consumer(engine: &SearchEngine) -> Arc<index::SearchConsumer> {
    Arc::new(index::SearchConsumer {
        indexer: engine.indexer.clone(),
    })
}

/// Keeps indices created and built while the process runs. A failure is logged by code and tried
/// again; it never stops the process or any command.
pub fn spawn_maintenance(engine: SearchEngine) -> tokio::task::JoinHandle<()> {
    crate::lifecycle::spawn_worker("search maintenance", async move {
        while !crate::lifecycle::stopping() {
            let wait = match crate::lifecycle::guarded(
                "search maintenance",
                engine.indexer.backfill_all(),
            )
            .await
            {
                Ok(done) if done.is_empty() => 30,
                Ok(_) => 1,
                Err(error) => {
                    tracing::warn!(?error, "search index maintenance failed");
                    15
                }
            };
            crate::lifecycle::pause(std::time::Duration::from_secs(wait)).await;
        }
    })
}
