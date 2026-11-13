//! Event pipeline: `domain_events` + `outbox` + a small dispatcher.
//!
//! A command writes its event and outbox row in its own transaction (`emit`). The dispatcher
//! fans an outbox row out to one receipt per interested consumer. Each consumer claims its own
//! receipts, so a slow or failing consumer never holds back another one. Delivery is at least
//! once: every consumer must give the same result when it sees an event again.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use uuid::Uuid;

use crate::error::CoreError;

pub type HandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + 'a>>;

/// An event as a consumer reads it.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub event_id: Uuid,
    pub seq: i64,
    pub workspace_id: Uuid,
    pub target_kind: String,
    pub target_id: Uuid,
    pub target_version: Option<i64>,
    pub kind: String,
    pub actor_user_id: Option<Uuid>,
    pub command_id: Uuid,
    pub facts: Value,
    pub security_context: Value,
    pub occurred_at: DateTime<Utc>,
}

impl EventRow {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            event_id: row.try_get("event_id")?,
            seq: row.try_get("seq")?,
            workspace_id: row.try_get("workspace_id")?,
            target_kind: row.try_get("target_kind")?,
            target_id: row.try_get("target_id")?,
            target_version: row.try_get("target_version")?,
            kind: row.try_get("event_kind")?,
            actor_user_id: row.try_get("actor_user_id")?,
            command_id: row.try_get("command_id")?,
            facts: row.try_get("facts")?,
            security_context: row.try_get("security_context")?,
            occurred_at: row.try_get("occurred_at")?,
        })
    }
}

/// What a command records. `actor_user_id: None` writes a SYSTEM event (for example a Reminder).
pub struct NewEvent<'a> {
    pub workspace_id: Uuid,
    pub target_kind: &'a str,
    pub target_id: Uuid,
    pub target_version: Option<i64>,
    pub kind: &'a str,
    pub actor_user_id: Option<Uuid>,
    pub command_id: Uuid,
    pub facts: Value,
    pub security_context: Value,
}

/// Writes the event and its outbox row. The caller owns the transaction.
pub async fn emit(conn: &mut PgConnection, event: NewEvent<'_>) -> Result<Uuid, CoreError> {
    let actor_kind = if event.actor_user_id.is_some() {
        "USER"
    } else {
        "SYSTEM"
    };
    emit_with_kind(conn, event, actor_kind).await
}

/// `emit` with an explicit `actor_kind`. Human commands stay on [`emit`] (`USER` / `SYSTEM`).
/// Incoming webhooks record `INTEGRATION`.
pub async fn emit_with_kind(
    conn: &mut PgConnection,
    event: NewEvent<'_>,
    actor_kind: &str,
) -> Result<Uuid, CoreError> {
    let event_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.domain_events(event_id,workspace_id,target_kind,target_id,target_version,event_kind,actor_kind,actor_user_id,initiated_by_user_id,command_id,schema_version,facts,security_context)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$8,$9,1,$10,$11)",
    )
    .bind(event_id)
    .bind(event.workspace_id)
    .bind(event.target_kind)
    .bind(event.target_id)
    .bind(event.target_version)
    .bind(event.kind)
    .bind(actor_kind)
    .bind(event.actor_user_id)
    .bind(event.command_id)
    .bind(event.facts)
    .bind(event.security_context)
    .execute(&mut *conn)
    .await?;
    sqlx::query!("INSERT INTO penfii.outbox(event_id) VALUES($1)", event_id)
        .execute(&mut *conn)
        .await?;
    Ok(event_id)
}

/// `emit` for events that have no version and carry only a few facts.
pub async fn emit_simple(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    target_kind: &str,
    target_id: Uuid,
    kind: &str,
    actor_user_id: Option<Uuid>,
    facts: Value,
) -> Result<Uuid, CoreError> {
    emit(
        conn,
        NewEvent {
            workspace_id,
            target_kind,
            target_id,
            target_version: None,
            kind,
            actor_user_id,
            command_id: Uuid::new_v4(),
            facts,
            security_context: Value::Object(Default::default()),
        },
    )
    .await
}

/// A consumer whose whole effect is rows in PostgreSQL. The effect and the receipt's DONE are
/// committed together, so a crash redoes both and a duplicate is impossible.
pub trait PgConsumer: Send + Sync {
    fn name(&self) -> &'static str;
    fn accepts(&self, event_kind: &str) -> bool;
    fn handle<'a>(&'a self, conn: &'a mut PgConnection, event: &'a EventRow) -> HandlerFuture<'a>;
    /// Called after a batch was committed, for work that must not happen inside the transaction
    /// (telling a connected client to look again).
    fn committed(&self) {}
}

