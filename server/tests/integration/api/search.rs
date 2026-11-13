//! Search over Tickets, files and Docs, and keeping the index in step.

use super::access::grant_project;
use super::docs::{api, events_of, indexed_count, make_doc_with, para};
use super::events::receipt_states;
use super::inbox::member;
use super::{
    World, call, channel, get_as, invite_member, make_ticket, new_project, send, viewer_cookie,
    workspace,
};
use crate::common::{self, doc};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub(super) struct SearchWorld {
    pub(super) engine: penfii_core::search::SearchEngine,
    pub(super) app: axum::Router,
    pub(super) prefix: String,
}

fn search_test_url() -> Option<String> {
    match std::env::var("PENFII_TEST_OPENSEARCH_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            if std::env::var("PENFII_SKIP_SEARCH_TESTS").ok().as_deref() == Some("1") {
                eprintln!("SKIPPING search tests: PENFII_SKIP_SEARCH_TESTS=1");
                None
            } else {
                panic!(
                    "PENFII_TEST_OPENSEARCH_URL is required for the search tests (set PENFII_SKIP_SEARCH_TESTS=1 to skip them on purpose)"
                );
            }
        }
    }
}

/// The search engine of this test, on indices of its own: their prefix is derived from the test
/// database, so a test that runs again starts by deleting what its last run left.
pub(super) async fn search_world(world: &World) -> Option<std::sync::Arc<SearchWorld>> {
    world
        .search
        .get_or_init(|| async {
            let url = search_test_url()?;
            let prefix = test_prefix(&world.pool, "");
            let config = penfii_core::search::client::OsConfig::with_prefix(&url, &prefix);
            let client = penfii_core::search::client::OsClient::new(config).unwrap();
            let engine = penfii_core::search::SearchEngine::new(world.pool.clone(), client);
            engine.indexer.delete_everything_for_tests().await;
            engine
                .indexer
                .backfill_all()
                .await
                .expect("first build of every index");
            let app = penfii_http_api::router_with(
                world.pool.clone(),
                common::limits(),
                false,
                None,
                Some(engine.clone()),
            );
            Some(std::sync::Arc::new(SearchWorld {
                engine,
                app,
                prefix,
            }))
        })
        .await
        .clone()
}

/// An OpenSearch index prefix that belongs to the test database behind `pool`.
fn test_prefix(pool: &PgPool, suffix: &str) -> String {
    use sha2::Digest;
    let database = pool.connect_options().get_database().unwrap().to_owned();
    let hash = sha2::Sha256::digest(database.as_bytes());
    let hex: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("t{hex}{suffix}")
}

/// Delivers the Workspace's events to the search index and makes them searchable.
pub(super) async fn index_now(sw: &SearchWorld, workspace_id: Uuid) {
    let mut registry = penfii_core::events::Registry {
        backoff_base_secs: 0,
        max_backoff_secs: 0,
        max_attempts: 5,
        workspace: Some(workspace_id),
        ..Default::default()
    };
    registry
        .external
        .push(penfii_core::search::consumer(&sw.engine));
    penfii_core::events::run_until_idle(&sw.engine.indexer.pool, &registry)
        .await
        .unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
}

pub(super) async fn find(
    sw: &SearchWorld,
    cookie: &str,
    workspace_id: Uuid,
    query: &str,
    kind: &str,
) -> (StatusCode, Value) {
    let encoded: String = query
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let (status, body, _) = call(
        &sw.app,
        "GET",
        &format!(
            "/v1/workspaces/{workspace_id}/search?q={encoded}&kind={kind}&limit=50&tz=Asia/Seoul"
        ),
        Some(cookie),
        None,
        None,
    )
    .await;
    (status, body)
}

