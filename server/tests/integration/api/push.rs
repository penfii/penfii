//! Web Push: subscriptions belong to a person, and only the directed kinds of Inbox item are pushed.

use std::sync::{Arc, Mutex};

use super::access::grant_user;
use super::inbox::{attention_now, member, mention_doc, send_doc};
use super::{World, call, channel, get_as, make_ticket, new_project, workspace};
use crate::common::doc;
use axum::http::StatusCode;
use penfii_core::push::{Outcome, PushFuture, PushService, PushTransport, Subscription};
use serde_json::{Value, json};
use uuid::Uuid;

const P256DH: &str =
    "BLn9b-VR0ca83knDNZ32dCHGyjJp-1riX9ZTN40MqV8K_LpQmLqxC_DoHvqvFXO_nGdAB4W9dogZb_sM-uV4JbY";
const AUTH: &str = "_ordMnz7uTCmrpBTeUV4Bw";

/// Records what would be sent and answers as told.
#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<(String, Value)>>,
    outcome: Mutex<Option<Outcome>>,
}

impl PushTransport for Recorder {
    fn send<'a>(&'a self, subscription: &'a Subscription, payload: &'a [u8]) -> PushFuture<'a> {
        let body: Value = serde_json::from_slice(payload).unwrap();
        self.sent
            .lock()
            .unwrap()
            .push((subscription.endpoint.clone(), body));
        let outcome = self
            .outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Outcome::Delivered);
        Box::pin(async move { outcome })
    }
}

impl Recorder {
    fn sent(&self) -> Vec<(String, Value)> {
        self.sent.lock().unwrap().clone()
    }
}

fn body(endpoint: &str) -> Value {
    json!({"endpoint": endpoint, "keys": {"p256dh": P256DH, "auth": AUTH}, "userAgent": "Test/1", "label": "Laptop"})
}

async fn subscribe(world: &World, cookie: &str, endpoint: &str) -> (StatusCode, Value) {
    let (s, v, _) = call(
        &world.app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(cookie),
        None,
        Some(body(endpoint)),
    )
    .await;
    (s, v)
}

/// The router with Web Push on and the transport that records.
fn push_world(world: &World) -> (axum::Router, Arc<Recorder>, Arc<PushService>) {
    let recorder = Arc::new(Recorder::default());
    let service =
        PushService::with_transport(world.pool.clone(), "test-public-key", recorder.clone());
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        push: Some(service.clone()),
        ..penfii_http_api::RouterConfig::new(world.pool.clone(), crate::common::limits(), false)
    });
    (app, recorder, service)
}

