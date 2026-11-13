//! Inbox delivery, watching and what the Inbox hides.

use super::access::grant_user;
use super::{
    World, call, channel, get_as, make_ticket, new_project, post_as, user_id_of, viewer_cookie,
    workspace,
};
use crate::common::doc;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub(super) async fn attention_now(workspace_id: Uuid, world: &World) {
    let mut registry = penfii_core::consumers::registry(None, None);
    registry.workspace = Some(workspace_id);
    registry.backoff_base_secs = 0;
    registry.max_backoff_secs = 0;
    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
}

pub(super) async fn member(world: &World, workspace_id: Uuid, tag: &str) -> (Uuid, String) {
    let email = format!("{tag}-{}@penfii.test", Uuid::new_v4());
    let cookie = viewer_cookie(world, workspace_id, &email).await;
    (user_id_of(world, &email).await, cookie)
}

pub(super) fn mention_doc(user: Uuid, text: &str) -> Value {
    json!({"type":"doc","content":[{"type":"paragraph","content":[
        {"type":"mention","attrs":{"occurrenceId":Uuid::new_v4(),"kind":"USER","userId":user}},
        {"type":"text","text":text}]}]})
}

pub(super) async fn send_doc(
    world: &World,
    cookie: &str,
    conversation: Uuid,
    body: Value,
    root: Option<Uuid>,
    reply: Option<Uuid>,
) -> Value {
    let (s, v, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation}/messages"),
        Some(cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"bodySchemaVersion":1,"body":body,"threadRootId":root,"replyToId":reply})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

