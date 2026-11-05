//! Favorites, Later, reminders, pins, folders and drafts through the API.

use super::access::{grant_project, grant_user};
use super::inbox::{attention_now, inbox, kinds, member};
use super::{
    World, call, channel, get_as, make_ticket, new_project, send, viewer_cookie, workspace,
};
use crate::common::doc;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn favorites_later_and_pins_keep_personal_and_shared_apart_and_hide_lost_targets(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Personal work").await;
    let owner = world.owner_cookie.clone();
    let (s, ticket) = make_ticket(world, &project_id, "p6pw-1", json!({"title":"Save me"})).await;
    assert_eq!(s, StatusCode::CREATED, "{ticket}");
    let ticket_id = ticket["id"].as_str().unwrap().to_owned();
    let base = format!("/v1/workspaces/{workspace_id}");
    let put = |cookie: String, path: String, body: Value| {
        let app = world.app.clone();
        async move { call(&app, "PUT", &path, Some(&cookie), None, Some(body)).await }
    };

    // Favorite: one card per target; a Favorite is not a Watch and not a Pin.
    let (s, fav, _) = put(
        owner.clone(),
        format!("{base}/favorites"),
        json!({"kind":"TICKET","targetId":ticket_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{fav}");
    assert!(fav["title"].as_str().unwrap().ends_with("Save me"));
    assert_eq!(fav["projectId"], project_id);
    let (_, again, _) = put(
        owner.clone(),
        format!("{base}/favorites"),
        json!({"kind":"TICKET","targetId":ticket_id}),
    )
    .await;
    assert_eq!(
        again["position"], fav["position"],
        "saving again keeps its place"
    );
    let (s, _, _) = put(
        owner.clone(),
        format!("{base}/favorites"),
        json!({"kind":"PROJECT","targetId":project_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, bad, _) = put(
        owner.clone(),
        format!("{base}/favorites"),
        json!({"kind":"MESSAGE","targetId":ticket_id}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    let (s, gone, _) = put(
        owner.clone(),
        format!("{base}/favorites"),
        json!({"kind":"TICKET","targetId":Uuid::new_v4()}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{gone}");
    let (_, list) = get_as(world, &owner, &format!("{base}/favorites")).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    // Reordering is the person's own.
    let (s, ordered, _) = put(owner.clone(), format!("{base}/favorites/order"), json!({"items":[{"kind":"PROJECT","targetId":project_id},{"kind":"TICKET","targetId":ticket_id}]})).await;
    assert_eq!(s, StatusCode::OK, "{ordered}");
    assert_eq!(ordered[0]["kind"], "PROJECT");
    let other = viewer_cookie(world, workspace_id, "p6pw-other@penfii.test").await;
    grant_project(world, &project_id, "p6pw-other@penfii.test", "VIEWER").await;
    let (_, theirs) = get_as(world, &other, &format!("{base}/favorites")).await;
    assert_eq!(theirs.as_array().unwrap().len(), 0, "Favorites are private");
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("{base}/favorites/PROJECT/{project_id}"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // A saved private-channel Ticket disappears for someone who loses access, and leaves no title.
    let (_, channel, _) = call(
        &world.app,
        "POST",
        &format!("{base}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"pw secret","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    let (_, secret) = make_ticket(
        world,
        &project_id,
        "p6pw-secret",
        json!({"title":"Private thing","channelId":channel["id"]}),
    )
    .await;
    let secret_id = secret["id"].as_str().unwrap().to_owned();
    let (s, v, _) = put(
        other.clone(),
        format!("{base}/favorites"),
        json!({"kind":"TICKET","targetId":secret_id}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "cannot save what cannot be seen: {v}"
    );
    let (s, _, _) = put(
        owner.clone(),
        format!("{base}/favorites"),
        json!({"kind":"TICKET","targetId":secret_id}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(Uuid::parse_str(channel["id"].as_str().unwrap()).unwrap())
        .bind(world.owner_id)
        .execute(&world.pool)
        .await
        .unwrap();
    // The owner is still the project admin but no longer in the channel.
    let (_, list) = get_as(world, &owner, &format!("{base}/favorites")).await;
    assert!(!list.to_string().contains("Private thing"));
    sqlx::query("INSERT INTO penfii.channel_members(channel_id,user_id,role) VALUES($1,$2,'MANAGER') ON CONFLICT DO NOTHING").bind(Uuid::parse_str(channel["id"].as_str().unwrap()).unwrap()).bind(world.owner_id).execute(&world.pool).await.unwrap();

    // Later: temporal, with a note; finishing it never finishes the Ticket.
    let (s, later, _) = call(
        &world.app,
        "POST",
        &format!("{base}/later"),
        Some(&owner),
        None,
        Some(json!({"kind":"TICKET","targetId":ticket_id,"note":"check after release"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{later}");
    assert_eq!(later["state"], "ACTIVE");
    let (_, dup, _) = call(
        &world.app,
        "POST",
        &format!("{base}/later"),
        Some(&owner),
        None,
        Some(json!({"kind":"TICKET","targetId":ticket_id})),
    )
    .await;
    assert_eq!(dup["id"], later["id"], "the same object keeps one card");
    let later_id = later["id"].as_str().unwrap().to_owned();
    let (s, done, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/later/{later_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"state":"DONE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let (_, t) = get_as(world, &owner, &format!("/v1/tickets/{ticket_id}")).await;
    assert_ne!(t["statusId"].as_str().map(|s| s.to_owned()), None);
    let still_open: bool = sqlx::query_scalar("SELECT s.category<>'DONE' FROM penfii.tickets t JOIN penfii.workflow_statuses s ON s.id=t.status_id WHERE t.id=$1").bind(Uuid::parse_str(&ticket_id).unwrap()).fetch_one(&world.pool).await.unwrap();
    assert!(still_open, "Later done does not complete the Ticket");
    let (s, stale, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/later/{later_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"state":"ARCHIVED"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");
    let (_, active) = get_as(world, &owner, &format!("{base}/later")).await;
    assert!(
        active
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["id"] != later_id)
    );
    let (_, finished) = get_as(world, &owner, &format!("{base}/later?state=DONE")).await;
    assert_eq!(finished.as_array().unwrap().len(), 1);
    let (s, _, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/later/{later_id}"),
        Some(&other),
        None,
        Some(json!({"expectedVersion":2,"state":"ACTIVE"})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "another person's card");
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/later/{later_id}"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, bad, _) = call(
        &world.app,
        "POST",
        &format!("{base}/later"),
        Some(&owner),
        None,
        Some(json!({"kind":"PROJECT","targetId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");

    // Pins are shared and separate from Favorites: an admin pins, everyone reads, a hidden Ticket is omitted.
    let (s, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/pins"),
        Some(&other),
        None,
        Some(json!({"ticketId":ticket_id})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    let (s, pins, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/pins"),
        Some(&owner),
        None,
        Some(json!({"ticketId":ticket_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{pins}");
    assert_eq!(pins["canManage"], true);
    let (s, dup, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/pins"),
        Some(&owner),
        None,
        Some(json!({"ticketId":ticket_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{dup}");
    let (s, link, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/pins"),
        Some(&owner),
        None,
        Some(json!({"url":"https://example.test/runbook","title":"Runbook"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    for body in [
        json!({"url":"javascript:alert(1)"}),
        json!({"url":"https://a.test","ticketId":ticket_id}),
        json!({}),
    ] {
        let (s, v, _) = call(
            &world.app,
            "POST",
            &format!("/v1/projects/{project_id}/pins"),
            Some(&owner),
            None,
            Some(body),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    let (s, v, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/pins"),
        Some(&owner),
        None,
        Some(json!({"ticketId":secret_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, seen) = get_as(world, &other, &format!("/v1/projects/{project_id}/pins")).await;
    assert_eq!(seen["canManage"], false);
    assert_eq!(
        seen["pins"].as_array().unwrap().len(),
        2,
        "the private Ticket's pin is not shown"
    );
    assert!(!seen.to_string().contains("Private thing"));
    let pin_id = seen["pins"][1]["id"].as_str().unwrap().to_owned();
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/pins/{pin_id}"),
        Some(&other),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/pins/{pin_id}"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // Pinning did not make a Favorite for anyone.
    let (_, mine) = get_as(world, &owner, &format!("{base}/favorites")).await;
    assert!(mine.as_array().unwrap().iter().all(|f| f["kind"] != "PIN"));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn drafts_follow_the_person_and_stay_private(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let owner = world.owner_cookie.clone();
    let channel_id = channel(world, workspace_id, "drafts open", "PUBLIC").await;
    let private_id = channel(world, workspace_id, "drafts private", "PRIVATE").await;
    let other = viewer_cookie(world, workspace_id, "p6dr-other@penfii.test").await;
    let put = |cookie: String, body: Value| {
        let app = world.app.clone();
        async move { call(&app, "PUT", "/v1/drafts", Some(&cookie), None, Some(body)).await }
    };

    let (s, created, _) = put(owner.clone(), json!({"contextKind":"CHANNEL_COMPOSE","contextId":channel_id,"payload":{"schemaVersion":1,"body":doc("half written")}})).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    assert_eq!(created["version"], 1);
    let (s, updated, _) = put(owner.clone(), json!({"contextKind":"CHANNEL_COMPOSE","contextId":channel_id,"expectedVersion":1,"payload":{"schemaVersion":1,"body":doc("more written")}})).await;
    assert_eq!(s, StatusCode::OK, "{updated}");
    assert_eq!(updated["version"], 2);
    let (s, stale, _) = put(owner.clone(), json!({"contextKind":"CHANNEL_COMPOSE","contextId":channel_id,"expectedVersion":1,"payload":{"schemaVersion":1,"body":doc("from an old tab")}})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "draft_conflict");
    let (s, blank, _) = put(owner.clone(), json!({"contextKind":"CHANNEL_COMPOSE","contextId":channel_id,"payload":{"schemaVersion":1,"body":{"type":"doc","content":[]}}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{blank}");
    let (s, kind, _) = put(owner.clone(), json!({"contextKind":"DM_COMPOSE","contextId":channel_id,"payload":{"schemaVersion":1,"body":doc("x")}})).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "a channel box and a DM box are one thing to the client: {kind}"
    );
    assert_eq!(kind["contextKind"], "CHANNEL_COMPOSE");
    assert_eq!(kind["version"], 3);

    let (s, loaded) = get_as(
        world,
        &owner,
        &format!("/v1/drafts?contextKind=CHANNEL_COMPOSE&contextId={channel_id}"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{loaded}");
    assert_eq!(
        loaded["payload"]["body"]["content"][0]["content"][0]["text"],
        "x"
    );
    let (s, list) = get_as(
        world,
        &owner,
        &format!("/v1/workspaces/{workspace_id}/drafts"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let mine = list
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["contextId"] == channel_id.to_string())
        .unwrap();
    assert_eq!(mine["title"], "drafts open");
    assert_eq!(mine["preview"], "x");

    // Private to the author, and tied to what the author can still reach.
    let (s, theirs) = get_as(
        world,
        &other,
        &format!("/v1/workspaces/{workspace_id}/drafts"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(theirs.as_array().unwrap().len(), 0);
    let (s, _) = get_as(
        world,
        &other,
        &format!("/v1/drafts?contextKind=CHANNEL_COMPOSE&contextId={channel_id}"),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "no draft of their own there");
    let (s, v, _) = put(other.clone(), json!({"contextKind":"CHANNEL_COMPOSE","contextId":private_id,"payload":{"schemaVersion":1,"body":doc("sneak")}})).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "cannot draft into a private channel they cannot see: {v}"
    );
    let draft_id = created["id"].as_str().unwrap();
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/drafts/{draft_id}"),
        Some(&other),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "not their draft");
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/drafts/{draft_id}"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = get_as(
        world,
        &owner,
        &format!("/v1/drafts?contextKind=CHANNEL_COMPOSE&contextId={channel_id}"),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::NO_CONTENT,
        "the channel is still writable, it just has no draft"
    );
}

async fn later_add(
    world: &World,
    cookie: &str,
    ws: Uuid,
    kind: &str,
    target: &str,
    note: Option<&str>,
) -> (StatusCode, Value) {
    let (s, v, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/later"),
        Some(cookie),
        None,
        Some(json!({"kind":kind,"targetId":target,"note":note})),
    )
    .await;
    (s, v)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn later_holds_messages_and_files_and_hides_what_you_can_no_longer_open(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-lt-{}", Uuid::new_v4())).await;
    let (_, m) = member(world, ws, "m").await;
    let o = world.owner_cookie.clone();
    let (project_id, _) = new_project(world, ws, "Later kinds").await;
    let (_, ticket) = make_ticket(
        world,
        &project_id,
        "p62lt-t",
        json!({"title":"Kept ticket"}),
    )
    .await;
    let (pc_status, pc, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        Some(&o),
        None,
        Some(json!({"name":"vault","visibility":"PRIVATE","teamId":null,"projectId":project_id})),
    )
    .await;
    assert_eq!(pc_status, StatusCode::CREATED);
    let vault: Uuid = pc["id"].as_str().unwrap().parse().unwrap();
    let secret = send(world, &o, vault, "비밀 메시지", None, None).await.1;
    let general = channel(world, ws, "lounge", "PUBLIC").await;
    let open = send(world, &o, general, "열린 메시지", None, None).await.1;

    // The old kind works as before; messages and files are new.
    let (s, t) = later_add(
        world,
        &o,
        ws,
        "TICKET",
        ticket["id"].as_str().unwrap(),
        Some("나중에"),
    )
    .await;
    assert!(s.is_success(), "{t}");
    assert!(t["title"].as_str().unwrap().contains("Kept ticket"));
    let (s, l) = later_add(world, &o, ws, "MESSAGE", open["id"].as_str().unwrap(), None).await;
    assert!(s.is_success(), "{l}");
    assert_eq!(l["conversationId"], general.to_string());
    assert!(l["title"].as_str().unwrap().contains("열린 메시지"));
    // A file needs a message to live in.
    let file_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'plan.pdf',2048,'application/pdf','AVAILABLE',$4)")
        .bind(file_id).bind(ws).bind(format!("p62/{file_id}")).bind(world.owner_id).execute(&world.pool).await.unwrap();
    sqlx::query("INSERT INTO penfii.message_attachments(id,message_id,file_id) VALUES($1,$2,$3)")
        .bind(Uuid::new_v4())
        .bind(vault_message_id(&secret))
        .bind(file_id)
        .execute(&world.pool)
        .await
        .unwrap();
    let (s, f) = later_add(world, &o, ws, "FILE", &file_id.to_string(), Some("읽기")).await;
    assert!(s.is_success(), "{f}");
    assert_eq!(f["title"], "plan.pdf");
    assert_eq!(f["conversationId"], vault.to_string());

    // Someone outside the vault cannot save what is inside it, and gets no hint.
    let (s, _) = later_add(
        world,
        &m,
        ws,
        "MESSAGE",
        secret["id"].as_str().unwrap(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = later_add(world, &m, ws, "FILE", &file_id.to_string(), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // A file seen only by its uploader before it is attached is theirs; no one else's.
    let loose = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,state,created_by) VALUES($1,$2,$3,'loose.txt',10,'AVAILABLE',$4)")
        .bind(loose).bind(ws).bind(format!("p62/{loose}")).bind(world.owner_id).execute(&world.pool).await.unwrap();
    assert!(
        later_add(world, &o, ws, "FILE", &loose.to_string(), None)
            .await
            .0
            .is_success()
    );
    assert_eq!(
        later_add(world, &m, ws, "FILE", &loose.to_string(), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // The owner loses the vault: a card with a note shows only the note; one without vanishes.
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(vault)
        .bind(world.owner_id)
        .execute(&world.pool)
        .await
        .unwrap();
    let (_, saved) = get_as(
        world,
        &o,
        &format!("/v1/workspaces/{ws}/later?state=ACTIVE"),
    )
    .await;
    let items = saved.as_array().unwrap();
    let gone = items
        .iter()
        .find(|i| i["kind"] == "FILE" && i["note"] == "읽기")
        .expect("the note stays");
    assert_eq!(gone["available"], false);
    assert!(
        gone["title"].is_null() && gone["conversationId"].is_null(),
        "{gone}"
    );
    assert!(!saved.to_string().contains("plan.pdf"));
    // A deleted message hides too.
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/messages/{}", open["id"].as_str().unwrap()),
        Some(&o),
        Some("p62-lt-del"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, saved) = get_as(
        world,
        &o,
        &format!("/v1/workspaces/{ws}/later?state=ACTIVE"),
    )
    .await;
    assert!(
        !saved
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "MESSAGE"),
        "{saved}"
    );
    // Finishing a card never touches the object (legacy rule, still true).
    let lid = t["id"].as_str().unwrap();
    let (s, d, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/later/{lid}"),
        Some(&o),
        None,
        Some(json!({"expectedVersion":t["version"],"state":"DONE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let (_, tk) = get_as(
        world,
        &o,
        &format!("/v1/tickets/{}", ticket["id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(tk["revisionNo"], 1);
}

fn vault_message_id(message: &Value) -> Uuid {
    message["id"].as_str().unwrap().parse().unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_reminder_fires_once_late_if_need_be_and_never_for_what_you_can_no_longer_open(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-rm-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    // The Reminder belongs to a member, so setting their preferences leaves other tests alone.
    let (o_id, o) = member(world, ws, "rm").await;
    let (project_id, _) = new_project(world, ws, "Reminders").await;
    grant_user(world, &project_id, o_id, "VIEWER").await;
    let (_, ticket) =
        make_ticket(world, &project_id, "p62rm-t", json!({"title":"Remind me"})).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_owned();
    let (_, later) = later_add(world, &o, ws, "TICKET", &ticket_id, None).await;
    let later_id = later["id"].as_str().unwrap().to_owned();
    let put = |when: chrono::DateTime<chrono::Utc>, zone: &str| {
        let app = world.app.clone();
        let cookie = o.clone();
        let path = format!("/v1/later/{later_id}/reminder");
        let body = json!({"dueAt":when.to_rfc3339(),"timeZone":zone});
        async move { call(&app, "PUT", &path, Some(&cookie), None, Some(body)).await }
    };
    let now = chrono::Utc::now();
    // Limits: not in the past, not beyond a year, a real zone.
    assert_eq!(
        put(now - chrono::Duration::hours(1), "Asia/Seoul").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(now + chrono::Duration::days(400), "Asia/Seoul").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(now + chrono::Duration::hours(1), "Mars/Base").await.0,
        StatusCode::BAD_REQUEST
    );
    // Setting a time twice replaces the first; there is one scheduled at a time.
    let (s, first, _) = put(now + chrono::Duration::hours(2), "Asia/Seoul").await;
    assert_eq!(s, StatusCode::OK, "{first}");
    let (s, second, _) = put(now + chrono::Duration::hours(3), "Asia/Seoul").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(second["version"], 2);
    let scheduled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.reminders WHERE later_id=$1 AND state='SCHEDULED'",
    )
    .bind(Uuid::parse_str(&later_id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(scheduled, 1);
    // Cancel before it is due: nothing fires.
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/later/{later_id}/reminder"),
        Some(&o),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        penfii_core::reminders::fire_due(&world.pool, 50, Some(ws))
            .await
            .unwrap(),
        0
    );

    // Do not disturb never drops a Reminder.
    let (s, p, _) = call(&world.app, "PUT", "/v1/me/notification-preferences", Some(&o), None,
        Some(json!({"level":"DIRECT_ONLY","dnd":{"enabled":true,"schedule":null,"snoozeUntil":null},"external":{},"expectedVersion":0}))).await;
    assert!(s == StatusCode::OK || s == StatusCode::CONFLICT, "{p}");
    // Due: set 30 seconds back (allowed), then pretend the server was down for ten minutes.
    let (s, due, _) = put(now - chrono::Duration::seconds(30), "Asia/Seoul").await;
    assert_eq!(s, StatusCode::OK, "{due}");
    sqlx::query("UPDATE penfii.reminders SET due_at=now()-interval '10 minutes' WHERE id=$1")
        .bind(Uuid::parse_str(due["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    // Two schedulers at once: one fires it.
    let (a, b) = tokio::join!(
        penfii_core::reminders::fire_due(&world.pool, 50, Some(ws)),
        penfii_core::reminders::fire_due(&world.pool, 50, Some(ws))
    );
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    attention_now(ws, world).await;
    attention_now(ws, world).await;
    let page = inbox(world, &o, ws, "?filter=reminders").await;
    assert_eq!(kinds(&page), vec!["reminder.due"], "{page}");
    let late = page["items"][0]["params"]["lateSeconds"].as_i64().unwrap();
    assert!(
        (590..700).contains(&late),
        "delivered late, and says so: {late}"
    );
    assert_eq!(page["items"][0]["subject"]["kind"], "TICKET");
    // Own-action suppression does not apply to a Reminder: it is meant for its owner.
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.domain_events WHERE workspace_id=$1 AND event_kind='reminder.due'").bind(ws).fetch_one(&world.pool).await.unwrap();
    assert_eq!(events, 1);
    // A fired Reminder does not fire again, and the Later card is untouched.
    assert_eq!(
        penfii_core::reminders::fire_due(&world.pool, 50, Some(ws))
            .await
            .unwrap(),
        0
    );
    let (_, saved) = get_as(
        world,
        &o,
        &format!("/v1/workspaces/{ws}/later?state=ACTIVE"),
    )
    .await;
    assert_eq!(saved[0]["reminder"]["state"], "FIRED");
    assert_eq!(saved[0]["state"], "ACTIVE");

    // Finishing the card stops a scheduled Reminder.
    let (s, again, _) = put(now + chrono::Duration::hours(5), "Asia/Seoul").await;
    assert_eq!(s, StatusCode::OK, "{again}");
    let (s, _, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/later/{later_id}"),
        Some(&o),
        None,
        Some(json!({"expectedVersion":saved[0]["version"],"state":"DONE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let reason: String =
        sqlx::query_scalar("SELECT cancel_reason FROM penfii.reminders WHERE id=$1")
            .bind(Uuid::parse_str(again["id"].as_str().unwrap()).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(reason, "LATER_CLOSED");

    // Access lost before it is due: it is dropped without a notice.
    let (_, member_cookie) = member(world, ws, "x").await;
    let member_id: Uuid = sqlx::query_scalar("SELECT user_id FROM penfii.workspace_members m WHERE workspace_id=$1 AND user_id<>$2 AND user_id NOT IN (SELECT user_id FROM penfii.later_items WHERE workspace_id=$1) ORDER BY user_id LIMIT 1").bind(ws).bind(world.owner_id).fetch_one(&world.pool).await.unwrap();
    let (pcs, pc, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        Some(&owner),
        None,
        Some(json!({"name":"room","visibility":"PRIVATE","teamId":null,"projectId":null})),
    )
    .await;
    assert_eq!(pcs, StatusCode::CREATED);
    let room: Uuid = pc["id"].as_str().unwrap().parse().unwrap();
    sqlx::query(
        "INSERT INTO penfii.channel_members(channel_id,user_id,role) VALUES($1,$2,'MEMBER')",
    )
    .bind(room)
    .bind(member_id)
    .execute(&world.pool)
    .await
    .unwrap();
    let msg = send(world, &owner, room, "비공개", None, None).await.1;
    let (_, ml) = later_add(
        world,
        &member_cookie,
        ws,
        "MESSAGE",
        msg["id"].as_str().unwrap(),
        None,
    )
    .await;
    let (s, mr, _) = call(&world.app, "PUT", &format!("/v1/later/{}/reminder", ml["id"].as_str().unwrap()), Some(&member_cookie), None,
        Some(json!({"dueAt":(chrono::Utc::now()+chrono::Duration::minutes(5)).to_rfc3339(),"timeZone":"UTC"}))).await;
    assert_eq!(s, StatusCode::OK, "{mr}");
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(room)
        .bind(member_id)
        .execute(&world.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE penfii.reminders SET due_at=now()-interval '1 second' WHERE id=$1")
        .bind(Uuid::parse_str(mr["id"].as_str().unwrap()).unwrap())
        .execute(&world.pool)
        .await
        .unwrap();
    assert_eq!(
        penfii_core::reminders::fire_due(&world.pool, 50, Some(ws))
            .await
            .unwrap(),
        1
    );
    attention_now(ws, world).await;
    let reason: String =
        sqlx::query_scalar("SELECT cancel_reason FROM penfii.reminders WHERE id=$1")
            .bind(Uuid::parse_str(mr["id"].as_str().unwrap()).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(reason, "ACCESS_LOST");
    assert!(
        inbox(world, &member_cookie, ws, "").await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn favorite_folders_organize_without_ever_deleting_a_favorite(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p62-ff-{}", Uuid::new_v4())).await;
    let o = world.owner_cookie.clone();
    let (_, other) = member(world, ws, "ff").await;
    let mut projects = Vec::new();
    for name in ["Folder One", "Folder Two", "Folder Three"] {
        projects.push(new_project(world, ws, name).await.0);
    }
    for p in &projects {
        let (s, v, _) = call(
            &world.app,
            "PUT",
            &format!("/v1/workspaces/{ws}/favorites"),
            Some(&o),
            None,
            Some(json!({"kind":"PROJECT","targetId":p})),
        )
        .await;
        assert!(s.is_success(), "{v}");
    }
    let (s, a, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/favorite-folders"),
        Some(&o),
        None,
        Some(json!({"name":"Daily"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    let folder = a["id"].as_str().unwrap().to_owned();
    // Names are unique ignoring case; bad names are refused.
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/favorite-folders"),
        Some(&o),
        None,
        Some(json!({"name":"daily"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    for bad in ["", "   ", &"x".repeat(61)] {
        let (s, _, _) = call(
            &world.app,
            "POST",
            &format!("/v1/workspaces/{ws}/favorite-folders"),
            Some(&o),
            None,
            Some(json!({"name":bad})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, r, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/favorite-folders/{folder}"),
        Some(&o),
        None,
        Some(json!({"name":"Every day"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");

    // Put two Favorites in the folder; one stays at the top.
    for p in &projects[..2] {
        let (s, _, _) = call(
            &world.app,
            "PUT",
            &format!("/v1/workspaces/{ws}/favorites/PROJECT/{p}/folder"),
            Some(&o),
            None,
            Some(json!({"folderId":folder})),
        )
        .await;
        assert_eq!(s, StatusCode::NO_CONTENT);
    }
    let (_, list) = get_as(world, &o, &format!("/v1/workspaces/{ws}/favorites")).await;
    let in_folder = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .filter(|f| f["folderId"] == folder.as_str())
            .count()
    };
    assert_eq!(in_folder(&list), 2, "{list}");
    // Another person has no access to my folder and cannot see it.
    let (s, _, _) = call(
        &world.app,
        "PUT",
        &format!(
            "/v1/workspaces/{ws}/favorites/PROJECT/{}/folder",
            projects[2]
        ),
        Some(&other),
        None,
        Some(json!({"folderId":folder})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, theirs) = get_as(
        world,
        &other,
        &format!("/v1/workspaces/{ws}/favorite-folders"),
    )
    .await;
    assert!(theirs["items"].as_array().unwrap().is_empty());
    let (s, _, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/favorite-folders/{folder}"),
        Some(&other),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Deleting the folder moves its Favorites to the top level, after the ones already there.
    let (s, d, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/favorite-folders/{folder}"),
        Some(&o),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["movedToTopLevel"], 2);
    let (_, list) = get_as(world, &o, &format!("/v1/workspaces/{ws}/favorites")).await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["targetId"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            projects[2].as_str(),
            projects[0].as_str(),
            projects[1].as_str()
        ],
        "nothing deleted, order kept: {list}"
    );
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|f| f["folderId"].is_null())
    );
    let (_, folders) = get_as(world, &o, &format!("/v1/workspaces/{ws}/favorite-folders")).await;
    assert!(folders["items"].as_array().unwrap().is_empty());

    // At most fifty folders.
    for n in 0..50 {
        let (s, v, _) = call(
            &world.app,
            "POST",
            &format!("/v1/workspaces/{ws}/favorite-folders"),
            Some(&o),
            None,
            Some(json!({"name":format!("f{n}")})),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let (s, _, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/favorite-folders"),
        Some(&o),
        None,
        Some(json!({"name":"one too many"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}