/// A consumer with an effect outside PostgreSQL (the search index). It runs without holding a
/// transaction and must be idempotent.
pub trait ExternalConsumer: Send + Sync {
    fn name(&self) -> &'static str;
    fn accepts(&self, event_kind: &str) -> bool;
    fn handle<'a>(&'a self, pool: &'a PgPool, event: &'a EventRow) -> HandlerFuture<'a>;
}

#[derive(Clone)]
pub struct Registry {
    pub pg: Vec<Arc<dyn PgConsumer>>,
    pub external: Vec<Arc<dyn ExternalConsumer>>,
    /// First retry delay; it doubles up to `max_backoff_secs`.
    pub backoff_base_secs: i64,
    pub max_backoff_secs: i64,
    pub max_attempts: i32,
    pub lease_secs: i64,
    pub batch: i64,
    /// Restricts every step to one Workspace. Tests use it so parallel tests do not take each
    /// other's events; production leaves it empty.
    pub workspace: Option<Uuid>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            pg: Vec::new(),
            external: Vec::new(),
            backoff_base_secs: 5,
            max_backoff_secs: 900,
            max_attempts: 12,
            lease_secs: 60,
            batch: 50,
            workspace: None,
        }
    }
}

impl Registry {
    fn interested(&self, event_kind: &str) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .pg
            .iter()
            .filter(|c| c.accepts(event_kind))
            .map(|c| c.name())
            .collect();
        names.extend(
            self.external
                .iter()
                .filter(|c| c.accepts(event_kind))
                .map(|c| c.name()),
        );
        names
    }

    fn backoff(&self, attempts: i32) -> i64 {
        let exp = u32::try_from(attempts.saturating_sub(1).clamp(0, 20)).unwrap_or(0);
        self.backoff_base_secs
            .saturating_mul(1i64 << exp)
            .min(self.max_backoff_secs)
    }
}

