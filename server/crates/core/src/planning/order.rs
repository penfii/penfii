//! The shared Ticket order within a Project: placement, midpoint keys and rebalancing.

use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::project_role::channel_allowed;

/// Distance between neighbours when a rank is appended or the order is rebalanced.
const ORDER_STEP: i64 = 1024;

/// Where a Ticket goes in the Project's shared order. An anchor places the Ticket right next
/// to another Ticket, so a stale client position can never overwrite someone else's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Keep,
    First,
    Last,
    After(Uuid),
    Before(Uuid),
}

impl Placement {
    pub fn to_json(self) -> Value {
        match self {
            Self::Keep => json!({}),
            Self::First => json!({"position":"first"}),
            Self::Last => json!({"position":"last"}),
            Self::After(id) => json!({"afterId":id}),
            Self::Before(id) => json!({"beforeId":id}),
        }
    }
}

pub(crate) async fn lock_project(
    conn: &mut PgConnection,
    project_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query("SELECT id FROM penfii.projects WHERE id=$1 FOR UPDATE")
        .bind(project_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(())
}

/// A new Ticket joins the end of the shared order. The caller holds the Project row lock.
pub(crate) async fn append_order(
    conn: &mut PgConnection,
    project_id: Uuid,
    ticket_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.project_ticket_order(ticket_id,project_id,sort_key)
         SELECT $1,$2,coalesce(max(sort_key),0)+$3::int FROM penfii.project_ticket_order WHERE project_id=$2",
    )
    .bind(ticket_id)
    .bind(project_id)
    .bind(ORDER_STEP as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

enum Target {
    Key(String),
    Nothing,
    Crowded,
}

async fn step(conn: &mut PgConnection, key: &str, delta: i32) -> Result<String, CoreError> {
    Ok(sqlx::query_scalar("SELECT ($1::numeric+$2::int)::text")
        .bind(key)
        .bind(delta)
        .fetch_one(&mut *conn)
        .await?)
}

/// Midpoint of two ranks, or `None` when they are too close to split.
async fn midpoint(conn: &mut PgConnection, a: &str, b: &str) -> Result<Option<String>, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT CASE WHEN abs($1::numeric-$2::numeric) >= 0.000001 THEN (($1::numeric+$2::numeric)/2)::text END",
    )
    .bind(a)
    .bind(b)
    .fetch_one(&mut *conn)
    .await?)
}

