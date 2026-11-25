//! Where a change came from, and what lists say about schedules: the defects found on the NAS after
//! the MCP extension (FEA-158), each with the case that showed it.
//!
//! - D1 (FEA-164): a bulk change names the MCP client on every Ticket it wrote, as a single edit does.
//! - D2 (FEA-160): channel names are not unique; a retry is told apart by its idempotency key.
//! - D3 (FEA-168): a board, My Work and a Ticket's children give the same planned end as the Ticket.
//! - D4 (FEA-171): an audit entry made through MCP names the client and grant, as `mcp.*` ones do.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, modern_call, modern_list, rest, setup, token_for, token_for_client,
};
use crate::mcp_support::{mcp_err, mcp_ok};

const CLAUDE: &str = "https://claude.example/oauth/client.json";
const TICKET_SCOPES: &str =
    "workspace:read ticket:read ticket:write plan:read plan:write project:read project:write";

async fn project(env: &Env, name: &str) -> String {
    let reply = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": name, "teamIds": []})),
    )
    .await;
    assert!(reply.status.is_success(), "{:?}", reply.body);
    reply.body["id"].as_str().unwrap().to_string()
}

async fn web_ticket(env: &Env, project_id: &str, body: Value) -> Value {
    let reply = rest(
        env,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        &env.owner_cookie,
        Some(&Uuid::new_v4().to_string()),
        Some(body),
    )
    .await;
    assert!(reply.status.is_success(), "{:?}", reply.body);
    reply.body
}

async fn web_get(env: &Env, path: &str) -> Value {
    let reply = rest(env, "GET", path, &env.owner_cookie, None, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{path}: {:?}", reply.body);
    reply.body
}

/// `via` of every revision of a Ticket, newest first, from the web route.
async fn revision_via(env: &Env, ticket_id: &str) -> Vec<(i64, Option<String>)> {
    web_get(env, &format!("/v1/tickets/{ticket_id}/revisions")).await["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["revisionNo"].as_i64().unwrap(),
                r["via"].as_str().map(str::to_string),
            )
        })
        .collect()
}

/// `via` of the newest Activity row of a kind, from the web route.
async fn activity_via(env: &Env, ticket_id: &str, kind: &str) -> Option<String> {
    let feed = web_get(env, &format!("/v1/tickets/{ticket_id}/activity")).await;
    let items = feed["items"].as_array().unwrap();
    items
        .iter()
        .find(|item| item["activityKind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} activity: {items:?}"))["via"]
        .as_str()
        .map(str::to_string)
}

