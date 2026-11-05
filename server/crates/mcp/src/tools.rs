//! Operation handlers: ports of the matching route handlers. Each parses its
//! arguments, calls the same `core` function with the principal's user id, and maps the result.
//! Nothing here decides who may see what; `core` does, on every call.

use penfii_core::oauth::{Principal, RiskClass, policy, scopes};
use penfii_core::search::{self, SearchError, SearchParams};
use penfii_core::work::{TicketInput, TicketUpdate};
use penfii_core::{CoreError, access, directory, organization, work};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::McpContext;
use crate::catalog::{Operation, WorkspaceResolver};

/// A tool-level failure: reported as `isError: true` with a stable code, never as a protocol
/// error (the model can often act on it).
#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    /// Scope the call would have needed (insufficient_scope only).
    pub needed_scope: Option<String>,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            needed_scope: None,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message)
    }

    pub fn outcome(&self) -> &'static str {
        match self.code {
            "insufficient_scope" => "scope_denied",
            "policy_denied" => "policy_denied",
            "forbidden" => "forbidden",
            "not_found" => "not_found",
            _ => "error",
        }
    }
}

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Self {
        match error {
            // Existence is not revealed.
            CoreError::NotFound => Self::new("not_found", "not found"),
            CoreError::Unauthenticated => Self::new("not_found", "not found"),
            CoreError::Forbidden(message) => Self::new("forbidden", message),
            CoreError::Validation(message) => Self::new("invalid_arguments", message),
            CoreError::Rule { code, message } => Self::new(code, message),
            CoreError::Conflict { code, message, .. } => Self::new(code, message),
            CoreError::ReauthRequired { .. } => {
                Self::new("reauth_required", "recent authentication is required")
            }
            CoreError::RateLimited { .. } => Self::new("rate_limited", "too many requests"),
            CoreError::Internal(message) => {
                tracing::error!(%message, "mcp tool internal error");
                Self::new("internal_error", "internal error")
            }
        }
    }
}

impl From<sqlx::Error> for Failure {
    fn from(error: sqlx::Error) -> Self {
        CoreError::from(error).into()
    }
}

/// What a successful operation hands back, plus the facts the call log records.
pub struct Done {
    pub value: Value,
    pub resource_type: Option<&'static str>,
    pub resource_id: Option<String>,
    /// Revision a write produced; ties the call to its Activity row.
    pub resource_revision: Option<i64>,
}

fn uuid_arg(args: &Value, name: &str) -> Result<Uuid, Failure> {
    args.get(name)
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .ok_or_else(|| Failure::invalid(format!("{name} must be a UUID")))
}

fn string_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn limit_arg(args: &Value, default: i64) -> Result<i64, Failure> {
    match args.get("limit") {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_i64()
            .filter(|limit| (1..=50).contains(limit))
            .ok_or_else(|| Failure::invalid("limit must be between 1 and 50")),
    }
}

/// Steps 2 to 3 of the Authorization Model: scope, grant restriction, membership, Workspace
/// policy. Returns the Workspace the operation targets, if any.
pub async fn authorize(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Result<Option<Uuid>, Failure> {
    for needed in op.scopes {
        if !scopes::grants(&principal.scopes, needed) {
            return Err(Failure {
                code: "insufficient_scope",
                message: format!("this connection was not granted {needed}"),
                needed_scope: Some((*needed).to_string()),
            });
        }
    }
    let workspace_id = match op.resolver {
        WorkspaceResolver::None => return Ok(None),
        WorkspaceResolver::Argument => uuid_arg(args, "workspaceId")?,
        WorkspaceResolver::Project => {
            let project_id = uuid_arg(args, "projectId")?;
            sqlx::query_scalar::<_, Uuid>("SELECT workspace_id FROM penfii.projects WHERE id = $1")
                .bind(project_id)
                .fetch_optional(&ctx.pool)
                .await?
                .ok_or_else(|| Failure::from(CoreError::NotFound))?
        }
        WorkspaceResolver::Ticket => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            sqlx::query_scalar::<_, Uuid>("SELECT workspace_id FROM penfii.tickets WHERE id = $1")
                .bind(ticket_id)
                .fetch_optional(&ctx.pool)
                .await?
                .ok_or_else(|| Failure::from(CoreError::NotFound))?
        }
    };
    if let Some(allowed) = &principal.workspace_filter
        && !allowed.contains(&workspace_id)
    {
        return Err(CoreError::NotFound.into());
    }
    // Only an active member learns a policy exists; anyone else gets the answer core would give.
    let mut conn = ctx.pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, principal.user_id).await?;
    drop(conn);
    let effective = policy::effective(&ctx.pool, ctx.oauth.policy_defaults, workspace_id).await?;
    if !effective.allows(op.risk) {
        return Err(Failure::new(
            "policy_denied",
            "a Workspace administrator has turned this kind of MCP access off",
        ));
    }
    Ok(Some(workspace_id))
}