/// Turns undispatched outbox rows into receipts. A crash before the commit leaves the rows
/// undispatched, so the step can simply run again.
pub async fn fan_out(pool: &PgPool, registry: &Registry, limit: i64) -> Result<usize, CoreError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT o.event_id, e.event_kind
         FROM penfii.outbox o JOIN penfii.domain_events e ON e.event_id=o.event_id
         WHERE o.dispatched_at IS NULL AND o.available_at <= now()
           AND ($2::uuid IS NULL OR e.workspace_id = $2)
         ORDER BY e.seq
         FOR UPDATE OF o SKIP LOCKED
         LIMIT $1",
    )
    .bind(limit)
    .bind(registry.workspace)
    .fetch_all(&mut *tx)
    .await?;
    for row in &rows {
        let event_id: Uuid = row.try_get("event_id")?;
        let kind: String = row.try_get("event_kind")?;
        for name in registry.interested(&kind) {
            sqlx::query(
                "INSERT INTO penfii.consumer_receipts(event_id, consumer) VALUES($1,$2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(event_id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query!(
            "UPDATE penfii.outbox SET dispatched_at=now() WHERE event_id=$1",
            event_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

const EVENT_COLUMNS: &str = "e.event_id,e.seq,e.workspace_id,e.target_kind,e.target_id,e.target_version,e.event_kind,e.actor_user_id,e.command_id,e.facts,e.security_context,e.occurred_at";

/// What a consumer's panic is recorded as: an `Internal` error whose text starts with this, and whose code
/// is `panic`. The consumer's event then follows the ordinary failure path (attempts, backoff, `DEAD`).
const PANIC_PREFIX: &str = "panic: ";

fn panic_error(message: &str) -> CoreError {
    CoreError::Internal(format!("{PANIC_PREFIX}{message}"))
}

fn code_of(error: &CoreError) -> String {
    match error {
        CoreError::Internal(message) if message.starts_with(PANIC_PREFIX) => "panic".into(),
        CoreError::NotFound => "not_found".into(),
        CoreError::Unauthenticated => "unauthenticated".into(),
        CoreError::Validation(_) => "validation".into(),
        CoreError::Forbidden(_) => "forbidden".into(),
        CoreError::Conflict { code, .. } | CoreError::Rule { code, .. } => (*code).into(),
        CoreError::ReauthRequired { .. } => "reauth_required".into(),
        CoreError::RateLimited { .. } => "rate_limited".into(),
        CoreError::DbUnavailable(_) => "db_unavailable".into(),
        CoreError::Internal(_) => "internal".into(),
    }
}

/// The failure of a PostgreSQL consumer's event. The receipt row is locked by the transaction that
/// handles it (`FOR UPDATE SKIP LOCKED` in [`run_pg_consumer`]), so nobody else can change it
/// meanwhile and no lease token is needed; the attempt is counted here, when it failed (a rolled
/// back transaction leaves no claim behind).
async fn record_pg_failure(
    conn: &mut PgConnection,
    registry: &Registry,
    event_id: Uuid,
    consumer: &str,
    error: &CoreError,
) -> Result<(), CoreError> {
    let current: i32 = sqlx::query_scalar(
        "SELECT attempts FROM penfii.consumer_receipts WHERE event_id=$1 AND consumer=$2",
    )
    .bind(event_id)
    .bind(consumer)
    .fetch_one(&mut *conn)
    .await?;
    let attempts = current + 1;
    let dead = attempts >= registry.max_attempts;
    // Only a code is stored. A payload could carry text the reader may not see.
    tracing::warn!(consumer, %event_id, attempts, code = %code_of(error), "event consumer failed");
    sqlx::query(
        "UPDATE penfii.consumer_receipts
         SET state=$3, attempts=$4, last_error_code=$5, lease_token=NULL, lease_until=NULL,
             available_at = now() + make_interval(secs => $6::double precision), updated_at=now()
         WHERE event_id=$1 AND consumer=$2",
    )
    .bind(event_id)
    .bind(consumer)
    .bind(if dead { "DEAD" } else { "PENDING" })
    .bind(attempts)
    .bind(code_of(error))
    .bind(registry.backoff(attempts) as f64)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Runs one batch for a PostgreSQL consumer. Returns how many receipts were handled.
pub async fn run_pg_consumer(
    pool: &PgPool,
    registry: &Registry,
    consumer: &Arc<dyn PgConsumer>,
) -> Result<usize, CoreError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM penfii.consumer_receipts r JOIN penfii.domain_events e ON e.event_id=r.event_id
         WHERE r.consumer=$1 AND r.state='PENDING' AND r.available_at <= now()
           AND ($3::uuid IS NULL OR e.workspace_id = $3)
         ORDER BY e.seq
         FOR UPDATE OF r SKIP LOCKED
         LIMIT $2"
    ))
    .bind(consumer.name())
    .bind(registry.batch)
    .bind(registry.workspace)
    .fetch_all(&mut *tx)
    .await?;
    let mut handled = 0;
    for row in &rows {
        let event = EventRow::from_row(row)?;
        let mut savepoint = sqlx::Connection::begin(&mut *tx).await?;
        // A panic in the consumer is this event's failure, like an error: the savepoint is rolled back and the
        // attempt is counted, so one event that always panics ends as DEAD instead of taking the worker down.
        let result = match crate::lifecycle::contained(consumer.handle(&mut savepoint, &event))
            .await
        {
            Ok(result) => result,
            Err(message) => {
                tracing::error!(consumer = consumer.name(), event_id = %event.event_id, %message, "event consumer panicked");
                Err(panic_error(&message))
            }
        };
        match result {
            Ok(()) => {
                savepoint.commit().await?;
                sqlx::query(
                    "UPDATE penfii.consumer_receipts SET state='DONE', last_error_code=NULL, updated_at=now() WHERE event_id=$1 AND consumer=$2",
                )
                .bind(event.event_id)
                .bind(consumer.name())
                .execute(&mut *tx)
                .await?;
            }
            Err(error) => {
                savepoint.rollback().await?;
                record_pg_failure(&mut tx, registry, event.event_id, consumer.name(), &error)
                    .await?;
            }
        }
        handled += 1;
    }
    tx.commit().await?;
    if handled > 0 {
        consumer.committed();
    }
    Ok(handled)
}

/// Runs one batch for an external consumer using a lease instead of a long transaction.
pub async fn run_external_consumer(
    pool: &PgPool,
    registry: &Registry,
    consumer: &Arc<dyn ExternalConsumer>,
) -> Result<usize, CoreError> {
    let token = Uuid::new_v4();
    // `attempts` counts claims: every claim of a receipt, the first and a re-claim after its lease
    // ran out, adds one, in the same statement that takes the lease. A receipt that has already been
    // claimed `max_attempts` times and is claimed again (its worker never reported back, again and
    // again) is not handed out: it becomes DEAD, so a worker that keeps dying cannot go on forever.
    let claimed = sqlx::query(&format!(
        "WITH picked AS (
            SELECT r.event_id FROM penfii.consumer_receipts r
            JOIN penfii.domain_events ev ON ev.event_id=r.event_id
            WHERE r.consumer=$1
              AND ((r.state='PENDING' AND r.available_at <= now())
                   OR (r.state='RUNNING' AND r.lease_until <= now()))
              AND ($5::uuid IS NULL OR ev.workspace_id = $5)
            ORDER BY r.event_id
            FOR UPDATE SKIP LOCKED
            LIMIT $2
         ),
         marked AS (
            UPDATE penfii.consumer_receipts r
            SET state = CASE WHEN r.attempts >= $6 THEN 'DEAD' ELSE 'RUNNING' END,
                attempts = CASE WHEN r.attempts >= $6 THEN r.attempts ELSE r.attempts + 1 END,
                lease_token = CASE WHEN r.attempts >= $6 THEN NULL ELSE $3 END,
                lease_until = CASE WHEN r.attempts >= $6 THEN NULL
                                   ELSE now() + make_interval(secs => $4::double precision) END,
                last_error_code = CASE WHEN r.attempts >= $6 THEN 'lease_expired' ELSE r.last_error_code END,
                updated_at = now()
            FROM picked WHERE r.event_id=picked.event_id AND r.consumer=$1
            RETURNING r.event_id, r.state
         )
         SELECT {EVENT_COLUMNS} FROM marked m JOIN penfii.domain_events e ON e.event_id=m.event_id
         WHERE m.state = 'RUNNING' ORDER BY e.seq"
    ))
    .bind(consumer.name())
    .bind(registry.batch)
    .bind(token)
    .bind(registry.lease_secs as f64)
    .bind(registry.workspace)
    .bind(registry.max_attempts)
    .fetch_all(pool)
    .await?;
    let mut handled = 0;
    for row in &claimed {
        let event = EventRow::from_row(row)?;
        let result = match crate::lifecycle::contained(consumer.handle(pool, &event)).await {
            Ok(result) => result,
            Err(message) => {
                tracing::error!(consumer = consumer.name(), event_id = %event.event_id, %message, "event consumer panicked");
                Err(panic_error(&message))
            }
        };
        // Both outcomes are reported under the lease token this worker was given: when the lease ran
        // out and another worker claimed the receipt, this report changes nothing.
        let applied = match result {
            Ok(()) => {
                sqlx::query(
                    "UPDATE penfii.consumer_receipts SET state='DONE', last_error_code=NULL, lease_token=NULL, lease_until=NULL, updated_at=now()
                     WHERE event_id=$1 AND consumer=$2 AND lease_token=$3 AND state='RUNNING'",
                )
                .bind(event.event_id)
                .bind(consumer.name())
                .bind(token)
                .execute(pool)
                .await?
                .rows_affected()
                    == 1
            }
            Err(error) => {
                fail_claimed(pool, registry, event.event_id, consumer.name(), token, &error).await?
            }
        };
        if !applied {
            tracing::warn!(
                consumer = consumer.name(),
                event_id = %event.event_id,
                "stale consumer result ignored: the lease was taken over by another worker"
            );
        }
        handled += 1;
    }
    Ok(handled)
}