fn revision_of(ticket: &Value) -> i64 {
    ticket["revisionNo"].as_i64().unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_bulk_change_names_the_client_on_every_ticket_it_wrote(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let project_id = project(&env, "Bulk").await;
    let claude = token_for_client(
        &env,
        &env.owner_cookie,
        CLAUDE,
        "Claude Code",
        TICKET_SCOPES,
    )
    .await;
    let other = token_for(&env, &env.owner_cookie, TICKET_SCOPES).await;
    let t1 = web_ticket(&env, &project_id, json!({"title": "one"})).await;
    let t2 = web_ticket(&env, &project_id, json!({"title": "two"})).await;
    let t3 = web_ticket(&env, &project_id, json!({"title": "three"})).await;
    let (id1, id2, id3) = (
        t1["id"].as_str().unwrap().to_string(),
        t2["id"].as_str().unwrap().to_string(),
        t3["id"].as_str().unwrap().to_string(),
    );
    // What the web app made names no client.
    assert!(
        revision_via(&env, &id1)
            .await
            .iter()
            .all(|(_, via)| via.is_none())
    );

    // PRIORITY on two Tickets and one that does not exist: a partial success.
    let missing = Uuid::new_v4().to_string();
    let args = json!({"workspaceId": ws, "ticketIds": [id1, id2, missing],
                      "change": {"type": "PRIORITY", "priority": "HIGH"}, "idempotencyKey": "bulk-p"});
    let bulk = mcp_ok(&env, &claude, "ticket_bulk_update", args.clone()).await;
    assert_eq!(
        (bulk["succeeded"].as_i64(), bulk["failed"].as_i64()),
        (Some(2), Some(1)),
        "{bulk}"
    );
    for id in [&id1, &id2] {
        let revisions = revision_via(&env, id).await;
        assert_eq!(
            revisions[0],
            (2, Some("Claude Code".into())),
            "{id}: {revisions:?}"
        );
        assert_eq!(
            revisions[1],
            (1, None),
            "the web-made revision stays unmarked"
        );
        assert_eq!(
            activity_via(&env, id, "ticket.updated").await.as_deref(),
            Some("Claude Code")
        );
        // The same through MCP.
        let listed = mcp_ok(
            &env,
            &other,
            "ticket_revisions_list",
            json!({"ticketId": id}),
        )
        .await;
        assert_eq!(listed["items"][0]["via"], "Claude Code");
    }
    // The failed item wrote nothing, so nothing names a client for it.
    let stray: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.ticket_revisions WHERE ticket_id::text = $1",
    )
    .bind(&missing)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(stray, 0);

    // The same request again replays: no new revision, the first call keeps the credit.
    let replay = mcp_ok(&env, &claude, "ticket_bulk_update", args).await;
    assert_eq!(replay["items"], bulk["items"]);
    assert_eq!(revision_via(&env, &id1).await.len(), 2);
    // Even when another client replays the same key, it is not credited (keys are the person's).
    let replay_other = mcp_ok(
        &env,
        &other,
        "ticket_bulk_update",
        json!({"workspaceId": ws, "ticketIds": [id1, id2, missing],
               "change": {"type": "PRIORITY", "priority": "HIGH"}, "idempotencyKey": "bulk-p"}),
    )
    .await;
    assert_eq!(replay_other["items"], bulk["items"]);
    assert_eq!(
        revision_via(&env, &id1).await[0],
        (2, Some("Claude Code".into()))
    );

    // Labels and status, each by a different client at the same time.
    let label = mcp_ok(
        &env,
        &claude,
        "label_create",
        json!({"projectId": project_id, "name": "infra", "color": "#3366ff"}),
    )
    .await;
    let workflow = mcp_ok(
        &env,
        &claude,
        "project_workflow",
        json!({"projectId": project_id}),
    )
    .await;
    let done = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .clone();
    let (labels, status) = tokio::join!(
        modern_call(
            &env.app,
            &claude,
            "ticket_bulk_update",
            json!({"workspaceId": ws, "ticketIds": [id1], "change": {"type": "ADD_LABELS", "labelIds": [label["id"]]}})
        ),
        modern_call(
            &env.app,
            &other,
            "ticket_bulk_update",
            json!({"workspaceId": ws, "ticketIds": [id2], "change": {"type": "STATUS", "statusId": done}})
        ),
    );
    assert_eq!(labels.body["result"]["isError"], false, "{:?}", labels.body);
    assert_eq!(status.body["result"]["isError"], false, "{:?}", status.body);
    assert_eq!(
        revision_via(&env, &id1).await[0],
        (3, Some("Claude Code".into()))
    );
    assert_eq!(
        revision_via(&env, &id2).await[0],
        (3, Some("Test Client".into()))
    );

    // A single edit from the other client, then the web app, then the web app's own bulk route.
    let edited = mcp_ok(
        &env,
        &other,
        "ticket_update",
        json!({"ticketId": id1, "expectedRevision": 3, "title": "one, renamed"}),
    )
    .await;
    assert_eq!(revision_of(&edited), 4);
    let by_hand = rest(
        &env,
        "PATCH",
        &format!("/v1/tickets/{id1}"),
        &env.owner_cookie,
        Some("web-edit"),
        Some(json!({"expectedRevision": 4, "title": "by hand"})),
    )
    .await;
    assert!(by_hand.status.is_success(), "{:?}", by_hand.body);
    let web_bulk = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/tickets/bulk"),
        &env.owner_cookie,
        Some("web-bulk"),
        Some(json!({"ticketIds": [id3], "change": {"type": "PRIORITY", "priority": "LOW"}})),
    )
    .await;
    assert!(web_bulk.status.is_success(), "{:?}", web_bulk.body);
    let one = revision_via(&env, &id1).await;
    assert_eq!(one[0], (5, None), "{one:?}");
    assert_eq!(one[1], (4, Some("Test Client".into())));
    assert_eq!(revision_via(&env, &id3).await[0], (2, None));

    // A dependency is an Activity row without a revision of its own: it names its call too.
    mcp_ok(
        &env,
        &claude,
        "dependency_add",
        json!({"ticketId": id3, "predecessorId": id2}),
    )
    .await;
    assert_eq!(
        activity_via(&env, &id3, "ticket.dependency_added")
            .await
            .as_deref(),
        Some("Claude Code")
    );

    // Trash through bulk is not a revision but an audit entry: it names the client the same way.
    mcp_ok(
        &env,
        &claude,
        "ticket_bulk_update",
        json!({"workspaceId": ws, "ticketIds": [id3], "change": {"type": "TRASH"}}),
    )
    .await;
    let trashed = web_get(
        &env,
        &format!("/v1/workspaces/{ws}/audit?action=ticket.trashed"),
    )
    .await;
    assert_eq!(trashed["items"][0]["targetId"], id3.as_str());
    assert_eq!(trashed["items"][0]["via"], "Claude Code");
    assert_eq!(trashed["items"][0]["data"]["via"], "mcp");

    // The project feed agrees.
    let feed = web_get(&env, &format!("/v1/projects/{project_id}/activity")).await;
    let feed = feed
        .as_array()
        .or(feed["items"].as_array())
        .unwrap()
        .clone();
    assert!(
        feed.iter()
            .any(|i| i["ticketId"] == id2.as_str() && i["via"] == "Test Client")
    );
    assert!(
        feed.iter()
            .any(|i| i["ticketId"] == id1.as_str() && i["via"] == "Claude Code")
    );

    // A bulk call is one log row (no per-Ticket rows), and every revision it wrote points at it:
    // the first two Tickets were revised by bulk calls; the third only trashed (an audit entry).
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT r.ticket_id) FROM penfii.ticket_revisions r
         JOIN penfii.mcp_call_log l ON l.id = r.command_id
         WHERE l.operation = 'ticket.bulk' AND r.ticket_id = ANY($1::uuid[])",
    )
    .bind(vec![
        id1.parse::<Uuid>().unwrap(),
        id2.parse::<Uuid>().unwrap(),
        id3.parse::<Uuid>().unwrap(),
    ])
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(linked, 2);
    let rows_per_call: Vec<i64> = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.mcp_call_log WHERE operation = 'ticket.bulk' GROUP BY request_id",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert!(rows_per_call.iter().all(|n| *n == 1), "{rows_per_call:?}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn channel_names_repeat_and_only_the_idempotency_key_makes_a_retry(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let scopes = "workspace:read conversation:read conversation:write";
    let owner = token_for(&env, &env.owner_cookie, scopes).await;
    let count = |name: &'static str| {
        let pool = env.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM penfii.channels WHERE name = $1")
                .bind(name)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    // The description says what happens.
    let listed = modern_list(&env.app, &owner).await;
    let tool = listed.body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "channel_create")
        .unwrap()
        .clone();
    let description = tool["description"].as_str().unwrap();
    assert!(
        !description.contains("fails on the duplicate name"),
        "{description}"
    );
    assert!(description.contains("Names are not unique") && description.contains("idempotencyKey"));
    assert!(tool["inputSchema"]["properties"]["idempotencyKey"].is_object());

    // Without a key every call creates, whatever the name.
    let plain = json!({"workspaceId": ws, "name": "ops", "visibility": "PUBLIC"});
    let a = mcp_ok(&env, &owner, "channel_create", plain.clone()).await;
    let b = mcp_ok(&env, &owner, "channel_create", plain).await;
    assert_ne!(a["id"], b["id"]);
    assert_eq!(count("ops").await, 2);

    // With a key the same request returns the first channel; another request with it is refused;
    // another key creates.
    let keyed = json!({"workspaceId": ws, "name": "keyed", "visibility": "PUBLIC", "idempotencyKey": "ch-1"});
    let first = mcp_ok(&env, &owner, "channel_create", keyed.clone()).await;
    let again = mcp_ok(&env, &owner, "channel_create", keyed).await;
    assert_eq!(first["id"], again["id"]);
    assert_eq!(count("keyed").await, 1);
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "channel_create",
            json!({"workspaceId": ws, "name": "keyed", "visibility": "PRIVATE", "idempotencyKey": "ch-1"})
        )
        .await,
        "idempotency_conflict"
    );
    let third = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "keyed", "visibility": "PUBLIC", "idempotencyKey": "ch-2"}),
    )
    .await;
    assert_ne!(third["id"], first["id"]);
    assert_eq!(count("keyed").await, 2);

    // Two copies of one request at once: one channel, both told its id.
    let racing = json!({"workspaceId": ws, "name": "race", "visibility": "PUBLIC", "idempotencyKey": "ch-race"});
    let (r1, r2) = tokio::join!(
        modern_call(&env.app, &owner, "channel_create", racing.clone()),
        modern_call(&env.app, &owner, "channel_create", racing),
    );
    let ids: Vec<Value> = [r1, r2]
        .iter()
        .map(|r| {
            assert_eq!(r.body["result"]["isError"], false, "{:?}", r.body);
            r.body["result"]["structuredContent"]["id"].clone()
        })
        .collect();
    assert_eq!(ids[0], ids[1]);
    assert_eq!(count("race").await, 1);

    // A private channel's name tells nobody anything: an admin who cannot see it makes a channel of
    // the same name like any other, and reusing someone else's key is not a replay of their channel.
    let secret = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "secret", "visibility": "PRIVATE", "idempotencyKey": "ch-s"}),
    )
    .await;
    let admin_cookie = add_member(&env, "admin@penfii.test", "ADMIN").await;
    let admin = token_for(&env, &admin_cookie, scopes).await;
    let theirs = mcp_ok(
        &env,
        &admin,
        "channel_create",
        json!({"workspaceId": ws, "name": "secret", "visibility": "PRIVATE", "idempotencyKey": "ch-s"}),
    )
    .await;
    assert_ne!(theirs["id"], secret["id"]);
    assert_eq!(
        mcp_err(
            &env,
            &admin,
            "conversation_get",
            json!({"conversationId": secret["id"]})
        )
        .await,
        "not_found"
    );

    // The web route: the same rules with the Idempotency-Key header, and without one it creates.
    let path = format!("/v1/workspaces/{ws}/channels");
    let body = json!({"name": "web", "visibility": "PUBLIC"});
    let w1 = rest(
        &env,
        "POST",
        &path,
        &env.owner_cookie,
        Some("web-1"),
        Some(body.clone()),
    )
    .await;
    let w2 = rest(
        &env,
        "POST",
        &path,
        &env.owner_cookie,
        Some("web-1"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(
        (w1.status, w2.status),
        (StatusCode::CREATED, StatusCode::CREATED)
    );
    assert_eq!(w1.body["id"], w2.body["id"]);
    let conflict = rest(
        &env,
        "POST",
        &path,
        &env.owner_cookie,
        Some("web-1"),
        Some(json!({"name": "web-other", "visibility": "PUBLIC"})),
    )
    .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{:?}", conflict.body);
    assert_eq!(conflict.body["code"], "idempotency_conflict");
    let w3 = rest(&env, "POST", &path, &env.owner_cookie, None, Some(body)).await;
    assert_eq!(w3.status, StatusCode::CREATED);
    assert_ne!(w3.body["id"], w1.body["id"]);
    assert_eq!(count("web").await, 2);

    // A key does not get around the permission checks: a plain member is refused, key or not.
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, scopes).await;
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "channel_create",
            json!({"workspaceId": ws, "name": "keyed", "visibility": "PUBLIC", "idempotencyKey": "ch-1"})
        )
        .await,
        "forbidden"
    );
    // Nor does a stored result outlive the right to create: the admin who made a channel with a
    // key and is then made a plain member is refused on the retry, not handed the stored channel.
    let first_try = json!({"workspaceId": ws, "name": "demoted", "visibility": "PUBLIC", "idempotencyKey": "ch-d"});
    mcp_ok(&env, &admin, "channel_create", first_try.clone()).await;
    sqlx::query("UPDATE penfii.workspace_members SET role = 'MEMBER' WHERE workspace_id = $1 AND user_id = (SELECT id FROM penfii.users WHERE email_key = 'admin@penfii.test')")
        .bind(env.workspace_id)
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        mcp_err(&env, &admin, "channel_create", first_try).await,
        "forbidden"
    );
    assert_eq!(count("demoted").await, 1);
}

