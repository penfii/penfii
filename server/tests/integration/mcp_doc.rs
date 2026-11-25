//! MCP doc operations against the router. Helpers: `crate::mcp_oauth` (`setup`, `setup_with`, `token_for`,
//! `modern_call`, `structured`, `error_code`, `rest`, `add_member`).

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, modern_list, rest, setup, structured, token_for,
    tool_names,
};

const DOC_SCOPES: &str = "workspace:read member:read search:read ticket:read doc:read doc:write";

fn ok(reply: &crate::common::Reply) -> bool {
    reply.body["result"]["isError"] == false
}

async fn call(env: &Env, token: &str, tool: &str, args: Value) -> crate::common::Reply {
    modern_call(&env.app, token, tool, args).await
}

async fn user_id(env: &Env, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = $1")
        .bind(email)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

fn text_doc(text: &str) -> Value {
    json!({"schema": "penfii.doc", "version": 1, "blocks": [
        {"id": "b1", "type": "paragraph", "props": {}, "children": [],
         "content": [{"type": "text", "text": text, "styles": {}}]}]})
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn doc_tools_are_ports_of_the_routes(pool: PgPool) {
    let env = setup(pool).await;
    let owner = token_for(&env, &env.owner_cookie, DOC_SCOPES).await;
    let ws = env.workspace_id.to_string();

    // Create from Markdown: the page is what the web route would have made from the same blocks.
    let created = call(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Plan", "visibility": "WORKSPACE",
               "markdown": "# Goal\n\n- one\n- two\n\nplain | table | row"}),
    )
    .await;
    assert!(ok(&created), "{:?}", created.body);
    let doc_id = structured(&created)["id"].as_str().unwrap().to_string();
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{doc_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.status, StatusCode::OK);
    let kinds: Vec<&str> = web.body["content"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["heading", "bulletListItem", "bulletListItem", "paragraph"]
    );
    assert!(
        web.body["content"]
            .to_string()
            .contains("plain | table | row")
    );

    // Reads are the routes' answers and change nothing.
    let got = call(&env, &owner, "doc_get", json!({"docId": doc_id})).await;
    assert!(ok(&got), "{:?}", got.body);
    let revision = structured(&got)["revisionNo"].as_i64().unwrap();
    assert_eq!(structured(&got)["content"], web.body["content"]);
    let again = call(&env, &owner, "doc_get", json!({"docId": doc_id})).await;
    assert_eq!(structured(&again)["revisionNo"], revision);
    let tree_web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/docs/tree"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let tree = call(&env, &owner, "doc_tree", json!({"workspaceId": ws})).await;
    assert_eq!(structured(&tree), &tree_web.body);

    // Save needs the current revision; a stale one names currentRevision and writes nothing.
    let saved = call(
        &env,
        &owner,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": revision, "title": "Plan v2", "content": text_doc("second")}),
    )
    .await;
    assert!(ok(&saved), "{:?}", saved.body);
    let new_revision = structured(&saved)["revisionNo"].as_i64().unwrap();
    assert!(new_revision > revision);
    let stale = call(
        &env,
        &owner,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": revision, "content": text_doc("lost")}),
    )
    .await;
    assert_eq!(error_code(&stale), "version_conflict");
    assert!(
        stale.body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("currentRevision"),
        "{:?}",
        stale.body
    );
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{doc_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.body["title"], "Plan v2");
    assert!(web.body["content"].to_string().contains("second"));
    // content that is not a document is refused, nothing is written.
    let bad = call(
        &env,
        &owner,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": new_revision, "content": {"blocks": []}}),
    )
    .await;
    assert!(!ok(&bad));

    // Versions are frozen; restore saves an old Version as a new revision.
    let v1 = call(
        &env,
        &owner,
        "doc_version_create",
        json!({"docId": doc_id, "expectedRevision": new_revision}),
    )
    .await;
    assert!(ok(&v1), "{:?}", v1.body);
    let dup = call(
        &env,
        &owner,
        "doc_version_create",
        json!({"docId": doc_id, "expectedRevision": new_revision}),
    )
    .await;
    assert!(!ok(&dup), "a duplicate Version is refused");
    let list = call(&env, &owner, "doc_version_list", json!({"docId": doc_id})).await;
    assert_eq!(structured(&list)["items"].as_array().unwrap().len(), 1);
    let version = call(
        &env,
        &owner,
        "doc_version_get",
        json!({"docId": doc_id, "versionNo": 1}),
    )
    .await;
    assert!(
        structured(&version)["content"]
            .to_string()
            .contains("second")
    );
    let third = call(
        &env,
        &owner,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": new_revision, "content": text_doc("third")}),
    )
    .await;
    let third_revision = structured(&third)["revisionNo"].as_i64().unwrap();
    let restored = call(
        &env,
        &owner,
        "doc_version_restore",
        json!({"docId": doc_id, "versionNo": 1, "expectedRevision": third_revision}),
    )
    .await;
    assert!(ok(&restored), "{:?}", restored.body);
    assert!(
        structured(&restored)["content"]
            .to_string()
            .contains("second")
    );
    assert!(structured(&restored)["revisionNo"].as_i64().unwrap() > third_revision);
    let version = call(
        &env,
        &owner,
        "doc_version_get",
        json!({"docId": doc_id, "versionNo": 1}),
    )
    .await;
    assert!(
        structured(&version)["content"]
            .to_string()
            .contains("second"),
        "history is untouched"
    );
    let stale = call(
        &env,
        &owner,
        "doc_version_restore",
        json!({"docId": doc_id, "versionNo": 1, "expectedRevision": third_revision}),
    )
    .await;
    assert_eq!(error_code(&stale), "version_conflict");

    // Comments.
    let comment = call(
        &env,
        &owner,
        "doc_comment_create",
        json!({"docId": doc_id, "body": "looks good"}),
    )
    .await;
    assert!(ok(&comment), "{:?}", comment.body);
    let comment_id = structured(&comment)["id"].as_str().unwrap().to_string();
    let comments = call(&env, &owner, "doc_comment_list", json!({"docId": doc_id})).await;
    assert_eq!(structured(&comments)["items"].as_array().unwrap().len(), 1);
    let edited = call(
        &env,
        &owner,
        "doc_comment_edit",
        json!({"docId": doc_id, "commentId": comment_id, "body": "better"}),
    )
    .await;
    assert!(ok(&edited), "{:?}", edited.body);
    // The comment and its files are gone for good: Destructive, off under the standard policy.
    let refused = call(
        &env,
        &owner,
        "doc_comment_delete",
        json!({"docId": doc_id, "commentId": comment_id}),
    )
    .await;
    assert_eq!(error_code(&refused), "policy_denied", "{:?}", refused.body);
    let allow = rest(&env, "PUT", &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id), &env.owner_cookie, None,
        Some(json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": true, "allowAdministrative": false}))).await;
    assert!(allow.status.is_success(), "{:?}", allow.body);
    let deleted = call(
        &env,
        &owner,
        "doc_comment_delete",
        json!({"docId": doc_id, "commentId": comment_id}),
    )
    .await;
    assert!(ok(&deleted), "{:?}", deleted.body);
    let comments = call(&env, &owner, "doc_comment_list", json!({"docId": doc_id})).await;
    assert!(
        structured(&comments)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Templates, rename, move.
    let template = call(
        &env,
        &owner,
        "doc_template_create",
        json!({"workspaceId": ws, "name": "Weekly", "fromDocId": doc_id}),
    )
    .await;
    assert!(ok(&template), "{:?}", template.body);
    let template_id = structured(&template)["id"].as_str().unwrap().to_string();
    let from_template = call(&env, &owner, "doc_create", json!({"workspaceId": ws, "title": "From template", "visibility": "WORKSPACE", "templateId": template_id})).await;
    assert!(ok(&from_template), "{:?}", from_template.body);
    let child_id = structured(&from_template)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let templates = call(
        &env,
        &owner,
        "doc_template_list",
        json!({"workspaceId": ws}),
    )
    .await;
    assert_eq!(
        structured(&templates)["items"].as_array().map(Vec::len),
        Some(1),
        "{:?}",
        structured(&templates)
    );
    let both = call(&env, &owner, "doc_create", json!({"workspaceId": ws, "title": "x", "visibility": "WORKSPACE", "templateId": template_id, "markdown": "a"})).await;
    assert!(!ok(&both), "template and markdown together are refused");
    let renamed = call(
        &env,
        &owner,
        "doc_update",
        json!({"docId": child_id, "title": "Renamed"}),
    )
    .await;
    assert!(ok(&renamed), "{:?}", renamed.body);
    let moved = call(
        &env,
        &owner,
        "doc_move",
        json!({"docId": child_id, "parentId": doc_id}),
    )
    .await;
    assert!(ok(&moved), "{:?}", moved.body);
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{child_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.body["parentId"], doc_id.as_str());
    assert_eq!(web.body["title"], "Renamed");
    let removed = call(
        &env,
        &owner,
        "doc_template_delete",
        json!({"workspaceId": ws, "templateId": template_id}),
    )
    .await;
    assert!(ok(&removed), "{:?}", removed.body);
    let references = call(&env, &owner, "doc_references", json!({"docId": doc_id})).await;
    assert!(ok(&references), "{:?}", references.body);
    let resolved = call(
        &env,
        &owner,
        "doc_resolve_references",
        json!({"workspaceId": ws, "items": [{"kind": "DOC", "id": doc_id}]}),
    )
    .await;
    assert!(ok(&resolved), "{:?}", resolved.body);

    // An unfinished file cannot be attached; nothing leaks about it.
    let file = call(
        &env,
        &owner,
        "doc_file_attach",
        json!({"docId": doc_id, "fileId": Uuid::new_v4()}),
    )
    .await;
    assert!(!ok(&file));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn restricted_docs_follow_core_visibility_and_roles(pool: PgPool) {
    let env = setup(pool).await;
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let member_id = user_id(&env, "member@penfii.test").await;
    let owner = token_for(&env, &env.owner_cookie, DOC_SCOPES).await;
    let member = token_for(&env, &member_cookie, DOC_SCOPES).await;
    let outsider = token_for(&env, &env.outsider_cookie, DOC_SCOPES).await;
    let ws = env.workspace_id.to_string();

    let created = call(&env, &owner, "doc_create", json!({"workspaceId": ws, "title": "Secret", "visibility": "RESTRICTED", "markdown": "hidden text"})).await;
    assert!(ok(&created), "{:?}", created.body);
    let doc_id = structured(&created)["id"].as_str().unwrap().to_string();

    // The member cannot see it, over the web or through MCP; the answers agree.
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{doc_id}"),
        &member_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.status, StatusCode::NOT_FOUND);
    for tool in [
        "doc_get",
        "doc_comment_list",
        "doc_version_list",
        "doc_participants",
    ] {
        let reply = call(&env, &member, tool, json!({"docId": doc_id})).await;
        assert_eq!(error_code(&reply), "not_found", "{tool}");
    }
    let write = call(
        &env,
        &member,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": 1, "content": text_doc("x")}),
    )
    .await;
    assert_eq!(error_code(&write), "not_found");
    let tree = call(&env, &member, "doc_tree", json!({"workspaceId": ws})).await;
    assert!(!structured(&tree).to_string().contains("Secret"));
    // Another Workspace's person gets the same answer as for any resource that is not theirs.
    let reply = call(&env, &outsider, "doc_get", json!({"docId": doc_id})).await;
    assert_eq!(error_code(&reply), "not_found");

    // Sharing: the member becomes a VIEWER; reads work, writes do not.
    let shared = call(&env, &owner, "doc_set_participants", json!({"docId": doc_id, "participants": [{"userId": env.owner_id, "role": "MANAGER"}, {"userId": member_id, "role": "VIEWER"}]})).await;
    assert!(ok(&shared), "{:?}", shared.body);
    let reply = call(&env, &member, "doc_get", json!({"docId": doc_id})).await;
    assert!(ok(&reply), "{:?}", reply.body);
    let revision = structured(&reply)["revisionNo"].as_i64().unwrap();
    let write = call(
        &env,
        &member,
        "doc_save",
        json!({"docId": doc_id, "expectedRevision": revision, "content": text_doc("x")}),
    )
    .await;
    assert_eq!(error_code(&write), "forbidden");
    let visibility = call(
        &env,
        &member,
        "doc_set_visibility",
        json!({"docId": doc_id, "visibility": "WORKSPACE"}),
    )
    .await;
    assert_eq!(error_code(&visibility), "forbidden");

    // Opening it to the Workspace and closing it again.
    let open = call(
        &env,
        &owner,
        "doc_set_visibility",
        json!({"docId": doc_id, "visibility": "WORKSPACE"}),
    )
    .await;
    assert!(ok(&open), "{:?}", open.body);
    let closed = call(
        &env,
        &owner,
        "doc_set_visibility",
        json!({"docId": doc_id, "visibility": "RESTRICTED"}),
    )
    .await;
    assert!(ok(&closed), "{:?}", closed.body);

    // A member removed from the Workspace loses the tools at once.
    let removed = rest(
        &env,
        "DELETE",
        &format!("/v1/workspaces/{ws}/members/{member_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert!(removed.status.is_success(), "{:?}", removed.body);
    let reply = call(&env, &member, "doc_get", json!({"docId": doc_id})).await;
    assert_eq!(error_code(&reply), "not_found");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scopes_policy_and_audit_cover_doc_tools(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    // A new consent of the same person and client replaces the grant, so tokens are used in turn.
    let reader = token_for(&env, &env.owner_cookie, "workspace:read doc:read").await;

    // tools/list shows what the standard policy allows, whatever scopes the grant has; the call is
    // where the scope is checked.
    let listed = modern_list(&env.app, &reader).await;
    assert!(
        listed.body["result"]["tools"].is_array(),
        "{:?}",
        listed.body
    );
    let names = tool_names(&listed);
    assert!(names.contains(&"doc_get".to_string()));
    assert!(names.contains(&"doc_create".to_string()));
    assert!(!names.contains(&"doc_delete".to_string()), "policy");
    let denied = call(
        &env,
        &reader,
        "doc_create",
        json!({"workspaceId": ws, "title": "x", "visibility": "WORKSPACE"}),
    )
    .await;
    assert_eq!(error_code(&denied), "insufficient_scope");
    let writer = token_for(&env, &env.owner_cookie, DOC_SCOPES).await;
    let names = tool_names(&modern_list(&env.app, &writer).await);
    assert!(
        names.contains(&"doc_create".to_string()) && !names.contains(&"doc_delete".to_string())
    );

    // Calls re-check: no scope, then no policy.
    let created = call(
        &env,
        &writer,
        "doc_create",
        json!({"workspaceId": ws, "title": "Doomed", "visibility": "WORKSPACE", "markdown": "bye"}),
    )
    .await;
    let doc_id = structured(&created)["id"].as_str().unwrap().to_string();
    let denied = call(&env, &writer, "doc_delete", json!({"docId": doc_id})).await;
    assert_eq!(error_code(&denied), "insufficient_scope");
    let deleter = token_for(&env, &env.owner_cookie, &format!("{DOC_SCOPES} doc:delete")).await;
    let names = tool_names(&modern_list(&env.app, &deleter).await);
    assert!(
        !names.contains(&"doc_delete".to_string()),
        "destructive is off in the standard policy"
    );
    let denied = call(&env, &deleter, "doc_delete", json!({"docId": doc_id})).await;
    assert_eq!(error_code(&denied), "policy_denied");
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{doc_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.status, StatusCode::OK, "nothing was deleted");

    // An administrator allows destructive actions; the delete then works like the route and is audited.
    let policy = rest(&env, "PUT", &format!("/v1/workspaces/{ws}/mcp-policy"), &env.owner_cookie, None,
        Some(json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": true, "allowAdministrative": false}))).await;
    assert!(policy.status.is_success(), "{:?}", policy.body);
    let names = tool_names(&modern_list(&env.app, &deleter).await);
    assert!(names.contains(&"doc_delete".to_string()));
    let deleted = call(&env, &deleter, "doc_delete", json!({"docId": doc_id})).await;
    assert!(ok(&deleted), "{:?}", deleted.body);
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{doc_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.status, StatusCode::NOT_FOUND);
    // The Audit entry and the call log carry the Workspace resolved from the Doc, not only an argument.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'mcp.doc_delete' AND workspace_id = $1 AND outcome_code = 'OK'",
    )
    .bind(env.workspace_id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
    let logged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.mcp_call_log WHERE tool = 'doc_save' OR tool = 'doc_create' AND workspace_id IS NULL",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(logged, 0, "calls on a Doc are logged with their Workspace");
}
