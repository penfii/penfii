//! Discovering a tool is not being allowed to run it (FEA-174). `tools/list` shows what the
//! Workspace policy allows whatever the token's scopes, so a client can learn which permission a
//! tool needs; `tools/call` checks scope, policy and the person's own permission as before.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    ALL_SCOPE_TEXT, Env, add_member, error_code, modern_call, modern_list, rest, setup, token_for,
    token_for_client, tool_names,
};
use crate::mcp_support::{mcp_ok, set_policy};

fn tools(reply: &crate::common::Reply) -> Vec<Value> {
    reply.body["result"]["tools"].as_array().unwrap().clone()
}

/// The scopes a listed tool declares, checked to be the same in both places a client may read.
fn declared(tool: &Value) -> Vec<String> {
    assert_eq!(tool["securitySchemes"], tool["_meta"]["securitySchemes"]);
    assert_eq!(tool["securitySchemes"][0]["type"], "oauth2", "{tool}");
    tool["securitySchemes"][0]["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn challenge_param(challenge: &str, name: &str) -> Option<String> {
    let start = challenge.find(&format!("{name}=\""))? + name.len() + 2;
    let end = challenge[start..].find('"')? + start;
    Some(challenge[start..end].to_string())
}

async fn project(env: &Env) -> String {
    let reply = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": "Discovery", "teamIds": []})),
    )
    .await;
    assert!(reply.status.is_success(), "{:?}", reply.body);
    reply.body["id"].as_str().unwrap().to_string()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn five_scopes_discover_every_tool_the_standard_policy_allows(pool: PgPool) {
    let env = setup(pool).await;
    // The five scopes a ChatGPT connection holds, then a connection with every scope.
    let five = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let narrow = tools(&modern_list(&env.app, &five).await);
    let everything = penfii_core::oauth::scopes::all().join(" ");
    // High-risk scopes need a recent sign-in at consent; the test owner has just signed in.
    let all = token_for(&env, &env.owner_cookie, &everything).await;
    let full = tools(&modern_list(&env.app, &all).await);

    assert_eq!(narrow.len(), 190, "the standard policy's tools");
    assert_eq!(
        narrow, full,
        "same tools, order and descriptors for any grant"
    );

    // securitySchemes is on every tool, in both places, and names only scopes that exist.
    let mut union = std::collections::BTreeSet::new();
    for tool in &narrow {
        for scope in declared(tool) {
            assert!(
                penfii_core::oauth::scopes::is_known(&scope),
                "{}: {scope}",
                tool["name"]
            );
            union.insert(scope);
        }
    }
    // A client that asks for all of them asks for no high-risk scope (which would need a recent
    // sign-in), since only tools the standard policy hides need those.
    assert!(
        union
            .iter()
            .all(|s| !penfii_core::oauth::scopes::is_high_risk(s)),
        "{union:?}"
    );
    assert!(union.len() > 5, "{union:?}");

    // What the standard policy hides is exactly the 13 high-risk tools: allowing everything shows
    // them, and with the standard policy back each one is refused for a connection with every scope.
    set_policy(&env, true, true, true).await;
    let visible = tool_names(&modern_list(&env.app, &all).await);
    assert_eq!(visible.len(), 203);
    set_policy(&env, true, false, false).await;
    let standard = tool_names(&modern_list(&env.app, &all).await);
    assert_eq!(standard.len(), 190);
    let hidden: Vec<&String> = visible.iter().filter(|n| !standard.contains(n)).collect();
    assert_eq!(hidden.len(), 13, "{hidden:?}");

    // Nothing about this connection or its secrets is in the list.
    let text = Value::Array(narrow.clone()).to_string();
    for secret in [five.as_str(), all.as_str(), env.owner_cookie.as_str()] {
        assert!(!text.contains(secret));
    }
    for marker in [
        "pfa_",
        "pfr_",
        "client_secret",
        &env.owner_id.to_string(),
        &env.workspace_id.to_string(),
    ] {
        assert!(!text.contains(marker), "{marker}");
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_listed_tool_asks_for_its_scopes_once_and_runs_when_they_are_granted(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let project_id = project(&env).await;
    let five = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let doc_args =
        json!({"workspaceId": ws, "title": "Plan", "visibility": "WORKSPACE", "markdown": "hi"});

    // One missing scope.
    let denied = modern_call(&env.app, &five, "doc_create", doc_args.clone()).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&denied), "insufficient_scope");
    let header = denied.header("www-authenticate").unwrap().to_string();
    assert_eq!(
        challenge_param(&header, "error").unwrap(),
        "insufficient_scope"
    );
    assert!(
        challenge_param(&header, "resource_metadata")
            .unwrap()
            .ends_with("/.well-known/oauth-protected-resource/mcp")
    );
    assert_eq!(
        challenge_param(&header, "scope").unwrap(),
        format!("doc:write {ALL_SCOPE_TEXT}"),
        "what it lacks first, then what the connection has"
    );
    assert!(
        challenge_param(&header, "error_description")
            .unwrap()
            .contains("doc:write")
    );
    // The same challenge is in the result, for clients that read it there.
    assert_eq!(
        denied.body["result"]["_meta"]["mcp/www_authenticate"],
        json!([header])
    );
    let docs: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.doc_pages")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(docs, 0, "a refused call writes nothing");

    // Two missing scopes are asked for together, so one new consent is enough.
    let both = modern_call(
        &env.app,
        &five,
        "ticket_attachment_add",
        json!({"ticketId": Uuid::new_v4(), "fileId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(error_code(&both), "insufficient_scope");
    let header = both.header("www-authenticate").unwrap();
    assert_eq!(
        challenge_param(header, "scope").unwrap(),
        format!("file:write {ALL_SCOPE_TEXT}")
    );
    let two_one = token_for(&env, &env.owner_cookie, "workspace:read").await;
    let two = modern_call(
        &env.app,
        &two_one,
        "doc_resolve_references",
        json!({"workspaceId": ws, "references": []}),
    )
    .await;
    assert_eq!(
        challenge_param(two.header("www-authenticate").unwrap(), "scope").unwrap(),
        "doc:read ticket:read workspace:read"
    );
    assert!(
        two.body["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("doc:read, ticket:read")
    );

    // A scope the arguments need is asked for in the same challenge: a search for Docs by a
    // connection without search:read asks for both, one without doc:read only for that one.
    let search = json!({"workspaceId": ws, "query": "plan", "kind": "doc"});
    let reply = modern_call(&env.app, &two_one, "search", search.clone()).await;
    assert_eq!(
        challenge_param(reply.header("www-authenticate").unwrap(), "scope").unwrap(),
        "search:read doc:read workspace:read"
    );
    let five = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let reply = modern_call(&env.app, &five, "search", search).await;
    assert_eq!(
        challenge_param(reply.header("www-authenticate").unwrap(), "scope").unwrap(),
        format!("doc:read {ALL_SCOPE_TEXT}")
    );

    // The old five-scope tools keep working.
    mcp_ok(&env, &five, "workspace_list", json!({})).await;
    let ticket = mcp_ok(
        &env,
        &five,
        "ticket_create",
        json!({"projectId": project_id, "title": "Still works"}),
    )
    .await;
    mcp_ok(&env, &five, "ticket_get", json!({"ticketId": ticket["id"]})).await;
    mcp_ok(
        &env,
        &five,
        "member_search",
        json!({"workspaceId": ws, "query": "x"}),
    )
    .await;

    // The person approves what the challenge named: the tool runs.
    let more = token_for(
        &env,
        &env.owner_cookie,
        &format!("{ALL_SCOPE_TEXT} doc:write"),
    )
    .await;
    let created = mcp_ok(&env, &more, "doc_create", doc_args).await;
    assert!(created["id"].is_string(), "{created}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn discovery_does_not_widen_what_a_call_may_do(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let project_id = project(&env).await;
    let everything = penfii_core::oauth::scopes::all().join(" ");
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, &everything).await;
    let outsider = token_for(&env, &env.outsider_cookie, &everything).await;

    // Another Workspace's person sees their own policy's list and nothing of this Workspace.
    assert_eq!(
        tool_names(&modern_list(&env.app, &outsider).await).len(),
        190
    );
    for (tool, args) in [
        ("workspace_overview", json!({"workspaceId": ws})),
        ("board_get", json!({"projectId": project_id})),
    ] {
        let reply = modern_call(&env.app, &outsider, tool, args).await;
        assert_eq!(error_code(&reply), "not_found", "{tool}");
    }
    // Core's own rules still decide for a member: no Project access, no admin work.
    let reply = modern_call(
        &env.app,
        &member,
        "board_get",
        json!({"projectId": project_id}),
    )
    .await;
    assert_eq!(error_code(&reply), "not_found");
    let reply = modern_call(&env.app, &member, "audit_list", json!({"workspaceId": ws})).await;
    assert_eq!(error_code(&reply), "forbidden");

    // A blocked risk class stays hidden, and calling it is refused although every scope is granted.
    let names = tool_names(&modern_list(&env.app, &member).await);
    assert!(!names.contains(&"doc_delete".to_string()));
    let reply = modern_call(
        &env.app,
        &member,
        "doc_delete",
        json!({"docId": Uuid::new_v4()}),
    )
    .await;
    assert!(
        ["policy_denied", "not_found"].contains(&error_code(&reply)),
        "{:?}",
        reply.body
    );
    // Web-only actions have no tool at all.
    for tool in [
        "ticket_purge",
        "member_remove",
        "webhook_create",
        "oauth_grant_revoke",
    ] {
        let reply = modern_call(&env.app, &member, tool, json!({})).await;
        assert_eq!(reply.body["error"]["code"], -32602, "{tool}");
    }
    // A Workspace invitation for OWNER stays web-only whatever the scopes.
    let owner = token_for(&env, &env.owner_cookie, &everything).await;
    let reply = modern_call(
        &env.app,
        &owner,
        "invitation_create",
        json!({"workspaceId": ws, "email": "boss@penfii.test", "role": "ADMIN"}),
    )
    .await;
    assert_eq!(error_code(&reply), "web_only");

    // The policy turns writes off: the write tools leave the list and a call is refused. (The owner
    // has this Workspace only; a person with another one that allows writes keeps seeing them.)
    set_policy(&env, false, false, false).await;
    let names = tool_names(&modern_list(&env.app, &owner).await);
    assert!(!names.contains(&"ticket_create".to_string()));
    assert!(names.contains(&"ticket_get".to_string()));
    let reply = modern_call(
        &env.app,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "x"}),
    )
    .await;
    assert_eq!(error_code(&reply), "policy_denied");
    set_policy(&env, true, false, false).await;

    // Removed from the Workspace: nothing of it is listed or callable.
    sqlx::query("UPDATE penfii.workspace_members SET state = 'INACTIVE' WHERE workspace_id = $1 AND user_id <> $2")
        .bind(env.workspace_id)
        .bind(env.owner_id)
        .execute(&env.pool)
        .await
        .unwrap();
    let reply = modern_call(
        &env.app,
        &member,
        "workspace_overview",
        json!({"workspaceId": ws}),
    )
    .await;
    assert_eq!(error_code(&reply), "not_found");

    // A revoked grant stops at the door: 401 before any list.
    let claude = token_for_client(
        &env,
        &env.owner_cookie,
        "https://claude.example/oauth/client.json",
        "Claude Code",
        "workspace:read",
    )
    .await;
    assert_eq!(modern_list(&env.app, &claude).await.status, StatusCode::OK);
    sqlx::query("UPDATE penfii.oauth_grants SET revoked_at = now() WHERE revoked_at IS NULL")
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        modern_list(&env.app, &claude).await.status,
        StatusCode::UNAUTHORIZED
    );
}
