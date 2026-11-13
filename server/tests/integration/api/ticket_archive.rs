//! Ticket archive: the lifecycle, read-only behaviour, the Trash combination, Sprint-completion
//! archive (now or scheduled), and bulk archive/trash with partial failure.

use super::tickets::state_id;
use super::{
    World, call, get_as, grant, make_ticket, new_project, post_as, user_id_of, viewer_cookie,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn ticket(world: &World, project: &str, key: &str, body: Value) -> String {
    let (status, value) = make_ticket(world, project, key, body).await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["id"].as_str().unwrap().to_owned()
}

async fn set_state(world: &World, id: &str, state: &str, revision: i64, key: &str) {
    let (status, value) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{id}/commands/change-status"),
        key,
        json!({"expectedRevision":revision,"statusId":state}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
}

async fn command(
    world: &World,
    cookie: &str,
    id: &str,
    name: &str,
    key: &str,
) -> (StatusCode, Value) {
    post_as(
        world,
        cookie,
        &format!("/v1/tickets/{id}/commands/{name}"),
        key,
        json!({}),
    )
    .await
}

async fn archived_at(world: &World, id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar("SELECT archived_at FROM penfii.tickets WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn archive_is_admin_only_read_only_and_independent_of_the_trash(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Archive").await;
    let done = state_id(&workflow, "DONE");
    let editor = viewer_cookie(world, workspace_id, "arch-editor@penfii.test").await;
    grant(
        world,
        &project_id,
        user_id_of(world, "arch-editor@penfii.test").await,
        "EDITOR",
    )
    .await;
    let parent = ticket(
        world,
        &project_id,
        "ar-p",
        json!({"title":"Parent","hierarchyKind":"TASK"}),
    )
    .await;
    let child = ticket(
        world,
        &project_id,
        "ar-c",
        json!({"title":"Child","hierarchyKind":"SUBTASK","parentId":parent}),
    )
    .await;

    // An open child keeps its parent from being archived.
    let (status, body) = command(world, &world.owner_cookie, &parent, "archive", "ar-1").await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("open_children")),
        "{body}"
    );
    // Only the Project admin archives; an editor is refused and the anonymous caller is not signed in.
    assert_eq!(
        command(world, &editor, &child, "archive", "ar-2").await.0,
        StatusCode::FORBIDDEN
    );
    let (status, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{child}/commands/archive"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        command(world, &world.owner_cookie, &child, "archive", "ar-3")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    // Archiving again changes nothing.
    assert_eq!(
        command(world, &world.owner_cookie, &child, "archive", "ar-4")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code='ticket.archived' AND target_id=$1")
        .bind(Uuid::parse_str(&child).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(audits, 1);

    // Out of the board, the parent's open count drops; but the Ticket still opens, read-only.
    let (_, board) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/board"),
    )
    .await;
    assert!(!board.to_string().contains("\"Child\""), "{board}");
    let (status, shown) = get_as(world, &editor, &format!("/v1/tickets/{child}")).await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert!(shown["archivedAt"].is_string(), "{shown}");
    let (status, _) = get_as(world, &editor, &format!("/v1/tickets/{child}/comments")).await;
    assert_eq!(status, StatusCode::OK);
    // Its attachments, activity and dependencies stay readable.
    let child_id = Uuid::parse_str(&child).unwrap();
    let file_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'spec.pdf',9,'application/pdf','AVAILABLE',$4)")
        .bind(file_id).bind(workspace_id).bind(format!("ar/{file_id}")).bind(world.owner_id)
        .execute(&world.pool).await.unwrap();
    sqlx::query("INSERT INTO penfii.ticket_attachments(id,ticket_id,file_id,attached_by) VALUES($1,$2,$3,$4)")
        .bind(Uuid::new_v4()).bind(child_id).bind(file_id).bind(world.owner_id)
        .execute(&world.pool).await.unwrap();
    for path in [
        "attachments",
        "activity",
        "dependencies",
        "children",
        "revisions",
    ] {
        let (status, body) = get_as(world, &editor, &format!("/v1/tickets/{child}/{path}")).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }
    let (_, files) = get_as(world, &editor, &format!("/v1/tickets/{child}/attachments")).await;
    assert!(files.to_string().contains("spec.pdf"), "{files}");
    // Every write is refused, for the admin too.
    let (status, _) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{child}/comments"),
        "ar-cm",
        json!({"body":{"type":"doc","content":[]}}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{child}/commands/change-status"),
        "ar-st",
        json!({"expectedRevision":1,"statusId":done}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{child}"),
        Some(&world.owner_cookie),
        Some("ar-pt"),
        Some(json!({"expectedRevision":1,"title":"changed"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The archive list is readable by a member and shows it.
    let (status, list) = get_as(
        world,
        &editor,
        &format!("/v1/projects/{project_id}/archive"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["items"][0]["title"], "Child");
    // The parent can now be archived (its only child is archived).
    assert_eq!(
        command(world, &world.owner_cookie, &parent, "archive", "ar-5")
            .await
            .0,
        StatusCode::NO_CONTENT
    );

    // Trash and archive are independent: an archived Ticket can be trashed, and restoring it keeps the archive.
    assert_eq!(
        command(world, &world.owner_cookie, &child, "trash", "ar-6")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (status, body) = command(world, &world.owner_cookie, &child, "unarchive", "ar-7").await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("ticket_trashed")),
        "{body}"
    );
    assert_eq!(
        command(
            world,
            &world.owner_cookie,
            &child,
            "restore-from-trash",
            "ar-8"
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(archived_at(world, &child).await.is_some());
    // Unarchive is admin only and brings it back to the board.
    assert_eq!(
        command(world, &editor, &child, "unarchive", "ar-9").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        command(world, &world.owner_cookie, &child, "unarchive", "ar-10")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(archived_at(world, &child).await.is_none());
    let (status, body) = command(world, &world.owner_cookie, &child, "unarchive", "ar-11").await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("not_archived"))
    );
    let (_, board) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/board"),
    )
    .await;
    assert!(board.to_string().contains("\"Child\""), "{board}");
    // A trashed Ticket cannot be archived.
    assert_eq!(
        command(world, &world.owner_cookie, &child, "trash", "ar-12")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (status, body) = command(world, &world.owner_cookie, &child, "archive", "ar-13").await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("ticket_trashed"))
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn completing_a_sprint_archives_finished_work_now_or_on_schedule(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "SprintArchive").await;
    let (done, canceled, doing) = (
        state_id(&workflow, "DONE"),
        state_id(&workflow, "CANCELED"),
        state_id(&workflow, "IN_PROGRESS"),
    );
    let owner = world.owner_cookie.clone();
    let mk_sprint =
        |name: &'static str, start: &'static str, end: &'static str, key: &'static str| {
            let (owner, project_id) = (owner.clone(), project_id.clone());
            async move {
                let (s, v, _) = call(
                    &world.app,
                    "POST",
                    &format!("/v1/projects/{project_id}/sprints"),
                    Some(&owner),
                    Some(key),
                    Some(json!({"name":name,"startsOn":start,"endsOn":end})),
                )
                .await;
                assert_eq!(s, StatusCode::CREATED, "{v}");
                v["id"].as_str().unwrap().to_owned()
            }
        };
    let s1 = mk_sprint("S1", "2026-11-02", "2026-11-13", "sa-s1").await;
    let s2 = mk_sprint("S2", "2026-11-16", "2026-11-27", "sa-s2").await;
    let s3 = mk_sprint("S3", "2026-11-30", "2026-12-11", "sa-s3").await;
    let s4 = mk_sprint("S4", "2026-12-14", "2026-12-25", "sa-s4").await;
    let mut tasks = Vec::new();
    for n in 0..4 {
        tasks.push(
            ticket(
                world,
                &project_id,
                &format!("sa-t{n}"),
                json!({"title":format!("T{n}"),"hierarchyKind":"TASK"}),
            )
            .await,
        );
    }
    let sub = ticket(
        world,
        &project_id,
        "sa-sub",
        json!({"title":"Sub","hierarchyKind":"SUBTASK","parentId":tasks[0]}),
    )
    .await;
    set_state(world, &sub, &done, 1, "sa-sub-done").await;
    for (n, sprint) in [(0, &s1), (1, &s1), (2, &s1), (3, &s3)] {
        let (s, v) = post_as(
            world,
            &owner,
            &format!("/v1/tickets/{}/commands/assign-sprint", tasks[n]),
            &format!("sa-as{n}"),
            json!({"sprintId":sprint}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let start = |id: String, key: &'static str, v: i64| {
        let owner = owner.clone();
        async move {
            let (s, b) = post_as(
                world,
                &owner,
                &format!("/v1/sprints/{id}/commands/start"),
                key,
                json!({"expectedVersion":v}),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{b}");
        }
    };
    start(s1.clone(), "sa-start1", 1).await;
    start(s3.clone(), "sa-start3", 1).await;
    set_state(world, &tasks[0], &done, 1, "sa-d0").await;
    set_state(world, &tasks[1], &doing, 1, "sa-d1").await;
    set_state(world, &tasks[2], &canceled, 1, "sa-d2").await;

    // A past time is refused before anything changes.
    let (s, past) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s1}/commands/complete"),
        "sa-past",
        json!({"expectedVersion":2,"archive":"AT","archiveAt":"2020-01-01T00:00:00Z"}),
    )
    .await;
    assert_eq!(
        (s, past["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("validation")),
        "{past}"
    );
    // Complete with an immediate archive and the existing carry-over of unfinished work.
    let (s, completed) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s1}/commands/complete"),
        "sa-c1",
        json!({"expectedVersion":2,"carryToSprintId":s2,"archive":"IMMEDIATE"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{completed}");
    assert!(
        archived_at(world, &tasks[0]).await.is_some(),
        "done task archived"
    );
    assert!(
        archived_at(world, &sub).await.is_some(),
        "its finished subtask goes with it"
    );
    assert!(
        archived_at(world, &tasks[2]).await.is_some(),
        "canceled task archived"
    );
    assert!(
        archived_at(world, &tasks[1]).await.is_none(),
        "in-progress task stays"
    );
    let carried: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.sprint_assignments WHERE ticket_id=$1 AND sprint_id=$2 AND ended_at IS NULL")
        .bind(Uuid::parse_str(&tasks[1]).unwrap())
        .bind(Uuid::parse_str(&s2).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(
        carried, 1,
        "the existing carry-over still moves unfinished work"
    );
    // The Sprint's history keeps the archived work.
    let (s, results) = get_as(world, &owner, &format!("/v1/sprints/{s1}/results")).await;
    assert_eq!(s, StatusCode::OK, "{results}");
    assert!(results.to_string().contains("T0"), "{results}");

    // Scheduled: nothing changes until the time; it is re-judged then.
    set_state(world, &tasks[3], &done, 1, "sa-d3").await;
    let at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let (s, completed) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s3}/commands/complete"),
        "sa-c3",
        json!({"expectedVersion":2,"archive":"AT","archiveAt":at}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{completed}");
    assert!(archived_at(world, &tasks[3]).await.is_none());
    let (s, schedule) = get_as(world, &owner, &format!("/v1/sprints/{s3}/archive-schedule")).await;
    assert_eq!(s, StatusCode::OK, "{schedule}");
    assert_eq!(schedule["schedule"]["state"], "SCHEDULED");
    assert_eq!(
        penfii_core::planning::run_due(&world.pool, 10)
            .await
            .unwrap(),
        0
    );
    // It can be moved (not into the past) and canceled.
    let version = schedule["schedule"]["version"].as_i64().unwrap();
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/sprints/{s3}/archive-schedule"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":version,"archiveAt":"2020-01-01T00:00:00Z"})),
    )
    .await;
    assert_ne!(s, StatusCode::OK);
    let later = (chrono::Utc::now() + chrono::Duration::hours(5)).to_rfc3339();
    let (s, moved, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/sprints/{s3}/archive-schedule"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":version,"archiveAt":later})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    // A reopened Ticket is left alone when the time comes; after a "restart" (no run at the due time) it still runs.
    set_state(world, &tasks[3], &doing, 2, "sa-reopen").await;
    sqlx::query("UPDATE penfii.sprint_archive_schedules SET due_at = now() - interval '1 hour' WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&s3).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    assert_eq!(
        penfii_core::planning::run_due(&world.pool, 10)
            .await
            .unwrap(),
        1
    );
    assert!(
        archived_at(world, &tasks[3]).await.is_none(),
        "reopened work is not archived"
    );
    let (_, after) = get_as(world, &owner, &format!("/v1/sprints/{s3}/archive-schedule")).await;
    assert_eq!(
        (
            after["schedule"]["state"].as_str(),
            after["schedule"]["skippedCount"].as_i64()
        ),
        (Some("DONE"), Some(1)),
        "{after}"
    );
    assert_eq!(
        penfii_core::planning::run_due(&world.pool, 10)
            .await
            .unwrap(),
        0,
        "runs once"
    );

    // A canceled schedule never runs.
    let t5 = ticket(
        world,
        &project_id,
        "sa-t5",
        json!({"title":"T5","hierarchyKind":"TASK"}),
    )
    .await;
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{t5}/commands/assign-sprint"),
        "sa-as5",
        json!({"sprintId":s4}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    start(s4.clone(), "sa-start4", 1).await;
    set_state(world, &t5, &done, 1, "sa-d5").await;
    let at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{s4}/commands/complete"),
        "sa-c4",
        json!({"expectedVersion":2,"archive":"AT","archiveAt":at}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, sched) = get_as(world, &owner, &format!("/v1/sprints/{s4}/archive-schedule")).await;
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/sprints/{s4}/archive-schedule"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":sched["schedule"]["version"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    sqlx::query("UPDATE penfii.sprint_archive_schedules SET due_at = now() - interval '1 hour' WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&s4).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    assert_eq!(
        penfii_core::planning::run_due(&world.pool, 10)
            .await
            .unwrap(),
        0
    );
    assert!(archived_at(world, &t5).await.is_none());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn bulk_archive_and_trash_report_every_ticket(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "BulkArchive").await;
    let done = state_id(&workflow, "DONE");
    let editor = viewer_cookie(world, workspace_id, "bulk-arch-editor@penfii.test").await;
    grant(
        world,
        &project_id,
        user_id_of(world, "bulk-arch-editor@penfii.test").await,
        "EDITOR",
    )
    .await;
    let a = ticket(world, &project_id, "ba-a", json!({"title":"A"})).await;
    let b = ticket(world, &project_id, "ba-b", json!({"title":"B"})).await;
    let parent = ticket(
        world,
        &project_id,
        "ba-p",
        json!({"title":"P","hierarchyKind":"TASK"}),
    )
    .await;
    let _open = ticket(
        world,
        &project_id,
        "ba-o",
        json!({"title":"O","hierarchyKind":"SUBTASK","parentId":parent}),
    )
    .await;
    for (id, key) in [(&a, "ba-da"), (&b, "ba-db")] {
        set_state(world, id, &done, 1, key).await;
    }
    let path = format!("/v1/workspaces/{workspace_id}/tickets/bulk");
    // An editor may not archive: every ticket is reported as refused.
    let (s, v) = post_as(
        world,
        &editor,
        &path,
        "ba-1",
        json!({"ticketIds":[a,b],"change":{"type":"ARCHIVE"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["succeeded"].as_i64(), v["failed"].as_i64()),
        (Some(0), Some(2)),
        "{v}"
    );
    assert!(archived_at(world, &a).await.is_none());
    // The admin archives; the parent with an open child fails with its own reason and the rest still go through.
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &path,
        "ba-2",
        json!({"ticketIds":[a,b,parent],"change":{"type":"ARCHIVE"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["succeeded"].as_i64(), v["failed"].as_i64()),
        (Some(2), Some(1)),
        "{v}"
    );
    let failed = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["ok"] == false)
        .unwrap();
    assert_eq!(failed["code"], "open_children");
    assert!(archived_at(world, &a).await.is_some() && archived_at(world, &parent).await.is_none());
    // Trash goes through the same path; an archived Ticket can be trashed.
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &path,
        "ba-3",
        json!({"ticketIds":[a,b],"change":{"type":"TRASH"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["succeeded"], 2, "{v}");
    let trashed: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.tickets WHERE trashed_at IS NOT NULL AND archived_at IS NOT NULL")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(trashed, 2);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn every_write_path_refuses_an_archived_ticket_and_dependencies_follow_the_policy(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "ReadOnly").await;
    let (done, doing) = (
        state_id(&workflow, "DONE"),
        state_id(&workflow, "IN_PROGRESS"),
    );
    let owner = world.owner_cookie.clone();
    let target = ticket(
        world,
        &project_id,
        "ro-t",
        json!({"title":"Frozen","hierarchyKind":"TASK"}),
    )
    .await;
    let other = ticket(
        world,
        &project_id,
        "ro-o",
        json!({"title":"Live other","hierarchyKind":"TASK"}),
    )
    .await;
    let pred = ticket(
        world,
        &project_id,
        "ro-p",
        json!({"title":"Predecessor","hierarchyKind":"TASK"}),
    )
    .await;
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{other}/dependencies"),
        "ro-d1",
        json!({"predecessorId":pred,"kind":"BLOCKS"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    set_state(world, &target, &done, 1, "ro-done").await;
    let (_, sprint, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&owner),
        Some("ro-sp"),
        Some(json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let (_, milestone, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/milestones"),
        Some(&owner),
        Some("ro-ms"),
        Some(json!({"name":"M"})),
    )
    .await;
    let file_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'x.txt',1,'text/plain','AVAILABLE',$4)")
        .bind(file_id).bind(workspace_id).bind(format!("ro/{file_id}")).bind(world.owner_id)
        .execute(&world.pool).await.unwrap();
    assert_eq!(
        command(world, &owner, &target, "archive", "ro-ar").await.0,
        StatusCode::NO_CONTENT
    );

    // Every write a Ticket has is refused. (Each body is valid, so the refusal is the lifecycle's.)
    let writes: Vec<(&str, String, Value)> = vec![
        (
            "PATCH",
            format!("/v1/tickets/{target}"),
            json!({"expectedRevision":2,"title":"x"}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/commands/change-status"),
            json!({"expectedRevision":2,"statusId":doing}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/commands/move"),
            json!({"position":"first"}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/commands/assign-sprint"),
            json!({"sprintId":sprint_id}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/commands/set-milestones"),
            json!({"milestoneIds":[milestone["id"]]}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/commands/restore"),
            json!({"expectedRevision":2,"sourceRevision":1,"fields":["title"]}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/comments"),
            json!({"body":{"type":"doc","content":[]}}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/attachments"),
            json!({"fileId":file_id}),
        ),
        (
            "POST",
            format!("/v1/tickets/{target}/dependencies"),
            json!({"predecessorId":other,"kind":"BLOCKS"}),
        ),
        (
            "POST",
            format!("/v1/tickets/{other}/dependencies"),
            json!({"predecessorId":target,"kind":"RELATED"}),
        ),
        (
            "POST",
            format!("/v1/projects/{project_id}/tickets"),
            json!({"title":"child","hierarchyKind":"SUBTASK","parentId":target}),
        ),
    ];
    for (n, (method, path, body)) in writes.into_iter().enumerate() {
        let (status, _, _) = call(
            &world.app,
            method,
            &path,
            Some(&owner),
            Some(&format!("ro-w{n}")),
            Some(body),
        )
        .await;
        assert!(
            matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::CONFLICT | StatusCode::FORBIDDEN
            ),
            "{method} {path} was not refused: {status}"
        );
    }
    let revision: i64 = sqlx::query_scalar("SELECT revision_no FROM penfii.tickets WHERE id=$1")
        .bind(Uuid::parse_str(&target).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(revision, 2, "nothing changed it");

    // Dependencies: an archived predecessor stays in the list, but no longer blocks its successor.
    let (s, deps) = get_as(world, &owner, &format!("/v1/tickets/{other}/dependencies")).await;
    assert_eq!(s, StatusCode::OK, "{deps}");
    assert_eq!(
        deps["blockedBy"].as_array().map(Vec::len),
        Some(1),
        "{deps}"
    );
    assert_eq!(
        command(world, &owner, &pred, "archive", "ro-arp").await.0,
        StatusCode::NO_CONTENT
    );
    let (_, deps) = get_as(world, &owner, &format!("/v1/tickets/{other}/dependencies")).await;
    assert_eq!(
        deps["blockedBy"].as_array().map(Vec::len),
        Some(1),
        "still listed: {deps}"
    );
    let (_, shown) = get_as(world, &owner, &format!("/v1/tickets/{other}")).await;
    assert_eq!(shown["dependencies"]["openBlockers"], 0, "{shown}");
    set_state(world, &other, &done, 1, "ro-other-done").await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn trashed_children_do_not_hold_a_parent_from_completing(pool: PgPool) {
    let world = &World::new(pool).await;
    let (project_id, workflow) = new_project(world, world.workspace_id, "Subtasks").await;
    let done = state_id(&workflow, "DONE");
    let parent = ticket(
        world,
        &project_id,
        "su-p",
        json!({"title":"P","hierarchyKind":"TASK"}),
    )
    .await;
    let child = ticket(
        world,
        &project_id,
        "su-c",
        json!({"title":"C","hierarchyKind":"SUBTASK","parentId":parent}),
    )
    .await;
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{parent}/commands/change-status"),
        "su-1",
        json!({"expectedRevision":1,"statusId":done}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("unfinished_subtasks")),
        "{v}"
    );
    assert_eq!(
        command(world, &world.owner_cookie, &child, "trash", "su-t")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{parent}/commands/change-status"),
        "su-2",
        json!({"expectedRevision":1,"statusId":done}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_trash_shows_archived_and_expiry_and_expired_tickets_are_purged_automatically(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let (project_id, _) = new_project(world, world.workspace_id, "Trash").await;
    let owner = world.owner_cookie.clone();
    let parent = ticket(
        world,
        &project_id,
        "tr-p",
        json!({"title":"Parent","hierarchyKind":"TASK"}),
    )
    .await;
    let child = ticket(
        world,
        &project_id,
        "tr-c",
        json!({"title":"Child","hierarchyKind":"SUBTASK","parentId":parent}),
    )
    .await;
    let plain = ticket(world, &project_id, "tr-x", json!({"title":"Plain"})).await;
    let archived = ticket(world, &project_id, "tr-a", json!({"title":"Archived one"})).await;
    let restored = ticket(world, &project_id, "tr-r", json!({"title":"Restored"})).await;
    let live = ticket(world, &project_id, "tr-l", json!({"title":"Live"})).await;
    assert_eq!(
        command(world, &owner, &archived, "archive", "tr-1").await.0,
        StatusCode::NO_CONTENT
    );
    for (n, id) in [&parent, &child, &plain, &archived, &restored]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            command(world, &owner, id, "trash", &format!("tr-t{n}"))
                .await
                .0,
            StatusCode::NO_CONTENT
        );
    }
    let (s, list) = get_as(world, &owner, &format!("/v1/projects/{project_id}/trash")).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let row = |title: &str| {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["title"] == title)
            .unwrap()
            .clone()
    };
    assert_eq!(row("Archived one")["archived"], true);
    assert_eq!(row("Plain")["archived"], false);
    // The automatic purge is off unless PENFII_TRASH_RETENTION_DAYS is set, so there is no purge date.
    assert!(row("Plain")["purgeAt"].is_null(), "{list}");
    assert_eq!(row("Parent")["purgeBlocked"], "children");
    assert!(row("Plain")["purgeBlocked"].is_null());

    // Nothing is due yet; with the sweep off nothing happens either.
    assert_eq!(
        penfii_core::ticket_lifecycle::purge_expired(&world.pool, None, 30, 100)
            .await
            .unwrap()
            .purged,
        0
    );
    sqlx::query(
        "UPDATE penfii.tickets SET trashed_at = now() - interval '31 days' WHERE title <> 'Live'",
    )
    .execute(&world.pool)
    .await
    .unwrap();
    assert_eq!(
        penfii_core::ticket_lifecycle::purge_expired(&world.pool, None, 0, 100)
            .await
            .unwrap()
            .purged,
        0
    );
    // Restored meanwhile: left alone.
    assert_eq!(
        command(world, &owner, &restored, "restore-from-trash", "tr-rs")
            .await
            .0,
        StatusCode::NO_CONTENT
    );

    let sweep = penfii_core::ticket_lifecycle::purge_expired(&world.pool, None, 30, 100)
        .await
        .unwrap();
    // Plain, the archived one and the Subtask go; so does its Task, in the same pass (children first).
    assert_eq!((sweep.purged, sweep.held), (4, 0), "{sweep:?}");
    let state = |id: &str| {
        let pool = world.pool.clone();
        let id = Uuid::parse_str(id).unwrap();
        async move {
            sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>, String)>(
                "SELECT purged_at, purged_by, title FROM penfii.tickets WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    for id in [&plain, &archived, &child, &parent] {
        let (purged_at, purged_by, title) = state(id).await;
        assert!(
            purged_at.is_some() && purged_by.is_none() && title == "[purged]",
            "{id}"
        );
    }
    assert!(state(&restored).await.0.is_none() && state(&live).await.0.is_none());
    // The audit entry names no person and says it was automatic; the marker row keeps the key.
    let (actor, automatic): (Option<Uuid>, bool) = sqlx::query_as("SELECT actor_user_id, (data->>'automatic')::boolean FROM penfii.audit_entries WHERE action_code='ticket.purged' AND target_id=$1")
        .bind(Uuid::parse_str(&archived).unwrap()).fetch_one(&world.pool).await.unwrap();
    assert!(actor.is_none() && automatic);
    // A second pass has nothing to do.
    assert_eq!(
        penfii_core::ticket_lifecycle::purge_expired(&world.pool, None, 30, 100)
            .await
            .unwrap(),
        Default::default()
    );
    let (s, list) = get_as(world, &owner, &format!("/v1/projects/{project_id}/trash")).await;
    assert_eq!(
        (s, list["items"].as_array().map(Vec::len)),
        (StatusCode::OK, Some(0)),
        "{list}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_expired_ticket_with_a_dependency_is_held_and_a_manual_purge_of_an_archived_one_keeps_history(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let (project_id, workflow) = new_project(world, world.workspace_id, "Held").await;
    let done = state_id(&workflow, "DONE");
    let owner = world.owner_cookie.clone();
    let a = ticket(world, &project_id, "he-a", json!({"title":"A"})).await;
    let b = ticket(world, &project_id, "he-b", json!({"title":"B"})).await;
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{b}/dependencies"),
        "he-d",
        json!({"predecessorId":a,"kind":"RELATED"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, deps) = get_as(world, &owner, &format!("/v1/tickets/{b}/dependencies")).await;
    let dep_id = deps["related"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        command(world, &owner, &a, "trash", "he-t").await.0,
        StatusCode::NO_CONTENT
    );
    sqlx::query("UPDATE penfii.tickets SET trashed_at = now() - interval '40 days' WHERE id=$1")
        .bind(Uuid::parse_str(&a).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let sweep = penfii_core::ticket_lifecycle::purge_expired(&world.pool, None, 30, 100)
        .await
        .unwrap();
    assert_eq!((sweep.purged, sweep.held), (0, 1), "kept for the next pass");
    let (_, list) = get_as(world, &owner, &format!("/v1/projects/{project_id}/trash")).await;
    assert_eq!(list["items"][0]["purgeBlocked"], "dependencies", "{list}");
    // Once the dependency is gone, the next pass takes it.
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/dependencies/{dep_id}"),
        Some(&owner),
        Some("he-rm"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        penfii_core::ticket_lifecycle::purge_expired(&world.pool, None, 30, 100)
            .await
            .unwrap()
            .purged,
        1
    );

    // Archive -> trash -> manual purge: the Sprint history keeps its row and the key survives as the marker.
    let (_, sprint, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&owner),
        Some("he-sp"),
        Some(json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let c = ticket(
        world,
        &project_id,
        "he-c",
        json!({"title":"C","hierarchyKind":"TASK"}),
    )
    .await;
    post_as(
        world,
        &owner,
        &format!("/v1/tickets/{c}/commands/assign-sprint"),
        "he-as",
        json!({"sprintId":sprint_id}),
    )
    .await;
    post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "he-st",
        json!({"expectedVersion":1}),
    )
    .await;
    set_state(world, &c, &done, 1, "he-cd").await;
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/complete"),
        "he-cp",
        json!({"expectedVersion":2,"archive":"IMMEDIATE"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(archived_at(world, &c).await.is_some());
    assert_eq!(
        command(world, &owner, &c, "trash", "he-ct").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        command(world, &owner, &c, "purge", "he-cx").await.0,
        StatusCode::NO_CONTENT
    );
    let (s, results) = get_as(world, &owner, &format!("/v1/sprints/{sprint_id}/results")).await;
    assert_eq!(s, StatusCode::OK, "{results}");
    // The closed Sprint keeps its counters and baseline; the purged work itself is erased from the results
    // (a purge is irreversible, the marker row keeps only the key).
    assert_eq!(results["baselineCount"], 1, "{results}");
    assert_eq!(results["sprint"]["state"], "COMPLETED");
    let marker: String = sqlx::query_scalar("SELECT public_key FROM penfii.tickets WHERE id=$1")
        .bind(Uuid::parse_str(&c).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert!(!marker.is_empty());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn one_failing_schedule_does_not_keep_another_from_running(pool: PgPool) {
    let world = &World::new(pool).await;
    let (project_id, workflow) = new_project(world, world.workspace_id, "Poison").await;
    let done = state_id(&workflow, "DONE");
    let owner = world.owner_cookie.clone();
    // A trigger makes archiving the ticket titled "Poison" fail with an error that is not a conflict.
    sqlx::query("CREATE FUNCTION penfii.poison() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.title = 'Poison' AND NEW.archived_at IS NOT NULL THEN RAISE EXCEPTION 'poisoned'; END IF; RETURN NEW; END $$")
        .execute(&world.pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER poison BEFORE UPDATE ON penfii.tickets FOR EACH ROW EXECUTE FUNCTION penfii.poison()")
        .execute(&world.pool)
        .await
        .unwrap();
    let mut sprints = Vec::new();
    for (n, (title, start, end)) in [
        ("Poison", "2026-11-02", "2026-11-13"),
        ("Fine", "2026-11-16", "2026-11-27"),
    ]
    .into_iter()
    .enumerate()
    {
        let (_, sprint, _) = call(
            &world.app,
            "POST",
            &format!("/v1/projects/{project_id}/sprints"),
            Some(&owner),
            Some(&format!("po-s{n}")),
            Some(json!({"name":title,"startsOn":start,"endsOn":end})),
        )
        .await;
        let sprint_id = sprint["id"].as_str().unwrap().to_owned();
        let task = ticket(
            world,
            &project_id,
            &format!("po-t{n}"),
            json!({"title":title,"hierarchyKind":"TASK"}),
        )
        .await;
        post_as(
            world,
            &owner,
            &format!("/v1/tickets/{task}/commands/assign-sprint"),
            &format!("po-a{n}"),
            json!({"sprintId":sprint_id}),
        )
        .await;
        post_as(
            world,
            &owner,
            &format!("/v1/sprints/{sprint_id}/commands/start"),
            &format!("po-st{n}"),
            json!({"expectedVersion":1}),
        )
        .await;
        set_state(world, &task, &done, 1, &format!("po-d{n}")).await;
        let at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let (s, v) = post_as(
            world,
            &owner,
            &format!("/v1/sprints/{sprint_id}/commands/complete"),
            &format!("po-c{n}"),
            json!({"expectedVersion":2,"archive":"AT","archiveAt":at}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        sprints.push((sprint_id, task));
    }
    // The failing one is due first.
    sqlx::query("UPDATE penfii.sprint_archive_schedules SET due_at = now() - interval '2 hours' WHERE sprint_id=$1").bind(Uuid::parse_str(&sprints[0].0).unwrap()).execute(&world.pool).await.unwrap();
    sqlx::query("UPDATE penfii.sprint_archive_schedules SET due_at = now() - interval '1 hour' WHERE sprint_id=$1").bind(Uuid::parse_str(&sprints[1].0).unwrap()).execute(&world.pool).await.unwrap();
    assert_eq!(
        penfii_core::planning::run_due(&world.pool, 10)
            .await
            .unwrap(),
        1,
        "the other one still ran"
    );
    assert!(archived_at(world, &sprints[1].1).await.is_some());
    assert!(archived_at(world, &sprints[0].1).await.is_none());
    let state = |sprint: &str| {
        let pool = world.pool.clone();
        let id = Uuid::parse_str(sprint).unwrap();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT state FROM penfii.sprint_archive_schedules WHERE sprint_id=$1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        state(&sprints[0].0).await,
        "SCHEDULED",
        "kept for the next pass"
    );
    assert_eq!(state(&sprints[1].0).await, "DONE");
    // Once the cause is gone, the next pass takes it.
    sqlx::query("DROP TRIGGER poison ON penfii.tickets")
        .execute(&world.pool)
        .await
        .unwrap();
    assert_eq!(
        penfii_core::planning::run_due(&world.pool, 10)
            .await
            .unwrap(),
        1
    );
    assert!(archived_at(world, &sprints[0].1).await.is_some());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn lifecycle_is_a_project_power_and_does_not_change_who_can_see_a_private_channels_tickets(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Private").await;
    let done = state_id(&workflow, "DONE");
    let owner = world.owner_cookie.clone();
    let (s, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"arch secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{channel}");
    let secret = ticket(
        world,
        &project_id,
        "pc-1",
        json!({"title":"Secret","channelId":channel["id"]}),
    )
    .await;
    // A Project admin who is not in the private channel.
    let admin = viewer_cookie(world, workspace_id, "arch-admin2@penfii.test").await;
    grant(
        world,
        &project_id,
        user_id_of(world, "arch-admin2@penfii.test").await,
        "ADMIN",
    )
    .await;
    let sees = |cookie: &str, id: &str| {
        let (cookie, path) = (cookie.to_owned(), format!("/v1/tickets/{id}"));
        async move { get_as(world, &cookie, &path).await.0 }
    };
    assert_eq!(
        sees(&admin, &secret).await,
        StatusCode::NOT_FOUND,
        "the admin cannot see it"
    );

    // Archive and unarchive are the admin's to do, whether or not they can see the ticket ...
    assert_eq!(
        command(world, &admin, &secret, "archive", "pc-a").await.0,
        StatusCode::NO_CONTENT
    );
    assert!(archived_at(world, &secret).await.is_some());
    // ... and they change nothing about who can see it: still hidden from the admin, open (read-only) for the channel.
    assert_eq!(sees(&admin, &secret).await, StatusCode::NOT_FOUND);
    let (s, shown) = get_as(world, &owner, &format!("/v1/tickets/{secret}")).await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    assert!(shown["archivedAt"].is_string());
    let titles = |list: &Value| -> Vec<String> {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_owned())
            .collect()
    };
    let (_, list) = get_as(world, &admin, &format!("/v1/projects/{project_id}/archive")).await;
    assert!(
        titles(&list).is_empty(),
        "the archive list keeps the read gate: {list}"
    );
    let (_, list) = get_as(world, &owner, &format!("/v1/projects/{project_id}/archive")).await;
    assert_eq!(titles(&list), vec!["Secret"]);
    assert_eq!(
        command(world, &admin, &secret, "unarchive", "pc-b").await.0,
        StatusCode::NO_CONTENT
    );
    assert!(archived_at(world, &secret).await.is_none());
    assert_eq!(sees(&admin, &secret).await, StatusCode::NOT_FOUND);
    // Ordinary work on the ticket still needs the channel, for the admin too.
    let (s, _) = post_as(
        world,
        &admin,
        &format!("/v1/tickets/{secret}/commands/change-status"),
        "pc-c",
        json!({"expectedRevision":1,"statusId":done}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Completing a Sprint archives finished work the same way: the admin's completion reaches the private one.
    let open = ticket(
        world,
        &project_id,
        "pc-2",
        json!({"title":"Open","hierarchyKind":"TASK"}),
    )
    .await;
    let secret_task = ticket(
        world,
        &project_id,
        "pc-3",
        json!({"title":"Secret task","hierarchyKind":"TASK","channelId":channel["id"]}),
    )
    .await;
    let (_, sprint, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&owner),
        Some("pc-sp"),
        Some(json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    for (n, id) in [&open, &secret_task].into_iter().enumerate() {
        post_as(
            world,
            &owner,
            &format!("/v1/tickets/{id}/commands/assign-sprint"),
            &format!("pc-as{n}"),
            json!({"sprintId":sprint_id}),
        )
        .await;
    }
    post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "pc-st",
        json!({"expectedVersion":1}),
    )
    .await;
    set_state(world, &open, &done, 1, "pc-d1").await;
    set_state(world, &secret_task, &done, 1, "pc-d2").await;
    let (s, v) = post_as(
        world,
        &admin,
        &format!("/v1/sprints/{sprint_id}/commands/complete"),
        "pc-cp",
        json!({"expectedVersion":2,"archive":"IMMEDIATE"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(archived_at(world, &open).await.is_some());
    assert!(
        archived_at(world, &secret_task).await.is_some(),
        "the Sprint archives it too"
    );
    assert_eq!(
        sees(&admin, &secret_task).await,
        StatusCode::NOT_FOUND,
        "still hidden from the admin"
    );
    let (_, list) = get_as(world, &admin, &format!("/v1/projects/{project_id}/archive")).await;
    assert_eq!(titles(&list), vec!["Open"]);
    let (_, list) = get_as(world, &owner, &format!("/v1/projects/{project_id}/archive")).await;
    assert_eq!(titles(&list).len(), 2, "{list}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_ticket_reopened_while_the_schedule_runs_is_not_archived(pool: PgPool) {
    let world = &World::new(pool).await;
    let (project_id, workflow) = new_project(world, world.workspace_id, "Race").await;
    let (done, doing) = (
        state_id(&workflow, "DONE"),
        state_id(&workflow, "IN_PROGRESS"),
    );
    let owner = world.owner_cookie.clone();
    let (_, sprint, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/sprints"),
        Some(&owner),
        Some("ra-sp"),
        Some(json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"})),
    )
    .await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let task = ticket(
        world,
        &project_id,
        "ra-t",
        json!({"title":"Task","hierarchyKind":"TASK"}),
    )
    .await;
    post_as(
        world,
        &owner,
        &format!("/v1/tickets/{task}/commands/assign-sprint"),
        "ra-as",
        json!({"sprintId":sprint_id}),
    )
    .await;
    post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "ra-st",
        json!({"expectedVersion":1}),
    )
    .await;
    set_state(world, &task, &done, 1, "ra-d").await;
    let at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/complete"),
        "ra-c",
        json!({"expectedVersion":2,"archive":"AT","archiveAt":at}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    sqlx::query("UPDATE penfii.sprint_archive_schedules SET due_at = now() - interval '1 minute' WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&sprint_id).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();

    // Someone reopens the ticket: their transaction holds the ticket's row lock and has not committed yet.
    let task_id = Uuid::parse_str(&task).unwrap();
    let mut reopening = world.pool.begin().await.unwrap();
    sqlx::query("UPDATE penfii.tickets SET status_id=$2 WHERE id=$1")
        .bind(task_id)
        .bind(Uuid::parse_str(&doing).unwrap())
        .execute(&mut *reopening)
        .await
        .unwrap();
    // The schedule runs now and has to wait for that lock.
    let pool = world.pool.clone();
    let run = tokio::spawn(async move { penfii_core::planning::run_due(&pool, 10).await });
    let mut waiting = 0_i64;
    for _ in 0..100 {
        waiting = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND datname = current_database()")
            .fetch_one(&world.pool)
            .await
            .unwrap();
        if waiting > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        waiting > 0,
        "the run never reached the ticket's lock, so the test proves nothing"
    );
    reopening.commit().await.unwrap();
    assert_eq!(run.await.unwrap().unwrap(), 1);

    // It was judged after the reopen, not before: the reopened ticket is left alone and counted as skipped.
    assert!(
        archived_at(world, &task).await.is_none(),
        "a reopened ticket must not be archived"
    );
    let (archived, skipped): (Option<i32>, Option<i32>) = sqlx::query_as("SELECT archived_count, skipped_count FROM penfii.sprint_archive_schedules WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&sprint_id).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!((archived, skipped), (Some(0), Some(1)));
}
