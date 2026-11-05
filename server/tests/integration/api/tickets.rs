//! Ticket, Board, workflow, bulk-change and work-scope behaviour through the API.

use super::access::grant_project;
use super::{
    World, call, get_as, grant, make_ticket, new_project, post_as, user_id_of, viewer_cookie,
    workspace,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn project_board_ticket_revision_activity_flow(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM penfii.workspace_members WHERE user_id=$1 AND state='ACTIVE' LIMIT 1")
        .bind(world.owner_id).fetch_one(&world.pool).await.unwrap();
    let (status, project, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"P4 board","teamIds":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, workflow, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflow}");
    let states = workflow["states"].as_array().unwrap();
    assert_eq!(states.len(), 4);
    let initial = states.iter().find(|s| s["isInitial"] == true).unwrap()["id"]
        .as_str()
        .unwrap();
    let doing = states
        .iter()
        .find(|s| s["category"] == "IN_PROGRESS")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (status, project_channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"public project channel","visibility":"PUBLIC","projectId":project_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project_channel}");
    let (status, channel_ticket, _) = call(&world.app,"POST",&format!("/v1/projects/{project_id}/tickets"),Some(&world.owner_cookie),Some("project-channel-ticket-create"),Some(json!({"title":"public channel work","hierarchyKind":"TASK","channelId":project_channel["id"]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{channel_ticket}");
    let (status, label, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/labels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"urgent","color":"#d44"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{label}");
    let (status, added_state, _) = call(&world.app,"POST",&format!("/v1/projects/{project_id}/workflow/states"),Some(&world.owner_cookie),None,Some(json!({"expectedWorkflowVersion":1,"name":"Review","category":"IN_PROGRESS","position":2}))).await;
    assert_eq!(status, StatusCode::CREATED, "{added_state}");
    assert_eq!(added_state["version"], 2);
    let (status, stale_workflow, _) = call(
        &world.app,
        "PATCH",
        &format!(
            "/v1/projects/{project_id}/workflow/states/{}",
            added_state["state"]["id"].as_str().unwrap()
        ),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedWorkflowVersion":1,"name":"Check"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale_workflow}");
    let (status, second_project, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"P4 other","teamIds":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second_project}");
    let second_project_id = second_project["id"].as_str().unwrap();
    let (status, second_workflow, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{second_project_id}/workflow"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_workflow}");
    let foreign_state = second_workflow["states"][0]["id"].as_str().unwrap();
    let (status, foreign_state_error, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        Some(&world.owner_cookie),
        Some("ticket-cross-state"),
        Some(json!({"title":"bad state","hierarchyKind":"TASK","statusId":foreign_state})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{foreign_state_error}");
    let (status, missing_parent, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        Some(&world.owner_cookie),
        Some("ticket-subtask-no-parent"),
        Some(json!({"title":"bad subtask","hierarchyKind":"SUBTASK"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing_parent}");
    let (status, ticket, _) = call(&world.app,"POST",&format!("/v1/projects/{project_id}/tickets"),Some(&world.owner_cookie),Some("ticket-create-1"),Some(json!({"title":"P4 task","hierarchyKind":"TASK","statusId":initial,"labelIds":[label["id"]],"description":{"type":"doc","content":[]}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    assert_eq!(ticket["revisionNo"], 1);
    assert_eq!(ticket["labels"][0]["name"], "urgent");
    let ticket_id = ticket["id"].as_str().unwrap();
    let (status, board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    assert!(board["columns"].as_array().unwrap().iter().any(|column| {
        column["tickets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == ticket["id"])
    }));
    let (status, updated, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{ticket_id}/commands/change-status"),
        Some(&world.owner_cookie),
        Some("ticket-move-1"),
        Some(json!({"expectedRevision":1,"statusId":doing})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["revisionNo"], 2);
    let (status, moved_board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved_board}");
    let old_column = moved_board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|column| column["stateId"] == initial)
        .unwrap();
    let new_column = moved_board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|column| column["stateId"] == doing)
        .unwrap();
    assert!(
        !old_column["tickets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == ticket["id"])
    );
    assert!(
        new_column["tickets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == ticket["id"])
    );
    let (status, revisions, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{ticket_id}/revisions"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revisions}");
    assert_eq!(revisions["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        revisions["items"][0]["revisionNo"], 2,
        "History is newest first"
    );
    assert_eq!(revisions["items"][0]["changedFields"], json!(["status"]));
    assert!(
        revisions["items"][0].get("snapshot").is_none(),
        "History must not expose raw snapshots"
    );
    let (status, activity, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/activity"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{activity}");
    assert!(
        activity
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["activityKind"] == "ticket.status_changed")
    );
    let (status, long_title, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{ticket_id}"),
        Some(&world.owner_cookie),
        Some("ticket-edit-too-long"),
        Some(json!({"expectedRevision":2,"title":"x".repeat(301)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{long_title}");
    let (status, conflict, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{ticket_id}/commands/change-status"),
        Some(&world.owner_cookie),
        Some("ticket-move-stale"),
        Some(json!({"expectedRevision":1,"statusId":initial})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["currentRevision"], 2);
    let (status, edited, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{ticket_id}"),
        Some(&world.owner_cookie),
        Some("ticket-edit-1"),
        Some(json!({"expectedRevision":2,"title":"Edited task"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["revisionNo"], 3);
    let (status, replay, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{ticket_id}"),
        Some(&world.owner_cookie),
        Some("ticket-edit-1"),
        Some(json!({"expectedRevision":2,"title":"Edited task"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["revisionNo"], 3);
    let (status, comment, _) = call(&world.app,"POST",&format!("/v1/tickets/{ticket_id}/comments"),Some(&world.owner_cookie),None,Some(json!({"body":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Looks good"}]}]}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{comment}");
    let (status, revisions, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{ticket_id}/revisions"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revisions}");
    assert_eq!(
        revisions["items"].as_array().unwrap().len(),
        3,
        "comment creation must not create a Ticket revision"
    );
    let (status, space, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/space"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{space}");
    let project_channel = space["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["projectId"] == project_id)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (status, channel, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/channels/{project_channel}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"visibility":"PRIVATE"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{channel}");
    let (status, private_ticket, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        Some(&world.owner_cookie),
        Some("private-ticket-create"),
        Some(json!({"title":"private work","hierarchyKind":"TASK","channelId":project_channel})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private_ticket}");
    let (status, invitation, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invitations"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"email":"board-viewer@penfii.test","role":"MEMBER"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invitation}");
    let (status, _, _) = call(&world.app,"POST","/v1/invitations/accept",None,None,Some(json!({"token":invitation["inviteToken"],"displayName":"Board Viewer","password":"board-viewer-password"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, viewer_cookie) = call(
        &world.app,
        "POST",
        "/v1/session/login",
        None,
        None,
        Some(json!({"email":"board-viewer@penfii.test","password":"board-viewer-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.unwrap();
    // Not taking part in the Project: it is found in the list but its board is not found.
    let (status, _, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    grant_project(world, project_id, "board-viewer@penfii.test", "VIEWER").await;
    let (status, denied_edit, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{ticket_id}"),
        Some(&viewer_cookie),
        Some("viewer-ticket-edit"),
        Some(json!({"expectedRevision":3,"title":"unauthorized edit"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied_edit}");
    let (status, viewer_board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_board}");
    assert!(!viewer_board.to_string().contains("private work"));
    // A participant reads the Project's public channel, so tickets made there are on the board.
    assert!(viewer_board.to_string().contains("public channel work"));
    let (status, hidden, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}", private_ticket["id"].as_str().unwrap()),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
    let (status, hidden_public_channel_ticket, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}", channel_ticket["id"].as_str().unwrap()),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hidden_public_channel_ticket}");
    let private_id = private_ticket["id"].as_str().unwrap();
    let (status, hidden_comments, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{private_id}/comments"),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden_comments}");
    let (status, hidden_comment_write, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{private_id}/comments"),
        Some(&viewer_cookie),
        None,
        Some(json!({"body":{"type":"doc","content":[]}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden_comment_write}");
    let (status, hidden_private_move, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{private_id}/commands/change-status"),
        Some(&viewer_cookie),
        Some("viewer-private-move"),
        Some(json!({"expectedRevision":1,"statusId":doing})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden_private_move}");
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        Some(&viewer_cookie),
        Some("viewer-ticket-create"),
        Some(json!({"title":"not allowed","hierarchyKind":"TASK"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn ticket_hierarchy_rules_and_child_progress(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM penfii.workspace_members WHERE user_id=$1 AND state='ACTIVE' LIMIT 1")
        .bind(world.owner_id).fetch_one(&world.pool).await.unwrap();
    let (project_id, workflow) = new_project(world, workspace_id, "Hierarchy").await;
    let (other_project_id, _) = new_project(world, workspace_id, "Hierarchy other").await;
    let states = workflow["states"].as_array().unwrap();
    let initial = states.iter().find(|s| s["isInitial"] == true).unwrap()["id"]
        .as_str()
        .unwrap();
    let done = states.iter().find(|s| s["category"] == "DONE").unwrap()["id"]
        .as_str()
        .unwrap();
    let canceled = states.iter().find(|s| s["category"] == "CANCELED").unwrap()["id"]
        .as_str()
        .unwrap();

    // Title-only create: TASK in the initial state, no description needed.
    let (status, quick) = make_ticket(
        world,
        &project_id,
        "h-quick",
        json!({"title":"빠르게 만든 작업"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{quick}");
    assert_eq!(quick["hierarchyKind"], "TASK");
    assert_eq!(quick["statusId"], initial);
    assert_eq!(quick["revisionNo"], 1);
    assert_eq!(
        quick["childSummary"],
        json!({"total":0,"done":0,"canceled":0})
    );
    // The title limit counts characters, not bytes.
    let (status, korean) = make_ticket(
        world,
        &project_id,
        "h-korean",
        json!({"title":"가".repeat(300)}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{korean}");
    let (status, too_long) = make_ticket(
        world,
        &project_id,
        "h-long",
        json!({"title":"가".repeat(301)}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{too_long}");

    let (status, epic) = make_ticket(
        world,
        &project_id,
        "h-epic",
        json!({"title":"Epic","hierarchyKind":"EPIC"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{epic}");
    let epic_id = epic["id"].as_str().unwrap();
    let (status, task) = make_ticket(
        world,
        &project_id,
        "h-task",
        json!({"title":"Task","hierarchyKind":"TASK","parentId":epic_id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(task["parent"]["id"], epic["id"]);
    assert_eq!(task["parent"]["hierarchyKind"], "EPIC");
    let (status, sub_a) = make_ticket(
        world,
        &project_id,
        "h-sub-a",
        json!({"title":"Sub A","hierarchyKind":"SUBTASK","parentId":task_id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sub_a}");
    let (status, sub_b) = make_ticket(
        world,
        &project_id,
        "h-sub-b",
        json!({"title":"Sub B","hierarchyKind":"SUBTASK","parentId":task_id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sub_b}");

    // Rejected shapes.
    for (key, body) in [
        (
            "h-bad-1",
            json!({"title":"Subtask under Epic","hierarchyKind":"SUBTASK","parentId":epic_id}),
        ),
        (
            "h-bad-2",
            json!({"title":"Task under Task","hierarchyKind":"TASK","parentId":task_id}),
        ),
        (
            "h-bad-3",
            json!({"title":"Epic with parent","hierarchyKind":"EPIC","parentId":epic_id}),
        ),
        (
            "h-bad-4",
            json!({"title":"Task under Subtask","hierarchyKind":"TASK","parentId":sub_a["id"]}),
        ),
        (
            "h-bad-5",
            json!({"title":"Subtask without parent","hierarchyKind":"SUBTASK"}),
        ),
        (
            "h-bad-6",
            json!({"title":"Unknown kind","hierarchyKind":"STORY"}),
        ),
    ] {
        let (status, value) = make_ticket(world, &project_id, key, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key}: {value}");
    }
    let (status, foreign_epic) = make_ticket(
        world,
        &other_project_id,
        "h-foreign-epic",
        json!({"title":"Foreign Epic","hierarchyKind":"EPIC"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign_epic}");
    let (status, cross) = make_ticket(
        world,
        &project_id,
        "h-cross",
        json!({"title":"Cross-project child","hierarchyKind":"TASK","parentId":foreign_epic["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cross}");
    let (status, project_channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"hierarchy channel","visibility":"PUBLIC","projectId":project_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project_channel}");
    let (status, epic_in_channel) = make_ticket(
        world,
        &project_id,
        "h-epic-channel",
        json!({"title":"Epic in channel","hierarchyKind":"EPIC","channelId":project_channel["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{epic_in_channel}");
    let (status, other_channel_sub) = make_ticket(world, &project_id, "h-sub-channel", json!({"title":"Subtask in another channel","hierarchyKind":"SUBTASK","parentId":task_id,"channelId":project_channel["id"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{other_channel_sub}");

    // Parent changes and clears follow the same rules and the revision gate.
    let sub_a_id = sub_a["id"].as_str().unwrap();
    let (status, self_parent, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        Some("h-self"),
        Some(json!({"expectedRevision":1,"parentId":task_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{self_parent}");
    let (status, cycle, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        Some("h-cycle"),
        Some(json!({"expectedRevision":1,"parentId":sub_a_id})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "descendant as parent: {cycle}"
    );
    let (status, epic_parent, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{epic_id}"),
        Some(&world.owner_cookie),
        Some("h-epic-parent"),
        Some(json!({"expectedRevision":1,"parentId":task_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{epic_parent}");
    let (status, orphan, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{sub_a_id}"),
        Some(&world.owner_cookie),
        Some("h-orphan"),
        Some(json!({"expectedRevision":1,"parentId":null})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Subtask parent is required: {orphan}"
    );
    let (status, moved, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{}", quick["id"].as_str().unwrap()),
        Some(&world.owner_cookie),
        Some("h-attach"),
        Some(json!({"expectedRevision":1,"parentId":epic_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["revisionNo"], 2);
    assert_eq!(moved["parent"]["publicKey"], epic["publicKey"]);
    let (status, detached, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{}", quick["id"].as_str().unwrap()),
        Some(&world.owner_cookie),
        Some("h-detach"),
        Some(json!({"expectedRevision":2,"parentId":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detached}");
    assert!(detached["parent"].is_null());
    let (status, stale, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{}", quick["id"].as_str().unwrap()),
        Some(&world.owner_cookie),
        Some("h-stale"),
        Some(json!({"expectedRevision":2,"parentId":epic_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["currentRevision"], 3);

    let (status, bad_priority, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        Some("h-priority"),
        Some(json!({"expectedRevision":1,"priority":"whenever"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_priority}");
    // Clearing optional fields with null really clears them.
    let (status, with_fields, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        Some("h-fields"),
        Some(json!({"expectedRevision":1,"priority":"HIGH","dueOn":"2026-12-24"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{with_fields}");
    assert_eq!(with_fields["priority"], "HIGH");
    let (status, cleared, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        Some("h-clear"),
        Some(json!({"expectedRevision":2,"priority":null,"dueOn":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert!(
        cleared["priority"].is_null() && cleared["dueOn"].is_null(),
        "{cleared}"
    );

    // Child progress is a count of visible children; no state is derived from it.
    let (status, task_now, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{task_now}");
    assert_eq!(
        task_now["childSummary"],
        json!({"total":2,"done":0,"canceled":0})
    );
    let (status, blocked, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{task_id}/commands/change-status"),
        Some(&world.owner_cookie),
        Some("h-done-early"),
        Some(json!({"expectedRevision":task_now["revisionNo"],"statusId":done})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    assert_eq!(blocked["code"], "unfinished_subtasks");
    let (status, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{sub_a_id}/commands/change-status"),
        Some(&world.owner_cookie),
        Some("h-sub-a-done"),
        Some(json!({"expectedRevision":1,"statusId":done})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/tickets/{}/commands/change-status",
            sub_b["id"].as_str().unwrap()
        ),
        Some(&world.owner_cookie),
        Some("h-sub-b-cancel"),
        Some(json!({"expectedRevision":1,"statusId":canceled})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, task_after, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        task_after["childSummary"],
        json!({"total":2,"done":1,"canceled":1})
    );
    assert_eq!(
        task_after["statusId"], initial,
        "children never change the parent's state"
    );
    let (status, children, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{task_id}/children"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{children}");
    assert_eq!(children.as_array().unwrap().len(), 2);
    let (status, epic_now, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{epic_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        epic_now["childSummary"]["total"], 1,
        "the Epic counts its Tasks only, not their Subtasks"
    );
    let (status, board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let on_board = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().iter())
        .find(|t| t["id"] == task_after["id"])
        .unwrap();
    assert_eq!(
        on_board["childSummary"],
        json!({"total":2,"done":1,"canceled":1})
    );
    assert_eq!(board["columns"][0]["stateIds"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn child_visibility_follows_channel_access(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM penfii.workspace_members WHERE user_id=$1 AND state='ACTIVE' LIMIT 1")
        .bind(world.owner_id).fetch_one(&world.pool).await.unwrap();
    let (project_id, _) = new_project(world, workspace_id, "Visibility").await;
    let (status, private_channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"hidden children","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private_channel}");
    let (status, epic) = make_ticket(
        world,
        &project_id,
        "v-epic",
        json!({"title":"Shared Epic","hierarchyKind":"EPIC"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{epic}");
    let epic_id = epic["id"].as_str().unwrap();
    let (status, open_task) = make_ticket(
        world,
        &project_id,
        "v-open",
        json!({"title":"Open task","hierarchyKind":"TASK","parentId":epic_id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{open_task}");
    let (status, secret_task) = make_ticket(world, &project_id, "v-secret", json!({"title":"Secret task","hierarchyKind":"TASK","parentId":epic_id,"channelId":private_channel["id"]})).await;
    assert_eq!(status, StatusCode::CREATED, "{secret_task}");
    let secret_id = secret_task["id"].as_str().unwrap();
    let (status, comment, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{secret_id}/comments"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"body":{"type":"doc","content":[]}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{comment}");

    let (_, owner_epic, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{epic_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(owner_epic["childSummary"]["total"], 2);
    let viewer = viewer_cookie(world, workspace_id, "visibility-viewer@penfii.test").await;
    grant_project(
        world,
        &project_id,
        "visibility-viewer@penfii.test",
        "VIEWER",
    )
    .await;
    let (status, viewer_epic, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{epic_id}"),
        Some(&viewer),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_epic}");
    assert_eq!(
        viewer_epic["childSummary"]["total"], 1,
        "a hidden child must not be counted"
    );
    let (status, viewer_children, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{epic_id}/children"),
        Some(&viewer),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_children}");
    assert_eq!(viewer_children.as_array().unwrap().len(), 1);
    assert!(!viewer_children.to_string().contains("Secret task"));
    let (status, viewer_board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&viewer),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!viewer_board.to_string().contains("Secret task"));
    let epic_on_viewer_board = viewer_board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().iter())
        .find(|t| t["id"] == epic["id"])
        .unwrap();
    assert_eq!(epic_on_viewer_board["childSummary"]["total"], 1);
    // History, Activity and revisions of a hidden ticket do not exist for this viewer.
    for path in [
        format!("/v1/tickets/{secret_id}/activity"),
        format!("/v1/tickets/{secret_id}/revisions"),
        format!("/v1/tickets/{secret_id}/revisions/1"),
        format!("/v1/tickets/{secret_id}/children"),
    ] {
        let (status, value, _) = call(&world.app, "GET", &path, Some(&viewer), None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {value}");
    }
    let (status, restore, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{secret_id}/commands/restore"),
        Some(&viewer),
        Some("v-restore"),
        Some(json!({"expectedRevision":1,"sourceRevision":1,"fields":["title"]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{restore}");
    let (status, parent_change, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{secret_id}"),
        Some(&viewer),
        Some("v-parent"),
        Some(json!({"expectedRevision":1,"parentId":null})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{parent_change}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn activity_history_diff_and_restore(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM penfii.workspace_members WHERE user_id=$1 AND state='ACTIVE' LIMIT 1")
        .bind(world.owner_id).fetch_one(&world.pool).await.unwrap();
    let (project_id, workflow) = new_project(world, workspace_id, "History").await;
    let doing = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "IN_PROGRESS")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, label, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/labels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"backend"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{label}");
    let (status, types, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/ticket-types"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{types}");
    let names: Vec<_> = types
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["BUGFIX", "FEATURE", "ISSUE"]);
    let bugfix = types
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "BUGFIX")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, members, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/members"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    assert!(
        members
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["userId"] == world.owner_id.to_string())
    );

    let (status, created) = make_ticket(
        world,
        &project_id,
        "a-create",
        json!({"title":"로그인 실패 조사","typeId":bugfix}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["ticketType"]["name"], "BUGFIX");
    let id = created["id"].as_str().unwrap();
    let patch = |rev: i64, key: &'static str, body: Value| {
        let mut body = body;
        body["expectedRevision"] = json!(rev);
        let path = format!("/v1/tickets/{id}");
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        async move {
            let (status, value, _) =
                call(&app, "PATCH", &path, Some(&cookie), Some(key), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{value}");
            value
        }
    };
    let r2 = patch(1, "a-p1", json!({"priority":"HIGH"})).await;
    assert_eq!(r2["revisionNo"], 2);
    let r3 = patch(
        2,
        "a-p2",
        json!({"assigneeId":world.owner_id,"labelIds":[label["id"]]}),
    )
    .await;
    assert_eq!(r3["assigneeName"], "Owner");
    let r4 = patch(3, "a-p3", json!({"title":"로그인 실패 원인 조사","description":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"첫 줄"}]},{"type":"paragraph","content":[{"type":"text","text":"둘째 줄"}]}]}})).await;
    assert_eq!(r4["revisionNo"], 4);
    let (status, moved, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/change-status"),
        Some(&world.owner_cookie),
        Some("a-move"),
        Some(json!({"expectedRevision":4,"statusId":doing})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    let (status, commented, _) = call(&world.app,"POST",&format!("/v1/tickets/{id}/comments"),Some(&world.owner_cookie),None,Some(json!({"body":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"확인했습니다"}]}]}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{commented}");
    assert_eq!(commented["authorName"], "Owner");

    // Activity: readable events with actor and before/after, newest first, paged.
    let (status, page1, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/activity?limit=3"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page1}");
    let items = page1["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["activityKind"], "ticket.commented");
    assert_eq!(items[0]["actorName"], "Owner");
    assert_eq!(items[1]["activityKind"], "ticket.status_changed");
    assert_eq!(
        items[1]["changes"],
        json!([{"field":"status","from":"Todo","to":"In Progress"}])
    );
    assert_eq!(
        items[2]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["field"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["title", "description"]
    );
    let cursor = page1["nextCursor"].as_str().expect("more activity remains");
    let (status, page2, _) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/tickets/{id}/activity?limit=3&before={}",
            cursor.replace('|', "%7C")
        ),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page2}");
    let second = page2["items"].as_array().unwrap();
    assert_eq!(second.len(), 3);
    assert_eq!(second[0]["changes"][0]["field"], "assignee");
    assert_eq!(second[0]["changes"][0]["to"], "Owner");
    assert_eq!(second[2]["activityKind"], "ticket.created");
    assert!(page2["nextCursor"].is_null());
    let (status, bad_cursor, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/activity?before=nonsense"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_cursor}");

    // History: revision metadata only, newest first, paged; values are opened on demand.
    let (status, history, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions?limit=2"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(
        history["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["revisionNo"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [5, 4]
    );
    assert_eq!(history["nextBefore"], 4);
    assert_eq!(
        history["items"][1]["changedFields"],
        json!(["title", "description"])
    );
    assert!(!history.to_string().contains("snapshot"));
    let (status, older, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions?limit=10&before=4"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(older["items"].as_array().unwrap().len(), 3);
    assert!(older["nextBefore"].is_null());

    let (status, detail, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions/1"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["currentRevisionNo"], 5);
    assert!(detail["changesFromPrevious"].as_array().unwrap().is_empty());
    let to_current: Vec<_> = detail["changesToCurrent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap())
        .collect();
    for field in [
        "title",
        "description",
        "status",
        "priority",
        "assignee",
        "labels",
    ] {
        assert!(
            to_current.contains(&field),
            "{field} should differ from the current ticket: {detail}"
        );
    }
    let title_change = detail["changesToCurrent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "title")
        .unwrap();
    assert_eq!(title_change["from"], "로그인 실패 조사");
    assert_eq!(title_change["to"], "로그인 실패 원인 조사");
    let restorable: Vec<_> = detail["restorable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap())
        .collect();
    assert!(
        restorable.contains(&"title") && !restorable.contains(&"status"),
        "status is not restorable here: {detail}"
    );
    let (status, missing, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions/99"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");

    // Restore: selected fields only, as a new revision; old revisions stay.
    let (status, bad_field, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&world.owner_cookie),
        Some("a-restore-bad"),
        Some(json!({"expectedRevision":5,"sourceRevision":1,"fields":["status"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_field}");
    let (status, stale, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&world.owner_cookie),
        Some("a-restore-stale"),
        Some(json!({"expectedRevision":4,"sourceRevision":1,"fields":["title"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["currentRevision"], 5);
    let (status, restored, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&world.owner_cookie),
        Some("a-restore-1"),
        Some(json!({"expectedRevision":5,"sourceRevision":1,"fields":["title","priority"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["revisionNo"], 6);
    assert_eq!(restored["title"], "로그인 실패 조사");
    assert!(
        restored["priority"].is_null(),
        "restoring an unset priority clears it"
    );
    assert_eq!(
        restored["assigneeId"],
        world.owner_id.to_string(),
        "unselected fields keep their current value"
    );
    assert_eq!(restored["statusId"], doing);
    let (status, replay, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&world.owner_cookie),
        Some("a-restore-1"),
        Some(json!({"expectedRevision":5,"sourceRevision":1,"fields":["title","priority"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(
        replay["revisionNo"], 6,
        "a retried restore does not create another revision"
    );
    let (status, noop, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&world.owner_cookie),
        Some("a-restore-noop"),
        Some(json!({"expectedRevision":6,"sourceRevision":1,"fields":["title"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{noop}");
    assert_eq!(
        noop["revisionNo"], 6,
        "restoring identical values makes no revision"
    );
    let (_, history, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions?limit=1"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(history["items"][0]["revisionNo"], 6);
    assert_eq!(history["items"][0]["restoreSourceRevisionNo"], 1);
    assert_eq!(
        history["items"][0]["changedFields"],
        json!(["title", "priority"])
    );
    let (_, activity, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/activity?limit=1"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(activity["items"][0]["activityKind"], "ticket.restored");
    assert_eq!(activity["items"][0]["data"]["sourceRevision"], 1);

    // A value that is no longer valid is reported, never applied or partially saved.
    let current = |label: &'static str| {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let path = format!("/v1/tickets/{id}");
        async move {
            let (status, value, _) = call(&app, "GET", &path, Some(&cookie), None, None).await;
            assert_eq!(status, StatusCode::OK, "{label}: {value}");
            value["revisionNo"].as_i64().unwrap()
        }
    };
    let rev = current("before labels").await;
    let (status, cleared, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{id}"),
        Some(&world.owner_cookie),
        Some("a-clear-labels"),
        Some(json!({"expectedRevision":rev,"labelIds":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    sqlx::query("UPDATE penfii.labels SET retired_at=now() WHERE project_id=$1")
        .bind(Uuid::parse_str(&project_id).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let (status, after_retire, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions/3"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after_retire}");
    let labels_entry = after_retire["restorable"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["field"] == "labels")
        .expect("labels differ from the current ticket");
    assert_eq!(labels_entry["applicable"], false, "{after_retire}");
    assert_eq!(labels_entry["reason"], "label_unavailable");
    let rev = current("after retire").await;
    let (status, rejected, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&world.owner_cookie),
        Some("a-restore-retired-labels"),
        Some(json!({"expectedRevision":rev,"sourceRevision":3,"fields":["labels"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
    assert_eq!(rejected["code"], "restore_not_applicable");
    assert_eq!(rejected["fields"][0]["field"], "labels");
    assert_eq!(
        current("after reject").await,
        rev,
        "a rejected restore saves nothing"
    );

    // Viewers can read History but not restore.
    let viewer = viewer_cookie(world, workspace_id, "history-viewer@penfii.test").await;
    grant_project(world, &project_id, "history-viewer@penfii.test", "VIEWER").await;
    let (status, viewer_history, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{id}/revisions"),
        Some(&viewer),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_history}");
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/restore"),
        Some(&viewer),
        Some("a-restore-viewer"),
        Some(json!({"expectedRevision":rev,"sourceRevision":1,"fields":["title"]})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
}

async fn project_with_prefix(
    world: &World,
    workspace_id: Uuid,
    name: &str,
    prefix: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({"name":name,"teamIds":[]});
    if let Some(prefix) = prefix {
        body["ticketPrefix"] = json!(prefix);
    }
    let (status, value, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(&world.owner_cookie),
        None,
        Some(body),
    )
    .await;
    (status, value)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn ticket_keys_use_the_project_prefix_and_a_forward_only_sequence(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM penfii.workspace_members WHERE user_id=$1 AND state='ACTIVE' LIMIT 1")
        .bind(world.owner_id).fetch_one(&world.pool).await.unwrap();

    // Prefix: suggested from the name, canonical upper case, unique ignoring case.
    let (status, auto) = project_with_prefix(world, workspace_id, "Keyed Alpha", None).await;
    assert_eq!(status, StatusCode::CREATED, "{auto}");
    assert_eq!(auto["ticketPrefix"], "KAL");
    let (status, pfi) = project_with_prefix(world, workspace_id, "Penfii Keys", Some("pfi")).await;
    assert_eq!(status, StatusCode::CREATED, "{pfi}");
    assert_eq!(pfi["ticketPrefix"], "PFI");
    for clashing in ["PFI", "pfi", "Pfi", "pFi"] {
        let (status, value) =
            project_with_prefix(world, workspace_id, "Clash", Some(clashing)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{clashing}: {value}");
        assert_eq!(value["code"], "ticket_prefix_taken");
    }
    for invalid in ["", "P", "1AB", "AB-C", "TOOLONGPREFIX", "가나다"] {
        let (status, value) =
            project_with_prefix(world, workspace_id, "Invalid", Some(invalid)).await;
        if invalid.is_empty() {
            // An empty choice falls back to the suggestion for the name.
            assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}: {value}");
        }
    }
    let (status, korean) = project_with_prefix(world, workspace_id, "페니파이", None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a name with no usable letters needs a typed prefix: {korean}"
    );
    let (status, korean) = project_with_prefix(world, workspace_id, "페니파이", Some("PNF")).await;
    assert_eq!(status, StatusCode::CREATED, "{korean}");
    let (status, check, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/project-prefix?name=Penfii%20Keys&prefix=pfi"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{check}");
    assert_eq!(
        check["prefix"],
        json!({"value":"PFI","valid":true,"available":false})
    );
    assert_eq!(check["suggestion"], "PKE");
    let (_, check, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/project-prefix?prefix=xx9"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(
        check["prefix"],
        json!({"value":"XX9","valid":true,"available":true})
    );

    // Race: the database index lets exactly one of several same-prefix creates win.
    let mut racers = Vec::new();
    for n in 0..6 {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let chosen = ["RCE", "rce", "Rce"][n % 3];
        racers.push(tokio::spawn(async move {
            call(
                &app,
                "POST",
                &format!("/v1/workspaces/{workspace_id}/projects"),
                Some(&cookie),
                None,
                Some(json!({"name":format!("Racer {n}"),"teamIds":[],"ticketPrefix":chosen})),
            )
            .await
            .0
        }));
    }
    let mut wins = 0;
    for racer in racers {
        match racer.await.unwrap() {
            StatusCode::CREATED => wins += 1,
            StatusCode::CONFLICT => {}
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(wins, 1, "only one project may take the prefix");

    // Sequence: starts at 1, pads to three digits, grows past 999, per project.
    let pfi_id = pfi["id"].as_str().unwrap();
    let (status, first) = make_ticket(world, pfi_id, "k-1", json!({"title":"first"})).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["key"], "PFI-001");
    assert!(first.get("publicKey").is_none());
    assert_ne!(
        first["id"], first["key"],
        "the internal id is not the display key"
    );
    let (_, second) = make_ticket(world, pfi_id, "k-2", json!({"title":"second"})).await;
    assert_eq!(second["key"], "PFI-002");
    let (status, replay) = make_ticket(world, pfi_id, "k-2", json!({"title":"second"})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        replay["key"], "PFI-002",
        "a retried create does not take another number"
    );
    let other_id = auto["id"].as_str().unwrap();
    let (_, other_first) =
        make_ticket(world, other_id, "k-other", json!({"title":"other project"})).await;
    assert_eq!(
        other_first["key"], "KAL-001",
        "projects count independently"
    );
    sqlx::query("UPDATE penfii.projects SET ticket_seq=997 WHERE id=$1")
        .bind(Uuid::parse_str(pfi_id).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let (_, t998) = make_ticket(world, pfi_id, "k-998", json!({"title":"998"})).await;
    let (_, t999) = make_ticket(world, pfi_id, "k-999", json!({"title":"999"})).await;
    let (_, t1000) = make_ticket(world, pfi_id, "k-1000", json!({"title":"1000"})).await;
    let (_, t1001) = make_ticket(world, pfi_id, "k-1001", json!({"title":"1001"})).await;
    assert_eq!(
        [
            t998["key"].clone(),
            t999["key"].clone(),
            t1000["key"].clone(),
            t1001["key"].clone()
        ],
        [
            json!("PFI-998"),
            json!("PFI-999"),
            json!("PFI-1000"),
            json!("PFI-1001")
        ]
    );

    // A trashed ticket keeps its number; the next one does not reuse it.
    sqlx::query("UPDATE penfii.tickets SET trashed_at=now() WHERE id=$1")
        .bind(Uuid::parse_str(t1001["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let (_, t1002) = make_ticket(world, pfi_id, "k-1002", json!({"title":"1002"})).await;
    assert_eq!(t1002["key"], "PFI-1002");

    // Concurrent creates in one project get distinct, gap-free numbers.
    let (project_id, _) = new_project(world, workspace_id, "Concurrent Keys").await;
    let mut creators = Vec::new();
    for n in 0..12 {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let project_id = project_id.clone();
        creators.push(tokio::spawn(async move {
            let (status, value, _) = call(
                &app,
                "POST",
                &format!("/v1/projects/{project_id}/tickets"),
                Some(&cookie),
                Some(&format!("concurrent-{n}")),
                Some(json!({"title":format!("c{n}")})),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{value}");
            value["key"].as_str().unwrap().to_owned()
        }));
    }
    let mut keys = Vec::new();
    for creator in creators {
        keys.push(creator.await.unwrap());
    }
    keys.sort();
    let prefix: String =
        sqlx::query_scalar("SELECT ticket_prefix FROM penfii.projects WHERE id=$1")
            .bind(Uuid::parse_str(&project_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    let mut expected: Vec<String> = (1..=12).map(|n| format!("{prefix}-{n:03}")).collect();
    expected.sort();
    assert_eq!(keys, expected);

    // The key is shown everywhere a ticket is read, while ids still address it.
    let (status, fetched, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}", first["id"].as_str().unwrap()),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["key"], "PFI-001");
    let (_, epic) = make_ticket(
        world,
        pfi_id,
        "k-epic",
        json!({"title":"epic","hierarchyKind":"EPIC"}),
    )
    .await;
    let (_, child) = make_ticket(
        world,
        pfi_id,
        "k-child",
        json!({"title":"child","parentId":epic["id"]}),
    )
    .await;
    assert_eq!(child["parent"]["key"], epic["key"]);
    let (status, by_key, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}", first["key"].as_str().unwrap()),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a key is not a resource id: {by_key}"
    );
}

pub(super) async fn board_json(world: &World, project_id: &str) -> Value {
    let (status, board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    board
}

/// Keys of the Tickets in one column, in shown order.
pub(super) fn column_keys(board: &Value, state_id: &str) -> Vec<String> {
    let column = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["stateId"] == state_id)
        .unwrap();
    column["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["key"].as_str().unwrap().to_owned())
        .collect()
}

pub(super) fn pick(keys: &[String], at: &[usize]) -> Vec<String> {
    at.iter().map(|i| keys[*i].clone()).collect()
}

pub(super) fn state_id(workflow: &Value, category: &str) -> String {
    workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == category)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wip_limit_is_a_warning_the_board_reports(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Wip").await;
    let doing = state_id(&workflow, "IN_PROGRESS");
    let version = workflow["version"].as_i64().unwrap();
    let (status, set, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{doing}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedWorkflowVersion":version,"wipLimit":1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(set["state"]["wipLimit"], 1);
    let mut ids = Vec::new();
    for n in 0..2 {
        let (status, ticket) = make_ticket(
            world,
            &project_id,
            &format!("w-{n}"),
            json!({"title":format!("W{n}")}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{ticket}");
        ids.push(ticket["id"].as_str().unwrap().to_owned());
    }
    // The limit warns; it never refuses a move.
    for (n, id) in ids.iter().enumerate() {
        let (status, moved, _) = call(
            &world.app,
            "POST",
            &format!("/v1/tickets/{id}/commands/move"),
            Some(&world.owner_cookie),
            Some(&format!("w-m{n}")),
            Some(json!({"expectedRevision":1,"statusId":doing,"position":"last"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{moved}");
    }
    let board = board_json(world, &project_id).await;
    let column = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["stateId"] == doing)
        .unwrap();
    assert_eq!(column["wipLimit"], 1);
    assert_eq!(column["ticketCount"], 2);
    // Bad limits and a non-admin are refused; null clears.
    let (status, bad, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{doing}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedWorkflowVersion":set["version"],"wipLimit":0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let member = viewer_cookie(world, workspace_id, "wip-member@penfii.test").await;
    grant_project(world, &project_id, "wip-member@penfii.test", "VIEWER").await;
    let (status, denied, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{doing}"),
        Some(&member),
        None,
        Some(json!({"expectedWorkflowVersion":set["version"],"wipLimit":5})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, cleared, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{doing}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedWorkflowVersion":set["version"],"wipLimit":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["state"]["wipLimit"], Value::Null);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn workflow_states_are_managed_by_project_admins(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "State admin").await;
    let editor = viewer_cookie(world, workspace_id, "state-editor@penfii.test").await;
    let user: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email='state-editor@penfii.test'")
            .fetch_one(&world.pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO penfii.project_user_grants(project_id,user_id,role) VALUES($1,$2,'EDITOR')",
    )
    .bind(Uuid::parse_str(&project_id).unwrap())
    .bind(user)
    .execute(&world.pool)
    .await
    .unwrap();
    let states_path = format!("/v1/projects/{project_id}/workflow/states");
    let version = workflow["version"].as_i64().unwrap();

    // Everyone who can see the Project reads the list; only an admin changes it.
    let (s, list, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        Some(&editor),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let body = json!({"expectedWorkflowVersion":version,"name":"Review","category":"IN_PROGRESS","position":50});
    let (s, denied, _) = call(
        &world.app,
        "POST",
        &states_path,
        Some(&editor),
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    let (s, created, _) = call(
        &world.app,
        "POST",
        &states_path,
        Some(&world.owner_cookie),
        None,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let review = created["state"]["id"].as_str().unwrap().to_owned();
    let version = created["version"].as_i64().unwrap();
    let one = format!("{states_path}/{review}");
    let (s, denied, _) = call(
        &world.app,
        "PATCH",
        &one,
        Some(&editor),
        None,
        Some(json!({"expectedWorkflowVersion":version,"name":"Check"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    let (s, renamed, _) = call(
        &world.app,
        "PATCH",
        &one,
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedWorkflowVersion":version,"name":"QA","position":-1})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{renamed}");
    let version = renamed["version"].as_i64().unwrap();
    let (s, stale, _) = call(
        &world.app,
        "PATCH",
        &one,
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedWorkflowVersion":version - 1,"name":"Stale"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "workflow_version_conflict");
    // The board and the workflow list follow the Workflow order.
    let (_, list, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        Some(&editor),
        None,
        None,
    )
    .await;
    assert_eq!(list["states"][0]["name"], "QA");
    let columns = board_json(world, &project_id).await["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["stateId"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(columns[0], review);

    // Deleting: never the initial State, the last NOT_STARTED/DONE, or one that Tickets use.
    let delete = |id: String, version: i64, cookie: String| {
        let app = world.app.clone();
        let path = format!("{states_path}/{id}?expectedWorkflowVersion={version}");
        async move { call(&app, "DELETE", &path, Some(&cookie), None, None).await }
    };
    let initial = state_id(&list, "NOT_STARTED");
    let (s, refused, _) = delete(initial, version, world.owner_cookie.clone()).await;
    assert_eq!(
        (s, refused["code"].as_str()),
        (StatusCode::CONFLICT, Some("workflow_requires_initial")),
        "{refused}"
    );
    let done = state_id(&list, "DONE");
    let (s, refused, _) = delete(done, version, world.owner_cookie.clone()).await;
    assert_eq!(
        (s, refused["code"].as_str()),
        (StatusCode::CONFLICT, Some("workflow_requires_done")),
        "{refused}"
    );
    let (s, ticket) = make_ticket(
        world,
        &project_id,
        "st-1",
        json!({"title":"Uses QA","hierarchyKind":"TASK"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{ticket}");
    let (s, moved, _) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/tickets/{}/commands/change-status",
            ticket["id"].as_str().unwrap()
        ),
        Some(&world.owner_cookie),
        Some("st-mv"),
        Some(json!({"expectedRevision":ticket["revisionNo"],"statusId":review})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    let (s, refused, _) = delete(review.clone(), version, world.owner_cookie.clone()).await;
    assert_eq!(
        (s, refused["code"].as_str()),
        (StatusCode::CONFLICT, Some("state_in_use")),
        "{refused}"
    );
    assert_eq!(refused["tickets"], 1);
    let (s, denied, _) = delete(review.clone(), version, editor.clone()).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    // Move the Ticket back; then the State can go and its column disappears.
    let (s, back, _) = call(&world.app, "POST", &format!("/v1/tickets/{}/commands/change-status", ticket["id"].as_str().unwrap()), Some(&world.owner_cookie), Some("st-back"), Some(json!({"expectedRevision":moved["revisionNo"],"statusId":state_id(&list, "NOT_STARTED")}))).await;
    assert_eq!(s, StatusCode::OK, "{back}");
    let (s, gone, _) = delete(review.clone(), version, world.owner_cookie.clone()).await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    let (_, list, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        Some(&editor),
        None,
        None,
    )
    .await;
    assert!(
        list["states"]
            .as_array()
            .unwrap()
            .iter()
            .all(|st| st["id"] != review.as_str())
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scope_dataset_gathers_the_same_tickets_across_projects_and_keeps_permissions(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let owner = world.owner_cookie.clone();
    let (team_a, team_b) = {
        let (_, a, _) = call(
            &world.app,
            "POST",
            &format!("/v1/workspaces/{workspace_id}/teams"),
            Some(&owner),
            None,
            Some(json!({"name":"Scope A","members":[{"userId":world.owner_id,"role":"MANAGER"}]})),
        )
        .await;
        let (_, b, _) = call(
            &world.app,
            "POST",
            &format!("/v1/workspaces/{workspace_id}/teams"),
            Some(&owner),
            None,
            Some(json!({"name":"Scope B","members":[{"userId":world.owner_id,"role":"MANAGER"}]})),
        )
        .await;
        (
            a["id"].as_str().unwrap().to_owned(),
            b["id"].as_str().unwrap().to_owned(),
        )
    };
    let create = |name: &str, teams: Vec<String>| {
        let app = world.app.clone();
        let cookie = owner.clone();
        let name = name.to_owned();
        async move {
            let (status, project, _) = call(
                &app,
                "POST",
                &format!("/v1/workspaces/{workspace_id}/projects"),
                Some(&cookie),
                None,
                Some(json!({"name":name,"teamIds":teams})),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{project}");
            project["id"].as_str().unwrap().to_owned()
        }
    };
    let p1 = create("Scope P1", vec![team_a.clone()]).await;
    let p2 = create("Scope P2", vec![team_a.clone()]).await;
    let p3 = create("Scope P3", vec![team_b.clone()]).await;
    let (_, t1) = make_ticket(world, &p1, "p6sc-1", json!({"title":"Alpha"})).await;
    let (_, t2) = make_ticket(world, &p2, "p6sc-2", json!({"title":"Beta"})).await;
    let (_, t3) = make_ticket(world, &p3, "p6sc-3", json!({"title":"Gamma"})).await;
    assert!(t1["id"].is_string() && t2["id"].is_string() && t3["id"].is_string());

    // Team scope: only that Team's Projects; each Ticket carries its Project and its own State.
    let (status, ds) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/work?scope=TEAM&teamId={team_a}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ds}");
    let titles: Vec<&str> = ds["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles.len(), 2, "{titles:?}");
    assert!(titles.contains(&"Alpha") && titles.contains(&"Beta") && !titles.contains(&"Gamma"));
    let alpha = ds["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "Alpha")
        .unwrap();
    assert_eq!(alpha["projectName"], "Scope P1");
    assert_eq!(alpha["stateCategory"], "NOT_STARTED");
    assert!(alpha["stateName"].is_string());
    assert_eq!(ds["truncated"], false);
    // Workspace scope with a lens: nothing is assigned yet, then it is.
    let (_, mine) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=ASSIGNED"),
    )
    .await;
    let before = mine["tickets"].as_array().unwrap().len();
    let (status, assigned, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{}", t1["id"].as_str().unwrap()),
        Some(&owner),
        Some("p6sc-assign"),
        Some(json!({"expectedRevision":1,"assigneeId":world.owner_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let (_, mine) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=ASSIGNED"),
    )
    .await;
    assert_eq!(mine["tickets"].as_array().unwrap().len(), before + 1);
    let (_, created) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=CREATED"),
    )
    .await;
    assert!(created["tickets"].as_array().unwrap().len() >= 3);
    let (status, bad) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=WATCHED"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");

    // Everyone in the workspace reads every Project, but a private-channel Ticket stays hidden.
    let (status, private_channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"scope secret","visibility":"PRIVATE","projectId":p1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private_channel}");
    let (status, secret) = make_ticket(
        world,
        &p1,
        "p6sc-secret",
        json!({"title":"Hidden one","channelId":private_channel["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{secret}");
    let reader = viewer_cookie(world, workspace_id, "p6sc-reader@penfii.test").await;
    sqlx::query("INSERT INTO penfii.project_user_grants(project_id,user_id,role) SELECT id,$2,'VIEWER' FROM penfii.projects WHERE workspace_id=$1")
        .bind(workspace_id).bind(user_id_of(world, "p6sc-reader@penfii.test").await).execute(&world.pool).await.unwrap();
    for path in [
        format!("/v1/workspaces/{workspace_id}/work?scope=TEAM&teamId={team_a}"),
        format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE"),
    ] {
        let (status, ds) = get_as(world, &reader, &path).await;
        assert_eq!(status, StatusCode::OK, "{ds}");
        assert!(ds.to_string().contains("Alpha"));
        assert!(
            !ds.to_string().contains("Hidden one"),
            "{path}: a private-channel Ticket must not appear"
        );
        assert!(!ds.to_string().contains("scope secret"));
    }
    let (_, owner_ds) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/work?scope=TEAM&teamId={team_a}"),
    )
    .await;
    assert!(owner_ds.to_string().contains("Hidden one"));
    // A person who is not a member of a workspace gets nothing from it.
    let other_workspace = workspace(world, "Scope elsewhere").await;
    let (status, _) = get_as(
        world,
        &reader,
        &format!("/v1/workspaces/{other_workspace}/work?scope=WORKSPACE"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A Team the workspace does not have is not found.
    let (status, _) = get_as(
        world,
        &owner,
        &format!(
            "/v1/workspaces/{workspace_id}/work?scope=TEAM&teamId={}",
            Uuid::new_v4()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn bulk_changes_report_every_ticket_and_never_bypass_the_single_ticket_rules(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Bulk").await;
    let (other_project, other_workflow) = new_project(world, workspace_id, "Bulk other").await;
    let done = state_id(&workflow, "DONE");
    let doing = state_id(&workflow, "IN_PROGRESS");
    let owner = world.owner_cookie.clone();
    let bulk = format!("/v1/workspaces/{workspace_id}/tickets/bulk");
    let mut ids = Vec::new();
    for n in 0..4 {
        let (s, t) = make_ticket(
            world,
            &project_id,
            &format!("p6bk-{n}"),
            json!({"title":format!("Bulk {n}")}),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    let (_, foreign) = make_ticket(
        world,
        &other_project,
        "p6bk-f",
        json!({"title":"Bulk foreign"}),
    )
    .await;
    let foreign_id = foreign["id"].as_str().unwrap().to_owned();
    // Bulk 3 waits for Bulk 2.
    let (s, d) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[3]),
        "p6bk-dep",
        json!({"predecessorId":ids[2]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    // A private-channel Ticket the second admin cannot see.
    let (_, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"bulk secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    let (_, secret) = make_ticket(
        world,
        &project_id,
        "p6bk-secret",
        json!({"title":"Bulk secret","channelId":channel["id"]}),
    )
    .await;
    let secret_id = secret["id"].as_str().unwrap().to_owned();
    let editor = viewer_cookie(world, workspace_id, "bulk-editor@penfii.test").await;
    let editor_id = user_id_of(world, "bulk-editor@penfii.test").await;
    grant(world, &project_id, editor_id, "EDITOR").await;
    let viewer = viewer_cookie(world, workspace_id, "bulk-viewer@penfii.test").await;
    grant_project(world, &project_id, "bulk-viewer@penfii.test", "VIEWER").await;
    grant_project(world, &other_project, "bulk-viewer@penfii.test", "VIEWER").await;

    // State to DONE for: the blocked one, free ones, an unseen one, another Project's Ticket.
    let (s, result) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-1",
        json!({
        "ticketIds":[ids[3], ids[0], ids[2], secret_id, foreign_id],
        "change":{"type":"STATUS","statusId":done}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{result}");
    assert_eq!(result["total"], 5);
    assert_eq!(result["succeeded"], 2);
    assert_eq!(result["failed"], 3);
    let item = |id: &str| {
        result["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["ticketId"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(item(&ids[0])["ok"], true);
    assert_eq!(item(&ids[2])["ok"], true);
    assert_eq!(
        item(&ids[3])["code"],
        "dependency_blocked",
        "the dependency rule applies to every Ticket in the batch"
    );
    assert_eq!(
        item(&secret_id)["code"],
        "not_found",
        "a Ticket the person cannot see looks missing"
    );
    assert!(!item(&secret_id).to_string().contains("Bulk secret"));
    assert_eq!(
        item(&foreign_id)["ok"],
        false,
        "a State of another Project is refused"
    );
    // The failures changed nothing; the successes are real.
    let (_, a) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[0])).await;
    assert_eq!(a["statusId"], done);
    let (_, blocked) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[3])).await;
    assert_ne!(blocked["statusId"], done);
    // The same request again returns the recorded result and does not run again.
    let revision_before = a["revisionNo"].clone();
    let (s, replay) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-1",
        json!({
        "ticketIds":[ids[3], ids[0], ids[2], secret_id, foreign_id],
        "change":{"type":"STATUS","statusId":done}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{replay}");
    assert_eq!(replay, result);
    let (_, a2) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[0])).await;
    assert_eq!(a2["revisionNo"], revision_before);

    // A viewer is refused for every Ticket, with the reason.
    let (s, denied) = post_as(
        world,
        &viewer,
        &bulk,
        "p6bk-2",
        json!({"ticketIds":[ids[1]],"change":{"type":"PRIORITY","priority":"HIGH"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{denied}");
    assert_eq!(denied["failed"], 1);
    assert_eq!(denied["items"][0]["code"], "forbidden");
    // Priority, assignee, labels, Sprint and Milestones.
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-3",
        json!({"ticketIds":[ids[1],ids[2]],"change":{"type":"PRIORITY","priority":"URGENT"}}),
    )
    .await;
    assert_eq!(
        (s, v["succeeded"].as_i64()),
        (StatusCode::OK, Some(2)),
        "{v}"
    );
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-4",
        json!({"ticketIds":[ids[1],ids[2]],"change":{"type":"ASSIGNEE","assigneeId":editor_id}}),
    )
    .await;
    assert_eq!(
        (s, v["succeeded"].as_i64()),
        (StatusCode::OK, Some(2)),
        "{v}"
    );
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-4b",
        json!({"ticketIds":[ids[1]],"change":{"type":"ASSIGNEE","assigneeId":Uuid::new_v4()}}),
    )
    .await;
    assert_eq!(
        (s, v["failed"].as_i64()),
        (StatusCode::OK, Some(1)),
        "an unknown assignee is refused per Ticket: {v}"
    );
    let (s, label) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/labels"),
        "p6bk-label",
        json!({"name":"bulk-label"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{label}");
    let label_id = label["id"].as_str().unwrap().to_owned();
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-5",
        json!({"ticketIds":[ids[1],ids[2]],"change":{"type":"ADD_LABELS","labelIds":[label_id]}}),
    )
    .await;
    assert_eq!(
        (s, v["succeeded"].as_i64()),
        (StatusCode::OK, Some(2)),
        "{v}"
    );
    let (_, one) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[1])).await;
    assert_eq!(one["priority"], "URGENT");
    assert_eq!(one["assigneeId"], editor_id.to_string());
    assert_eq!(one["labels"][0]["name"], "bulk-label");
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-6",
        json!({"ticketIds":[ids[1]],"change":{"type":"REMOVE_LABELS","labelIds":[label_id]}}),
    )
    .await;
    assert_eq!(v["succeeded"], 1, "{s} {v}");
    let (_, one) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[1])).await;
    assert_eq!(one["labels"].as_array().unwrap().len(), 0);
    let (_, sprint, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&owner),
        Some("p6bk-sp"),
        Some(json!({"name":"Bulk sprint","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-7",
        json!({"ticketIds":[ids[1],ids[2]],"change":{"type":"SPRINT","sprintId":sprint["id"]}}),
    )
    .await;
    assert_eq!(
        (s, v["succeeded"].as_i64()),
        (StatusCode::OK, Some(2)),
        "{v}"
    );
    let (_, milestone, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/milestones"),
        Some(&owner),
        Some("p6bk-ms"),
        Some(json!({"name":"Bulk milestone"})),
    )
    .await;
    let (s, v) = post_as(world, &editor, &bulk, "p6bk-8", json!({"ticketIds":[ids[1],ids[2]],"change":{"type":"ADD_MILESTONES","milestoneIds":[milestone["id"]]}})).await;
    assert_eq!(
        (s, v["succeeded"].as_i64()),
        (StatusCode::OK, Some(2)),
        "{v}"
    );
    let (_, one) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[1])).await;
    assert_eq!(one["sprint"]["name"], "Bulk sprint");
    assert_eq!(one["milestones"][0]["name"], "Bulk milestone");
    // Start changes work too, and a State that does not exist for the Project is refused.
    let (s, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-9",
        json!({"ticketIds":[ids[1]],"change":{"type":"STATUS","statusId":doing}}),
    )
    .await;
    assert_eq!(
        (s, v["succeeded"].as_i64()),
        (StatusCode::OK, Some(1)),
        "{v}"
    );
    let foreign_state = state_id(&other_workflow, "DONE");
    let (_, v) = post_as(
        world,
        &editor,
        &bulk,
        "p6bk-10",
        json!({"ticketIds":[ids[1]],"change":{"type":"STATUS","statusId":foreign_state}}),
    )
    .await;
    assert_eq!(v["failed"], 1, "{v}");
    // Shape errors fail the whole request, not individual Tickets.
    for (n, body) in [
        json!({"ticketIds":[],"change":{"type":"PRIORITY","priority":"LOW"}}),
        json!({"ticketIds":[ids[1]],"change":{"type":"NOPE"}}),
        json!({"ticketIds":(0..101).map(|_| Uuid::new_v4().to_string()).collect::<Vec<_>>(),"change":{"type":"PRIORITY","priority":"LOW"}}),
    ].into_iter().enumerate() {
        let (s, v) = post_as(world, &editor, &bulk, &format!("p6bk-bad{n}"), body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    // Someone outside the workspace is told it does not exist.
    let elsewhere = workspace(world, "Bulk elsewhere").await;
    let (s, _) = post_as(
        world,
        &editor,
        &format!("/v1/workspaces/{elsewhere}/tickets/bulk"),
        "p6bk-out",
        json!({"ticketIds":[ids[1]],"change":{"type":"PRIORITY","priority":"LOW"}}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_same_bulk_request_sent_twice_at_once_changes_each_ticket_once(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Bulk race").await;
    let mut ids = Vec::new();
    for n in 0..5 {
        let (_, t) = make_ticket(
            world,
            &project_id,
            &format!("p6br-{n}"),
            json!({"title":format!("Race {n}")}),
        )
        .await;
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    let send = || {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let ids = ids.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/workspaces/{workspace_id}/tickets/bulk"),
                Some(&cookie),
                Some("p6br-same-key"),
                Some(json!({"ticketIds":ids,"change":{"type":"PRIORITY","priority":"URGENT"}})),
            )
            .await
        }
    };
    let (a, b) = tokio::join!(send(), send());
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["succeeded"], 5, "{}", a.1);
    assert_eq!(b.1["succeeded"], 5, "{}", b.1);
    for id in &ids {
        let revision: i64 =
            sqlx::query_scalar("SELECT revision_no FROM penfii.tickets WHERE id=$1")
                .bind(Uuid::parse_str(id).unwrap())
                .fetch_one(&world.pool)
                .await
                .unwrap();
        assert_eq!(revision, 2, "each Ticket was changed exactly once");
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_scope_over_the_cap_keeps_the_most_recently_updated_tickets_and_says_so(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Cap").await;
    let pid = Uuid::parse_str(&project_id).unwrap();
    // 1001 Tickets, inserted directly: the cap is a property of the query, not of the API.
    let (_, first) =
        make_ticket(world, &project_id, "p6cap-0", json!({"title":"Cap oldest"})).await;
    let status: Uuid = sqlx::query_scalar("SELECT status_id FROM penfii.tickets WHERE id=$1")
        .bind(Uuid::parse_str(first["id"].as_str().unwrap()).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE penfii.tickets SET updated_at=now()-interval '10 days' WHERE id=$1")
        .bind(Uuid::parse_str(first["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO penfii.tickets(id,workspace_id,project_id,project_sequence,public_key,title,hierarchy_kind,status_id,created_by,updated_at)
         SELECT gen_random_uuid(),$1,$2,1000+g,'CAP-'||g,'Cap '||g,'TASK',$3,$4,now()-(g||' seconds')::interval FROM generate_series(1,1000) g",
    )
    .bind(workspace_id).bind(pid).bind(status).bind(world.owner_id)
    .execute(&world.pool).await.unwrap();
    let (s, ds) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=CREATED"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let in_cap: Vec<&Value> = ds["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["projectId"] == project_id)
        .collect();
    assert_eq!(ds["truncated"], true, "more than the cap exists");
    assert!(ds["tickets"].as_array().unwrap().len() <= 1000);
    assert!(
        !in_cap.iter().any(|t| t["title"] == "Cap oldest"),
        "the least recently updated Ticket is the one left out"
    );
    assert!(
        in_cap.iter().any(|t| t["title"] == "Cap 1"),
        "recent work is kept"
    );
    // Naming Projects narrows the Tickets before the cap.
    let (s, narrowed) = get_as(world, &world.owner_cookie, &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=CREATED&projects={project_id},{}", Uuid::new_v4())).await;
    assert_eq!(s, StatusCode::OK, "{narrowed}");
    assert!(
        narrowed["tickets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["projectId"] == project_id),
        "only the named Project's Tickets"
    );
    assert!(
        !narrowed["projects"].as_array().unwrap().is_empty(),
        "the Project list stays whole so the scope can be widened again"
    );
    let (s, bad) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&projects=nope"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
}
