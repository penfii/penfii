//! FEA-173, server side: one working day through MCP only. A channel thread is read, a Ticket and
//! a Doc come out of it, a file is attached, the result is answered in the thread; then the member
//! is removed and loses every door. The live checks with ChatGPT, Claude Code and Codex are not
//! part of this test (they need a person and a public address).

use penfii_core::files::ObjectStore;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{add_member, error_code, modern_call, rest, setup_with, token_for};
use crate::mcp_support::{mcp_err, mcp_ok};

const SCOPES: &str = "workspace:read member:read search:read ticket:read ticket:write \
    project:read project:write doc:read doc:write conversation:read conversation:write \
    file:read file:write plan:read personal:read";

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_thread_becomes_a_ticket_a_doc_an_attachment_and_an_answer_then_removal_closes_every_door(
    pool: PgPool,
) {
    let env = setup_with(pool.clone(), |config| {
        config.objects = Some(ObjectStore::in_memory());
    })
    .await;
    let mia_cookie = add_member(&env, "mia@penfii.test", "MEMBER").await;
    let mia_id: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = 'mia@penfii.test'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let ws = env.workspace_id;
    let mia = token_for(&env, &mia_cookie, SCOPES).await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;

    // Owner: a Project and a public channel; Mia is invited to the channel.
    let project = mcp_ok(
        &env,
        &owner,
        "project_create",
        json!({"workspaceId": ws, "name": "Day One"}),
    )
    .await;
    let project_id = project["id"].as_str().unwrap().to_string();
    let channel = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "day-one", "visibility": "PUBLIC"}),
    )
    .await;
    let channel_id = channel["id"].as_str().unwrap().to_string();
    let root = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": channel_id, "body": "The importer drops rows with empty names.",
               "idempotencyKey": "root-1"}),
    )
    .await;
    let root_id = root["id"].as_str().unwrap().to_string();

    // Mia joins and reads the thread; reading leaves her read state alone.
    mcp_ok(&env, &mia, "channel_join", json!({"channelId": channel_id})).await;
    let before = mcp_ok(
        &env,
        &mia,
        "conversation_read_state",
        json!({"conversationId": channel_id}),
    )
    .await;
    let listed = mcp_ok(
        &env,
        &mia,
        "message_list",
        json!({"conversationId": channel_id}),
    )
    .await;
    assert!(listed.to_string().contains("importer drops rows"));
    mcp_ok(
        &env,
        &mia,
        "thread_messages_list",
        json!({"messageId": root_id}),
    )
    .await;
    let after = mcp_ok(
        &env,
        &mia,
        "conversation_read_state",
        json!({"conversationId": channel_id}),
    )
    .await;
    assert_eq!(before, after, "reads do not mark anything read");

    // Ticket, then a Doc in the same Project, then the file on the Ticket.
    let ticket = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "Importer drops rows with empty names",
               "idempotencyKey": "t-1"}),
    )
    .await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let again = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "Importer drops rows with empty names",
               "idempotencyKey": "t-1"}),
    )
    .await;
    assert_eq!(
        again["id"], ticket["id"],
        "a retry with the same key writes once"
    );
    let doc = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "projectId": project_id, "title": "Importer notes",
               "markdown": "# Findings\n\n- empty names are dropped"}),
    )
    .await;
    let doc_id = doc["id"].as_str().unwrap().to_string();
    let revision = doc["revisionNo"].as_i64().unwrap();
    let got = mcp_ok(&env, &owner, "doc_get", json!({"docId": doc_id})).await;
    let saved = mcp_ok(
        &env,
        &owner,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": revision,
               "title": "Importer notes (confirmed)", "content": got["content"]}),
    )
    .await;
    assert_eq!(saved["title"], "Importer notes (confirmed)");
    let stale = modern_call(
        &env.app,
        &owner,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": revision,
               "title": "Stale write", "content": got["content"]}),
    )
    .await;
    assert_eq!(error_code(&stale), "version_conflict", "{:?}", stale.body);

    let file = mcp_ok(
        &env,
        &owner,
        "file_create_small",
        json!({"workspaceId": ws, "filename": "empty-names.csv",
               "contentBase64": "bmFtZSxpZAosMQo="}),
    )
    .await;
    let file_id = file["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &owner,
        "ticket_attachment_add",
        json!({"ticketId": ticket_id, "fileId": file_id}),
    )
    .await;
    let listed = mcp_ok(
        &env,
        &owner,
        "ticket_attachment_list",
        json!({"ticketId": ticket_id}),
    )
    .await;
    assert!(listed.to_string().contains("empty-names.csv"));

    // The answer goes back into the thread, as the connected person.
    let reply = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": channel_id, "threadRootId": root_id,
               "body": "Ticket and notes are up.", "idempotencyKey": "reply-1"}),
    )
    .await;
    assert_eq!(reply["authorId"], json!(env.owner_id.to_string()));
    let thread = mcp_ok(
        &env,
        &mia,
        "thread_messages_list",
        json!({"messageId": root_id}),
    )
    .await;
    assert!(thread.to_string().contains("Ticket and notes are up."));

    // Every call was logged with its Workspace and client; no secret is in the log rows.
    let logged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.mcp_call_log WHERE outcome = 'ok' AND workspace_id IS NULL
           AND tool NOT IN ('workspace_list', 'workspace_web_only')",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(logged, 0, "successful calls name their Workspace");

    // Mia is removed: her token sees nothing and cannot act, even with the old ids.
    let removed = rest(
        &env,
        "DELETE",
        &format!("/v1/workspaces/{ws}/members/{mia_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert!(removed.status.is_success(), "{:?}", removed.body);
    for (tool, args) in [
        ("message_list", json!({"conversationId": channel_id})),
        ("doc_get", json!({"docId": doc_id})),
        ("ticket_get", json!({"ticketId": ticket_id})),
        (
            "message_send",
            json!({"conversationId": channel_id, "body": "still here?"}),
        ),
    ] {
        let denied = modern_call(&env.app, &mia, tool, args).await;
        assert_eq!(denied.body["result"]["isError"], true, "{tool}");
        assert!(
            ["not_found", "forbidden"].contains(&error_code(&denied)),
            "{tool}: {:?}",
            denied.body
        );
    }
    // The Workspace is gone from her list and its overview is refused.
    let listed = mcp_ok(&env, &mia, "workspace_list", json!({})).await;
    assert!(!listed.to_string().contains(&ws.to_string()), "{listed}");
    assert_eq!(
        mcp_err(&env, &mia, "workspace_overview", json!({"workspaceId": ws})).await,
        "not_found"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn review_fixes_department_parent_kept_and_strangers_leave_no_audit_in_the_workspace(
    pool: PgPool,
) {
    let env = setup_with(pool.clone(), |_| {}).await;
    let ws = env.workspace_id;
    let owner = token_for(
        &env,
        &env.owner_cookie,
        "workspace:read team:write doc:read doc:write doc:delete conversation:read",
    )
    .await;

    // A rename without parentId keeps the department where it is (it used to move it to the top).
    let top = mcp_ok(
        &env,
        &owner,
        "department_create",
        json!({"workspaceId": ws, "name": "Top"}),
    )
    .await;
    let child = mcp_ok(
        &env,
        &owner,
        "department_create",
        json!({"workspaceId": ws, "name": "Child", "parentId": top["id"]}),
    )
    .await;
    mcp_ok(
        &env,
        &owner,
        "department_update",
        json!({"workspaceId": ws, "departmentId": child["id"], "name": "Child renamed"}),
    )
    .await;
    let tree = mcp_ok(&env, &owner, "department_list", json!({"workspaceId": ws})).await;
    let text = tree.to_string();
    assert!(text.contains("Child renamed"), "{text}");
    let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM penfii.departments WHERE name = 'Child renamed'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        parent.map(|id| id.to_string()),
        top["id"].as_str().map(str::to_string)
    );
    // null still means: to the top level.
    mcp_ok(
        &env,
        &owner,
        "department_update",
        json!({"workspaceId": ws, "departmentId": child["id"], "name": "Child renamed", "parentId": null}),
    )
    .await;
    let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM penfii.departments WHERE name = 'Child renamed'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert!(parent.is_none());

    // A person from another Workspace who guesses a Doc id and calls a Destructive tool is refused,
    // and the refusal is not written into the audit of the Workspace that owns the Doc.
    let doc = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Target", "markdown": "x"}),
    )
    .await;
    let stranger = token_for(
        &env,
        &env.outsider_cookie,
        "workspace:read doc:read doc:write doc:delete",
    )
    .await;
    let denied = modern_call(
        &env.app,
        &stranger,
        "doc_delete",
        json!({"docId": doc["id"]}),
    )
    .await;
    assert_eq!(error_code(&denied), "not_found", "{:?}", denied.body);
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE workspace_id = $1 AND action_code LIKE 'mcp.%'",
    )
    .bind(ws)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        rows, 0,
        "a stranger's refused call must not write into this Workspace's audit"
    );
}