/// The failure of an external consumer's event, reported by the worker that holds the lease `token`.
/// One statement: the attempts were counted when the receipt was claimed, so this only decides what
/// the receipt becomes (`DEAD` once `max_attempts` claims are used up, else `PENDING` after the
/// backoff of that attempt count) and releases the lease. `false` when the lease is no longer this
/// worker's (another worker claimed the receipt, or an operator reset it): nothing is changed.
async fn fail_claimed(
    pool: &PgPool,
    registry: &Registry,
    event_id: Uuid,
    consumer: &str,
    token: Uuid,
    error: &CoreError,
) -> Result<bool, CoreError> {
    // Only a code is stored. A payload could carry text the reader may not see.
    let code = code_of(error);
    let updated = sqlx::query(
        "UPDATE penfii.consumer_receipts
         SET state = CASE WHEN attempts >= $4 THEN 'DEAD' ELSE 'PENDING' END,
             last_error_code = $5, lease_token = NULL, lease_until = NULL,
             available_at = now() + make_interval(secs =>
                 LEAST($7::double precision,
                       $6::double precision * power(2, LEAST(GREATEST(attempts - 1, 0), 20)))),
             updated_at = now()
         WHERE event_id = $1 AND consumer = $2 AND lease_token = $3 AND state = 'RUNNING'
         RETURNING attempts, state",
    )
    .bind(event_id)
    .bind(consumer)
    .bind(token)
    .bind(registry.max_attempts)
    .bind(&code)
    .bind(registry.backoff_base_secs as f64)
    .bind(registry.max_backoff_secs as f64)
    .fetch_optional(pool)
    .await?;
    let Some(row) = updated else {
        return Ok(false);
    };
    let attempts: i32 = row.try_get("attempts")?;
    let state: String = row.try_get("state")?;
    tracing::warn!(consumer, %event_id, attempts, %state, code = %code, "event consumer failed");
    Ok(true)
}

