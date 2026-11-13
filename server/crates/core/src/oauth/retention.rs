//! Retention of MCP and OAuth records. Nothing here touches `audit_entries`,
//! `activity_entries` or `ticket_revisions`: those follow their own policy, and this module only
//! removes what the OAuth server and the MCP gateway themselves accumulate.
//!
//! | Data | Kept |
//! |---|---|
//! | `mcp_call_log`, read operations | 30 days (technical log) |
//! | `mcp_call_log`, write / destructive / administrative | 365 days (security audit) |
//! | Authorization requests, codes | 1 day after expiry |
//! | Access / refresh tokens | 7 days after expiry or revocation (replay detection window) |
//! | Revoked grants | 365 days after revocation, once no token or code refers to them |
//! | Unused DCR clients | 7 days |

use std::time::Duration;

use serde::Serialize;
use sqlx::PgPool;

use crate::error::CoreError;

#[derive(Debug, Clone, Copy)]
pub struct Retention {
    pub read_log_days: i64,
    pub change_log_days: i64,
    pub grant_days: i64,
}

impl Retention {
    /// The days kept: not set or `0` is the default, a positive number is taken, anything else is an error.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(&crate::config::process_env)
    }

    pub fn from_lookup(lookup: crate::config::Lookup<'_>) -> Result<Self, String> {
        let days = |name: &str, default: i64| -> Result<i64, String> {
            Ok(crate::config::integer(lookup, name, 0)?
                .filter(|value| *value > 0)
                .unwrap_or(default))
        };
        Ok(Self {
            read_log_days: days("PENFII_MCP_LOG_RETENTION_DAYS", 30)?,
            change_log_days: days("PENFII_MCP_AUDIT_LOG_RETENTION_DAYS", 365)?,
            grant_days: days("PENFII_MCP_GRANT_RETENTION_DAYS", 365)?,
        })
    }

    /// The days kept when nothing is set.
    pub fn defaults() -> Self {
        Self {
            read_log_days: 30,
            change_log_days: 365,
            grant_days: 365,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub call_log: u64,
    pub authorization_requests: u64,
    pub codes: u64,
    pub tokens: u64,
    pub grants: u64,
    pub clients: u64,
}

impl PurgeReport {
    pub fn total(&self) -> u64 {
        self.call_log
            + self.authorization_requests
            + self.codes
            + self.tokens
            + self.grants
            + self.clients
    }
}

/// One pass. Order matters: tokens and codes go before the grants they point at, and grants
/// before the clients they point at.
pub async fn purge(pool: &PgPool, retention: Retention) -> Result<PurgeReport, CoreError> {
    let call_log = sqlx::query(
        "DELETE FROM penfii.mcp_call_log
         WHERE at < now() - CASE WHEN risk_class = 'read'
                                 THEN make_interval(days => $1::int)
                                 ELSE make_interval(days => $2::int) END",
    )
    .bind(retention.read_log_days as i32)
    .bind(retention.change_log_days as i32)
    .execute(pool)
    .await?
    .rows_affected();
    let authorization_requests = sqlx::query(
        "DELETE FROM penfii.oauth_authorization_requests WHERE expires_at < now() - interval '1 day'",
    )
    .execute(pool)
    .await?
    .rows_affected();
    let codes =
        sqlx::query("DELETE FROM penfii.oauth_codes WHERE expires_at < now() - interval '1 day'")
            .execute(pool)
            .await?
            .rows_affected();
    let tokens = sqlx::query(
        "DELETE FROM penfii.oauth_tokens
         WHERE LEAST(expires_at, COALESCE(revoked_at, expires_at)) < now() - interval '7 days'",
    )
    .execute(pool)
    .await?
    .rows_affected();
    let grants = sqlx::query(
        "DELETE FROM penfii.oauth_grants g
         WHERE g.revoked_at < now() - make_interval(days => $1::int)
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_tokens t WHERE t.grant_id = g.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_codes c WHERE c.grant_id = g.id)",
    )
    .bind(retention.grant_days as i32)
    .execute(pool)
    .await?
    .rows_affected();
    let clients = sqlx::query(
        "DELETE FROM penfii.oauth_clients c WHERE c.kind = 'dcr' AND c.fetched_at < now() - interval '7 days'
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_grants g WHERE g.client_pk = c.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_authorization_requests r WHERE r.client_pk = c.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_codes k WHERE k.client_pk = c.id)",
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(PurgeReport {
        call_log,
        authorization_requests,
        codes,
        tokens,
        grants,
        clients,
    })
}

/// Hourly background pass. A failure is logged and retried on the next tick.
pub fn spawn(pool: PgPool) -> tokio::task::JoinHandle<()> {
    crate::lifecycle::spawn_worker("mcp retention", async move {
        // Checked when the process started (`config::startup_checks`), so this reads what was accepted there.
        let retention = Retention::from_env().unwrap_or_else(|error| {
            tracing::error!(%error, "mcp retention settings: using the defaults");
            Retention::defaults()
        });
        while !crate::lifecycle::stopping() {
            match crate::lifecycle::guarded("mcp retention", purge(&pool, retention)).await {
                Ok(report) if report.total() > 0 => {
                    tracing::info!(?report, "mcp retention purge");
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(?error, "mcp retention purge failed"),
            }
            crate::lifecycle::pause(Duration::from_secs(3600)).await;
        }
    })
}
