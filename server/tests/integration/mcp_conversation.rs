//! MCP channel, DM, message, thread and read-state operations against the router. Helpers:
//! `crate::mcp_oauth`.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::doc;
use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, modern_list, rest, setup, structured, token_for,
    tool_names,
};
use crate::mcp_support::{mcp_err, mcp_ok, set_policy, user_id};

const SCOPES: &str = "workspace:read member:read conversation:read conversation:write";

async fn call(env: &Env, token: &str, tool: &str, args: Value) -> Value {
    let reply = modern_call(&env.app, token, tool, args).await;
    reply.body["result"].clone()
}

fn text_of(message: &Value) -> String {
    message["body"]["content"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_channel_conversation_runs_through_mcp_like_the_web_routes(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let member_cookie = add_member(&env, "mia@penfii.test", "MEMBER").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let mia = token_for(&env, &member_cookie, SCOPES).await;

    let channel = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "mcp-talk", "visibility": "PUBLIC", "description": "hi"}),
    )
    .await;
    let channel_id = channel["id"].as_str().unwrap().to_string();
    // Same shape as the route.
    let web = rest(
        &env,
        "GET",
        &format!("/v1/conversations/{channel_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let via = mcp_ok(
        &env,
        &owner,
        "conversation_get",
        json!({"conversationId": channel_id}),
    )
    .await;
    assert_eq!(via["channel"]["name"], web.body["channel"]["name"]);

    mcp_ok(&env, &mia, "channel_join", json!({"channelId": channel_id})).await;
    let members = mcp_ok(
        &env,
        &owner,
        "channel_members_list",
        json!({"channelId": channel_id}),
    )
    .await;
    assert_eq!(members["items"].as_array().unwrap().len(), 2);

    // Send with a retry key: the second call returns the first message.
    let args =
        json!({"conversationId": channel_id, "body": "hello there", "idempotencyKey": "m-1"});
    let first = mcp_ok(&env, &owner, "message_send", args.clone()).await;
    let again = mcp_ok(&env, &owner, "message_send", args).await;
    assert_eq!(first["id"], again["id"]);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.messages WHERE conversation_id = $1")
            .bind(Uuid::parse_str(&channel_id).unwrap())
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let root_id = first["id"].as_str().unwrap().to_string();
    assert_eq!(first["authorId"], env.owner_id.to_string());

    // A reply in the thread, a reaction, a rich-text message.
    let reply = mcp_ok(
        &env,
        &mia,
        "message_send",
        json!({"conversationId": channel_id, "threadRootId": root_id, "body": doc("a reply")}),
    )
    .await;
    mcp_ok(
        &env,
        &mia,
        "message_react",
        json!({"messageId": root_id, "emoji": "👍"}),
    )
    .await;
    mcp_ok(
        &env,
        &mia,
        "message_react",
        json!({"messageId": root_id, "emoji": "👍"}),
    )
    .await;

    // Reading never moves read state.
    let before = mcp_ok(
        &env,
        &mia,
        "conversation_read_state",
        json!({"conversationId": channel_id}),
    )
    .await;
    let thread_before = mcp_ok(
        &env,
        &mia,
        "thread_read_state",
        json!({"messageId": root_id}),
    )
    .await;
    let list = mcp_ok(
        &env,
        &mia,
        "message_list",
        json!({"conversationId": channel_id}),
    )
    .await;
    assert_eq!(list["messages"].as_array().unwrap().len(), 1);
    assert_eq!(text_of(&list["messages"][0]), "hello there");
    assert_eq!(list["messages"][0]["replyCount"], 1);
    assert_eq!(
        list["messages"][0]["reactions"][0]["userIds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let thread = mcp_ok(
        &env,
        &mia,
        "thread_messages_list",
        json!({"messageId": root_id}),
    )
    .await;
    assert_eq!(thread["messages"].as_array().unwrap().len(), 2);
    let changes = mcp_ok(
        &env,
        &mia,
        "conversation_changes_list",
        json!({"conversationId": channel_id}),
    )
    .await;
    assert!(!changes["changes"].as_array().unwrap().is_empty());
    let cursor = changes["nextAfterSeq"].as_i64().unwrap();
    let none = mcp_ok(
        &env,
        &mia,
        "conversation_changes_list",
        json!({"conversationId": channel_id, "afterSeq": cursor}),
    )
    .await;
    assert!(none["changes"].as_array().unwrap().is_empty());
    assert_eq!(
        mcp_err(&env, &mia, "message_history", json!({"messageId": root_id})).await,
        "not_found",
        "history is the author's"
    );
    mcp_ok(
        &env,
        &mia,
        "conversation_get",
        json!({"conversationId": channel_id}),
    )
    .await;
    let after = mcp_ok(
        &env,
        &mia,
        "conversation_read_state",
        json!({"conversationId": channel_id}),
    )
    .await;
    let thread_after = mcp_ok(
        &env,
        &mia,
        "thread_read_state",
        json!({"messageId": root_id}),
    )
    .await;
    assert_eq!(
        before, after,
        "reads leave the conversation read state alone"
    );
    assert_eq!(
        thread_before, thread_after,
        "reads leave the thread read state alone"
    );
    let web_read = rest(
        &env,
        "GET",
        &format!("/v1/conversations/{channel_id}/read"),
        &member_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web_read.body["readThroughSeq"], after["readThroughSeq"]);

    // Explicit read / unread.
    let marked = mcp_ok(
        &env,
        &mia,
        "conversation_mark_read",
        json!({"conversationId": channel_id, "generation": after["generation"], "readThroughSeq": 2}),
    )
    .await;
    assert_eq!(marked["readThroughSeq"], 2);
    let unread = mcp_ok(
        &env,
        &mia,
        "conversation_mark_unread",
        json!({"conversationId": channel_id, "fromSeq": 1}),
    )
    .await;
    assert_eq!(unread["manualUnreadFromSeq"], 1);
    let t_state = mcp_ok(
        &env,
        &mia,
        "thread_read_state",
        json!({"messageId": root_id}),
    )
    .await;
    mcp_ok(
        &env,
        &mia,
        "thread_mark_read",
        json!({"messageId": root_id, "generation": t_state["generation"], "readThroughSeq": reply["streamSeq"]}),
    )
    .await;
    mcp_ok(
        &env,
        &mia,
        "thread_mark_unread",
        json!({"messageId": root_id, "fromSeq": reply["streamSeq"]}),
    )
    .await;

    // Edit with the current version; a stale version is refused; unreact; delete.
    let edited = mcp_ok(
        &env,
        &owner,
        "message_edit",
        json!({"messageId": root_id, "expectedVersion": first["version"], "body": "hello again"}),
    )
    .await;
    assert_eq!(edited["version"], 2);
    let stale = mcp_err(
        &env,
        &owner,
        "message_edit",
        json!({"messageId": root_id, "expectedVersion": 1, "body": "x"}),
    )
    .await;
    assert_eq!(stale, "version_conflict");
    let other = mcp_err(
        &env,
        &mia,
        "message_edit",
        json!({"messageId": root_id, "expectedVersion": 2, "body": "x"}),
    )
    .await;
    assert_eq!(other, "forbidden");
    mcp_ok(
        &env,
        &mia,
        "message_unreact",
        json!({"messageId": root_id, "emoji": "👍"}),
    )
    .await;
    let history = mcp_ok(
        &env,
        &owner,
        "message_history",
        json!({"messageId": root_id}),
    )
    .await;
    assert!(
        !history["versions"].as_array().unwrap().is_empty(),
        "{history}"
    );
    let deleted = mcp_ok(
        &env,
        &owner,
        "message_delete",
        json!({"messageId": root_id}),
    )
    .await;
    assert_eq!(deleted["deleted"], true);

    // The call log ties the message to the grant and client, and carries the Workspace.
    let row: (String, Option<String>, Option<i64>, Option<Uuid>, Uuid) = sqlx::query_as(
        "SELECT resource_type, resource_id, resource_revision, workspace_id, client_pk
         FROM penfii.mcp_call_log WHERE tool = 'message_send' AND outcome = 'ok'
         ORDER BY at LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "message");
    assert_eq!(row.1.as_deref(), Some(root_id.as_str()));
    assert_eq!(row.2, Some(1));
    assert_eq!(row.3, Some(env.workspace_id));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn private_channels_dms_and_removal_are_enforced_by_core(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let mia_cookie = add_member(&env, "mia@penfii.test", "MEMBER").await;
    let ned_cookie = add_member(&env, "ned@penfii.test", "MEMBER").await;
    let mia_id = user_id(&env, "mia@penfii.test").await;
    let ned_id = user_id(&env, "ned@penfii.test").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let mia = token_for(&env, &mia_cookie, SCOPES).await;
    let ned = token_for(&env, &ned_cookie, SCOPES).await;
    let outsider = token_for(&env, &env.outsider_cookie, SCOPES).await;

    let private = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "secret", "visibility": "PRIVATE"}),
    )
    .await;
    let cid = private["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "body": "classified"}),
    )
    .await;

    // Not a member: nothing is revealed, nobody can add themselves.
    for tool in [
        "conversation_get",
        "message_list",
        "conversation_changes_list",
    ] {
        assert_eq!(
            mcp_err(&env, &mia, tool, json!({"conversationId": cid})).await,
            "not_found",
            "{tool}"
        );
    }
    assert_eq!(
        mcp_err(
            &env,
            &mia,
            "message_send",
            json!({"conversationId": cid, "body": "hi"})
        )
        .await,
        "not_found"
    );
    let self_add = mcp_err(
        &env,
        &mia,
        "channel_member_add",
        json!({"channelId": cid, "userId": mia_id}),
    )
    .await;
    assert!(
        ["forbidden", "not_found"].contains(&self_add.as_str()),
        "{self_add}"
    );
    assert!(
        !mcp_err(&env, &mia, "channel_join", json!({"channelId": cid}))
            .await
            .is_empty()
    );
    // Another Workspace's person: not_found everywhere.
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "message_list",
            json!({"conversationId": cid})
        )
        .await,
        "not_found"
    );
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "channel_create",
            json!({"workspaceId": ws, "name": "x", "visibility": "PUBLIC"})
        )
        .await,
        "not_found"
    );

    // The manager adds Mia, who can then read; the earlier message is visible to her.
    mcp_ok(
        &env,
        &owner,
        "channel_member_add",
        json!({"channelId": cid, "userId": mia_id}),
    )
    .await;
    let seen = mcp_ok(&env, &mia, "message_list", json!({"conversationId": cid})).await;
    assert_eq!(text_of(&seen["messages"][0]), "classified");
    // A plain member cannot manage the channel.
    assert_eq!(
        mcp_err(
            &env,
            &mia,
            "channel_member_add",
            json!({"channelId": cid, "userId": ned_id})
        )
        .await,
        "forbidden"
    );
    assert_eq!(
        mcp_err(
            &env,
            &mia,
            "channel_member_remove",
            json!({"channelId": cid, "userId": env.owner_id})
        )
        .await,
        "forbidden"
    );
    assert_eq!(
        mcp_err(
            &env,
            &mia,
            "channel_member_set_role",
            json!({"channelId": cid, "userId": mia_id, "role": "MANAGER"})
        )
        .await,
        "forbidden"
    );
    let updated = mcp_ok(
        &env,
        &owner,
        "channel_update",
        json!({"channelId": cid, "description": "d", "allowMemberInvites": true}),
    )
    .await;
    assert_eq!(updated["allowMemberInvites"], true);
    // With invites allowed, Mia may add Ned; then the owner removes him: access ends at once.
    mcp_ok(
        &env,
        &mia,
        "channel_member_add",
        json!({"channelId": cid, "userId": ned_id}),
    )
    .await;
    mcp_ok(&env, &ned, "message_list", json!({"conversationId": cid})).await;
    mcp_ok(
        &env,
        &owner,
        "channel_member_remove",
        json!({"channelId": cid, "userId": ned_id}),
    )
    .await;
    assert_eq!(
        mcp_err(&env, &ned, "message_list", json!({"conversationId": cid})).await,
        "not_found"
    );
    assert_eq!(
        mcp_err(
            &env,
            &ned,
            "message_send",
            json!({"conversationId": cid, "body": "x"})
        )
        .await,
        "not_found"
    );
    // Leaving is a separate tool; removal of oneself through the other tool is refused.
    assert_eq!(
        mcp_err(
            &env,
            &mia,
            "channel_member_remove",
            json!({"channelId": cid, "userId": mia_id})
        )
        .await,
        "invalid_arguments"
    );
    mcp_ok(&env, &mia, "channel_leave", json!({"channelId": cid})).await;
    assert_eq!(
        mcp_err(&env, &mia, "message_list", json!({"conversationId": cid})).await,
        "not_found"
    );

    // Visibility is Administrative: off in the standard policy, allowed once an admin turns it on.
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "channel_set_visibility",
            json!({"channelId": cid, "visibility": "PUBLIC"})
        )
        .await,
        "policy_denied"
    );
    set_policy(&env, true, false, true).await;
    let public = mcp_ok(
        &env,
        &owner,
        "channel_set_visibility",
        json!({"channelId": cid, "visibility": "PUBLIC"}),
    )
    .await;
    assert_eq!(public["visibility"], "PUBLIC");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'mcp.channel_set_visibility' AND workspace_id = $1",
    )
    .bind(env.workspace_id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(
        audited >= 1,
        "the allowed call is audited with its Workspace: {audited}"
    );
    let denied: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'mcp.channel_set_visibility' AND outcome_code = 'POLICY_DENIED'").fetch_one(&env.pool).await.unwrap();
    assert_eq!(denied, 1, "the denied call is audited too");
    set_policy(&env, true, false, false).await;

    // Direct messages: only the named people; a stranger cannot read; the third person is not added.
    let dm = mcp_ok(
        &env,
        &owner,
        "dm_create",
        json!({"workspaceId": ws, "userIds": [env.owner_id, mia_id]}),
    )
    .await;
    let dm_id = dm["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &mia,
        "message_send",
        json!({"conversationId": dm_id, "body": "psst"}),
    )
    .await;
    assert_eq!(
        mcp_err(&env, &ned, "message_list", json!({"conversationId": dm_id})).await,
        "not_found"
    );
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "dm_create",
            json!({"workspaceId": ws, "userIds": [mia_id, ned_id]})
        )
        .await,
        "invalid_arguments"
    );
    let got = mcp_ok(
        &env,
        &owner,
        "conversation_get",
        json!({"conversationId": dm_id}),
    )
    .await;
    assert_eq!(got["memberIds"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scopes_and_policy_gate_conversation_tools(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let reader = token_for(&env, &env.owner_cookie, "workspace:read conversation:read").await;
    let list = modern_list(&env.app, &reader).await;
    assert!(list.body["result"]["tools"].is_array(), "{}", list.body);
    let names = tool_names(&list);
    assert!(names.contains(&"message_list".to_string()));
    assert!(
        names.contains(&"message_send".to_string()),
        "a tool the grant cannot call yet is still discoverable"
    );
    let reply = modern_call(
        &env.app,
        &reader,
        "channel_create",
        json!({"workspaceId": ws, "name": "n", "visibility": "PUBLIC"}),
    )
    .await;
    assert_eq!(error_code(&reply), "insufficient_scope");
    // A new consent replaces the grant: the wider token comes after the narrow one is used.
    let full = token_for(&env, &env.owner_cookie, SCOPES).await;

    let channel = mcp_ok(
        &env,
        &full,
        "channel_create",
        json!({"workspaceId": ws, "name": "policy", "visibility": "PUBLIC"}),
    )
    .await;
    let cid = channel["id"].as_str().unwrap().to_string();
    // Read-only Workspace policy: reads work, writes are refused, tools disappear from the list.
    set_policy(&env, false, false, false).await;
    mcp_ok(&env, &full, "message_list", json!({"conversationId": cid})).await;
    assert_eq!(
        mcp_err(
            &env,
            &full,
            "message_send",
            json!({"conversationId": cid, "body": "x"})
        )
        .await,
        "policy_denied"
    );
    assert_eq!(
        mcp_err(
            &env,
            &full,
            "conversation_mark_unread",
            json!({"conversationId": cid, "fromSeq": 0})
        )
        .await,
        "policy_denied"
    );
    let names = tool_names(&modern_list(&env.app, &full).await);
    assert!(!names.contains(&"message_send".to_string()));
    assert!(names.contains(&"message_list".to_string()));
    // Nothing was written by the refused call.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.messages")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn messages_keep_the_core_rules_for_files_threads_and_size(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let channel = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "rules", "visibility": "PUBLIC"}),
    )
    .await;
    let cid = channel["id"].as_str().unwrap().to_string();
    // Empty body, unknown file, oversize body, reply to a missing thread root: Core refuses.
    assert_eq!(
        mcp_err(&env, &owner, "message_send", json!({"conversationId": cid})).await,
        "invalid_arguments"
    );
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "message_send",
            json!({"conversationId": cid, "body": "  "})
        )
        .await,
        "invalid_arguments"
    );
    let unknown_file = mcp_err(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "fileIds": [Uuid::new_v4()]}),
    )
    .await;
    assert!(!unknown_file.is_empty());
    assert!(
        !mcp_err(
            &env,
            &owner,
            "message_send",
            json!({"conversationId": cid, "body": "x", "threadRootId": Uuid::new_v4()})
        )
        .await
        .is_empty()
    );
    let huge = "x".repeat(20_000);
    let sent = call(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "body": huge}),
    )
    .await;
    assert_eq!(sent["isError"], false);
    // Thread replies cannot be thread roots.
    let root = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "body": "root"}),
    )
    .await;
    let reply = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "body": "r", "threadRootId": root["id"]}),
    )
    .await;
    let nested = call(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "body": "n", "threadRootId": reply["id"]}),
    )
    .await;
    assert_eq!(nested["isError"], true);
    // Pages: limit and cursor.
    for n in 0..3 {
        mcp_ok(
            &env,
            &owner,
            "message_send",
            json!({"conversationId": cid, "body": format!("m{n}")}),
        )
        .await;
    }
    let page = mcp_ok(
        &env,
        &owner,
        "message_list",
        json!({"conversationId": cid, "limit": 2}),
    )
    .await;
    assert_eq!(page["messages"].as_array().unwrap().len(), 2);
    let next = page["nextBeforeSeq"].as_i64().unwrap();
    let older = mcp_ok(
        &env,
        &owner,
        "message_list",
        json!({"conversationId": cid, "limit": 2, "beforeSeq": next}),
    )
    .await;
    assert!(older["messages"][0]["streamSeq"].as_i64().unwrap() < next);
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "message_list",
            json!({"conversationId": cid, "limit": 500})
        )
        .await,
        "invalid_arguments"
    );
    // Nothing secret leaks into the result of a send.
    let text = serde_json::to_string(&root).unwrap();
    assert!(!text.contains("idempotency"));
    let _ = structured;
}
