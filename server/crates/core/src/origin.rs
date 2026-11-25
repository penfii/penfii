//! Where a change came from, when it did not come from the web app.
//!
//! An MCP tool call runs Core commands as the connected person, so the rows those commands write
//! (Ticket revisions, Activity, domain events, audit entries) carry that person as the actor, as
//! they would for the web. What tells them apart is the command id: inside an MCP call every row
//! records the call's id (`mcp_call_log.id`) as its `command_id`, and an audit entry also says
//! `via: "mcp"` with the grant and client in its data. Outside one, a command keeps the fresh id
//! it made for itself.
//!
//! The link is exact: a row has the id of the one call whose transaction wrote it. Each Ticket of
//! a bulk change, each of several concurrent calls, and a retry that writes again each get the call
//! that wrote them; a failed command commits nothing, and an idempotent replay writes nothing, so
//! neither can be credited to a call. Only ids are stored, never a token or a payload.

use serde_json::{Value, json};
use uuid::Uuid;

/// The MCP call a command runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpCall {
    /// `mcp_call_log.id` of the call, written before the call ends.
    pub call_id: Uuid,
    pub grant_id: Uuid,
    pub client_pk: Uuid,
}

tokio::task_local! {
    static MCP_CALL: McpCall;
}

/// Runs `work` as part of `call`: every command it runs records the call as its origin.
pub async fn within_mcp_call<F: Future>(call: McpCall, work: F) -> F::Output {
    MCP_CALL.scope(call, work).await
}

/// The MCP call the current task runs in, if any.
pub fn mcp_call() -> Option<McpCall> {
    MCP_CALL.try_with(|call| *call).ok()
}

/// The `command_id` a row records: the MCP call's id inside one, else the command's own id.
pub fn command_id(own: Uuid) -> Uuid {
    mcp_call().map_or(own, |call| call.call_id)
}

/// An audit entry's data with where the action came from. Inside an MCP call an object gains
/// `via: "mcp"`, `grantId`, `clientPk` and `mcpCallId` (keys already set are kept); anything else,
/// and everything outside a call, is returned as it is (the entry's `command_id` still names the call).
pub fn audit_data(mut data: Value) -> Value {
    let Some(call) = mcp_call() else {
        return data;
    };
    if let Value::Object(map) = &mut data {
        for (key, value) in [
            ("via", json!("mcp")),
            ("grantId", json!(call.grant_id)),
            ("clientPk", json!(call.client_pk)),
            ("mcpCallId", json!(call.call_id)),
        ] {
            map.entry(key).or_insert(value);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> McpCall {
        McpCall {
            call_id: Uuid::new_v4(),
            grant_id: Uuid::new_v4(),
            client_pk: Uuid::new_v4(),
        }
    }

    #[tokio::test]
    async fn outside_a_call_nothing_changes() {
        let own = Uuid::new_v4();
        assert_eq!(command_id(own), own);
        assert_eq!(audit_data(json!({"email": "a@b"})), json!({"email": "a@b"}));
        assert_eq!(mcp_call(), None);
    }

    #[tokio::test]
    async fn inside_a_call_rows_carry_the_call() {
        let call = call();
        within_mcp_call(call, async move {
            assert_eq!(command_id(Uuid::new_v4()), call.call_id);
            let data = audit_data(json!({"email": "a@b", "via": "admin-cli"}));
            assert_eq!(data["email"], "a@b");
            assert_eq!(data["via"], "admin-cli", "a key Core set itself is kept");
            assert_eq!(data["grantId"], json!(call.grant_id));
            assert_eq!(data["clientPk"], json!(call.client_pk));
            assert_eq!(data["mcpCallId"], json!(call.call_id));
            assert_eq!(audit_data(json!(["kept"])), json!(["kept"]));
        })
        .await;
    }

    #[tokio::test]
    async fn concurrent_calls_keep_their_own_origin() {
        let (a, b) = (call(), call());
        let (seen_a, seen_b) = tokio::join!(
            within_mcp_call(a, async {
                tokio::task::yield_now().await;
                command_id(Uuid::new_v4())
            }),
            within_mcp_call(b, async {
                tokio::task::yield_now().await;
                command_id(Uuid::new_v4())
            }),
        );
        assert_eq!((seen_a, seen_b), (a.call_id, b.call_id));
    }
}