pub(super) fn titles(body: &Value) -> Vec<String> {
    body["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|x| x["title"].as_str().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_finds_ticket_files_and_follows_the_ticket_through_trash_and_restore(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("tf-s1-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Files").await;
    let (_, ticket) = make_ticket(world, &project_id, "tf-1", json!({"title": "has files"})).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let file_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'quarterly_budget.xlsx',99,'application/octet-stream','AVAILABLE',$4)")
        .bind(file_id).bind(workspace_id).bind(format!("tf/{file_id}")).bind(world.owner_id).execute(&world.pool).await.unwrap();
    sqlx::query("INSERT INTO penfii.ticket_attachments(id,ticket_id,file_id,attached_by) VALUES($1,$2,$3,$4)")
        .bind(Uuid::new_v4()).bind(ticket_id).bind(file_id).bind(world.owner_id).execute(&world.pool).await.unwrap();
    sw.engine.indexer.refresh("file", &[file_id]).await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let c = world.owner_cookie.clone();

    let (s, b) = find(&sw, &c, workspace_id, "quarterly_budget", "file").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(titles(&b), vec!["quarterly_budget.xlsx"], "{b}");
    assert_eq!(
        b["results"][0]["fields"]["ticketId"],
        json!(ticket_id),
        "{b}"
    );
    assert_eq!(
        b["results"][0]["fields"]["projectId"],
        json!(project_id),
        "{b}"
    );
    assert!(b["results"][0]["fields"]["conversationId"].is_null(), "{b}");

    // In the trash the file is out of search; restoring brings it back.
    let trash = format!("/v1/tickets/{ticket_id}/commands/trash");
    let (s, _, _) = call(&world.app, "POST", &trash, Some(&c), None, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    index_now(&sw, workspace_id).await;
    let (_, b) = find(&sw, &c, workspace_id, "quarterly_budget", "file").await;
    assert!(titles(&b).is_empty(), "{b}");
    let restore = format!("/v1/tickets/{ticket_id}/commands/restore-from-trash");
    let (s, _, _) = call(&world.app, "POST", &restore, Some(&c), None, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    index_now(&sw, workspace_id).await;
    let (_, b) = find(&sw, &c, workspace_id, "quarterly_budget", "file").await;
    assert_eq!(titles(&b), vec!["quarterly_budget.xlsx"], "{b}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_archived_ticket_and_its_files_leave_search_and_return_when_unarchived(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("ar-s1-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Archived").await;
    let (_, ticket) = make_ticket(
        world,
        &project_id,
        "ar-1",
        json!({"title": "zebra roadmap"}),
    )
    .await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let c = world.owner_cookie.clone();
    index_now(&sw, workspace_id).await;
    let (_, b) = find(&sw, &c, workspace_id, "zebra", "ticket").await;
    assert_eq!(titles(&b), vec!["zebra roadmap"], "{b}");

    let archive = format!("/v1/tickets/{ticket_id}/commands/archive");
    let (s, _, _) = call(&world.app, "POST", &archive, Some(&c), None, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    index_now(&sw, workspace_id).await;
    let (_, b) = find(&sw, &c, workspace_id, "zebra", "ticket").await;
    assert!(titles(&b).is_empty(), "archived work is out of search: {b}");

    let unarchive = format!("/v1/tickets/{ticket_id}/commands/unarchive");
    let (s, _, _) = call(&world.app, "POST", &unarchive, Some(&c), None, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    index_now(&sw, workspace_id).await;
    let (_, b) = find(&sw, &c, workspace_id, "zebra", "ticket").await;
    assert_eq!(titles(&b), vec!["zebra roadmap"], "{b}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_finds_korean_text_keys_code_and_file_names_and_applies_filters(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("p62-s1-{}", Uuid::new_v4())).await;
    let (project_id, workflow) = new_project(world, workspace_id, "검색 Alpha").await;
    let done = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, t1) = make_ticket(
        world,
        &project_id,
        "p62s-1",
        json!({"title":"프로젝트를 만들었습니다","assigneeId":world.owner_id}),
    )
    .await;
    let (_, _t2) = make_ticket(
        world,
        &project_id,
        "p62s-2",
        json!({"title":"Deploy 배포 자동화"}),
    )
    .await;
    let (_, t3) = make_ticket(
        world,
        &project_id,
        "p62s-3",
        json!({"title":"결제 모듈 점검"}),
    )
    .await;
    let key1 = t1["key"].as_str().unwrap().to_owned();
    let id3 = t3["id"].as_str().unwrap();
    call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id3}/commands/move"),
        Some(&world.owner_cookie),
        Some("p62s-mv"),
        Some(json!({"expectedRevision":1,"statusId":done})),
    )
    .await;
    let general = channel(world, workspace_id, "general", "PUBLIC").await;
    send(
        world,
        &world.owner_cookie,
        general,
        "릴리스 노트를 정리했습니다 foo_bar() 호출",
        None,
        None,
    )
    .await;
    index_now(&sw, workspace_id).await;
    let c = world.owner_cookie.clone();

    // Korean: an inflected form, a two-syllable prefix, and mixed Korean/English.
    for q in ["프로젝트", "프로젝트를", "프로", "만들"] {
        let (s, b) = find(&sw, &c, workspace_id, q, "ticket").await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(
            titles(&b).contains(&"프로젝트를 만들었습니다".to_owned()),
            "{q}: {b}"
        );
    }
    let (_, b) = find(&sw, &c, workspace_id, "deploy 배포", "ticket").await;
    assert_eq!(titles(&b), vec!["Deploy 배포 자동화"], "{b}");
    // A Ticket key, exact and case-insensitive.
    let (_, b) = find(&sw, &c, workspace_id, &key1.to_lowercase(), "ticket").await;
    assert_eq!(
        titles(&b).first().map(String::as_str),
        Some("프로젝트를 만들었습니다"),
        "{b}"
    );
    // A code token in a message and the Space by name.
    let (_, b) = find(&sw, &c, workspace_id, "foo_bar()", "message").await;
    assert_eq!(b["results"].as_array().unwrap().len(), 1, "{b}");
    let (_, b) = find(&sw, &c, workspace_id, "릴리스", "message").await;
    assert_eq!(b["results"].as_array().unwrap().len(), 1, "{b}");
    let (_, b) = find(&sw, &c, workspace_id, "검색", "space").await;
    assert!(titles(&b).contains(&"검색 Alpha".to_owned()), "{b}");
    // A File by the stem of its name.
    let message_id: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.messages WHERE conversation_id=$1 LIMIT 1")
            .bind(general)
            .fetch_one(&world.pool)
            .await
            .unwrap();
    let file_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'분기_report_v2.pdf',1234,'application/pdf','AVAILABLE',$4)")
        .bind(file_id).bind(workspace_id).bind(format!("p62/{file_id}")).bind(world.owner_id).execute(&world.pool).await.unwrap();
    sqlx::query("INSERT INTO penfii.message_attachments(id,message_id,file_id) VALUES($1,$2,$3)")
        .bind(Uuid::new_v4())
        .bind(message_id)
        .bind(file_id)
        .execute(&world.pool)
        .await
        .unwrap();
    sw.engine.indexer.refresh("file", &[file_id]).await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    for q in ["report_v2.pdf", "report", "분기"] {
        let (_, b) = find(&sw, &c, workspace_id, q, "file").await;
        assert_eq!(titles(&b), vec!["분기_report_v2.pdf"], "{q}: {b}");
    }

    // Filters: category word, assignee:me, from:me, in:, and the UI/syntax pair gives one answer.
    let (_, b) = find(&sw, &c, workspace_id, "status:done", "ticket").await;
    assert_eq!(titles(&b), vec!["결제 모듈 점검"], "{b}");
    let (_, b) = find(&sw, &c, workspace_id, "assignee:me", "all").await;
    assert_eq!(
        titles(&b),
        vec!["프로젝트를 만들었습니다"],
        "assignee: leaves out kinds without an assignee: {b}"
    );
    let (_, b) = find(&sw, &c, workspace_id, "from:me 릴리스", "message").await;
    assert_eq!(b["results"].as_array().unwrap().len(), 1, "{b}");
    let (_, b) = find(&sw, &c, workspace_id, "in:general 릴리스", "message").await;
    assert_eq!(b["results"].as_array().unwrap().len(), 1, "{b}");
    let (_, b) = find(&sw, &c, workspace_id, "in:general has:file", "message").await;
    assert_eq!(
        b["results"].as_array().unwrap().len(),
        0,
        "the message has no attachment row at index time: {b}"
    );
    // A State chip, never a global state name.
    let (s, body, _) = call(
        &sw.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/search?q=&kind=ticket&stateIds={done}"),
        Some(&c),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(titles(&body), vec!["결제 모듈 점검"], "{body}");
    // before/after use the person's day in their time zone.
    let (_, b) = find(&sw, &c, workspace_id, "after:2999-01-01", "ticket").await;
    assert!(titles(&b).is_empty(), "{b}");
    let (_, b) = find(
        &sw,
        &c,
        workspace_id,
        "before:2999-01-01 프로젝트",
        "ticket",
    )
    .await;
    assert_eq!(titles(&b).len(), 1, "{b}");

    // Invalid input is an error and never a wider search.
    for (q, code) in [
        ("state:Review", "unsupported_key"),
        ("status:review", "invalid_status"),
        ("foo:bar", "unsupported_key"),
        ("before:tomorrow", "invalid_date"),
        ("has:image", "invalid_value"),
        ("mentions:bob", "invalid_value"),
        ("(a b)", "unsupported_syntax"),
        ("assignee:nobody-here", "unknown_filter_value"),
        ("project:NOPE", "unknown_filter_value"),
    ] {
        let (s, b) = find(&sw, &c, workspace_id, q, "all").await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{q}: {b}");
        assert_eq!(b["code"], code, "{q}: {b}");
    }
    let (s, b) = find(&sw, &c, workspace_id, "", "ticket").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        titles(&b).is_empty(),
        "an empty query is not everything: {b}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_pages_through_results_without_gaps_and_rejects_a_foreign_cursor(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("p62-s2-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Paging").await;
    for n in 0..7 {
        make_ticket(
            world,
            &project_id,
            &format!("p62pg-{n}"),
            json!({"title":format!("pagingword item {n}")}),
        )
        .await;
    }
    index_now(&sw, workspace_id).await;
    let c = world.owner_cookie.clone();
    let mut seen = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let path = format!(
            "/v1/workspaces/{workspace_id}/search?q=pagingword&kind=ticket&limit=3{}",
            cursor
                .as_ref()
                .map(|c| format!("&cursor={c}"))
                .unwrap_or_default()
        );
        let (s, b, _) = call(&sw.app, "GET", &path, Some(&c), None, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        for r in b["results"].as_array().unwrap() {
            assert!(
                seen.insert(r["id"].as_str().unwrap().to_owned()),
                "no duplicates: {b}"
            );
        }
        pages += 1;
        assert!(b["results"].as_array().unwrap().len() <= 3);
        assert!(
            b.get("total").is_none() && b.get("count").is_none(),
            "no totals"
        );
        cursor = b["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() || pages > 6 {
            break;
        }
    }
    assert_eq!(
        seen.len(),
        7,
        "every result exactly once across {pages} pages"
    );
    // The cursor belongs to its query.
    let (_, first, _) = call(
        &sw.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/search?q=pagingword&kind=ticket&limit=3"),
        Some(&c),
        None,
        None,
    )
    .await;
    let cur = first["nextCursor"].as_str().unwrap();
    let (s, b, _) = call(
        &sw.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/search?q=other&kind=ticket&limit=3&cursor={cur}"),
        Some(&c),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(b["code"], "invalid_cursor");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_never_shows_what_the_server_would_not_let_the_person_open(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("p62-s3-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Access").await;
    let outsider_email = format!("p62-out-{}@penfii.test", Uuid::new_v4());
    let outsider = viewer_cookie(world, workspace_id, &outsider_email).await;
    // Taking part in the Project, but not in its private channel.
    grant_project(world, &project_id, &outsider_email, "VIEWER").await;
    let outsider_id: Uuid = sqlx::query_scalar("SELECT user_id FROM penfii.sessions s JOIN penfii.workspace_members m USING(user_id) WHERE m.workspace_id=$1 AND m.user_id<>$2 LIMIT 1")
        .bind(workspace_id).bind(world.owner_id).fetch_one(&world.pool).await.unwrap();
    let (s, pc, _) = call(&world.app, "POST", &format!("/v1/workspaces/{workspace_id}/channels"), Some(&world.owner_cookie), None,
        Some(json!({"name":"secret-room","visibility":"PRIVATE","teamId":null,"projectId":project_id}))).await;
    assert_eq!(s, StatusCode::CREATED, "{pc}");
    let private: Uuid = pc["id"].as_str().unwrap().parse().unwrap();
    let public = channel(world, workspace_id, "town-hall", "PUBLIC").await;
    let project_channel: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.channels WHERE project_id=$1")
            .bind(Uuid::parse_str(&project_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    let (_, private_message) = send(
        world,
        &world.owner_cookie,
        private,
        "secretword in the private room",
        None,
        None,
    )
    .await;
    let (_, public_message) = send(
        world,
        &world.owner_cookie,
        public,
        "secretword in public",
        None,
        None,
    )
    .await;
    send(
        world,
        &world.owner_cookie,
        project_channel,
        "secretword in the project channel",
        None,
        None,
    )
    .await;
    let (s, private_ticket) = make_ticket(
        world,
        &project_id,
        "p62-acc-t",
        json!({"title":"secretword private ticket","channelId":private}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{private_ticket}");
    let (_, open_ticket) = make_ticket(
        world,
        &project_id,
        "p62-acc-o",
        json!({"title":"secretword open ticket"}),
    )
    .await;
    index_now(&sw, workspace_id).await;

    let owner = world.owner_cookie.clone();
    let (_, mine) = find(&sw, &owner, workspace_id, "secretword", "all").await;
    assert_eq!(
        mine["results"].as_array().unwrap().len(),
        5,
        "the owner sees all five: {mine}"
    );

    // A participant of the Project, not of its private channel: the public channel, the Project's channel and the open Ticket.
    let (_, theirs) = find(&sw, &outsider, workspace_id, "secretword", "all").await;
    let got = titles(&theirs);
    assert_eq!(got.len(), 3, "{theirs}");
    assert!(
        got.contains(&"secretword open ticket".to_owned()),
        "{got:?}"
    );
    let ids: Vec<&str> = theirs["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&public_message["id"].as_str().unwrap()));
    assert!(!ids.contains(&private_message["id"].as_str().unwrap()));
    assert!(
        !ids.contains(&private_ticket["id"].as_str().unwrap()),
        "a private-channel Ticket is not found"
    );
    // The same answer the screens give: the Work list and the visibility service agree.
    let mut conn = world.pool.acquire().await.unwrap();
    let both = [
        private_ticket["id"].as_str().unwrap().parse().unwrap(),
        open_ticket["id"].as_str().unwrap().parse().unwrap(),
    ];
    let visible = penfii_core::visibility::tickets(&mut conn, outsider_id, &both)
        .await
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert!(visible.contains(&both[1]));
    let (_, work) = get_as(
        world,
        &outsider,
        &format!("/v1/workspaces/{workspace_id}/work?scope=WORKSPACE&lens=ALL"),
    )
    .await;
    let work_ids: Vec<&str> = work["tickets"]
        .as_array()
        .map(|t| t.iter().filter_map(|x| x["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        !work_ids.contains(&private_ticket["id"].as_str().unwrap()),
        "{work}"
    );
    drop(conn);

    // Membership gained and then revoked takes effect at once, with no re-indexing in between.
    sqlx::query(
        "INSERT INTO penfii.channel_members(channel_id,user_id,role) VALUES($1,$2,'MEMBER')",
    )
    .bind(private)
    .bind(outsider_id)
    .execute(&world.pool)
    .await
    .unwrap();
    let (_, gained) = find(&sw, &outsider, workspace_id, "secretword", "all").await;
    assert_eq!(gained["results"].as_array().unwrap().len(), 5, "{gained}");
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(private)
        .bind(outsider_id)
        .execute(&world.pool)
        .await
        .unwrap();
    let (_, revoked) = find(&sw, &outsider, workspace_id, "secretword", "all").await;
    assert_eq!(revoked["results"].as_array().unwrap().len(), 3, "{revoked}");

    // A deleted message disappears once its event is handled, and never reappears for a viewer
    // who could not read it anyway.
    let mid = public_message["id"].as_str().unwrap();
    let (s, b, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/messages/{mid}"),
        Some(&world.owner_cookie),
        Some("p62-del"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, before_index) = find(&sw, &owner, workspace_id, "secretword", "message").await;
    assert!(
        !before_index["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == mid),
        "removed at once because the server re-checks: {before_index}"
    );
    index_now(&sw, workspace_id).await;
    let doc = sw
        .engine
        .indexer
        .client
        .admin(
            penfii_core::search::client::Method::GET,
            &format!("/{}-message/_doc/{mid}", sw.prefix),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        doc.0, 404,
        "and gone from the index after the event: {}",
        doc.1
    );
    // A non-member of the workspace gets nothing and no hint.
    let (s, _) = find(&sw, &outsider, Uuid::new_v4(), "secretword", "all").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_is_separate_from_commands_and_recovers_after_an_outage(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("p62-s4-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Outage").await;
    // A search engine that cannot be reached.
    let dead_config =
        penfii_core::search::client::OsConfig::with_prefix("http://127.0.0.1:9", "p62test");
    let dead = penfii_core::search::SearchEngine::new(
        world.pool.clone(),
        penfii_core::search::client::OsClient::new(dead_config).unwrap(),
    );
    let dead_app = penfii_http_api::router_with(
        world.pool.clone(),
        common::limits(),
        false,
        None,
        Some(dead.clone()),
    );
    let (s, b) = make_ticket(
        world,
        &project_id,
        "p62-out-1",
        json!({"title":"outageword before"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // Commands work while the index is down; the delivery waits and is retried.
    let mut registry = penfii_core::events::Registry {
        backoff_base_secs: 3600,
        max_backoff_secs: 3600,
        max_attempts: 5,
        workspace: Some(workspace_id),
        ..Default::default()
    };
    registry.external.push(penfii_core::search::consumer(&dead));
    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    let pending: Vec<(String, i32)> = receipt_states(&world.pool, "search", workspace_id).await;
    assert!(
        !pending.is_empty() && pending.iter().all(|(st, a)| st == "PENDING" && *a >= 1),
        "{pending:?}"
    );
    let (s, b) = make_ticket(
        world,
        &project_id,
        "p62-out-2",
        json!({"title":"outageword during"}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CREATED,
        "ticket creation does not depend on the index: {b}"
    );
    // Search itself answers 503 with the retry hint, never a PostgreSQL substitute.
    let (s, b, _) = call(
        &dead_app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/search?q=outageword&kind=ticket"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert_eq!(b["code"], "search_unavailable");
    assert_eq!(b["retryable"], true);
    // Without any engine configured the answer is the same.
    let none_app = penfii_http_api::router(world.pool.clone(), common::limits(), false, None);
    let (s, b, _) = call(
        &none_app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/search?q=outageword&kind=ticket"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    // People do not depend on the index.
    let (s, _, _) = call(
        &none_app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/search?q=&kind=person"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // The index comes back: the waiting deliveries are handled and the work is found.
    sqlx::query("UPDATE penfii.consumer_receipts r SET available_at=now() FROM penfii.domain_events e WHERE e.event_id=r.event_id AND e.workspace_id=$1 AND r.consumer='search' AND r.state='PENDING'")
        .bind(workspace_id).execute(&world.pool).await.unwrap();
    index_now(&sw, workspace_id).await;
    let (_, b) = find(
        &sw,
        &world.owner_cookie,
        workspace_id,
        "outageword",
        "ticket",
    )
    .await;
    assert_eq!(b["results"].as_array().unwrap().len(), 2, "{b}");
    // The same event delivered again leaves one document.
    sqlx::query("UPDATE penfii.consumer_receipts r SET state='PENDING', attempts=0 FROM penfii.domain_events e WHERE e.event_id=r.event_id AND e.workspace_id=$1 AND r.consumer='search'")
        .bind(workspace_id).execute(&world.pool).await.unwrap();
    index_now(&sw, workspace_id).await;
    let (_, b) = find(
        &sw,
        &world.owner_cookie,
        workspace_id,
        "outageword",
        "ticket",
    )
    .await;
    assert_eq!(b["results"].as_array().unwrap().len(), 2, "{b}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_full_rebuild_keeps_search_working_and_ends_with_the_same_documents(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("p62-s5-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Rebuild").await;
    for n in 0..30 {
        make_ticket(
            world,
            &project_id,
            &format!("p62rb-{n}"),
            json!({"title":format!("rebuildword number {n}")}),
        )
        .await;
    }
    let general = channel(world, workspace_id, "rebuild-room", "PUBLIC").await;
    send(
        world,
        &world.owner_cookie,
        general,
        "rebuildword in chat",
        None,
        None,
    )
    .await;
    // Data that existed before any event was ever delivered is found after the rebuild alone.
    let before = sw.engine.indexer.state("ticket").await.unwrap();
    sw.engine.indexer.request_reindex("ticket").await.unwrap();
    sw.engine.indexer.request_reindex("message").await.unwrap();
    // While the new generation is being built, the old one still answers.
    let (s, b) = find(
        &sw,
        &world.owner_cookie,
        workspace_id,
        "rebuildword",
        "ticket",
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    sw.engine.indexer.backfill_all().await.unwrap();
    // Running it again changes nothing.
    sw.engine.indexer.request_reindex("ticket").await.unwrap();
    sw.engine.indexer.backfill_all().await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let after = sw.engine.indexer.state("ticket").await.unwrap();
    assert_ne!(
        before.live_index, after.live_index,
        "a new generation was promoted"
    );
    assert_eq!(after.phase, "READY");
    assert!(after.building_index.is_none());
    let (_, b) = find(&sw, &world.owner_cookie, workspace_id, "rebuildword", "all").await;
    assert_eq!(b["results"].as_array().unwrap().len(), 31, "{b}");
    // Every Ticket exactly once.
    let ids: std::collections::HashSet<&str> = b["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 31);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_index_prefix_is_validated_recorded_and_checked_against_other_databases(pool: PgPool) {
    use penfii_core::search::client::{DEFAULT_PREFIX, Method, OsClient, OsConfig, valid_prefix};
    let world = &World::new(pool).await;
    assert_eq!(DEFAULT_PREFIX, "penfii-dev");
    for ok in ["penfii-dev", "staging_1", "a"] {
        assert!(valid_prefix(ok), "{ok}");
    }
    for bad in [
        "",
        "Penfii",
        "-x",
        "has space",
        "a*",
        "a/b",
        &"x".repeat(41),
    ] {
        assert!(!valid_prefix(bad), "{bad:?}");
        assert!(OsClient::new(OsConfig::with_prefix("http://127.0.0.1:9", bad)).is_err());
    }
    let Some(url) = search_test_url() else { return };
    let sw = search_world(world).await.unwrap();

    // This test's indices record its database as their owner.
    let owner = sw.engine.indexer.owner_id().await.unwrap();
    assert_eq!(owner, sw.engine.indexer.owner_id().await.unwrap(), "stable");
    let (_, mapping) = sw
        .engine
        .indexer
        .client
        .admin(
            Method::GET,
            &format!(
                "/{}-v1-*/_mapping?filter_path=*.mappings._meta",
                test_prefix(&world.pool, "")
            ),
            None,
        )
        .await
        .unwrap();
    let owners: Vec<&str> = mapping
        .as_object()
        .unwrap()
        .values()
        .filter_map(|i| i["mappings"]["_meta"]["owner"].as_str())
        .collect();
    assert!(
        !owners.is_empty() && owners.iter().all(|o| *o == owner),
        "{mapping}"
    );
    sw.engine.indexer.check_prefix().await.unwrap();

    // A second prefix: empty, then another database's index, then an old one without an owner.
    let other = test_prefix(&world.pool, "o");
    let client = OsClient::new(OsConfig::with_prefix(&url, &other)).unwrap();
    let engine = penfii_core::search::SearchEngine::new(world.pool.clone(), client);
    let admin = |method, path: &str, body: Option<Value>| {
        let client = engine.indexer.client.clone();
        let path = path.to_owned();
        async move { client.admin(method, &path, body.as_ref()).await.unwrap().0 }
    };
    admin(Method::DELETE, &format!("/{other}-*"), None).await;
    engine.indexer.check_prefix().await.unwrap();
    assert_eq!(
        admin(
            Method::PUT,
            &format!("/{other}-v1-ticket-20200101000000000"),
            Some(json!({"mappings":{"_meta":{"owner":"another-database"}}}))
        )
        .await,
        200
    );
    match engine.indexer.check_prefix().await {
        Err(penfii_core::CoreError::Conflict { code, .. }) => {
            assert_eq!(code, "search_prefix_in_use")
        }
        other => panic!("{other:?}"),
    }
    // Nothing was deleted by the refusal.
    assert_eq!(
        admin(
            Method::HEAD,
            &format!("/{other}-v1-ticket-20200101000000000"),
            None
        )
        .await,
        200
    );
    admin(Method::DELETE, &format!("/{other}-*"), None).await;
    admin(
        Method::PUT,
        &format!("/{other}-v1-ticket-20200101000000000"),
        Some(json!({})),
    )
    .await;
    engine.indexer.check_prefix().await.unwrap();
    admin(Method::DELETE, &format!("/{other}-*"), None).await;
    admin(
        Method::PUT,
        &format!("/{other}-v1-ticket-20200101000000000"),
        Some(json!({"mappings":{"_meta":{"owner":owner}}})),
    )
    .await;
    engine.indexer.check_prefix().await.unwrap();
    admin(Method::DELETE, &format!("/{other}-*"), None).await;
    // A prefix that merely starts the same does not count as the same prefix.
    admin(
        Method::PUT,
        &format!("/{other}-extra-v1-ticket-20200101000000000"),
        Some(json!({"mappings":{"_meta":{"owner":"another-database"}}})),
    )
    .await;
    engine.indexer.check_prefix().await.unwrap();
    admin(Method::DELETE, &format!("/{other}-*"), None).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t1_t2_t5_t6_t11_t12_t20_docs_are_found_by_title_body_and_comments_and_forgotten_on_delete(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let ws = workspace(world, &format!("p63-ds1-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (_, bob) = member(world, ws, "bob").await;
    let (project_id, _) = new_project(world, ws, "Secret Project").await;
    let (s, private_channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":"secret-room","visibility":"PRIVATE","projectId":project_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{private_channel}");
    let (s, ticket) = make_ticket(
        world,
        &project_id,
        "p63ds-1",
        json!({"title":"극비 장애 보고", "channelId": private_channel["id"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{ticket}");
    let ticket_id = ticket["id"].as_str().unwrap().to_owned();

    let deploy = make_doc_with(
        world,
        &owner,
        ws,
        "배포 절차",
        json!([para("스테이징 환경에서 먼저 검증한다")]),
        json!({}),
    )
    .await;
    let kube = make_doc_with(world, &owner, ws, "운영 메모", json!([{"id":"c","type":"codeBlock","props":{"language":"bash"},"content":[{"type":"text","text":"kubectl apply -f deploy.yaml"}],"children":[]}]), json!({})).await;
    let compact = make_doc_with(
        world,
        &owner,
        ws,
        "배포절차 요약",
        json!([para("본문 없음 ")]),
        json!({}),
    )
    .await;
    let linker = make_doc_with(world, &owner, ws, "링크 문서", json!([{"id":"r","type":"paragraph","props":{},"content":[
        {"type":"text","text":"참고: "},{"type":"reference","props":{"kind":"TICKET","id": ticket_id}}],"children":[]}]), json!({})).await;
    let (s, _) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/docs/{deploy}/comments"),
        Some(json!({"body": doc("롤백 기준 추가")})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    index_now(&sw, ws).await;

    // T1: title, body and a Korean word inside a longer one; the title hit ranks above a body-only hit.
    for q in ["배포", "검증", "스테이징", "스테이징 검증"] {
        let (s, b) = find(&sw, &owner, ws, q, "doc").await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(titles(&b).contains(&"배포 절차".to_owned()), "{q}: {b}");
    }
    let (_, b) = find(&sw, &owner, ws, "배포", "doc").await;
    assert_eq!(
        titles(&b).first().map(String::as_str),
        Some("배포 절차"),
        "{b}"
    );
    assert!(
        titles(&b).contains(&"배포절차 요약".to_owned()),
        "a prefix finds the compact title: {b}"
    );
    assert_eq!(b["results"][0]["kind"], "doc");
    assert!(b["results"][0]["fields"]["authorName"].is_string());
    // T2: a comment makes the page findable, with a snippet from PostgreSQL text.
    let (_, b) = find(&sw, &owner, ws, "롤백", "doc").await;
    assert_eq!(titles(&b), vec!["배포 절차"], "{b}");
    // Code identifiers.
    for q in ["kubectl", "deploy.yaml"] {
        let (_, b) = find(&sw, &owner, ws, q, "doc").await;
        assert_eq!(titles(&b), vec!["운영 메모"], "{q}: {b}");
    }
    // The All tab includes docs.
    let (_, b) = find(&sw, &owner, ws, "배포", "all").await;
    assert!(
        b["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "doc"),
        "{b}"
    );

    // T12: the page that links a private Ticket is not found by that Ticket's title, and the
    // link renders as unavailable for a person who cannot read the Ticket.
    let (_, b) = find(&sw, &owner, ws, "극비", "doc").await;
    assert!(
        titles(&b).is_empty(),
        "the Ticket's title is never copied into the page: {b}"
    );
    let (_, resolved) = api(
        world,
        "POST",
        &bob,
        &format!("/v1/workspaces/{ws}/references/resolve"),
        Some(json!({"items":[{"kind":"TICKET","id": ticket_id}]})),
    )
    .await;
    assert_eq!(
        resolved["items"][0],
        json!({"kind":"TICKET","id": ticket_id,"available": false})
    );
    let (_, resolved) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/workspaces/{ws}/references/resolve"),
        Some(json!({"items":[{"kind":"TICKET","id": ticket_id}]})),
    )
    .await;
    assert!(
        resolved["items"][0]["title"]
            .as_str()
            .unwrap()
            .contains("극비 장애 보고")
    );
    let _ = linker;

    // T20: a filter a Doc does not have leaves the kind out; the answer is empty, not wider.
    let (s, b) = find(&sw, &owner, ws, "status:done 배포", "doc").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(titles(&b).is_empty(), "{b}");
    let (_, b) = find(&sw, &owner, ws, "from:me 배포", "doc").await;
    assert!(!titles(&b).is_empty(), "{b}");

    // T5/T6: the same event twice, and events in the wrong order, give one document that equals
    // PostgreSQL.
    let page = Uuid::parse_str(&deploy).unwrap();
    api(
        world,
        "PATCH",
        &owner,
        &format!("/v1/docs/{deploy}"),
        Some(json!({"title":"배포 절차 v2"})),
    )
    .await;
    api(
        world,
        "PATCH",
        &owner,
        &format!("/v1/docs/{deploy}"),
        Some(json!({"title":"배포 절차 v3"})),
    )
    .await;
    let events = events_of(&world.pool, page).await;
    for event in events.iter().rev().chain(events.iter()) {
        sw.engine.indexer.apply(event).await.unwrap();
    }
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let (_, b) = find(&sw, &owner, ws, "배포 절차", "doc").await;
    assert_eq!(
        titles(&b)
            .iter()
            .filter(|t| t.starts_with("배포 절차"))
            .count(),
        1,
        "{b}"
    );
    assert!(titles(&b).contains(&"배포 절차 v3".to_owned()), "{b}");
    assert_eq!(indexed_count(&sw, "doc", ws).await, 4);

    // T11: a deleted page is gone from search at once, and from the index after the consumer ran.
    let (s, _) = api(world, "DELETE", &owner, &format!("/v1/docs/{kube}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, b) = find(&sw, &owner, ws, "kubectl", "doc").await;
    assert!(titles(&b).is_empty(), "hidden before the consumer ran: {b}");
    index_now(&sw, ws).await;
    assert_eq!(
        indexed_count(&sw, "doc", ws).await,
        3,
        "the document was removed"
    );
    // A late or replayed upsert event cannot resurrect it.
    let events = events_of(&world.pool, Uuid::parse_str(&kube).unwrap()).await;
    for event in &events {
        sw.engine.indexer.apply(event).await.unwrap();
    }
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    assert_eq!(indexed_count(&sw, "doc", ws).await, 3);
    let _ = compact;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t8_t9_t10_search_never_shows_a_restricted_page_to_someone_who_is_not_on_it(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let ws = workspace(world, &format!("p63-ds2-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (_, admin) = invite_member(world, ws, "ADMIN").await;
    let (bob_id, bob) = member(world, ws, "bob").await;
    let (carol_id, carol) = member(world, ws, "carol").await;
    let id = make_doc_with(
        world,
        &carol,
        ws,
        "인사 평가 기준",
        json!([para("민감한 인사 정보")]),
        json!({
        "visibility":"RESTRICTED","participants":[{"userId": bob_id, "role":"EDITOR"}]}),
    )
    .await;
    index_now(&sw, ws).await;
    // Participants find it; the index knows the text, PostgreSQL decides who sees it.
    for cookie in [&carol, &bob] {
        let (_, b) = find(&sw, cookie, ws, "인사", "doc").await;
        assert_eq!(titles(&b), vec!["인사 평가 기준"], "{b}");
    }
    // T8/T10: the Workspace owner and the admin are not participants: nothing, in any tab.
    for (who, cookie) in [("owner", &owner), ("admin", &admin)] {
        for tab in ["doc", "all"] {
            let (s, b) = find(&sw, cookie, ws, "인사", tab).await;
            assert_eq!(s, StatusCode::OK);
            assert!(titles(&b).is_empty(), "{who} {tab}: {b}");
        }
    }
    // T9: bob is removed. The index still holds the page (no consumer ran), and he gets nothing.
    let (s, v) = api(
        world,
        "PUT",
        &carol,
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants":[{"userId": carol_id, "role":"MANAGER"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        indexed_count(&sw, "doc", ws).await,
        1,
        "the document is still in the index"
    );
    let (_, b) = find(&sw, &bob, ws, "인사", "doc").await;
    assert!(titles(&b).is_empty(), "{b}");
    // Opening the page to the whole Workspace makes it findable at once, before any consumer.
    let (s, _) = api(
        world,
        "PUT",
        &carol,
        &format!("/v1/docs/{id}/visibility"),
        Some(json!({"visibility":"WORKSPACE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, b) = find(&sw, &bob, ws, "인사", "doc").await;
    assert_eq!(titles(&b), vec!["인사 평가 기준"], "{b}");
    // A deleted page is not returned although the document is still indexed.
    api(world, "DELETE", &carol, &format!("/v1/docs/{id}"), None).await;
    let (_, b) = find(&sw, &bob, ws, "인사", "doc").await;
    assert!(titles(&b).is_empty(), "{b}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t17_t18_docs_work_without_search_and_catch_up_and_recover_from_dead_deliveries(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let ws = workspace(world, &format!("p63-ds3-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let dead_config =
        penfii_core::search::client::OsConfig::with_prefix("http://127.0.0.1:9", "p62test");
    let dead = penfii_core::search::SearchEngine::new(
        world.pool.clone(),
        penfii_core::search::client::OsClient::new(dead_config).unwrap(),
    );
    let dead_app = penfii_http_api::router_with(
        world.pool.clone(),
        common::limits(),
        false,
        None,
        Some(dead.clone()),
    );
    // Everything about the page works with the index down: create, comment, rename.
    let id = make_doc_with(
        world,
        &owner,
        ws,
        "장애중 문서",
        json!([para("장애단어 본문")]),
        json!({}),
    )
    .await;
    let (s, _) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/docs/{id}/comments"),
        Some(json!({"body": doc("장애단어 댓글")})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = api(
        world,
        "PATCH",
        &owner,
        &format!("/v1/docs/{id}"),
        Some(json!({"title":"장애중 문서 개정"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let mut registry = penfii_core::events::Registry {
        backoff_base_secs: 0,
        max_backoff_secs: 0,
        max_attempts: 3,
        workspace: Some(ws),
        ..Default::default()
    };
    registry.external.push(penfii_core::search::consumer(&dead));
    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    // T18: the deliveries gave up (DEAD) and are listed; nothing is lost.
    let states = receipt_states(&world.pool, "search", ws).await;
    assert!(
        !states.is_empty() && states.iter().all(|(st, _)| st == "DEAD"),
        "{states:?}"
    );
    let (s, b, _) = call(
        &dead_app,
        "GET",
        &format!("/v1/workspaces/{ws}/search?q=장애단어&kind=doc"),
        Some(&owner),
        None,
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "T17: search 503, not a PostgreSQL substitute: {b}"
    );
    assert_eq!(b["code"], "search_unavailable");
    let (s, v) = api(world, "GET", &owner, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["title"], "장애중 문서 개정");
    let dead_list = penfii_core::events::dead_receipts(&world.pool, 1000)
        .await
        .unwrap();
    assert!(
        dead_list
            .iter()
            .any(|d| d.consumer == "search" && d.event_kind.starts_with("doc."))
    );
    // events-retry: the deliveries come back once and the document ends up there once.
    let requeued = penfii_core::events::requeue_dead(&world.pool, Some("search"), None)
        .await
        .unwrap();
    assert!(requeued >= 3, "{requeued}");
    index_now(&sw, ws).await;
    let (_, b) = find(&sw, &owner, ws, "장애단어", "doc").await;
    assert_eq!(titles(&b), vec!["장애중 문서 개정"], "{b}");
    assert_eq!(
        indexed_count(&sw, "doc", ws).await,
        1,
        "no duplicate document"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t23_the_all_tab_interleaves_kinds_and_pages_without_gaps_or_duplicates(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let ws = workspace(world, &format!("p63-ds4-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (project_id, _) = new_project(world, ws, "Mixed").await;
    let general = channel(world, ws, "general", "PUBLIC").await;
    for n in 0..6 {
        make_doc_with(
            world,
            &owner,
            ws,
            &format!("혼합검색 문서 {n}"),
            json!([para("혼합검색 본문")]),
            json!({}),
        )
        .await;
        make_ticket(
            world,
            &project_id,
            &format!("p63mix-{n}"),
            json!({"title": format!("혼합검색 티켓 {n}")}),
        )
        .await;
        send(
            world,
            &owner,
            general,
            &format!("혼합검색 메시지 {n}"),
            None,
            None,
        )
        .await;
    }
    index_now(&sw, ws).await;
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut path = format!(
            "/v1/workspaces/{ws}/search?q={}&kind=all&limit=4&tz=Asia/Seoul",
            "%ED%98%BC%ED%95%A9%EA%B2%80%EC%83%89"
        );
        if let Some(c) = &cursor {
            path.push_str(&format!("&cursor={c}"));
        }
        let (s, b, _) = call(&sw.app, "GET", &path, Some(&owner), None, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        for r in b["results"].as_array().unwrap() {
            seen.push((
                r["kind"].as_str().unwrap().to_owned(),
                r["id"].as_str().unwrap().to_owned(),
            ));
        }
        cursor = b["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    let unique: std::collections::BTreeSet<_> = seen.iter().cloned().collect();
    assert_eq!(unique.len(), seen.len(), "no duplicates across pages");
    for kind in ["doc", "ticket", "message"] {
        assert_eq!(
            seen.iter().filter(|(k, _)| k == kind).count(),
            6,
            "{kind}: {seen:?}"
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t19_a_rebuild_of_a_thousand_pages_keeps_counts_equal_and_search_working(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let ws = workspace(world, &format!("p63-ds5-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    // 1 200 pages straight into PostgreSQL (the shape the API writes).
    sqlx::query(
        "INSERT INTO penfii.doc_pages(id,workspace_id,title,visibility,position,content,content_schema_version,content_hash,created_by)
         SELECT gen_random_uuid(), $1, '대량문서 ' || g, 'WORKSPACE', g,
                jsonb_build_object('schema','penfii.doc','version',1,'blocks',jsonb_build_array(
                  jsonb_build_object('id','b'||g,'type','paragraph','props','{}'::jsonb,
                    'content',jsonb_build_array(jsonb_build_object('type','text','text','대량본문 ' || g,'styles','{}'::jsonb)),'children','[]'::jsonb))),
                1, md5(g::text), $2
         FROM generate_series(1,1200) g",
    )
    .bind(ws).bind(world.owner_id).execute(&world.pool).await.unwrap();
    // A full rebuild of the kind from PostgreSQL.
    sw.engine.indexer.request_reindex("doc").await.unwrap();
    // While it builds, search keeps answering from the old generation.
    let (s, _) = find(&sw, &owner, ws, "대량문서", "doc").await;
    assert_eq!(s, StatusCode::OK);
    sw.engine.indexer.backfill_all().await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    assert_eq!(indexed_count(&sw, "doc", ws).await, 1200);
    let (_, b) = find(&sw, &owner, ws, "대량본문 777", "doc").await;
    assert!(titles(&b).contains(&"대량문서 777".to_owned()), "{b}");
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.doc_pages WHERE deleted_at IS NULL")
            .fetch_one(&world.pool)
            .await
            .unwrap();
    let live = sw.engine.indexer.state("doc").await.unwrap();
    assert_eq!(live.phase, "READY");
    let count: i64 = {
        let (_, body) = sw
            .engine
            .indexer
            .client
            .admin(
                penfii_core::search::client::Method::GET,
                &format!("/{}/_count", sw.engine.indexer.client.alias("doc")),
                None,
            )
            .await
            .unwrap();
        body["count"].as_i64().unwrap()
    };
    assert_eq!(count, total, "the whole index equals the PostgreSQL rows");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn gap_h1_holds_for_the_comment_read_and_for_a_delete_of_a_page(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let ws = workspace(world, &format!("p63-h1c-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let id = make_doc_with(
        world,
        &owner,
        ws,
        "h1 page",
        json!([para("본문")]),
        json!({}),
    )
    .await;
    let doc_id = Uuid::parse_str(&id).unwrap();
    index_now(&sw, ws).await;
    let build = || async {
        let mut c = world.pool.acquire().await.unwrap();
        penfii_core::search::docs::build(&mut c, "doc", &[doc_id], &Default::default())
            .await
            .unwrap()
    };
    // The comment is part of the page's document and is read in the same snapshot.
    let old = build().await;
    let (s, v) = api(world, "POST", &owner, &format!("/v1/docs/{id}/comments"), Some(json!({"body":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"zetacomment"}]}]}}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let new = build().await;
    assert!(new[0].version > old[0].version);
    sw.engine.indexer.write("doc", new).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    sw.engine.indexer.write("doc", old).await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let (_, b) = find(&sw, &owner, ws, "zetacomment", "doc").await;
    assert_eq!(
        b["results"].as_array().unwrap().len(),
        1,
        "the stale read did not drop the comment: {b}"
    );
    // A delete read after an upsert wins even when the upsert is written last.
    let up = build().await;
    let (s, _) = api(world, "DELETE", &owner, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let del = build().await;
    assert!(del[0].doc.is_none() && del[0].version > up[0].version);
    sw.engine.indexer.write("doc", del).await.unwrap();
    sw.engine.indexer.write("doc", up).await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let (_, b) = find(&sw, &owner, ws, "zetacomment", "doc").await;
    assert!(b["results"].as_array().unwrap().is_empty(), "{b}");
}

/// T7 (H1): two writers read different states and write in the opposite order. The index must keep
/// the document built from the later read, because the version is the PostgreSQL clock read with the
/// data and not the clock of whichever process happens to write last.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t7_a_stale_read_written_last_does_not_overwrite_a_newer_read(pool: PgPool) {
    let world = &World::new(pool).await;
    let Some(sw) = search_world(world).await else {
        return;
    };
    let workspace_id = workspace(world, &format!("p63-t7-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "T7").await;
    let (_, ticket) = make_ticket(
        world,
        &project_id,
        "p63t7-1",
        json!({"title":"alphaone original"}),
    )
    .await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    index_now(&sw, workspace_id).await;
    let c = world.owner_cookie.clone();

    // Writer 1 reads the old state; the row then changes; writer 2 reads the new state.
    let r1 = {
        let mut conn = world.pool.acquire().await.unwrap();
        penfii_core::search::docs::build(&mut conn, "ticket", &[ticket_id], &Default::default())
            .await
            .unwrap()
    };
    sqlx::query("UPDATE penfii.tickets SET title='betatwo changed', revision_no=revision_no+1, updated_at=now() WHERE id=$1")
        .bind(ticket_id).execute(&world.pool).await.unwrap();
    let r2 = {
        let mut conn = world.pool.acquire().await.unwrap();
        penfii_core::search::docs::build(&mut conn, "ticket", &[ticket_id], &Default::default())
            .await
            .unwrap()
    };
    assert!(
        r2[0].version > r1[0].version,
        "a later read carries a larger version"
    );

    // Writer 2 is fast, writer 1 is slow and lands last.
    sw.engine.indexer.write("ticket", r2).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    sw.engine.indexer.write("ticket", r1).await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();

    let (_, b) = find(&sw, &c, workspace_id, "betatwo", "ticket").await;
    assert_eq!(
        b["results"].as_array().unwrap().len(),
        1,
        "the newer read stays in the index: {b}"
    );
    let (_, b) = find(&sw, &c, workspace_id, "alphaone", "ticket").await;
    assert!(
        b["results"].as_array().unwrap().is_empty(),
        "the stale read did not overwrite it: {b}"
    );

    // A delete read later than an upsert wins, and a stale upsert written after it cannot bring
    // the document back (the object is gone: the stale document is not the current state).
    let up = {
        let mut conn = world.pool.acquire().await.unwrap();
        penfii_core::search::docs::build(&mut conn, "ticket", &[ticket_id], &Default::default())
            .await
            .unwrap()
    };
    sqlx::query("UPDATE penfii.tickets SET trashed_at=now() WHERE id=$1")
        .bind(ticket_id)
        .execute(&world.pool)
        .await
        .unwrap();
    let del = {
        let mut conn = world.pool.acquire().await.unwrap();
        penfii_core::search::docs::build(&mut conn, "ticket", &[ticket_id], &Default::default())
            .await
            .unwrap()
    };
    assert!(del[0].doc.is_none() && del[0].version > up[0].version);
    sw.engine.indexer.write("ticket", del).await.unwrap();
    sw.engine.indexer.write("ticket", up).await.unwrap();
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let (_, b) = find(&sw, &c, workspace_id, "betatwo", "ticket").await;
    assert!(b["results"].as_array().unwrap().is_empty(), "{b}");
}