async fn anchor_key(
    conn: &mut PgConnection,
    actor_id: Uuid,
    project_id: Uuid,
    ticket_id: Uuid,
    anchor: Uuid,
) -> Result<String, CoreError> {
    if anchor == ticket_id {
        return Err(CoreError::validation(
            "a Ticket cannot be placed next to itself",
        ));
    }
    let stale = || {
        CoreError::conflict(
            "stale_anchor",
            "the Ticket to place next to is no longer on this board",
        )
    };
    let row = sqlx::query(
        "SELECT o.sort_key::text AS sort_key, t.channel_id FROM penfii.project_ticket_order o
         JOIN penfii.tickets t ON t.id=o.ticket_id
         WHERE o.ticket_id=$1 AND o.project_id=$2
           AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL",
    )
    .bind(anchor)
    .bind(project_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(stale)?;
    // A Ticket the actor cannot see is no neighbour: it looks the same as a missing one.
    let channel: Option<Uuid> = row.try_get("channel_id")?;
    if !channel_allowed(conn, channel, actor_id).await? {
        return Err(stale());
    }
    Ok(row.try_get("sort_key")?)
}

async fn target_key(
    conn: &mut PgConnection,
    actor_id: Uuid,
    project_id: Uuid,
    ticket_id: Uuid,
    placement: Placement,
) -> Result<Target, CoreError> {
    match placement {
        Placement::Keep => Ok(Target::Nothing),
        Placement::First | Placement::Last => {
            let (direction, delta) = if placement == Placement::First {
                ("ASC", -(ORDER_STEP as i32))
            } else {
                ("DESC", ORDER_STEP as i32)
            };
            let edge: Option<String> = sqlx::query_scalar(&format!(
                "SELECT sort_key::text FROM penfii.project_ticket_order
                 WHERE project_id=$1 AND ticket_id<>$2
                 ORDER BY sort_key {direction}, ticket_id {direction} LIMIT 1"
            ))
            .bind(project_id)
            .bind(ticket_id)
            .fetch_optional(&mut *conn)
            .await?;
            match edge {
                None => Ok(Target::Nothing),
                Some(key) => Ok(Target::Key(step(conn, &key, delta).await?)),
            }
        }
        Placement::After(anchor) | Placement::Before(anchor) => {
            let after = matches!(placement, Placement::After(_));
            let key = anchor_key(conn, actor_id, project_id, ticket_id, anchor).await?;
            let (cmp, direction, delta) = if after {
                (">", "ASC", ORDER_STEP as i32)
            } else {
                ("<", "DESC", -(ORDER_STEP as i32))
            };
            let neighbour: Option<String> = sqlx::query_scalar(&format!(
                "SELECT sort_key::text FROM penfii.project_ticket_order
                 WHERE project_id=$1 AND ticket_id<>$2 AND (sort_key, ticket_id) {cmp} ($3::numeric, $4::uuid)
                 ORDER BY sort_key {direction}, ticket_id {direction} LIMIT 1"
            ))
            .bind(project_id)
            .bind(ticket_id)
            .bind(&key)
            .bind(anchor)
            .fetch_optional(&mut *conn)
            .await?;
            match neighbour {
                None => Ok(Target::Key(step(conn, &key, delta).await?)),
                Some(other) => match midpoint(conn, &key, &other).await? {
                    Some(mid) => Ok(Target::Key(mid)),
                    None => Ok(Target::Crowded),
                },
            }
        }
    }
}

async fn rebalance(conn: &mut PgConnection, project_id: Uuid) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.project_ticket_order o SET sort_key=r.rn*$2::int, version=o.version+1
         FROM (SELECT ticket_id, row_number() OVER (ORDER BY sort_key, ticket_id) AS rn
               FROM penfii.project_ticket_order WHERE project_id=$1) r
         WHERE o.ticket_id=r.ticket_id",
    )
    .bind(project_id)
    .bind(ORDER_STEP as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Moves a Ticket in the shared order. The caller holds the Project row lock and has checked
/// edit permission. Returns whether the rank changed.
pub(crate) async fn reposition(
    conn: &mut PgConnection,
    actor_id: Uuid,
    project_id: Uuid,
    ticket_id: Uuid,
    placement: Placement,
) -> Result<bool, CoreError> {
    if placement == Placement::Keep {
        return Ok(false);
    }
    for _ in 0..2 {
        match target_key(conn, actor_id, project_id, ticket_id, placement).await? {
            Target::Nothing => return Ok(false),
            Target::Crowded => rebalance(conn, project_id).await?,
            Target::Key(key) => {
                sqlx::query(
                    "INSERT INTO penfii.project_ticket_order(ticket_id,project_id,sort_key)
                     VALUES($1,$2,$3::numeric)
                     ON CONFLICT (ticket_id) DO UPDATE
                     SET sort_key=EXCLUDED.sort_key, version=penfii.project_ticket_order.version+1",
                )
                .bind(ticket_id)
                .bind(project_id)
                .bind(&key)
                .execute(&mut *conn)
                .await?;
                if placement == Placement::First {
                    sqlx::query("UPDATE penfii.project_ticket_order SET moved_to_top_at=now() WHERE ticket_id=$1")
                        .bind(ticket_id)
                        .execute(&mut *conn)
                        .await?;
                }
                return Ok(true);
            }
        }
    }
    Err(CoreError::Internal(
        "order rebalance did not make room".into(),
    ))
}
