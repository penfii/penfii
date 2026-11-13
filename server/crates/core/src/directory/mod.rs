//! Directory search over people, workspaces, and readable messages.
//!
//! A hit carries a `type` so a later OpenSearch index can store the same
//! documents. PostgreSQL stays the source of truth. Channel, team, project,
//! and document are reserved type names and are not queried yet.

use penfii_contracts::{
    DirectoryHit, DirectoryOrganization, DirectoryPerson, DirectorySearch, OrganizationDirectory,
};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::identity;

const KNOWN_TYPES: &[&str] = &["person", "organization"];

pub async fn search(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    query: &str,
    types: Option<&str>,
    limit: Option<i64>,
) -> Result<DirectorySearch, CoreError> {
    let query = normalize_query(query)?;
    let limit = crate::paging::require_limit(limit, 20, 50, "limit is invalid")?;
    let selected = parse_types(types, query.is_empty())?;
    let pattern = if query.is_empty() {
        None
    } else {
        Some(like_contains(&query))
    };

    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor_id).await?;
    let organization_name: String =
        sqlx::query_scalar("SELECT name FROM penfii.workspaces WHERE id = $1")
            .bind(workspace_id)
            .fetch_one(&mut *tx)
            .await?;

    let mut results = Vec::new();
    if selected.contains(&"person") {
        let people = load_people(
            &mut tx,
            actor_id,
            workspace_id,
            &organization_name,
            pattern.as_deref(),
            false,
            limit,
        )
        .await?;
        results.extend(people.into_iter().map(person_hit));
    }
    if selected.contains(&"organization") && pattern.is_some() {
        let organizations =
            load_organizations(&mut tx, actor_id, pattern.as_deref().unwrap(), limit).await?;
        results.extend(organizations.into_iter().map(organization_hit));
    }
    tx.commit().await?;
    Ok(DirectorySearch { query, results })
}

pub async fn organization(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    organization_id: Uuid,
) -> Result<OrganizationDirectory, CoreError> {
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor_id).await?;
    let organization = load_organization(&mut tx, actor_id, organization_id).await?;
    let members = load_people(
        &mut tx,
        actor_id,
        organization_id,
        &organization.name,
        None,
        true,
        200,
    )
    .await?;
    tx.commit().await?;
    Ok(OrganizationDirectory {
        organization,
        members,
    })
}

/// Creates the shared development directory when this binary is a debug build.
/// Release binaries ignore the call so production does not create accounts.
pub async fn seed_dev(pool: &PgPool) -> Result<bool, CoreError> {
    if !cfg!(debug_assertions) {
        return Ok(false);
    }
    // One hash for every development account, computed before the transaction opens.
    let dev_hash = identity::hash_password("1234").await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4815162342)")
        .execute(&mut *tx)
        .await?;

    let ours = [
        ("alice@ourspace.dev", "alice", "Alice Kim", true),
        ("bob@ourspace.dev", "bob", "Bob Lee", false),
        ("charlie@ourspace.dev", "charlie", "Charlie Park", false),
        ("david@ourspace.dev", "david", "David Choi", false),
    ];
    let mut our_ids = Vec::with_capacity(ours.len());
    let mut owner_id = None;
    for (email, username, display_name, owner) in ours {
        let user_id = ensure_user(&mut tx, email, username, display_name, &dev_hash).await?;
        if owner {
            owner_id = Some(user_id);
        }
        our_ids.push(user_id);
    }
    let owner_id = owner_id.expect("alice is the directory owner");
    let workspace_id =
        ensure_workspace(&mut tx, owner_id, "alice@ourspace.dev", "OurSpace").await?;
    for user_id in &our_ids {
        let role = if *user_id == owner_id {
            "OWNER"
        } else {
            "MEMBER"
        };
        sqlx::query(
            "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
             VALUES ($1, $2, $3, 'ACTIVE')
             ON CONFLICT (workspace_id, user_id) DO NOTHING",
        )
        .bind(workspace_id)
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }
    let team_id = ensure_team(&mut tx, workspace_id, "Product").await?;
    for user_id in &our_ids {
        sqlx::query(
            "INSERT INTO penfii.team_members (team_id, user_id, role)
             VALUES ($1, $2, 'MEMBER')
             ON CONFLICT (team_id, user_id) DO NOTHING",
        )
        .bind(team_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }

    let outsider_id =
        ensure_user(&mut tx, "erin@outside.dev", "erin", "Erin Cho", &dev_hash).await?;
    let outside_id =
        ensure_workspace(&mut tx, outsider_id, "erin@outside.dev", "Outside Org").await?;
    sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, 'OWNER', 'ACTIVE')
         ON CONFLICT (workspace_id, user_id) DO NOTHING",
    )
    .bind(outside_id)
    .bind(outsider_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

async fn ensure_user(
    conn: &mut PgConnection,
    email: &str,
    username: &str,
    display_name: &str,
    password_hash: &str,
) -> Result<Uuid, CoreError> {
    let (email, email_key) = identity::normalize_email(email)?;
    let display_name = identity::require_label(display_name, "displayName")?;
    let username_key = username.trim().to_lowercase();
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO penfii.users
            (id, email, email_key, email_verified_at, display_name, locale, time_zone, username, username_key)
         VALUES ($1, $2, $3, now(), $4, 'ko-KR', 'Asia/Seoul', $5, $6)
         ON CONFLICT (email_key) DO UPDATE
         SET username = COALESCE(penfii.users.username, EXCLUDED.username),
             username_key = COALESCE(penfii.users.username_key, EXCLUDED.username_key)
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(&email)
    .bind(&email_key)
    .bind(&display_name)
    .bind(username)
    .bind(&username_key)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.password_credentials (user_id, password_hash)
         VALUES ($1, $2)
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(&mut *conn)
    .await?;
    Ok(user_id)
}

async fn ensure_workspace(
    conn: &mut PgConnection,
    owner_id: Uuid,
    owner_email_key: &str,
    name: &str,
) -> Result<Uuid, CoreError> {
    let (_, email_key) = identity::normalize_email(owner_email_key)?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT w.id
         FROM penfii.workspaces w
         JOIN penfii.workspace_members m ON m.workspace_id = w.id AND m.state = 'ACTIVE'
         JOIN penfii.users u ON u.id = m.user_id
         WHERE u.email_key = $1 AND w.name = $2
         ORDER BY w.created_at
         LIMIT 1",
    )
    .bind(&email_key)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    identity::insert_owned_workspace(conn, owner_id, name, "Asia/Seoul").await
}