pub async fn run(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Result<Done, Failure> {
    let user = principal.user_id;
    match op.id {
        "workspace.list" => {
            let mut items = Vec::new();
            for workspace in penfii_core::organization::list_workspaces(&ctx.pool, user).await? {
                if principal
                    .workspace_filter
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(&workspace.id))
                {
                    continue;
                }
                let effective =
                    policy::effective(&ctx.pool, ctx.oauth.policy_defaults, workspace.id).await?;
                if effective.allows(RiskClass::Read) {
                    items.push(to_value(&workspace)?);
                }
            }
            Ok(done(json!({"workspaces": items})))
        }
        "workspace.overview" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let space = organization::space(&ctx.pool, user, workspace_id).await?;
            Ok(done(to_value(&space)?))
        }
        "member.search" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let found = directory::search(
                &ctx.pool,
                user,
                workspace_id,
                string_arg(args, "query").unwrap_or(""),
                Some("person"),
                Some(limit_arg(args, 20)?),
            )
            .await?;
            Ok(done(to_value(&found)?))
        }
        "search.query" => search_workspace(ctx, principal, args).await,
        "ticket.get" => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            let ticket = work::get_ticket(&ctx.pool, user, ticket_id).await?;
            Ok(Done {
                value: to_value(&ticket)?,
                resource_type: Some("ticket"),
                resource_id: Some(ticket_id.to_string()),
                resource_revision: None,
            })
        }
        "ticket.comments" => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            let comments = work::comments(&ctx.pool, user, ticket_id).await?;
            Ok(Done {
                value: json!({"comments": to_value(&comments)?}),
                resource_type: Some("ticket"),
                resource_id: Some(ticket_id.to_string()),
                resource_revision: None,
            })
        }
        "workflow.get" => {
            let project_id = uuid_arg(args, "projectId")?;
            let (version, states) = work::workflow(&ctx.pool, project_id, user).await?;
            Ok(Done {
                value: json!({"projectId": project_id, "version": version, "states": to_value(&states)?}),
                resource_type: Some("project"),
                resource_id: Some(project_id.to_string()),
                resource_revision: None,
            })
        }
        "ticket.create" => ticket_create(ctx, principal, args).await,
        "ticket.update" => ticket_update(ctx, principal, args).await,
        "ticket.change_status" => ticket_change_status(ctx, principal, args).await,
        "ticket.comment" => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            let body = rich_text(args.get("body"), "body")?
                .ok_or_else(|| Failure::invalid("body is required"))?;
            let (comment, change) =
                work::create_comment(&ctx.pool, user, ticket_id, body, &[]).await?;
            announce(ctx, Some(change));
            Ok(Done {
                value: to_value(&comment)?,
                resource_type: Some("ticket_comment"),
                resource_id: Some(comment.id.to_string()),
                resource_revision: None,
            })
        }
        "project.create" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let name = string_arg(args, "name").unwrap_or("");
            let project = organization::create_project(
                &ctx.pool,
                user,
                workspace_id,
                name,
                &[],
                string_arg(args, "ticketPrefix"),
            )
            .await?;
            Ok(Done {
                value: to_value(&project)?,
                resource_type: Some("project"),
                resource_id: Some(project.id.to_string()),
                resource_revision: None,
            })
        }
        "workflow.delete_state" => {
            let project_id = uuid_arg(args, "projectId")?;
            let state_id = uuid_arg(args, "stateId")?;
            let version = args
                .get("expectedWorkflowVersion")
                .and_then(Value::as_i64)
                .ok_or_else(|| Failure::invalid("expectedWorkflowVersion must be an integer"))?;
            let version =
                work::delete_state(&ctx.pool, user, project_id, state_id, version).await?;
            Ok(Done {
                value: json!({"version": version}),
                resource_type: Some("workflow_state"),
                resource_id: Some(state_id.to_string()),
                resource_revision: None,
            })
        }
        other => Err(Failure::new(
            "internal_error",
            format!("operation {other} has no handler"),
        )),
    }
}

fn done(value: Value) -> Done {
    Done {
        value,
        resource_type: None,
        resource_id: None,
        resource_revision: None,
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|e| CoreError::Internal(e.to_string()).into())
}

