//! Sprint reports, velocity, status summaries and the Gantt list through the API.

use super::estimates::{
    calendar_body, patch_as, put_calendar, set_unit, sprint_version, week_9_to_18,
};
use super::tickets::state_id;
use super::{
    World, call, get_as, grant, make_ticket, new_project, post_as, user_id_of, viewer_cookie,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

// Reports
async fn report_of(world: &World, cookie: &str, sprint_id: &str) -> Value {
    let (s, v) = get_as(world, cookie, &format!("/v1/sprints/{sprint_id}/report")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn estimated_ticket(
    world: &World,
    project_id: &str,
    key: &str,
    title: &str,
    estimate: Option<&str>,
) -> String {
    let mut body = json!({"title":title});
    if let Some(value) = estimate {
        body["estimate"] = json!({"value":value});
    }
    let (s, t) = make_ticket(world, project_id, key, body).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_owned()
}

async fn assign_to(world: &World, ticket: &str, sprint: Option<&str>, key: &str) {
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{ticket}/commands/assign-sprint"),
        key,
        json!({"sprintId":sprint}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn move_to(world: &World, ticket: &str, state: &str, revision: i64, key: &str) {
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{ticket}/commands/change-status"),
        key,
        json!({"expectedRevision":revision,"statusId":state}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn new_sprint(world: &World, project_id: &str, name: &str, key: &str) -> (String, i64) {
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/sprints"),
        key,
        json!({"name":name,"startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (
        v["id"].as_str().unwrap().to_owned(),
        v["version"].as_i64().unwrap(),
    )
}

async fn sprint_command_as_owner(
    world: &World,
    project_id: &str,
    sprint: &str,
    command: &str,
    key: &str,
) -> Value {
    let version = sprint_version(world, project_id, sprint).await;
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/sprints/{sprint}/commands/{command}"),
        key,
        json!({"expectedVersion":version}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn amount(value: &Value) -> (String, i64, i64, i64) {
    (
        value["value"].as_str().unwrap().to_owned(),
        value["tasks"].as_i64().unwrap(),
        value["unestimated"].as_i64().unwrap(),
        value["otherUnit"].as_i64().unwrap(),
    )
}

fn row<'a>(report: &'a Value, title: &str) -> &'a Value {
    report["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["ticket"]["title"] == title)
        .unwrap_or_else(|| panic!("no row {title}"))
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scn019_a_sprint_report_is_read_from_the_baseline_the_assignments_and_the_revisions(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Report 019").await;
    let done = state_id(&workflow, "DONE");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let a = estimated_ticket(world, &project_id, "p72a-a", "A", Some("3")).await;
    let b = estimated_ticket(world, &project_id, "p72a-b", "B", Some("5")).await;
    let (sprint, _) = new_sprint(world, &project_id, "Plan 8 completed 7", "p72a-sprint").await;

    // Before the start there is nothing to report, and the answer says why.
    let before = report_of(world, &world.owner_cookie, &sprint).await;
    assert_eq!(
        (
            before["availability"]["available"].clone(),
            before["availability"]["reason"].clone()
        ),
        (json!(false), json!("NO_BASELINE"))
    );
    assert!(before["startPlan"].is_null() && before["tickets"].as_array().unwrap().is_empty());

    assign_to(world, &a, Some(&sprint), "p72a-as-a").await;
    assign_to(world, &b, Some(&sprint), "p72a-as-b").await;
    sprint_command_as_owner(world, &project_id, &sprint, "start", "p72a-start").await;
    // A grows to 5, C = 2 is added, B is taken out; A and C are done.
    let (s, v) = patch_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{a}"),
        "p72a-a-est",
        json!({"expectedRevision":1,"estimate":{"value":"5"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let c = estimated_ticket(world, &project_id, "p72a-c", "C", Some("2")).await;
    assign_to(world, &c, Some(&sprint), "p72a-as-c").await;
    assign_to(world, &b, None, "p72a-out-b").await;
    move_to(world, &a, &done, 2, "p72a-done-a").await;
    move_to(world, &c, &done, 1, "p72a-done-c").await;

    // While it runs the figures are provisional and read the timeline up to now.
    let running = report_of(world, &world.owner_cookie, &sprint).await;
    assert_eq!(running["provisional"], true);
    assert_eq!(amount(&running["startPlan"]), ("8".into(), 2, 0, 0));
    assert_eq!(amount(&running["completed"]), ("7".into(), 2, 0, 0));
    assert_eq!(amount(&running["remaining"]).0, "0");

    sprint_command_as_owner(world, &project_id, &sprint, "complete", "p72a-close").await;
    let report = report_of(world, &world.owner_cookie, &sprint).await;
    assert_eq!(report["provisional"], false);
    assert_eq!(report["scope"], "VISIBLE_TICKETS");
    assert_eq!(report["sprint"]["estimateUnit"], "POINTS");
    assert_eq!(
        amount(&report["startPlan"]),
        ("8".into(), 2, 0, 0),
        "A 3 + B 5 at the start"
    );
    assert_eq!(
        amount(&report["completed"]),
        ("7".into(), 2, 0, 0),
        "A at its end estimate 5, C at 2"
    );
    assert_eq!(
        report["adherence"]["percent"], "37.5",
        "A counts at its start estimate 3 over 8"
    );
    assert_eq!(amount(&report["adherence"]["completedFromPlan"]).0, "3");
    assert_eq!(report["adherence"]["notApplicable"], false);
    assert_eq!(amount(&report["remaining"]).0, "0");
    assert_eq!(
        (
            row(&report, "A")["reason"].clone(),
            row(&report, "A")["startEstimate"].clone(),
            row(&report, "A")["endEstimate"].clone()
        ),
        (json!("COUNTED"), json!("3"), json!("5"))
    );
    assert_eq!(
        (
            row(&report, "B")["reason"].clone(),
            row(&report, "B")["outcome"].clone(),
            row(&report, "B")["counted"].clone()
        ),
        (json!("REMOVED"), json!("REMOVED"), json!(false))
    );
    assert_eq!(
        (
            row(&report, "C")["reason"].clone(),
            row(&report, "C")["inBaseline"].clone()
        ),
        (json!("COUNTED"), json!(false))
    );
    let kinds: Vec<(String, String)> = report["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["kind"].as_str().unwrap().to_owned(),
                c["ticket"]["title"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(
        kinds.contains(&("ESTIMATE_CHANGED".into(), "A".into()))
            && kinds.contains(&("ADDED".into(), "C".into()))
            && kinds.contains(&("REMOVED".into(), "B".into())),
        "{kinds:?}"
    );
    let estimate = report["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "ESTIMATE_CHANGED")
        .unwrap();
    assert_eq!(
        (
            estimate["estimateFrom"].clone(),
            estimate["estimateTo"].clone()
        ),
        (json!("3"), json!("5"))
    );
    // The Burndown starts at the plan and ends at the work left; its days are the Sprint's.
    let points = report["burndown"].as_array().unwrap();
    assert_eq!(points[0]["remaining"]["value"], "8");
    assert!(points[0]["day"].is_null());
    assert_eq!(points.last().unwrap()["remaining"]["value"], "0");
    assert!(points.len() >= 2 && points[1]["day"].is_string());

    // A finished Sprint does not move when its Tickets do.
    let (s, v) = patch_as(
        world,
        &world.owner_cookie,
        &format!("/v1/tickets/{a}"),
        "p72a-late",
        json!({"expectedRevision":3,"estimate":{"value":"13"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let again = report_of(world, &world.owner_cookie, &sprint).await;
    assert_eq!(amount(&again["completed"]), ("7".into(), 2, 0, 0));
    assert_eq!(again["changes"], report["changes"]);
    // Anyone who can open the Project reads it; an unknown Sprint is not found.
    let viewer = viewer_cookie(world, workspace_id, "p72a-viewer@penfii.test").await;
    let viewer_id = user_id_of(world, "p72a-viewer@penfii.test").await;
    grant(world, &project_id, viewer_id, "VIEWER").await;
    assert_eq!(
        report_of(world, &viewer, &sprint).await["adherence"]["percent"],
        "37.5"
    );
    let (s, _) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/sprints/{}/report", Uuid::new_v4()),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scn020_cancel_zero_unestimated_other_unit_and_work_finished_before_the_start(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Report 020").await;
    let (done, canceled, doing) = (
        state_id(&workflow, "DONE"),
        state_id(&workflow, "CANCELED"),
        state_id(&workflow, "IN_PROGRESS"),
    );
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let x = estimated_ticket(
        world,
        &project_id,
        "p72b-x",
        "X canceled in the Sprint",
        Some("4"),
    )
    .await;
    let y = estimated_ticket(world, &project_id, "p72b-y", "Y done before", Some("2")).await;
    let z = estimated_ticket(world, &project_id, "p72b-z", "Z unestimated", None).await;
    let w = estimated_ticket(world, &project_id, "p72b-w", "W zero", Some("0")).await;
    let q = estimated_ticket(world, &project_id, "p72b-q", "Q canceled before", Some("1")).await;
    let o = estimated_ticket(world, &project_id, "p72b-o", "O other unit", Some("6")).await;
    move_to(world, &y, &done, 1, "p72b-y-done").await;
    move_to(world, &q, &canceled, 1, "p72b-q-cancel").await;
    move_to(world, &x, &doing, 1, "p72b-x-doing").await;
    // The Project changes unit before this Sprint exists: O stays in points, the Sprint counts hours.
    let (s, v) = set_unit(world, &project_id, "HOURS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let h = estimated_ticket(world, &project_id, "p72b-h", "H in hours", Some("3")).await;
    let (sprint, _) = new_sprint(world, &project_id, "Cancel lowers burndown", "p72b-sprint").await;
    for (n, id) in [&x, &y, &z, &w, &q, &o, &h].into_iter().enumerate() {
        assign_to(world, id, Some(&sprint), &format!("p72b-as{n}")).await;
    }
    // X was raised to hours too, so the plan is H 3 + X 4... X is in points, only H is in hours.
    sprint_command_as_owner(world, &project_id, &sprint, "start", "p72b-start").await;
    move_to(world, &x, &canceled, 2, "p72b-x-cancel").await;
    move_to(world, &z, &done, 1, "p72b-z-done").await;
    move_to(world, &w, &done, 1, "p72b-w-done").await;
    move_to(world, &h, &done, 1, "p72b-h-done").await;
    sprint_command_as_owner(world, &project_id, &sprint, "complete", "p72b-close").await;

    let report = report_of(world, &world.owner_cookie, &sprint).await;
    assert_eq!(report["sprint"]["estimateUnit"], "HOURS");
    // Planned: X (points), Z (none), W (points 0), O (points), H 3 hours. Y and Q were finished before.
    assert_eq!(
        amount(&report["startPlan"]),
        ("3".into(), 5, 1, 3),
        "only H is in the Sprint's unit; Z is unestimated, X, W and O are in another unit"
    );
    // Completed: Z, W, H. X was canceled and Y finished before the start; O stayed open.
    assert_eq!(amount(&report["completed"]), ("3".into(), 3, 1, 1));
    let reasons: Vec<(String, String)> = report["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["ticket"]["title"].as_str().unwrap().to_owned(),
                t["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let reason = |title: &str| reasons.iter().find(|r| r.0 == title).unwrap().1.clone();
    assert_eq!(
        (
            reason("X canceled in the Sprint"),
            reason("Y done before"),
            reason("Q canceled before"),
            reason("Z unestimated"),
            reason("W zero"),
            reason("O other unit"),
            reason("H in hours")
        ),
        (
            "CANCELED".to_owned(),
            "DONE_BEFORE".to_owned(),
            "CANCELED".to_owned(),
            "COUNTED".to_owned(),
            "COUNTED".to_owned(),
            "NOT_DONE".to_owned(),
            "COUNTED".to_owned()
        )
    );
    assert_eq!(row(&report, "O other unit")["otherUnit"], true);
    assert_eq!(
        row(&report, "O other unit")["startEstimate"],
        Value::Null,
        "another unit is not shown as if it were this one"
    );
    // A cancel lowers the burndown: after it, only O (another unit, counted apart) is left.
    let last = report["burndown"].as_array().unwrap().last().unwrap();
    assert_eq!(amount(&last["remaining"]), ("0".into(), 1, 0, 1));
    assert_eq!(
        report["adherence"]["percent"], "100",
        "H is the whole plan in this unit"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_report_never_counts_what_the_viewer_cannot_see_and_old_sprints_have_no_figures(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Report scope").await;
    let done = state_id(&workflow, "DONE");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"report secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{channel}");
    let open = estimated_ticket(world, &project_id, "p72c-open", "Open work", Some("3")).await;
    let (s, secret) = make_ticket(
        world,
        &project_id,
        "p72c-secret",
        json!({"title":"Secret report work","channelId":channel["id"],"estimate":{"value":"8"}}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{secret}");
    let secret_id = secret["id"].as_str().unwrap().to_owned();
    let (sprint, _) = new_sprint(world, &project_id, "Scope", "p72c-sprint").await;
    assign_to(world, &open, Some(&sprint), "p72c-as1").await;
    assign_to(world, &secret_id, Some(&sprint), "p72c-as2").await;
    sprint_command_as_owner(world, &project_id, &sprint, "start", "p72c-start").await;
    move_to(world, &secret_id, &done, 1, "p72c-secret-done").await;
    sprint_command_as_owner(world, &project_id, &sprint, "complete", "p72c-close").await;

    let member = viewer_cookie(world, workspace_id, "p72c-member@penfii.test").await;
    let member_id = user_id_of(world, "p72c-member@penfii.test").await;
    grant(world, &project_id, member_id, "EDITOR").await;
    let mine = report_of(world, &world.owner_cookie, &sprint).await;
    let theirs = report_of(world, &member, &sprint).await;
    assert_eq!(amount(&mine["startPlan"]), ("11".into(), 2, 0, 0));
    assert_eq!(amount(&mine["completed"]).0, "8");
    // Without the channel the secret Task is in no sum, no count, no row, no change, no point.
    assert_eq!(amount(&theirs["startPlan"]), ("3".into(), 1, 0, 0));
    assert_eq!(amount(&theirs["completed"]), ("0".into(), 0, 0, 0));
    assert!(
        !theirs.to_string().contains("Secret report work")
            && !theirs.to_string().contains("p72c-secret")
    );
    assert_eq!(theirs["tickets"].as_array().unwrap().len(), 1);
    assert!(
        theirs["burndown"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["remaining"]["tasks"].as_i64().unwrap() <= 1)
    );
    assert_eq!(theirs["adherence"]["percent"], "0");

    // A Sprint that started before baselines carried figures says so and invents nothing.
    sqlx::query("UPDATE penfii.sprint_baselines SET snapshot_version=1 WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&sprint).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let legacy = report_of(world, &world.owner_cookie, &sprint).await;
    assert_eq!(
        (
            legacy["availability"]["available"].clone(),
            legacy["availability"]["reason"].clone()
        ),
        (json!(false), json!("LEGACY_BASELINE"))
    );
    assert!(
        legacy["startPlan"].is_null()
            && legacy["tickets"].as_array().unwrap().is_empty()
            && legacy["burndown"].as_array().unwrap().is_empty()
    );
    // The unit NONE counts Tasks.
    let (plain, _) = new_project(world, workspace_id, "Report none").await;
    let t1 = estimated_ticket(world, &plain, "p72c-n1", "N1", None).await;
    let t2 = estimated_ticket(world, &plain, "p72c-n2", "N2", None).await;
    let (np, _) = new_sprint(world, &plain, "Counting", "p72c-nsprint").await;
    assign_to(world, &t1, Some(&np), "p72c-nas1").await;
    assign_to(world, &t2, Some(&np), "p72c-nas2").await;
    sprint_command_as_owner(world, &plain, &np, "start", "p72c-nstart").await;
    let counting = report_of(world, &world.owner_cookie, &np).await;
    assert_eq!(
        amount(&counting["startPlan"]),
        ("2".into(), 2, 0, 0),
        "each Task is 1"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn velocity_compares_the_last_completed_sprints_in_one_unit_and_lists_the_rest_apart(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Velocity").await;
    let done = state_id(&workflow, "DONE");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut sprints = Vec::new();
    for (n, estimate) in ["3", "5"].into_iter().enumerate() {
        let t = estimated_ticket(
            world,
            &project_id,
            &format!("p72v-t{n}"),
            &format!("V{n}"),
            Some(estimate),
        )
        .await;
        let (sprint, _) = new_sprint(
            world,
            &project_id,
            &format!("Velocity {n}"),
            &format!("p72v-s{n}"),
        )
        .await;
        assign_to(world, &t, Some(&sprint), &format!("p72v-as{n}")).await;
        sprint_command_as_owner(
            world,
            &project_id,
            &sprint,
            "start",
            &format!("p72v-start{n}"),
        )
        .await;
        move_to(world, &t, &done, 1, &format!("p72v-done{n}")).await;
        sprint_command_as_owner(
            world,
            &project_id,
            &sprint,
            "complete",
            &format!("p72v-close{n}"),
        )
        .await;
        sprints.push(sprint);
    }
    // A canceled Sprint is not a completed one.
    let (canceled, _) = new_sprint(world, &project_id, "Never", "p72v-c").await;
    sprint_command_as_owner(world, &project_id, &canceled, "cancel", "p72v-cancel").await;
    let (s, v) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/velocity"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let entries = v["sprints"].as_array().unwrap();
    assert_eq!(v["unit"], "POINTS");
    assert_eq!(
        entries
            .iter()
            .map(|e| e["sprint"]["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Velocity 1", "Velocity 0"],
        "newest first, completed only"
    );
    assert_eq!(
        entries
            .iter()
            .map(|e| e["completed"]["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["5", "3"]
    );
    assert!(entries.iter().all(|e| e["included"] == true));

    // The Project moves to hours; the next Sprint is counted in hours and the old ones are listed apart.
    let (s, v) = set_unit(world, &project_id, "HOURS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let t = estimated_ticket(world, &project_id, "p72v-h", "Hours work", Some("12")).await;
    let (hours, _) = new_sprint(world, &project_id, "Velocity hours", "p72v-sh").await;
    assign_to(world, &t, Some(&hours), "p72v-ash").await;
    sprint_command_as_owner(world, &project_id, &hours, "start", "p72v-starth").await;
    move_to(world, &t, &done, 1, "p72v-doneh").await;
    sprint_command_as_owner(world, &project_id, &hours, "complete", "p72v-closeh").await;
    sqlx::query("UPDATE penfii.sprint_baselines SET snapshot_version=1 WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&sprints[0]).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let (_, v) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/velocity"),
    )
    .await;
    assert_eq!(v["unit"], "HOURS");
    let by = |name: &str| {
        v["sprints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["sprint"]["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            by("Velocity hours")["included"].clone(),
            by("Velocity hours")["completed"]["value"].clone()
        ),
        (json!(true), json!("12"))
    );
    assert_eq!(
        (
            by("Velocity 1")["included"].clone(),
            by("Velocity 1")["reason"].clone(),
            by("Velocity 1")["completed"].clone()
        ),
        (json!(false), json!("DIFFERENT_UNIT"), Value::Null)
    );
    assert_eq!(
        (
            by("Velocity 0")["included"].clone(),
            by("Velocity 0")["reason"].clone()
        ),
        (json!(false), json!("LEGACY_BASELINE"))
    );
    // The limit narrows the list and is bounded.
    let (_, one) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/velocity?limit=1"),
    )
    .await;
    assert_eq!(one["sprints"].as_array().unwrap().len(), 1);
    let (s, _) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{}/velocity", Uuid::new_v4()),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_status_summary_counts_categories_blocked_and_due_soon_for_the_tickets_the_viewer_sees(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Summary").await;
    let (done, canceled, doing) = (
        state_id(&workflow, "DONE"),
        state_id(&workflow, "CANCELED"),
        state_id(&workflow, "IN_PROGRESS"),
    );
    let (s, empty) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/status-summary"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{empty}");
    assert_eq!(empty["distribution"]["total"], 0);
    assert_eq!(
        (empty["dueWithinDays"].clone(), empty["scope"].clone()),
        (json!(7), json!("VISIBLE_TICKETS"))
    );
    let today =
        chrono::NaiveDate::parse_from_str(empty["today"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let day = |n: i64| (today + chrono::Duration::days(n)).to_string();
    let mk = |n: u32, due: Option<String>| {
        let project_id = project_id.clone();
        async move {
            let mut body = json!({"title":format!("S{n}")});
            if let Some(due) = due {
                body["dueOn"] = json!(due);
            }
            let (s, t) = make_ticket(world, &project_id, &format!("p72s-{n}"), body).await;
            assert_eq!(s, StatusCode::CREATED, "{t}");
            t["id"].as_str().unwrap().to_owned()
        }
    };
    let soon = mk(1, Some(day(3))).await;
    let late = mk(2, Some(day(-4))).await;
    let edge = mk(3, Some(day(7))).await;
    let far = mk(4, Some(day(8))).await;
    let finished = mk(5, Some(day(1))).await;
    let dropped = mk(6, Some(day(1))).await;
    let working = mk(7, None).await;
    let waiting = mk(8, None).await;
    move_to(world, &finished, &done, 1, "p72s-d").await;
    move_to(world, &dropped, &canceled, 1, "p72s-c").await;
    move_to(world, &working, &doing, 1, "p72s-w").await;
    // `waiting` is blocked by `far` (open); `soon` by `finished` (done, so not blocked).
    let dep = |successor: &str, predecessor: &str, key: &str| {
        let (successor, predecessor, key) =
            (successor.to_owned(), predecessor.to_owned(), key.to_owned());
        async move {
            call(
                &world.app,
                "POST",
                &format!("/v1/tickets/{successor}/dependencies"),
                Some(&world.owner_cookie),
                Some(&key),
                Some(json!({"predecessorId":predecessor})),
            )
            .await
        }
    };
    assert_eq!(
        dep(&waiting, &far, "p72s-dep1").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        dep(&soon, &finished, "p72s-dep2").await.0,
        StatusCode::CREATED
    );
    let (s, summary) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/status-summary"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{summary}");
    assert_eq!(
        summary["distribution"],
        json!({"notStarted":5,"inProgress":1,"done":1,"canceled":1,"total":8})
    );
    assert_eq!(summary["blocked"], 1);
    assert_eq!(
        summary["dueSoon"], 3,
        "in 3 days, 4 days overdue and exactly in 7 days; not 8 days, not done, not canceled"
    );
    let _ = (late, edge);
    // Blocked counts Tasks: a Subtask waiting for an open predecessor is a Ticket in the
    // distribution and not "blocked".
    let (s, sub) = make_ticket(
        world,
        &project_id,
        "p72s-sub",
        json!({"title":"Waiting subtask","hierarchyKind":"SUBTASK","parentId":working}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sub}");
    assert_eq!(
        dep(sub["id"].as_str().unwrap(), &far, "p72s-dep3").await.0,
        StatusCode::CREATED
    );
    let (_, with_sub) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/status-summary"),
    )
    .await;
    assert_eq!(with_sub["distribution"]["total"], 9);
    assert_eq!(
        with_sub["blocked"], 1,
        "the blocked Task is counted, the blocked Subtask is not"
    );
    // A Ticket in a private channel is in nothing for someone outside it.
    let (s, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"summary secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{channel}");
    let (s, secret) = make_ticket(
        world,
        &project_id,
        "p72s-secret",
        json!({"title":"Secret","channelId":channel["id"],"dueOn":day(1)}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{secret}");
    let member = viewer_cookie(world, workspace_id, "p72s-member@penfii.test").await;
    let member_id = user_id_of(world, "p72s-member@penfii.test").await;
    grant(world, &project_id, member_id, "VIEWER").await;
    let (_, mine) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/status-summary"),
    )
    .await;
    let (_, theirs) = get_as(
        world,
        &member,
        &format!("/v1/projects/{project_id}/status-summary"),
    )
    .await;
    assert_eq!(
        (
            mine["distribution"]["total"].clone(),
            mine["dueSoon"].clone()
        ),
        (json!(10), json!(4))
    );
    assert_eq!(
        (
            theirs["distribution"]["total"].clone(),
            theirs["dueSoon"].clone()
        ),
        (json!(9), json!(3))
    );
    let (s, _) = get_as(
        world,
        &member,
        &format!("/v1/projects/{}/status-summary", Uuid::new_v4()),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn velocity_compares_only_sprints_with_the_same_unit_and_calculation_policy_version(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, workflow) = new_project(world, workspace_id, "Velocity policy").await;
    let done = state_id(&workflow, "DONE");
    let (s, v) = set_unit(world, &project_id, "POINTS").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut sprints = Vec::new();
    for (n, estimate) in ["2", "4", "6"].into_iter().enumerate() {
        let t = estimated_ticket(
            world,
            &project_id,
            &format!("p72p-t{n}"),
            &format!("P{n}"),
            Some(estimate),
        )
        .await;
        let (sprint, _) = new_sprint(
            world,
            &project_id,
            &format!("Policy {n}"),
            &format!("p72p-s{n}"),
        )
        .await;
        assign_to(world, &t, Some(&sprint), &format!("p72p-as{n}")).await;
        sprint_command_as_owner(
            world,
            &project_id,
            &sprint,
            "start",
            &format!("p72p-start{n}"),
        )
        .await;
        move_to(world, &t, &done, 1, &format!("p72p-done{n}")).await;
        sprint_command_as_owner(
            world,
            &project_id,
            &sprint,
            "complete",
            &format!("p72p-close{n}"),
        )
        .await;
        sprints.push(sprint);
    }
    let velocity = || async {
        let (s, v) = get_as(
            world,
            &world.owner_cookie,
            &format!("/v1/projects/{project_id}/velocity"),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    };
    let set_policy = |sprint: &str, policy: i16| {
        let (pool, id) = (world.pool.clone(), Uuid::parse_str(sprint).unwrap());
        async move {
            sqlx::query("UPDATE penfii.sprint_baselines SET policy_version=$2 WHERE sprint_id=$1")
                .bind(id)
                .bind(policy)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    let entry = |v: &Value, name: &str| {
        v["sprints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["sprint"]["name"] == name)
            .unwrap()
            .clone()
    };
    // One policy: all three are compared, and the report says which policy that is.
    let all = velocity().await;
    assert_eq!(
        (all["unit"].clone(), all["policyVersion"].clone()),
        (json!("POINTS"), json!(1))
    );
    assert!(
        all["sprints"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["included"] == true)
    );
    // The middle Sprint was calculated under another policy: it is listed apart with that reason.
    set_policy(&sprints[1], 2).await;
    let mixed = velocity().await;
    assert_eq!(
        (mixed["unit"].clone(), mixed["policyVersion"].clone()),
        (json!("POINTS"), json!(1))
    );
    let middle = entry(&mixed, "Policy 1");
    assert_eq!(
        (
            middle["included"].clone(),
            middle["reason"].clone(),
            middle["completed"].clone()
        ),
        (json!(false), json!("DIFFERENT_POLICY"), Value::Null)
    );
    assert_eq!(
        (
            entry(&mixed, "Policy 2")["included"].clone(),
            entry(&mixed, "Policy 0")["included"].clone()
        ),
        (json!(true), json!(true))
    );
    // The newest Sprint decides which policy is compared: now the oldest is the odd one out.
    set_policy(&sprints[2], 2).await;
    let newest = velocity().await;
    assert_eq!(newest["policyVersion"], 2);
    assert_eq!(
        (
            entry(&newest, "Policy 2")["included"].clone(),
            entry(&newest, "Policy 1")["included"].clone(),
            entry(&newest, "Policy 0")["reason"].clone()
        ),
        (json!(true), json!(true), json!("DIFFERENT_POLICY"))
    );
    // The unit is checked first; a Sprint with no figures keeps its own reason.
    sqlx::query("UPDATE penfii.sprints SET estimate_unit='HOURS' WHERE id=$1")
        .bind(Uuid::parse_str(&sprints[0]).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE penfii.sprint_baselines SET snapshot_version=1 WHERE sprint_id=$1")
        .bind(Uuid::parse_str(&sprints[1]).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let reasons = velocity().await;
    assert_eq!(
        (
            entry(&reasons, "Policy 0")["reason"].clone(),
            entry(&reasons, "Policy 1")["reason"].clone()
        ),
        (json!("DIFFERENT_UNIT"), json!("LEGACY_BASELINE"))
    );
}

// Timeline
const DAY: i64 = 540;

async fn leaf(
    world: &World,
    project_id: &str,
    key: &str,
    title: &str,
    minutes: Option<i64>,
) -> String {
    let mut body = json!({"title":title});
    if let Some(m) = minutes {
        body["schedule"] = json!({"durationMinutes":m});
    }
    let (s, t) = make_ticket(world, project_id, key, body).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_owned()
}

async fn blocks(world: &World, successor: &str, predecessor: &str, key: &str) {
    let (s, v, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{successor}/dependencies"),
        Some(&world.owner_cookie),
        Some(key),
        Some(json!({"predecessorId":predecessor})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

/// An edge from a Ticket the person cannot see. The API refuses to create one (a private Ticket may only
/// block its own channel), but it is reachable once a channel turns private afterwards, so it is inserted directly.
async fn blocks_from_hidden(world: &World, successor: &str, predecessor: &str) {
    sqlx::query("INSERT INTO penfii.ticket_dependencies(id,workspace_id,project_id,predecessor_id,successor_id,created_by) SELECT $1, workspace_id, project_id, $2, id, $3 FROM penfii.tickets WHERE id=$4")
        .bind(Uuid::new_v4())
        .bind(Uuid::parse_str(predecessor).unwrap())
        .bind(world.owner_id)
        .bind(Uuid::parse_str(successor).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
}

async fn seoul_calendar(world: &World, project_id: &str) {
    let (s, v) = put_calendar(
        world,
        &world.owner_cookie,
        project_id,
        calendar_body(0, "Asia/Seoul", week_9_to_18(), json!([])),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

const START: &str = "2026-11-02T00:00:00Z"; // Monday 09:00 in Seoul

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_gantt_list_has_schedules_with_derived_ends_visible_edges_and_pages(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Gantt list").await;
    let owner = world.owner_cookie.clone();
    // Without a calendar the ends are null; the planned Tickets still list.
    let first = leaf(world, &project_id, "p73g-1", "First", Some(DAY)).await;
    let (s, v) = patch_as(
        world,
        &owner,
        &format!("/v1/tickets/{first}"),
        "p73g-1s",
        json!({"expectedRevision":1,"schedule":{"plannedStartAt":START,"durationMinutes":2 * DAY}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, list) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/schedule"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert!(list["calendar"].is_null() && list["items"][0]["schedule"]["plannedEndAt"].is_null());
    assert_eq!(list["canEdit"], true);
    seoul_calendar(world, &project_id).await;
    let second = leaf(world, &project_id, "p73g-2", "Second", None).await;
    let node = {
        let (s, t) = make_ticket(world, &project_id, "p73g-3", json!({"title":"Node","schedule":{"plannedStartAt":"2026-11-07T03:00:00Z","durationMinutes":0,"isScheduleMilestone":true}})).await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        t["id"].as_str().unwrap().to_owned()
    };
    blocks(world, &second, &first, "p73g-dep").await;
    let (_, list) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/schedule"),
    )
    .await;
    let item = |title: &str| {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["ticket"]["title"] == title)
            .unwrap()
            .clone()
    };
    assert_eq!(list["calendar"]["timeZone"], "Asia/Seoul");
    assert_eq!(
        item("First")["schedule"]["plannedEndAt"],
        "2026-11-03T09:00:00Z",
        "two working days from Monday 09:00"
    );
    assert_eq!(
        item("Node")["schedule"]["plannedEndAt"],
        "2026-11-09T00:00:00Z",
        "a node on Saturday waits for Monday"
    );
    assert!(
        item("Second")["schedule"].is_null(),
        "no schedule means unplanned"
    );
    assert_eq!(
        (
            item("First")["statusCategory"].clone(),
            item("First")["revisionNo"].clone()
        ),
        (json!("NOT_STARTED"), json!(2))
    );
    assert_eq!(list["edges"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            list["edges"][0]["predecessorId"].clone(),
            list["edges"][0]["waived"].clone()
        ),
        (json!(first), json!(false))
    );
    // Pages by Ticket number.
    let (_, one) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/schedule?limit=2"),
    )
    .await;
    assert_eq!(one["items"].as_array().unwrap().len(), 2);
    let cursor = one["nextCursor"].as_str().unwrap().to_owned();
    let (_, rest) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/schedule?limit=2&cursor={cursor}"),
    )
    .await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert!(rest["nextCursor"].is_null());
    assert_eq!(rest["items"][0]["ticket"]["id"], node.as_str());
    let (s, _) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/schedule?cursor=abc"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // A private channel's Ticket is not listed for someone outside it, and neither is its edge.
    let (s, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"gantt secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{channel}");
    let (s, secret) = make_ticket(
        world,
        &project_id,
        "p73g-s",
        json!({"title":"Secret slot","channelId":channel["id"],"schedule":{"durationMinutes":DAY}}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{secret}");
    blocks_from_hidden(world, &first, secret["id"].as_str().unwrap()).await;
    let viewer = viewer_cookie(world, workspace_id, "p73g-viewer@penfii.test").await;
    let viewer_id = user_id_of(world, "p73g-viewer@penfii.test").await;
    grant(world, &project_id, viewer_id, "VIEWER").await;
    let (_, mine) = get_as(
        world,
        &owner,
        &format!("/v1/projects/{project_id}/schedule"),
    )
    .await;
    let (_, theirs) = get_as(
        world,
        &viewer,
        &format!("/v1/projects/{project_id}/schedule"),
    )
    .await;
    assert_eq!(
        (
            mine["items"].as_array().unwrap().len(),
            mine["edges"].as_array().unwrap().len()
        ),
        (4, 2)
    );
    assert_eq!(
        (
            theirs["items"].as_array().unwrap().len(),
            theirs["edges"].as_array().unwrap().len()
        ),
        (3, 1)
    );
    assert_eq!(theirs["canEdit"], false);
    assert!(!theirs.to_string().contains("Secret slot"));
    // The channel filter narrows the list.
    let (_, only) = get_as(
        world,
        &owner,
        &format!(
            "/v1/projects/{project_id}/schedule?channelId={}",
            channel["id"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(only["items"].as_array().unwrap().len(), 1);
}
