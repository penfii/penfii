//! MCP audit, webhook list, trash/archive and recovery lists (FEA-171): reads only, no secrets,
//! Core's roles decide, and the web-only administration is not reachable through MCP.

use serde_json::json;
use sqlx::PgPool;

use crate::mcp_oauth::{add_member, rest, setup, token_for};
use crate::mcp_support::{mcp_err, mcp_ok};

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn audit_and_webhook_lists_follow_roles_and_hold_no_secret(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id;
    let owner = token_for(&env, &env.owner_cookie, "admin:read workspace:read").await;
    let member_cookie = add_member(&env, "mina@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, "admin:read workspace:read").await;
    let outsider = token_for(&env, &env.outsider_cookie, "admin:read").await;

    // A web action leaves an audit entry; MCP reads it through the same Core function.
    let invitation = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        &env.owner_cookie,
        Some("inv-1"),
        Some(json!({"email": "x@penfii.test", "role": "MEMBER"})),
    )
    .await;
    assert_eq!(invitation.status, 201);
    let via_mcp = mcp_ok(
        &env,
        &owner,
        "audit_list",
        json!({"workspaceId": ws, "action": "invitation.", "limit": 5}),
    )
    .await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/audit?action=invitation.&limit=5"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(via_mcp["items"], web.body["items"]);
    assert!(!via_mcp["items"].as_array().unwrap().is_empty());
    assert!(
        !via_mcp
            .to_string()
            .contains(invitation.body["inviteToken"].as_str().unwrap())
    );
    assert_eq!(
        mcp_err(&env, &member, "audit_list", json!({"workspaceId": ws})).await,
        "forbidden"
    );
    assert_eq!(
        mcp_err(&env, &outsider, "audit_list", json!({"workspaceId": ws})).await,
        "not_found"
    );

    // Webhooks: listed without any secret; creating is not a tool.
    let channel = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        &env.owner_cookie,
        None,
        Some(json!({"name": "hooks", "visibility": "PUBLIC"})),
    )
    .await;
    assert_eq!(channel.status, 201, "{}", channel.body);
    let channel_id = channel.body["id"].as_str().unwrap().to_string();
    let created = rest(
        &env,
        "POST",
        &format!("/v1/channels/{channel_id}/webhooks"),
        &env.owner_cookie,
        None,
        Some(json!({"name": "deploys", "authMode": "URL_SECRET", "displayName": "Deploy Bot"})),
    )
    .await;
    assert_eq!(created.status, 201, "{}", created.body);
    let secret = created.body["secret"].as_str().unwrap().to_string();
    let listed = mcp_ok(
        &env,
        &owner,
        "webhook_list",
        json!({"channelId": channel_id}),
    )
    .await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    let text = listed.to_string().to_lowercase();
    assert!(
        !text.contains(&secret.to_lowercase())
            && !text.contains("\"secret\"")
            && !text.contains("\"token"),
        "{text}"
    );
    // The call log names the Workspace of a call that names only a channel.
    let logged: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.mcp_call_log WHERE operation = 'webhook.list' ORDER BY at LIMIT 1")
        .fetch_one(&env.pool).await.unwrap();
    assert_eq!(logged, Some(ws));
    let secret_in_logs: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM penfii.mcp_call_log l WHERE row_to_json(l)::text LIKE '%' || $1 || '%')
              + (SELECT count(*) FROM penfii.audit_entries a WHERE row_to_json(a)::text LIKE '%' || $1 || '%')")
        .bind(&secret).fetch_one(&env.pool).await.unwrap();
    assert_eq!(secret_in_logs, 0);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn trash_archive_and_recovery_lists_are_read_only_views_of_the_web_lists(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id;
    let token = token_for(
        &env,
        &env.owner_cookie,
        "admin:read workspace:read ticket:write",
    )
    .await;
    let project = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/projects"),
        &env.owner_cookie,
        Some("p1"),
        Some(json!({"name": "Trash", "teamIds": []})),
    )
    .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let ticket = rest(&env, "POST", &format!("/v1/projects/{project_id}/tickets"), &env.owner_cookie, Some("t1"),
        Some(json!({"title": "to trash", "description": null, "hierarchyKind": "TASK", "fileIds": []}))).await;
    let ticket_id = ticket.body["id"].as_str().unwrap().to_string();
    assert_eq!(
        rest(
            &env,
            "POST",
            &format!("/v1/tickets/{ticket_id}/commands/trash"),
            &env.owner_cookie,
            Some("tr"),
            Some(json!({}))
        )
        .await
        .status,
        204
    );

    let listed = mcp_ok(&env, &token, "trash_list", json!({"projectId": project_id})).await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/trash"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(listed["items"], web.body["items"]);
    assert_eq!(listed["total"], 1);
    mcp_ok(
        &env,
        &token,
        "archive_list",
        json!({"projectId": project_id}),
    )
    .await;

    // A member who is not an admin of the Project may not list its trash (nor even see the Project).
    let member_cookie = add_member(&env, "mina@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, "admin:read workspace:read").await;
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "trash_list",
            json!({"projectId": project_id})
        )
        .await,
        "not_found"
    );

    for tool in [
        "recovery_projects_without_admin",
        "recovery_docs_without_manager",
        "recovery_channels_without_manager",
    ] {
        let result = mcp_ok(&env, &token, tool, json!({"workspaceId": ws})).await;
        assert!(
            result["items"].is_array() && result["recovery"].as_str().unwrap().contains("web-only"),
            "{tool}: {result}"
        );
        assert_eq!(
            mcp_err(&env, &member, tool, json!({"workspaceId": ws})).await,
            "forbidden",
            "{tool}"
        );
    }
}
