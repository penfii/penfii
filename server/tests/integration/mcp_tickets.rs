//! MCP ticket operations against the router (FEA-164). Helpers: `crate::mcp_oauth`.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, api, modern_call, modern_list, rest, setup, token_for, tool_names,
};
use crate::mcp_support::{mcp_err, mcp_ok};

const SCOPES: &str =
    "workspace:read member:read ticket:read ticket:write project:read project:write";

async fn new_project(env: &Env, name: &str) -> String {
    let project = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": name, "teamIds": []})),
    )
    .await;
    project.body["id"].as_str().unwrap().to_string()
}

async fn user_id(env: &Env, email: &str) -> String {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM penfii.users WHERE email_key = $1")
        .bind(email)
        .fetch_one(&env.pool)
        .await
        .unwrap()
        .to_string()
}

async fn grant(env: &Env, project_id: &str, email: &str, role: &str) {
    let uid = user_id(env, email).await;
    let reply = rest(
        env,
        "PUT",
        &format!("/v1/projects/{project_id}/grants/{uid}"),
        &env.owner_cookie,
        Some(&format!("grant-{uid}")),
        Some(json!({"role": role})),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
}

async fn set_policy(env: &Env, write: bool, destructive: bool, administrative: bool) {
    let put = api(
        &env.app,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.owner_cookie),
        Some(json!({"mcpEnabled": true, "allowWrite": write,
                    "allowDestructive": destructive, "allowAdministrative": administrative})),
    )
    .await;
    assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
}

