//! How far the event consumers are behind, for `/readyz` and the operator.
//!
//! The numbers come from `consumer_receipts`, which is never emptied, so they are not read per request:
//! a background pass computes them every [`REFRESH`] and keeps the last answer ([`ReceiptHealth`]). The
//! age of the oldest waiting receipt uses the partial index of the receipts that still have work to do;
//! the count of `DEAD` ones reads the table once per pass (a delivery gives up after twelve attempts, and
//! an operator brings it back with `admin-cli events-retry`).

use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::error::CoreError;

/// How often the numbers are computed.
pub const REFRESH: Duration = Duration::from_secs(60);
/// A receipt that is due and has waited longer than this makes the instance `degraded`.
pub const STALE_AFTER_SECONDS: i64 = 300;

/// The oldest receipt of one consumer that is due and not yet done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerWait {
    pub consumer: String,
    pub oldest_waiting_seconds: i64,
}

/// One reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub taken_at: DateTime<Utc>,
    /// Consumers that have a due receipt, oldest first.
    pub consumers: Vec<ConsumerWait>,
    /// Receipts a consumer gave up on (`DEAD`).
    pub dead: i64,
}

impl Snapshot {
    pub fn oldest_waiting_seconds(&self) -> Option<i64> {
        self.consumers
            .iter()
            .map(|c| c.oldest_waiting_seconds)
            .max()
    }

    /// Something is stuck or given up on: a due receipt is older than [`STALE_AFTER_SECONDS`], or a
    /// receipt is `DEAD`.
    pub fn degraded(&self) -> bool {
        self.dead > 0
            || self
                .oldest_waiting_seconds()
                .is_some_and(|age| age > STALE_AFTER_SECONDS)
    }
}

/// The last reading, shared between the pass that makes it and `/readyz`.
#[derive(Default)]
pub struct ReceiptHealth {
    last: RwLock<Option<Snapshot>>,
}

impl ReceiptHealth {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The last reading; `None` before the first pass has finished.
    pub fn current(&self) -> Option<Snapshot> {
        self.last.read().ok().and_then(|last| last.clone())
    }

    /// Reads the receipts and keeps the answer.
    pub async fn refresh(&self, pool: &PgPool) -> Result<Snapshot, CoreError> {
        let snapshot = read(pool).await?;
        if let Ok(mut last) = self.last.write() {
            *last = Some(snapshot.clone());
        }
        Ok(snapshot)
    }
}

/// Reads the receipts once.
pub async fn read(pool: &PgPool) -> Result<Snapshot, CoreError> {
    // Due and not done: PENDING past `available_at`, RUNNING past its lease. The partial index of the
    // receipts that have work to do serves this.
    let rows = sqlx::query(
        "SELECT consumer,
                extract(epoch FROM now() - min(CASE WHEN state = 'PENDING' THEN available_at ELSE lease_until END))::bigint AS waited
         FROM penfii.consumer_receipts
         WHERE (state = 'PENDING' AND available_at <= now()) OR (state = 'RUNNING' AND lease_until <= now())
         GROUP BY consumer
         ORDER BY waited DESC, consumer",
    )
    .fetch_all(pool)
    .await?;
    let mut consumers = Vec::with_capacity(rows.len());
    for row in &rows {
        consumers.push(ConsumerWait {
            consumer: row.try_get("consumer")?,
            oldest_waiting_seconds: row.try_get::<Option<i64>, _>("waited")?.unwrap_or(0).max(0),
        });
    }
    let dead: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.consumer_receipts WHERE state = 'DEAD'")
            .fetch_one(pool)
            .await?;
    Ok(Snapshot {
        taken_at: Utc::now(),
        consumers,
        dead,
    })
}

/// Computes the reading every [`REFRESH`] for the life of the process.
pub fn spawn(pool: PgPool, health: Arc<ReceiptHealth>) -> tokio::task::JoinHandle<()> {
    crate::lifecycle::spawn_worker("receipt health", async move {
        while !crate::lifecycle::stopping() {
            if let Err(error) =
                crate::lifecycle::guarded("receipt health", health.refresh(&pool)).await
            {
                tracing::warn!(?error, "receipt health could not be read");
            }
            crate::lifecycle::pause(REFRESH).await;
        }
    })
}