/// A delivery that failed too often and was given up on.
#[derive(Debug)]
pub struct DeadReceipt {
    pub event_id: Uuid,
    pub consumer: String,
    pub event_kind: String,
    pub attempts: i32,
    pub last_error_code: Option<String>,
}

pub async fn dead_receipts(pool: &PgPool, limit: i64) -> Result<Vec<DeadReceipt>, CoreError> {
    Ok(sqlx::query_as!(
        DeadReceipt,
        "SELECT r.event_id, r.consumer, e.event_kind, r.attempts, r.last_error_code
         FROM penfii.consumer_receipts r JOIN penfii.domain_events e ON e.event_id=r.event_id
         WHERE r.state='DEAD' ORDER BY e.seq LIMIT $1",
        limit
    )
    .fetch_all(pool)
    .await?)
}

/// Gives dead deliveries a fresh set of attempts. Only `DEAD` receipts change, so a delivery that
/// is pending, running or done is never touched. Consumers see the event again exactly as they
/// would after a crash, which they already have to tolerate.
pub async fn requeue_dead(
    pool: &PgPool,
    consumer: Option<&str>,
    event_id: Option<Uuid>,
) -> Result<u64, CoreError> {
    let done = sqlx::query(
        "UPDATE penfii.consumer_receipts
         SET state='PENDING', attempts=0, available_at=now(), lease_token=NULL, lease_until=NULL,
             last_error_code=NULL, updated_at=now()
         WHERE state='DEAD' AND ($1::text IS NULL OR consumer=$1) AND ($2::uuid IS NULL OR event_id=$2)",
    )
    .bind(consumer)
    .bind(event_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Fans out and runs every consumer until nothing is left to do. Tests and the admin CLI use it;
/// the background tasks use the same functions on a timer.
pub async fn run_until_idle(pool: &PgPool, registry: &Registry) -> Result<usize, CoreError> {
    let mut total = 0;
    for _ in 0..1000 {
        let mut round = fan_out(pool, registry, 100).await?;
        for consumer in &registry.pg {
            round += run_pg_consumer(pool, registry, consumer).await?;
        }
        for consumer in &registry.external {
            round += run_external_consumer(pool, registry, consumer).await?;
        }
        if round == 0 {
            break;
        }
        total += round;
    }
    Ok(total)
}

/// Starts one background task for the fan-out and one per consumer. Consumers are independent
/// tasks, which is what keeps a failing consumer from delaying the others.
pub fn spawn(pool: PgPool, registry: Registry) -> Vec<tokio::task::JoinHandle<()>> {
    let registry = Arc::new(registry);
    let mut handles = Vec::new();
    let idle = Duration::from_millis(500);
    {
        let (pool, registry) = (pool.clone(), registry.clone());
        handles.push(crate::lifecycle::spawn_worker(
            "event fan-out",
            async move {
                while !crate::lifecycle::stopping() {
                    match fan_out(&pool, &registry, 100).await {
                        Ok(0) => crate::lifecycle::pause(idle).await,
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(?error, "event fan-out failed");
                            crate::lifecycle::pause(Duration::from_secs(2)).await;
                        }
                    }
                }
            },
        ));
    }
    for consumer in registry.pg.clone() {
        let (pool, registry) = (pool.clone(), registry.clone());
        handles.push(crate::lifecycle::spawn_worker(
            "event consumer",
            async move {
                while !crate::lifecycle::stopping() {
                    match run_pg_consumer(&pool, &registry, &consumer).await {
                        Ok(0) => crate::lifecycle::pause(idle).await,
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(
                                ?error,
                                consumer = consumer.name(),
                                "consumer batch failed"
                            );
                            crate::lifecycle::pause(Duration::from_secs(2)).await;
                        }
                    }
                }
            },
        ));
    }
    for consumer in registry.external.clone() {
        let (pool, registry) = (pool.clone(), registry.clone());
        handles.push(crate::lifecycle::spawn_worker(
            "external event consumer",
            async move {
                while !crate::lifecycle::stopping() {
                    match run_external_consumer(&pool, &registry, &consumer).await {
                        Ok(0) => crate::lifecycle::pause(idle).await,
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(
                                ?error,
                                consumer = consumer.name(),
                                "consumer batch failed"
                            );
                            crate::lifecycle::pause(Duration::from_secs(2)).await;
                        }
                    }
                }
            },
        ));
    }
    handles
}