pub(super) async fn inbox(world: &World, cookie: &str, workspace_id: Uuid, query: &str) -> Value {
    let (s, v) = get_as(
        world,
        cookie,
        &format!("/v1/workspaces/{workspace_id}/inbox{query}"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// The `thread.reply` items a person has. They are `replies` now; `watching` is no longer a type.
async fn thread_replies(world: &World, cookie: &str, workspace_id: Uuid) -> Vec<String> {
    kinds(&inbox(world, cookie, workspace_id, "").await)
        .into_iter()
        .filter(|k| k == "thread.reply")
        .collect()
}

pub(super) fn kinds(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn inbox_reaches_the_right_people_once_for_messages_and_threads(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-ib-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (n_id, n) = member(world, ws, "n").await;
    let o = world.owner_cookie.clone();
    let general = channel(world, ws, "general", "PUBLIC").await;
    for cookie in [&m, &n] {
        let (s, v, _) = call(
            &world.app,
            "POST",
            &format!("/v1/channels/{general}/join"),
            Some(cookie),
            None,
            None,
        )
        .await;
        assert!(s.is_success(), "{v}");
    }

    // A mention reaches the person, not the author.
    let first = send_doc(
        world,
        &o,
        general,
        mention_doc(m_id, " 확인 부탁"),
        None,
        None,
    )
    .await;
    attention_now(ws, world).await;
    let page = inbox(world, &m, ws, "").await;
    assert_eq!(kinds(&page), vec!["mention.user"], "{page}");
    assert_eq!(page["unreadCount"], 1);
    assert_eq!(page["items"][0]["subject"]["kind"], "MESSAGE");
    assert!(
        page["items"][0]["subject"]["title"]
            .as_str()
            .unwrap()
            .contains("확인 부탁")
    );
    assert_eq!(page["items"][0]["actor"]["name"], "Owner");
    assert_eq!(
        kinds(&inbox(world, &o, ws, "").await),
        Vec::<String>::new(),
        "no notice of your own action"
    );
    assert!(kinds(&inbox(world, &n, ws, "").await).is_empty());

    // Handling the same event again changes nothing.
    sqlx::query("UPDATE penfii.consumer_receipts r SET state='PENDING' FROM penfii.domain_events e WHERE e.event_id=r.event_id AND e.workspace_id=$1").bind(ws).execute(&world.pool).await.unwrap();
    attention_now(ws, world).await;
    assert_eq!(
        inbox(world, &m, ws, "").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A reply reaches the author of what was answered; a reply that also mentions is one item,
    // and the mention is the stronger reason.
    let first_id: Uuid = first["id"].as_str().unwrap().parse().unwrap();
    let answer = send_doc(
        world,
        &m,
        general,
        doc("답변"),
        Some(first_id),
        Some(first_id),
    )
    .await;
    let answer_id: Uuid = answer["id"].as_str().unwrap().parse().unwrap();
    send_doc(
        world,
        &n,
        general,
        mention_doc(m_id, " 같이 봐요"),
        Some(first_id),
        Some(answer_id),
    )
    .await;
    attention_now(ws, world).await;
    let mut replies = kinds(&inbox(world, &o, ws, "?filter=replies").await);
    replies.sort();
    assert_eq!(
        replies,
        vec!["reply.direct", "thread.reply"],
        "O answered directly, then follows the thread"
    );
    let mine = inbox(world, &m, ws, "").await;
    assert_eq!(kinds(&mine), vec!["mention.user", "mention.user"], "{mine}");

    // An edit that adds a mention notifies the new person once, however often it is saved.
    let (s, edited, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/messages/{first_id}"),
        Some(&o),
        Some("p62-ib-edit-1"),
        Some(json!({"expectedVersion":1,"bodySchemaVersion":1,"body":mention_doc(n_id," + n")})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{edited}");
    attention_now(ws, world).await;
    assert_eq!(
        kinds(&inbox(world, &n, ws, "?filter=mentions").await),
        vec!["mention.user"]
    );
    let (s, _, _) = call(&world.app, "PATCH", &format!("/v1/messages/{first_id}"), Some(&o), Some("p62-ib-edit-2"),
        Some(json!({"expectedVersion":2,"bodySchemaVersion":1,"body":mention_doc(n_id," + n again")}))).await;
    assert_eq!(s, StatusCode::OK);
    attention_now(ws, world).await;
    assert_eq!(
        inbox(world, &n, ws, "?filter=mentions").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "once per message and person"
    );

    // Threads: taking part follows the thread; an explicit unwatch is never undone by activity.
    let root = send_doc(world, &o, general, doc("스레드 시작"), None, None).await;
    let root_id: Uuid = root["id"].as_str().unwrap().parse().unwrap();
    send_doc(world, &m, general, doc("m 답글"), Some(root_id), None).await;
    attention_now(ws, world).await;
    send_doc(world, &n, general, doc("n 답글"), Some(root_id), None).await;
    attention_now(ws, world).await;
    let watch_page = thread_replies(world, &o, ws).await;
    assert_eq!(
        watch_page,
        vec!["thread.reply", "thread.reply", "thread.reply"],
        "the starter follows both of their threads: {watch_page:?}"
    );
    assert_eq!(
        thread_replies(world, &m, ws).await,
        vec!["thread.reply"],
        "m follows after replying, hears n only"
    );
    let (s, v) = {
        let (s, v, _) = call(
            &world.app,
            "DELETE",
            &format!("/v1/messages/{root_id}/subscription"),
            Some(&m),
            None,
            None,
        )
        .await;
        (s, v)
    };
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["watching"], false);
    send_doc(world, &m, general, doc("m 또 답글"), Some(root_id), None).await;
    send_doc(world, &n, general, doc("n 또 답글"), Some(root_id), None).await;
    attention_now(ws, world).await;
    let (_, status) = get_as(world, &m, &format!("/v1/messages/{root_id}/subscription")).await;
    assert_eq!(
        status["mode"], "OPT_OUT",
        "later replies do not undo the refusal: {status}"
    );
    assert_eq!(
        thread_replies(world, &m, ws).await.len(),
        1,
        "nothing new after unwatching"
    );

    // Mute: it does not hold back a thread the person is in (that is addressed to them), and a direct mention arrives too.
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/channels/{general}/mute"),
        Some(&o),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let before = thread_replies(world, &o, ws).await.len();
    send_doc(world, &n, general, doc("muted reply"), Some(root_id), None).await;
    send_doc(
        world,
        &n,
        general,
        mention_doc(
            Uuid::parse_str(&world.owner_id.to_string()).unwrap(),
            " 직접 멘션",
        ),
        None,
        None,
    )
    .await;
    attention_now(ws, world).await;
    assert_eq!(thread_replies(world, &o, ws).await.len(), before + 1);
    assert_eq!(
        kinds(&inbox(world, &o, ws, "?filter=mentions").await),
        vec!["mention.user"]
    );

    // Level DIRECT_ONLY keeps the direct kinds, and a thread reply to someone in the thread is one.
    let (s, p, _) = call(&world.app, "PUT", "/v1/me/notification-preferences", Some(&n), None,
        Some(json!({"level":"DIRECT_ONLY","dnd":{"enabled":false,"schedule":null,"snoozeUntil":null},"external":{},"expectedVersion":0}))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let n_before = thread_replies(world, &n, ws).await.len();
    send_doc(world, &o, general, doc("o again"), Some(root_id), None).await;
    attention_now(ws, world).await;
    assert_eq!(thread_replies(world, &n, ws).await.len(), n_before + 1);
    let (s, stale, _) = call(&world.app, "PUT", "/v1/me/notification-preferences", Some(&n), None,
        Some(json!({"level":"ALL","dnd":{"enabled":false,"schedule":null,"snoozeUntil":null},"external":{},"expectedVersion":0}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn inbox_items_can_be_read_snoozed_and_archived_and_hide_what_you_may_no_longer_see(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-ib2-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (x_id, outsider) = member(world, ws, "x").await;
    let o = world.owner_cookie.clone();
    let (project_id, workflow) = new_project(world, ws, "Inbox tickets").await;
    grant_user(world, &project_id, x_id, "VIEWER").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    let done = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Assignment reaches the assignee (not the author) and is independent of Watch.
    let (s, a) = make_ticket(
        world,
        &project_id,
        "p62ib-a",
        json!({"title":"Task A","assigneeId":m_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    let (_, b) = make_ticket(
        world,
        &project_id,
        "p62ib-b",
        json!({"title":"Task B","assigneeId":m_id}),
    )
    .await;
    let (a_id, b_id) = (
        a["id"].as_str().unwrap().to_owned(),
        b["id"].as_str().unwrap().to_owned(),
    );
    attention_now(ws, world).await;
    let page = inbox(world, &m, ws, "?filter=assignments").await;
    assert_eq!(
        kinds(&page),
        vec!["ticket.assigned", "ticket.assigned"],
        "{page}"
    );
    assert!(kinds(&inbox(world, &o, ws, "").await).is_empty());
    // The assignee now follows both Tickets.
    let (_, st) = get_as(world, &m, &format!("/v1/tickets/{a_id}/subscription")).await;
    assert_eq!(st["watching"], true, "{st}");

    // B waits for A. Finishing A frees B for its assignee; comments, State and field changes reach followers.
    let (s, _) = post_as(
        world,
        &o,
        &format!("/v1/tickets/{b_id}/dependencies"),
        "p62ib-dep",
        json!({"predecessorId":a_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    // A third person chooses to follow A.
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/tickets/{a_id}/subscription"),
        Some(&outsider),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, c, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{a_id}/comments"),
        Some(&o),
        None,
        Some(json!({"body":doc("진행 상황 공유")})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let (s, e, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/tickets/{a_id}"),
        Some(&o),
        Some("p62ib-edit"),
        Some(json!({"expectedRevision":1,"priority":"URGENT"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    let (s, mv, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{a_id}/commands/move"),
        Some(&o),
        Some("p62ib-done"),
        Some(json!({"expectedRevision":2,"statusId":done})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{mv}");
    attention_now(ws, world).await;
    let all = kinds(&inbox(world, &m, ws, "").await);
    for expected in [
        "ticket.commented",
        "ticket.field_changed",
        "dependency.released",
    ] {
        assert!(all.contains(&expected.to_owned()), "{expected} in {all:?}");
    }
    // Every Ticket item names the Project, whatever kind of notification it is (a comment does not
    // record it when it is made); a link without it opens nothing.
    for person in [&m, &outsider] {
        let page = inbox(world, person, ws, "").await;
        for item in page["items"].as_array().unwrap() {
            if item["link"]["type"] == "ticket" {
                assert_eq!(item["link"]["projectId"], json!(project_id), "{item}");
            }
        }
    }
    // One reason per person per event: finishing A frees B for its assignee (the stronger reason).
    assert!(
        !all.contains(&"ticket.status_changed".to_owned()),
        "{all:?}"
    );
    let follower = kinds(&inbox(world, &outsider, ws, "").await);
    for expected in [
        "ticket.commented",
        "ticket.field_changed",
        "ticket.status_changed",
    ] {
        assert!(
            follower.contains(&expected.to_owned()),
            "{expected} in {follower:?}"
        );
    }
    assert!(
        !follower.contains(&"dependency.released".to_owned()),
        "{follower:?}"
    );
    // The author followed what they created, but they made every change themselves.
    assert!(kinds(&inbox(world, &o, ws, "").await).is_empty());
    // The commenter follows the Ticket.
    let (_, st) = get_as(world, &o, &format!("/v1/tickets/{a_id}/subscription")).await;
    assert_eq!(st["watching"], true);

    // A Sprint start reaches the Project's followers.
    let (s, _) = {
        let (s, v, _) = call(
            &world.app,
            "PUT",
            &format!("/v1/projects/{project_id}/subscription"),
            Some(&m),
            None,
            None,
        )
        .await;
        (s, v)
    };
    assert_eq!(s, StatusCode::OK);
    let (s, sprint) = post_as(
        world,
        &o,
        &format!("/v1/projects/{project_id}/sprints"),
        "p62ib-sp",
        json!({"name":"Sprint X","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sprint}");
    let sprint_id = sprint["id"].as_str().unwrap();
    let (s, v) = post_as(
        world,
        &o,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "p62ib-sp-start",
        json!({"expectedVersion":1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    attention_now(ws, world).await;
    assert!(
        kinds(&inbox(world, &m, ws, "?filter=watching").await)
            .contains(&"sprint.started".to_owned())
    );

    // Commands: read, unread, snooze, archive; stale versions are refused.
    let page = inbox(world, &m, ws, "").await;
    let item = page["items"][0].clone();
    let (id, version) = (
        item["id"].as_str().unwrap().to_owned(),
        item["version"].as_i64().unwrap(),
    );
    let unread_before = page["unreadCount"].as_i64().unwrap();
    let (s, r, _) = call(
        &world.app,
        "POST",
        &format!("/v1/inbox/{id}/commands/read"),
        Some(&m),
        None,
        Some(json!({"expectedVersion":version})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["state"], "READ");
    let (s, stale, _) = call(
        &world.app,
        "POST",
        &format!("/v1/inbox/{id}/commands/read"),
        Some(&m),
        None,
        Some(json!({"expectedVersion":version})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");
    assert_eq!(
        inbox(world, &m, ws, "").await["unreadCount"]
            .as_i64()
            .unwrap(),
        unread_before - 1
    );
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/inbox/{id}/commands/read"),
        Some(&outsider),
        None,
        Some(json!({"expectedVersion":version + 1})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "someone else's item");
    let until = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let (s, sn, _) = call(
        &world.app,
        "POST",
        &format!("/v1/inbox/{id}/commands/snooze"),
        Some(&m),
        None,
        Some(json!({"expectedVersion":version + 1,"until":until})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{sn}");
    let listed: Vec<String> = inbox(world, &m, ws, "").await["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(!listed.contains(&id), "snoozed items wait");
    sqlx::query(
        "UPDATE penfii.inbox_items SET snoozed_until=now()-interval '1 minute' WHERE id=$1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .execute(&world.pool)
    .await
    .unwrap();
    let back = inbox(world, &m, ws, "").await;
    let returned = back["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(returned["state"], "UNREAD", "back as unread");
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/inbox/{id}/commands/archive"),
        Some(&m),
        None,
        Some(json!({"expectedVersion":returned["version"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !inbox(world, &m, ws, "").await["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == id.as_str())
    );
    let (s, all_read, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/inbox/commands/read-all"),
        Some(&m),
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{all_read}");
    assert_eq!(inbox(world, &m, ws, "").await["unreadCount"], 0);

    // Access lost: the Ticket moves into a private channel the assignee is not in. The item
    // disappears from the list and the count, and nothing about the Ticket is shown.
    let before = inbox(world, &m, ws, "").await["items"]
        .as_array()
        .unwrap()
        .len();
    let (s, pc, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        Some(&o),
        None,
        Some(json!({"name":"locked","visibility":"PRIVATE","teamId":null,"projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{pc}");
    sqlx::query("UPDATE penfii.tickets SET channel_id=$2 WHERE id=$1")
        .bind(Uuid::parse_str(&b_id).unwrap())
        .bind(Uuid::parse_str(pc["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let after = inbox(world, &m, ws, "").await;
    assert!(after["items"].as_array().unwrap().len() < before, "{after}");
    assert!(
        !after.to_string().contains("Task B"),
        "no title of what cannot be opened: {after}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn watching_a_ticket_is_chosen_remembered_and_shown_in_my_work(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-wt-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let o = world.owner_cookie.clone();
    let (project_id, _) = new_project(world, ws, "Watching").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    let (_, t) = make_ticket(
        world,
        &project_id,
        "p62wt-1",
        json!({"title":"Someone elses"}),
    )
    .await;
    let (_, mine) = make_ticket(
        world,
        &project_id,
        "p62wt-2",
        json!({"title":"Mine","assigneeId":m_id}),
    )
    .await;
    let id = t["id"].as_str().unwrap();
    let _ = mine;
    attention_now(ws, world).await;
    // Not following yet; My Work's 관찰 lens lists what the viewer follows (here the assigned one).
    let (_, st) = get_as(world, &m, &format!("/v1/tickets/{id}/subscription")).await;
    assert_eq!(st["watching"], false);
    let (_, work) = get_as(
        world,
        &m,
        &format!("/v1/workspaces/{ws}/work?scope=WORKSPACE&lens=WATCHING"),
    )
    .await;
    assert_eq!(work["tickets"].as_array().unwrap().len(), 1, "{work}");
    assert_eq!(work["tickets"][0]["watching"], true);
    // Choosing to follow adds it; the flag is on every Ticket in the dataset.
    let (s, v, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/tickets/{id}/subscription"),
        Some(&m),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "EXPLICIT");
    let (_, work) = get_as(
        world,
        &m,
        &format!("/v1/workspaces/{ws}/work?scope=WORKSPACE&lens=WATCHING"),
    )
    .await;
    assert_eq!(work["tickets"].as_array().unwrap().len(), 2);
    let (_, all) = get_as(
        world,
        &m,
        &format!("/v1/workspaces/{ws}/work?scope=WORKSPACE&lens=ALL"),
    )
    .await;
    assert!(
        all["tickets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["watching"].is_boolean())
    );
    // Favorite adds no Watch, unwatch keeps Later: they are separate lists.
    let (_, list) = get_as(
        world,
        &m,
        &format!("/v1/workspaces/{ws}/subscriptions?kind=TICKET"),
    )
    .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 2, "{list}");
    // Refusing is remembered through later activity by the same person.
    call(
        &world.app,
        "DELETE",
        &format!("/v1/tickets/{id}/subscription"),
        Some(&m),
        None,
        None,
    )
    .await;
    call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/comments"),
        Some(&m),
        None,
        Some(json!({"body":doc("나도 한마디")})),
    )
    .await;
    attention_now(ws, world).await;
    let (_, st) = get_as(world, &m, &format!("/v1/tickets/{id}/subscription")).await;
    assert_eq!(st["mode"], "OPT_OUT", "{st}");
    // A Ticket nobody may open cannot be followed, and says not found.
    let (s, pc, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        Some(&o),
        None,
        Some(json!({"name":"hush","visibility":"PRIVATE","teamId":null,"projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{pc}");
    let (_, secret) = make_ticket(
        world,
        &project_id,
        "p62wt-3",
        json!({"title":"Secret","channelId":pc["id"]}),
    )
    .await;
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!(
            "/v1/tickets/{}/subscription",
            secret["id"].as_str().unwrap()
        ),
        Some(&m),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // The migration back-fills only the creator and the current assignee, and is safe to run again.
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.subscriptions")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    penfii_core::apply_migrations(&world.pool).await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.subscriptions")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert!(after >= before);
    let (_, st) = get_as(world, &m, &format!("/v1/tickets/{id}/subscription")).await;
    assert_eq!(
        st["mode"], "OPT_OUT",
        "a refusal survives the migration: {st}"
    );
}

// ---- Audit follow-ups: freed Tickets, dead deliveries, Teams from the seed, the index prefix -------

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn dependency_released_is_about_the_freed_ticket_and_asks_whether_that_ticket_can_be_seen(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-dep-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (v_id, v) = member(world, ws, "v").await;
    let o = world.owner_cookie.clone();
    let (project_id, workflow) = new_project(world, ws, "Freed").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    grant_user(world, &project_id, v_id, "VIEWER").await;
    let done = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // A blocks B, and C blocks D. B is assigned to m and followed by v; D is assigned to m.
    let mut ids = Vec::new();
    for (key, title, assignee) in [
        ("a", "Task A", None),
        ("b", "Task B", Some(m_id)),
        ("c", "Task C", None),
        ("d", "Task D", Some(m_id)),
    ] {
        let mut body = json!({"title": title});
        if let Some(assignee) = assignee {
            body["assigneeId"] = json!(assignee);
        }
        let (s, t) = make_ticket(world, &project_id, &format!("p62dep-{key}"), body).await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    let (a_id, b_id, c_id, d_id) = (
        ids[0].clone(),
        ids[1].clone(),
        ids[2].clone(),
        ids[3].clone(),
    );
    for (succ, pred, key) in [(&b_id, &a_id, "p62dep-ab"), (&d_id, &c_id, "p62dep-cd")] {
        let (s, r) = post_as(
            world,
            &o,
            &format!("/v1/tickets/{succ}/dependencies"),
            key,
            json!({"predecessorId":pred}),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{r}");
    }
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/tickets/{b_id}/subscription"),
        Some(&v),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let _ = v_id;
    attention_now(ws, world).await;

    // B moves into a private channel that v belongs to and m does not.
    let (s, pc, _) = call(&world.app, "POST", &format!("/v1/workspaces/{ws}/channels"), Some(&o), None,
        Some(json!({"name":"freed-locked","visibility":"PRIVATE","teamId":null,"projectId":project_id}))).await;
    assert_eq!(s, StatusCode::CREATED, "{pc}");
    let channel = Uuid::parse_str(pc["id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO penfii.channel_members(channel_id,user_id,role) VALUES($1,$2,'MEMBER')",
    )
    .bind(channel)
    .bind(v_id)
    .execute(&world.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE penfii.tickets SET channel_id=$2 WHERE id=$1")
        .bind(Uuid::parse_str(&b_id).unwrap())
        .bind(channel)
        .execute(&world.pool)
        .await
        .unwrap();

    for (pred, key) in [(&a_id, "p62dep-a-done"), (&c_id, "p62dep-c-done")] {
        let (s, mv, _) = call(
            &world.app,
            "POST",
            &format!("/v1/tickets/{pred}/commands/move"),
            Some(&o),
            Some(key),
            Some(json!({"expectedRevision":1,"statusId":done})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{mv}");
    }
    attention_now(ws, world).await;

    // The item is about the freed Ticket: the follower who can open B gets it, with B as its subject and link.
    let page = inbox(world, &v, ws, "").await;
    let released: Vec<&Value> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "dependency.released")
        .collect();
    assert_eq!(released.len(), 1, "{page}");
    assert_eq!(released[0]["subject"]["kind"], "TICKET");
    assert_eq!(released[0]["subject"]["id"], b_id.as_str());
    assert!(
        released[0]["subject"]["title"]
            .as_str()
            .unwrap()
            .contains("Task B")
    );
    assert_eq!(released[0]["link"]["ticketId"], b_id.as_str());
    // m can open A but not B: nothing is delivered for B and nothing of B shows, while D (open to m) arrives.
    let page = inbox(world, &m, ws, "").await;
    let mine: Vec<&Value> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "dependency.released")
        .collect();
    assert_eq!(mine.len(), 1, "only D: {page}");
    assert_eq!(mine[0]["subject"]["id"], d_id.as_str());
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.inbox_items WHERE recipient_id=$1 AND notification_kind='dependency.released'").bind(m_id).fetch_one(&world.pool).await.unwrap();
    assert_eq!(held, 1, "no item was stored for the Ticket m cannot open");
}

async fn inbox_command(
    world: &World,
    cookie: &str,
    item: &Value,
    command: &str,
    body: Value,
) -> (StatusCode, Value) {
    let id = item["id"].as_str().unwrap();
    let (s, v, _) = call(
        &world.app,
        "POST",
        &format!("/v1/inbox/{id}/commands/{command}"),
        Some(cookie),
        None,
        Some(body),
    )
    .await;
    (s, v)
}

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_archive_is_a_view_ordered_by_when_items_were_archived_and_snoozed_items_are_optional(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("fea2-ar-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (_, other) = member(world, ws, "x").await;
    let (project_id, _) = new_project(world, ws, "Archive tickets").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    for n in 0..3 {
        let (s, t) = make_ticket(
            world,
            &project_id,
            &format!("fea2-t{n}"),
            json!({"title": format!("Task {n}"), "assigneeId": m_id}),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
    }
    attention_now(ws, world).await;
    let page = inbox(world, &m, ws, "").await;
    let items = page["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 3, "{page}");
    // Newest first: items[0] is the newest. Archive the oldest first, then the newest.
    let (oldest, middle, newest) = (&items[2], &items[1], &items[0]);

    let (s, a1) = inbox_command(
        world,
        &m,
        oldest,
        "archive",
        json!({"expectedVersion": oldest["version"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a1}");
    assert!(a1["archivedAt"].is_string(), "archive records when: {a1}");
    let (s, a2) = inbox_command(
        world,
        &m,
        newest,
        "archive",
        json!({"expectedVersion": newest["version"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a2}");

    // The Inbox no longer lists them, the archive does, last archived first (not by creation).
    let inbox_ids = ids(&inbox(world, &m, ws, "").await);
    assert_eq!(inbox_ids, vec![middle["id"].as_str().unwrap().to_owned()]);
    let archive = inbox(world, &m, ws, "?view=archived").await;
    assert_eq!(
        ids(&archive),
        vec![
            newest["id"].as_str().unwrap().to_owned(),
            oldest["id"].as_str().unwrap().to_owned()
        ],
        "{archive}"
    );
    assert!(archive["items"][0]["archivedAt"].is_string());
    assert_eq!(archive["items"][0]["state"], "ARCHIVED");
    // Read and snooze options do not apply in the archive.
    let archive_unread = inbox(
        world,
        &m,
        ws,
        "?view=archived&state=unread&includeSnoozed=true",
    )
    .await;
    assert_eq!(ids(&archive_unread).len(), 2);
    // Archive paging follows the same order with a stable cursor.
    let p1 = inbox(world, &m, ws, "?view=archived&limit=1").await;
    assert_eq!(ids(&p1), vec![newest["id"].as_str().unwrap().to_owned()]);
    let cursor = p1["nextCursor"].as_str().expect("more").to_owned();
    let p2 = inbox(
        world,
        &m,
        ws,
        &format!("?view=archived&limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(ids(&p2), vec![oldest["id"].as_str().unwrap().to_owned()]);
    // Someone else's archive is empty; an unknown view is refused.
    assert!(ids(&inbox(world, &other, ws, "?view=archived").await).is_empty());
    let (s, _) = get_as(world, &m, &format!("/v1/workspaces/{ws}/inbox?view=trash")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // An archived item is not snoozed (it comes back by hand).
    let (s, refused) = inbox_command(
        world,
        &m,
        &archive["items"][0],
        "snooze",
        json!({"expectedVersion": archive["items"][0]["version"], "until": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{refused}");

    // Unarchive puts it back in the Inbox as READ and clears archivedAt.
    let (s, back) = inbox_command(
        world,
        &m,
        &archive["items"][0],
        "unarchive",
        json!({"expectedVersion": archive["items"][0]["version"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{back}");
    assert_eq!(back["state"], "READ");
    assert!(back["archivedAt"].is_null());
    assert_eq!(ids(&inbox(world, &m, ws, "").await).len(), 2);
    assert_eq!(ids(&inbox(world, &m, ws, "?view=archived").await).len(), 1);

    // A row archived before the column existed is listed by creation time and reports it.
    sqlx::query("UPDATE penfii.inbox_items SET archived_at=NULL WHERE id=$1")
        .bind(Uuid::parse_str(oldest["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    let legacy = inbox(world, &m, ws, "?view=archived").await;
    assert_eq!(
        legacy["items"][0]["archivedAt"],
        legacy["items"][0]["createdAt"]
    );

    // Snoozed items are left out unless asked for; the unread count never counts them.
    let (s, sn) = inbox_command(
        world,
        &m,
        middle,
        "snooze",
        json!({"expectedVersion": middle["version"], "until": (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339()}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{sn}");
    let hidden = inbox(world, &m, ws, "").await;
    assert!(!ids(&hidden).contains(&middle["id"].as_str().unwrap().to_owned()));
    let shown = inbox(world, &m, ws, "?includeSnoozed=true").await;
    let snoozed = shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == middle["id"])
        .expect("snoozed item listed on request");
    assert!(snoozed["snoozedUntil"].is_string());
    assert_eq!(hidden["unreadCount"], shown["unreadCount"]);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn several_types_are_one_union_and_a_type_names_its_kinds(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("fea2-ty-{}", Uuid::new_v4())).await;
    let (m_id, m) = member(world, ws, "m").await;
    let (project_id, _) = new_project(world, ws, "Type tickets").await;
    grant_user(world, &project_id, m_id, "VIEWER").await;
    let (s, t) = make_ticket(
        world,
        &project_id,
        "fea2-ty-a",
        json!({"title":"Assigned","assigneeId":m_id}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    attention_now(ws, world).await;
    let all = kinds(&inbox(world, &m, ws, "").await);
    assert_eq!(all, vec!["ticket.assigned"]);
    // OR: the union holds the item once; a type without it holds nothing; `all` and nothing are alike.
    assert_eq!(
        kinds(&inbox(world, &m, ws, "?filter=mentions,assignments").await),
        all
    );
    assert_eq!(
        kinds(&inbox(world, &m, ws, "?filter=assignments,assignments").await),
        all
    );
    assert!(kinds(&inbox(world, &m, ws, "?filter=mentions,replies").await).is_empty());
    assert_eq!(kinds(&inbox(world, &m, ws, "?filter=all").await), all);
    assert_eq!(kinds(&inbox(world, &m, ws, "?filter=").await), all);
    // AND with unread.
    assert_eq!(
        kinds(&inbox(world, &m, ws, "?filter=assignments&state=unread").await),
        all
    );
    // An unknown type is refused.
    let (s, _) = get_as(
        world,
        &m,
        &format!("/v1/workspaces/{ws}/inbox?filter=mentions,bogus"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // The kinds each type names are fixed by the code, thread.reply only under replies.
    let kinds_of = |f: &str| penfii_core::inbox::kinds_for_filter(f).unwrap().unwrap();
    assert_eq!(kinds_of("replies"), vec!["reply.direct", "thread.reply"]);
    assert!(!kinds_of("watching").contains(&"thread.reply"));
    assert!(
        kinds_of("mentions,replies,assignments,reminders,invitations")
            .iter()
            .all(|k| *k != "dependency.released")
    );
    assert!(penfii_core::inbox::kinds_for_filter("").unwrap().is_none());
}
