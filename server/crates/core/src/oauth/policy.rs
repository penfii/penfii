//! Workspace MCP Policy: a per-Workspace feature switch by risk
//! class. It can only remove ability; it has no per-user, role or resource entries.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::identity::AuthSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Read,
    Write,
    Destructive,
    Administrative,
}

impl RiskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
            Self::Administrative => "administrative",
        }
    }
}

/// What a Workspace without a stored row gets. Chosen by `PENFII_MCP_DEFAULT_POLICY`:
/// `standard` (default: on, writes allowed, destructive and administrative off), `read_only`,
/// or `closed` (an administrator has to opt in).
#[derive(Debug, Clone, Copy)]
pub struct PolicyDefaults {
    pub mcp_enabled: bool,
    pub allow_write: bool,
    pub allow_destructive: bool,
    pub allow_administrative: bool,
}

impl PolicyDefaults {
    pub fn standard() -> Self {
        Self {
            mcp_enabled: true,
            allow_write: true,
            allow_destructive: false,
            allow_administrative: false,
        }
    }

    /// `PENFII_MCP_DEFAULT_POLICY`: `standard`, `read_only` or `closed`; not set means `standard`. Any other
    /// word is an error: a typo must not turn into the policy that allows the most.
    pub fn from_lookup(lookup: crate::config::Lookup<'_>) -> Result<Self, String> {
        Ok(
            match crate::config::choice(
                lookup,
                "PENFII_MCP_DEFAULT_POLICY",
                &["standard", "read_only", "closed"],
            )? {
                Some("read_only") => Self {
                    allow_write: false,
                    ..Self::standard()
                },
                Some("closed") => Self {
                    mcp_enabled: false,
                    allow_write: false,
                    ..Self::standard()
                },
                _ => Self::standard(),
            },
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPolicy {
    pub workspace_id: Uuid,
    pub mcp_enabled: bool,
    pub allow_write: bool,
    pub allow_destructive: bool,
    pub allow_administrative: bool,
    /// True while no row is stored and the instance defaults apply.
    pub is_default: bool,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyInput {
    pub mcp_enabled: bool,
    pub allow_write: bool,
    pub allow_destructive: bool,
    pub allow_administrative: bool,
}

impl McpPolicy {
    /// The class → flag map.
    pub fn allows(&self, risk: RiskClass) -> bool {
        self.mcp_enabled
            && match risk {
                RiskClass::Read => true,
                RiskClass::Write => self.allow_write,
                RiskClass::Destructive => self.allow_destructive,
                RiskClass::Administrative => self.allow_administrative,
            }
    }
}

/// The policy that applies now. No membership check: callers that reveal it do that themselves.
pub async fn effective(
    pool: &PgPool,
    defaults: PolicyDefaults,
    workspace_id: Uuid,
) -> Result<McpPolicy, CoreError> {
    let row = sqlx::query(
        "SELECT mcp_enabled, allow_write, allow_destructive, allow_administrative, updated_at
         FROM penfii.workspace_mcp_policy WHERE workspace_id = $1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(row) => McpPolicy {
            workspace_id,
            mcp_enabled: row.try_get("mcp_enabled")?,
            allow_write: row.try_get("allow_write")?,
            allow_destructive: row.try_get("allow_destructive")?,
            allow_administrative: row.try_get("allow_administrative")?,
            is_default: false,
            updated_at: row.try_get("updated_at")?,
        },
        None => McpPolicy {
            workspace_id,
            mcp_enabled: defaults.mcp_enabled,
            allow_write: defaults.allow_write,
            allow_destructive: defaults.allow_destructive,
            allow_administrative: defaults.allow_administrative,
            is_default: true,
            updated_at: None,
        },
    })
}

/// Active members may read the policy so they can tell why something is refused.
pub async fn get(
    pool: &PgPool,
    defaults: PolicyDefaults,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<McpPolicy, CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, user_id).await?;
    drop(conn);
    effective(pool, defaults, workspace_id).await
}

/// OWNER or ADMIN only, recent authentication required, audited.
pub async fn set(
    pool: &PgPool,
    defaults: PolicyDefaults,
    session: &AuthSession,
    workspace_id: Uuid,
    input: PolicyInput,
) -> Result<McpPolicy, CoreError> {
    let mut tx = pool.begin().await?;
    let role = access::require_member(&mut tx, workspace_id, session.user.id).await?;
    if role != "OWNER" && role != "ADMIN" {
        return Err(CoreError::forbidden(
            "only a Workspace owner or admin can change the MCP policy",
        ));
    }
    if !crate::identity::recently_authenticated(session) {
        // The ways to authenticate again are read on the pool: let go of this connection first.
        drop(tx);
        return Err(crate::identity::reauth_required(pool, session).await);
    }
    sqlx::query(
        "INSERT INTO penfii.workspace_mcp_policy
            (workspace_id, mcp_enabled, allow_write, allow_destructive, allow_administrative,
             updated_by, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (workspace_id) DO UPDATE SET
            mcp_enabled = EXCLUDED.mcp_enabled, allow_write = EXCLUDED.allow_write,
            allow_destructive = EXCLUDED.allow_destructive,
            allow_administrative = EXCLUDED.allow_administrative,
            updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(workspace_id)
    .bind(input.mcp_enabled)
    .bind(input.allow_write)
    .bind(input.allow_destructive)
    .bind(input.allow_administrative)
    .bind(session.user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, $2, $3, 'workspace.mcp_policy_changed', 'OK', 'workspace', $2, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(workspace_id)
    .bind(session.user.id)
    .bind(serde_json::json!({
        "mcpEnabled": input.mcp_enabled, "allowWrite": input.allow_write,
        "allowDestructive": input.allow_destructive,
        "allowAdministrative": input.allow_administrative,
    }))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    effective(pool, defaults, workspace_id).await
}

/// One attribution row: ids and outcome, never payloads.
pub struct CallLog<'a> {
    pub user_id: Uuid,
    pub grant_id: Uuid,
    pub client_pk: Uuid,
    pub tool: &'a str,
    pub operation: &'a str,
    pub risk: RiskClass,
    pub workspace_id: Option<Uuid>,
    pub resource_type: Option<&'a str>,
    pub resource_id: Option<String>,
    pub resource_revision: Option<i64>,
    pub request_id: &'a str,
    pub outcome: &'a str,
    pub duration_ms: i32,
}

pub async fn log_call(pool: &PgPool, entry: CallLog<'_>) {
    let result = sqlx::query(
        "INSERT INTO penfii.mcp_call_log
            (id, user_id, grant_id, client_pk, tool, operation, risk_class, workspace_id,
             resource_type, resource_id, resource_revision, request_id, outcome, duration_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(Uuid::new_v4())
    .bind(entry.user_id)
    .bind(entry.grant_id)
    .bind(entry.client_pk)
    .bind(entry.tool)
    .bind(entry.operation)
    .bind(entry.risk.as_str())
    .bind(entry.workspace_id)
    .bind(entry.resource_type)
    .bind(entry.resource_id)
    .bind(entry.resource_revision)
    .bind(entry.request_id)
    .bind(entry.outcome)
    .bind(entry.duration_ms)
    .execute(pool)
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "mcp call log write failed");
    }
}

/// An existing Audit row (`audit_entries`) for something done through OAuth / MCP: grants made
/// and revoked, and every destructive or administrative tool call. Never a payload.
pub struct AuditRow<'a> {
    pub user_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub action_code: &'a str,
    pub outcome_code: &'a str,
    pub target_kind: &'a str,
    pub target_id: Option<Uuid>,
    pub data: serde_json::Value,
}

pub async fn audit(pool: &PgPool, row: AuditRow<'_>) {
    let result = sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(row.workspace_id)
    .bind(row.user_id)
    .bind(row.action_code)
    .bind(row.outcome_code)
    .bind(row.target_kind)
    .bind(row.target_id)
    .bind(row.data)
    .execute(pool)
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "mcp audit write failed");
    }
}
