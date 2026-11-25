//! MCP search operations against the router. Helpers: `crate::mcp_oauth` (`setup`, `setup_with`, `token_for`,
//! `modern_call`, `structured`, `error_code`, `rest`, `add_member`).

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, rest, setup_with, structured, token_for,
};

fn prefix_of(pool: &PgPool) -> String {
    use sha2::Digest;
    let database = pool.connect_options().get_database().unwrap().to_owned();
    let hash = sha2::Sha256::digest(database.as_bytes());
    let hex: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("t{hex}mcp")
}

async fn engine_for(pool: &PgPool) -> Option<penfii_core::search::SearchEngine> {
    let url = std::env::var("PENFII_TEST_OPENSEARCH_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())?;
    let config = penfii_core::search::client::OsConfig::with_prefix(&url, &prefix_of(pool));
    let client = penfii_core::search::client::OsClient::new(config).unwrap();
    let engine = penfii_core::search::SearchEngine::new(pool.clone(), client);
    engine.indexer.delete_everything_for_tests().await;
    engine
        .indexer
        .backfill_all()
        .await
        .expect("first build of every index");
    Some(engine)
}

async fn index_now(engine: &penfii_core::search::SearchEngine, workspace_id: Uuid) {
    let mut registry = penfii_core::events::Registry {
        backoff_base_secs: 0,
        max_backoff_secs: 0,
        max_attempts: 5,
        workspace: Some(workspace_id),
        ..Default::default()
    };
    registry
        .external
        .push(penfii_core::search::consumer(engine));
    penfii_core::events::run_until_idle(&engine.indexer.pool, &registry)
        .await
        .unwrap();
    penfii_core::search::refresh_all(engine).await.unwrap();
}

async fn find(env: &Env, token: &str, args: Value) -> crate::common::Reply {
    modern_call(&env.app, token, "search", args).await
}

fn kinds_of(reply: &crate::common::Reply) -> Vec<String> {
    structured(reply)["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_kinds_follow_scopes_and_core_visibility(pool: PgPool) {
    crate::common::migrate(&pool).await;
    let Some(engine) = engine_for(&pool).await else {
        eprintln!("SKIPPING: PENFII_TEST_OPENSEARCH_URL is not set");
        return;
    };
    let tweak_engine = engine.clone();
    let env = setup_with(pool, move |config| config.search = Some(tweak_engine)).await;
    let ws = env.workspace_id.to_string();
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;

    // Content: a project with a Ticket, an open Doc, a restricted Doc only the owner can read.
    let project = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/projects"),
        &env.owner_cookie,
        Some("p1"),
        Some(json!({"name": "Zebra project", "teamIds": []})),
    )
    .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let ticket = rest(
        &env,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        &env.owner_cookie,
        Some("t1"),
        Some(json!({"title": "Zebrafish ticket", "hierarchyKind": "TASK"})),
    )
    .await;
    assert!(ticket.status.is_success(), "{:?}", ticket.body);
    for (title, visibility, key) in [
        ("Zebrafish open doc", "WORKSPACE", "d1"),
        ("Zebrafish secret doc", "RESTRICTED", "d2"),
    ] {
        let mut body = json!({"title": title, "visibility": visibility});
        if visibility == "RESTRICTED" {
            body["participants"] = json!([{"userId": env.owner_id, "role": "MANAGER"}]);
        }
        let doc = rest(
            &env,
            "POST",
            &format!("/v1/workspaces/{ws}/docs"),
            &env.owner_cookie,
            Some(key),
            Some(body),
        )
        .await;
        assert!(doc.status.is_success(), "{:?}", doc.body);
    }
    index_now(&engine, env.workspace_id).await;

    // A grant with only ticket + search scopes: 'all' covers tickets (and people), docs need doc:read.
    let narrow = token_for(
        &env,
        &env.owner_cookie,
        "workspace:read search:read ticket:read member:read",
    )
    .await;
    let all = find(
        &env,
        &narrow,
        json!({"workspaceId": ws, "query": "Zebrafish"}),
    )
    .await;
    assert_eq!(all.body["result"]["isError"], false, "{:?}", all.body);
    let kinds = kinds_of(&all);
    assert!(
        kinds.iter().all(|k| k == "ticket" || k == "person"),
        "{kinds:?}"
    );
    assert!(kinds.contains(&"ticket".to_string()));
    let denied = find(
        &env,
        &narrow,
        json!({"workspaceId": ws, "query": "Zebrafish", "kind": "doc"}),
    )
    .await;
    assert_eq!(error_code(&denied), "insufficient_scope");
    let denied = find(
        &env,
        &narrow,
        json!({"workspaceId": ws, "query": "x", "kind": "nonsense"}),
    )
    .await;
    assert_eq!(error_code(&denied), "invalid_arguments");

    // With doc:read the owner finds both Docs; 'all' now includes them.
    let wide = token_for(
        &env,
        &env.owner_cookie,
        "workspace:read search:read ticket:read doc:read",
    )
    .await;
    let docs = find(
        &env,
        &wide,
        json!({"workspaceId": ws, "query": "Zebrafish", "kind": "doc"}),
    )
    .await;
    assert_eq!(docs.body["result"]["isError"], false, "{:?}", docs.body);
    assert_eq!(structured(&docs)["results"].as_array().unwrap().len(), 2);
    let all = find(
        &env,
        &wide,
        json!({"workspaceId": ws, "query": "Zebrafish"}),
    )
    .await;
    let kinds = kinds_of(&all);
    assert!(
        kinds.contains(&"ticket".to_string()) && kinds.contains(&"doc".to_string()),
        "{kinds:?}"
    );
    // Paging a single kind: limit 1 gives a cursor for the rest.
    let page = find(
        &env,
        &wide,
        json!({"workspaceId": ws, "query": "Zebrafish", "kind": "doc", "limit": 1}),
    )
    .await;
    assert_eq!(structured(&page)["results"].as_array().unwrap().len(), 1);
    let cursor = structured(&page)["nextCursor"]
        .as_str()
        .expect("a cursor")
        .to_string();
    let next = find(&env, &wide, json!({"workspaceId": ws, "query": "Zebrafish", "kind": "doc", "limit": 1, "cursor": cursor})).await;
    assert_eq!(structured(&next)["results"].as_array().unwrap().len(), 1);
    // A cursor cannot be mixed with several kinds that are searched one by one.
    let mixed = find(
        &env,
        &wide,
        json!({"workspaceId": ws, "query": "Zebrafish", "cursor": "abc"}),
    )
    .await;
    assert_eq!(error_code(&mixed), "invalid_arguments");

    // The member cannot see the restricted Doc through search, like the web search.
    let member = token_for(&env, &member_cookie, "workspace:read search:read doc:read").await;
    let docs = find(
        &env,
        &member,
        json!({"workspaceId": ws, "query": "Zebrafish", "kind": "doc"}),
    )
    .await;
    let titles: Vec<String> = structured(&docs)["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["Zebrafish open doc"], "{titles:?}");
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/search?q=Zebrafish&kind=doc"),
        &member_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.body["results"].as_array().unwrap().len(), 1);

    // Another Workspace's person gets not_found; a reader is not blocked from kinds they hold.
    let outsider = token_for(
        &env,
        &env.outsider_cookie,
        "workspace:read search:read doc:read",
    )
    .await;
    let denied = find(
        &env,
        &outsider,
        json!({"workspaceId": ws, "query": "Zebrafish", "kind": "doc"}),
    )
    .await;
    assert_eq!(error_code(&denied), "not_found");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn search_without_an_index_is_a_clear_error(pool: PgPool) {
    let env = setup_with(pool, |_| {}).await;
    let token = token_for(
        &env,
        &env.owner_cookie,
        "workspace:read search:read ticket:read",
    )
    .await;
    let reply = find(
        &env,
        &token,
        json!({"workspaceId": env.workspace_id.to_string(), "query": "x", "kind": "ticket"}),
    )
    .await;
    assert_eq!(error_code(&reply), "search_unavailable", "{:?}", reply.body);
}
