//! Estimates, schedules, calendars and working time through the API.

use super::tickets::{board_json, state_id};
use super::{
    World, call, get_as, grant, make_ticket, new_project, post_as, user_id_of, viewer_cookie,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

// Planning values: estimate, schedule, work calendar, Sprint baseline v2, State history
pub(super) async fn patch_as(
    world: &World,
    cookie: &str,
    path: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, value, _) = call(
        &world.app,
        "PATCH",
        path,
        Some(cookie),
        Some(key),
        Some(body),
    )
    .await;
    (status, value)
}

pub(super) async fn set_unit(world: &World, project_id: &str, unit: &str) -> (StatusCode, Value) {
    patch_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}"),
        "",
        json!({"estimateUnit":unit}),
    )
    .await
}

pub(super) async fn sprint_version(world: &World, project_id: &str, sprint_id: &str) -> i64 {
    let (s, plan) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/planning"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{plan}");
    plan["sprints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == sprint_id)
        .unwrap()["version"]
        .as_i64()
        .unwrap()
}

pub(super) fn week_9_to_18() -> Value {
    let day = json!([{"from":"09:00","to":"18:00"}]);
    json!({"mon":day,"tue":day,"wed":day,"thu":day,"fri":day})
}

pub(super) fn calendar_body(expected: i64, zone: &str, week: Value, holidays: Value) -> Value {
    json!({"expectedVersion":expected,"timeZone":zone,"definition":{"version":1,"week":week,"holidays":holidays}})
}

pub(super) async fn put_calendar(
    world: &World,
    cookie: &str,
    project_id: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, value, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/projects/{project_id}/calendar"),
        Some(cookie),
        None,
        Some(body),
    )
    .await;
    (status, value)
}

