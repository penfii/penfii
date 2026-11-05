//! Who may see and change what: roles, grants, share scope and the visibility matrix.

use super::docs::{api, make_doc, node_ids, page_content, save_doc};
use super::inbox::member;
use super::search::{find, index_now, search_world, titles};
use super::tickets::state_id;
use super::{
    World, call, channel, get_as, grant, make_ticket, new_project, post_as, send, user_id_of,
    viewer_cookie, workspace,
};
use crate::common::doc;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn roles_are_enforced_by_the_api_not_only_by_hidden_buttons(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Roles").await;
    let done = state_id(&workflow, "DONE");
    let admin = world.owner_cookie.clone();
    let editor = viewer_cookie(world, workspace_id, "p6role-editor@penfii.test").await;
    grant(
        world,
        &project_id,
        user_id_of(world, "p6role-editor@penfii.test").await,
        "EDITOR",
    )
    .await;
    let reader = viewer_cookie(world, workspace_id, "p6role-reader@penfii.test").await;
    grant(
        world,
        &project_id,
        user_id_of(world, "p6role-reader@penfii.test").await,
        "VIEWER",
    )
    .await;
    let (_, a) = make_ticket(world, &project_id, "p6role-a", json!({"title":"Role A"})).await;
    let (_, b) = make_ticket(world, &project_id, "p6role-b", json!({"title":"Role B"})).await;
    let (a_id, b_id) = (
        a["id"].as_str().unwrap().to_owned(),
        b["id"].as_str().unwrap().to_owned(),
    );
    let (s, dep) = post_as(
        world,
        &admin,
        &format!("/v1/tickets/{b_id}/dependencies"),
        "p6role-dep",
        json!({"predecessorId":a_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dep}");
    let edge = dep["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    let (_, sprint, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&admin),
        Some("p6role-sp"),
        Some(json!({"name":"R","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let base = format!("/v1/workspaces/{workspace_id}");

    // (path, body) the admin may call and the others may not.
    let denied_for = |who: &'static str, cookie: String, expected: StatusCode| {
        let app = world.app.clone();
        let (project_id, edge, sprint_id, a_id) = (
            project_id.clone(),
            edge.clone(),
            sprint_id.clone(),
            a_id.clone(),
        );
        async move {
            let mut seen = Vec::new();
            for (method, path, body) in [
                (
                    "POST",
                    format!("/v1/dependencies/{edge}/commands/waive"),
                    json!({"reason":"because"}),
                ),
                (
                    "POST",
                    format!("/v1/sprints/{sprint_id}/commands/start"),
                    json!({"expectedVersion":1}),
                ),
                (
                    "GET",
                    format!("/v1/sprints/{sprint_id}/close-preview"),
                    Value::Null,
                ),
                (
                    "POST",
                    format!("/v1/projects/{project_id}/pins"),
                    json!({"ticketId":a_id}),
                ),
                (
                    "POST",
                    format!("/v1/projects/{project_id}/views"),
                    json!({"name":"Shared","scope":"PROJECT"}),
                ),
            ] {
                let body = if body.is_null() { None } else { Some(body) };
                let (status, value, _) = call(
                    &app,
                    method,
                    &path,
                    Some(&cookie),
                    Some(&format!("p6role-{who}-{path}")),
                    body,
                )
                .await;
                seen.push((path, status, value));
            }
            for (path, status, value) in seen {
                assert_eq!(status, expected, "{who} on {path}: {value}");
            }
        }
    };
    denied_for("editor", editor.clone(), StatusCode::FORBIDDEN).await;
    denied_for("reader", reader.clone(), StatusCode::FORBIDDEN).await;

    // The reader can read and keep personal things, but cannot change work.
    let (s, _) = get_as(world, &reader, &format!("/v1/projects/{project_id}/board")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = post_as(
        world,
        &reader,
        &format!("/v1/tickets/{a_id}/commands/change-status"),
        "p6role-r1",
        json!({"expectedRevision":1,"statusId":done}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = post_as(
        world,
        &reader,
        &format!("/v1/tickets/{b_id}/dependencies"),
        "p6role-r2",
        json!({"predecessorId":a_id,"kind":"RELATED"}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a read-only member cannot relate Tickets either"
    );
    let (s, bulk) = post_as(
        world,
        &reader,
        &format!("{base}/tickets/bulk"),
        "p6role-r3",
        json!({"ticketIds":[a_id,b_id],"change":{"type":"PRIORITY","priority":"HIGH"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{bulk}");
    assert_eq!(
        bulk["failed"], 2,
        "bulk refuses every Ticket for a reader: {bulk}"
    );
    assert!(
        bulk["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["code"] == "forbidden")
    );
    let (_, still) = get_as(world, &admin, &format!("/v1/tickets/{a_id}")).await;
    assert!(still["priority"].is_null());
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!("{base}/favorites"),
        Some(&reader),
        None,
        Some(json!({"kind":"TICKET","targetId":a_id})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "personal shortcuts are open to a reader");
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{edge}"),
        Some(&reader),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // An editor changes work through bulk but cannot touch planning or the blocked completion.
    let (s, ok) = post_as(
        world,
        &editor,
        &format!("{base}/tickets/bulk"),
        "p6role-e1",
        json!({"ticketIds":[a_id],"change":{"type":"PRIORITY","priority":"LOW"}}),
    )
    .await;
    assert_eq!(
        (s, ok["succeeded"].as_i64()),
        (StatusCode::OK, Some(1)),
        "{ok}"
    );
    let (s, blocked) = post_as(
        world,
        &editor,
        &format!("{base}/tickets/bulk"),
        "p6role-e2",
        json!({"ticketIds":[b_id],"change":{"type":"STATUS","statusId":done}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(blocked["items"][0]["code"], "dependency_blocked");
    // The admin may do all of it.
    let (s, v) = post_as(
        world,
        &admin,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6role-admin-w",
        json!({"reason":"admin decides"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = post_as(
        world,
        &admin,
        &format!("/v1/projects/{project_id}/pins"),
        "p6role-admin-p",
        json!({"ticketId":a_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = get_as(world, &admin, &format!("/v1/sprints/{sprint_id}/results")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_designated_editor_can_waive_and_pin_until_the_designation_is_taken_back(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Designation").await;
    let admin = world.owner_cookie.clone();
    let editor = viewer_cookie(world, workspace_id, "p6des-editor@penfii.test").await;
    let editor_id = user_id_of(world, "p6des-editor@penfii.test").await;
    grant(world, &project_id, editor_id, "EDITOR").await;
    let _reader = viewer_cookie(world, workspace_id, "p6des-reader@penfii.test").await;
    let reader_id = user_id_of(world, "p6des-reader@penfii.test").await;
    grant(world, &project_id, reader_id, "VIEWER").await;
    let (_, a) = make_ticket(world, &project_id, "p6des-a", json!({"title":"Des A"})).await;
    let (_, b) = make_ticket(world, &project_id, "p6des-b", json!({"title":"Des B"})).await;
    let (a_id, b_id) = (
        a["id"].as_str().unwrap().to_owned(),
        b["id"].as_str().unwrap().to_owned(),
    );
    let (_, dep) = post_as(
        world,
        &admin,
        &format!("/v1/tickets/{b_id}/dependencies"),
        "p6des-dep",
        json!({"predecessorId":a_id}),
    )
    .await;
    let edge = dep["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    let put = |cookie: String, user: Uuid, waive: bool, pins: bool| {
        let app = world.app.clone();
        let path = format!("/v1/projects/{project_id}/members/{user}/designation");
        async move {
            call(
                &app,
                "PUT",
                &path,
                Some(&cookie),
                None,
                Some(json!({"canWaiveDependencies":waive,"canManagePins":pins})),
            )
            .await
        }
    };
    // Not designated yet: refused, and the Ticket view says so.
    let (s, v) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6des-w0",
        json!({"reason":"x"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (_, deps) = get_as(world, &editor, &format!("/v1/tickets/{b_id}/dependencies")).await;
    assert_eq!(deps["canWaive"], false);
    // Only an admin designates, and only a direct editor can be designated.
    let (s, _, _) = put(editor.clone(), editor_id, true, true).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v, _) = put(admin.clone(), reader_id, true, false).await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "a viewer cannot be designated: {v}"
    );
    let (s, v, _) = put(admin.clone(), editor_id, true, true).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Designated: may waive with a reason, restore, remove a blocking relation, and pin.
    let (_, deps) = get_as(world, &editor, &format!("/v1/tickets/{b_id}/dependencies")).await;
    assert_eq!(deps["canWaive"], true);
    let (s, v) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6des-w1",
        json!({"reason":"delegated"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge}/commands/restore"),
        "p6des-r1",
        json!({"reason":"back"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = post_as(
        world,
        &editor,
        &format!("/v1/projects/{project_id}/pins"),
        "p6des-p1",
        json!({"ticketId":a_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{edge}"),
        Some(&editor),
        None,
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::NO_CONTENT,
        "a designated editor may remove a blocking relation"
    );
    // Taken back: refused again.
    let (_, dep2) = post_as(
        world,
        &admin,
        &format!("/v1/tickets/{b_id}/dependencies"),
        "p6des-dep2",
        json!({"predecessorId":a_id}),
    )
    .await;
    let edge2 = dep2["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    let (s, _, _) = put(admin.clone(), editor_id, false, false).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge2}/commands/waive"),
        "p6des-w2",
        json!({"reason":"again"}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "after the designation is taken back"
    );
    let (s, _) = post_as(
        world,
        &editor,
        &format!("/v1/projects/{project_id}/pins"),
        "p6des-p2",
        json!({"url":"https://example.test/x"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A designation never reaches a Ticket the person cannot open (a PRIVATE channel they are not in).
    let (_, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&admin),
        None,
        Some(json!({"name":"des secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    let (_, secret) = make_ticket(
        world,
        &project_id,
        "p6des-s",
        json!({"title":"Secret succ","channelId":channel["id"]}),
    )
    .await;
    let (_, other) = make_ticket(world, &project_id, "p6des-o", json!({"title":"Open pred"})).await;
    let (s, dep3) = post_as(
        world,
        &admin,
        &format!(
            "/v1/tickets/{}/dependencies",
            secret["id"].as_str().unwrap()
        ),
        "p6des-dep3",
        json!({"predecessorId":other["id"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dep3}");
    let edge3 = dep3["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    put(admin.clone(), editor_id, true, true).await;
    let (s, v) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge3}/commands/waive"),
        "p6des-w3",
        json!({"reason":"peek"}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "a designated editor outside the channel gets nothing: {v}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_owner_renames_the_workspace_and_a_team(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, "rename-me").await;
    let (status, renamed, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/workspaces/{ws}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name": "Renamed", "timeZone": "Asia/Seoul"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Renamed");
    assert_eq!(renamed["timeZone"], "Asia/Seoul");
    let (_, member_cookie) = member(world, ws, "renamer").await;
    let (status, denied, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/workspaces/{ws}"),
        Some(&member_cookie),
        None,
        Some(json!({"name": "Nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, bad, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/workspaces/{ws}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"timeZone": "Not/AZone"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let (status, created, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/teams"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name": "Old", "members": [{"userId": world.owner_id, "role": "MANAGER"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let team_id = created["id"].as_str().unwrap();
    let (status, team, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/workspaces/{ws}/teams/{team_id}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name": "New"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{team}");
    assert_eq!(team["name"], "New");
    let (status, refused, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/workspaces/{ws}/teams/{team_id}"),
        Some(&member_cookie),
        None,
        Some(json!({"name": "Stolen"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_permissions_page_lists_grants_audits_designations_and_ignores_stale_flags(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-pm-{}", Uuid::new_v4())).await;
    let admin = world.owner_cookie.clone();
    let (project_id, _) = new_project(world, ws, "Permissions").await;
    let project_uuid = Uuid::parse_str(&project_id).unwrap();
    let (editor_id, editor) = member(world, ws, "ed").await;
    let (reader_id, reader) = member(world, ws, "rd").await;
    let (team_user, team_cookie) = member(world, ws, "tm").await;
    grant(world, &project_id, editor_id, "EDITOR").await;
    grant(world, &project_id, reader_id, "VIEWER").await;
    let (s, team, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/teams"),
        Some(&admin),
        None,
        Some(json!({"name":"Builders","members":[{"userId":world.owner_id,"role":"MANAGER"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let team_id: Uuid = team["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("INSERT INTO penfii.team_members(team_id,user_id,role) VALUES($1,$2,'MEMBER') ON CONFLICT DO NOTHING").bind(team_id).bind(team_user).execute(&world.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO penfii.project_teams(project_id,team_id,access_role) VALUES($1,$2,'EDITOR')",
    )
    .bind(project_uuid)
    .bind(team_id)
    .execute(&world.pool)
    .await
    .unwrap();
    let path = format!("/v1/projects/{project_id}/permissions");

    // Only the Project's admin sees the page; everyone else is refused, a stranger finds nothing.
    for cookie in [&editor, &reader, &team_cookie] {
        assert_eq!(get_as(world, cookie, &path).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(
        get_as(
            world,
            &admin,
            &format!("/v1/projects/{}/permissions", Uuid::new_v4())
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (s, page) = get_as(world, &admin, &path).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    let find_member = |id: Uuid| {
        page["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["userId"] == id.to_string())
            .cloned()
            .unwrap()
    };
    assert_eq!(find_member(world.owner_id)["role"], "ADMIN");
    assert_eq!(find_member(world.owner_id)["designatable"], false);
    assert_eq!(find_member(editor_id)["designatable"], true);
    assert_eq!(find_member(reader_id)["designatable"], false);
    assert_eq!(page["teams"][0]["name"], "Builders");
    assert_eq!(page["teams"][0]["role"], "EDITOR");
    assert_eq!(
        page["capabilities"],
        json!(["dependency_waiver", "pin_management"])
    );

    // Someone who edits through a Team is not on the member list and cannot be designated.
    let put = |cookie: String, user: Uuid, waive: bool, pins: bool| {
        let app = world.app.clone();
        let path = format!("/v1/projects/{project_id}/members/{user}/designation");
        async move {
            call(
                &app,
                "PUT",
                &path,
                Some(&cookie),
                None,
                Some(json!({"canWaiveDependencies":waive,"canManagePins":pins})),
            )
            .await
        }
    };
    assert!(
        !page["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["userId"] == team_user.to_string())
    );
    assert_eq!(
        put(admin.clone(), team_user, true, true).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(editor.clone(), editor_id, true, true).await.0,
        StatusCode::FORBIDDEN
    );

    // Designating, repeating, changing and withdrawing: each real change is one audit row.
    let audit = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM penfii.audit_entries WHERE target_kind='PROJECT' AND target_id=$1 AND action_code='project.designation.set'")
            .bind(project_uuid).fetch_one(&world.pool).await.unwrap()
    };
    assert_eq!(audit().await, 0);
    assert_eq!(
        put(admin.clone(), editor_id, true, false).await.0,
        StatusCode::OK
    );
    assert_eq!(audit().await, 1);
    assert_eq!(
        put(admin.clone(), editor_id, true, false).await.0,
        StatusCode::OK
    );
    assert_eq!(audit().await, 1, "no change, no entry");
    assert_eq!(
        put(admin.clone(), editor_id, true, true).await.0,
        StatusCode::OK
    );
    assert_eq!(
        put(admin.clone(), editor_id, false, false).await.0,
        StatusCode::OK
    );
    assert_eq!(audit().await, 3);
    let (_, page) = get_as(world, &admin, &path).await;
    let changes = page["recentChanges"].as_array().unwrap();
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[0]["actorName"], "Owner");
    assert_eq!(changes[0]["userId"], editor_id.to_string());
    assert_eq!(
        changes[0]["before"],
        json!({"canWaiveDependencies":true,"canManagePins":true})
    );
    assert_eq!(
        changes[0]["after"],
        json!({"canWaiveDependencies":false,"canManagePins":false})
    );

    // A flag left on a grant that is no longer an editor's counts for nothing.
    assert_eq!(
        put(admin.clone(), editor_id, true, true).await.0,
        StatusCode::OK
    );
    sqlx::query(
        "UPDATE penfii.project_user_grants SET role='VIEWER' WHERE project_id=$1 AND user_id=$2",
    )
    .bind(project_uuid)
    .bind(editor_id)
    .execute(&world.pool)
    .await
    .unwrap();
    let (s, pin, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/pins"),
        Some(&editor),
        None,
        Some(json!({"url":"https://example.com","title":"x"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{pin}");
    let (_, page) = get_as(world, &admin, &path).await;
    assert_eq!(
        page["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["userId"] == editor_id.to_string())
            .unwrap()["designatable"],
        false
    );
}

// Sharing and access: one Project, its members and people who are not
async fn status_of(
    world: &World,
    who: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> StatusCode {
    let key = Uuid::new_v4().to_string();
    let (s, _, _) = call(&world.app, method, path, Some(who), Some(&key), body).await;
    s
}

fn readable(s: StatusCode) -> bool {
    s.is_success()
}

/// A Project's channels belong to the Project: the Space list and every read or write path
/// (history, thread, changes, reactions, posting, joining) refuse a Workspace member who is not in the
/// Project, in the same way for the Chat entry and the Project entry, and nobody outside the Workspace
/// reaches anything. A Project's tickets and board are read by every Workspace member (the Project
/// VIEWER rule) except tickets that came from a private channel.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn access_a_project_and_its_chat_docs_and_tickets_follow_one_rule_everywhere(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("acl-{}", Uuid::new_v4())).await;
    let other_ws = workspace(world, &format!("acl-other-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    // `bob` is a Workspace member who is in no Project; `dm_peer` shares one DM with the owner.
    let (bob_id, bob) = member(world, ws, "bob").await;
    let (peer_id, peer) = member(world, ws, "peer").await;
    let (project_id, _) = new_project(world, ws, "Access").await;
    let (_, space) = api(
        world,
        "GET",
        &owner,
        &format!("/v1/workspaces/{ws}/space"),
        None,
    )
    .await;
    let public: Uuid = space["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["projectId"].as_str() == Some(project_id.as_str()))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let (s, private_project) = api(world, "POST", &owner, &format!("/v1/workspaces/{ws}/channels"),
        Some(json!({"name":"Access private","visibility":"PRIVATE","teamId":null,"projectId":project_id}))).await;
    assert_eq!(s, StatusCode::CREATED, "{private_project}");
    let private_project: Uuid = private_project["id"].as_str().unwrap().parse().unwrap();
    let private_global = channel_as_owner_private(world, ws, "Access global private").await;
    let global_public = channel(world, ws, "Access global public", "PUBLIC").await;
    let (s, dm) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/workspaces/{ws}/dms"),
        Some(json!({"userIds":[world.owner_id, peer_id]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dm}");
    let dm: Uuid = dm["id"].as_str().unwrap().parse().unwrap();

    let (_, m_public) = send(world, &owner, public, "access public", None, None).await;
    let m_public: Uuid = m_public["id"].as_str().unwrap().parse().unwrap();
    let (_, m_priv_project) = send(
        world,
        &owner,
        private_project,
        "access private project",
        None,
        None,
    )
    .await;
    let m_priv_project: Uuid = m_priv_project["id"].as_str().unwrap().parse().unwrap();
    let (_, m_dm) = send(world, &owner, dm, "access dm", None, None).await;
    let m_dm: Uuid = m_dm["id"].as_str().unwrap().parse().unwrap();
    let (_, m_global) = send(world, &owner, global_public, "access global", None, None).await;
    let _ = (m_global, bob_id);
    let (s, _) = send(
        world,
        &owner,
        public,
        "access thread reply",
        Some(m_public),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // The Space list is what Chat and the Project screen both draw from.
    let ids = |v: &Value| -> Vec<String> {
        v["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let (_, bob_space) = api(
        world,
        "GET",
        &bob,
        &format!("/v1/workspaces/{ws}/space"),
        None,
    )
    .await;
    let bob_ids = ids(&bob_space);
    assert!(
        bob_ids.contains(&global_public.to_string()),
        "a public Workspace channel is listed"
    );
    for hidden in [public, private_project, private_global] {
        assert!(
            !bob_ids.contains(&hidden.to_string()),
            "{hidden} must not be listed to a non-member"
        );
    }
    let (_, owner_space) = api(
        world,
        "GET",
        &owner,
        &format!("/v1/workspaces/{ws}/space"),
        None,
    )
    .await;
    let owner_ids = ids(&owner_space);
    assert!(
        owner_ids.contains(&public.to_string()) && owner_ids.contains(&private_project.to_string()),
        "the Project's own people see both"
    );
    assert!(
        !bob_space["dms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"].as_str() == Some(&dm.to_string()))
    );

    // Every way of reading or writing a conversation the person is not part of says "not found".
    for (label, conv, msg) in [
        ("project public", public, m_public),
        ("project private", private_project, m_priv_project),
        ("global private", private_global, m_priv_project),
        ("dm", dm, m_dm),
    ] {
        for (who, cookie) in [("bob", &bob)] {
            for path in [
                format!("/v1/conversations/{conv}"),
                format!("/v1/conversations/{conv}/messages?limit=10"),
                format!("/v1/conversations/{conv}/changes?afterSeq=0"),
                format!("/v1/conversations/{conv}/read"),
            ] {
                assert_eq!(
                    status_of(world, cookie, "GET", &path, None).await,
                    StatusCode::NOT_FOUND,
                    "{label} {who} GET {path}"
                );
            }
            let post = Some(
                json!({"bodySchemaVersion":1,"body":doc("nope"),"threadRootId":null,"replyToId":null}),
            );
            let s = status_of(
                world,
                cookie,
                "POST",
                &format!("/v1/conversations/{conv}/messages"),
                post,
            )
            .await;
            assert!(!readable(s), "{label} {who} post -> {s}");
            let _ = msg;
        }
    }
    for path in [
        format!("/v1/messages/{m_public}/thread"),
        format!("/v1/messages/{m_public}/history"),
        format!("/v1/messages/{m_priv_project}/thread"),
        format!("/v1/messages/{m_dm}/thread"),
    ] {
        assert_eq!(
            status_of(world, &bob, "GET", &path, None).await,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert_eq!(
        status_of(
            world,
            &bob,
            "PUT",
            &format!("/v1/messages/{m_public}/reactions"),
            Some(json!({"emoji":"👍"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            world,
            &bob,
            "PATCH",
            &format!("/v1/messages/{m_public}"),
            Some(json!({"expectedVersion":1,"body":doc("x")}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            world,
            &bob,
            "DELETE",
            &format!("/v1/messages/{m_public}"),
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    for conv in [public, private_project, private_global] {
        assert_eq!(
            status_of(
                world,
                &bob,
                "POST",
                &format!("/v1/channels/{conv}/join"),
                None
            )
            .await,
            StatusCode::NOT_FOUND,
            "joining {conv}"
        );
    }
    // The DM is the DM members' alone; the Workspace owner reading a DM she is in is fine.
    assert!(readable(
        status_of(
            world,
            &peer,
            "GET",
            &format!("/v1/conversations/{dm}/messages?limit=5"),
            None
        )
        .await
    ));
    // Somebody in another Workspace entirely.
    let (_, outsider) = member(world, other_ws, "outsider").await;
    for path in [
        format!("/v1/workspaces/{ws}/space"),
        format!("/v1/conversations/{global_public}/messages?limit=5"),
        format!("/v1/projects/{project_id}/board"),
    ] {
        assert_eq!(
            status_of(world, &outsider, "GET", &path, None).await,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    // Tickets: a participant reads a Project's tickets, but a ticket made in a private channel
    // follows that channel; reading is not editing.
    let (s, open) = make_ticket(
        world,
        &project_id,
        "acl-open",
        json!({"title":"Open ticket"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{open}");
    let (s, closed) = make_ticket(
        world,
        &project_id,
        "acl-closed",
        json!({"title":"Private-channel ticket","channelId":private_project}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{closed}");
    let (open_id, closed_id) = (open["id"].as_str().unwrap(), closed["id"].as_str().unwrap());
    // Not taking part in the Project: its tickets and board are not found at all.
    assert_eq!(
        status_of(world, &bob, "GET", &format!("/v1/tickets/{open_id}"), None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            world,
            &bob,
            "GET",
            &format!("/v1/projects/{project_id}/board"),
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    grant_user(world, &project_id, bob_id, "VIEWER").await;
    assert!(readable(
        status_of(world, &bob, "GET", &format!("/v1/tickets/{open_id}"), None).await
    ));
    assert_eq!(
        status_of(
            world,
            &bob,
            "PATCH",
            &format!("/v1/tickets/{open_id}"),
            Some(json!({"expectedRevision":1,"priority":"URGENT"}))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    for path in [
        format!("/v1/tickets/{closed_id}"),
        format!("/v1/tickets/{closed_id}/comments"),
        format!("/v1/tickets/{closed_id}/activity"),
        format!("/v1/tickets/{closed_id}/revisions"),
        format!("/v1/tickets/{closed_id}/dependencies"),
        format!("/v1/tickets/{closed_id}/subscription"),
    ] {
        assert_eq!(
            status_of(world, &bob, "GET", &path, None).await,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let (_, board) = api(
        world,
        "GET",
        &bob,
        &format!("/v1/projects/{project_id}/board"),
        None,
    )
    .await;
    let on_board = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().iter())
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(on_board, vec!["Open ticket".to_owned()], "{board}");

    // Docs: RESTRICTED pages and everything hanging off them follow the page.
    let restricted = make_doc(
        world,
        &owner,
        ws,
        "Access restricted",
        json!({"visibility":"RESTRICTED"}),
    )
    .await;
    let rid = restricted["id"].as_str().unwrap();
    let (s, c) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/docs/{rid}/comments"),
        Some(json!({"body": doc("restricted comment")})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let (s, v) = save_doc(
        world,
        &owner,
        rid,
        1,
        "Access restricted",
        "restricted body",
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/docs/{rid}/versions"),
        Some(json!({"expectedRevisionNo": 2})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    for (method, path, body) in [
        ("GET", format!("/v1/docs/{rid}"), None),
        ("GET", format!("/v1/docs/{rid}/comments"), None),
        ("GET", format!("/v1/docs/{rid}/versions"), None),
        ("GET", format!("/v1/docs/{rid}/versions/1"), None),
        ("GET", format!("/v1/docs/{rid}/participants"), None),
        (
            "POST",
            format!("/v1/docs/{rid}/comments"),
            Some(json!({"body": doc("x")})),
        ),
        (
            "PUT",
            format!("/v1/docs/{rid}/content"),
            Some(json!({"expectedRevisionNo":3,"title":"x","content":page_content("x")})),
        ),
        (
            "POST",
            format!("/v1/docs/{rid}/versions"),
            Some(json!({"expectedRevisionNo":3})),
        ),
        (
            "PATCH",
            format!("/v1/docs/{rid}"),
            Some(json!({"title":"x"})),
        ),
        (
            "PUT",
            format!("/v1/docs/{rid}/visibility"),
            Some(json!({"visibility":"WORKSPACE"})),
        ),
        ("DELETE", format!("/v1/docs/{rid}"), None),
    ] {
        assert_eq!(
            status_of(world, &bob, method, &path, body).await,
            StatusCode::NOT_FOUND,
            "{method} {path}"
        );
    }
    let (_, tree) = api(
        world,
        "GET",
        &bob,
        &format!("/v1/workspaces/{ws}/docs/tree"),
        None,
    )
    .await;
    assert!(!node_ids(&tree).contains(&rid.to_owned()));
    // A Project document that is shared with the Workspace is readable by it; a Project does not narrow it.
    let shared = make_doc(
        world,
        &owner,
        ws,
        "Access shared",
        json!({"projectId": project_id}),
    )
    .await;
    assert!(readable(
        status_of(
            world,
            &bob,
            "GET",
            &format!("/v1/docs/{}", shared["id"].as_str().unwrap()),
            None
        )
        .await
    ));

    // The index may hold everything; what a person is shown is decided by PostgreSQL.
    if let Some(sw) = search_world(world).await {
        index_now(&sw, ws).await;
        for (query, kind) in [
            ("access", "message"),
            ("restricted", "doc"),
            ("Private-channel", "ticket"),
            ("Access", "space"),
            ("access", "all"),
        ] {
            let (s, b) = find(&sw, &bob, ws, query, kind).await;
            assert_eq!(s, StatusCode::OK);
            let seen = titles(&b).join(" | ") + &b.to_string();
            for secret in [
                "access public",
                "access private project",
                "access dm",
                "restricted body",
                "restricted comment",
                "Access restricted",
                "Private-channel ticket",
                "Access private",
                "Access global private",
            ] {
                assert!(
                    !seen.contains(secret),
                    "bob found {secret:?} searching {query:?}/{kind}: {b}"
                );
            }
            let (_, mine) = find(&sw, &outsider, ws, query, kind).await;
            assert!(
                mine["results"].as_array().is_none_or(Vec::is_empty) || mine["code"].is_string(),
                "outsider: {mine}"
            );
        }
        for (query, kind, what) in [
            ("public", "message", "a message of the Project channel"),
            ("restricted", "doc", "the restricted page"),
        ] {
            let (_, found) = find(&sw, &owner, ws, query, kind).await;
            assert!(
                found["results"].as_array().is_some_and(|r| !r.is_empty()),
                "the people the object belongs to do find {what}: {found}"
            );
        }
    }
}

async fn channel_as_owner_private(world: &World, ws: Uuid, name: &str) -> Uuid {
    channel(world, ws, name, "PRIVATE").await
}

// Share scope: where an object lives is not who it is shared with
async fn team_named(world: &World, ws: Uuid, name: &str) -> Uuid {
    let (s, v) = api(
        world,
        "POST",
        &world.owner_cookie,
        &format!("/v1/workspaces/{ws}/teams"),
        Some(json!({"name": name, "members": [{"userId": world.owner_id, "role": "MANAGER"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().parse().unwrap()
}

async fn project_with_teams(world: &World, ws: Uuid, name: &str, teams: &[Uuid]) -> Uuid {
    let (s, v) = api(
        world,
        "POST",
        &world.owner_cookie,
        &format!("/v1/workspaces/{ws}/projects"),
        Some(json!({"name": name, "teamIds": teams})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let project: Uuid = v["id"].as_str().unwrap().parse().unwrap();
    // The Teams take part with the editor role; a link alone gives no one access.
    for team in teams {
        let (s, v) = api(
            world,
            "PUT",
            &world.owner_cookie,
            &format!("/v1/projects/{project}/teams/{team}/participation"),
            Some(json!({"role": "EDITOR"})),
        )
        .await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    }
    project
}

async fn join_team(world: &World, team: Uuid, user: Uuid) {
    sqlx::query("INSERT INTO penfii.team_members(team_id,user_id,role) VALUES($1,$2,'MEMBER') ON CONFLICT DO NOTHING")
        .bind(team).bind(user).execute(&world.pool).await.unwrap();
}

/// "Project A", "Project A + Design Team" (the intersection), and "invited people only" are different
/// audiences; the Project a page is made in does not decide them; a Team never opens a Project's material
/// by itself; people outside the default audience can be invited by name.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn share_scope_is_a_project_or_a_project_and_team_intersection_or_invited_people(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("scope-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let design = team_named(world, ws, "Design").await;
    let marketing = team_named(world, ws, "Marketing").await;
    let design_b = team_named(world, ws, "Design (B)").await;
    let project_a = project_with_teams(world, ws, "Scope A", &[design, marketing]).await;
    let project_b = project_with_teams(world, ws, "Scope B", &[design_b]).await;
    // d: Design of A; m: Marketing of A; b: Design of B; g: in A by name only; x: in the Workspace only; inv: invited by name.
    let (d_id, d) = member(world, ws, "design").await;
    let (m_id, m) = member(world, ws, "marketing").await;
    let (b_id, b) = member(world, ws, "design-b").await;
    let (g_id, g) = member(world, ws, "granted").await;
    let (_, x) = member(world, ws, "nobody").await;
    let (inv_id, inv) = member(world, ws, "invited").await;
    join_team(world, design, d_id).await;
    join_team(world, marketing, m_id).await;
    join_team(world, design_b, b_id).await;
    sqlx::query(
        "INSERT INTO penfii.project_user_grants(project_id,user_id,role) VALUES($1,$2,'EDITOR')",
    )
    .bind(project_a)
    .bind(g_id)
    .execute(&world.pool)
    .await
    .unwrap();

    let can = |s: StatusCode| s.is_success();
    let read = |who: String, id: String| async move {
        status_of(world, &who, "GET", &format!("/v1/docs/{id}"), None).await
    };
    // Pages made in Project A, shared three different ways.
    let in_project = make_doc(
        world,
        &owner,
        ws,
        "A people",
        json!({"projectId": project_a, "visibility": "PROJECT", "scopeProjectId": project_a}),
    )
    .await;
    let in_team = make_doc(world, &owner, ws, "A + Design", json!({"projectId": project_a, "visibility": "PROJECT", "scopeProjectId": project_a, "scopeTeamId": design,
        "participants": [{"userId": inv_id, "role": "EDITOR"}]})).await;
    let private = make_doc(
        world,
        &owner,
        ws,
        "Private",
        json!({"projectId": project_a, "visibility": "RESTRICTED"}),
    )
    .await;
    // The place is not the audience: made in A, open to the Workspace; made in B, shared with A.
    let open_in_a = make_doc(
        world,
        &owner,
        ws,
        "Open, made in A",
        json!({"projectId": project_a}),
    )
    .await;
    let a_in_b = make_doc(
        world,
        &owner,
        ws,
        "A's, made in B",
        json!({"projectId": project_b, "visibility": "PROJECT", "scopeProjectId": project_a}),
    )
    .await;
    let ids = |v: &Value| v["id"].as_str().unwrap().to_owned();
    let (in_project, in_team, private, open_in_a, a_in_b) = (
        ids(&in_project),
        ids(&in_team),
        ids(&private),
        ids(&open_in_a),
        ids(&a_in_b),
    );
    assert_eq!(
        make_doc_scope(world, &owner, &in_team).await,
        (json!(project_a), json!(design))
    );

    // who -> [A people, A + Design, Private, Open, A's made in B]
    let table: [(&str, &String, [bool; 5]); 7] = [
        (
            "owner (in A and Design)",
            &owner,
            [true, true, true, true, true],
        ),
        ("Design of A", &d, [true, true, false, true, true]),
        ("Marketing of A", &m, [true, false, false, true, true]),
        ("in A by name only", &g, [true, false, false, true, true]),
        ("Design of B", &b, [false, false, false, true, false]),
        (
            "Workspace member only",
            &x,
            [false, false, false, true, false],
        ),
        ("invited by name", &inv, [false, true, false, true, false]),
    ];
    for (label, cookie, want) in table {
        for (i, id) in [&in_project, &in_team, &private, &open_in_a, &a_in_b]
            .into_iter()
            .enumerate()
        {
            let got = can(read((*cookie).clone(), id.clone()).await);
            // "Private" is the owner's alone; the owner-only row says so for the owner.
            let expect = if i == 2 {
                label.starts_with("owner")
            } else {
                want[i]
            };
            assert_eq!(got, expect, "{label} reading page #{i}");
        }
    }
    // Everything hanging off a page follows it: comments, Versions, the tree, a second path to the same page.
    for path in [
        format!("/v1/docs/{in_team}/comments"),
        format!("/v1/docs/{in_team}/versions"),
        format!("/v1/docs/{in_team}/participants"),
    ] {
        assert!(
            can(status_of(world, &d, "GET", &path, None).await),
            "{path} as Design"
        );
        assert!(
            !can(status_of(world, &m, "GET", &path, None).await),
            "{path} as Marketing"
        );
        assert!(
            !can(status_of(world, &b, "GET", &path, None).await),
            "{path} as Design of B"
        );
    }
    let (_, tree) = api(
        world,
        "GET",
        &m,
        &format!("/v1/workspaces/{ws}/docs/tree"),
        None,
    )
    .await;
    assert!(
        node_ids(&tree).contains(&in_project) && !node_ids(&tree).contains(&in_team),
        "{tree}"
    );
    // The invited person edits; the audience cannot change the audience.
    let (s, _) = save_doc(world, &inv, &in_team, 1, "A + Design", "by invitee").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        status_of(
            world,
            &d,
            "PUT",
            &format!("/v1/docs/{in_team}/visibility"),
            Some(json!({"visibility":"WORKSPACE"}))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(
            world,
            &m,
            "PUT",
            &format!("/v1/docs/{in_team}/visibility"),
            Some(json!({"visibility":"WORKSPACE"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // The scope is checked when it is set.
    let docs_path = format!("/v1/workspaces/{ws}/docs");
    let post = |body: Value| api(world, "POST", &owner, &docs_path, Some(body));
    for (why, body) in [
        (
            "a Team of another Project",
            json!({"title":"t","visibility":"PROJECT","scopeProjectId":project_a,"scopeTeamId":design_b}),
        ),
        (
            "a Team without its Project",
            json!({"title":"t","visibility":"PROJECT","scopeTeamId":design}),
        ),
        (
            "PROJECT without a Project",
            json!({"title":"t","visibility":"PROJECT"}),
        ),
        (
            "a scope on a Workspace page",
            json!({"title":"t","visibility":"WORKSPACE","scopeProjectId":project_a}),
        ),
        (
            "an unknown Project",
            json!({"title":"t","visibility":"PROJECT","scopeProjectId":Uuid::new_v4()}),
        ),
    ] {
        let (s, v) = post(body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{why}: {v}");
    }
    // Changing the scope later: Workspace -> A + Marketing; then Marketing reads and Design no longer does by Team alone.
    let page = make_doc(world, &owner, ws, "Moves", json!({})).await;
    let moves = ids(&page);
    let (s, v) = api(
        world,
        "PUT",
        &owner,
        &format!("/v1/docs/{moves}/visibility"),
        Some(json!({"visibility":"PROJECT","scopeProjectId":project_a,"scopeTeamId":marketing})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(
        can(read(m.clone(), moves.clone()).await)
            && !can(read(d.clone(), moves.clone()).await)
            && !can(read(x.clone(), moves.clone()).await)
    );
    let (s, v) = api(
        world,
        "PUT",
        &owner,
        &format!("/v1/docs/{moves}/visibility"),
        Some(json!({"visibility":"WORKSPACE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["scopeProjectId"], Value::Null);
    assert!(can(read(x.clone(), moves.clone()).await));

    // Channels: a Project's channel, narrowed to a Team, is read, joined and written by that intersection only.
    let (s, ch) = api(world, "POST", &owner, &format!("/v1/workspaces/{ws}/channels"), Some(json!({"name":"design only","visibility":"PUBLIC","projectId":project_a,"teamId":design}))).await;
    assert_eq!(s, StatusCode::CREATED, "{ch}");
    let ch: Uuid = ch["id"].as_str().unwrap().parse().unwrap();
    let (s, _) = send(world, &owner, ch, "design channel", None, None).await;
    assert_eq!(s, StatusCode::CREATED);
    for (label, cookie, want) in [
        ("Design of A", &d, true),
        ("Marketing of A", &m, false),
        ("in A by name", &g, false),
        ("Design of B", &b, false),
        ("nobody", &x, false),
    ] {
        assert_eq!(
            can(status_of(
                world,
                cookie,
                "GET",
                &format!("/v1/conversations/{ch}/messages?limit=5"),
                None
            )
            .await),
            want,
            "{label} reads"
        );
        let (_, space) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/space"),
            None,
        )
        .await;
        let listed = space["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"].as_str() == Some(&ch.to_string()));
        assert_eq!(listed, want, "{label} sees it in the Space list");
        assert_eq!(
            can(status_of(
                world,
                cookie,
                "POST",
                &format!("/v1/channels/{ch}/join"),
                None
            )
            .await),
            want,
            "{label} joins"
        );
    }
    let (s, v) = send(world, &d, ch, "from design", None, None).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    // A Team is not a place on its own, nor one of another Project's.
    for body in [
        json!({"name":"team only","visibility":"PUBLIC","teamId":design}),
        json!({"name":"x","visibility":"PUBLIC","projectId":project_a,"teamId":design_b}),
    ] {
        let (s, v) = api(
            world,
            "POST",
            &owner,
            &format!("/v1/workspaces/{ws}/channels"),
            Some(body),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    // Search follows the pages (the index is not an authority).
    if let Some(sw) = search_world(world).await {
        index_now(&sw, ws).await;
        let (_, found) = find(&sw, &m, ws, "invitee", "doc").await;
        assert!(titles(&found).iter().all(|t| t != "A + Design"), "{found}");
        let (_, found) = find(&sw, &d, ws, "invitee", "doc").await;
        assert!(titles(&found).contains(&"A + Design".to_owned()), "{found}");
        let (_, found) = find(&sw, &b, ws, "people", "doc").await;
        assert!(titles(&found).iter().all(|t| t != "A people"), "{found}");
    }
    let _ = (inv, m_id);
}

async fn make_doc_scope(world: &World, cookie: &str, id: &str) -> (Value, Value) {
    let (_, v) = api(world, "GET", cookie, &format!("/v1/docs/{id}"), None).await;
    (v["scopeProjectId"].clone(), v["scopeTeamId"].clone())
}

/// A Project is found by every Workspace member and opened only by its participants (a Project grant, or a
/// Team linked to it). The same answer comes from the list, the Project's own routes, its Docs and Databases,
/// the Chat list, a direct ID and Search; a Share Scope inside the Project stays what it was.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_project_is_listed_for_everyone_and_opened_only_by_its_participants(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("pjvis-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let design = team_named(world, ws, "Design").await;
    let project_a = project_with_teams(world, ws, "Visible A", &[design]).await;
    let project_b = project_with_teams(world, ws, "Visible B", &[]).await;
    let (d_id, d) = member(world, ws, "team-member").await;
    let (g_id, g) = member(world, ws, "granted").await;
    let (_, x) = member(world, ws, "outsider").await;
    join_team(world, design, d_id).await;
    sqlx::query(
        "INSERT INTO penfii.project_user_grants(project_id,user_id,role) VALUES($1,$2,'VIEWER')",
    )
    .bind(project_a)
    .bind(g_id)
    .execute(&world.pool)
    .await
    .unwrap();

    // The list: everyone finds both Projects; only the participants are marked as taking part.
    for (who, cookie, a, b) in [
        ("owner", &owner, true, true),
        ("team member", &d, true, false),
        ("grant", &g, true, false),
        ("outsider", &x, false, false),
    ] {
        let (s, space) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/space"),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{space}");
        let flag = |id: Uuid| {
            space["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == json!(id))
                .map(|p| p["participating"].as_bool().unwrap())
        };
        assert_eq!(
            (flag(project_a), flag(project_b)),
            (Some(a), Some(b)),
            "{who}: found everywhere, taking part only where it is true"
        );
    }

    // The inside: participants open it, the outsider gets "not found" on every path, by ID too.
    let inside = [
        format!("/v1/projects/{project_a}/board"),
        format!("/v1/projects/{project_a}/members"),
        format!("/v1/projects/{project_a}/workflow"),
        format!("/v1/projects/{project_a}/labels"),
        format!("/v1/projects/{project_a}/activity"),
        format!("/v1/projects/{project_a}/planning"),
        format!("/v1/workspaces/{ws}/docs/tree?projectId={project_a}"),
    ];
    for path in &inside {
        assert!(
            status_of(world, &d, "GET", path, None).await.is_success(),
            "team member opens {path}"
        );
        assert!(
            status_of(world, &g, "GET", path, None).await.is_success(),
            "grant opens {path}"
        );
        assert_eq!(
            status_of(world, &x, "GET", path, None).await,
            StatusCode::NOT_FOUND,
            "outsider on {path}"
        );
    }
    // Creating inside a Project needs taking part in it.
    assert_eq!(
        status_of(
            world,
            &x,
            "POST",
            &format!("/v1/workspaces/{ws}/docs"),
            Some(json!({"title":"Sneak","projectId":project_a,"visibility":"WORKSPACE"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert!(
        status_of(
            world,
            &d,
            "POST",
            &format!("/v1/workspaces/{ws}/docs"),
            Some(json!({"title":"Fine","projectId":project_a,"visibility":"WORKSPACE"}))
        )
        .await
        .is_success()
    );

    // A Ticket of the Project is as hidden as the Project: by ID and in Search.
    let (s, t) = make_ticket(
        world,
        &project_a.to_string(),
        "pjvis-t",
        json!({"title":"Inside ticket zebra"}),
    )
    .await;
    assert!(s.is_success(), "{t}");
    let ticket = t["id"].as_str().unwrap().to_owned();
    assert!(
        status_of(world, &d, "GET", &format!("/v1/tickets/{ticket}"), None)
            .await
            .is_success()
    );
    assert_eq!(
        status_of(world, &x, "GET", &format!("/v1/tickets/{ticket}"), None).await,
        StatusCode::NOT_FOUND
    );

    // Work is given only to people who take part: on create, on a new assignee, and in the assignee list.
    let x_id_row = user_id_of_cookie(world, &x).await;
    let ticket_path = format!("/v1/tickets/{ticket}");
    let (s, v) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/projects/{project_a}/tickets"),
        Some(json!({"title":"For the outsider","hierarchyKind":"TASK","assigneeId":x_id_row})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (_, current) = api(world, "GET", &owner, &ticket_path, None).await;
    let rev = current["revisionNo"]
        .as_i64()
        .or(current["revision"].as_i64())
        .unwrap_or(1);
    let (s, v) = call_with_key(
        world,
        "PATCH",
        &owner,
        &ticket_path,
        "pjvis-assign-x",
        json!({"expectedRevision":rev,"assigneeId":x_id_row}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = call_with_key(
        world,
        "PATCH",
        &owner,
        &ticket_path,
        "pjvis-assign-d",
        json!({"expectedRevision":rev,"assigneeId":d_id}),
    )
    .await;
    assert!(s.is_success(), "a Team member can be assigned: {v}");
    let (_, people) = api(
        world,
        "GET",
        &owner,
        &format!("/v1/projects/{project_a}/members"),
        None,
    )
    .await;
    let listed: Vec<String> = people
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["userId"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        listed.contains(&d_id.to_string())
            && listed.contains(&g_id.to_string())
            && !listed.contains(&x_id_row.to_string()),
        "{people}"
    );

    // A Workspace admin who does not take part has no way in either: reads, writes, a channel, a page made there.
    let admin = admin_cookie(
        world,
        ws,
        &format!("pjvis-admin-{}@penfii.test", Uuid::new_v4()),
    )
    .await;
    for path in &inside {
        assert_eq!(
            status_of(world, &admin, "GET", path, None).await,
            StatusCode::NOT_FOUND,
            "Workspace admin on {path}"
        );
    }
    assert_eq!(
        status_of(world, &admin, "GET", &ticket_path, None).await,
        StatusCode::NOT_FOUND
    );
    let (s, v) = api(
        world,
        "POST",
        &admin,
        &format!("/v1/workspaces/{ws}/channels"),
        Some(json!({"name":"admin-in","visibility":"PUBLIC","teamId":null,"projectId":project_a})),
    )
    .await;
    assert!(!s.is_success(), "{v}");
    let (_, aspace) = api(
        world,
        "GET",
        &admin,
        &format!("/v1/workspaces/{ws}/space"),
        None,
    )
    .await;
    assert!(
        aspace["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == json!(project_a) && p["participating"] == json!(false)),
        "found in the list, not taking part"
    );

    // A page made in the Project but shared with the Workspace stays readable: Project access and Share Scope are two rules.
    let open = make_doc(
        world,
        &owner,
        ws,
        "Open, made in A",
        json!({"projectId": project_a}),
    )
    .await;
    let open = open["id"].as_str().unwrap().to_owned();
    assert!(
        status_of(world, &x, "GET", &format!("/v1/docs/{open}"), None)
            .await
            .is_success()
    );
    assert!(
        status_of(world, &admin, "GET", &format!("/v1/docs/{open}"), None)
            .await
            .is_success()
    );
}

/// A Workspace member who takes part in a Project through a grant of their own (the Project's inside is for participants only).
pub(super) async fn grant_project(world: &World, project: &str, email: &str, role: &str) {
    let user = user_id_of(world, email).await;
    sqlx::query("INSERT INTO penfii.project_user_grants(project_id,user_id,role) VALUES($1::uuid,$2,$3) ON CONFLICT (project_id,user_id) DO UPDATE SET role=EXCLUDED.role")
        .bind(project).bind(user).bind(role).execute(&world.pool).await.unwrap();
}

pub(super) async fn grant_user(world: &World, project: &str, user: Uuid, role: &str) {
    sqlx::query("INSERT INTO penfii.project_user_grants(project_id,user_id,role) VALUES($1::uuid,$2,$3) ON CONFLICT (project_id,user_id) DO UPDATE SET role=EXCLUDED.role")
        .bind(project).bind(user).bind(role).execute(&world.pool).await.unwrap();
}

async fn user_id_of_cookie(world: &World, cookie: &str) -> Uuid {
    let (_, me) = api(world, "GET", cookie, "/v1/me", None).await;
    me["id"].as_str().unwrap().parse().unwrap()
}

async fn call_with_key(
    world: &World,
    method: &str,
    cookie: &str,
    path: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (s, v, _) = call(
        &world.app,
        method,
        path,
        Some(cookie),
        Some(key),
        Some(body),
    )
    .await;
    (s, v)
}

/// A Workspace ADMIN who is in no Project.
async fn admin_cookie(world: &World, workspace_id: Uuid, email: &str) -> String {
    let (status, invitation, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invitations"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"email":email,"role":"ADMIN"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invitation}");
    let (status, _, _) = call(&world.app, "POST", "/v1/invitations/accept", None, None, Some(json!({"token":invitation["inviteToken"],"displayName":"Admin Two","password":"admin-two-password"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, cookie) = call(
        &world.app,
        "POST",
        "/v1/session/login",
        None,
        None,
        Some(json!({"email":email,"password":"admin-two-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    cookie.unwrap()
}

// Visibility matrix: one table of who may read what, checked at every entry point
/// The people of the matrix, in the order of every expectation row below.
const WHO: [&str; 6] = ["owner", "admin", "direct", "team", "plain", "removed"];

/// One resource and the people (by index into [`WHO`]) who may read it. Every list, every direct
/// read and Search must agree with this single row, whichever way the resource is reached.
struct Row {
    name: &'static str,
    readers: [bool; 6],
}

fn ids_of(value: &Value, key: &str) -> std::collections::BTreeSet<String> {
    value[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// People are the Workspace owner (who made the Project, so takes part), a Workspace admin who does not,
/// a Project grant, a member of a Team linked to the Project, a plain member of neither, and a member who
/// was removed. A Workspace role, a Project's participants and a resource's own audience are three
/// different axes; none of them is derived from another.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_visibility_matrix_gives_one_answer_at_every_entry_point(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("matrix-{}", Uuid::new_v4())).await;
    let design = team_named(world, ws, "Design").await;
    let project = project_with_teams(world, ws, "Matrix", &[design]).await;
    let admin = admin_cookie(
        world,
        ws,
        &format!("mx-admin-{}@penfii.test", Uuid::new_v4()),
    )
    .await;
    let (direct_id, direct) = member(world, ws, "mx-direct").await;
    let (team_id, team) = member(world, ws, "mx-team").await;
    let (plain_id, plain) = member(world, ws, "mx-plain").await;
    let (removed_id, removed) = member(world, ws, "mx-removed").await;
    grant_user(world, &project.to_string(), direct_id, "VIEWER").await;
    join_team(world, design, team_id).await;
    let cookies = [
        world.owner_cookie.clone(),
        admin,
        direct,
        team,
        plain,
        removed,
    ];
    let word = "matrixword";

    // Channels: a global public one, a global private one, a Project public one, a Project private one, a DM.
    let global_public = channel(world, ws, "mx global public", "PUBLIC").await;
    let global_private = channel(world, ws, "mx global private", "PRIVATE").await;
    let project_public: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.channels WHERE project_id=$1")
            .bind(project)
            .fetch_one(&world.pool)
            .await
            .unwrap();
    let (s, v) = api(world, "POST", &cookies[0], &format!("/v1/workspaces/{ws}/channels"),
        Some(json!({"name":"mx project private","visibility":"PRIVATE","teamId":null,"projectId":project}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let project_private: Uuid = v["id"].as_str().unwrap().parse().unwrap();
    let (s, v) = api(
        world,
        "POST",
        &cookies[0],
        &format!("/v1/workspaces/{ws}/dms"),
        Some(json!({"userIds":[world.owner_id, direct_id]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let dm: Uuid = v["id"].as_str().unwrap().parse().unwrap();
    let channels = [
        (
            global_public,
            Row {
                name: "global public channel",
                readers: [true, true, true, true, true, false],
            },
        ),
        (
            global_private,
            Row {
                name: "global private channel",
                readers: [true, false, false, false, false, false],
            },
        ),
        (
            project_public,
            Row {
                name: "project public channel",
                readers: [true, false, true, true, false, false],
            },
        ),
        (
            project_private,
            Row {
                name: "project private channel",
                readers: [true, false, false, false, false, false],
            },
        ),
        (
            dm,
            Row {
                name: "direct message",
                readers: [true, false, true, false, false, false],
            },
        ),
    ];
    let mut message_ids = Vec::new();
    for (id, _) in &channels {
        let (s, m) = send(
            world,
            &cookies[0],
            *id,
            &format!("{word} message"),
            None,
            None,
        )
        .await;
        assert!(s.is_success(), "{m}");
        message_ids.push(m["id"].as_str().unwrap().to_owned());
    }

    // Pages: Workspace, Project, Project + Team, private, and private with one invited person (plain).
    let project_s = project.to_string();
    let docs = [
        (make_doc(world, &cookies[0], ws, &format!("{word} workspace page"), json!({"projectId": project})).await,
         Row { name: "workspace page", readers: [true, true, true, true, true, false] }),
        (make_doc(world, &cookies[0], ws, &format!("{word} project page"), json!({"projectId": project, "visibility":"PROJECT", "scopeProjectId": project})).await,
         Row { name: "project page", readers: [true, false, true, true, false, false] }),
        (make_doc(world, &cookies[0], ws, &format!("{word} team page"), json!({"projectId": project, "visibility":"PROJECT", "scopeProjectId": project, "scopeTeamId": design})).await,
         Row { name: "project + team page", readers: [true, false, false, true, false, false] }),
        (make_doc(world, &cookies[0], ws, &format!("{word} private page"), json!({"projectId": project, "visibility":"RESTRICTED"})).await,
         Row { name: "private page", readers: [true, false, false, false, false, false] }),
        (make_doc(world, &cookies[0], ws, &format!("{word} invited page"), json!({"projectId": project, "visibility":"RESTRICTED", "participants":[{"userId": plain_id, "role":"EDITOR"}]})).await,
         Row { name: "invited page", readers: [true, false, false, false, true, false] }),
    ];

    // Tickets: one of the Project, one that came from the Project's private channel.
    let (s, t_open) = make_ticket(
        world,
        &project_s,
        "mx-open",
        json!({"title": format!("{word} open ticket"), "hierarchyKind":"TASK"}),
    )
    .await;
    assert!(s.is_success(), "{t_open}");
    let (s, t_private) = make_ticket(world, &project_s, "mx-private", json!({"title": format!("{word} private ticket"), "hierarchyKind":"TASK", "channelId": project_private})).await;
    assert!(s.is_success(), "{t_private}");
    let tickets = [
        (
            t_open["id"].as_str().unwrap().to_owned(),
            Row {
                name: "project ticket",
                readers: [true, false, true, true, false, false],
            },
        ),
        (
            t_private["id"].as_str().unwrap().to_owned(),
            Row {
                name: "ticket of a private channel",
                readers: [true, false, false, false, false, false],
            },
        ),
    ];

    // The member is removed after everything exists: access ends at once, in every entry point.
    sqlx::query(
        "UPDATE penfii.workspace_members SET state='INACTIVE', deactivated_at=now() WHERE workspace_id=$1 AND user_id=$2",
    )
    .bind(ws)
    .bind(removed_id)
    .execute(&world.pool)
    .await
    .unwrap();

    let search = search_world(world).await;
    if let Some(sw) = &search {
        index_now(sw, ws).await;
    }

    for (at, who) in WHO.iter().enumerate() {
        let cookie = &cookies[at];
        let active = at != 5;
        // Entry: the Space list (Chat, Context Nav). A Project is found by every active member; its participation flag is the Project rule.
        let (s, space) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/space"),
            None,
        )
        .await;
        if active {
            assert_eq!(s, StatusCode::OK, "{who}: {space}");
            let participating = space["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == json!(project))
                .map(|p| p["participating"].as_bool().unwrap());
            assert_eq!(
                participating,
                Some([true, false, true, true, false][at]),
                "{who}: found by all, takes part only by the Project rule"
            );
            let listed = ids_of(&space, "channels");
            for (id, row) in channels.iter().filter(|(id, _)| *id != dm) {
                assert_eq!(
                    listed.contains(&id.to_string()),
                    row.readers[at],
                    "{who}: Space list of {}",
                    row.name
                );
            }
            let dms = ids_of(&space, "dms");
            assert_eq!(
                dms.contains(&dm.to_string()),
                channels[4].1.readers[at],
                "{who}: Space list of the direct message"
            );
        } else {
            assert!(
                !s.is_success(),
                "{who}: a removed member gets no Space list"
            );
        }
        // Entry: reading a conversation by ID, which the list, the thread and every history call share.
        for (id, row) in &channels {
            for path in [
                format!("/v1/conversations/{id}"),
                format!("/v1/conversations/{id}/messages"),
            ] {
                let got = readable(status_of(world, cookie, "GET", &path, None).await);
                assert_eq!(got, row.readers[at], "{who}: {path} ({})", row.name);
            }
        }
        // Entry: the Docs tree (Docs, Context Nav), a page by ID and its comments.
        let (_, tree) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/docs/tree"),
            None,
        )
        .await;
        let listed = ids_of(&tree, "nodes");
        let (s, project_tree) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/docs/tree?projectId={project}"),
            None,
        )
        .await;
        let in_project = ids_of(&project_tree, "nodes");
        assert_eq!(
            readable(s),
            [true, false, true, true, false, false][at],
            "{who}: the Project's page list opens by the Project rule"
        );
        for (doc, row) in &docs {
            let id = doc["id"].as_str().unwrap().to_owned();
            assert_eq!(
                listed.contains(&id),
                row.readers[at],
                "{who}: Docs tree lists {}",
                row.name
            );
            if readable(s) {
                assert_eq!(
                    in_project.contains(&id),
                    row.readers[at],
                    "{who}: Project page list shows {}",
                    row.name
                );
            }
            for path in [format!("/v1/docs/{id}"), format!("/v1/docs/{id}/comments")] {
                let got = readable(status_of(world, cookie, "GET", &path, None).await);
                assert_eq!(got, row.readers[at], "{who}: {path} ({})", row.name);
            }
        }
        // Entry: a Ticket by ID and the Project's board.
        let (s, board) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/projects/{project}/board"),
            None,
        )
        .await;
        assert_eq!(
            readable(s),
            [true, false, true, true, false, false][at],
            "{who}: the board opens by the Project rule"
        );
        let on_board: std::collections::BTreeSet<String> = board["columns"]
            .as_array()
            .map(|columns| {
                columns
                    .iter()
                    .flat_map(|c| c["tickets"].as_array().cloned().unwrap_or_default())
                    .filter_map(|t| t["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        for (id, row) in &tickets {
            assert_eq!(
                readable(status_of(world, cookie, "GET", &format!("/v1/tickets/{id}"), None).await),
                row.readers[at],
                "{who}: ticket by ID, {}",
                row.name
            );
            if readable(s) {
                assert_eq!(
                    on_board.contains(id),
                    row.readers[at],
                    "{who}: board shows {}",
                    row.name
                );
            }
        }
        // Entry: the Workspace scope (My Work) and the Team scope gather the same Tickets the board shows, and nothing else.
        for scope in [
            "scope=WORKSPACE".to_string(),
            format!("scope=TEAM&teamId={design}"),
        ] {
            let (s, gathered) = api(
                world,
                "GET",
                cookie,
                &format!("/v1/workspaces/{ws}/work?{scope}"),
                None,
            )
            .await;
            if !active {
                assert!(!readable(s), "{who}: a removed member gets no {scope}");
                continue;
            }
            assert_eq!(s, StatusCode::OK, "{who}: {gathered}");
            let in_scope = ids_of(&gathered, "tickets");
            let scoped_projects = ids_of(&gathered, "projects");
            assert_eq!(
                scoped_projects.contains(&project.to_string()),
                [true, false, true, true, false][at],
                "{who}: {scope} lists the Project only for its participants"
            );
            for (id, row) in &tickets {
                assert_eq!(
                    in_scope.contains(id),
                    row.readers[at],
                    "{who}: {scope} gathers {}",
                    row.name
                );
            }
        }
        // Entry: Search, for every kind, must agree with the direct reads above.
        if let Some(sw) = &search {
            let (s, found) = find(sw, cookie, ws, word, "all").await;
            if !active {
                assert!(!s.is_success(), "{who}: a removed member cannot search");
                continue;
            }
            assert_eq!(s, StatusCode::OK, "{who}: {found}");
            let hits: std::collections::BTreeSet<String> = found["results"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|r| r["id"].as_str().map(str::to_owned))
                .collect();
            for (message, (_, row)) in message_ids.iter().zip(&channels) {
                assert_eq!(
                    hits.contains(message),
                    row.readers[at],
                    "{who}: Search finds the message of {}",
                    row.name
                );
            }
            for (doc, row) in &docs {
                assert_eq!(
                    hits.contains(doc["id"].as_str().unwrap()),
                    row.readers[at],
                    "{who}: Search finds {}",
                    row.name
                );
            }
            for (id, row) in &tickets {
                assert_eq!(
                    hits.contains(id),
                    row.readers[at],
                    "{who}: Search finds {}",
                    row.name
                );
            }
        }
    }
    let _ = plain_id;
}

/// "Takes part in a Project" has two implementations (the SQL function every list and Share Scope uses, and
/// the role lookup every Project route uses). A person who takes part only through a group grant must get
/// the same answer from both: the Space flag, the Project's routes, a page shared with the Project and the
/// Project's channel.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn participation_through_a_group_grant_is_the_same_answer_everywhere(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("group-{}", Uuid::new_v4())).await;
    let project = project_with_teams(world, ws, "Grouped", &[]).await;
    let (member_id, member_cookie) = member(world, ws, "in-group").await;
    let (_, other) = member(world, ws, "not-in-group").await;
    let group = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.groups(id, workspace_id, name) VALUES($1,$2,'Grouped people')")
        .bind(group)
        .bind(ws)
        .execute(&world.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.group_members(group_id, user_id) VALUES($1,$2)")
        .bind(group)
        .bind(member_id)
        .execute(&world.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.project_group_grants(project_id, group_id, role) VALUES($1,$2,'VIEWER')")
        .bind(project)
        .bind(group)
        .execute(&world.pool)
        .await
        .unwrap();
    let page = make_doc(
        world,
        &world.owner_cookie,
        ws,
        "Grouped page",
        json!({"projectId": project, "visibility":"PROJECT", "scopeProjectId": project}),
    )
    .await;
    let channel_id: Uuid = sqlx::query_scalar("SELECT id FROM penfii.channels WHERE project_id=$1")
        .bind(project)
        .fetch_one(&world.pool)
        .await
        .unwrap();
    for (who, cookie, takes_part) in [
        ("group member", &member_cookie, true),
        ("other member", &other, false),
    ] {
        let (_, space) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/space"),
            None,
        )
        .await;
        let flag = space["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == json!(project))
            .map(|p| p["participating"].as_bool().unwrap());
        assert_eq!(flag, Some(takes_part), "{who}: Space flag");
        assert_eq!(
            ids_of(&space, "channels").contains(&channel_id.to_string()),
            takes_part,
            "{who}: Project channel in the list"
        );
        for path in [
            format!("/v1/projects/{project}/board"),
            format!("/v1/docs/{}", page["id"].as_str().unwrap()),
            format!("/v1/conversations/{channel_id}/messages"),
        ] {
            assert_eq!(
                readable(status_of(world, cookie, "GET", &path, None).await),
                takes_part,
                "{who}: {path}"
            );
        }
    }
}
