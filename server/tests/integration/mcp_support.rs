//! Helpers shared by the MCP tool tests (`mcp_*.rs`). The OAuth side (setup, consent, tokens, raw
//! calls) is in `mcp_oauth`.

use serde_json::Value;
use uuid::Uuid;

use crate::mcp_oauth::{Env, error_code, modern_call, structured};

/// Calls a tool that must succeed and returns its `structuredContent`.
pub(crate) async fn mcp_ok(env: &Env, token: &str, tool: &str, args: Value) -> Value {
    let reply = modern_call(&env.app, token, tool, args).await;
    assert_eq!(
        reply.body["result"]["isError"], false,
        "{tool}: {} {:?}",
        reply.status, reply.body
    );
    structured(&reply).clone()
}

/// Calls a tool that must be refused and returns the error code.
pub(crate) async fn mcp_err(env: &Env, token: &str, tool: &str, args: Value) -> String {
    let reply = modern_call(&env.app, token, tool, args).await;
    assert_eq!(
        reply.body["result"]["isError"], true,
        "{tool} should fail: {:?}",
        reply.body
    );
    error_code(&reply).to_string()
}

/// Stores the Workspace MCP policy directly (no web session needed); the web route that changes it
/// is tested in `mcp_oauth`.
pub(crate) async fn set_policy(env: &Env, write: bool, destructive: bool, administrative: bool) {
    sqlx::query(
        "INSERT INTO penfii.workspace_mcp_policy
            (workspace_id, mcp_enabled, allow_write, allow_destructive, allow_administrative, updated_by, updated_at)
         VALUES ($1, true, $2, $3, $4, $5, now())
         ON CONFLICT (workspace_id) DO UPDATE SET allow_write = $2, allow_destructive = $3,
                                                  allow_administrative = $4, updated_at = now()",
    )
    .bind(env.workspace_id)
    .bind(write)
    .bind(destructive)
    .bind(administrative)
    .bind(env.owner_id)
    .execute(&env.pool)
    .await
    .unwrap();
}

/// The id of the account with this e-mail.
pub(crate) async fn user_id(env: &Env, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = lower($1)")
        .bind(email)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}