/// Result kinds that exist as MCP-readable content today, with the scope each one needs
///.
const SEARCH_KINDS: &[(&str, &str)] = &[("ticket", "ticket:read"), ("person", "member:read")];

async fn search_workspace(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let workspace_id = uuid_arg(args, "workspaceId")?;
    let query = string_arg(args, "query").unwrap_or("");
    let kind = string_arg(args, "kind").unwrap_or("all");
    let readable: Vec<&str> = SEARCH_KINDS
        .iter()
        .filter(|(_, scope)| scopes::grants(&principal.scopes, scope))
        .map(|(kind, _)| *kind)
        .collect();
    let wanted: Vec<&str> = match kind {
        "all" => readable.clone(),
        other => {
            let Some((name, scope)) = SEARCH_KINDS.iter().find(|(name, _)| *name == other) else {
                return Err(Failure::invalid("kind must be all, ticket or person"));
            };
            if !readable.contains(name) {
                return Err(Failure {
                    code: "insufficient_scope",
                    message: format!("this connection was not granted {scope}"),
                    needed_scope: Some((*scope).to_string()),
                });
            }
            vec![*name]
        }
    };
    let limit = limit_arg(args, 20)?;
    let mut results: Vec<Value> = Vec::new();
    let mut next_cursor = Value::Null;
    if wanted.contains(&"person") {
        let found = directory::search(
            &ctx.pool,
            principal.user_id,
            workspace_id,
            query,
            Some("person"),
            Some(if wanted.len() == 1 { limit } else { 5 }),
        )
        .await?;
        for hit in found.results {
            results.push(json!({
                "kind": "person", "id": hit.id, "title": hit.title, "subtitle": hit.subtitle,
            }));
        }
    }
    if wanted.contains(&"ticket") {
        let params = SearchParams {
            q: query.to_string(),
            kind: "ticket".into(),
            state_ids: Vec::new(),
            limit: Some(limit),
            cursor: string_arg(args, "cursor").map(str::to_string),
            tz: None,
        };
        match search::search(
            &ctx.pool,
            ctx.search.as_ref(),
            principal.user_id,
            workspace_id,
            params,
        )
        .await
        {
            Ok(response) => {
                let value = to_value(&response)?;
                if let Some(items) = value["results"].as_array() {
                    results.extend(items.iter().cloned());
                }
                next_cursor = value["nextCursor"].clone();
            }
            Err(SearchError::Invalid(invalid)) => {
                return Err(Failure::invalid(format!(
                    "{}: {}",
                    invalid.code, invalid.message
                )));
            }
            Err(SearchError::Ambiguous { token, .. }) => {
                return Err(Failure::new(
                    "ambiguous_filter",
                    format!("more than one match for {token}; use a more specific value"),
                ));
            }
            Err(SearchError::Unavailable) => {
                return Err(Failure::new(
                    "search_unavailable",
                    "search is not available right now; try again later",
                ));
            }
            Err(SearchError::Core(error)) => return Err(error.into()),
        }
    }
    Ok(done(json!({
        "query": query,
        "kinds": wanted,
        "results": results,
        "nextCursor": next_cursor,
    })))
}

/// Plain text becomes a one-paragraph document; an object is passed to `core`, which validates
/// rich text exactly as it does for the web app (JSON first, plain text as a convenience).
fn rich_text(value: Option<&Value>, name: &str) -> Result<Option<Value>, Failure> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            if text.trim().is_empty() {
                return Err(Failure::invalid(format!("{name} must not be empty")));
            }
            if text.len() > 20_000 {
                return Err(Failure::invalid(format!("{name} is too long")));
            }
            Ok(Some(json!({"type": "doc", "content": text
                .split("\n\n")
                .map(|paragraph| json!({"type": "paragraph", "content": [{"type": "text", "text": paragraph}]}))
                .collect::<Vec<_>>()})))
        }
        Some(object @ Value::Object(_)) => Ok(Some(object.clone())),
        Some(_) => Err(Failure::invalid(format!(
            "{name} must be text or a document object"
        ))),
    }
}

/// Retry key: the caller's, else a fresh one (no replay protection, and the tool says so).
fn idempotency(args: &Value) -> Result<String, Failure> {
    match args.get("idempotencyKey") {
        None | Some(Value::Null) => Ok(format!("mcp-{}", Uuid::new_v4())),
        Some(Value::String(key)) if !key.trim().is_empty() && key.len() <= 200 => {
            Ok(format!("mcp:{}", key.trim()))
        }
        Some(_) => Err(Failure::invalid("idempotencyKey must be 1..200 characters")),
    }
}

