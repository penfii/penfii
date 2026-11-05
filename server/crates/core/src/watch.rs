//! Watch: a personal, per-person choice to follow a Ticket, a Project or a Thread.
//!
//! Following is automatic after taking part (`AUTO`), chosen (`EXPLICIT`) or refused (`OPT_OUT`).
//! Activity creates only `AUTO` rows and never touches a row that already exists, so a refusal
//! stays a refusal.

use serde::Serialize;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::visibility;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub target_kind: String,
    pub target_id: Uuid,
    pub mode: String,
    pub watching: bool,
    pub version: i64,
}

fn watching(mode: &str) -> bool {
    mode == "AUTO" || mode == "EXPLICIT"
}

fn target_kind(kind: &str) -> Result<&'static str, CoreError> {
    match kind {
        "TICKET" => Ok("TICKET"),
        "PROJECT" => Ok("PROJECT"),
        "THREAD" => Ok("THREAD"),
        _ => Err(CoreError::validation("unknown watch target")),
    }
}

/// The Workspace of a target the person can see, or NotFound.
async fn visible_target(
    conn: &mut PgConnection,
    actor: Uuid,
    kind: &str,
    id: Uuid,
) -> Result<Uuid, CoreError> {
    match kind {
        "TICKET" => {
            if !visibility::tickets(conn, actor, &[id]).await?.contains(&id) {
                return Err(CoreError::NotFound);
            }
            Ok(
                sqlx::query_scalar("SELECT workspace_id FROM penfii.tickets WHERE id=$1")
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?,
            )
        }
        "PROJECT" => {
            // Following a Project is for its participants: the list shows every Project, its inside is theirs.
            let (_, role) = crate::project_role::project_role(conn, id, actor).await?;
            if role.is_none() {
                return Err(CoreError::NotFound);
            }
            Ok(
                sqlx::query_scalar("SELECT workspace_id FROM penfii.projects WHERE id=$1")
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?,
            )
        }
        _ => {
            // A Thread is its root message.
            let root: Option<(Option<Uuid>,)> =
                sqlx::query_as("SELECT thread_root_id FROM penfii.messages WHERE id=$1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?;
            match root {
                Some((None,)) if visibility::messages(conn, actor, &[id]).await?.contains(&id) => {
                    Ok(sqlx::query_scalar(
                        "SELECT c.workspace_id FROM penfii.messages m JOIN penfii.conversations c ON c.id=m.conversation_id WHERE m.id=$1",
                    )
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?)
                }
                _ => Err(CoreError::NotFound),
            }
        }
    }
}

/// Starts (`on`) or refuses (`off`) following. The row is kept either way.
pub async fn set(
    pool: &PgPool,
    actor: Uuid,
    kind: &str,
    target_id: Uuid,
    on: bool,
) -> Result<Subscription, CoreError> {
    let kind = target_kind(kind)?;
    let mut tx = pool.begin().await?;
    let workspace = visible_target(&mut tx, actor, kind, target_id).await?;
    let mode = if on { "EXPLICIT" } else { "OPT_OUT" };
    let row = sqlx::query(
        "INSERT INTO penfii.subscriptions(user_id, workspace_id, target_kind, target_id, mode, initialized_at)
         VALUES($1,$2,$3,$4,$5,now())
         ON CONFLICT (user_id, target_kind, target_id)
         DO UPDATE SET mode = EXCLUDED.mode, version = penfii.subscriptions.version + 1
            WHERE penfii.subscriptions.mode <> EXCLUDED.mode
         RETURNING mode, version",
    )
    .bind(actor)
    .bind(workspace)
    .bind(kind)
    .bind(target_id)
    .bind(mode)
    .fetch_optional(&mut *tx)
    .await?;
    let (mode, version): (String, i64) = match row {
        Some(row) => (row.try_get("mode")?, row.try_get("version")?),
        None => {
            let row = sqlx::query(
                "SELECT mode, version FROM penfii.subscriptions WHERE user_id=$1 AND target_kind=$2 AND target_id=$3",
            )
            .bind(actor)
            .bind(kind)
            .bind(target_id)
            .fetch_one(&mut *tx)
            .await?;
            (row.try_get("mode")?, row.try_get("version")?)
        }
    };
    tx.commit().await?;
    Ok(Subscription {
        target_kind: kind.into(),
        target_id,
        watching: watching(&mode),
        mode,
        version,
    })
}

pub async fn status(
    pool: &PgPool,
    actor: Uuid,
    kind: &str,
    target_id: Uuid,
) -> Result<Subscription, CoreError> {
    let kind = target_kind(kind)?;
    let mut conn = pool.acquire().await?;
    visible_target(&mut conn, actor, kind, target_id).await?;
    let row = sqlx::query(
        "SELECT mode, version FROM penfii.subscriptions WHERE user_id=$1 AND target_kind=$2 AND target_id=$3",
    )
    .bind(actor)
    .bind(kind)
    .bind(target_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match row {
        Some(row) => {
            let mode: String = row.try_get("mode")?;
            Subscription {
                target_kind: kind.into(),
                target_id,
                watching: watching(&mode),
                mode,
                version: row.try_get("version")?,
            }
        }
        None => Subscription {
            target_kind: kind.into(),
            target_id,
            mode: "NONE".into(),
            watching: false,
            version: 0,
        },
    })
}

/// What a person follows in a Workspace, limited to what they can still see.
pub async fn list(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    kind: Option<&str>,
) -> Result<Vec<Subscription>, CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let kind = kind.map(target_kind).transpose()?;
    let rows = sqlx::query(
        "SELECT target_kind, target_id, mode, version FROM penfii.subscriptions
         WHERE user_id=$1 AND workspace_id=$2 AND mode IN ('AUTO','EXPLICIT') AND ($3::text IS NULL OR target_kind=$3)
         ORDER BY created_at DESC, target_id LIMIT 500",
    )
    .bind(actor)
    .bind(workspace_id)
    .bind(kind)
    .fetch_all(&mut *conn)
    .await?;
    let mut by_kind: std::collections::HashMap<String, Vec<Uuid>> = Default::default();
    for row in &rows {
        by_kind
            .entry(row.try_get("target_kind")?)
            .or_default()
            .push(row.try_get("target_id")?);
    }
    let mut visible = std::collections::HashSet::new();
    for (k, ids) in &by_kind {
        let found = match k.as_str() {
            "TICKET" => visibility::tickets(&mut conn, actor, ids).await?,
            "PROJECT" => visibility::projects(&mut conn, actor, ids).await?,
            _ => visibility::messages(&mut conn, actor, ids).await?,
        };
        visible.extend(found);
    }
    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("target_id")?;
        if visible.contains(&id) {
            let mode: String = row.try_get("mode")?;
            out.push(Subscription {
                target_kind: row.try_get("target_kind")?,
                target_id: id,
                watching: watching(&mode),
                mode,
                version: row.try_get("version")?,
            });
        }
    }
    Ok(out)
}

/// Follows on the person's behalf after they took part. Never changes an existing row.
pub async fn follow_automatically(
    conn: &mut PgConnection,
    user_id: Uuid,
    workspace_id: Uuid,
    kind: &str,
    target_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.subscriptions(user_id, workspace_id, target_kind, target_id, mode, initialized_at)
         VALUES($1,$2,$3,$4,'AUTO',now()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(kind)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// People who follow a target now (AUTO or EXPLICIT). Whether they may still see it is the
/// caller's question to the visibility service.
pub async fn followers(
    conn: &mut PgConnection,
    kind: &str,
    target_id: Uuid,
) -> Result<Vec<Uuid>, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM penfii.subscriptions WHERE target_kind=$1 AND target_id=$2 AND mode IN ('AUTO','EXPLICIT')",
    )
    .bind(kind)
    .bind(target_id)
    .fetch_all(&mut *conn)
    .await?)
}