async fn last_ticket_event(world: &World, ticket_id: &str, kind: &str) -> Value {
    sqlx::query_scalar("SELECT facts FROM penfii.domain_events WHERE target_kind='TICKET' AND target_id=$1 AND event_kind=$2 ORDER BY seq DESC LIMIT 1")
        .bind(Uuid::parse_str(ticket_id).unwrap()).bind(kind).fetch_one(&world.pool).await.unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_estimate_is_a_task_only_value_in_the_project_unit_and_never_converted(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Estimate rules").await;
    let owner = world.owner_cookie.clone();

    // A Project that does not estimate takes no estimate.
    let (s, v) = make_ticket(
        world,
        &project_id,
        "p71e-none",
        json!({"title":"No unit","estimate":{"value":"5"}}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "estimate_not_allowed");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["estimateUnit"], "POINTS");
    assert_eq!(
        board_json(world, &project_id).await["estimateUnit"],
        "POINTS",
        "the Board tells the screen the unit"
    );

    let (s, task) = make_ticket(
        world,
        &project_id,
        "p71e-task",
        json!({"title":"Estimated","estimate":{"value":"5"}}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{task}");
    assert_eq!(task["estimate"], json!({"value":"5","unit":"POINTS"}));
    let id = task["id"].as_str().unwrap().to_owned();
    let stored: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT estimate_value::text, estimate_unit FROM penfii.tickets WHERE id=$1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(stored, (Some("5.00".into()), Some("POINTS".into())));

    // Only a Task; only in the Project's unit; only a plain decimal.
    let (s, v) = make_ticket(
        world,
        &project_id,
        "p71e-epic",
        json!({"title":"Epic","hierarchyKind":"EPIC","estimate":{"value":"3"}}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("estimate_not_allowed")
        ),
        "{v}"
    );
    let (s, v) = make_ticket(
        world,
        &project_id,
        "p71e-sub",
        json!({"title":"Sub","hierarchyKind":"SUBTASK","parentId":id,"estimate":{"value":"3"}}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("estimate_not_allowed")
        ),
        "{v}"
    );
    let (s, v) = make_ticket(
        world,
        &project_id,
        "p71e-unit",
        json!({"title":"Wrong unit","estimate":{"value":"3","unit":"HOURS"}}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("estimate_unit_mismatch")
        ),
        "{v}"
    );
    for (n, bad) in ["5.123", "-1", "1e3", "", ".5"].iter().enumerate() {
        let (s, v) = make_ticket(
            world,
            &project_id,
            &format!("p71e-bad{n}"),
            json!({"title":"Bad","estimate":{"value":bad}}),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    // Unestimated and an explicit 0 are different things.
    let (s, plain) = make_ticket(
        world,
        &project_id,
        "p71e-plain",
        json!({"title":"Unestimated"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{plain}");
    assert!(plain["estimate"].is_null());
    let plain_id = plain["id"].as_str().unwrap().to_owned();
    let (s, zero) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{plain_id}"),
        "p71e-zero",
        json!({"expectedRevision":1,"estimate":{"value":"0"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{zero}");
    assert_eq!(zero["estimate"], json!({"value":"0","unit":"POINTS"}));
    let (s, cleared) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{plain_id}"),
        "p71e-clear",
        json!({"expectedRevision":2,"estimate":null}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["estimate"].is_null());
    assert_eq!(cleared["revisionNo"], 3);

    // Editing: a new revision, a stale revision never overwrites, the same key replays.
    let (s, r2) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-r2",
        json!({"expectedRevision":1,"estimate":{"value":"8.5"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r2}");
    assert_eq!(r2["estimate"]["value"], "8.5");
    assert_eq!(r2["revisionNo"], 2);
    let (s, again) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-r2",
        json!({"expectedRevision":1,"estimate":{"value":"8.5"}}),
    )
    .await;
    assert_eq!((s, again["revisionNo"].clone()), (StatusCode::OK, json!(2)));
    let (s, stale) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-stale",
        json!({"expectedRevision":1,"estimate":{"value":"1"}}),
    )
    .await;
    assert_eq!(
        (s, stale["code"].as_str()),
        (StatusCode::CONFLICT, Some("version_conflict")),
        "{stale}"
    );
    // Writing the same estimate again changes nothing (no revision).
    let (s, same) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-same",
        json!({"expectedRevision":2,"estimate":{"value":"8.50"}}),
    )
    .await;
    assert_eq!(
        (s, same["revisionNo"].clone()),
        (StatusCode::OK, json!(2)),
        "{same}"
    );
    let (s, r3) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-r3",
        json!({"expectedRevision":2,"estimate":{"value":"13"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r3}");

    // History shows the estimate as a field with before and after.
    let (s, detail) = get_as(world, &owner, &format!("/v1/tickets/{id}/revisions/3")).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["changesFromPrevious"],
        json!([{"field":"estimate","from":["8.5","POINTS"],"to":["13","POINTS"]}])
    );
    // The event a consumer reads carries before and after.
    let facts = last_ticket_event(world, &id, "ticket.updated").await;
    assert_eq!(facts["before"]["estimateValue"], "8.5");
    assert_eq!(facts["after"]["estimateValue"], "13");
    assert_eq!(facts["after"]["estimateUnit"], "POINTS");

    // Restore puts an older estimate back as a new revision.
    let (s, restored) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71e-restore",
        json!({"expectedRevision":3,"sourceRevision":1,"fields":["estimate"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(restored["estimate"]["value"], "5");
    assert_eq!(restored["revisionNo"], 4);
    // A revision from before planning values has no estimate: restoring it clears the estimate.
    sqlx::query("UPDATE penfii.ticket_revisions SET snapshot = snapshot - 'estimateValue' - 'estimateUnit' - 'schedule' WHERE ticket_id=$1 AND revision_no=1").bind(Uuid::parse_str(&id).unwrap()).execute(&world.pool).await.unwrap();
    let (s, legacy) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71e-legacy",
        json!({"expectedRevision":4,"sourceRevision":1,"fields":["estimate"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{legacy}");
    assert!(legacy["estimate"].is_null());
    // Put a value back for the unit checks below.
    let (s, back) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-back",
        json!({"expectedRevision":5,"estimate":{"value":"21"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{back}");

    // The unit: a Sprint keeps it locked while ACTIVE; nothing is converted afterwards.
    let (s, planned) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/sprints"),
        "p71e-sprint",
        json!({"name":"Unit sprint","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{planned}");
    assert_eq!(
        planned["estimateUnit"], "POINTS",
        "a Sprint copies the Project's unit"
    );
    let sprint_id = planned["id"].as_str().unwrap().to_owned();
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/assign-sprint"),
        "p71e-assign",
        json!({"sprintId":sprint_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // A PLANNED Sprint follows a unit change.
    let (s, v) = set_unit(world, &project_id, "HOURS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, plan) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/planning"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{plan}");
    let sprint = plan["sprints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == sprint_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(sprint["estimateUnit"], "HOURS");
    let (s, ticket_now) = get_as(world, &owner, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{ticket_now}");
    assert_eq!(
        ticket_now["estimate"],
        json!({"value":"21","unit":"POINTS"}),
        "an existing estimate keeps its own unit"
    );
    // Restoring a POINTS estimate into an HOURS Project is refused, and History says why.
    let (s, detail) = get_as(world, &owner, &format!("/v1/tickets/{id}/revisions/4")).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    let field = detail["restorable"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["field"] == "estimate")
        .unwrap()
        .clone();
    assert_eq!(
        (field["applicable"].clone(), field["reason"].clone()),
        (json!(false), json!("estimate_unit_changed"))
    );
    let (s, refused) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71e-refuse",
        json!({"expectedRevision":6,"sourceRevision":4,"fields":["estimate"]}),
    )
    .await;
    assert_eq!(
        (s, refused["code"].as_str()),
        (StatusCode::CONFLICT, Some("restore_not_applicable")),
        "{refused}"
    );
    // Writing an estimate now means writing it in HOURS.
    let (s, hours) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71e-hours",
        json!({"expectedRevision":6,"estimate":{"value":"4"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{hours}");
    assert_eq!(hours["estimate"], json!({"value":"4","unit":"HOURS"}));

    let (s, started) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "p71e-start",
        json!({"expectedVersion":sprint["version"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{started}");
    for target in ["POINTS", "NONE"] {
        let (s, v) = set_unit(world, &project_id, target).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::CONFLICT, Some("estimate_unit_locked")),
            "HOURS to {target} while ACTIVE: {v}"
        );
    }
    let (s, v) = set_unit(world, &project_id, "HOURS").await;
    assert_eq!(
        s,
        StatusCode::OK,
        "setting the same unit is not a change: {v}"
    );
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/complete"),
        "p71e-complete",
        json!({"expectedVersion":started["version"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "after the Sprint ended: {v}");
    let (s, v) = set_unit(world, &project_id, "NONE").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // From NONE a unit can always be chosen, even while a Sprint is ACTIVE.
    let (s, s2) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/sprints"),
        "p71e-sprint2",
        json!({"name":"Second","startsOn":"2026-11-16","endsOn":"2026-11-27"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{s2}");
    assert_eq!(s2["estimateUnit"], "NONE");
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{}/commands/start", s2["id"].as_str().unwrap()),
        "p71e-start2",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = set_unit(world, &project_id, "HOURS").await;
    assert_eq!(s, StatusCode::OK, "NONE to a unit is always allowed: {v}");
    let (s, v) = set_unit(world, &project_id, "DAYS").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // Only a Project admin changes the unit; an editor edits estimates.
    let editor = viewer_cookie(world, workspace_id, "p71-editor@penfii.test").await;
    let editor_id = user_id_of(world, "p71-editor@penfii.test").await;
    grant(world, &project_id, editor_id, "EDITOR").await;
    let (s, v) = patch_as(
        world,
        &editor,
        &format!("/v1/projects/{project_id}"),
        "",
        json!({"estimateUnit":"NONE"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, v) = patch_as(
        world,
        &editor,
        &format!("/v1/tickets/{plain_id}"),
        "p71e-editor",
        json!({"expectedRevision":3,"estimate":{"value":"2"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let viewer = viewer_cookie(world, workspace_id, "p71-viewer@penfii.test").await;
    let viewer_id = user_id_of(world, "p71-viewer@penfii.test").await;
    grant(world, &project_id, viewer_id, "VIEWER").await;
    let (s, v) = patch_as(
        world,
        &viewer,
        &format!("/v1/tickets/{plain_id}"),
        "p71e-viewer",
        json!({"expectedRevision":4,"estimate":{"value":"1"}}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_calendar_is_created_on_purpose_by_an_admin_and_never_overwrites_a_newer_version(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Calendar rules").await;
    let owner = world.owner_cookie.clone();

    // No calendar exists until someone makes one; the screen is offered a draft to confirm.
    let (s, view) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/calendar"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{view}");
    assert!(view["calendar"].is_null());
    assert_eq!(view["canManage"], true);
    assert_eq!(view["suggestion"]["timeZone"], "Asia/Seoul");
    assert_eq!(
        view["suggestion"]["definition"]["week"]["mon"],
        json!([{"from":"09:00","to":"18:00"}])
    );
    assert_eq!(view["suggestion"]["definition"]["week"]["sat"], json!([]));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.project_calendars WHERE project_id=$1")
            .bind(Uuid::parse_str(&project_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "reading the draft stores nothing");

    // Invalid definitions are refused: no working time, overlap, order, a day written loosely,
    // an unknown version, an unknown zone, a holiday twice.
    let bad = [
        calendar_body(0, "Asia/Seoul", json!({}), json!([])),
        calendar_body(
            0,
            "Asia/Seoul",
            json!({"mon":[{"from":"09:00","to":"13:00"},{"from":"12:00","to":"18:00"}]}),
            json!([]),
        ),
        calendar_body(
            0,
            "Asia/Seoul",
            json!({"mon":[{"from":"18:00","to":"09:00"}]}),
            json!([]),
        ),
        calendar_body(
            0,
            "Asia/Seoul",
            json!({"mon":[{"from":"9:00","to":"18:00"}]}),
            json!([]),
        ),
        calendar_body(0, "Asia/Nowhere", week_9_to_18(), json!([])),
        calendar_body(
            0,
            "Asia/Seoul",
            week_9_to_18(),
            json!([{"date":"2026-12-25","name":"A"},{"date":"2026-12-25","name":"B"}]),
        ),
        json!({"expectedVersion":0,"timeZone":"Asia/Seoul","definition":{"version":2,"week":{"mon":[{"from":"09:00","to":"18:00"}]}}}),
        json!({"expectedVersion":0,"timeZone":"Asia/Seoul","definition":{"version":1,"week":{"mon":[{"from":"09:00","to":"18:00"}],"funday":[]}}}),
    ];
    for (n, body) in bad.into_iter().enumerate() {
        let (s, v) = put_calendar(world, &owner, &project_id, body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "case {n}: {v}");
    }

    // An editor reads but cannot make or change one.
    let editor = viewer_cookie(world, workspace_id, "p71c-editor@penfii.test").await;
    let editor_id = user_id_of(world, "p71c-editor@penfii.test").await;
    grant(world, &project_id, editor_id, "EDITOR").await;
    let (s, view) = get_as(
        world,
        &editor,
        &format!("/v1/projects/{project_id}/calendar"),
    )
    .await;
    assert_eq!(
        (s, view["canManage"].clone()),
        (StatusCode::OK, json!(false)),
        "{view}"
    );
    let (s, v) = put_calendar(
        world,
        &editor,
        &project_id,
        calendar_body(0, "Asia/Seoul", week_9_to_18(), json!([])),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");

    // Two people creating at once: one wins, the other is told the current version.
    let a = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(0, "Asia/Seoul", week_9_to_18(), json!([])),
    );
    let b = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(0, "Europe/London", week_9_to_18(), json!([])),
    );
    let ((sa, va), (sb, vb)) = tokio::join!(a, b);
    let mut statuses = [sa, sb];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "{va} {vb}"
    );
    let loser = if sa == StatusCode::CONFLICT { &va } else { &vb };
    assert_eq!(
        (loser["code"].clone(), loser["currentVersion"].clone()),
        (json!("calendar_version_conflict"), json!(1))
    );
    let (s, view) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/calendar"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{view}");
    assert_eq!(view["calendar"]["version"], 1);
    assert!(view["suggestion"].is_null());

    // Replacing needs the version that was read; a stale one never overwrites.
    let holidays =
        json!([{"date":"2026-12-25","name":" Christmas "},{"date":"2026-01-01","name":"New year"}]);
    let (s, v) = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(1, "Asia/Seoul", week_9_to_18(), holidays),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["version"], 2);
    assert_eq!(
        v["definition"]["holidays"][0],
        json!({"date":"2026-01-01","name":"New year"}),
        "holidays are sorted"
    );
    assert_eq!(
        v["definition"]["holidays"][1]["name"], "Christmas",
        "names are trimmed"
    );
    let (s, v) = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(1, "Asia/Tokyo", week_9_to_18(), json!([])),
    )
    .await;
    assert_eq!(
        (s, v["currentVersion"].clone()),
        (StatusCode::CONFLICT, json!(2)),
        "{v}"
    );
    let zone: String =
        sqlx::query_scalar("SELECT time_zone FROM penfii.project_calendars WHERE project_id=$1")
            .bind(Uuid::parse_str(&project_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(zone, "Asia/Seoul");
    let (s, v) = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(0, "Asia/Seoul", week_9_to_18(), json!([])),
    )
    .await;
    assert_eq!(
        (s, v["currentVersion"].clone()),
        (StatusCode::CONFLICT, json!(2)),
        "creating again is a conflict: {v}"
    );
    // Another Project's calendar is not reachable through this one.
    let (other, _) = new_project(world, workspace_id, "Calendar other").await;
    let (s, view) = get_as(world, &owner, &format!("/v1/projects/{other}/calendar")).await;
    assert_eq!((s, view["calendar"].is_null()), (StatusCode::OK, true));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_schedule_is_a_ticket_field_and_its_end_is_derived_from_the_current_calendar(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Schedule rules").await;
    let owner = world.owner_cookie.clone();

    let (s, task) = make_ticket(world, &project_id, "p71s-1", json!({"title":"Scheduled","schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":600}})).await;
    assert_eq!(s, StatusCode::CREATED, "{task}");
    let id = task["id"].as_str().unwrap().to_owned();
    assert_eq!(
        task["schedule"],
        json!({"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":600,"isScheduleMilestone":false,"plannedEndAt":null})
    );
    let columns: i64 = sqlx::query_scalar("SELECT count(*) FROM information_schema.columns WHERE table_schema='penfii' AND table_name='ticket_schedules' AND column_name LIKE '%end%'").fetch_one(&world.pool).await.unwrap();
    assert_eq!(columns, 0, "the planned end is not stored");

    // Without a calendar the end cannot be computed; with one it follows the calendar.
    let (s, read) = get_as(world, &owner, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert!(read["schedule"]["plannedEndAt"].is_null());
    let (s, v) = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(0, "Asia/Seoul", week_9_to_18(), json!([])),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, read) = get_as(world, &owner, &format!("/v1/tickets/{id}")).await;
    // 09:00 KST Monday + 600 working minutes = one day (540) + 60 minutes into Tuesday.
    assert_eq!(read["schedule"]["plannedEndAt"], "2026-11-03T01:00:00Z");
    // A status change and a board move return the same derived end. Neither rewrites the schedule.
    let doing = state_id(&workflow, "IN_PROGRESS");
    let waiting = state_id(&workflow, "NOT_STARTED");
    let (s, held) = make_ticket(world, &project_id, "p71s-hold", json!({"title":"Held","schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":600}})).await;
    assert_eq!(s, StatusCode::CREATED, "{held}");
    let held_id = held["id"].as_str().unwrap();
    let (s, changed) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{held_id}/commands/change-status"),
        "p71s-hold-status",
        json!({"expectedRevision":1,"statusId":doing}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{changed}");
    assert_eq!(changed["schedule"]["plannedEndAt"], "2026-11-03T01:00:00Z");
    assert_eq!(changed["schedule"]["durationMinutes"], 600);
    let (s, moved) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{held_id}/commands/move"),
        "p71s-hold-move",
        json!({"expectedRevision":2,"statusId":waiting}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    assert_eq!(moved["schedule"]["plannedEndAt"], "2026-11-03T01:00:00Z");
    let revision_before = read["revisionNo"].clone();
    // A holiday moves the end; changing the calendar writes no Ticket revision.
    let (s, v) = put_calendar(
        world,
        &owner,
        &project_id,
        calendar_body(
            1,
            "Asia/Seoul",
            week_9_to_18(),
            json!([{"date":"2026-11-03","name":"Day off"}]),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, read) = get_as(world, &owner, &format!("/v1/tickets/{id}")).await;
    assert_eq!(read["schedule"]["plannedEndAt"], "2026-11-04T01:00:00Z");
    assert_eq!(read["revisionNo"], revision_before);
    // A list leaves the derived end empty.
    let board = board_json(world, &project_id).await;
    let in_board = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().clone())
        .find(|t| t["id"] == id.as_str())
        .unwrap();
    assert_eq!(in_board["schedule"]["durationMinutes"], 600);
    assert!(in_board["schedule"]["plannedEndAt"].is_null());

    // Starting on a Friday afternoon, over the weekend; a holiday on Monday pushes it again.
    let (s, v) = patch_as(world, &owner, &format!("/v1/tickets/{id}"), "p71s-r2", json!({"expectedRevision":1,"schedule":{"plannedStartAt":"2026-11-06T06:00:00Z","durationMinutes":240}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["schedule"]["plannedEndAt"], "2026-11-09T01:00:00Z");
    // A zero-duration schedule node is placed at the next working instant.
    let (s, node) = make_ticket(world, &project_id, "p71s-2", json!({"title":"Node","schedule":{"plannedStartAt":"2026-11-07T03:00:00Z","durationMinutes":0,"isScheduleMilestone":true}})).await;
    assert_eq!(s, StatusCode::CREATED, "{node}");
    assert_eq!(node["schedule"]["plannedEndAt"], "2026-11-09T00:00:00Z");
    assert_eq!(node["schedule"]["isScheduleMilestone"], true);
    let milestones: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.milestones WHERE project_id=$1")
            .bind(Uuid::parse_str(&project_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(milestones, 0, "a schedule node is not a Project Milestone");

    // The rules: zero only for a schedule node, never negative, nothing absurd.
    let node_id = node["id"].as_str().unwrap().to_owned();
    for (n, schedule) in [
        json!({"durationMinutes":0}),
        json!({"durationMinutes":-5}),
        json!({"durationMinutes":9_999_999_999_i64}),
        json!({"isScheduleMilestone":true,"durationMinutes":30}),
        json!({"plannedStartAt":"1999-12-31T00:00:00Z"}),
    ]
    .into_iter()
    .enumerate()
    {
        let (s, v) = patch_as(
            world,
            &owner,
            &format!("/v1/tickets/{node_id}"),
            &format!("p71s-bad{n}"),
            json!({"expectedRevision":1,"schedule":schedule}),
        )
        .await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("schedule_invalid")),
            "case {n}: {v}"
        );
    }
    // plannedEndAt sent back by a client is ignored, not stored and not an error.
    let (s, v) = patch_as(world, &owner, &format!("/v1/tickets/{node_id}"), "p71s-echo", json!({"expectedRevision":1,"schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":60,"plannedEndAt":"2030-01-01T00:00:00Z"}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["schedule"]["plannedEndAt"], "2026-11-02T01:00:00Z");

    // History shows the schedule as a field and Restore puts an older one back.
    let (s, detail) = get_as(world, &owner, &format!("/v1/tickets/{id}/revisions/2")).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    assert_eq!(detail["changesFromPrevious"][0]["field"], "schedule");
    assert_eq!(
        detail["changesFromPrevious"][0]["from"],
        json!(["2026-11-02T00:00:00Z", "600", ""])
    );
    assert_eq!(
        detail["changesFromPrevious"][0]["to"],
        json!(["2026-11-06T06:00:00Z", "240", ""])
    );
    let facts = last_ticket_event(world, &id, "ticket.updated").await;
    assert_eq!(facts["before"]["schedule"]["durationMinutes"], 600);
    assert_eq!(facts["after"]["schedule"]["durationMinutes"], 240);
    let (s, restored) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71s-restore",
        json!({"expectedRevision":2,"sourceRevision":1,"fields":["schedule"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(restored["schedule"]["durationMinutes"], 600);
    assert_eq!(restored["revisionNo"], 3);
    // Clearing removes it; the same edit again is not a change.
    let (s, cleared) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71s-clear",
        json!({"expectedRevision":3,"schedule":null}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["schedule"].is_null());
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.ticket_schedules WHERE ticket_id=$1")
            .bind(Uuid::parse_str(&id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(rows, 0);
    let (s, same) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71s-clear2",
        json!({"expectedRevision":4,"schedule":null}),
    )
    .await;
    assert_eq!(
        (s, same["revisionNo"].clone()),
        (StatusCode::OK, json!(4)),
        "{same}"
    );
    // A schedule needs the Ticket's edit right.
    let viewer = viewer_cookie(world, workspace_id, "p71s-viewer@penfii.test").await;
    let viewer_id = user_id_of(world, "p71s-viewer@penfii.test").await;
    grant(world, &project_id, viewer_id, "VIEWER").await;
    let (s, v) = patch_as(
        world,
        &viewer,
        &format!("/v1/tickets/{id}"),
        "p71s-viewer",
        json!({"expectedRevision":4,"schedule":{"durationMinutes":60}}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");

    // Beyond the five-year horizon there is no derived end rather than a wrong one.
    let (far, _) = new_project(world, workspace_id, "Schedule horizon").await;
    let (s, v) = put_calendar(
        world,
        &owner,
        &far,
        calendar_body(
            0,
            "Asia/Seoul",
            json!({"mon":[{"from":"09:00","to":"09:01"}]}),
            json!([]),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, within) = make_ticket(world, &far, "p71s-far1", json!({"title":"Within","schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":200}})).await;
    assert_eq!(s, StatusCode::CREATED, "{within}");
    assert!(
        within["schedule"]["plannedEndAt"].is_string(),
        "200 Mondays fit in five years: {within}"
    );
    let (s, beyond) = make_ticket(world, &far, "p71s-far2", json!({"title":"Beyond","schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":5000}})).await;
    assert_eq!(s, StatusCode::CREATED, "{beyond}");
    assert!(beyond["schedule"]["plannedEndAt"].is_null(), "{beyond}");
}

fn intervals_json(list: &[penfii_core::calendar::WorkInterval]) -> Vec<(String, String)> {
    list.iter()
        .map(|i| (i.start.to_rfc3339(), i.end.to_rfc3339()))
        .collect()
}

async fn expand_week(
    world: &World,
    zone: &str,
    week: Value,
    holidays: Value,
    first: &str,
    last: &str,
) -> Vec<penfii_core::calendar::WorkInterval> {
    let definition: penfii_contracts::CalendarDefinition =
        serde_json::from_value(json!({"version":1,"week":week,"holidays":holidays})).unwrap();
    let definition = penfii_core::calendar::normalize(definition).unwrap();
    let mut conn = world.pool.acquire().await.unwrap();
    penfii_core::calendar::expand(
        &mut conn,
        zone,
        &definition,
        first.parse().unwrap(),
        last.parse().unwrap(),
    )
    .await
    .unwrap()
}

fn minutes(i: &penfii_core::calendar::WorkInterval) -> i64 {
    (i.end - i.start).num_minutes()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn working_time_is_elapsed_time_between_instants_so_daylight_saving_changes_a_working_day(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let night = json!([{"from":"00:00","to":"06:00"}]);

    // America/New_York: the clocks skip 02:00-03:00 on 2026-03-08 and repeat 01:00-02:00 on 2026-11-01.
    let spring = expand_week(
        world,
        "America/New_York",
        json!({"sat":night,"sun":night}),
        json!([]),
        "2026-03-07",
        "2026-03-10",
    )
    .await;
    assert_eq!(
        spring.iter().map(minutes).collect::<Vec<_>>(),
        vec![360, 300],
        "Saturday is 6 h, the spring-forward Sunday 5 h"
    );
    assert_eq!(spring[1].start.to_rfc3339(), "2026-03-08T05:00:00+00:00");
    assert_eq!(spring[1].end.to_rfc3339(), "2026-03-08T10:00:00+00:00");
    let autumn = expand_week(
        world,
        "America/New_York",
        json!({"sat":night,"sun":night}),
        json!([]),
        "2026-10-31",
        "2026-11-03",
    )
    .await;
    assert_eq!(
        autumn.iter().map(minutes).collect::<Vec<_>>(),
        vec![360, 420],
        "the fall-back Sunday is 7 h"
    );
    // An interval that lies wholly in the skipped hour disappears instead of becoming zero-length work.
    let skipped = expand_week(
        world,
        "America/New_York",
        json!({"sun":[{"from":"02:00","to":"03:00"},{"from":"09:00","to":"10:00"}]}),
        json!([]),
        "2026-03-08",
        "2026-03-09",
    )
    .await;
    assert_eq!(skipped.iter().map(minutes).collect::<Vec<_>>(), vec![60]);
    // Europe/London changes on 2026-03-29 and 2026-10-25; Lord Howe moves by thirty minutes.
    let london = expand_week(
        world,
        "Europe/London",
        json!({"sun":night}),
        json!([]),
        "2026-03-29",
        "2026-03-30",
    )
    .await;
    assert_eq!(london.iter().map(minutes).collect::<Vec<_>>(), vec![300]);
    let london_back = expand_week(
        world,
        "Europe/London",
        json!({"sun":night}),
        json!([]),
        "2026-10-25",
        "2026-10-26",
    )
    .await;
    assert_eq!(
        london_back.iter().map(minutes).collect::<Vec<_>>(),
        vec![420]
    );
    let howe_end = expand_week(
        world,
        "Australia/Lord_Howe",
        json!({"sun":night}),
        json!([]),
        "2026-04-05",
        "2026-04-06",
    )
    .await;
    assert_eq!(
        howe_end.iter().map(minutes).collect::<Vec<_>>(),
        vec![390],
        "DST ends: 30 minutes are repeated"
    );
    let howe_start = expand_week(
        world,
        "Australia/Lord_Howe",
        json!({"sun":night}),
        json!([]),
        "2026-10-04",
        "2026-10-05",
    )
    .await;
    assert_eq!(
        howe_start.iter().map(minutes).collect::<Vec<_>>(),
        vec![330],
        "DST starts: 30 minutes are skipped"
    );
    // Seoul has no daylight saving: every working day is the same length.
    let seoul = expand_week(
        world,
        "Asia/Seoul",
        week_9_to_18(),
        json!([]),
        "2026-03-02",
        "2026-03-16",
    )
    .await;
    assert_eq!(seoul.len(), 10);
    assert!(seoul.iter().all(|i| minutes(i) == 540));
    assert_eq!(seoul[0].start.to_rfc3339(), "2026-03-02T00:00:00+00:00");

    // Weekends, holidays and a lunch break are not working time; 24:00 ends at midnight.
    let lunch = json!([{"from":"09:00","to":"12:00"},{"from":"13:00","to":"18:00"}]);
    let with_holiday = expand_week(
        world,
        "Asia/Seoul",
        json!({"mon":lunch,"tue":lunch,"wed":lunch,"thu":lunch,"fri":lunch}),
        json!([{"date":"2026-11-04","name":"Wednesday off"}]),
        "2026-11-02",
        "2026-11-09",
    )
    .await;
    assert_eq!(
        intervals_json(&with_holiday).len(),
        8,
        "4 days x 2 intervals"
    );
    assert!(
        !with_holiday
            .iter()
            .any(|i| i.start.to_rfc3339().starts_with("2026-11-04")
                || i.start.to_rfc3339().starts_with("2026-11-03T16"))
    );
    let late = expand_week(
        world,
        "Asia/Seoul",
        json!({"sat":[{"from":"20:00","to":"24:00"}]}),
        json!([]),
        "2026-11-07",
        "2026-11-08",
    )
    .await;
    assert_eq!(
        late[0].end.to_rfc3339(),
        "2026-11-07T15:00:00+00:00",
        "24:00 Saturday is Sunday 00:00 KST"
    );
    assert_eq!(minutes(&late[0]), 240);

    // Counting across the change: 8 working hours from Saturday midnight in New York span the
    // Sunday that is one hour short.
    let around = expand_week(
        world,
        "America/New_York",
        json!({"sat":[{"from":"00:00","to":"24:00"}],"sun":[{"from":"00:00","to":"24:00"}]}),
        json!([]),
        "2026-03-07",
        "2026-03-09",
    )
    .await;
    assert_eq!(
        around.iter().map(minutes).collect::<Vec<_>>(),
        vec![1440, 1380],
        "a 23-hour day"
    );
    let start = "2026-03-07T20:00:00Z".parse().unwrap();
    // From Saturday 15:00 local: 9 h left on Saturday, then Sunday's 23 h.
    let end = penfii_core::calendar::add_working_minutes(&around, start, 9 * 60 + 23 * 60).unwrap();
    assert_eq!(
        end.to_rfc3339(),
        "2026-03-09T04:00:00+00:00",
        "ends at Sunday midnight local (00:00 EDT)"
    );
    let end = penfii_core::calendar::add_working_minutes(&around, start, 9 * 60 + 23 * 60 + 1);
    assert!(end.is_none(), "the list holds no more working time");

    // An unknown zone is refused by the database rather than guessed.
    let definition: penfii_contracts::CalendarDefinition =
        serde_json::from_value(json!({"version":1,"week":week_9_to_18()})).unwrap();
    let mut conn = world.pool.acquire().await.unwrap();
    let bad = penfii_core::calendar::expand(
        &mut conn,
        "Mars/Olympus",
        &definition,
        "2026-11-02".parse().unwrap(),
        "2026-11-03".parse().unwrap(),
    )
    .await;
    assert!(bad.is_err());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_state_keeps_the_history_of_its_category_and_the_past_is_not_reread_from_the_workflow(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "State history").await;
    let owner = world.owner_cookie.clone();
    let pid = Uuid::parse_str(&project_id).unwrap();
    let history = |status: String| {
        let pool = world.pool.clone();
        async move {
            sqlx::query_as::<_, (String,)>("SELECT category FROM penfii.workflow_status_history WHERE status_id=$1 ORDER BY effective_from, id")
                .bind(Uuid::parse_str(&status).unwrap()).fetch_all(&pool).await.unwrap().into_iter().map(|r| r.0).collect::<Vec<_>>()
        }
    };
    // A new Project starts every State with one row.
    for state in workflow["states"].as_array().unwrap() {
        assert_eq!(
            history(state["id"].as_str().unwrap().to_owned()).await,
            vec![state["category"].as_str().unwrap().to_owned()],
            "{state}"
        );
    }
    let version = workflow["version"].as_i64().unwrap();
    let (s, created, _) = call(&world.app, "POST", &format!("/v1/projects/{project_id}/workflow/states"), Some(&owner), None, Some(json!({"expectedWorkflowVersion":version,"name":"Review","category":"IN_PROGRESS","position":5}))).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let review = created["state"]["id"]
        .as_str()
        .or(created["id"].as_str())
        .unwrap()
        .to_owned();
    assert_eq!(history(review.clone()).await, vec!["IN_PROGRESS"]);
    let version = created["workflowVersion"]
        .as_i64()
        .or(created["version"].as_i64())
        .unwrap();
    // A rename adds no row; a category change on a State no Ticket uses adds one, in order.
    let (s, renamed, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{review}"),
        Some(&owner),
        None,
        Some(json!({"expectedWorkflowVersion":version,"name":"Code review"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{renamed}");
    assert_eq!(history(review.clone()).await, vec!["IN_PROGRESS"]);
    let version = renamed["workflowVersion"]
        .as_i64()
        .or(renamed["version"].as_i64())
        .unwrap();
    let (s, changed, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{review}"),
        Some(&owner),
        None,
        Some(json!({"expectedWorkflowVersion":version,"category":"NOT_STARTED"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{changed}");
    assert_eq!(
        history(review.clone()).await,
        vec!["IN_PROGRESS", "NOT_STARTED"]
    );
    // A refused change leaves no row.
    let (_, ticket) = make_ticket(
        world,
        &project_id,
        "p71h-1",
        json!({"title":"Uses a State"}),
    )
    .await;
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/tickets/{}/commands/change-status",
            ticket["id"].as_str().unwrap()
        ),
        Some(&owner),
        Some("p71h-move"),
        Some(json!({"expectedRevision":1,"statusId":review})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let version = changed["workflowVersion"]
        .as_i64()
        .or(changed["version"].as_i64())
        .unwrap();
    let (s, v, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/projects/{project_id}/workflow/states/{review}"),
        Some(&owner),
        None,
        Some(json!({"expectedWorkflowVersion":version,"category":"DONE"})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("state_in_use")),
        "{v}"
    );
    assert_eq!(
        history(review.clone()).await,
        vec!["IN_PROGRESS", "NOT_STARTED"]
    );
    // The revision of that move records the category it had then.
    let snapshot: Value = sqlx::query_scalar(
        "SELECT snapshot FROM penfii.ticket_revisions WHERE ticket_id=$1 AND revision_no=2",
    )
    .bind(Uuid::parse_str(ticket["id"].as_str().unwrap()).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(snapshot["statusCategory"], "NOT_STARTED");
    let _ = pid;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_sprint_start_records_baseline_v2_once_and_every_end_records_what_the_task_was(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Baseline v2").await;
    let owner = world.owner_cookie.clone();
    let done = state_id(&workflow, "DONE");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // A Sprint takes the Workspace zone and the Project unit unless told otherwise.
    let (s, plain) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/sprints"),
        "p71b-plain",
        json!({"name":"Defaults","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{plain}");
    assert_eq!(
        (plain["timeZone"].clone(), plain["estimateUnit"].clone()),
        (json!("Asia/Seoul"), json!("POINTS"))
    );
    let (s, picked) = post_as(world, &owner, &format!("/v1/projects/{project_id}/sprints"), "p71b-picked", json!({"name":"Chosen","startsOn":"2026-11-02","endsOn":"2026-11-13","timeZone":"Europe/London","estimateUnit":"HOURS"})).await;
    assert_eq!(s, StatusCode::CREATED, "{picked}");
    assert_eq!(
        (picked["timeZone"].clone(), picked["estimateUnit"].clone()),
        (json!("Europe/London"), json!("HOURS"))
    );
    for (n, extra) in [
        json!({"timeZone":"Mars/Olympus"}),
        json!({"estimateUnit":"DAYS"}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut body = json!({"name":"Bad","startsOn":"2026-11-02","endsOn":"2026-11-13"});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (s, v) = post_as(
            world,
            &owner,
            &format!("/v1/projects/{project_id}/sprints"),
            &format!("p71b-bad{n}"),
            body,
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    // While PLANNED they can change; once started they are fixed.
    let sprint_id = plain["id"].as_str().unwrap().to_owned();
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}"),
        "",
        json!({"expectedVersion":1,"timeZone":"America/New_York"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["timeZone"], "America/New_York");

    let mk = |n: u32, body: Value| {
        let project_id = project_id.clone();
        async move {
            let (s, t) = make_ticket(world, &project_id, &format!("p71b-t{n}"), body).await;
            assert_eq!(s, StatusCode::CREATED, "{t}");
            t["id"].as_str().unwrap().to_owned()
        }
    };
    let a = mk(1, json!({"title":"A estimated 3","estimate":{"value":"3"}})).await;
    let b = mk(
        2,
        json!({"title":"B estimated 5, done before the start","estimate":{"value":"5"}}),
    )
    .await;
    let c = mk(3, json!({"title":"C unestimated"})).await;
    let d = mk(4, json!({"title":"D added later","estimate":{"value":"2"}})).await;
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{b}/commands/change-status"),
        "p71b-done-b",
        json!({"expectedRevision":1,"statusId":done}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for (n, id) in [&a, &b, &c].into_iter().enumerate() {
        let (s, v) = post_as(
            world,
            &owner,
            &format!("/v1/tickets/{id}/commands/assign-sprint"),
            &format!("p71b-as{n}"),
            json!({"sprintId":sprint_id}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (s, started) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "p71b-start",
        json!({"expectedVersion":sprint_version(world, &project_id, &sprint_id).await}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{started}");

    let sid = Uuid::parse_str(&sprint_id).unwrap();
    let header: (i16, Option<String>, Option<String>, Option<i16>, i32) = sqlx::query_as("SELECT snapshot_version, estimate_unit, time_zone, policy_version, task_count FROM penfii.sprint_baselines WHERE sprint_id=$1").bind(sid).fetch_one(&world.pool).await.unwrap();
    assert_eq!(
        header,
        (
            2,
            Some("POINTS".into()),
            Some("America/New_York".into()),
            Some(1),
            3
        )
    );
    type Row = (
        Uuid,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let read_baseline = || async {
        sqlx::query_as::<_, Row>("SELECT ticket_id, ticket_revision_no, status_category, estimate_value::text, estimate_unit, hierarchy_kind FROM penfii.sprint_baseline_tickets WHERE sprint_id=$1 ORDER BY ticket_id")
            .bind(sid).fetch_all(&world.pool).await.unwrap()
    };
    let baseline = read_baseline().await;
    let find = |id: &str| {
        baseline
            .iter()
            .find(|r| r.0 == Uuid::parse_str(id).unwrap())
            .unwrap()
            .clone()
    };
    assert_eq!(
        find(&a),
        (
            Uuid::parse_str(&a).unwrap(),
            Some(1),
            Some("NOT_STARTED".into()),
            Some("3.00".into()),
            Some("POINTS".into()),
            Some("TASK".into())
        )
    );
    assert_eq!(
        find(&b).2.as_deref(),
        Some("DONE"),
        "done before the start is recorded as such"
    );
    assert_eq!(find(&b).1, Some(2));
    assert_eq!(find(&c).3, None, "unestimated stays unestimated");

    // Later edits, moves and additions never touch the baseline.
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{a}"),
        "p71b-a-est",
        json!({"expectedRevision":1,"estimate":{"value":"9"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{a}/commands/change-status"),
        "p71b-a-done",
        json!({"expectedRevision":2,"statusId":done}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{d}/commands/assign-sprint"),
        "p71b-as-d",
        json!({"sprintId":sprint_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(read_baseline().await, baseline);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.sprint_baselines WHERE sprint_id=$1")
            .bind(sid)
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);

    // Fixed at start: a change is refused, the same values are not a change.
    let (s, v) = patch_as(world, &owner, &format!("/v1/sprints/{sprint_id}"), "", json!({"expectedVersion":sprint_version(world, &project_id, &sprint_id).await,"timeZone":"Asia/Seoul"})).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("invalid_sprint_state")),
        "{v}"
    );
    let (s, v) = patch_as(world, &owner, &format!("/v1/sprints/{sprint_id}"), "", json!({"expectedVersion":sprint_version(world, &project_id, &sprint_id).await,"estimateUnit":"HOURS"})).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("invalid_sprint_state")),
        "{v}"
    );
    let (s, v) = patch_as(world, &owner, &format!("/v1/sprints/{sprint_id}"), "", json!({"expectedVersion":sprint_version(world, &project_id, &sprint_id).await,"name":"Renamed","timeZone":"America/New_York","estimateUnit":"POINTS"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // A Task taken out in the middle records what it was at that moment.
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{c}/commands/assign-sprint"),
        "p71b-out-c",
        json!({"sprintId":null}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    type End = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let end_of = |id: String| async move {
        sqlx::query_as::<_, End>("SELECT end_reason, ended_status_category, ended_estimate_value::text, ended_estimate_unit FROM penfii.sprint_assignments WHERE ticket_id=$1 AND sprint_id=$2")
            .bind(Uuid::parse_str(&id).unwrap()).bind(sid).fetch_one(&world.pool).await.unwrap()
    };
    assert_eq!(
        end_of(c.clone()).await,
        (
            Some("REMOVED".into()),
            Some("NOT_STARTED".into()),
            None,
            None
        )
    );
    // Closing records every remaining Task's category and estimate once.
    let (s, closed) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/complete"),
        "p71b-close",
        json!({"expectedVersion":sprint_version(world, &project_id, &sprint_id).await}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{closed}");
    assert_eq!(
        end_of(a.clone()).await,
        (
            Some("SPRINT_COMPLETED".into()),
            Some("DONE".into()),
            Some("9.00".into()),
            Some("POINTS".into())
        )
    );
    assert_eq!(
        end_of(d.clone()).await,
        (
            Some("SPRINT_COMPLETED".into()),
            Some("NOT_STARTED".into()),
            Some("2.00".into()),
            Some("POINTS".into())
        )
    );
    // Edits after the close do not reach the closed Sprint.
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{a}"),
        "p71b-a-late",
        json!({"expectedRevision":3,"estimate":{"value":"1"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(end_of(a.clone()).await.2.as_deref(), Some("9.00"));
    assert_eq!(read_baseline().await, baseline);

    // Cancel records the end too; carry-over is a new assignment with its own, later, end.
    let (s, third) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/sprints"),
        "p71b-third",
        json!({"name":"Canceled","startsOn":"2026-11-16","endsOn":"2026-11-27"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{third}");
    let third_id = third["id"].as_str().unwrap().to_owned();
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{d}/commands/assign-sprint"),
        "p71b-as-d3",
        json!({"sprintId":third_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{third_id}/commands/cancel"),
        "p71b-cancel",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let cancel_end: End = sqlx::query_as("SELECT end_reason, ended_status_category, ended_estimate_value::text, ended_estimate_unit FROM penfii.sprint_assignments WHERE ticket_id=$1 AND sprint_id=$2").bind(Uuid::parse_str(&d).unwrap()).bind(Uuid::parse_str(&third_id).unwrap()).fetch_one(&world.pool).await.unwrap();
    assert_eq!(
        cancel_end,
        (
            Some("SPRINT_CANCELED".into()),
            Some("NOT_STARTED".into()),
            Some("2.00".into()),
            Some("POINTS".into())
        )
    );
    let no_baseline: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.sprint_baselines WHERE sprint_id=$1")
            .bind(Uuid::parse_str(&third_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(
        no_baseline, 0,
        "a Sprint that never started has no baseline"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_view_can_filter_sort_and_show_the_estimate_but_only_compares_inside_one_project(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Estimate view").await;
    let owner = world.owner_cookie.clone();
    let path = format!("/v1/projects/{project_id}/views");
    let (s, view) = post_as(world, &owner, &path, "p71v-1", json!({"name":"Big","layout":"TABLE","definition":{"filter":{"estimate":{"op":"GTE","value":"5.50"}},"sort":{"field":"ESTIMATE","dir":"DESC"},"display":{"columns":["KEY","TITLE","ESTIMATE"]}}})).await;
    assert_eq!(s, StatusCode::CREATED, "{view}");
    assert_eq!(
        view["definition"]["filter"]["estimate"],
        json!({"op":"GTE","value":"5.5"})
    );
    assert_eq!(view["definition"]["sort"]["field"], "ESTIMATE");
    for (n, op) in [
        json!({"op":"SET"}),
        json!({"op":"UNSET"}),
        json!({"op":"LTE","value":"3"}),
    ]
    .into_iter()
    .enumerate()
    {
        let (s, v) = post_as(
            world,
            &owner,
            &path,
            &format!("p71v-ok{n}"),
            json!({"name":format!("Ok {n}"),"definition":{"filter":{"estimate":op}}}),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    for (n, op) in [
        json!({"op":"GTE"}),
        json!({"op":"SET","value":"1"}),
        json!({"op":"EQ","value":"1"}),
        json!({"op":"GTE","value":"x"}),
        json!({"op":"SET","extra":1}),
    ]
    .into_iter()
    .enumerate()
    {
        let (s, v) = post_as(
            world,
            &owner,
            &path,
            &format!("p71v-bad{n}"),
            json!({"name":format!("Bad {n}"),"definition":{"filter":{"estimate":op}}}),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "case {n}: {v}");
    }
    // Units differ between Projects: a comparison across Projects is refused, presence is not.
    let wide = format!("/v1/workspaces/{workspace_id}/views");
    let (s, v) = post_as(world, &owner, &wide, "p71v-w1", json!({"targetKind":"WORKSPACE","name":"Wide compare","layout":"LIST","definition":{"group":"CATEGORY","filter":{"estimate":{"op":"GTE","value":"5"}}}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = post_as(world, &owner, &wide, "p71v-w2", json!({"targetKind":"WORKSPACE","name":"Wide unestimated","layout":"LIST","definition":{"group":"CATEGORY","filter":{"estimate":{"op":"UNSET"}}}})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn restore_skips_only_an_estimate_in_another_unit_and_restores_the_rest(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Restore skips").await;
    let owner = world.owner_cookie.clone();
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, task) = make_ticket(
        world,
        &project_id,
        "p71r-1",
        json!({"title":"Old title","estimate":{"value":"5"}}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{task}");
    let id = task["id"].as_str().unwrap().to_owned();
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71r-2",
        json!({"expectedRevision":1,"title":"New title","estimate":{"value":"9"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = set_unit(world, &project_id, "HOURS").await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // History says which field cannot come back and why; the others can.
    let (s, detail) = get_as(world, &owner, &format!("/v1/tickets/{id}/revisions/1")).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    let restorable = detail["restorable"].as_array().unwrap();
    let of = |field: &str| {
        restorable
            .iter()
            .find(|r| r["field"] == field)
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            of("title")["applicable"].clone(),
            of("estimate")["applicable"].clone(),
            of("estimate")["reason"].clone()
        ),
        (json!(true), json!(false), json!("estimate_unit_changed"))
    );

    // Title and estimate chosen together: the title comes back, the estimate stays as it is.
    let (s, restored) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71r-3",
        json!({"expectedRevision":2,"sourceRevision":1,"fields":["title","estimate"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(restored["title"], "Old title");
    assert_eq!(
        restored["estimate"],
        json!({"value":"9","unit":"POINTS"}),
        "never converted, never overwritten"
    );
    assert_eq!(restored["revisionNo"], 3);
    let metadata: Value = sqlx::query_scalar(
        "SELECT restore_metadata FROM penfii.ticket_revisions WHERE ticket_id=$1 AND revision_no=3",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(metadata["applied"], json!(["title"]));
    assert_eq!(
        metadata["skipped"],
        json!([{"field":"estimate","reason":"estimate_unit_changed"}])
    );
    let facts = last_ticket_event(world, &id, "ticket.restored").await;
    assert_eq!(facts["fields"], json!(["title"]));
    assert_eq!(facts["skipped"][0]["field"], "estimate");
    // The same request again is a replay, not a second revision.
    let (s, again) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71r-3",
        json!({"expectedRevision":2,"sourceRevision":1,"fields":["title","estimate"]}),
    )
    .await;
    assert_eq!((s, again["revisionNo"].clone()), (StatusCode::OK, json!(3)));

    // Only the estimate chosen: nothing is left to restore, so it is refused with the reason.
    let (s, none) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71r-4",
        json!({"expectedRevision":3,"sourceRevision":1,"fields":["estimate"]}),
    )
    .await;
    assert_eq!(
        (s, none["code"].as_str()),
        (StatusCode::CONFLICT, Some("restore_not_applicable")),
        "{none}"
    );
    assert_eq!(none["fields"][0]["reason"], "estimate_unit_changed");
    // Any other field that cannot come back still refuses the whole restore (unchanged rule).
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71r-5",
        json!({"expectedRevision":3,"assigneeId":world.owner_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}"),
        "p71r-6",
        json!({"expectedRevision":4,"assigneeId":null}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    sqlx::query("UPDATE penfii.ticket_revisions SET snapshot = jsonb_set(snapshot,'{assigneeId}',to_jsonb($2::text)) WHERE ticket_id=$1 AND revision_no=4").bind(Uuid::parse_str(&id).unwrap()).bind(Uuid::new_v4().to_string()).execute(&world.pool).await.unwrap();
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/tickets/{id}/commands/restore"),
        "p71r-7",
        json!({"expectedRevision":5,"sourceRevision":4,"fields":["assignee","estimate"]}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("restore_not_applicable")),
        "{v}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_planned_sprint_changes_its_zone_and_unit_through_the_api_and_an_active_one_does_not(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Planned sprint basis").await;
    let owner = world.owner_cookie.clone();
    let (s, sprint) = post_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/sprints"),
        "p71q-1",
        json!({"name":"Planned","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sprint}");
    let id = sprint["id"].as_str().unwrap().to_owned();
    assert_eq!(
        (sprint["timeZone"].clone(), sprint["estimateUnit"].clone()),
        (json!("Asia/Seoul"), json!("NONE"))
    );
    // Both change together or one at a time while PLANNED; each change is a new version.
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/sprints/{id}"),
        "",
        json!({"expectedVersion":1,"estimateUnit":"HOURS"}),
    )
    .await;
    assert_eq!(
        (
            s,
            v["estimateUnit"].clone(),
            v["timeZone"].clone(),
            v["version"].clone()
        ),
        (
            StatusCode::OK,
            json!("HOURS"),
            json!("Asia/Seoul"),
            json!(2)
        ),
        "{v}"
    );
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/sprints/{id}"),
        "",
        json!({"expectedVersion":2,"timeZone":"Europe/London","estimateUnit":"POINTS"}),
    )
    .await;
    assert_eq!(
        (
            s,
            v["estimateUnit"].clone(),
            v["timeZone"].clone(),
            v["version"].clone()
        ),
        (
            StatusCode::OK,
            json!("POINTS"),
            json!("Europe/London"),
            json!(3)
        ),
        "{v}"
    );
    for (n, body) in [
        json!({"expectedVersion":3,"estimateUnit":"DAYS"}),
        json!({"expectedVersion":3,"timeZone":"Mars/Olympus"}),
    ]
    .into_iter()
    .enumerate()
    {
        let (s, v) = patch_as(world, &owner, &format!("/v1/sprints/{id}"), "", body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "case {n}: {v}");
    }
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/sprints/{id}"),
        "",
        json!({"expectedVersion":1,"timeZone":"Asia/Seoul"}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("version_conflict")),
        "a stale version never overwrites: {v}"
    );
    // The baseline of the Sprint that starts carries what the Sprint was last set to.
    let (s, v) = post_as(
        world,
        &owner,
        &format!("/v1/sprints/{id}/commands/start"),
        "p71q-start",
        json!({"expectedVersion":3}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let header: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT estimate_unit, time_zone FROM penfii.sprint_baselines WHERE sprint_id=$1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(
        header,
        (Some("POINTS".into()), Some("Europe/London".into()))
    );
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/sprints/{id}"),
        "",
        json!({"expectedVersion":4,"estimateUnit":"HOURS"}),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("invalid_sprint_state")),
        "{v}"
    );
}
