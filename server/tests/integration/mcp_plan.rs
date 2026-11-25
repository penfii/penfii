//! MCP planning operations (FEA-167/168) against the router: effects equal the web routes, Core's
//! rules and permissions hold, history stays put, and reads are bounded.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, modern_list, rest, setup, structured, token_for,
    tool_names,
};
use crate::mcp_support::{mcp_err, mcp_ok};

const PLAN_SCOPES: &str = "workspace:read ticket:read ticket:write plan:read plan:write";

async fn project(env: &Env, name: &str) -> String {
    let created = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        Some(&format!("k-proj-{name}")),
        Some(json!({"name": name, "teamIds": []})),
    )
    .await;
    created.body["id"].as_str().unwrap().to_string()
}

async fn ticket(env: &Env, project_id: &str, title: &str) -> String {
    let created = rest(
        env,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        &env.owner_cookie,
        Some(&format!("k-ticket-{title}")),
        Some(json!({"title": title, "description": null, "hierarchyKind": "TASK", "fileIds": []})),
    )
    .await;
    assert_eq!(created.status, 201, "{}", created.body);
    created.body["id"].as_str().unwrap().to_string()
}

fn planning_sprints(planning: &Value) -> Vec<Value> {
    planning["sprints"].as_array().cloned().unwrap_or_default()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sprint_lifecycle_matches_the_web_and_keeps_history(pool: PgPool) {
    let env = setup(pool).await;
    let token = token_for(&env, &env.owner_cookie, PLAN_SCOPES).await;
    let project_id = project(&env, "Plan").await;
    let done = ticket(&env, &project_id, "finished work").await;
    let open = ticket(&env, &project_id, "open work").await;

    let args = json!({"projectId": project_id, "name": "S1", "goal": "ship", "startsOn": "2026-11-23",
                      "endsOn": "2026-12-04", "idempotencyKey": "sprint-1"});
    let sprint = mcp_ok(&env, &token, "sprint_create", args.clone()).await;
    let again = mcp_ok(&env, &token, "sprint_create", args).await;
    assert_eq!(sprint["id"], again["id"], "same key, same Sprint");
    let sprint_id = sprint["id"].as_str().unwrap().to_string();
    let version = sprint["version"].as_i64().unwrap();

    // plan_get is the planning route's answer.
    let web = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/planning"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let via_mcp = mcp_ok(&env, &token, "plan_get", json!({"projectId": project_id})).await;
    assert_eq!(via_mcp["sprints"], web.body["sprints"]);
    assert_eq!(planning_sprints(&via_mcp).len(), 1);

    // A stale version is refused and changes nothing.
    let stale = mcp_err(
        &env,
        &token,
        "sprint_update",
        json!({"sprintId": sprint_id, "expectedVersion": version + 5, "name": "nope"}),
    )
    .await;
    assert!(!stale.is_empty());
    let renamed = mcp_ok(&env, &token, "sprint_update",
        json!({"sprintId": sprint_id, "expectedVersion": version, "name": "S1 renamed", "goal": null})).await;
    assert_eq!(renamed["name"], "S1 renamed");
    assert!(renamed["goal"].is_null());

    let started = mcp_ok(&env, &token, "sprint_start",
        json!({"sprintId": sprint_id, "expectedVersion": renamed["version"], "idempotencyKey": "start-1"})).await;
    assert_eq!(started["state"], "ACTIVE", "{started}");
    for t in [&done, &open] {
        mcp_ok(
            &env,
            &token,
            "ticket_assign_sprint",
            json!({"ticketId": t, "sprintId": sprint_id, "idempotencyKey": format!("as-{t}")}),
        )
        .await;
    }
    // Finish one Task through the ordinary status tool.
    let workflow = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let done_state = workflow.body["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .clone();
    let t = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{done}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "ticket_change_status",
        json!({"ticketId": done, "expectedRevision": t.body["revisionNo"], "statusId": done_state}),
    )
    .await;

    let preview = mcp_ok(
        &env,
        &token,
        "sprint_close_preview",
        json!({"sprintId": sprint_id}),
    )
    .await;
    let web_preview = rest(
        &env,
        "GET",
        &format!("/v1/sprints/{sprint_id}/close-preview"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(preview, web_preview.body);

    let closed = mcp_ok(&env, &token, "sprint_complete",
        json!({"sprintId": sprint_id, "expectedVersion": started["version"], "archive": "NONE", "idempotencyKey": "close-1"})).await;
    assert_eq!(closed["state"], "COMPLETED", "{closed}");
    let results = mcp_ok(
        &env,
        &token,
        "sprint_results",
        json!({"sprintId": sprint_id}),
    )
    .await;
    let web_results = rest(
        &env,
        "GET",
        &format!("/v1/sprints/{sprint_id}/results"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(results, web_results.body);
    let report = mcp_ok(
        &env,
        &token,
        "sprint_report",
        json!({"sprintId": sprint_id}),
    )
    .await;
    assert!(report.is_object());

    // Historical figures do not move when the work changes later.
    let t = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{open}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "ticket_change_status",
        json!({"ticketId": open, "expectedRevision": t.body["revisionNo"], "statusId": done_state}),
    )
    .await;
    let after = mcp_ok(
        &env,
        &token,
        "sprint_results",
        json!({"sprintId": sprint_id}),
    )
    .await;
    assert_eq!(after, results, "closed Sprint results are frozen");

    // A closed Sprint cannot be started again.
    assert!(
        !mcp_err(
            &env,
            &token,
            "sprint_start",
            json!({"sprintId": sprint_id, "expectedVersion": closed["version"]})
        )
        .await
        .is_empty()
    );

    // The call log names the Workspace of calls that name only a Sprint.
    let logged: Option<Uuid> = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.mcp_call_log WHERE operation = 'sprint.start' ORDER BY at LIMIT 1",
    ).fetch_one(&env.pool).await.unwrap();
    assert_eq!(logged, Some(env.workspace_id));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn milestones_dependencies_and_views_follow_core_rules(pool: PgPool) {
    let env = setup(pool).await;
    let token = token_for(&env, &env.owner_cookie, PLAN_SCOPES).await;
    let project_id = project(&env, "Deps").await;
    let a = ticket(&env, &project_id, "A").await;
    let b = ticket(&env, &project_id, "B").await;

    let milestone = mcp_ok(&env, &token, "milestone_create",
        json!({"projectId": project_id, "name": "M1", "targetOn": "2026-12-31", "idempotencyKey": "m1"})).await;
    let milestone_id = milestone["id"].as_str().unwrap().to_string();
    let set = mcp_ok(
        &env,
        &token,
        "ticket_set_milestones",
        json!({"ticketId": a, "milestoneIds": [milestone_id], "idempotencyKey": "sm1"}),
    )
    .await;
    assert_eq!(set["milestones"].as_array().map(Vec::len), Some(1), "{set}");
    // A Ticket can only be linked to an open Milestone: Core says so through the tool as well.
    let updated = mcp_ok(&env, &token, "milestone_update",
        json!({"milestoneId": milestone_id, "expectedVersion": milestone["version"], "state": "REACHED"})).await;
    assert_eq!(updated["state"], "REACHED");
    assert_eq!(
        mcp_err(
            &env,
            &token,
            "ticket_set_milestones",
            json!({"ticketId": b, "milestoneIds": [milestone_id], "idempotencyKey": "sm2"})
        )
        .await,
        "invalid_arguments"
    );

    // B depends on A; the reverse would be a cycle.
    let added = mcp_ok(
        &env,
        &token,
        "dependency_add",
        json!({"ticketId": b, "predecessorId": a, "idempotencyKey": "d1"}),
    )
    .await;
    assert!(added.is_object());
    assert!(
        !mcp_err(
            &env,
            &token,
            "dependency_add",
            json!({"ticketId": a, "predecessorId": b, "idempotencyKey": "d2"})
        )
        .await
        .is_empty()
    );
    let listed = mcp_ok(&env, &token, "dependency_list", json!({"ticketId": b})).await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{b}/dependencies"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(listed, web.body);
    let dependency_id = listed["blockedBy"][0]["id"]
        .as_str()
        .or_else(|| listed["blockedBy"][0]["dependencyId"].as_str())
        .unwrap_or_else(|| panic!("dependency id in {listed}"))
        .to_string();
    mcp_ok(
        &env,
        &token,
        "dependency_waive",
        json!({"dependencyId": dependency_id, "reason": "ok", "idempotencyKey": "w1"}),
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "dependency_restore",
        json!({"dependencyId": dependency_id, "reason": "again", "idempotencyKey": "r1"}),
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "dependency_remove",
        json!({"dependencyId": dependency_id}),
    )
    .await;
    let listed = mcp_ok(&env, &token, "dependency_list", json!({"ticketId": b})).await;
    assert_eq!(listed["blockedBy"].as_array().map(Vec::len), Some(0));

    // Saved views: create, list, stale update refused, delete.
    let view = mcp_ok(&env, &token, "view_create",
        json!({"projectId": project_id, "name": "Mine", "layout": "LIST", "definition": {}, "idempotencyKey": "v1"})).await;
    let view_id = view["id"].as_str().unwrap().to_string();
    let views = mcp_ok(&env, &token, "view_list", json!({"projectId": project_id})).await;
    assert!(
        views["views"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == json!(view_id))
    );
    assert!(
        !mcp_err(
            &env,
            &token,
            "view_update",
            json!({"viewId": view_id, "expectedVersion": 99, "name": "x"})
        )
        .await
        .is_empty()
    );
    let renamed = mcp_ok(
        &env,
        &token,
        "view_update",
        json!({"viewId": view_id, "expectedVersion": view["version"], "name": "Renamed"}),
    )
    .await;
    assert_eq!(renamed["name"], "Renamed");
    mcp_ok(
        &env,
        &token,
        "view_delete",
        json!({"viewId": view_id, "expectedVersion": renamed["version"]}),
    )
    .await;
    let views = mcp_ok(&env, &token, "view_list", json!({"projectId": project_id})).await;
    assert!(views["views"].as_array().unwrap().is_empty());
    // Workspace-wide views.
    let wide = mcp_ok(&env, &token, "view_create_workspace",
        json!({"workspaceId": env.workspace_id, "targetKind": "WORKSPACE", "name": "All", "idempotencyKey": "vw1"})).await;
    let wide_list = mcp_ok(
        &env,
        &token,
        "view_list_workspace",
        json!({"workspaceId": env.workspace_id, "targetKind": "WORKSPACE"}),
    )
    .await;
    assert!(
        wide_list["views"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == wide["id"])
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn board_and_reports_are_bounded_and_hide_trashed_work(pool: PgPool) {
    let env = setup(pool).await;
    let token = token_for(&env, &env.owner_cookie, PLAN_SCOPES).await;
    let project_id = project(&env, "Board").await;
    let first = ticket(&env, &project_id, "alpha-visible").await;
    ticket(&env, &project_id, "beta-visible").await;
    let secret = ticket(&env, &project_id, "gamma-trashed").await;

    let board = mcp_ok(
        &env,
        &token,
        "board_get",
        json!({"projectId": project_id, "perColumnLimit": 1}),
    )
    .await;
    let columns = board["columns"].as_array().unwrap();
    let first_column = columns
        .iter()
        .find(|c| c["ticketCount"].as_i64().unwrap_or(0) > 1)
        .expect("a column with several Tickets");
    assert_eq!(first_column["tickets"].as_array().unwrap().len(), 1);
    assert_eq!(first_column["ticketsTruncated"], true);

    // Trash one Ticket: it disappears from every read.
    let trashed = rest(
        &env,
        "POST",
        &format!("/v1/tickets/{secret}/commands/trash"),
        &env.owner_cookie,
        Some("trash-1"),
        Some(json!({})),
    )
    .await;
    assert_eq!(trashed.status, 204, "{}", trashed.body);
    for (tool, args) in [
        (
            "board_get",
            json!({"projectId": project_id, "perColumnLimit": 50}),
        ),
        ("work_list", json!({"workspaceId": env.workspace_id})),
        ("plan_schedule", json!({"projectId": project_id})),
        ("report_project_activity", json!({"projectId": project_id})),
        ("report_status_summary", json!({"projectId": project_id})),
    ] {
        let text = mcp_ok(&env, &token, tool, args).await.to_string();
        assert!(
            !text.contains("gamma-trashed"),
            "{tool} leaks a trashed Ticket"
        );
    }
    let summary = mcp_ok(
        &env,
        &token,
        "report_status_summary",
        json!({"projectId": project_id}),
    )
    .await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/status-summary"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let strip = |mut v: Value| {
        v.as_object_mut().unwrap().remove("generatedAt");
        v
    };
    assert_eq!(strip(summary), strip(web.body));
    let velocity = mcp_ok(
        &env,
        &token,
        "report_velocity",
        json!({"projectId": project_id}),
    )
    .await;
    assert!(velocity.is_object());

    // work_list pages by cursor; reading changes no state.
    let page1 = mcp_ok(
        &env,
        &token,
        "work_list",
        json!({"workspaceId": env.workspace_id, "limit": 1}),
    )
    .await;
    assert_eq!(page1["tickets"].as_array().unwrap().len(), 1);
    let cursor = page1["nextCursor"]
        .as_str()
        .expect("more pages")
        .to_string();
    let page2 = mcp_ok(
        &env,
        &token,
        "work_list",
        json!({"workspaceId": env.workspace_id, "limit": 1, "cursor": cursor}),
    )
    .await;
    assert_ne!(page1["tickets"][0]["id"], page2["tickets"][0]["id"]);
    assert_eq!(page1["total"], 2);
    let _ = first;
    // A bad lens is refused, not guessed.
    assert_eq!(
        mcp_err(
            &env,
            &token,
            "work_list",
            json!({"workspaceId": env.workspace_id, "lens": "EVERYTHING"})
        )
        .await,
        "invalid_arguments"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scopes_roles_and_policy_are_checked_for_planning_tools(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = project(&env, "Guarded").await;
    let reader = token_for(&env, &env.owner_cookie, "workspace:read plan:read").await;

    // (A new consent replaces the person's grant for the client, so the tokens are made one at a time.)
    // tools/list offers the write tools too; the scope is checked when they are called.
    let listed = modern_list(&env.app, &reader).await;
    assert!(listed.body["result"]["tools"].is_array(), "{}", listed.body);
    let names = tool_names(&listed);
    assert!(
        names.contains(&"board_get".to_string()) && names.contains(&"sprint_report".to_string())
    );
    assert!(names.contains(&"sprint_create".to_string()), "{names:?}");
    // A read-only connection cannot write, and is told which scope is missing.
    let denied = modern_call(&env.app, &reader, "sprint_create",
        json!({"projectId": project_id, "name": "x", "startsOn": "2026-11-23", "endsOn": "2026-11-30"})).await;
    assert_eq!(error_code(&denied), "insufficient_scope");
    let writer = token_for(&env, &env.owner_cookie, PLAN_SCOPES).await;
    let names = tool_names(&modern_list(&env.app, &writer).await);
    assert!(names.contains(&"sprint_create".to_string()));

    // A member without access to the (private) Project, and an outsider, get not_found.
    let member_cookie = add_member(&env, "plain@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, PLAN_SCOPES).await;
    assert_eq!(
        mcp_err(&env, &member, "board_get", json!({"projectId": project_id})).await,
        "not_found"
    );
    let outsider = token_for(&env, &env.outsider_cookie, PLAN_SCOPES).await;
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "plan_get",
            json!({"projectId": project_id})
        )
        .await,
        "not_found"
    );
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "work_list",
            json!({"workspaceId": env.workspace_id})
        )
        .await,
        "not_found"
    );

    // The Workspace policy can turn writes off; reads keep working.
    sqlx::query("INSERT INTO penfii.workspace_mcp_policy (workspace_id, mcp_enabled, allow_write, allow_destructive, allow_administrative, updated_by)
                 VALUES ($1, true, false, false, false, $2)")
        .bind(env.workspace_id).bind(env.owner_id).execute(&env.pool).await.unwrap();
    assert_eq!(mcp_err(&env, &writer, "sprint_create",
        json!({"projectId": project_id, "name": "x", "startsOn": "2026-11-23", "endsOn": "2026-11-30"})).await, "policy_denied");
    mcp_ok(&env, &writer, "plan_get", json!({"projectId": project_id})).await;
    let _ = structured;
}