async fn attention_with_push(ws: Uuid, world: &World, service: &Arc<PushService>) {
    let mut registry =
        penfii_core::consumers::registry_with_push(None, None, Some(service.clone()), None);
    registry.workspace = Some(ws);
    registry.backoff_base_secs = 0;
    registry.max_backoff_secs = 0;
    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    // The pushes the pass dispatched have been sent (and their results recorded) when this returns.
    service.wait_idle().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn subscriptions_belong_to_the_person_and_the_key_is_public_only_when_configured(
    pool: sqlx::PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("push-sub-{}", Uuid::new_v4())).await;
    let (_, m) = member(world, ws, "m").await;
    let (_, x) = member(world, ws, "x").await;

    // Off by default: the key says so and subscribing is refused.
    let (s, off) = get_as(world, &m, "/v1/push/vapid-public-key").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(off, json!({"configured": false, "publicKey": null}));
    let (s, refused) = subscribe(world, &m, "https://push.example.com/a").await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{refused}");

    let (app, _, _) = push_world(world);
    let on = |method: &'static str, path: String, cookie: String, payload: Option<Value>| {
        let app = app.clone();
        async move {
            let (s, v, _) = call(&app, method, &path, Some(&cookie), None, payload).await;
            (s, v)
        }
    };
    let (_, key) = on("GET", "/v1/push/vapid-public-key".into(), m.clone(), None).await;
    assert_eq!(
        key,
        json!({"configured": true, "publicKey": "test-public-key"})
    );

    // A person may have several devices; the same endpoint again updates, it does not duplicate.
    let (s, first) = on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        m.clone(),
        Some(body("https://push.example.com/a")),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert!(
        first.get("keys").is_none() && first.get("p256dh").is_none(),
        "keys are not returned: {first}"
    );
    let (_, again) = on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        m.clone(),
        Some(body("https://push.example.com/a")),
    )
    .await;
    assert_eq!(again["id"], first["id"]);
    on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        m.clone(),
        Some(body("https://push.example.com/b")),
    )
    .await;
    let (_, mine) = on("GET", "/v1/me/push-subscriptions".into(), m.clone(), None).await;
    assert_eq!(mine["items"].as_array().unwrap().len(), 2);

    // Nobody else sees or removes them.
    let (_, theirs) = on("GET", "/v1/me/push-subscriptions".into(), x.clone(), None).await;
    assert!(theirs["items"].as_array().unwrap().is_empty());
    let id = first["id"].as_str().unwrap();
    let (s, _) = on(
        "DELETE",
        format!("/v1/me/push-subscriptions/{id}"),
        x.clone(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Not an https URL, or keys that are not keys.
    for bad in [
        json!({"endpoint": "http://push.example.com/a", "keys": {"p256dh": P256DH, "auth": AUTH}}),
        // The server would call this address, so it must be a public name.
        json!({"endpoint": "https://127.0.0.1/a", "keys": {"p256dh": P256DH, "auth": AUTH}}),
        json!({"endpoint": "https://169.254.169.254/latest", "keys": {"p256dh": P256DH, "auth": AUTH}}),
        json!({"endpoint": "https://localhost/a", "keys": {"p256dh": P256DH, "auth": AUTH}}),
        json!({"endpoint": "https://db.internal/a", "keys": {"p256dh": P256DH, "auth": AUTH}}),
        json!({"endpoint": "https://push.example.com/c", "keys": {"p256dh": "AAAA", "auth": AUTH}}),
        json!({"endpoint": "https://push.example.com/c", "keys": {"p256dh": P256DH, "auth": "AAAA"}}),
    ] {
        let (s, v) = on(
            "POST",
            "/v1/me/push-subscriptions".into(),
            m.clone(),
            Some(bad),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }

    // One person cannot register without limit.
    let (_, many) = member(world, ws, "many").await;
    for n in 0..20 {
        let (s, v) = on(
            "POST",
            "/v1/me/push-subscriptions".into(),
            many.clone(),
            Some(body(&format!("https://push.example.com/many{n}"))),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (s, v) = on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        many.clone(),
        Some(body("https://push.example.com/many-extra")),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    // The same device again is not a new one.
    let (s, _) = on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        many,
        Some(body("https://push.example.com/many0")),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Signing out (ending every session) does not remove a subscription.
    let (k_id, k) = member(world, ws, "k").await;
    on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        k.clone(),
        Some(body("https://push.example.com/k")),
    )
    .await;
    sqlx::query("DELETE FROM penfii.sessions WHERE user_id=$1")
        .bind(k_id)
        .execute(&world.pool)
        .await
        .unwrap();
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.push_subscriptions WHERE user_id=$1")
            .bind(k_id)
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(kept, 1);

    // The owner removes their own; a browser that signs in as someone else takes the endpoint over.
    let (s, _) = on(
        "DELETE",
        format!("/v1/me/push-subscriptions/{id}"),
        m.clone(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, taken) = on(
        "POST",
        "/v1/me/push-subscriptions".into(),
        x.clone(),
        Some(body("https://push.example.com/b")),
    )
    .await;
    assert!(taken["id"].is_string());
    let (_, mine) = on("GET", "/v1/me/push-subscriptions".into(), m, None).await;
    assert!(mine["items"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_assignment_is_pushed_once_a_comment_never_and_nothing_during_quiet_hours(
    pool: sqlx::PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("push-send-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (project_id, _) = new_project(world, ws, "Push tickets").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    let (app, recorder, service) = push_world(world);
    let (s, _, _) = call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/m1")),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/m2")),
    )
    .await;

    // An assignment is pushed to every device, with no text of the Task and no workspace name.
    let (s, t) = make_ticket(
        world,
        &project_id,
        "push-a",
        json!({"title": "Secret launch plan", "assigneeId": m_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    attention_with_push(ws, world, &service).await;
    let sent = recorder.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    let message = &sent[0].1;
    assert_eq!(message["kind"], "ticket.assigned");
    let text = message.to_string();
    assert!(!text.contains("Secret launch plan"), "{text}");
    assert!(
        message["path"].as_str().unwrap().contains("panel=ticket:"),
        "{text}"
    );
    assert!(message["id"].is_string());

    // The same event handled again does not push again.
    sqlx::query("UPDATE penfii.consumer_receipts SET state='PENDING' WHERE consumer='inbox'")
        .execute(&world.pool)
        .await
        .unwrap();
    attention_with_push(ws, world, &service).await;
    assert_eq!(recorder.sent().len(), 2);

    // A kind that is not in the list reaches the Inbox but not the phone (a comment on a followed Task).
    let ticket_id = t["id"].as_str().unwrap();
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{ticket_id}/comments"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"body": crate::common::doc("hello")})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    attention_with_push(ws, world, &service).await;
    assert_eq!(recorder.sent().len(), 2, "a comment is not pushed");
    assert!(!super::inbox::kinds(&super::inbox::inbox(world, &m, ws, "").await).is_empty());

    // Quiet hours: the Inbox item is made, nothing is sent, and nothing is sent afterwards either.
    let (s, p, _) = call(&app, "PUT", "/v1/me/notification-preferences", Some(&m), None, Some(json!({"level": "ALL", "dnd": {"enabled": true, "schedule": null, "snoozeUntil": null}, "external": {}, "expectedVersion": 0}))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    make_ticket(
        world,
        &project_id,
        "push-b",
        json!({"title": "During quiet hours", "assigneeId": m_id}),
    )
    .await;
    attention_with_push(ws, world, &service).await;
    assert_eq!(recorder.sent().len(), 2, "DND sends nothing");
    let items = super::inbox::inbox(world, &m, ws, "?filter=assignments").await;
    assert_eq!(
        items["items"].as_array().unwrap().len(),
        2,
        "the Inbox still has it"
    );
    let (_, p2, _) = call(
        &app,
        "GET",
        "/v1/me/notification-preferences",
        Some(&m),
        None,
        None,
    )
    .await;
    call(&app, "PUT", "/v1/me/notification-preferences", Some(&m), None, Some(json!({"level": "ALL", "dnd": {"enabled": false, "schedule": null, "snoozeUntil": null}, "external": {}, "expectedVersion": p2["version"]}))).await;
    attention_with_push(ws, world, &service).await;
    assert_eq!(
        recorder.sent().len(),
        2,
        "no catch-up after quiet hours end"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_gone_subscription_is_removed_and_a_disabled_account_is_skipped(pool: sqlx::PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("push-gone-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (project_id, _) = new_project(world, ws, "Push cleanup").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    let (app, recorder, service) = push_world(world);
    call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/gone")),
    )
    .await;

    // The push service says the subscription is gone (410): the row is deleted.
    *recorder.outcome.lock().unwrap() = Some(Outcome::Gone);
    make_ticket(
        world,
        &project_id,
        "push-g1",
        json!({"title": "One", "assigneeId": m_id}),
    )
    .await;
    attention_with_push(ws, world, &service).await;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.push_subscriptions")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);

    // A temporary failure is tried again, then given up; the subscription stays and the failure is noted.
    *recorder.outcome.lock().unwrap() = Some(Outcome::Retry("status 503".into()));
    call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/slow")),
    )
    .await;
    let before = recorder.sent().len();
    make_ticket(
        world,
        &project_id,
        "push-g2",
        json!({"title": "Two", "assigneeId": m_id}),
    )
    .await;
    attention_with_push(ws, world, &service).await;
    assert_eq!(
        recorder.sent().len() - before,
        3,
        "one send and two retries"
    );
    // The failure is noted when the batch has been sent, which `attention_with_push` waited for.
    let failed: bool = sqlx::query_scalar(
        "SELECT COALESCE(bool_or(last_failure_at IS NOT NULL), false) FROM penfii.push_subscriptions",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert!(failed);

    // A disabled account keeps the row but is not sent to.
    *recorder.outcome.lock().unwrap() = None;
    sqlx::query("UPDATE penfii.users SET disabled_at=now() WHERE id=$1")
        .bind(m_id)
        .execute(&world.pool)
        .await
        .unwrap();
    let before = recorder.sent().len();
    make_ticket(
        world,
        &project_id,
        "push-g3",
        json!({"title": "Three", "assigneeId": m_id}),
    )
    .await;
    attention_with_push(ws, world, &service).await;
    assert_eq!(recorder.sent().len(), before);
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.push_subscriptions")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(kept, 1);
    let _ = attention_now;
}

async fn post(
    world: &World,
    cookie: &str,
    conversation: Uuid,
    body: Value,
    root: Option<Uuid>,
    reply: Option<Uuid>,
) -> Uuid {
    let sent = send_doc(world, cookie, conversation, body, root, reply).await;
    sent["id"].as_str().unwrap().parse().unwrap()
}

async fn dm_between(world: &World, ws: Uuid, other: Uuid) -> Uuid {
    let (s, v, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/dms"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"userIds": [world.owner_id, other]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().parse().unwrap()
}

fn kinds_sent(recorder: &Recorder) -> Vec<String> {
    let mut seen: Vec<String> = recorder
        .sent()
        .iter()
        .map(|(_, m)| m["kind"].as_str().unwrap().to_owned())
        .collect();
    seen.dedup();
    seen
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn dms_mentions_replies_threads_assignments_and_reminders_are_pushed_but_channel_chatter_is_not(
    pool: sqlx::PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("push-dir-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (app, recorder, service) = push_world(world);
    let (s, _, _) = call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/dir")),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let general = channel(world, ws, "chatter", "PUBLIC").await;
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/channels/{general}/join"),
        Some(&m),
        None,
        None,
    )
    .await;
    assert!(s.is_success(), "{s}");
    let dm = dm_between(world, ws, m_id).await;
    let owner = world.owner_cookie.clone();
    let step = |_expected: usize| {
        let service = service.clone();
        let recorder = recorder.clone();
        async move {
            attention_with_push(ws, world, &service).await;
            recorder.sent().len()
        }
    };

    // Plain channel message: nothing in the Inbox, nothing pushed.
    post(world, &owner, general, doc("just talking"), None, None).await;
    assert_eq!(step(1).await, 0, "channel chatter is not pushed");

    // A new message in a DM: an Inbox item and a push, opening that DM.
    post(world, &owner, dm, doc("hello"), None, None).await;
    assert_eq!(step(1).await, 1);
    let first = recorder.sent()[0].1.clone();
    assert_eq!(first["kind"], "dm.message");
    assert_eq!(first["path"], format!("/w/{ws}/c/{dm}"));
    assert!(!first.to_string().contains("hello"));
    let items = super::inbox::inbox(world, &m, ws, "").await;
    assert_eq!(super::inbox::kinds(&items), vec!["dm.message"]);
    // The sender is not told about their own message.
    let own = super::inbox::inbox(world, &owner, ws, "").await;
    assert!(super::inbox::kinds(&own).is_empty(), "{own}");

    // The same event handled again does not push again.
    sqlx::query("UPDATE penfii.consumer_receipts SET state='PENDING' WHERE consumer='inbox'")
        .execute(&world.pool)
        .await
        .unwrap();
    assert_eq!(step(2).await, 1, "no second push for the same event");

    // A mention in a channel.
    post(world, &owner, general, mention_doc(m_id, " hi"), None, None).await;
    assert_eq!(step(2).await, 2);
    assert_eq!(recorder.sent()[1].1["kind"], "mention.user");

    // A direct reply to m's own message (inside a thread, as the app does it).
    let thread_root = post(world, &owner, general, doc("a thread"), None, None).await;
    let mine = post(
        world,
        &m,
        general,
        doc("my answer"),
        Some(thread_root),
        None,
    )
    .await;
    post(
        world,
        &owner,
        general,
        doc("answer"),
        Some(thread_root),
        Some(mine),
    )
    .await;
    assert_eq!(step(3).await, 3);
    assert_eq!(recorder.sent()[2].1["kind"], "reply.direct");

    // A reply in the thread m joined by answering in it.
    post(
        world,
        &owner,
        general,
        doc("in the thread"),
        Some(thread_root),
        None,
    )
    .await;
    assert_eq!(step(4).await, 4);
    let thread = recorder.sent()[3].1.clone();
    assert_eq!(thread["kind"], "thread.reply");
    assert_eq!(
        thread["path"],
        format!("/w/{ws}/c/{general}/t/{thread_root}")
    );

    // A Task assigned to m.
    let (project_id, _) = new_project(world, ws, "Push directed").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    make_ticket(
        world,
        &project_id,
        "pd-a",
        json!({"title": "Assigned", "assigneeId": m_id}),
    )
    .await;
    assert_eq!(step(5).await, 5);
    assert_eq!(recorder.sent()[4].1["kind"], "ticket.assigned");

    // A Reminder that is due.
    let (_, ticket) = make_ticket(world, &project_id, "pd-r", json!({"title": "Remind"})).await;
    let (s, later, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/later"),
        Some(&m),
        None,
        Some(json!({"kind":"TICKET","targetId":ticket["id"],"note":null})),
    )
    .await;
    assert!(s.is_success(), "{later}");
    let later_id = later["id"].as_str().unwrap();
    let due = chrono::Utc::now() + chrono::Duration::hours(1);
    let (s, r, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/later/{later_id}/reminder"),
        Some(&m),
        None,
        Some(json!({"dueAt": due.to_rfc3339(), "timeZone": "Asia/Seoul"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    sqlx::query("UPDATE penfii.reminders SET due_at=now()-interval '1 minute' WHERE id=$1")
        .bind(Uuid::parse_str(r["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    penfii_core::reminders::fire_due(&world.pool, 50, Some(ws))
        .await
        .unwrap();
    assert_eq!(step(6).await, 6);
    assert_eq!(kinds_sent(&recorder).last().unwrap(), "reminder.due");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn direct_only_and_a_muted_channel_do_not_hold_back_dms_and_threads_but_quiet_hours_do(
    pool: sqlx::PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("push-dnd-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (app, recorder, service) = push_world(world);
    call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/dnd")),
    )
    .await;
    let general = channel(world, ws, "muted", "PUBLIC").await;
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/channels/{general}/join"),
        Some(&m),
        None,
        None,
    )
    .await;
    assert!(s.is_success(), "{s}");
    let dm = dm_between(world, ws, m_id).await;
    let owner = world.owner_cookie.clone();
    let root = post(world, &m, general, doc("root"), None, None).await;
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/v1/channels/{general}/mute"),
        Some(&m),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, p, _) = call(&app, "PUT", "/v1/me/notification-preferences", Some(&m), None,
        Some(json!({"level":"DIRECT_ONLY","dnd":{"enabled":false,"schedule":null,"snoozeUntil":null},"external":{},"expectedVersion":0}))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let step = || async {
        attention_with_push(ws, world, &service).await;
        recorder.sent().len()
    };
    assert_eq!(step().await, 0);

    post(world, &owner, dm, doc("dm while direct-only"), None, None).await;
    assert_eq!(step().await, 1, "a DM is pushed at DIRECT_ONLY");
    post(
        world,
        &owner,
        general,
        doc("thread reply in a muted channel"),
        Some(root),
        None,
    )
    .await;
    assert_eq!(
        step().await,
        2,
        "a thread the person is in is pushed although the channel is muted"
    );
    assert_eq!(kinds_sent(&recorder), vec!["dm.message", "thread.reply"]);

    // Quiet hours: the Inbox gets the item, the phone gets nothing, and nothing comes later.
    let (_, prefs, _) = call(
        &app,
        "GET",
        "/v1/me/notification-preferences",
        Some(&m),
        None,
        None,
    )
    .await;
    let (s, p, _) = call(&app, "PUT", "/v1/me/notification-preferences", Some(&m), None,
        Some(json!({"level":"DIRECT_ONLY","dnd":{"enabled":true,"schedule":null,"snoozeUntil":null},"external":{},"expectedVersion":prefs["version"]}))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let before = super::inbox::kinds(&super::inbox::inbox(world, &m, ws, "").await).len();
    post(world, &owner, dm, doc("dm in quiet hours"), None, None).await;
    assert_eq!(step().await, 2, "DND sends nothing");
    let after = super::inbox::kinds(&super::inbox::inbox(world, &m, ws, "").await).len();
    assert_eq!(after, before + 1, "the Inbox still has it");
    let (_, prefs, _) = call(
        &app,
        "GET",
        "/v1/me/notification-preferences",
        Some(&m),
        None,
        None,
    )
    .await;
    call(&app, "PUT", "/v1/me/notification-preferences", Some(&m), None,
        Some(json!({"level":"DIRECT_ONLY","dnd":{"enabled":false,"schedule":null,"snoozeUntil":null},"external":{},"expectedVersion":prefs["version"]}))).await;
    assert_eq!(step().await, 2, "no catch-up after quiet hours end");
}

/// A push service that never answers.
#[derive(Default)]
struct Stuck {
    reached: tokio::sync::Notify,
}

impl PushTransport for Stuck {
    fn send<'a>(&'a self, _subscription: &'a Subscription, _payload: &'a [u8]) -> PushFuture<'a> {
        Box::pin(async move {
            self.reached.notify_one();
            std::future::pending::<Outcome>().await
        })
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_stop_cancels_the_pushes_on_their_way_and_a_burst_is_capped(pool: sqlx::PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("push-stop-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let stuck = Arc::new(Stuck::default());
    let service = PushService::with_transport(world.pool.clone(), "test-public-key", stuck.clone());
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        push: Some(service.clone()),
        ..penfii_http_api::RouterConfig::new(world.pool.clone(), crate::common::limits(), false)
    });
    let (s, _, _) = call(
        &app,
        "POST",
        "/v1/me/push-subscriptions",
        Some(&m),
        None,
        Some(body("https://push.example.com/stuck")),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // A direct message makes an Inbox item and one batch of pushes, which the push service never answers.
    let dm = dm_between(world, ws, m_id).await;
    post(world, &world.owner_cookie, dm, doc("hello"), None, None).await;
    let mut registry =
        penfii_core::consumers::registry_with_push(None, None, Some(service.clone()), None);
    registry.workspace = Some(ws);
    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), stuck.reached.notified())
        .await
        .expect("the batch reached the push service");
    let item: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.inbox_items WHERE recipient_id = $1")
            .bind(m_id)
            .fetch_one(&world.pool)
            .await
            .unwrap();

    // A burst: one batch is already on its way, so MAX_BATCHES_IN_FLIGHT - 1 more are taken and the rest dropped.
    let job = || penfii_core::push::PushJob {
        recipient: m_id,
        inbox_item_id: item,
        payload: json!({}),
    };
    for _ in 0..(penfii_core::push::MAX_BATCHES_IN_FLIGHT + 6) {
        service.dispatch(vec![job()]);
    }

    // The stop gives them a moment, then ends them, and nothing is sent after it.
    let cancelled = service.drain(std::time::Duration::from_millis(100)).await;
    assert_eq!(cancelled, penfii_core::push::MAX_BATCHES_IN_FLIGHT);
    service.dispatch(vec![job()]);
    service.wait_idle().await;
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.push_subscriptions")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(
        open, 1,
        "the pool still works and the subscription is untouched"
    );
}