async fn ensure_team(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    name: &str,
) -> Result<Uuid, CoreError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM penfii.teams
         WHERE workspace_id = $1 AND name = $2 AND archived_at IS NULL
         ORDER BY id
         LIMIT 1",
    )
    .bind(workspace_id)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.teams (id, workspace_id, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(workspace_id)
        .bind(name)
        .execute(&mut *conn)
        .await?;
    // Same event the Team API writes, so the new Team is indexed like any other.
    crate::events::emit_simple(
        &mut *conn,
        workspace_id,
        "TEAM",
        id,
        "team.created",
        None,
        serde_json::json!({}),
    )
    .await?;
    Ok(id)
}

async fn load_people(
    conn: &mut PgConnection,
    actor_id: Uuid,
    workspace_id: Uuid,
    organization_name: &str,
    pattern: Option<&str>,
    include_self: bool,
    limit: i64,
) -> Result<Vec<DirectoryPerson>, CoreError> {
    let rows = sqlx::query_as::<_, PersonRow>(
        "SELECT u.id, u.display_name, u.username, u.email,
                COALESCE(
                    array_agg(DISTINCT t.name) FILTER (WHERE t.name IS NOT NULL),
                    ARRAY[]::text[]
                ) AS affiliation
         FROM penfii.workspace_members wm
         JOIN penfii.users u ON u.id = wm.user_id
         LEFT JOIN penfii.team_members tm ON tm.user_id = u.id
         LEFT JOIN penfii.teams t
            ON t.id = tm.team_id AND t.workspace_id = wm.workspace_id AND t.archived_at IS NULL
         LEFT JOIN (
             SELECT other.user_id AS user_id, MAX(c.last_message_seq) AS activity
             FROM penfii.dm_members mine
             JOIN penfii.conversations c ON c.id = mine.conversation_id
             JOIN penfii.dm_members other
                ON other.conversation_id = c.id AND other.user_id <> mine.user_id
             WHERE mine.user_id = $1 AND c.workspace_id = $2 AND c.kind = 'DM'
             GROUP BY other.user_id
         ) recent ON recent.user_id = u.id
         WHERE wm.workspace_id = $2
           AND wm.state = 'ACTIVE'
           AND u.disabled_at IS NULL
           AND ($5::bool OR u.id <> $1)
           AND (
                $3::text IS NULL
                OR u.display_name ILIKE $3 ESCAPE '\\'
                OR u.username_key ILIKE $3 ESCAPE '\\'
                OR u.email_key ILIKE $3 ESCAPE '\\'
           )
         GROUP BY u.id, u.display_name, u.username, u.email, recent.activity
         ORDER BY recent.activity DESC NULLS LAST, u.display_name ASC
         LIMIT $4",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .bind(pattern)
    .bind(limit)
    .bind(include_self)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DirectoryPerson {
            id: row.id,
            display_name: row.display_name,
            username: row.username,
            email: row.email,
            organization_id: workspace_id,
            organization_name: organization_name.to_string(),
            affiliation: row.affiliation,
        })
        .collect())
}