/// `plannedEndAt` of a Ticket in a list of Tickets (board columns, scope, children, ticket_list).
fn end_in(list: &[Value], id: &str) -> Value {
    list.iter()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("{id} not listed"))["schedule"]["plannedEndAt"]
        .clone()
}

fn board_tickets(board: &Value) -> Vec<Value> {
    board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["tickets"].as_array().unwrap().clone())
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn lists_give_the_same_planned_end_as_the_ticket(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let project_id = project(&env, "Schedules").await;
    let token = token_for(&env, &env.owner_cookie, TICKET_SCOPES).await;
    let day = json!([{"from": "09:00", "to": "18:00"}]);
    let week = json!({"mon": day, "tue": day, "wed": day, "thu": day, "fri": day});
    mcp_ok(
        &env,
        &token,
        "project_calendar_put",
        json!({"projectId": project_id, "expectedVersion": 0, "timeZone": "Asia/Seoul",
               "definition": {"version": 1, "week": week, "holidays": []}}),
    )
    .await;
    let epic = web_ticket(
        &env,
        &project_id,
        json!({"title": "epic", "hierarchyKind": "EPIC"}),
    )
    .await;
    // Monday 2026-12-07 09:00 in Seoul, 12 working hours: past the first day's end.
    let long = mcp_ok(
        &env,
        &token,
        "ticket_create",
        json!({"projectId": project_id, "title": "long", "parentId": epic["id"],
               "schedule": {"plannedStartAt": "2026-12-07T00:00:00Z", "durationMinutes": 720}}),
    )
    .await;
    let milestone = mcp_ok(
        &env,
        &token,
        "ticket_create",
        json!({"projectId": project_id, "title": "milestone", "parentId": epic["id"],
               "schedule": {"plannedStartAt": "2026-12-08T05:00:00Z", "isScheduleMilestone": true}}),
    )
    .await;
    let start_only = mcp_ok(
        &env,
        &token,
        "ticket_create",
        json!({"projectId": project_id, "title": "start only", "parentId": epic["id"],
               "schedule": {"plannedStartAt": "2026-12-09T00:00:00Z"}}),
    )
    .await;
    let unplanned = web_ticket(&env, &project_id, json!({"title": "unplanned"})).await;
    let ids: Vec<String> = [&long, &milestone, &start_only, &unplanned]
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // Every read, web and MCP, against the Ticket read on its own; `expected` pins the values that
    // are known here (`None` = only "the same as ticket_get, and set").
    let check = |expected: Vec<Option<Value>>| {
        let env = &env;
        let (token, ids, project_id, ws) = (&token, &ids, &project_id, &ws);
        let epic_id = epic["id"].as_str().unwrap().to_string();
        async move {
            let board = mcp_ok(
                env,
                token,
                "board_get",
                json!({"projectId": project_id, "perColumnLimit": 50}),
            )
            .await;
            let web_board = web_get(env, &format!("/v1/projects/{project_id}/board")).await;
            let work = mcp_ok(
                env,
                token,
                "work_list",
                json!({"workspaceId": ws, "limit": 50}),
            )
            .await;
            let web_work = web_get(env, &format!("/v1/workspaces/{ws}/work?scope=WORKSPACE")).await;
            let listed = mcp_ok(
                env,
                token,
                "ticket_list",
                json!({"projectId": project_id, "limit": 50}),
            )
            .await;
            // MCP lists children as compact items without schedules; the web route has them.
            let children = web_get(env, &format!("/v1/tickets/{epic_id}/children")).await;
            let schedule = mcp_ok(
                env,
                token,
                "plan_schedule",
                json!({"projectId": project_id}),
            )
            .await;
            let gantt: Vec<Value> = schedule["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| json!({"id": i["ticket"]["id"], "schedule": i["schedule"]}))
                .collect();
            let children = children
                .as_array()
                .or(children["items"].as_array())
                .unwrap()
                .clone();
            for (id, pinned) in ids.iter().zip(&expected) {
                let single = mcp_ok(env, token, "ticket_get", json!({"ticketId": id})).await;
                let want = &single["schedule"]["plannedEndAt"];
                match pinned {
                    Some(value) => assert_eq!(want, value, "ticket_get {id}"),
                    None => assert!(want.is_string(), "ticket_get {id}: {single}"),
                }
                assert_eq!(&end_in(&board_tickets(&board), id), want, "board_get {id}");
                assert_eq!(
                    &end_in(&board_tickets(&web_board), id),
                    want,
                    "web board {id}"
                );
                assert_eq!(
                    &end_in(work["tickets"].as_array().unwrap(), id),
                    want,
                    "work_list {id}"
                );
                assert_eq!(
                    &end_in(web_work["tickets"].as_array().unwrap(), id),
                    want,
                    "web work {id}"
                );
                assert_eq!(&end_in(&gantt, id), want, "plan_schedule {id}");
                if let Some(items) = listed["items"].as_array()
                    && items
                        .iter()
                        .any(|t| t["id"] == id.as_str() && t.get("schedule").is_some())
                {
                    assert_eq!(&end_in(items, id), want, "ticket_list {id}");
                }
                if children.iter().any(|t| t["id"] == id.as_str()) {
                    assert_eq!(&end_in(&children, id), want, "children {id}");
                }
            }
        }
    };
    // 09:00-18:00 Monday is 9 hours; 3 more on Tuesday end at 12:00 Seoul (03:00Z).
    check(vec![
        Some(json!("2026-12-08T03:00:00Z")),
        Some(json!("2026-12-08T05:00:00Z")),
        Some(Value::Null),
        Some(Value::Null),
    ])
    .await;

    // Tuesday becomes a holiday: every read moves the end to Wednesday together.
    let calendar = mcp_ok(
        &env,
        &token,
        "project_calendar_get",
        json!({"projectId": project_id}),
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "project_calendar_put",
        json!({"projectId": project_id, "expectedVersion": calendar["calendar"]["version"], "timeZone": "Asia/Seoul",
               "definition": {"version": 1, "week": week,
                              "holidays": [{"date": "2026-12-08", "name": "closed"}]}}),
    )
    .await;
    check(vec![
        Some(json!("2026-12-09T03:00:00Z")),
        None,
        Some(Value::Null),
        Some(Value::Null),
    ])
    .await;

    // A changed start moves it in every list too.
    let current = mcp_ok(&env, &token, "ticket_get", json!({"ticketId": ids[0]})).await;
    mcp_ok(
        &env,
        &token,
        "ticket_update",
        json!({"ticketId": ids[0], "expectedRevision": current["revisionNo"],
               "schedule": {"plannedStartAt": "2026-12-09T00:00:00Z", "durationMinutes": 60}}),
    )
    .await;
    check(vec![
        Some(json!("2026-12-09T01:00:00Z")),
        None,
        Some(Value::Null),
        Some(Value::Null),
    ])
    .await;

    // Someone who takes no part in the Project still sees nothing of it.
    let outsider = token_for(&env, &env.outsider_cookie, TICKET_SCOPES).await;
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "board_get",
            json!({"projectId": project_id})
        )
        .await,
        "not_found"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn audit_entries_made_through_mcp_name_the_client_and_grant(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let scopes = "workspace:read member:read member:write admin:read";
    let claude = token_for_client(&env, &env.owner_cookie, CLAUDE, "Claude Code", scopes).await;
    let other = token_for(&env, &env.owner_cookie, scopes).await;

    // Create through one client, reissue through another, and two at once from both.
    let created = mcp_ok(
        &env,
        &claude,
        "invitation_create",
        json!({"workspaceId": ws, "email": "first@penfii.test"}),
    )
    .await;
    let invitation = created["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &other,
        "invitation_reissue",
        json!({"workspaceId": ws, "invitationId": invitation}),
    )
    .await;
    let (a, b) = tokio::join!(
        modern_call(
            &env.app,
            &claude,
            "invitation_create",
            json!({"workspaceId": ws, "email": "a@penfii.test"})
        ),
        modern_call(
            &env.app,
            &other,
            "invitation_create",
            json!({"workspaceId": ws, "email": "b@penfii.test"})
        ),
    );
    assert_eq!(a.body["result"]["isError"], false, "{:?}", a.body);
    assert_eq!(b.body["result"]["isError"], false, "{:?}", b.body);
    mcp_ok(
        &env,
        &claude,
        "invitation_revoke",
        json!({"workspaceId": ws, "invitationId": invitation}),
    )
    .await;
    // And one in the web app.
    let by_hand = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        &env.owner_cookie,
        Some("inv-web"),
        Some(json!({"email": "web@penfii.test", "role": "MEMBER"})),
    )
    .await;
    assert_eq!(by_hand.status, 201, "{:?}", by_hand.body);
    let invite_token = by_hand.body["inviteToken"].as_str().unwrap().to_string();

    let web = web_get(
        &env,
        &format!("/v1/workspaces/{ws}/audit?action=invitation.&limit=50"),
    )
    .await;
    let mcp = mcp_ok(
        &env,
        &other,
        "audit_list",
        json!({"workspaceId": ws, "action": "invitation.", "limit": 50}),
    )
    .await;
    for (source, page) in [("web", &web), ("mcp", &mcp)] {
        let items = page["items"].as_array().unwrap();
        let find = |action: &str, label: &str| -> &Value {
            items
                .iter()
                .find(|e| e["action"] == action && e["targetLabel"] == label)
                .unwrap_or_else(|| panic!("{source}: no {action} {label}: {items:?}"))
        };
        let expect = [
            (
                "invitation.created",
                "first@penfii.test",
                Some("Claude Code"),
            ),
            (
                "invitation.reissued",
                "first@penfii.test",
                Some("Test Client"),
            ),
            ("invitation.created", "a@penfii.test", Some("Claude Code")),
            ("invitation.created", "b@penfii.test", Some("Test Client")),
            (
                "invitation.revoked",
                "first@penfii.test",
                Some("Claude Code"),
            ),
            ("invitation.created", "web@penfii.test", None),
        ];
        for (action, label, client) in expect {
            let entry = find(action, label);
            assert_eq!(
                entry["via"].as_str(),
                client,
                "{source} {action} {label}: {entry}"
            );
            assert_eq!(
                entry["actorUserId"],
                env.owner_id.to_string(),
                "the person stays the actor"
            );
            match client {
                Some(_) => {
                    assert_eq!(entry["data"]["via"], "mcp");
                    assert!(
                        entry["data"]["grantId"].is_string()
                            && entry["data"]["clientPk"].is_string()
                    );
                    assert!(entry["data"]["mcpCallId"].is_string());
                }
                None => assert!(entry["data"].get("via").is_none(), "{entry}"),
            }
        }
        // No credential in any entry: not the invitation link, not an access token.
        let text = page.to_string();
        assert!(!text.contains(&invite_token));
        assert!(!text.contains(&claude) && !text.contains(&other));
    }

    // The entry's command is the call: its log row is the one that made it, with the same grant.
    let rows: Vec<(Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT command_id, data FROM penfii.audit_entries
         WHERE workspace_id = $1 AND action_code = 'invitation.created' AND data->>'via' = 'mcp'",
    )
    .bind(env.workspace_id)
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    for (command, data) in rows {
        let (operation, grant): (String, Uuid) =
            sqlx::query_as("SELECT operation, grant_id FROM penfii.mcp_call_log WHERE id = $1")
                .bind(command.unwrap())
                .fetch_one(&env.pool)
                .await
                .unwrap();
        assert_eq!(operation, "invitation.create");
        assert_eq!(data["grantId"], json!(grant));
        assert_eq!(data["mcpCallId"], json!(command.unwrap()));
    }

    // The mcp.* entries of high-risk calls carry their call the same way (here one the policy refuses).
    let manage = token_for_client(
        &env,
        &env.owner_cookie,
        CLAUDE,
        "Claude Code",
        "workspace:read conversation:write",
    )
    .await;
    let refused = modern_call(
        &env.app,
        &manage,
        "channel_set_visibility",
        json!({"channelId": Uuid::new_v4(), "visibility": "PUBLIC"}),
    )
    .await;
    assert_eq!(refused.body["result"]["isError"], true);
    let (command, data): (Option<Uuid>, Value) = sqlx::query_as(
        "SELECT command_id, data FROM penfii.audit_entries WHERE action_code = 'mcp.channel_set_visibility'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    let logged: String = sqlx::query_scalar("SELECT tool FROM penfii.mcp_call_log WHERE id = $1")
        .bind(command.unwrap())
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(logged, "channel_set_visibility");
    assert_eq!(data["via"], "mcp");
    assert_eq!(data["mcpCallId"], json!(command.unwrap()));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn history_recorded_before_command_ids_keeps_its_client(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = project(&env, "Earlier").await;
    let token = token_for(&env, &env.owner_cookie, TICKET_SCOPES).await;
    let ticket = web_ticket(&env, &project_id, json!({"title": "old"})).await;
    let id = ticket["id"].as_str().unwrap().to_string();
    let edited = rest(
        &env,
        "PATCH",
        &format!("/v1/tickets/{id}"),
        &env.owner_cookie,
        Some("old-edit"),
        Some(json!({"expectedRevision": 1, "title": "old, edited"})),
    )
    .await;
    assert!(edited.status.is_success(), "{:?}", edited.body);
    let comment = rest(
        &env,
        "POST",
        &format!("/v1/tickets/{id}/comments"),
        &env.owner_cookie,
        None,
        Some(json!({"body": crate::common::doc("by hand")})),
    )
    .await;
    assert!(comment.status.is_success(), "{:?}", comment.body);
    let comment_id = comment.body["id"].as_str().unwrap().to_string();
    // What a v0.4.0 server logged for an MCP edit: the Ticket and the revision it produced, with
    // the revision's command id unrelated to the call. Revision 1 is credited this way; revision 2
    // (no such row) is not, and an MCP edit of a comment written in the web app does not credit
    // the comment's creation to the client.
    let (grant, client): (Uuid, Uuid) =
        sqlx::query_as("SELECT id, client_pk FROM penfii.oauth_grants LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    for (resource_type, resource_id, revision, operation) in [
        ("ticket", id.clone(), Some(1_i64), "ticket.update"),
        (
            "ticket_comment",
            comment_id.clone(),
            None,
            "ticket.comment_edit",
        ),
    ] {
        sqlx::query(
            "INSERT INTO penfii.mcp_call_log (id, user_id, grant_id, client_pk, tool, operation,
                 risk_class, workspace_id, resource_type, resource_id, resource_revision, request_id, outcome)
             VALUES ($1, $2, $3, $4, 'x', $5, 'write', $6, $7, $8, $9, 'r', 'ok')",
        )
        .bind(Uuid::new_v4())
        .bind(env.owner_id)
        .bind(grant)
        .bind(client)
        .bind(operation)
        .bind(env.workspace_id)
        .bind(resource_type)
        .bind(resource_id)
        .bind(revision)
        .execute(&env.pool)
        .await
        .unwrap();
    }
    assert_eq!(
        revision_via(&env, &id).await,
        vec![(2, None), (1, Some("Test Client".into()))]
    );
    let listed = mcp_ok(
        &env,
        &token,
        "ticket_activity_list",
        json!({"ticketId": id}),
    )
    .await;
    let created = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["activityKind"] == "ticket.created")
        .unwrap()
        .clone();
    assert_eq!(created["via"], "Test Client");
    let commented = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["activityKind"] == "ticket.commented")
        .unwrap()
        .clone();
    assert!(commented["via"].is_null(), "{commented}");
}
