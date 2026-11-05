//! Ticket order, dependencies, saved Views, Sprints and Milestones through the API.

use super::access::grant_project;
use super::tickets::{board_json, column_keys, pick, state_id};
use super::{
    World, call, get_as, grant, make_ticket, new_project, post_as, user_id_of, viewer_cookie,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn shared_order_survives_moves_status_changes_and_crowding(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Order").await;
    let todo = state_id(&workflow, "NOT_STARTED");
    let doing = state_id(&workflow, "IN_PROGRESS");
    let mut ids = Vec::new();
    let mut keys = Vec::new();
    for n in 0..4 {
        let (status, ticket) = make_ticket(
            world,
            &project_id,
            &format!("o-{n}"),
            json!({"title":format!("T{n}")}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{ticket}");
        ids.push(ticket["id"].as_str().unwrap().to_owned());
        keys.push(ticket["key"].as_str().unwrap().to_owned());
    }
    // New Tickets join the end of the column.
    assert_eq!(
        column_keys(&board_json(world, &project_id).await, &todo),
        keys
    );

    let mv = |ticket: String, key: &str, body: Value| {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let key = key.to_owned();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/tickets/{ticket}/commands/move"),
                Some(&cookie),
                Some(&key),
                Some(body),
            )
            .await
        }
    };
    // Before the first, after the last, and first/last positions.
    let (status, moved, _) = mv(ids[3].clone(), "o-m1", json!({"beforeId":ids[0]})).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(
        moved["revisionNo"], 1,
        "a pure reorder is not a Ticket edit"
    );
    assert_eq!(
        column_keys(&board_json(world, &project_id).await, &todo),
        pick(&keys, &[3, 0, 1, 2])
    );
    let (status, _, _) = mv(ids[3].clone(), "o-m2", json!({"position":"last"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        column_keys(&board_json(world, &project_id).await, &todo),
        pick(&keys, &[0, 1, 2, 3])
    );
    let (status, _, _) = mv(ids[2].clone(), "o-m3", json!({"afterId":ids[0]})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        column_keys(&board_json(world, &project_id).await, &todo),
        pick(&keys, &[0, 2, 1, 3])
    );
    // A replay of the same command changes nothing more.
    let (status, replay, _) = mv(ids[2].clone(), "o-m3", json!({"afterId":ids[0]})).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    // The same key with another body is refused.
    let (status, _, _) = mv(ids[2].clone(), "o-m3", json!({"afterId":ids[1]})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Bad shapes and stale anchors.
    let (status, _, _) = mv(
        ids[1].clone(),
        "o-bad1",
        json!({"afterId":ids[0],"position":"last"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = mv(ids[1].clone(), "o-bad2", json!({"afterId":ids[1]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, stale, _) = mv(ids[1].clone(), "o-bad3", json!({"afterId":Uuid::new_v4()})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "stale_anchor");

    // Status and place change in one command and add exactly one revision.
    let (status, across, _) = mv(
        ids[1].clone(),
        "o-x1",
        json!({"expectedRevision":1,"statusId":doing,"position":"first"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{across}");
    assert_eq!(across["statusId"], doing);
    assert_eq!(across["revisionNo"], 2);
    let board = board_json(world, &project_id).await;
    assert_eq!(column_keys(&board, &doing), pick(&keys, &[1]));
    assert_eq!(column_keys(&board, &todo), pick(&keys, &[0, 2, 3]));
    // A stale revision is refused and the order stays.
    let (status, conflict, _) = mv(
        ids[1].clone(),
        "o-x2",
        json!({"expectedRevision":1,"statusId":todo,"position":"last"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "version_conflict");
    assert_eq!(
        column_keys(&board_json(world, &project_id).await, &doing),
        pick(&keys, &[1])
    );
    // A status change without a revision is not allowed.
    let (status, _, _) = mv(ids[1].clone(), "o-x3", json!({"statusId":todo})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Crowding: repeated inserts at the same spot force a rebalance and keep the order.
    let (status, _, _) = mv(ids[3].clone(), "o-c0", json!({"afterId":ids[0]})).await;
    assert_eq!(status, StatusCode::OK);
    for n in 1..=45 {
        let (mover, anchor) = if n % 2 == 0 {
            (&ids[3], &ids[0])
        } else {
            (&ids[2], &ids[0])
        };
        let (status, body, _) =
            mv(mover.clone(), &format!("o-c{n}"), json!({"afterId":anchor})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // n = 45 is odd, so ids[2] was last placed right after ids[0], and ids[3] follows it.
    assert_eq!(
        column_keys(&board_json(world, &project_id).await, &todo),
        pick(&keys, &[0, 2, 3])
    );

    // Concurrent moves leave one valid order that contains every Ticket once.
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let target = ids[n % 4].clone();
            let anchor = ids[(n + 1) % 4].clone();
            tokio::spawn(mv(target, &format!("o-p{n}"), json!({"afterId":anchor})))
        })
        .collect();
    for handle in handles {
        let (status, body, _) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let board = board_json(world, &project_id).await;
    let mut all: Vec<String> = column_keys(&board, &todo);
    all.extend(column_keys(&board, &doing));
    all.sort();
    let mut expected = keys.clone();
    expected.sort();
    assert_eq!(all, expected);

    // A read-only member can see the order but not change it.
    let viewer = viewer_cookie(world, workspace_id, "order-viewer@penfii.test").await;
    grant_project(world, &project_id, "order-viewer@penfii.test", "VIEWER").await;
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/move", ids[0]),
        Some(&viewer),
        Some("o-v1"),
        Some(json!({"position":"last"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn dependencies_limit_completion_and_refuse_loops_and_foreign_tickets(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Deps").await;
    let (other_project_id, _) = new_project(world, workspace_id, "Deps other").await;
    let todo = state_id(&workflow, "NOT_STARTED");
    let doing = state_id(&workflow, "IN_PROGRESS");
    let done = state_id(&workflow, "DONE");
    let mut ids = Vec::new();
    for n in 0..4 {
        let (status, ticket) = make_ticket(
            world,
            &project_id,
            &format!("d-{n}"),
            json!({"title":format!("D{n}")}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{ticket}");
        ids.push(ticket["id"].as_str().unwrap().to_owned());
    }
    let (status, foreign) = make_ticket(
        world,
        &other_project_id,
        "d-foreign",
        json!({"title":"Foreign"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let add = |successor: &str, predecessor: &str, key: &str| {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let (successor, predecessor, key) =
            (successor.to_owned(), predecessor.to_owned(), key.to_owned());
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/tickets/{successor}/dependencies"),
                Some(&cookie),
                Some(&key),
                Some(json!({"predecessorId":predecessor})),
            )
            .await
        }
    };

    // D1 waits for D0.
    let (status, created, _) = add(&ids[1], &ids[0], "d-add1").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["blockedBy"][0]["ticket"]["id"], ids[0]);
    assert_eq!(created["blockedBy"][0]["satisfied"], false);
    // Same key, same request: replay, not a second edge.
    let (status, replayed, _) = add(&ids[1], &ids[0], "d-add1").await;
    assert_eq!(status, StatusCode::CREATED, "{replayed}");
    // Rejected shapes.
    let (status, own, _) = add(&ids[1], &ids[1], "d-self").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{own}");
    let (status, duplicate, _) = add(&ids[1], &ids[0], "d-dup").await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    assert_eq!(duplicate["code"], "dependency_exists");
    let (status, missing, _) = add(&ids[1], &Uuid::new_v4().to_string(), "d-missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    let (status, cross, _) = add(&ids[1], foreign["id"].as_str().unwrap(), "d-cross").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cross}");
    let (status, mutual, _) = add(&ids[0], &ids[1], "d-mutual").await;
    assert_eq!(status, StatusCode::CONFLICT, "{mutual}");
    assert_eq!(mutual["code"], "dependency_cycle");
    // A longer loop: D1 waits D0, D2 waits D1, so D0 may not wait for D2.
    let (status, _, _) = add(&ids[2], &ids[1], "d-add2").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, loop_, _) = add(&ids[0], &ids[2], "d-loop").await;
    assert_eq!(status, StatusCode::CONFLICT, "{loop_}");
    assert_eq!(loop_["code"], "dependency_cycle");

    // A Subtask cannot wait for its own parent.
    let (status, task) = make_ticket(world, &project_id, "d-task", json!({"title":"Task"})).await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let (status, sub) = make_ticket(
        world,
        &project_id,
        "d-sub",
        json!({"title":"Sub","hierarchyKind":"SUBTASK","parentId":task["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sub}");
    let (status, family, _) = add(
        sub["id"].as_str().unwrap(),
        task["id"].as_str().unwrap(),
        "d-family",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{family}");

    // The board shows the block; the blocked Ticket can start but cannot finish.
    let board = board_json(world, &project_id).await;
    let blocked = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().clone())
        .find(|t| t["id"] == ids[1])
        .unwrap();
    assert_eq!(
        blocked["dependencies"],
        json!({"openBlockers":1,"blockedBy":1,"blocks":1})
    );
    let revision = blocked["revisionNo"].as_i64().unwrap();
    // Parallel work is allowed: the successor may start while the predecessor is not done.
    let (status, started_early, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[1]),
        Some(&world.owner_cookie),
        Some("d-start"),
        Some(json!({"expectedRevision":revision,"statusId":doing})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started_early}");
    let revision = started_early["revisionNo"].as_i64().unwrap();
    // Finishing first is refused, from IN_PROGRESS by command and by drag.
    let (status, refused, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[1]),
        Some(&world.owner_cookie),
        Some("d-finish"),
        Some(json!({"expectedRevision":revision,"statusId":done})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "dependency_blocked");
    let (status, refused, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/move", ids[1]),
        Some(&world.owner_cookie),
        Some("d-drag"),
        Some(json!({"expectedRevision":revision,"statusId":done,"position":"last"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "dependency_blocked");
    let done_count = board_json(world, &project_id).await["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["stateId"] == done)
        .unwrap()["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["id"] == ids[1])
        .count();
    assert_eq!(done_count, 0);
    // Removing one that still blocks is for an admin; the owner is one.
    let (status, deps, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}/dependencies", ids[1]),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deps}");
    let viewer = viewer_cookie(world, workspace_id, "deps-viewer@penfii.test").await;
    grant_project(world, &project_id, "deps-viewer@penfii.test", "VIEWER").await;
    grant_project(
        world,
        &other_project_id,
        "deps-viewer@penfii.test",
        "VIEWER",
    )
    .await;
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/dependencies", ids[3]),
        Some(&viewer),
        Some("d-viewer"),
        Some(json!({"predecessorId":ids[0]})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    // Finishing the predecessor frees the successor.
    let (status, d0, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}", ids[0]),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, first, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[0]),
        Some(&world.owner_cookie),
        Some("d-p1"),
        Some(json!({"expectedRevision":d0["revisionNo"],"statusId":doing})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (status, second, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[0]),
        Some(&world.owner_cookie),
        Some("d-p2"),
        Some(json!({"expectedRevision":first["revisionNo"],"statusId":done})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let (status, started, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[1]),
        Some(&world.owner_cookie),
        Some("d-s2"),
        Some(json!({"expectedRevision":revision,"statusId":done})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["dependencies"]["openBlockers"], 0);

    // Delete: the edge goes, the Ticket stays.
    let edge = deps["blockedBy"][0]["id"].as_str().unwrap();
    let (status, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{edge}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{edge}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, after, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}/dependencies", ids[1]),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after["blockedBy"].as_array().unwrap().len(), 0);
    // The loop that was refused is fine now that the chain is cut.
    let (status, ok, _) = add(&ids[0], &ids[2], "d-after-cut").await;
    assert_eq!(status, StatusCode::CREATED, "{ok}");
    let _ = todo;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn private_channel_tickets_do_not_leak_through_dependencies(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Deps private").await;
    let (status, private, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"deps private","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private}");
    let (status, hidden) = make_ticket(
        world,
        &project_id,
        "dp-hidden",
        json!({"title":"Hidden","channelId":private["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hidden}");
    let (status, open) = make_ticket(world, &project_id, "dp-open", json!({"title":"Open"})).await;
    assert_eq!(status, StatusCode::CREATED, "{open}");
    // A private Ticket may not block a Ticket outside its channel.
    let (status, refused, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/dependencies", open["id"].as_str().unwrap()),
        Some(&world.owner_cookie),
        Some("dp-1"),
        Some(json!({"predecessorId":hidden["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    // The other direction is allowed: a private Ticket may wait for a shared one.
    let (status, allowed, _) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/tickets/{}/dependencies",
            hidden["id"].as_str().unwrap()
        ),
        Some(&world.owner_cookie),
        Some("dp-2"),
        Some(json!({"predecessorId":open["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{allowed}");
    // A member outside the private channel sees neither the Ticket nor the edge.
    let outsider = viewer_cookie(world, workspace_id, "deps-outsider@penfii.test").await;
    grant_project(world, &project_id, "deps-outsider@penfii.test", "VIEWER").await;
    let (status, view, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{}/dependencies", open["id"].as_str().unwrap()),
        Some(&outsider),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["blocks"].as_array().unwrap().len(), 0, "{view}");
    let (status, board, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/board"),
        Some(&outsider),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let shown = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().clone())
        .find(|t| t["id"] == open["id"])
        .unwrap();
    assert_eq!(
        shown["dependencies"],
        json!({"openBlockers":0,"blockedBy":0,"blocks":0}),
        "no trace of the hidden Ticket"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn saved_views_are_validated_scoped_and_versioned(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Views").await;
    let (other_project_id, _) = new_project(world, workspace_id, "Views other").await;
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
    let (status, foreign_label, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{other_project_id}/labels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"backend"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign_label}");
    let create = |cookie: String, project: String, key: &str, body: Value| {
        let app = world.app.clone();
        let key = key.to_owned();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/projects/{project}/views"),
                Some(&cookie),
                Some(&key),
                Some(body),
            )
            .await
        }
    };
    let definition = json!({"filter":{"labelIds":[label["id"]],"priorities":["HIGH"]},"group":"ASSIGNEE","sort":{"field":"DUE_ON","dir":"DESC"}});
    let (status, view, _) = create(
        world.owner_cookie.clone(),
        project_id.clone(),
        "v-1",
        json!({"name":"Backend","scope":"PROJECT","definition":definition}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{view}");
    assert_eq!(view["layout"], "BOARD");
    assert_eq!(view["definition"]["group"], "ASSIGNEE");
    assert_eq!(view["canEdit"], true);
    // An empty definition gets the defaults: the whole Project, by State, in shared order.
    let (status, plain, _) = create(
        world.owner_cookie.clone(),
        project_id.clone(),
        "v-2",
        json!({"name":"All work","scope":"PERSONAL"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    assert_eq!(plain["definition"]["group"], "STATUS");
    assert_eq!(
        plain["definition"]["sort"],
        json!({"field":"RANK","dir":"ASC"})
    );
    // Replay returns the same View.
    let (status, replayed, _) = create(
        world.owner_cookie.clone(),
        project_id.clone(),
        "v-2",
        json!({"name":"All work","scope":"PERSONAL"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(replayed["id"], plain["id"]);
    // Same name in the same scope is refused.
    let (status, taken, _) = create(
        world.owner_cookie.clone(),
        project_id.clone(),
        "v-3",
        json!({"name":"all work","scope":"PERSONAL"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{taken}");
    assert_eq!(taken["code"], "view_name_taken");
    // Shared Views share one namespace for the whole Project, whoever owns them.
    let (status, shared_again, _) = create(
        world.owner_cookie.clone(),
        project_id.clone(),
        "v-3b",
        json!({"name":"backend","scope":"PROJECT"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{shared_again}");
    assert_eq!(shared_again["code"], "view_name_taken");
    // Bad definitions.
    for (key, body) in [
        (
            "v-b1",
            json!({"name":"Bad group","definition":{"group":"COLOR"}}),
        ),
        (
            "v-b2",
            json!({"name":"Bad sort","definition":{"sort":{"field":"RANK","dir":"UP"}}}),
        ),
        (
            "v-b3",
            json!({"name":"Bad priority","definition":{"filter":{"priorities":["SOON"]}}}),
        ),
        (
            "v-b4",
            json!({"name":"Unknown key","definition":{"filter":{"color":"red"}}}),
        ),
        (
            "v-b5",
            json!({"name":"Foreign label","definition":{"filter":{"labelIds":[foreign_label["id"]]}}}),
        ),
        (
            "v-b6",
            json!({"name":"Missing label","definition":{"filter":{"labelIds":[Uuid::new_v4()]}}}),
        ),
        ("v-b7", json!({"name":"   ","definition":{}})),
    ] {
        let (status, value, _) =
            create(world.owner_cookie.clone(), project_id.clone(), key, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key}: {value}");
    }

    // Another member: sees the shared View, not the owner's personal one, and cannot share.
    let member = viewer_cookie(world, workspace_id, "views-member@penfii.test").await;
    grant_project(world, &project_id, "views-member@penfii.test", "VIEWER").await;
    grant_project(
        world,
        &other_project_id,
        "views-member@penfii.test",
        "VIEWER",
    )
    .await;
    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/views"),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names: Vec<_> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Backend"]);
    assert_eq!(listed[0]["canEdit"], false);
    let (status, denied, _) = create(
        member.clone(),
        project_id.clone(),
        "v-m1",
        json!({"name":"Mine shared","scope":"PROJECT"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, mine, _) = create(
        member.clone(),
        project_id.clone(),
        "v-m2",
        json!({"name":"Mine","scope":"PERSONAL"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{mine}");
    let (status, _, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{}", view["id"].as_str().unwrap()),
        Some(&member),
        None,
        Some(json!({"expectedVersion":1,"name":"Hijack"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The member's personal View is invisible to the owner, even by id.
    let (status, _, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{}", mine["id"].as_str().unwrap()),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedVersion":1,"name":"Hijack"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Updates are version-checked.
    let id = view["id"].as_str().unwrap();
    let (status, updated, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{id}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedVersion":1,"definition":{"group":"PRIORITY"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["version"], 2);
    assert_eq!(updated["definition"]["group"], "PRIORITY");
    assert_eq!(
        updated["definition"]["filter"]["labelIds"],
        json!([]),
        "the new definition replaces the old one"
    );
    let (status, stale, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{id}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedVersion":1,"name":"Late"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "version_conflict");
    // Delete needs the current version, and the View is gone afterwards.
    let (status, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/views/{id}?expectedVersion=1"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/views/{id}?expectedVersion=2"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/views"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    let names: Vec<_> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["All work"]);
    // A deleted name can be used again.
    let (status, again, _) = create(
        world.owner_cookie.clone(),
        project_id.clone(),
        "v-4",
        json!({"name":"Backend","scope":"PROJECT"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sprint_and_milestone_rules(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Plan").await;
    let (other_project_id, _) = new_project(world, workspace_id, "Plan other").await;
    let doing = state_id(&workflow, "IN_PROGRESS");
    let post = |path: String, key: &str, body: Value| {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let key = key.to_owned();
        async move { call(&app, "POST", &path, Some(&cookie), Some(&key), Some(body)).await }
    };
    let (status, epic) = make_ticket(
        world,
        &project_id,
        "p-epic",
        json!({"title":"Epic","hierarchyKind":"EPIC"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, task) = make_ticket(world, &project_id, "p-task", json!({"title":"Task"})).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, sub) = make_ticket(
        world,
        &project_id,
        "p-sub",
        json!({"title":"Sub","hierarchyKind":"SUBTASK","parentId":task["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sub}");
    let task_id = task["id"].as_str().unwrap().to_owned();

    // Only an admin plans Sprints.
    let member = viewer_cookie(world, workspace_id, "plan-member@penfii.test").await;
    grant_project(world, &project_id, "plan-member@penfii.test", "VIEWER").await;
    grant_project(
        world,
        &other_project_id,
        "plan-member@penfii.test",
        "VIEWER",
    )
    .await;
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&member),
        Some("s-m"),
        Some(json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, bad, _) = post(
        format!("/v1/projects/{project_id}/sprints"),
        "s-bad",
        json!({"name":"S","startsOn":"2026-11-13","endsOn":"2026-11-02"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let (status, one, _) = post(format!("/v1/projects/{project_id}/sprints"), "s-1", json!({"name":"Sprint 1","goal":"Ship the board","startsOn":"2026-11-02","endsOn":"2026-11-13"})).await;
    assert_eq!(status, StatusCode::CREATED, "{one}");
    assert_eq!(one["state"], "PLANNED");
    let (status, two, _) = post(
        format!("/v1/projects/{project_id}/sprints"),
        "s-2",
        json!({"name":"Sprint 2","startsOn":"2026-11-16","endsOn":"2026-11-27"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{two}");
    let (status, foreign, _) = post(
        format!("/v1/projects/{other_project_id}/sprints"),
        "s-3",
        json!({"name":"Foreign","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");

    // Assignment rules: Tasks only, same Project, one current Sprint.
    let assign = |ticket: &str, key: &str, sprint: Value| {
        post(
            format!("/v1/tickets/{ticket}/commands/assign-sprint"),
            key,
            json!({"sprintId":sprint}),
        )
    };
    let (status, refused, _) =
        assign(epic["id"].as_str().unwrap(), "a-epic", one["id"].clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, refused, _) =
        assign(sub["id"].as_str().unwrap(), "a-sub", one["id"].clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, refused, _) = assign(&task_id, "a-foreign", foreign["id"].clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, placed, _) = assign(&task_id, "a-1", one["id"].clone()).await;
    assert_eq!(status, StatusCode::OK, "{placed}");
    assert_eq!(placed["sprint"]["name"], "Sprint 1");
    assert_eq!(placed["sprint"]["inherited"], false);
    // The Subtask shows its parent's Sprint.
    let board = board_json(world, &project_id).await;
    let shown = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().clone())
        .find(|t| t["id"] == sub["id"])
        .unwrap();
    assert_eq!(shown["sprint"]["name"], "Sprint 1");
    assert_eq!(shown["sprint"]["inherited"], true);
    // Moving to another Sprint keeps one current assignment and the history.
    let (status, moved, _) = assign(&task_id, "a-2", two["id"].clone()).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["sprint"]["name"], "Sprint 2");
    let current: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.sprint_assignments WHERE ticket_id=$1 AND ended_at IS NULL",
    )
    .bind(Uuid::parse_str(&task_id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    let history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.sprint_assignments WHERE ticket_id=$1")
            .bind(Uuid::parse_str(&task_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!((current, history), (1, 2));

    // Lifecycle: PLANNED -> ACTIVE -> COMPLETED; no skipping, no repeats.
    let sprint_two = two["id"].as_str().unwrap();
    let (status, early, _) = post(
        format!("/v1/sprints/{sprint_two}/commands/complete"),
        "s-c0",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{early}");
    assert_eq!(early["code"], "invalid_sprint_state");
    let (status, started, _) = post(
        format!("/v1/sprints/{sprint_two}/commands/start"),
        "s-s1",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["state"], "ACTIVE");
    let (status, stale, _) = post(
        format!("/v1/sprints/{sprint_two}/commands/complete"),
        "s-c1",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "version_conflict");
    // A Ticket's State does not change when the Sprint starts, ends or is canceled.
    let (status, d, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{task_id}/commands/change-status"),
        Some(&world.owner_cookie),
        Some("s-d"),
        Some(json!({"expectedRevision":1,"statusId":doing})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{d}");
    let (status, completed, _) = post(
        format!("/v1/sprints/{sprint_two}/commands/complete"),
        "s-c2",
        json!({"expectedVersion":2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "COMPLETED");
    let (status, after, _) = call(
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
        after["sprint"],
        Value::Null,
        "completing ends the current assignment"
    );
    assert_eq!(after["statusId"], doing, "the Ticket keeps its State");
    let ended: String = sqlx::query_scalar(
        "SELECT end_reason FROM penfii.sprint_assignments WHERE ticket_id=$1 AND sprint_id=$2",
    )
    .bind(Uuid::parse_str(&task_id).unwrap())
    .bind(Uuid::parse_str(sprint_two).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(ended, "SPRINT_COMPLETED");
    // A finished Sprint cannot take new work.
    let (status, refused, _) = assign(&task_id, "a-3", two["id"].clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    // Cancel releases assignments too.
    let (status, back, _) = assign(&task_id, "a-4", one["id"].clone()).await;
    assert_eq!(status, StatusCode::OK, "{back}");
    let sprint_one = one["id"].as_str().unwrap();
    let (status, canceled, _) = post(
        format!("/v1/sprints/{sprint_one}/commands/cancel"),
        "s-x1",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["state"], "CANCELED");
    let (_, after, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(after["sprint"], Value::Null);

    // Milestones are separate from Sprints and from State.
    let (status, milestone, _) = post(
        format!("/v1/projects/{project_id}/milestones"),
        "m-1",
        json!({"name":"Beta","targetOn":"2026-12-01"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{milestone}");
    assert_eq!(milestone["state"], "OPEN");
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/milestones"),
        Some(&member),
        Some("m-m"),
        Some(json!({"name":"Mine"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, foreign_milestone, _) = post(
        format!("/v1/projects/{other_project_id}/milestones"),
        "m-2",
        json!({"name":"Foreign"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign_milestone}");
    let set = |ids: Value, key: &str| {
        post(
            format!("/v1/tickets/{task_id}/commands/set-milestones"),
            key,
            json!({"milestoneIds":ids}),
        )
    };
    let (status, refused, _) = set(json!([foreign_milestone["id"]]), "m-s0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, linked, _) = set(json!([milestone["id"]]), "m-s1").await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["milestones"][0]["name"], "Beta");
    // Reaching the Milestone leaves the Ticket, its State and its Sprint alone.
    let (status, _, _) = assign(&task_id, "a-5", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let milestone_id = milestone["id"].as_str().unwrap();
    let (status, reached, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/milestones/{milestone_id}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"expectedVersion":1,"state":"REACHED"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reached}");
    assert_eq!(reached["state"], "REACHED");
    assert!(reached["reachedAt"].as_str().is_some(), "{reached}");
    let (_, after, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{task_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(after["statusId"], doing);
    assert_eq!(after["milestones"][0]["state"], "REACHED");
    // A reached Milestone takes no new links, but the old link can be dropped.
    let (status, refused, _) = set(json!([milestone["id"], milestone["id"]]), "m-s2").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an existing link stays valid: {refused}"
    );
    let (status, cleared, _) = set(json!([]), "m-s3").await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["milestones"], json!([]));
    let (status, refused, _) = set(json!([milestone["id"]]), "m-s4").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, planning, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/planning"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{planning}");
    assert_eq!(planning["canManage"], true);
    assert_eq!(planning["sprints"].as_array().unwrap().len(), 2);
    assert_eq!(planning["milestones"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn read_only_member_cannot_change_any_planning_data_through_the_api(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Readonly").await;
    let doing = state_id(&workflow, "IN_PROGRESS");
    let owner = world.owner_cookie.clone();
    let post = |cookie: String, path: String, key: String, body: Value| {
        let app = world.app.clone();
        async move { call(&app, "POST", &path, Some(&cookie), Some(&key), Some(body)).await }
    };
    let (_, a) = make_ticket(world, &project_id, "ro-a", json!({"title":"A"})).await;
    let (_, b) = make_ticket(world, &project_id, "ro-b", json!({"title":"B"})).await;
    let (a_id, b_id) = (
        a["id"].as_str().unwrap().to_owned(),
        b["id"].as_str().unwrap().to_owned(),
    );
    let (s, sprint, _) = post(
        owner.clone(),
        format!("/v1/projects/{project_id}/sprints"),
        "ro-s".into(),
        json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sprint}");
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let (s, milestone, _) = post(
        owner.clone(),
        format!("/v1/projects/{project_id}/milestones"),
        "ro-m".into(),
        json!({"name":"M"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{milestone}");
    let milestone_id = milestone["id"].as_str().unwrap().to_owned();
    let (s, dep, _) = post(
        owner.clone(),
        format!("/v1/tickets/{b_id}/dependencies"),
        "ro-d".into(),
        json!({"predecessorId":a_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dep}");
    let edge_id = dep["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    let (s, view, _) = post(
        owner.clone(),
        format!("/v1/projects/{project_id}/views"),
        "ro-v".into(),
        json!({"name":"Shared","scope":"PROJECT"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{view}");
    let view_id = view["id"].as_str().unwrap().to_owned();
    let (s, _, _) = post(
        owner.clone(),
        format!("/v1/tickets/{a_id}/commands/assign-sprint"),
        "ro-as".into(),
        json!({"sprintId":sprint_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let viewer = viewer_cookie(world, workspace_id, "ro-viewer@penfii.test").await;
    grant_project(world, &project_id, "ro-viewer@penfii.test", "VIEWER").await;
    let version = workflow["version"].clone();
    let denied: Vec<(&str, StatusCode)> = vec![
        (
            "shared view create",
            post(
                viewer.clone(),
                format!("/v1/projects/{project_id}/views"),
                "x1".into(),
                json!({"name":"V","scope":"PROJECT"}),
            )
            .await
            .0,
        ),
        (
            "order",
            post(
                viewer.clone(),
                format!("/v1/tickets/{a_id}/commands/move"),
                "x2".into(),
                json!({"position":"last"}),
            )
            .await
            .0,
        ),
        (
            "move with status",
            post(
                viewer.clone(),
                format!("/v1/tickets/{a_id}/commands/move"),
                "x3".into(),
                json!({"expectedRevision":1,"statusId":doing}),
            )
            .await
            .0,
        ),
        (
            "dependency create",
            post(
                viewer.clone(),
                format!("/v1/tickets/{a_id}/dependencies"),
                "x4".into(),
                json!({"predecessorId":b_id}),
            )
            .await
            .0,
        ),
        (
            "sprint create",
            post(
                viewer.clone(),
                format!("/v1/projects/{project_id}/sprints"),
                "x5".into(),
                json!({"name":"S2","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
            )
            .await
            .0,
        ),
        (
            "sprint start",
            post(
                viewer.clone(),
                format!("/v1/sprints/{sprint_id}/commands/start"),
                "x6".into(),
                json!({"expectedVersion":1}),
            )
            .await
            .0,
        ),
        (
            "sprint complete",
            post(
                viewer.clone(),
                format!("/v1/sprints/{sprint_id}/commands/complete"),
                "x7".into(),
                json!({"expectedVersion":1}),
            )
            .await
            .0,
        ),
        (
            "sprint cancel",
            post(
                viewer.clone(),
                format!("/v1/sprints/{sprint_id}/commands/cancel"),
                "x8".into(),
                json!({"expectedVersion":1}),
            )
            .await
            .0,
        ),
        (
            "sprint assign",
            post(
                viewer.clone(),
                format!("/v1/tickets/{b_id}/commands/assign-sprint"),
                "x9".into(),
                json!({"sprintId":sprint_id}),
            )
            .await
            .0,
        ),
        (
            "sprint unassign",
            post(
                viewer.clone(),
                format!("/v1/tickets/{a_id}/commands/assign-sprint"),
                "x10".into(),
                json!({"sprintId":null}),
            )
            .await
            .0,
        ),
        (
            "milestone create",
            post(
                viewer.clone(),
                format!("/v1/projects/{project_id}/milestones"),
                "x11".into(),
                json!({"name":"M2"}),
            )
            .await
            .0,
        ),
        (
            "milestone link",
            post(
                viewer.clone(),
                format!("/v1/tickets/{a_id}/commands/set-milestones"),
                "x12".into(),
                json!({"milestoneIds":[milestone_id]}),
            )
            .await
            .0,
        ),
        (
            "sprint edit",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/sprints/{sprint_id}"),
                Some(&viewer),
                None,
                Some(json!({"expectedVersion":1,"name":"X"})),
            )
            .await
            .0,
        ),
        (
            "milestone edit",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/milestones/{milestone_id}"),
                Some(&viewer),
                None,
                Some(json!({"expectedVersion":1,"name":"X"})),
            )
            .await
            .0,
        ),
        (
            "milestone reach",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/milestones/{milestone_id}"),
                Some(&viewer),
                None,
                Some(json!({"expectedVersion":1,"state":"REACHED"})),
            )
            .await
            .0,
        ),
        (
            "milestone cancel",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/milestones/{milestone_id}"),
                Some(&viewer),
                None,
                Some(json!({"expectedVersion":1,"state":"CANCELED"})),
            )
            .await
            .0,
        ),
        (
            "wip",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/projects/{project_id}/workflow/states/{doing}"),
                Some(&viewer),
                None,
                Some(json!({"expectedWorkflowVersion":version,"wipLimit":2})),
            )
            .await
            .0,
        ),
        (
            "dependency delete",
            call(
                &world.app,
                "DELETE",
                &format!("/v1/dependencies/{edge_id}"),
                Some(&viewer),
                None,
                None,
            )
            .await
            .0,
        ),
        (
            "shared view edit",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/views/{view_id}"),
                Some(&viewer),
                None,
                Some(json!({"expectedVersion":1,"name":"X"})),
            )
            .await
            .0,
        ),
        (
            "shared view delete",
            call(
                &world.app,
                "DELETE",
                &format!("/v1/views/{view_id}?expectedVersion=1"),
                Some(&viewer),
                None,
                None,
            )
            .await
            .0,
        ),
    ];
    for (what, status) in denied {
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{what} must be refused for a read-only member"
        );
    }
    // Nothing changed.
    let (_, deps, _) = call(
        &world.app,
        "GET",
        &format!("/v1/tickets/{b_id}/dependencies"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(deps["blockedBy"].as_array().unwrap().len(), 1);
    let (_, planning, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/planning"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(planning["sprints"].as_array().unwrap().len(), 1);
    assert_eq!(planning["sprints"][0]["state"], "PLANNED");
    assert_eq!(planning["milestones"].as_array().unwrap().len(), 1);
    assert_eq!(planning["milestones"][0]["state"], "OPEN");
    // The read-only member can still read everything.
    for path in [
        format!("/v1/projects/{project_id}/board"),
        format!("/v1/projects/{project_id}/planning"),
        format!("/v1/projects/{project_id}/views"),
        format!("/v1/tickets/{b_id}/dependencies"),
    ] {
        let (status, _, _) = call(&world.app, "GET", &path, Some(&viewer), None, None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
    // A personal View is the member's own setting, not Project data; it stays theirs alone.
    let (s, mine, _) = post(
        viewer.clone(),
        format!("/v1/projects/{project_id}/views"),
        "x20".into(),
        json!({"name":"Mine","scope":"PERSONAL"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{mine}");
    // A member of another Workspace-less outsider (no session) gets nothing.
    let (status, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{a_id}/commands/move"),
        None,
        Some("x21"),
        Some(json!({"position":"last"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn editor_edits_tickets_but_only_admin_manages_planning(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Editor").await;
    let doing = state_id(&workflow, "IN_PROGRESS");
    let owner = world.owner_cookie.clone();
    let post = |cookie: String, path: String, key: String, body: Value| {
        let app = world.app.clone();
        async move { call(&app, "POST", &path, Some(&cookie), Some(&key), Some(body)).await }
    };
    let (_, a) = make_ticket(world, &project_id, "ed-a", json!({"title":"A"})).await;
    let (_, b) = make_ticket(world, &project_id, "ed-b", json!({"title":"B"})).await;
    let (a_id, b_id) = (
        a["id"].as_str().unwrap().to_owned(),
        b["id"].as_str().unwrap().to_owned(),
    );
    let (_, sprint, _) = post(
        owner.clone(),
        format!("/v1/projects/{project_id}/sprints"),
        "ed-s".into(),
        json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let (_, milestone, _) = post(
        owner.clone(),
        format!("/v1/projects/{project_id}/milestones"),
        "ed-m".into(),
        json!({"name":"M"}),
    )
    .await;
    let milestone_id = milestone["id"].as_str().unwrap().to_owned();
    let (_, dep, _) = post(
        owner.clone(),
        format!("/v1/tickets/{b_id}/dependencies"),
        "ed-d".into(),
        json!({"predecessorId":a_id}),
    )
    .await;
    let edge_id = dep["blockedBy"][0]["id"].as_str().unwrap().to_owned();

    let editor = viewer_cookie(world, workspace_id, "ed-editor@penfii.test").await;
    let user: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email='ed-editor@penfii.test'")
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

    // What an editor may do.
    let (s, body, _) = post(
        editor.clone(),
        format!("/v1/tickets/{a_id}/commands/move"),
        "e1".into(),
        json!({"position":"last"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body, _) = post(
        editor.clone(),
        format!("/v1/tickets/{a_id}/commands/assign-sprint"),
        "e2".into(),
        json!({"sprintId":sprint_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body, _) = post(
        editor.clone(),
        format!("/v1/tickets/{a_id}/commands/set-milestones"),
        "e3".into(),
        json!({"milestoneIds":[milestone_id]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body, _) = post(
        editor.clone(),
        format!("/v1/projects/{project_id}/views"),
        "e4".into(),
        json!({"name":"Mine","scope":"PERSONAL"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    // What stays with a project admin.
    let denied: Vec<(&str, StatusCode)> = vec![
        (
            "shared view",
            post(
                editor.clone(),
                format!("/v1/projects/{project_id}/views"),
                "e5".into(),
                json!({"name":"Shared","scope":"PROJECT"}),
            )
            .await
            .0,
        ),
        (
            "sprint create",
            post(
                editor.clone(),
                format!("/v1/projects/{project_id}/sprints"),
                "e6".into(),
                json!({"name":"S2","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
            )
            .await
            .0,
        ),
        (
            "sprint start",
            post(
                editor.clone(),
                format!("/v1/sprints/{sprint_id}/commands/start"),
                "e7".into(),
                json!({"expectedVersion":1}),
            )
            .await
            .0,
        ),
        (
            "milestone create",
            post(
                editor.clone(),
                format!("/v1/projects/{project_id}/milestones"),
                "e8".into(),
                json!({"name":"M2"}),
            )
            .await
            .0,
        ),
        (
            "milestone reach",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/milestones/{milestone_id}"),
                Some(&editor),
                None,
                Some(json!({"expectedVersion":1,"state":"REACHED"})),
            )
            .await
            .0,
        ),
        (
            "wip",
            call(
                &world.app,
                "PATCH",
                &format!("/v1/projects/{project_id}/workflow/states/{doing}"),
                Some(&editor),
                None,
                Some(json!({"expectedWorkflowVersion":workflow["version"],"wipLimit":2})),
            )
            .await
            .0,
        ),
        (
            "blocking dependency delete",
            call(
                &world.app,
                "DELETE",
                &format!("/v1/dependencies/{edge_id}"),
                Some(&editor),
                None,
                None,
            )
            .await
            .0,
        ),
    ];
    for (what, status) in denied {
        assert_eq!(status, StatusCode::FORBIDDEN, "{what} is admin-only");
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn view_names_are_unique_per_project_scope_and_owner_even_under_races(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (p1, _) = new_project(world, workspace_id, "Names one").await;
    let (p2, _) = new_project(world, workspace_id, "Names two").await;
    let create =
        |cookie: String, project: String, key: String, name: String, scope: &'static str| {
            let app = world.app.clone();
            async move {
                call(
                    &app,
                    "POST",
                    &format!("/v1/projects/{project}/views"),
                    Some(&cookie),
                    Some(&key),
                    Some(json!({"name":name,"scope":scope})),
                )
                .await
            }
        };
    let owner = world.owner_cookie.clone();
    let (s, first, _) = create(
        owner.clone(),
        p1.clone(),
        "n1".into(),
        "Roadmap".into(),
        "PROJECT",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    for (key, name) in [
        ("n2", "Roadmap"),
        ("n3", "roadmap"),
        ("n4", "ROADMAP"),
        ("n5", "  Roadmap  "),
    ] {
        let (s, body, _) = create(
            owner.clone(),
            p1.clone(),
            key.into(),
            name.into(),
            "PROJECT",
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{name}: {body}");
        assert_eq!(body["code"], "view_name_taken");
    }
    // Another Project may use the name.
    let (s, body, _) = create(
        owner.clone(),
        p2.clone(),
        "n6".into(),
        "Roadmap".into(),
        "PROJECT",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    // A personal View may share the name of a shared one; two members' personal Views do not collide.
    let (s, mine, _) = create(
        owner.clone(),
        p1.clone(),
        "n7".into(),
        "Roadmap".into(),
        "PERSONAL",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{mine}");
    let member = viewer_cookie(world, workspace_id, "names-member@penfii.test").await;
    grant_project(world, &p1, "names-member@penfii.test", "VIEWER").await;
    grant_project(world, &p2, "names-member@penfii.test", "VIEWER").await;
    let (s, body, _) = create(
        member.clone(),
        p1.clone(),
        "n8".into(),
        "Roadmap".into(),
        "PERSONAL",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (s, body, _) = create(
        member.clone(),
        p1.clone(),
        "n9".into(),
        "roadmap".into(),
        "PERSONAL",
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");

    // Renaming follows the same rule.
    let (_, other, _) = create(
        owner.clone(),
        p1.clone(),
        "n10".into(),
        "Backlog".into(),
        "PROJECT",
    )
    .await;
    let other_id = other["id"].as_str().unwrap().to_owned();
    let (s, body, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{other_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"name":"ROADMAP"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "view_name_taken");
    let (s, body, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{other_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"name":"Backlog 2"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    // Renaming to its own name in another case is fine.
    let (s, body, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{other_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":2,"name":"backlog 2"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    // Delete frees the name.
    let first_id = first["id"].as_str().unwrap();
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/views/{first_id}?expectedVersion=1"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, body, _) = create(
        owner.clone(),
        p1.clone(),
        "n11".into(),
        "Roadmap".into(),
        "PROJECT",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    // Twelve creators race for one new name: exactly one wins, and the table holds one row.
    let handles: Vec<_> = (0..12)
        .map(|n| {
            tokio::spawn(create(
                owner.clone(),
                p1.clone(),
                format!("race-{n}"),
                "Race".into(),
                "PROJECT",
            ))
        })
        .collect();
    let mut created = 0;
    for handle in handles {
        let (s, body, _) = handle.await.unwrap();
        match s {
            StatusCode::CREATED => created += 1,
            StatusCode::CONFLICT => assert_eq!(body["code"], "view_name_taken", "{body}"),
            other => panic!("unexpected {other}: {body}"),
        }
    }
    assert_eq!(created, 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.project_views WHERE project_id=$1 AND lower(name)='race' AND deleted_at IS NULL").bind(Uuid::parse_str(&p1).unwrap()).fetch_one(&world.pool).await.unwrap();
    assert_eq!(rows, 1);
    // The constraint itself, bypassing the API, also refuses a duplicate.
    let dup = sqlx::query("INSERT INTO penfii.project_views(id,workspace_id,project_id,owner_id,scope,name,layout,definition) VALUES($1,$2,$3,$4,'PROJECT','RACE','BOARD','{}')")
        .bind(Uuid::new_v4()).bind(workspace_id).bind(Uuid::parse_str(&p1).unwrap()).bind(world.owner_id).execute(&world.pool).await;
    assert!(dup.is_err(), "the unique index is the last line of defence");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn dependency_completion_rule_follows_categories_not_names(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Dep categories").await;
    let version = workflow["version"].as_i64().unwrap();
    // A second DONE-category State with an unrelated name, and an extra IN_PROGRESS one.
    let (s, shipped, _) = call(&world.app, "POST", &format!("/v1/projects/{project_id}/workflow/states"), Some(&world.owner_cookie), None, Some(json!({"expectedWorkflowVersion":version,"name":"Shipped","category":"DONE","position":90}))).await;
    assert_eq!(s, StatusCode::CREATED, "{shipped}");
    let shipped = shipped["state"]["id"].as_str().unwrap().to_owned();
    let (_, workflow, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    let todo = state_id(&workflow, "NOT_STARTED");
    let doing = state_id(&workflow, "IN_PROGRESS");
    let done = state_id(&workflow, "DONE");
    let canceled = state_id(&workflow, "CANCELED");
    let mut ids = Vec::new();
    for n in 0..2 {
        let (s, t) = make_ticket(
            world,
            &project_id,
            &format!("depcat-{n}"),
            json!({"title":format!("dc {n}"),"hierarchyKind":"TASK"}),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    let (s, dep, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/dependencies", ids[1]),
        Some(&world.owner_cookie),
        Some("depcat-dep"),
        Some(json!({"predecessorId":ids[0]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dep}");
    let go = |target: String, key: &'static str, rev: i64| {
        let app = world.app.clone();
        let cookie = world.owner_cookie.clone();
        let path = format!("/v1/tickets/{}/commands/change-status", ids[1]);
        async move {
            call(
                &app,
                "POST",
                &path,
                Some(&cookie),
                Some(key),
                Some(json!({"expectedRevision":rev,"statusId":target})),
            )
            .await
        }
    };
    // Predecessor in Todo: successor may start; any DONE-category State is refused, even directly.
    let (s, r, _) = go(shipped.clone(), "depgo-1", 1).await;
    assert_eq!(
        (s, r["code"].as_str()),
        (StatusCode::CONFLICT, Some("dependency_blocked")),
        "{r}"
    );
    let (s, r, _) = go(doing.clone(), "depgo-2", 1).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let rev = r["revisionNo"].as_i64().unwrap();
    // Back to Todo is also free.
    let (s, r, _) = go(todo.clone(), "depgo-3", rev).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let rev = r["revisionNo"].as_i64().unwrap();
    let (s, r, _) = go(doing.clone(), "depgo-4", rev).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let rev = r["revisionNo"].as_i64().unwrap();
    // Predecessor IN_PROGRESS: still refused from IN_PROGRESS, for either DONE State.
    let (s, p, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[0]),
        Some(&world.owner_cookie),
        Some("depcat-p1"),
        Some(json!({"expectedRevision":1,"statusId":doing})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    for (key, target) in [("depgo-5", done.clone()), ("depgo-6", shipped.clone())] {
        let (s, r, _) = go(target, key, rev).await;
        assert_eq!(
            (s, r["code"].as_str()),
            (StatusCode::CONFLICT, Some("dependency_blocked")),
            "{r}"
        );
    }
    // Cancelling is not completion; the provisional behavior stays (allowed).
    let (s, r, _) = go(canceled, "depgo-7", rev).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let rev = r["revisionNo"].as_i64().unwrap();
    // Predecessor DONE: the successor may complete into any DONE-category State.
    let (s, p2, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/change-status", ids[0]),
        Some(&world.owner_cookie),
        Some("depcat-p2"),
        Some(json!({"expectedRevision":p["revisionNo"],"statusId":done})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p2}");
    let (s, r, _) = go(shipped, "depgo-8", rev).await;
    assert_eq!(s, StatusCode::OK, "{r}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn move_to_top_is_persistent_order_and_leaves_a_time_only_for_first(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Top").await;
    let mut ids = Vec::new();
    for n in 0..3 {
        let (s, t) = make_ticket(
            world,
            &project_id,
            &format!("top-{n}"),
            json!({"title":format!("top {n}"),"hierarchyKind":"TASK"}),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    let stamp = |board: &Value, id: &str| -> Value {
        board["columns"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["tickets"].as_array().unwrap().clone())
            .find(|t| t["id"] == id)
            .unwrap()["movedToTopAt"]
            .clone()
    };
    let before = board_json(world, &project_id).await;
    assert!(ids.iter().all(|id| stamp(&before, id).is_null()));
    // A neighbour-based reorder changes the shared order only.
    let (s, r, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/move", ids[2]),
        Some(&world.owner_cookie),
        Some("top-m1"),
        Some(json!({"beforeId":ids[1]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(stamp(&board_json(world, &project_id).await, &ids[2]).is_null());
    // "Move to top" is the same order change plus a time.
    let (s, r, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{}/commands/move", ids[1]),
        Some(&world.owner_cookie),
        Some("top-m2"),
        Some(json!({"position":"first"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let after = board_json(world, &project_id).await;
    assert!(!stamp(&after, &ids[1]).is_null());
    assert_eq!(
        keys_of_first_column(&after),
        vec![ids[1].clone(), ids[0].clone(), ids[2].clone()]
    );
    // It adds no Ticket revision.
    let revision = after["columns"][0]["tickets"][0]["revisionNo"]
        .as_i64()
        .unwrap();
    assert_eq!(revision, 1);
}

fn keys_of_first_column(board: &Value) -> Vec<String> {
    board["columns"][0]["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn views_have_layouts_scopes_and_a_second_definition_schema(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "View layouts").await;
    let owner = world.owner_cookie.clone();

    // The same Tickets can be shown as a Board, a List or a Table; the layout is just a field.
    for (n, layout) in ["BOARD", "LIST", "TABLE"].into_iter().enumerate() {
        let (status, view) = post_as(world, &owner, &format!("/v1/projects/{project_id}/views"), &format!("p6v6-{n}"),
            json!({"name":format!("As {layout}"),"layout":layout,"definition":{"group":"CATEGORY","display":{"compact":true,"columns":["KEY","TITLE","STATE"]}}})).await;
        assert_eq!(status, StatusCode::CREATED, "{view}");
        assert_eq!(view["layout"], layout);
        assert_eq!(view["targetKind"], "PROJECT");
        assert_eq!(view["definition"]["display"]["compact"], true);
        assert_eq!(view["definition"]["display"]["subtasks"], true);
    }
    let (status, bad) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/views"),
        "p6v6-bad-layout",
        json!({"name":"x","layout":"GANTT"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let (status, bad) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/views"),
        "p6v6-bad-col",
        json!({"name":"y","layout":"TABLE","definition":{"display":{"columns":["NOPE"]}}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    // A Project View cannot group by Project or filter by Project.
    let (status, bad) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/views"),
        "p6v6-bad-grp",
        json!({"name":"z","definition":{"group":"PROJECT"}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    // A version 1 definition (no display) still loads, with defaults.
    let (status, old) = post_as(world, &owner, &format!("/v1/projects/{project_id}/views"), "p6v6-v1", json!({"name":"Old shape","definition":{"filter":{},"group":"STATUS","sort":{"field":"RANK","dir":"ASC"}}})).await;
    assert_eq!(status, StatusCode::CREATED, "{old}");
    assert_eq!(old["definition"]["display"]["columns"], json!([]));

    // The layout of a saved View changes without touching the definition.
    let view_id = old["id"].as_str().unwrap();
    let (status, changed, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{view_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"layout":"TABLE"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["layout"], "TABLE");

    // Workspace scope: personal, List/Table only, no Project-bound ids, no State group.
    let wide = format!("/v1/workspaces/{workspace_id}/views");
    let (status, mine) = post_as(world, &owner, &wide, "p6v6-w1", json!({"targetKind":"WORKSPACE","name":"My work","layout":"LIST","definition":{"group":"CATEGORY","filter":{"categories":["IN_PROGRESS"]}}})).await;
    assert_eq!(status, StatusCode::CREATED, "{mine}");
    assert_eq!(mine["targetKind"], "WORKSPACE");
    assert!(mine["projectId"].is_null());
    let (status, bad) = post_as(
        world,
        &owner,
        &wide,
        "p6v6-w2",
        json!({"targetKind":"WORKSPACE","name":"Board","layout":"BOARD"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "board is one Project only: {bad}"
    );
    let (status, bad) = post_as(world, &owner, &wide, "p6v6-w3", json!({"targetKind":"WORKSPACE","name":"State","layout":"LIST","definition":{"group":"STATUS"}})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let (status, bad) = post_as(world, &owner, &wide, "p6v6-w4", json!({"targetKind":"WORKSPACE","name":"Label","layout":"LIST","definition":{"filter":{"labelIds":[Uuid::new_v4()]}}})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let (status, dup) = post_as(
        world,
        &owner,
        &wide,
        "p6v6-w5",
        json!({"targetKind":"WORKSPACE","name":"my WORK","layout":"TABLE"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{dup}");
    assert_eq!(dup["code"], "view_name_taken");

    // Team scope needs an existing Team; Views of other people stay private.
    let (status, team, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/teams"),
        Some(&owner),
        None,
        Some(json!({"name":"View team","members":[{"userId":world.owner_id,"role":"MANAGER"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().unwrap().to_owned();
    let (status, tv) = post_as(
        world,
        &owner,
        &wide,
        "p6v6-t1",
        json!({"targetKind":"TEAM","teamId":team_id,"name":"Team table","layout":"TABLE"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tv}");
    let (status, listed) = get_as(
        world,
        &owner,
        &format!("{wide}?targetKind=TEAM&teamId={team_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (status, none) = post_as(
        world,
        &owner,
        &wide,
        "p6v6-t2",
        json!({"targetKind":"TEAM","teamId":Uuid::new_v4(),"name":"Ghost","layout":"LIST"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{none}");
    let other = viewer_cookie(world, workspace_id, "p6v6-other@penfii.test").await;
    let (status, theirs) = get_as(world, &other, &format!("{wide}?targetKind=WORKSPACE")).await;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(
        theirs.as_array().unwrap().len(),
        0,
        "another person's View is not listed"
    );
    let mine_id = mine["id"].as_str().unwrap();
    let (status, _, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/views/{mine_id}"),
        Some(&other),
        None,
        Some(json!({"expectedVersion":1,"name":"Stolen"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn canceled_predecessor_blocks_until_the_relation_is_removed_or_waived(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Cancel rules").await;
    let done = state_id(&workflow, "DONE");
    let canceled = state_id(&workflow, "CANCELED");
    let doing = state_id(&workflow, "IN_PROGRESS");
    let owner = world.owner_cookie.clone();
    let mut ids = Vec::new();
    for n in 0..5 {
        let (status, t) = make_ticket(
            world,
            &project_id,
            &format!("p6cr-{n}"),
            json!({"title":format!("C{n}")}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{t}");
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    let editor = viewer_cookie(world, workspace_id, "p6cr-editor@penfii.test").await;
    let editor_id = user_id_of(world, "p6cr-editor@penfii.test").await;
    grant(world, &project_id, editor_id, "EDITOR").await;
    let status_of = |cookie: String, ticket: String, state: String, rev: i64, key: String| {
        let app = world.app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/tickets/{ticket}/commands/change-status"),
                Some(&cookie),
                Some(&key),
                Some(json!({"expectedRevision":rev,"statusId":state})),
            )
            .await
        }
    };

    // C1 waits for C0; C2 waits for C3.
    let (s, dep) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[1]),
        "p6cr-d1",
        json!({"predecessorId":ids[0]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dep}");
    let edge = dep["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(dep["blockedBy"][0]["kind"], "BLOCKS");
    assert_eq!(dep["blockedBy"][0]["predecessorCanceled"], false);
    // C0 is canceled. That is not completion: C1 still cannot be DONE, and the reason says so.
    let (s, body, _) = status_of(
        owner.clone(),
        ids[0].clone(),
        canceled.clone(),
        1,
        "p6cr-c0".into(),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, blocked, _) = status_of(
        owner.clone(),
        ids[1].clone(),
        done.clone(),
        1,
        "p6cr-b1".into(),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{blocked}");
    assert_eq!(blocked["code"], "dependency_blocked");
    assert_eq!(blocked["openBlockers"], 1);
    assert_eq!(blocked["canceledBlockers"], 1);
    let (_, deps) = get_as(
        world,
        &editor,
        &format!("/v1/tickets/{}/dependencies", ids[1]),
    )
    .await;
    assert_eq!(deps["blockedBy"][0]["predecessorCanceled"], true);
    assert_eq!(deps["blockedBy"][0]["satisfied"], false);
    assert_eq!(deps["canWaive"], false, "an editor cannot waive");
    // Starting is not blocked, and a successor can always be canceled.
    let (s, body, _) = status_of(
        owner.clone(),
        ids[1].clone(),
        doing.clone(),
        1,
        "p6cr-s1".into(),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body, _) = status_of(
        owner.clone(),
        ids[1].clone(),
        canceled.clone(),
        2,
        "p6cr-x1".into(),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "canceling the successor is never blocked: {body}"
    );
    let (s, body, _) = status_of(
        owner.clone(),
        ids[1].clone(),
        doing.clone(),
        3,
        "p6cr-s1b".into(),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    // Waiver: an editor may not; an admin needs a reason; the relation stays but stops blocking.
    let (s, denied) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6cr-w0",
        json!({"reason":"not mine to decide"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    let (s, empty) = post_as(
        world,
        &owner,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6cr-w1",
        json!({"reason":"  "}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{empty}");
    let (s, waived) = post_as(
        world,
        &owner,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6cr-w2",
        json!({"reason":"Replaced by C4"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{waived}");
    assert_eq!(waived["blockedBy"][0]["waived"], true);
    assert_eq!(waived["blockedBy"][0]["waiveReason"], "Replaced by C4");
    assert_eq!(waived["blockedBy"][0]["satisfied"], true);
    let (s, again) = post_as(
        world,
        &owner,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6cr-w3",
        json!({"reason":"again"}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{again}");
    // Same key, same request: a replay, not a second history entry.
    let (s, replay) = post_as(
        world,
        &owner,
        &format!("/v1/dependencies/{edge}/commands/waive"),
        "p6cr-w2",
        json!({"reason":"Replaced by C4"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{replay}");
    let history: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.dependency_waiver_history WHERE dependency_id=$1",
    )
    .bind(Uuid::parse_str(&edge).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(history, 1);
    let (_, board_ticket) = get_as(world, &owner, &format!("/v1/tickets/{}", ids[1])).await;
    assert_eq!(board_ticket["dependencies"]["openBlockers"], 0);
    let rev = board_ticket["revisionNo"].as_i64().unwrap();
    let (s, body, _) = status_of(
        editor.clone(),
        ids[1].clone(),
        done.clone(),
        rev,
        "p6cr-d1done".into(),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "waived: the successor can be completed: {body}"
    );
    // Restore needs the same permission and a reason; the history keeps both acts.
    let (s, _) = post_as(
        world,
        &editor,
        &format!("/v1/dependencies/{edge}/commands/restore"),
        "p6cr-r0",
        json!({"reason":"x"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, restored) = post_as(
        world,
        &owner,
        &format!("/v1/dependencies/{edge}/commands/restore"),
        "p6cr-r1",
        json!({"reason":"C0 is back"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(restored["blockedBy"][0]["waived"], false);
    let history: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.dependency_waiver_history WHERE dependency_id=$1",
    )
    .bind(Uuid::parse_str(&edge).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(history, 2);

    // Removing: while the predecessor is open an editor may not (even if waived); once it is
    // canceled the successor's editor may, and the removal is logged.
    let (s, dep2) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[3]),
        "p6cr-d2",
        json!({"predecessorId":ids[2]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dep2}");
    let open_edge = dep2["blockedBy"][0]["id"].as_str().unwrap().to_owned();
    let (s, refused, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{open_edge}"),
        Some(&editor),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{refused}");
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
        "the predecessor is canceled, so the successor's editor can remove it"
    );
    let logged: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.activity_entries WHERE ticket_id=$1 AND activity_kind='ticket.dependency_removed' AND actor_user_id=$2 AND data->>'predecessorCategory'='CANCELED'")
        .bind(Uuid::parse_str(&ids[1]).unwrap()).bind(editor_id).fetch_one(&world.pool).await.unwrap();
    assert_eq!(logged, 1);
    // A viewer cannot remove anything.
    let viewer = viewer_cookie(world, workspace_id, "p6cr-viewer@penfii.test").await;
    grant_project(world, &project_id, "p6cr-viewer@penfii.test", "VIEWER").await;
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{open_edge}"),
        Some(&viewer),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // RELATED: no direction, no block, no loops rule, one per pair, either side's editor.
    let (s, rel) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[4]),
        "p6cr-rel",
        json!({"predecessorId":ids[2],"kind":"RELATED"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{rel}");
    assert_eq!(rel["related"][0]["kind"], "RELATED");
    assert_eq!(rel["related"][0]["satisfied"], true);
    assert_eq!(rel["blockedBy"].as_array().unwrap().len(), 0);
    let (s, dup) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[2]),
        "p6cr-rel2",
        json!({"predecessorId":ids[4],"kind":"RELATED"}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "A-B and B-A are the same relation: {dup}"
    );
    let (s, own) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[4]),
        "p6cr-rel3",
        json!({"predecessorId":ids[4],"kind":"RELATED"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{own}");
    let (s, kind) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[4]),
        "p6cr-rel4",
        json!({"predecessorId":ids[0],"kind":"MAYBE"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{kind}");
    // A related Ticket never blocks completion.
    let (s, body, _) = status_of(
        owner.clone(),
        ids[4].clone(),
        done.clone(),
        1,
        "p6cr-rel-done".into(),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, other_side) = get_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[2]),
    )
    .await;
    assert_eq!(
        other_side["related"][0]["ticket"]["id"], ids[4],
        "both sides list it"
    );
    let related_id = rel["related"][0]["id"].as_str().unwrap().to_owned();
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{related_id}"),
        Some(&editor),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // Cross-Project RELATED links work; BLOCKS stays inside one Project.
    let (other_project, _) = new_project(world, workspace_id, "Cancel rules other").await;
    let (_, foreign) =
        make_ticket(world, &other_project, "p6cr-f", json!({"title":"Foreign"})).await;
    let (s, cross) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[0]),
        "p6cr-x1r",
        json!({"predecessorId":foreign["id"],"kind":"RELATED"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{cross}");
    let (s, cross) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{}/dependencies", ids[0]),
        "p6cr-x2r",
        json!({"predecessorId":foreign["id"]}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{cross}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sprint_close_is_one_decision_in_one_transaction_and_the_result_is_recorded(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Close sprint").await;
    let done = state_id(&workflow, "DONE");
    let canceled = state_id(&workflow, "CANCELED");
    let doing = state_id(&workflow, "IN_PROGRESS");
    let owner = world.owner_cookie.clone();
    let ticket = |n: i32| {
        let project_id = project_id.clone();
        async move {
            let (s, t) = make_ticket(
                world,
                &project_id,
                &format!("p6cl-t{n}"),
                json!({"title":format!("Sprint task {n}")}),
            )
            .await;
            assert_eq!(s, StatusCode::CREATED, "{t}");
            t["id"].as_str().unwrap().to_owned()
        }
    };
    let (t1, t2, t3, t4, t6) = (
        ticket(1).await,
        ticket(2).await,
        ticket(3).await,
        ticket(4).await,
        ticket(6).await,
    );
    let (s, sub) = make_ticket(
        world,
        &project_id,
        "p6cl-sub",
        json!({"title":"Sub of two","hierarchyKind":"SUBTASK","parentId":t2}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sub}");
    // T4 lives in a private channel the second admin is not part of.
    let (s, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"close secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{channel}");
    let (s, secret) = make_ticket(
        world,
        &project_id,
        "p6cl-secret",
        json!({"title":"Secret sprint task","channelId":channel["id"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{secret}");
    let secret_id = secret["id"].as_str().unwrap().to_owned();
    let admin2 = viewer_cookie(world, workspace_id, "close-admin@penfii.test").await;
    let admin2_id = user_id_of(world, "close-admin@penfii.test").await;
    grant(world, &project_id, admin2_id, "ADMIN").await;
    let editor = viewer_cookie(world, workspace_id, "close-editor@penfii.test").await;
    let editor_id = user_id_of(world, "close-editor@penfii.test").await;
    grant(world, &project_id, editor_id, "EDITOR").await;

    let mk = |name: &str, start: &str, end: &str, key: &str| {
        let (name, start, end, key) = (
            name.to_owned(),
            start.to_owned(),
            end.to_owned(),
            key.to_owned(),
        );
        let app = world.app.clone();
        let cookie = owner.clone();
        let project_id = project_id.clone();
        async move {
            let (s, v, _) = call(
                &app,
                "POST",
                &format!("/v1/projects/{project_id}/sprints"),
                Some(&cookie),
                Some(&key),
                Some(json!({"name":name,"startsOn":start,"endsOn":end})),
            )
            .await;
            assert_eq!(s, StatusCode::CREATED, "{v}");
            v
        }
    };
    let s1 = mk("Close 1", "2026-11-02", "2026-11-13", "p6cs-1").await;
    let s2 = mk("Close 2", "2026-11-16", "2026-11-27", "p6cs-2").await;
    let s3 = mk("Close 3", "2026-11-30", "2026-12-11", "p6cs-3").await;
    let (s1_id, s2_id, s3_id) = (
        s1["id"].as_str().unwrap().to_owned(),
        s2["id"].as_str().unwrap().to_owned(),
        s3["id"].as_str().unwrap().to_owned(),
    );
    for (n, id) in [&t1, &t2, &t3, &secret_id].into_iter().enumerate() {
        let (s, v) = post_as(
            world,
            &owner,
            &format!("/v1/tickets/{id}/commands/assign-sprint"),
            &format!("p6cs-a{n}"),
            json!({"sprintId":s1_id}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    // Start records the baseline once; later additions are scope changes.
    let (s, started) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s1_id}/commands/start"),
        "p6cs-start",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{started}");
    let baseline: i64 = sqlx::query_scalar(
        "SELECT task_count::bigint FROM penfii.sprint_baselines WHERE sprint_id=$1",
    )
    .bind(Uuid::parse_str(&s1_id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(baseline, 4);
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{t6}/commands/assign-sprint"),
        "p6cs-a6",
        json!({"sprintId":s1_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Work happens: T1 done, T3 canceled, T2 in progress.
    for (id, state, rev, key) in [
        (&t1, &done, 1, "p6cs-w1"),
        (&t3, &canceled, 1, "p6cs-w3"),
        (&t2, &doing, 1, "p6cs-w2"),
    ] {
        let (s, v) = post_as(
            world,
            &owner,
            &format!("/v1/tickets/{id}/commands/change-status"),
            key,
            json!({"expectedRevision":rev,"statusId":state}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    // Only an admin previews or closes.
    let (s, _) = get_as(
        world,
        &editor,
        &format!("/v1/sprints/{s1_id}/close-preview"),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, denied) = post_as(
        world,
        &editor,
        &format!("/v1/sprints/{s1_id}/commands/complete"),
        "p6cs-denied",
        json!({"expectedVersion":2}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    // The preview is the cancel point: it lists what the admin can see and changes nothing.
    let (s, preview) = get_as(
        world,
        &admin2,
        &format!("/v1/sprints/{s1_id}/close-preview"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{preview}");
    let keys = |list: &Value| -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|t| t["title"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(keys(&preview["done"]), vec!["Sprint task 1"]);
    assert_eq!(keys(&preview["canceled"]), vec!["Sprint task 3"]);
    let mut unfinished = keys(&preview["unfinished"]);
    unfinished.sort();
    assert_eq!(
        unfinished,
        vec!["Sprint task 2", "Sprint task 6"],
        "a Ticket the admin cannot see is neither listed nor counted"
    );
    assert!(!preview.to_string().contains("Secret sprint task"));
    assert_eq!(
        preview["defaultTargetId"], s2_id,
        "the next Sprint is the default"
    );
    assert_eq!(preview["targets"].as_array().unwrap().len(), 2);
    let still_current: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.sprint_assignments WHERE sprint_id=$1 AND ended_at IS NULL",
    )
    .bind(Uuid::parse_str(&s1_id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(still_current, 5, "previewing changes nothing");

    // Invalid destinations are refused before anything moves.
    let (other_project, _) = new_project(world, workspace_id, "Close sprint other").await;
    let foreign = {
        let (_, v, _) = call(
            &world.app,
            "POST",
            &format!("/v1/projects/{other_project}/sprints"),
            Some(&owner),
            Some("p6cs-f"),
            Some(json!({"name":"Foreign","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
        )
        .await;
        v["id"].as_str().unwrap().to_owned()
    };
    for (n, target) in [s1_id.clone(), foreign, Uuid::new_v4().to_string()]
        .into_iter()
        .enumerate()
    {
        let (s, v) = post_as(
            world,
            &admin2,
            &format!("/v1/sprints/{s1_id}/commands/complete"),
            &format!("p6cs-bad{n}"),
            json!({"expectedVersion":2,"carryToSprintId":target}),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    let (s, stale) = post_as(
        world,
        &admin2,
        &format!("/v1/sprints/{s1_id}/commands/complete"),
        "p6cs-stale",
        json!({"expectedVersion":1,"carryToSprintId":s2_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");

    // Two closes race on the same version: exactly one wins, the other changes nothing.
    let close = |key: &'static str| {
        let app = world.app.clone();
        let cookie = admin2.clone();
        let path = format!("/v1/sprints/{s1_id}/commands/complete");
        let body = json!({"expectedVersion":2,"carryToSprintId":s2_id});
        async move { call(&app, "POST", &path, Some(&cookie), Some(key), Some(body)).await }
    };
    let (a, b) = tokio::join!(close("p6cs-race-a"), close("p6cs-race-b"));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{} / {}",
        a.1,
        b.1
    );
    // The same key replays the same result.
    let (s, replay) = post_as(
        world,
        &admin2,
        &format!("/v1/sprints/{s1_id}/commands/complete"),
        if a.0 == StatusCode::OK {
            "p6cs-race-a"
        } else {
            "p6cs-race-b"
        },
        json!({"expectedVersion":2,"carryToSprintId":s2_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{replay}");
    assert_eq!(replay["state"], "COMPLETED");
    let (s, again) = post_as(
        world,
        &admin2,
        &format!("/v1/sprints/{s1_id}/commands/complete"),
        "p6cs-again",
        json!({"expectedVersion":3}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "a closed Sprint is not closed or reopened again: {again}"
    );
    assert_eq!(again["code"], "invalid_sprint_state");

    // Where everything is now. Ticket States never changed.
    let current_sprint = |id: String| {
        let pool = world.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>("SELECT s.name FROM penfii.sprint_assignments a JOIN penfii.sprints s ON s.id=a.sprint_id WHERE a.ticket_id=$1 AND a.ended_at IS NULL")
                .bind(Uuid::parse_str(&id).unwrap()).fetch_optional(&pool).await.unwrap().flatten()
        }
    };
    assert_eq!(current_sprint(t2.clone()).await.as_deref(), Some("Close 2"));
    assert_eq!(current_sprint(t6.clone()).await.as_deref(), Some("Close 2"));
    assert_eq!(
        current_sprint(t1.clone()).await,
        None,
        "finished work is recorded, not carried"
    );
    assert_eq!(current_sprint(t3.clone()).await, None);
    assert_eq!(
        current_sprint(secret_id.clone()).await,
        None,
        "an unseen Task is released, not carried"
    );
    let (_, two) = get_as(world, &owner, &format!("/v1/tickets/{t2}")).await;
    assert_eq!(two["statusId"], doing, "closing never changes a State");
    assert_eq!(two["sprint"]["name"], "Close 2");
    let (_, sub_now) = get_as(
        world,
        &owner,
        &format!("/v1/tickets/{}", sub["id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(
        sub_now["sprint"]["name"], "Close 2",
        "a Subtask goes with its parent"
    );
    assert_eq!(sub_now["sprint"]["inherited"], true);

    // The closed Sprint's list comes from the recorded participation, not from a filter.
    let (s, results) = get_as(world, &owner, &format!("/v1/sprints/{s1_id}/results")).await;
    assert_eq!(s, StatusCode::OK, "{results}");
    assert_eq!(results["sprint"]["state"], "COMPLETED");
    assert_eq!(results["baselineCount"], 4);
    let rows = results["results"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    let by = |title: &str| rows.iter().find(|r| r["ticket"]["title"] == title).unwrap();
    assert_eq!(by("Sprint task 1")["outcome"], "DONE");
    assert_eq!(by("Sprint task 3")["outcome"], "CANCELED");
    assert_eq!(by("Sprint task 2")["outcome"], "NOT_DONE");
    assert_eq!(by("Sprint task 2")["carriedTo"]["name"], "Close 2");
    assert_eq!(by("Sprint task 2")["inBaseline"], true);
    assert_eq!(
        by("Sprint task 6")["inBaseline"],
        false,
        "added after the start"
    );
    assert_eq!(by("Secret sprint task")["needsNextAssignment"], true);
    assert!(by("Secret sprint task")["carriedTo"].is_null());
    // Another person does not see the unseen Task in the list at all.
    let (_, for_admin2) = get_as(world, &admin2, &format!("/v1/sprints/{s1_id}/results")).await;
    assert_eq!(for_admin2["results"].as_array().unwrap().len(), 4);
    assert!(!for_admin2.to_string().contains("Secret sprint task"));
    // The next Sprint knows where its work came from.
    let (_, next) = get_as(world, &owner, &format!("/v1/sprints/{s2_id}/results")).await;
    let carried = next["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["ticket"]["title"] == "Sprint task 2")
        .unwrap();
    assert_eq!(carried["carriedFrom"]["name"], "Close 1");

    // No choice means the backlog; Cancel only releases.
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s2_id}/commands/start"),
        "p6cs-start2",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, closed) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s2_id}/commands/complete"),
        "p6cs-close2",
        json!({"expectedVersion":2}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{closed}");
    assert_eq!(
        current_sprint(t2.clone()).await,
        None,
        "the backlog is the default destination"
    );
    let (_, results2) = get_as(world, &owner, &format!("/v1/sprints/{s2_id}/results")).await;
    assert!(
        results2["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["carriedTo"].is_null())
    );
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{t4}/commands/assign-sprint"),
        "p6cs-a-t4",
        json!({"sprintId":s3_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, canceled_sprint) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s3_id}/commands/cancel"),
        "p6cs-cancel",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{canceled_sprint}");
    assert_eq!(current_sprint(t4.clone()).await, None);
    let (_, results3) = get_as(world, &owner, &format!("/v1/sprints/{s3_id}/results")).await;
    assert_eq!(results3["results"][0]["outcome"], "NOT_DONE");
    let (s, bad) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s3_id}/commands/cancel"),
        "p6cs-cancel2",
        json!({"expectedVersion":2,"carryToSprintId":s1_id}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
}