async fn web_ticket(env: &Env, ticket_id: &str) -> Value {
    rest(
        env,
        "GET",
        &format!("/v1/tickets/{ticket_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await
    .body
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn every_ticket_field_round_trips_and_a_ticket_moves_under_another_parent(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Fields").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;

    // Settings the fields depend on, all through MCP.
    let project = mcp_ok(
        &env,
        &owner,
        "project_update",
        json!({"projectId": project_id, "estimateUnit": "POINTS"}),
    )
    .await;
    assert_eq!(project["estimateUnit"], "POINTS");
    let day = json!([{"from": "09:00", "to": "18:00"}]);
    mcp_ok(
        &env,
        &owner,
        "project_calendar_put",
        json!({"projectId": project_id, "expectedVersion": 0, "timeZone": "UTC",
               "definition": {"version": 1, "week": {"mon": day, "tue": day, "wed": day, "thu": day, "fri": day}, "holidays": []}}),
    )
    .await;
    let label = mcp_ok(
        &env,
        &owner,
        "label_create",
        json!({"projectId": project_id, "name": "backend", "color": "#3366ff"}),
    )
    .await;
    let label_id = label["id"].as_str().unwrap().to_string();
    let types = mcp_ok(
        &env,
        &owner,
        "project_ticket_types",
        json!({"projectId": project_id}),
    )
    .await;
    let type_id = types["ticketTypes"][0]["id"].as_str().unwrap().to_string();

    let epic = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "Epic A", "hierarchyKind": "EPIC"}),
    )
    .await;
    let epic2 = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "Epic B", "hierarchyKind": "EPIC"}),
    )
    .await;
    let task_args = json!({"projectId": project_id, "title": "Task", "description": "body", "hierarchyKind": "TASK",
               "parentId": epic["id"], "priority": "HIGH", "dueOn": "2026-12-31",
               "labelIds": [label_id], "typeId": type_id,
               "estimate": {"value": "5"},
               "schedule": {"plannedStartAt": "2026-12-07T09:00:00Z", "durationMinutes": 480},
               "idempotencyKey": "task-1"});
    let task = mcp_ok(&env, &owner, "ticket_create", task_args.clone()).await;
    let task_id = task["id"].as_str().unwrap().to_string();
    // Same key, same Ticket.
    let again = mcp_ok(&env, &owner, "ticket_create", task_args.clone()).await;
    assert_eq!(again["id"], task["id"]);
    // The same key with other content is refused, never a second Ticket.
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_create",
            json!({"projectId": project_id, "title": "Other", "idempotencyKey": "task-1"})
        )
        .await,
        "idempotency_conflict"
    );

    // What MCP wrote is what the web app reads.
    let web = web_ticket(&env, &task_id).await;
    assert_eq!(web["parentId"], epic["id"]);
    assert_eq!(web["priority"], "HIGH");
    assert_eq!(web["dueOn"], "2026-12-31");
    assert_eq!(web["labels"][0]["id"], label_id.as_str());
    assert_eq!(web["ticketType"]["id"], type_id.as_str());
    assert_eq!(web["estimate"]["value"], "5");
    assert_eq!(web["schedule"]["durationMinutes"], 480);
    let via_mcp = mcp_ok(&env, &owner, "ticket_get", json!({"ticketId": task_id})).await;
    assert_eq!(via_mcp["estimate"], web["estimate"]);
    assert_eq!(
        via_mcp["schedule"]["plannedStartAt"],
        web["schedule"]["plannedStartAt"]
    );

    // Move under another Epic (the FEA-157 under FEA-158 case), then clear every optional field.
    let revision = web["revisionNo"].as_i64().unwrap();
    let moved = mcp_ok(
        &env,
        &owner,
        "ticket_update",
        json!({"ticketId": task_id, "expectedRevision": revision, "parentId": epic2["id"]}),
    )
    .await;
    assert_eq!(moved["parentId"], epic2["id"]);
    let cleared = mcp_ok(
        &env,
        &owner,
        "ticket_update",
        json!({"ticketId": task_id, "expectedRevision": moved["revisionNo"], "labelIds": [],
               "typeId": null, "estimate": null, "schedule": null, "priority": null, "dueOn": null}),
    )
    .await;
    assert_eq!(cleared["labels"], json!([]));
    assert!(cleared["estimate"].is_null() && cleared["schedule"].is_null());
    assert!(cleared["ticketType"].is_null() && cleared["priority"].is_null());
    // Stale revision: refused, nothing written.
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_update",
            json!({"ticketId": task_id, "expectedRevision": revision, "title": "stale"})
        )
        .await,
        "version_conflict"
    );
    // A wrong-typed field is an argument error, not a silent default.
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_update",
            json!({"ticketId": task_id, "expectedRevision": cleared["revisionNo"], "labelIds": "x"})
        )
        .await,
        "invalid_arguments"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn lists_history_comments_and_restore_follow_the_routes(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "History").await;
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    grant(&env, &project_id, "member@penfii.test", "EDITOR").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let member = token_for(&env, &member_cookie, SCOPES).await;
    let _ = member_cookie;

    let epic = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "Parent", "hierarchyKind": "EPIC"}),
    )
    .await;
    let mut ids = Vec::new();
    for n in 0..3 {
        let ticket = mcp_ok(
            &env,
            &owner,
            "ticket_create",
            json!({"projectId": project_id, "title": format!("Item {n}"), "parentId": epic["id"], "priority": "LOW"}),
        )
        .await;
        ids.push(ticket);
    }
    let workflow = mcp_ok(
        &env,
        &owner,
        "project_workflow",
        json!({"projectId": project_id}),
    )
    .await;
    let done_state = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .clone();

    // ticket_list: filters and pages; items are compact.
    let all = mcp_ok(
        &env,
        &owner,
        "ticket_list",
        json!({"projectId": project_id}),
    )
    .await;
    assert_eq!(all["total"], 4);
    assert!(all["items"][0].get("description").is_none());
    let page1 = mcp_ok(
        &env,
        &owner,
        "ticket_list",
        json!({"projectId": project_id, "limit": 3}),
    )
    .await;
    assert_eq!(page1["items"].as_array().unwrap().len(), 3);
    let cursor = page1["nextCursor"].as_str().unwrap();
    let page2 = mcp_ok(
        &env,
        &owner,
        "ticket_list",
        json!({"projectId": project_id, "limit": 3, "cursor": cursor}),
    )
    .await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert!(page2["nextCursor"].is_null());
    let by_parent = mcp_ok(
        &env,
        &owner,
        "ticket_list",
        json!({"projectId": project_id, "parentId": epic["id"], "text": "item 1"}),
    )
    .await;
    assert_eq!(by_parent["total"], 1);
    let children = mcp_ok(
        &env,
        &owner,
        "ticket_children_list",
        json!({"ticketId": epic["id"]}),
    )
    .await;
    assert_eq!(children["items"].as_array().unwrap().len(), 3);
    assert_eq!(
        mcp_ok(
            &env,
            &owner,
            "ticket_list",
            json!({"projectId": project_id, "statusId": done_state})
        )
        .await["total"],
        0
    );

    // Revisions, activity and restore.
    let first = &ids[0];
    let id = first["id"].as_str().unwrap();
    let renamed = mcp_ok(
        &env,
        &owner,
        "ticket_update",
        json!({"ticketId": id, "expectedRevision": first["revisionNo"], "title": "Renamed"}),
    )
    .await;
    let revisions = mcp_ok(
        &env,
        &owner,
        "ticket_revisions_list",
        json!({"ticketId": id}),
    )
    .await;
    assert!(revisions["items"].as_array().unwrap().len() >= 2);
    let old = mcp_ok(
        &env,
        &owner,
        "ticket_revision_get",
        json!({"ticketId": id, "revisionNo": first["revisionNo"]}),
    )
    .await;
    assert_eq!(old["revisionNo"], first["revisionNo"]);
    let to_current: Vec<&str> = old["changesToCurrent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap())
        .collect();
    assert!(to_current.contains(&"title"), "{old}");
    let activity = mcp_ok(
        &env,
        &owner,
        "ticket_activity_list",
        json!({"ticketId": id, "limit": 5}),
    )
    .await;
    assert!(!activity["items"].as_array().unwrap().is_empty());
    let restored = mcp_ok(
        &env,
        &owner,
        "ticket_restore_revision",
        json!({"ticketId": id, "expectedRevision": renamed["revisionNo"], "sourceRevision": first["revisionNo"], "fields": ["title"]}),
    )
    .await;
    assert_eq!(restored["title"], "Item 0");
    assert_eq!(
        restored["revisionNo"].as_i64().unwrap(),
        renamed["revisionNo"].as_i64().unwrap() + 1
    );
    // The old revision is still there, unchanged.
    let still = mcp_ok(
        &env,
        &owner,
        "ticket_revision_get",
        json!({"ticketId": id, "revisionNo": first["revisionNo"]}),
    )
    .await;
    assert_eq!(still["revisionNo"], first["revisionNo"]);
    assert!(
        still["changesToCurrent"].as_array().unwrap().is_empty(),
        "restored: nothing differs now: {still}"
    );
    assert_eq!(
        mcp_err(&env, &owner, "ticket_restore_revision", json!({"ticketId": id, "expectedRevision": 1, "sourceRevision": first["revisionNo"], "fields": ["title"]})).await,
        "version_conflict"
    );

    // Comments: the author edits, others may not; an admin may delete.
    let comment = mcp_ok(
        &env,
        &member,
        "ticket_comment",
        json!({"ticketId": id, "body": "first"}),
    )
    .await;
    let comment_id = comment["id"].as_str().unwrap();
    let edited = mcp_ok(
        &env,
        &member,
        "ticket_comment_edit",
        json!({"ticketId": id, "commentId": comment_id, "body": "second"}),
    )
    .await;
    assert_eq!(edited["id"], comment["id"]);
    let owner_comment = mcp_ok(
        &env,
        &owner,
        "ticket_comment",
        json!({"ticketId": id, "body": "owner"}),
    )
    .await;
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "ticket_comment_edit",
            json!({"ticketId": id, "commentId": owner_comment["id"], "body": "tamper"})
        )
        .await,
        "forbidden"
    );
    // Deleting a comment cannot be undone: Destructive, off under the standard policy.
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "ticket_comment_delete",
            json!({"ticketId": id, "commentId": owner_comment["id"]})
        )
        .await,
        "policy_denied"
    );
    set_policy(&env, true, true, false).await;
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "ticket_comment_delete",
            json!({"ticketId": id, "commentId": owner_comment["id"]})
        )
        .await,
        "forbidden"
    );
    mcp_ok(
        &env,
        &owner,
        "ticket_comment_delete",
        json!({"ticketId": id, "commentId": comment_id}),
    )
    .await;
    let remaining = mcp_ok(
        &env,
        &owner,
        "ticket_comments_list",
        json!({"ticketId": id}),
    )
    .await;
    let list = remaining["comments"].as_array().unwrap();
    assert!(list.iter().all(|c| c["id"] != comment["id"]), "{list:?}");

    // Reordering: the same order the web route gives.
    let last = ids[2]["id"].as_str().unwrap();
    mcp_ok(
        &env,
        &owner,
        "ticket_move",
        json!({"ticketId": last, "position": "first"}),
    )
    .await;
    let order = mcp_ok(
        &env,
        &owner,
        "ticket_list",
        json!({"projectId": project_id}),
    )
    .await;
    assert_eq!(order["items"][0]["id"], last);
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_move",
            json!({"ticketId": last, "position": "first", "afterId": id})
        )
        .await,
        "invalid_arguments"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn trash_archive_and_bulk_keep_the_project_admin_rules_and_purge_stays_in_the_web_app(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Lifecycle").await;
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    grant(&env, &project_id, "member@penfii.test", "EDITOR").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let member = token_for(&env, &member_cookie, SCOPES).await;
    let outsider = token_for(&env, &env.outsider_cookie, SCOPES).await;
    let t1 = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "One"}),
    )
    .await;
    let t2 = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "Two"}),
    )
    .await;
    let (id1, id2) = (t1["id"].as_str().unwrap(), t2["id"].as_str().unwrap());

    // An editor may not trash or archive (Project admins only, like the web routes).
    for tool in ["ticket_trash", "ticket_archive"] {
        assert_eq!(
            mcp_err(&env, &member, tool, json!({"ticketId": id1})).await,
            "forbidden",
            "{tool}"
        );
    }
    // The outsider learns nothing.
    for tool in [
        "ticket_trash",
        "ticket_children_list",
        "ticket_revisions_list",
        "ticket_activity_list",
    ] {
        assert_eq!(
            mcp_err(&env, &outsider, tool, json!({"ticketId": id1})).await,
            "not_found",
            "{tool}"
        );
    }
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "ticket_list",
            json!({"projectId": project_id})
        )
        .await,
        "not_found"
    );

    // Trash hides the Ticket from lists and from non-admins; restore brings it back.
    mcp_ok(&env, &owner, "ticket_trash", json!({"ticketId": id1})).await;
    assert_eq!(
        mcp_ok(
            &env,
            &owner,
            "ticket_list",
            json!({"projectId": project_id})
        )
        .await["total"],
        1
    );
    assert_eq!(
        mcp_err(&env, &member, "ticket_get", json!({"ticketId": id1})).await,
        "not_found"
    );
    assert_eq!(
        mcp_err(&env, &owner, "ticket_trash", json!({"ticketId": id1})).await,
        "already_trashed"
    );
    let trash = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/trash"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(trash.body["items"].as_array().unwrap().len(), 1);
    mcp_ok(
        &env,
        &owner,
        "ticket_restore_from_trash",
        json!({"ticketId": id1}),
    )
    .await;
    assert_eq!(
        mcp_ok(
            &env,
            &owner,
            "ticket_list",
            json!({"projectId": project_id})
        )
        .await["total"],
        2
    );

    // Archive and unarchive; archived Tickets leave the default list.
    mcp_ok(&env, &owner, "ticket_archive", json!({"ticketId": id2})).await;
    mcp_ok(&env, &owner, "ticket_archive", json!({"ticketId": id2})).await;
    assert_eq!(
        mcp_ok(
            &env,
            &owner,
            "ticket_list",
            json!({"projectId": project_id})
        )
        .await["total"],
        1
    );
    mcp_ok(&env, &owner, "ticket_unarchive", json!({"ticketId": id2})).await;
    assert_eq!(
        mcp_ok(
            &env,
            &owner,
            "ticket_list",
            json!({"projectId": project_id})
        )
        .await["total"],
        2
    );

    // Bulk: one change over several Tickets, every Ticket in the result; same retry key, same result.
    let args = json!({"workspaceId": env.workspace_id, "ticketIds": [id1, id2],
                      "change": {"type": "PRIORITY", "priority": "URGENT"}, "idempotencyKey": "bulk-1"});
    let bulk = mcp_ok(&env, &owner, "ticket_bulk_update", args.clone()).await;
    assert!(bulk.to_string().contains(id1), "{bulk}");
    assert_eq!(
        mcp_ok(&env, &owner, "ticket_get", json!({"ticketId": id1})).await["priority"],
        "URGENT"
    );
    mcp_ok(&env, &owner, "ticket_bulk_update", args).await;
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_bulk_update",
            json!({"workspaceId": env.workspace_id, "ticketIds": [id1], "change": {"type": "NOPE"}})
        )
        .await,
        "invalid_arguments"
    );
    // A member's bulk archive is refused per the web rules (the route returns per-Ticket failures).
    let denied = modern_call(
        &env.app,
        &member,
        "ticket_bulk_update",
        json!({"workspaceId": env.workspace_id, "ticketIds": [id1], "change": {"type": "TRASH"}}),
    )
    .await;
    let still_live = mcp_ok(&env, &owner, "ticket_get", json!({"ticketId": id1})).await;
    assert!(
        still_live["id"] == id1,
        "member bulk trash must not trash: {:?}",
        denied.body
    );

    // Permanent delete is web-only: no tool, and the route needs a recent sign-in.
    let listed = modern_list(&env.app, &owner).await;
    let names = tool_names(&listed);
    assert!(names.iter().all(|n| !n.contains("purge")), "{names:?}");
    assert!(names.contains(&"ticket_trash".to_string()));
    let purge = modern_call(&env.app, &owner, "ticket_purge", json!({"ticketId": id1})).await;
    assert_eq!(purge.body["error"]["code"], -32602, "{:?}", purge.body);
    let remains: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.tickets WHERE id = $1 AND purged_at IS NULL",
    )
    .bind(Uuid::parse_str(id1).unwrap())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(remains, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scopes_policy_and_other_workspaces_gate_the_ticket_tools(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Gates").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let t = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "T"}),
    )
    .await;
    let id = t["id"].as_str().unwrap();

    // A read-only token also discovers the write tools, and a call to one names the missing scope.
    // (A new consent replaces the grant, so the full token is minted again afterwards.)
    let reader = token_for(&env, &env.owner_cookie, "workspace:read ticket:read").await;
    let names = tool_names(&modern_list(&env.app, &reader).await);
    assert!(names.contains(&"ticket_list".to_string()));
    assert!(names.contains(&"ticket_revisions_list".to_string()));
    for listed in [
        "ticket_trash",
        "ticket_bulk_update",
        "ticket_comment_edit",
        "label_create",
    ] {
        assert!(names.contains(&listed.to_string()), "{listed}");
    }
    assert_eq!(
        mcp_err(&env, &reader, "ticket_trash", json!({"ticketId": id})).await,
        "insufficient_scope"
    );
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;

    // The Workspace policy turns writes off; reads keep working.
    set_policy(&env, false, false, false).await;
    assert_eq!(
        mcp_err(&env, &owner, "ticket_trash", json!({"ticketId": id})).await,
        "policy_denied"
    );
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_comment_edit",
            json!({"ticketId": id, "commentId": Uuid::new_v4(), "body": "x"})
        )
        .await,
        "policy_denied"
    );
    mcp_ok(
        &env,
        &owner,
        "ticket_list",
        json!({"projectId": project_id}),
    )
    .await;
    set_policy(&env, true, false, false).await;

    // The call log carries the Workspace for Ticket-addressed tools (the Ticket resolves it).
    mcp_ok(
        &env,
        &owner,
        "ticket_children_list",
        json!({"ticketId": id}),
    )
    .await;
    let logged: Option<Uuid> = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.mcp_call_log WHERE operation = 'ticket.children' LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(logged, Some(env.workspace_id));
}