async fn load_organizations(
    conn: &mut PgConnection,
    actor_id: Uuid,
    pattern: &str,
    limit: i64,
) -> Result<Vec<DirectoryOrganization>, CoreError> {
    let rows = sqlx::query_as::<_, OrganizationRow>(
        "SELECT w.id, w.name, w.time_zone,
                (SELECT count(*)::bigint FROM penfii.workspace_members m
                 WHERE m.workspace_id = w.id AND m.state = 'ACTIVE') AS member_count
         FROM penfii.workspaces w
         JOIN penfii.workspace_members mine
            ON mine.workspace_id = w.id AND mine.user_id = $1 AND mine.state = 'ACTIVE'
         WHERE w.name ILIKE $2 ESCAPE '\\'
         ORDER BY w.name, w.id
         LIMIT $3",
    )
    .bind(actor_id)
    .bind(pattern)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(organization_from).collect())
}

async fn load_organization(
    conn: &mut PgConnection,
    actor_id: Uuid,
    organization_id: Uuid,
) -> Result<DirectoryOrganization, CoreError> {
    let row = sqlx::query_as::<_, OrganizationRow>(
        "SELECT w.id, w.name, w.time_zone,
                (SELECT count(*)::bigint FROM penfii.workspace_members m
                 WHERE m.workspace_id = w.id AND m.state = 'ACTIVE') AS member_count
         FROM penfii.workspaces w
         JOIN penfii.workspace_members mine
            ON mine.workspace_id = w.id AND mine.user_id = $1 AND mine.state = 'ACTIVE'
         WHERE w.id = $2",
    )
    .bind(actor_id)
    .bind(organization_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(organization_from).ok_or(CoreError::NotFound)
}

fn person_hit(person: DirectoryPerson) -> DirectoryHit {
    let subtitle = person_subtitle(&person);
    let id = person.id;
    let title = person.display_name.clone();
    DirectoryHit {
        kind: "person".to_string(),
        id,
        title,
        subtitle,
        person: Some(person),
        organization: None,
        message: None,
    }
}

fn organization_hit(organization: DirectoryOrganization) -> DirectoryHit {
    DirectoryHit {
        kind: "organization".to_string(),
        id: organization.id,
        title: organization.name.clone(),
        subtitle: organization.time_zone.clone(),
        person: None,
        organization: Some(organization),
        message: None,
    }
}

fn person_subtitle(person: &DirectoryPerson) -> String {
    let handle = person
        .username
        .as_ref()
        .map(|username| format!("@{username}"))
        .unwrap_or_else(|| person.email.clone());
    let mut parts = vec![handle, person.organization_name.clone()];
    if !person.affiliation.is_empty() {
        parts.insert(1, person.affiliation.join(", "));
    }
    parts.join(" · ")
}

fn organization_from(row: OrganizationRow) -> DirectoryOrganization {
    DirectoryOrganization {
        id: row.id,
        name: row.name,
        time_zone: row.time_zone,
        member_count: row.member_count,
    }
}

fn parse_types(raw: Option<&str>, query_empty: bool) -> Result<Vec<&'static str>, CoreError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(if query_empty {
            vec!["person"]
        } else {
            vec!["person", "organization"]
        });
    };
    let mut selected = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(known) = KNOWN_TYPES.iter().copied().find(|item| *item == part) else {
            return Err(CoreError::validation(
                "types contains an unknown search type",
            ));
        };
        if !selected.contains(&known) {
            selected.push(known);
        }
    }
    if selected.is_empty() {
        return Err(CoreError::validation("types is invalid"));
    }
    Ok(selected)
}

fn normalize_query(query: &str) -> Result<String, CoreError> {
    let trimmed = query.trim();
    if trimmed.chars().count() > 200 || trimmed.chars().any(|ch| ch.is_control()) {
        return Err(CoreError::validation("q is invalid"));
    }
    Ok(trimmed.to_string())
}

fn like_contains(query: &str) -> String {
    let mut escaped = String::new();
    for ch in query.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    format!("%{escaped}%")
}

#[derive(sqlx::FromRow)]
struct PersonRow {
    id: Uuid,
    display_name: String,
    username: Option<String>,
    email: String,
    affiliation: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct OrganizationRow {
    id: Uuid,
    name: String,
    time_zone: String,
    member_count: i64,
}
