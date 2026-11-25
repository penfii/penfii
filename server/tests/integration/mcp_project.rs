//! MCP project, workflow, label, calendar, pin and Project-access operations (FEA-165).

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{Env, add_member, api, modern_list, rest, setup, token_for, tool_names};
use crate::mcp_support::{mcp_err, mcp_ok};

const SCOPES: &str =
    "workspace:read member:read ticket:read ticket:write project:read project:write";
const MANAGE: &str =
    "workspace:read member:read ticket:read ticket:write project:read project:write project:manage";

async fn new_project(env: &Env, name: &str) -> String {
    rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": name, "teamIds": []})),
    )
    .await
    .body["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn user_id(env: &Env, email: &str) -> String {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM penfii.users WHERE email_key = $1")
        .bind(email)
        .fetch_one(&env.pool)
        .await
        .unwrap()
        .to_string()
}

async fn set_policy(env: &Env, administrative: bool, destructive: bool) {
    let put = api(
        &env.app,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.owner_cookie),
        Some(json!({"mcpEnabled": true, "allowWrite": true,
                    "allowDestructive": destructive, "allowAdministrative": administrative})),
    )
    .await;
    assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn project_reads_and_routine_settings_match_the_web_app(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Settings").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;

    let got = mcp_ok(
        &env,
        &owner,
        "project_get",
        json!({"projectId": project_id}),
    )
    .await;
    assert_eq!(got["project"]["name"], "Settings");
    assert_eq!(got["project"]["workspaceId"], env.workspace_id.to_string());
    let web_flow = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(got["workflow"]["states"], web_flow.body["states"]);
    assert_eq!(got["workflow"]["version"], web_flow.body["version"]);
    assert!(!got["ticketTypes"].as_array().unwrap().is_empty());
    let members = mcp_ok(
        &env,
        &owner,
        "project_members_list",
        json!({"projectId": project_id}),
    )
    .await;
    assert!(!members["members"].as_array().unwrap().is_empty());
    let prefix = mcp_ok(
        &env,
        &owner,
        "project_prefix_check",
        json!({"workspaceId": env.workspace_id, "name": "Brand New"}),
    )
    .await;
    assert!(prefix.is_object());

    // Labels.
    let label = mcp_ok(
        &env,
        &owner,
        "label_create",
        json!({"projectId": project_id, "name": "urgent"}),
    )
    .await;
    let labels = mcp_ok(&env, &owner, "label_list", json!({"projectId": project_id})).await;
    let web_labels = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/labels"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(labels["labels"], web_labels.body);
    assert_eq!(labels["labels"][0]["id"], label["id"]);
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "label_create",
            json!({"projectId": project_id, "name": ""})
        )
        .await,
        "invalid_arguments"
    );

    // Workflow states: add, change, version conflict.
    let flow = mcp_ok(
        &env,
        &owner,
        "project_workflow",
        json!({"projectId": project_id}),
    )
    .await;
    let version = flow["version"].as_i64().unwrap();
    let created = mcp_ok(
        &env,
        &owner,
        "workflow_state_create",
        json!({"projectId": project_id, "expectedWorkflowVersion": version, "name": "In review",
               "category": "IN_PROGRESS", "position": 15}),
    )
    .await;
    let state_id = created["state"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["version"].as_i64().unwrap(), version + 1);
    assert_eq!(
        mcp_err(&env, &owner, "workflow_state_create", json!({"projectId": project_id, "expectedWorkflowVersion": version, "name": "Late", "category": "DONE", "position": 99})).await,
        "workflow_version_conflict"
    );
    let updated = mcp_ok(
        &env,
        &owner,
        "workflow_state_update",
        json!({"projectId": project_id, "stateId": state_id, "expectedWorkflowVersion": version + 1, "name": "Reviewing", "wipLimit": 3}),
    )
    .await;
    assert_eq!(updated["state"]["name"], "Reviewing");
    assert_eq!(updated["state"]["wipLimit"], 3);
    let cleared = mcp_ok(
        &env,
        &owner,
        "workflow_state_update",
        json!({"projectId": project_id, "stateId": state_id, "expectedWorkflowVersion": version + 2, "wipLimit": null}),
    )
    .await;
    assert!(cleared["state"]["wipLimit"].is_null());
    let web = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert!(
        web.body["states"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "Reviewing")
    );

    // Deleting a state is destructive: its own scope and policy class.
    assert_eq!(
        mcp_err(&env, &owner, "workflow_state_delete", json!({"projectId": project_id, "stateId": state_id, "expectedWorkflowVersion": version + 3})).await,
        "insufficient_scope"
    );

    // Calendar and pins.
    let none = mcp_ok(
        &env,
        &owner,
        "project_calendar_get",
        json!({"projectId": project_id}),
    )
    .await;
    let day = json!([{"from": "09:00", "to": "18:00"}]);
    let saved = mcp_ok(
        &env,
        &owner,
        "project_calendar_put",
        json!({"projectId": project_id, "expectedVersion": 0, "timeZone": "Asia/Seoul",
               "definition": {"version": 1, "week": {"mon": day}, "holidays": []}}),
    )
    .await;
    assert_eq!(saved["timeZone"], "Asia/Seoul");
    let read = mcp_ok(
        &env,
        &owner,
        "project_calendar_get",
        json!({"projectId": project_id}),
    )
    .await;
    assert_ne!(read, none);
    // Creating again with version 0 is a conflict, never an overwrite.
    assert!(!mcp_err(&env, &owner, "project_calendar_put", json!({"projectId": project_id, "expectedVersion": 0, "timeZone": "Asia/Seoul", "definition": {"version": 1, "week": {}, "holidays": []}})).await.is_empty());
    assert_eq!(
        mcp_err(&env, &owner, "project_calendar_put", json!({"projectId": project_id, "expectedVersion": 1, "timeZone": "Asia/Seoul", "definition": {"nope": 1}})).await,
        "invalid_arguments"
    );
    let pins = mcp_ok(
        &env,
        &owner,
        "project_pin_add",
        json!({"projectId": project_id, "url": "https://example.test/doc", "title": "Doc"}),
    )
    .await;
    let listed = mcp_ok(
        &env,
        &owner,
        "project_pins_list",
        json!({"projectId": project_id}),
    )
    .await;
    assert_eq!(pins, listed);
    let pin_id = listed.to_string();
    assert!(pin_id.contains("https://example.test/doc"));
    let pin_uuid = {
        let rows: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM penfii.project_pins WHERE project_id = $1")
                .bind(Uuid::parse_str(&project_id).unwrap())
                .fetch_all(&env.pool)
                .await
                .unwrap();
        rows[0]
    };
    mcp_ok(
        &env,
        &owner,
        "project_pin_remove",
        json!({"pinId": pin_uuid}),
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM penfii.project_pins WHERE project_id = $1"
        )
        .bind(Uuid::parse_str(&project_id).unwrap())
        .fetch_one(&env.pool)
        .await
        .unwrap(),
        0
    );

    // Estimate unit setting, same validation as the web route.
    assert_eq!(
        mcp_ok(
            &env,
            &owner,
            "project_update",
            json!({"projectId": project_id, "estimateUnit": "HOURS"})
        )
        .await["estimateUnit"],
        "HOURS"
    );
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "project_update",
            json!({"projectId": project_id, "estimateUnit": "DAYS"})
        )
        .await,
        "invalid_arguments"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn project_reads_and_settings_respect_roles_scopes_and_other_workspaces(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Roles").await;
    let viewer_cookie = add_member(&env, "viewer@penfii.test", "MEMBER").await;
    let editor_cookie = add_member(&env, "editor@penfii.test", "MEMBER").await;
    let owner_token = token_for(&env, &env.owner_cookie, SCOPES).await;
    for (email, role) in [
        ("viewer@penfii.test", "VIEWER"),
        ("editor@penfii.test", "EDITOR"),
    ] {
        let uid = user_id(&env, email).await;
        let granted = rest(
            &env,
            "PUT",
            &format!("/v1/projects/{project_id}/grants/{uid}"),
            &env.owner_cookie,
            Some(&format!("g-{role}")),
            Some(json!({"role": role})),
        )
        .await;
        assert_eq!(granted.status, StatusCode::NO_CONTENT);
    }
    let viewer = token_for(&env, &viewer_cookie, SCOPES).await;
    let editor = token_for(&env, &editor_cookie, SCOPES).await;
    let outsider = token_for(&env, &env.outsider_cookie, SCOPES).await;

    // Everyone with a role reads; only a Project admin changes settings (the web rules).
    for token in [&viewer, &editor, &owner_token] {
        mcp_ok(&env, token, "project_get", json!({"projectId": project_id})).await;
        mcp_ok(&env, token, "label_list", json!({"projectId": project_id})).await;
    }
    // Labels follow the web rule: an editor may add one, a viewer may not.
    assert_eq!(
        mcp_err(
            &env,
            &viewer,
            "label_create",
            json!({"projectId": project_id, "name": "x"})
        )
        .await,
        "forbidden"
    );
    mcp_ok(
        &env,
        &editor,
        "label_create",
        json!({"projectId": project_id, "name": "x"}),
    )
    .await;
    for (name, token) in [("viewer", &viewer), ("editor", &editor)] {
        for (tool, args) in [
            (
                "project_update",
                json!({"projectId": project_id, "estimateUnit": "POINTS"}),
            ),
            (
                "workflow_state_create",
                json!({"projectId": project_id, "expectedWorkflowVersion": 1, "name": "x", "category": "DONE", "position": 9}),
            ),
        ] {
            assert_eq!(
                mcp_err(&env, token, tool, args).await,
                "forbidden",
                "{name} {tool}"
            );
        }
    }
    // The access listings are for Project admins.
    assert_eq!(
        mcp_err(
            &env,
            &editor,
            "project_permissions_get",
            json!({"projectId": project_id})
        )
        .await,
        "forbidden"
    );
    mcp_ok(
        &env,
        &owner_token,
        "project_permissions_get",
        json!({"projectId": project_id}),
    )
    .await;
    // Another Workspace learns nothing.
    for tool in [
        "project_get",
        "label_list",
        "project_calendar_get",
        "project_pins_list",
        "project_ticket_types",
        "project_members_list",
    ] {
        assert_eq!(
            mcp_err(&env, &outsider, tool, json!({"projectId": project_id})).await,
            "not_found",
            "{tool}"
        );
    }

    // A read-only scope discovers the write tools the policy allows (not the administrative one)
    // and cannot call them.
    let reader = token_for(&env, &env.owner_cookie, "workspace:read project:read").await;
    let names = tool_names(&modern_list(&env.app, &reader).await);
    assert!(names.contains(&"project_get".to_string()));
    for listed in ["label_create", "workflow_state_create", "ticket_create"] {
        assert!(names.contains(&listed.to_string()), "{listed}");
    }
    assert!(!names.contains(&"project_grant_set".to_string()), "policy");
    assert_eq!(
        mcp_err(
            &env,
            &reader,
            "label_create",
            json!({"projectId": project_id, "name": "x"})
        )
        .await,
        "insufficient_scope"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn changing_who_has_access_is_administrative_and_cannot_escalate(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Access").await;
    let editor_cookie = add_member(&env, "editor@penfii.test", "MEMBER").await;
    let other_cookie = add_member(&env, "other@penfii.test", "MEMBER").await;
    let _ = other_cookie;
    let editor_id = user_id(&env, "editor@penfii.test").await;
    let other_id = user_id(&env, "other@penfii.test").await;
    let owner_id = env.owner_id.to_string();
    let owner = token_for(&env, &env.owner_cookie, MANAGE).await;

    // Standard policy: administrative is off, so the tools are hidden and refused.
    let names = tool_names(&modern_list(&env.app, &owner).await);
    assert!(!names.contains(&"project_grant_set".to_string()));
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "project_grant_set",
            json!({"projectId": project_id, "userId": editor_id, "role": "EDITOR"})
        )
        .await,
        "policy_denied"
    );
    set_policy(&env, true, true).await;
    let names = tool_names(&modern_list(&env.app, &owner).await);
    for tool in [
        "project_grant_set",
        "project_grant_remove",
        "project_team_link",
        "project_team_unlink",
        "project_team_participation_set",
        "project_designation_set",
    ] {
        assert!(names.contains(&tool.to_string()), "{tool}");
    }

    // Grant, designate, escalate-refuse, remove.
    mcp_ok(
        &env,
        &owner,
        "project_grant_set",
        json!({"projectId": project_id, "userId": editor_id, "role": "EDITOR"}),
    )
    .await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/permissions"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert!(web.body.to_string().contains(&editor_id), "{}", web.body);
    let designated = mcp_ok(&env, &owner, "project_designation_set", json!({"projectId": project_id, "userId": editor_id, "canWaiveDependencies": true, "canManagePins": false})).await;
    assert_eq!(designated["canWaiveDependencies"], true);
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "project_grant_set",
            json!({"projectId": project_id, "userId": editor_id, "role": "OWNER"})
        )
        .await,
        "invalid_arguments"
    );

    let editor = token_for(&env, &editor_cookie, MANAGE).await;
    for (tool, args) in [
        (
            "project_grant_set",
            json!({"projectId": project_id, "userId": editor_id, "role": "ADMIN"}),
        ),
        (
            "project_grant_set",
            json!({"projectId": project_id, "userId": other_id, "role": "VIEWER"}),
        ),
        (
            "project_grant_remove",
            json!({"projectId": project_id, "userId": owner_id}),
        ),
        (
            "project_designation_set",
            json!({"projectId": project_id, "userId": editor_id, "canWaiveDependencies": true, "canManagePins": true}),
        ),
    ] {
        assert_eq!(
            mcp_err(&env, &editor, tool, args).await,
            "forbidden",
            "{tool}"
        );
    }
    let role_in_db = |uid: String| {
        let pool = env.pool.clone();
        let project = Uuid::parse_str(&project_id).unwrap();
        async move {
            sqlx::query_scalar::<_, String>("SELECT role FROM penfii.project_user_grants WHERE project_id = $1 AND user_id = $2")
                .bind(project)
                .bind(Uuid::parse_str(&uid).unwrap())
                .fetch_optional(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        role_in_db(editor_id.clone()).await.as_deref(),
        Some("EDITOR"),
        "no escalation happened"
    );
    assert_eq!(role_in_db(other_id.clone()).await, None);

    // A Team: link, give a role, take it away, unlink.
    let owner = token_for(&env, &env.owner_cookie, MANAGE).await;
    let team = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{}/teams", env.workspace_id),
        &env.owner_cookie,
        Some("team-1"),
        Some(json!({"name": "Crew", "members": [{"userId": other_id, "role": "MANAGER"}]})),
    )
    .await;
    assert!(team.status.is_success(), "{:?}", team.body);
    let team_id = team.body["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &owner,
        "project_team_link",
        json!({"projectId": project_id, "teamId": team_id}),
    )
    .await;
    mcp_ok(
        &env,
        &owner,
        "project_team_participation_set",
        json!({"projectId": project_id, "teamId": team_id, "role": "VIEWER"}),
    )
    .await;
    let roster = mcp_ok(
        &env,
        &owner,
        "project_team_members_list",
        json!({"projectId": project_id, "teamId": team_id}),
    )
    .await;
    assert!(roster.to_string().contains(&other_id));
    let teams_web = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/permissions"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert!(teams_web.body.to_string().contains(&team_id));
    mcp_ok(
        &env,
        &owner,
        "project_team_participation_set",
        json!({"projectId": project_id, "teamId": team_id, "role": null}),
    )
    .await;
    mcp_ok(
        &env,
        &owner,
        "project_team_unlink",
        json!({"projectId": project_id, "teamId": team_id}),
    )
    .await;
    assert!(
        !mcp_err(
            &env,
            &owner,
            "project_team_link",
            json!({"projectId": project_id, "teamId": Uuid::new_v4()})
        )
        .await
        .is_empty()
    );

    // Removal ends access at once; the removed person reads nothing.
    mcp_ok(
        &env,
        &owner,
        "project_grant_remove",
        json!({"projectId": project_id, "userId": editor_id}),
    )
    .await;
    let editor = token_for(&env, &editor_cookie, SCOPES).await;
    assert_eq!(
        mcp_err(
            &env,
            &editor,
            "project_get",
            json!({"projectId": project_id})
        )
        .await,
        "not_found"
    );
    // The audit shows these calls with their Workspace and without payloads.
    let audit: Vec<(Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT workspace_id, data FROM penfii.audit_entries WHERE action_code = 'mcp.project_grant_set' AND outcome_code = 'OK' ORDER BY occurred_at",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert!(!audit.is_empty());
    assert!(audit.iter().all(|(ws, data)| *ws == Some(env.workspace_id) && !data.to_string().contains("EDITOR")));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn deleting_a_workflow_state_is_destructive_and_announced(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Delete state").await;
    let owner = token_for(&env, &env.owner_cookie, MANAGE).await;
    let flow = mcp_ok(
        &env,
        &owner,
        "project_workflow",
        json!({"projectId": project_id}),
    )
    .await;
    let version = flow["version"].as_i64().unwrap();
    let created = mcp_ok(
        &env,
        &owner,
        "workflow_state_create",
        json!({"projectId": project_id, "expectedWorkflowVersion": version, "name": "Temp", "category": "IN_PROGRESS", "position": 20}),
    )
    .await;
    let state_id = created["state"]["id"].as_str().unwrap();
    assert_eq!(
        mcp_err(&env, &owner, "workflow_state_delete", json!({"projectId": project_id, "stateId": state_id, "expectedWorkflowVersion": version + 1})).await,
        "policy_denied"
    );
    set_policy(&env, false, true).await;
    let mut realtime = env.changes.subscribe();
    mcp_ok(&env, &owner, "workflow_state_delete", json!({"projectId": project_id, "stateId": state_id, "expectedWorkflowVersion": version + 1})).await;
    let mut saw = false;
    while let Ok(notice) = realtime.try_recv() {
        saw |= format!("{notice:?}").contains(&project_id);
    }
    assert!(saw, "open boards are told, like after the web route");
    // The Workspace of the call is in the call log and the audit entry (resolved from the Project).
    let logged: Option<Uuid> = sqlx::query_scalar("SELECT workspace_id FROM penfii.mcp_call_log WHERE operation = 'workflow.delete_state' AND outcome = 'ok' LIMIT 1")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(logged, Some(env.workspace_id));
    let audited: Option<Uuid> = sqlx::query_scalar("SELECT workspace_id FROM penfii.audit_entries WHERE action_code = 'mcp.workflow_state_delete' AND outcome_code = 'OK' LIMIT 1")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(audited, Some(env.workspace_id));
}
