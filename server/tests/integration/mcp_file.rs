//! MCP file operations against the router. Helpers: `crate::mcp_oauth`.

use base64::Engine;
use penfii_core::files::ObjectStore;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, modern_list, rest, setup, setup_with, structured,
    token_for, tool_names,
};
use crate::mcp_support::set_policy;

const SCOPES: &str = "workspace:read file:read file:write ticket:read ticket:write";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn project_and_ticket(env: &Env, token: &str) -> (String, String) {
    let project = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"name": "Files", "teamIds": []})),
    )
    .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let ticket = modern_call(
        &env.app,
        token,
        "ticket_create",
        json!({"projectId": project_id, "title": "Has a file"}),
    )
    .await;
    assert_eq!(ticket.body["result"]["isError"], false, "{:?}", ticket.body);
    (
        project_id,
        structured(&ticket)["id"].as_str().unwrap().to_string(),
    )
}

fn ok(reply: &crate::common::Reply) -> &Value {
    assert_eq!(reply.body["result"]["isError"], false, "{:?}", reply.body);
    structured(reply)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn small_upload_attach_download_round_trip_matches_the_web(pool: PgPool) {
    let env = setup_with(pool, |config| {
        config.objects = Some(ObjectStore::in_memory())
    })
    .await;
    let token = token_for(&env, &env.owner_cookie, SCOPES).await;
    let (_project, ticket) = project_and_ticket(&env, &token).await;
    let content = b"hello from an MCP client\n";

    let up = modern_call(
        &env.app,
        &token,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "note.txt", "contentBase64": b64(content)}),
    )
    .await;
    let file = ok(&up).clone();
    assert_eq!(file["state"], "AVAILABLE", "{file}");
    assert_eq!(file["byteSize"], content.len());
    let text = file.to_string();
    assert!(
        !text.contains("storage") && !text.contains("workspaces/") && !text.contains("http"),
        "no storage key or URL: {text}"
    );
    let file_id = file["id"].as_str().unwrap().to_string();

    let attach = modern_call(
        &env.app,
        &token,
        "ticket_attachment_add",
        json!({"ticketId": ticket, "fileId": file_id}),
    )
    .await;
    let attached = ok(&attach).clone();
    let attachment_id = attached["id"].as_str().unwrap().to_string();
    // A retry is the same attachment, not a second one.
    let again = modern_call(
        &env.app,
        &token,
        "ticket_attachment_add",
        json!({"ticketId": ticket, "fileId": file_id}),
    )
    .await;
    assert_eq!(ok(&again)["id"], attachment_id.as_str());

    // The list is what the web route lists.
    let listed = modern_call(
        &env.app,
        &token,
        "ticket_attachment_list",
        json!({"ticketId": ticket}),
    )
    .await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket}/attachments"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&listed)["attachments"], web.body);
    assert_eq!(web.body.as_array().unwrap().len(), 1);

    // Download through MCP returns the bytes the web download returns.
    let read = modern_call(
        &env.app,
        &token,
        "attachment_download",
        json!({"attachmentId": attachment_id}),
    )
    .await;
    let got = ok(&read);
    assert_eq!(got["filename"], "note.txt");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(got["contentBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes, content);
    assert!(!got.to_string().contains("workspaces/"), "no storage key");

    // The audit rows carry the Workspace although the calls name a Ticket / attachment.
    let logged: Vec<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT operation, workspace_id FROM penfii.mcp_call_log
         WHERE operation IN ('ticket_attachment.list','ticket_attachment.add','attachment.download','file.create_small')",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert!(logged.len() >= 4);
    assert!(
        logged.iter().all(|(_, ws)| *ws == Some(env.workspace_id)),
        "{logged:?}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn size_encoding_and_storage_limits_are_enforced(pool: PgPool) {
    let env = setup_with(pool, |config| {
        config.objects = Some(ObjectStore::in_memory())
    })
    .await;
    let token = token_for(&env, &env.owner_cookie, SCOPES).await;
    let call = |args: Value| modern_call(&env.app, &token, "file_create_small", args);

    let too_big = vec![7u8; 256 * 1024 + 1];
    let reply = call(json!({"workspaceId": env.workspace_id, "filename": "big.bin", "contentBase64": b64(&too_big)})).await;
    assert_eq!(
        error_code(&reply),
        "file_too_large",
        "{:?} {}",
        reply.body,
        reply.status
    );
    // Far beyond the cap the request is refused before it is even parsed (HTTP 413) or by the tool.
    let huge = "A".repeat(2_000_000);
    let reply = call(
        json!({"workspaceId": env.workspace_id, "filename": "big.bin", "contentBase64": huge}),
    )
    .await;
    assert!(
        reply.status.as_u16() == 413 || error_code(&reply) == "file_too_large",
        "{:?} {}",
        reply.body,
        reply.status
    );
    let near = "A".repeat(400_000);
    let reply = call(
        json!({"workspaceId": env.workspace_id, "filename": "big.bin", "contentBase64": near}),
    )
    .await;
    assert_eq!(
        error_code(&reply),
        "file_too_large",
        "{:?} {}",
        reply.body,
        reply.status
    );
    let reply =
        call(json!({"workspaceId": env.workspace_id, "filename": "x.txt", "contentBase64": "***"}))
            .await;
    assert_eq!(error_code(&reply), "invalid_arguments");
    let reply =
        call(json!({"workspaceId": env.workspace_id, "filename": "x.txt", "contentBase64": ""}))
            .await;
    assert_eq!(error_code(&reply), "invalid_arguments");
    let exact = vec![b'a'; 256 * 1024];
    let reply = call(json!({"workspaceId": env.workspace_id, "filename": "exact.txt", "contentBase64": b64(&exact)})).await;
    ok(&reply);
    // Nothing half-made is left from the refused ones.
    let stuck: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.files WHERE state = 'UPLOADING'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(stuck, 0);

    // A file over a larger limit than MCP carries is read back with a pointer to the web app.
    let big_id = {
        let created = rest(
            &env,
            "POST",
            &format!("/v1/workspaces/{}/files", env.workspace_id),
            &env.owner_cookie,
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"filename": "x.txt", "byteSize": 5})),
        )
        .await;
        created.body["id"].as_str().unwrap().to_string()
    };
    // A pending upload is not readable content.
    let pending = modern_call(&env.app, &token, "file_download", json!({"fileId": big_id})).await;
    assert_eq!(error_code(&pending), "not_found");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn without_object_storage_the_tool_says_so(pool: PgPool) {
    let env = setup(pool).await;
    let token = token_for(&env, &env.owner_cookie, SCOPES).await;
    let reply = modern_call(
        &env.app,
        &token,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "a.txt", "contentBase64": b64(b"abc")}),
    )
    .await;
    assert_eq!(
        error_code(&reply),
        "storage_unconfigured",
        "{:?}",
        reply.body
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn deleting_and_detaching_are_destructive_and_follow_the_core_rules(pool: PgPool) {
    let env = setup_with(pool, |config| {
        config.objects = Some(ObjectStore::in_memory())
    })
    .await;
    let token = token_for(&env, &env.owner_cookie, SCOPES).await;
    let (_project, ticket) = project_and_ticket(&env, &token).await;
    let upload = |name: &'static str| {
        let (app, token, ws) = (env.app.clone(), token.clone(), env.workspace_id);
        async move {
            let reply = modern_call(
                &app,
                &token,
                "file_create_small",
                json!({"workspaceId": ws, "filename": name, "contentBase64": b64(b"data")}),
            )
            .await;
            structured(&reply)["id"].as_str().unwrap().to_string()
        }
    };
    let attached_file = upload("kept.txt").await;
    let loose_file = upload("loose.txt").await;
    let attach = modern_call(
        &env.app,
        &token,
        "ticket_attachment_add",
        json!({"ticketId": ticket, "fileId": attached_file}),
    )
    .await;
    let attachment_id = ok(&attach)["id"].as_str().unwrap().to_string();

    // Destructive is off by default in a Workspace: refused, and hidden from tools/list.
    let denied = modern_call(
        &env.app,
        &token,
        "file_delete",
        json!({"fileId": loose_file}),
    )
    .await;
    assert_eq!(error_code(&denied), "policy_denied");
    let list = modern_list(&env.app, &token).await;
    let names = tool_names(&list);
    assert!(names.contains(&"file_create_small".to_string()));
    assert!(!names.contains(&"file_delete".to_string()));
    assert!(!names.contains(&"ticket_attachment_remove".to_string()));

    set_policy(&env, true, true, false).await;
    let list = modern_list(&env.app, &token).await;
    assert!(tool_names(&list).contains(&"file_delete".to_string()));
    assert!(crate::mcp_oauth::tool_destructive(&list, "file_delete"));

    let attached_delete = modern_call(
        &env.app,
        &token,
        "file_delete",
        json!({"fileId": attached_file}),
    )
    .await;
    assert_eq!(error_code(&attached_delete), "file_attached");
    let deleted = modern_call(
        &env.app,
        &token,
        "file_delete",
        json!({"fileId": loose_file}),
    )
    .await;
    assert_eq!(ok(&deleted)["deleted"], true);
    // Deleting again is harmless.
    let gone = modern_call(
        &env.app,
        &token,
        "file_delete",
        json!({"fileId": loose_file}),
    )
    .await;
    assert_eq!(gone.body["result"]["isError"], false, "{:?}", gone.body);

    let removed = modern_call(
        &env.app,
        &token,
        "ticket_attachment_remove",
        json!({"ticketId": ticket, "attachmentId": attachment_id}),
    )
    .await;
    assert_eq!(ok(&removed)["removed"], true);
    let listed = modern_call(
        &env.app,
        &token,
        "ticket_attachment_list",
        json!({"ticketId": ticket}),
    )
    .await;
    assert!(ok(&listed)["attachments"].as_array().unwrap().is_empty());
    // Destructive calls are also in the Workspace audit.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE workspace_id = $1 AND action_code LIKE 'mcp.file_delete'",
    )
    .bind(env.workspace_id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(audited >= 2, "{audited}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scopes_policy_membership_and_other_peoples_files_are_checked(pool: PgPool) {
    let env = setup_with(pool, |config| {
        config.objects = Some(ObjectStore::in_memory())
    })
    .await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let (_project, ticket) = project_and_ticket(&env, &owner).await;
    let created = modern_call(
        &env.app,
        &owner,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "mine.txt", "contentBase64": b64(b"private")}),
    )
    .await;
    let file_id = ok(&created)["id"].as_str().unwrap().to_string();

    // A read-only grant also lists the write tools, and is refused with the scope it lacks.
    let reader = token_for(&env, &env.owner_cookie, "workspace:read file:read").await;
    let names = tool_names(&modern_list(&env.app, &reader).await);
    assert!(names.contains(&"file_download".to_string()));
    assert!(names.contains(&"file_create_small".to_string()));
    let refused = modern_call(
        &env.app,
        &reader,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "a.txt", "contentBase64": b64(b"abc")}),
    )
    .await;
    assert_eq!(error_code(&refused), "insufficient_scope");
    assert_eq!(refused.status.as_u16(), 403);

    // Another person's pending upload is not readable, whoever asks.
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, SCOPES).await;
    let peek = modern_call(
        &env.app,
        &member,
        "file_download",
        json!({"fileId": file_id}),
    )
    .await;
    assert_eq!(error_code(&peek), "not_found");
    let drop_it = modern_call(&env.app, &member, "file_delete", json!({"fileId": file_id})).await;
    assert!(matches!(
        error_code(&drop_it),
        "not_found" | "policy_denied"
    ));
    // The member's own upload works, but the owner's private project's Ticket is not theirs.
    let own = modern_call(
        &env.app,
        &member,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "m.txt", "contentBase64": b64(b"m")}),
    )
    .await;
    let own_id = ok(&own)["id"].as_str().unwrap().to_string();
    let listed = modern_call(
        &env.app,
        &member,
        "ticket_attachment_list",
        json!({"ticketId": ticket}),
    )
    .await;
    assert_eq!(error_code(&listed), "not_found");
    let attach = modern_call(
        &env.app,
        &member,
        "ticket_attachment_add",
        json!({"ticketId": ticket, "fileId": own_id}),
    )
    .await;
    assert_eq!(error_code(&attach), "not_found");

    // An outsider of this Workspace gets nothing, and creates nothing.
    let outsider = token_for(&env, &env.outsider_cookie, SCOPES).await;
    let out = modern_call(
        &env.app,
        &outsider,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "o.txt", "contentBase64": b64(b"o")}),
    )
    .await;
    assert_eq!(out.body["result"]["isError"], true);
    let out = modern_call(
        &env.app,
        &outsider,
        "ticket_attachment_list",
        json!({"ticketId": ticket}),
    )
    .await;
    assert_eq!(out.body["result"]["isError"], true);

    // A member removed from the Workspace loses the tools at once.
    sqlx::query("UPDATE penfii.workspace_members SET state = 'INACTIVE' WHERE workspace_id = $1 AND role = 'MEMBER'")
        .bind(env.workspace_id)
        .execute(&env.pool)
        .await
        .unwrap();
    let after = modern_call(
        &env.app,
        &member,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "late.txt", "contentBase64": b64(b"x")}),
    )
    .await;
    assert_eq!(after.body["result"]["isError"], true, "{:?}", after.body);

    // Write access turned off by the Workspace: refused and hidden. (A new consent replaced the
    // owner's grant when the read-only token was made, so the owner connects again.)
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    set_policy(&env, false, false, false).await;
    let blocked = modern_call(
        &env.app,
        &owner,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "p.txt", "contentBase64": b64(b"p")}),
    )
    .await;
    assert_eq!(
        error_code(&blocked),
        "policy_denied",
        "{:?} {}",
        blocked.body,
        blocked.status
    );
    // Reads stay available under a no-write policy.
    let still = modern_call(
        &env.app,
        &owner,
        "file_download",
        json!({"fileId": file_id}),
    )
    .await;
    assert_eq!(still.body["result"]["isError"], false, "{:?}", still.body);
}