/// `absent` leaves a field alone, `null` clears it, anything else sets it.
fn nullable<T>(
    args: &Value,
    name: &str,
    parse: impl Fn(&Value) -> Option<T>,
) -> Result<Option<Option<T>>, Failure> {
    match args.get(name) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => parse(value)
            .map(|parsed| Some(Some(parsed)))
            .ok_or_else(|| Failure::invalid(format!("{name} has the wrong type"))),
    }
}

fn parse_uuid(value: &Value) -> Option<Uuid> {
    value.as_str().and_then(|raw| Uuid::parse_str(raw).ok())
}

fn parse_date(value: &Value) -> Option<chrono::NaiveDate> {
    value.as_str().and_then(|raw| raw.parse().ok())
}

fn revision_arg(args: &Value) -> Result<i64, Failure> {
    args.get("expectedRevision")
        .and_then(Value::as_i64)
        .ok_or_else(|| Failure::invalid("expectedRevision must be an integer"))
}

/// The same post-commit effect the route applies: open browsers learn of the change.
fn announce(ctx: &McpContext, change: Option<penfii_core::ChangeNotice>) {
    if let Some(change) = change {
        let _ = ctx.changes.send(change);
    }
}

/// `previous` is the revision the caller expected: a result that did not move past it wrote
/// nothing, so the log must not claim a revision (and with it the Activity row) for this call.
fn ticket_done(
    ctx: &McpContext,
    ticket: penfii_contracts::Ticket,
    change: Option<penfii_core::ChangeNotice>,
    previous: Option<i64>,
) -> Result<Done, Failure> {
    announce(ctx, change);
    let wrote = previous.is_none_or(|previous| ticket.revision_no > previous);
    Ok(Done {
        value: to_value(&ticket)?,
        resource_type: Some("ticket"),
        resource_id: Some(ticket.id.to_string()),
        resource_revision: wrote.then_some(ticket.revision_no),
    })
}

async fn ticket_create(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let project_id = uuid_arg(args, "projectId")?;
    let title = string_arg(args, "title").ok_or_else(|| Failure::invalid("title is required"))?;
    let input = TicketInput {
        title: title.to_string(),
        description: rich_text(args.get("description"), "description")?.unwrap_or(Value::Null),
        hierarchy_kind: string_arg(args, "hierarchyKind")
            .unwrap_or("TASK")
            .to_string(),
        parent_id: nullable(args, "parentId", parse_uuid)?.flatten(),
        channel_id: None,
        status_id: nullable(args, "statusId", parse_uuid)?.flatten(),
        assignee_id: nullable(args, "assigneeId", parse_uuid)?.flatten(),
        priority: nullable(args, "priority", |v| v.as_str().map(str::to_string))?.flatten(),
        due_on: nullable(args, "dueOn", parse_date)?.flatten(),
        label_ids: Vec::new(),
        type_id: None,
        estimate: None,
        schedule: None,
        file_ids: Vec::new(),
    };
    let (ticket, change) = work::create_ticket(
        &ctx.pool,
        principal.user_id,
        project_id,
        input,
        &idempotency(args)?,
    )
    .await?;
    ticket_done(ctx, ticket, change, None)
}

async fn ticket_update(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let ticket_id = uuid_arg(args, "ticketId")?;
    let update = TicketUpdate {
        title: string_arg(args, "title").map(str::to_string),
        description: rich_text(args.get("description"), "description")?,
        assignee_id: nullable(args, "assigneeId", parse_uuid)?,
        priority: nullable(args, "priority", |v| v.as_str().map(str::to_string))?,
        due_on: nullable(args, "dueOn", parse_date)?,
        ..TicketUpdate::default()
    };
    let expected = revision_arg(args)?;
    let (ticket, change) = work::update_ticket(
        &ctx.pool,
        principal.user_id,
        ticket_id,
        expected,
        update,
        &idempotency(args)?,
    )
    .await?;
    ticket_done(ctx, ticket, change, Some(expected))
}

async fn ticket_change_status(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let expected = revision_arg(args)?;
    let (ticket, change) = work::change_status(
        &ctx.pool,
        principal.user_id,
        uuid_arg(args, "ticketId")?,
        expected,
        uuid_arg(args, "statusId")?,
        &idempotency(args)?,
    )
    .await?;
    ticket_done(ctx, ticket, change, Some(expected))
}
