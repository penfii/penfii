//! MCP OAuth Authorization Server, Resource Server (`/mcp`), Workspace MCP Policy and the
//! first tool slices, against the router with an injected client-metadata fetcher. No network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::future::BoxFuture;
use penfii_core::CoreError;
use penfii_core::identity::{self, BootstrapInput};
use penfii_core::oauth::{ClientFetcher, OAuthConfig};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, OWNER_EMAIL, OWNER_PASSWORD, Reply, limits, send, sign_in as login};

pub(crate) const PUBLIC: &str = "https://penfii.test";
pub(crate) const CLIENT: &str = "https://client.example/oauth/client.json";
pub(crate) const CALLBACK: &str = "https://client.example/callback";
pub(crate) const VERIFIER: &str = "a-test-code-verifier-that-is-long-enough-0123456789";

#[derive(Default, Clone)]
pub(crate) struct FakeClients(Arc<Mutex<HashMap<String, Value>>>);

impl ClientFetcher for FakeClients {
    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<(Value, i64), CoreError>> {
        let found = self.0.lock().unwrap().get(url).cloned();
        Box::pin(async move {
            found
                .map(|doc| (doc, 600))
                .ok_or_else(|| CoreError::validation("no such client"))
        })
    }
}

pub(crate) async fn api(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Reply {
    send(app, common::request(method, path, cookie, None, body)).await
}

/// No CSRF header, no cookie: how real OAuth clients call.
pub(crate) async fn get_plain(app: &axum::Router, path: &str) -> Reply {
    send(
        app,
        Request::builder().uri(path).body(Body::empty()).unwrap(),
    )
    .await
}

pub(crate) async fn form(app: &axum::Router, path: &str, pairs: &[(&str, &str)]) -> Reply {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    send(
        app,
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

pub(crate) fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) struct Env {
    pub(crate) pool: PgPool,
    pub(crate) app: axum::Router,
    pub(crate) clients: FakeClients,
    pub(crate) owner_cookie: String,
    pub(crate) owner_id: Uuid,
    pub(crate) workspace_id: Uuid,
    pub(crate) outsider_cookie: String,
    pub(crate) changes: tokio::sync::broadcast::Sender<penfii_core::ChangeNotice>,
}

pub(crate) async fn setup(pool: PgPool) -> Env {
    setup_with(pool, |_| {}).await
}

/// `setup` with a change to the router configuration (object storage, mail, ...).
pub(crate) async fn setup_with(
    pool: PgPool,
    tweak: impl FnOnce(&mut penfii_http_api::RouterConfig),
) -> Env {
    let owner = common::setup(&pool).await;
    crate::common::migrate(&pool).await;
    identity::create_account(
        &pool,
        BootstrapInput {
            email: "outsider@penfii.test".into(),
            password: "outsider-test-passphrase".into(),
            display_name: "Outsider".into(),
            workspace_name: "Other".into(),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .unwrap();
    let clients = FakeClients::default();
    clients.0.lock().unwrap().insert(
        CLIENT.into(),
        json!({"client_id": CLIENT, "client_name": "Test Client",
               "redirect_uris": [CALLBACK, "http://127.0.0.1/callback"],
               "token_endpoint_auth_method": "none"}),
    );
    let (changes, _keep) = tokio::sync::broadcast::channel(64);
    let mut config = OAuthConfig::new(PUBLIC, Arc::new(clients.clone())).unwrap();
    config.dcr_enabled = true;
    config.allowed_origins = vec!["https://console.penfii.test".into()];
    let mut router_config = penfii_http_api::RouterConfig {
        changes: Some(changes.clone()),
        oauth: Some(config),
        ..penfii_http_api::RouterConfig::new(pool.clone(), limits(), false)
    };
    tweak(&mut router_config);
    let app = penfii_http_api::router_from(router_config);
    let owner_cookie = login(&app, OWNER_EMAIL, OWNER_PASSWORD).await;
    let outsider_cookie = login(&app, "outsider@penfii.test", "outsider-test-passphrase").await;
    Env {
        pool,
        app,
        clients,
        owner_cookie,
        owner_id: owner.user_id,
        workspace_id: owner.workspace_id,
        outsider_cookie,
        changes,
    }
}

pub(crate) fn authorize_path(
    client: &str,
    redirect: &str,
    scope: &str,
    challenge: &str,
    state: &str,
) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("response_type", "code"),
            ("client_id", client),
            ("redirect_uri", redirect),
            ("scope", scope),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("resource", &format!("{PUBLIC}/mcp")),
        ])
        .finish();
    format!("/oauth/authorize?{query}")
}

pub(crate) fn request_id(location: &str) -> String {
    location
        .strip_prefix("/consent?request=")
        .unwrap()
        .to_string()
}

pub(crate) fn query_of(location: &str) -> HashMap<String, String> {
    url::Url::parse(location)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Authorize + consent; returns the authorization code.
pub(crate) async fn get_code(env: &Env, cookie: &str, scope: &str, approve: &[&str]) -> String {
    let start = get_plain(
        &env.app,
        &authorize_path(CLIENT, CALLBACK, scope, &challenge_of(VERIFIER), "st-1"),
    )
    .await;
    assert_eq!(start.status, StatusCode::FOUND, "{:?}", start.body);
    let id = request_id(&start.location.unwrap());
    let decision = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(cookie),
        Some(json!({"approve": true, "scopes": approve})),
    )
    .await;
    assert_eq!(decision.status, StatusCode::OK, "{:?}", decision.body);
    let back = query_of(decision.body["redirectTo"].as_str().unwrap());
    assert_eq!(back["state"], "st-1");
    assert_eq!(back["iss"], PUBLIC);
    back["code"].clone()
}

pub(crate) async fn exchange(env: &Env, code: &str) -> Reply {
    form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", CALLBACK),
            ("client_id", CLIENT),
            ("code_verifier", VERIFIER),
        ],
    )
    .await
}

pub(crate) async fn token_pair(env: &Env, scope: &str, approve: &[&str]) -> (String, String) {
    let code = get_code(env, &env.owner_cookie, scope, approve).await;
    let reply = exchange(env, &code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    (
        reply.body["access_token"].as_str().unwrap().to_string(),
        reply.body["refresh_token"].as_str().unwrap().to_string(),
    )
}

pub(crate) async fn mcp(
    app: &axum::Router,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: Value,
) -> Reply {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    send(app, builder.body(Body::from(body.to_string())).unwrap()).await
}

pub(crate) fn modern_meta() -> Value {
    json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28",
           "io.modelcontextprotocol/clientInfo": {"name": "t", "version": "1"},
           "io.modelcontextprotocol/clientCapabilities": {}})
}

pub(crate) async fn modern_call(
    app: &axum::Router,
    token: &str,
    tool: &str,
    arguments: Value,
) -> Reply {
    mcp(
        app,
        Some(token),
        &[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", tool),
        ],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": tool, "arguments": arguments, "_meta": modern_meta()}}),
    )
    .await
}

pub(crate) async fn modern_list(app: &axum::Router, token: &str) -> Reply {
    mcp(
        app,
        Some(token),
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/list")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": modern_meta()}}),
    )
    .await
}

pub(crate) fn tool_names(reply: &Reply) -> Vec<String> {
    reply.body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// The tools of the first release. Later tools appear in `tools/list` too; the tests that pin the
/// exact first-release list for a token look at these only.
pub(crate) const FIRST_RELEASE_TOOLS: &[&str] = &[
    "workspace_list",
    "workspace_overview",
    "member_search",
    "search",
    "ticket_get",
    "ticket_comments_list",
    "project_workflow",
    "ticket_create",
    "ticket_update",
    "ticket_change_status",
    "ticket_comment",
    "project_create",
    "workflow_state_delete",
];

pub(crate) fn first_release_tool_names(reply: &Reply) -> Vec<String> {
    tool_names(reply)
        .into_iter()
        .filter(|name| FIRST_RELEASE_TOOLS.contains(&name.as_str()))
        .collect()
}

pub(crate) const ALL_SCOPES: &[&str] = &[
    "workspace:read",
    "member:read",
    "search:read",
    "ticket:read",
    "ticket:write",
];
pub(crate) const ALL_SCOPE_TEXT: &str =
    "workspace:read member:read search:read ticket:read ticket:write";

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn metadata_and_challenge_follow_the_adr(pool: PgPool) {
    let env = setup(pool).await;
    let server = get_plain(&env.app, "/.well-known/oauth-authorization-server").await;
    assert_eq!(server.status, StatusCode::OK);
    let m = &server.body;
    assert_eq!(m["issuer"], PUBLIC);
    assert_eq!(
        m["authorization_endpoint"],
        format!("{PUBLIC}/oauth/authorize")
    );
    assert_eq!(m["token_endpoint"], format!("{PUBLIC}/oauth/token"));
    assert_eq!(m["revocation_endpoint"], format!("{PUBLIC}/oauth/revoke"));
    assert_eq!(m["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(m["token_endpoint_auth_methods_supported"], json!(["none"]));
    assert_eq!(m["client_id_metadata_document_supported"], true);
    assert_eq!(m["authorization_response_iss_parameter_supported"], true);
    assert_eq!(
        m["grant_types_supported"],
        json!(["authorization_code", "refresh_token"])
    );
    assert!(
        m["scopes_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("offline_access"))
    );
    assert!(
        m["registration_endpoint"].is_string(),
        "DCR flag is on in this test"
    );
    let alias = get_plain(&env.app, "/.well-known/openid-configuration").await;
    assert_eq!(alias.body, server.body);

    let resource = get_plain(&env.app, "/.well-known/oauth-protected-resource/mcp").await;
    assert_eq!(resource.body["resource"], format!("{PUBLIC}/mcp"));
    assert_eq!(resource.body["authorization_servers"], json!([PUBLIC]));
    assert_eq!(resource.body["bearer_methods_supported"], json!(["header"]));
    let root = get_plain(&env.app, "/.well-known/oauth-protected-resource").await;
    assert_eq!(root.body, resource.body);

    // No token: 401 with a challenge that points at the resource metadata.
    let anonymous = mcp(
        &env.app,
        None,
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let challenge = anonymous.header("www-authenticate").unwrap();
    assert!(challenge.starts_with("Bearer "));
    assert!(challenge.contains(&format!(
        "resource_metadata=\"{PUBLIC}/.well-known/oauth-protected-resource/mcp\""
    )));
    assert!(challenge.contains("scope=\""));
    // A token in the query string is never looked at.
    let query = send(
        &env.app,
        Request::builder()
            .method("POST")
            .uri("/mcp?access_token=pfa_nothing")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(query.status, StatusCode::UNAUTHORIZED);
    // GET has no stream here.
    let got = get_plain(&env.app, "/mcp").await;
    assert_eq!(got.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn full_flow_tokens_tools_refresh_and_revocation(pool: PgPool) {
    let env = setup(pool).await;
    let (access, refresh) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    assert!(access.starts_with("pfa_") && refresh.starts_with("pfr_"));

    // Tokens are stored as digests only.
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.oauth_tokens WHERE token_hash = $1")
            .bind(Sha256::digest(access.as_bytes()).to_vec())
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(stored, 1);

    // Modern era: list, call, attribution.
    let listed = modern_list(&env.app, &access).await;
    assert_eq!(listed.status, StatusCode::OK, "{:?}", listed.body);
    assert_eq!(listed.body["result"]["resultType"], "complete");
    assert_eq!(listed.body["result"]["cacheScope"], "private");
    assert_eq!(
        first_release_tool_names(&listed),
        [
            "workspace_list",
            "workspace_overview",
            "member_search",
            "ticket_get",
            "ticket_comments_list",
            "project_workflow",
            "ticket_create",
            "ticket_update",
            "ticket_change_status",
            "ticket_comment",
            // Not callable with this grant (project:write), listed so a client can ask for it.
            "project_create",
            "search"
        ]
    );
    let called = modern_call(&env.app, &access, "workspace_list", json!({})).await;
    assert_eq!(called.status, StatusCode::OK);
    let result = &called.body["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(
        result["structuredContent"]["workspaces"][0]["id"],
        env.workspace_id.to_string()
    );
    let request_id = result["_meta"]["penfii/requestId"].as_str().unwrap();
    let logged: (String, String, String) = sqlx::query_as(
        "SELECT operation, outcome, risk_class FROM penfii.mcp_call_log WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        logged,
        ("workspace.list".into(), "ok".into(), "read".into())
    );
    let actor: Uuid =
        sqlx::query_scalar("SELECT user_id FROM penfii.mcp_call_log WHERE request_id = $1")
            .bind(request_id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        actor, env.owner_id,
        "the actor is the person, not an MCP user"
    );

    let overview = modern_call(
        &env.app,
        &access,
        "workspace_overview",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(overview.body["result"]["isError"], false);
    let teammate = identity::create_account(
        &env.pool,
        BootstrapInput {
            email: "mate@penfii.test".into(),
            password: "mate-test-passphrase-1".into(),
            display_name: "Teammate".into(),
            workspace_name: "Mate".into(),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, 'MEMBER', 'ACTIVE')",
    )
    .bind(env.workspace_id)
    .bind(teammate.user_id)
    .execute(&env.pool)
    .await
    .unwrap();
    let people = modern_call(
        &env.app,
        &access,
        "member_search",
        json!({"workspaceId": env.workspace_id, "query": "mate"}),
    )
    .await;
    assert_eq!(
        people.body["result"]["structuredContent"]["results"][0]["title"], "Teammate",
        "{:?}",
        people.body
    );

    // Legacy era on the same endpoint, no session id issued.
    let init = mcp(
        &env.app,
        Some(&access),
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
    )
    .await;
    assert_eq!(init.body["result"]["protocolVersion"], "2025-11-25");
    assert!(init.body["result"]["instructions"].as_str().unwrap().len() > 100);
    let legacy_list = mcp(
        &env.app,
        Some(&access),
        &[("mcp-protocol-version", "2025-11-25")],
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    assert_eq!(first_release_tool_names(&legacy_list).len(), 12);
    assert!(legacy_list.body["result"].get("resultType").is_none());
    let ping = mcp(
        &env.app,
        Some(&access),
        &[],
        json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}),
    )
    .await;
    assert_eq!(ping.body["result"], json!({}));

    // server/discover works as a first request; nothing needs it.
    let discover = mcp(
        &env.app,
        Some(&access),
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "server/discover")],
        json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": {"_meta": modern_meta()}}),
    )
    .await;
    assert_eq!(
        discover.body["result"]["supportedVersions"],
        json!(["2026-07-28", "2025-11-25"])
    );

    // Refresh rotation: a new pair; the old access token stops working at once.
    let rotated = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", CLIENT),
        ],
    )
    .await;
    assert_eq!(rotated.status, StatusCode::OK, "{:?}", rotated.body);
    let access2 = rotated.body["access_token"].as_str().unwrap().to_string();
    let refresh2 = rotated.body["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(refresh2, refresh);
    assert_eq!(
        modern_list(&env.app, &access).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(modern_list(&env.app, &access2).await.status, StatusCode::OK);

    // Narrowing is fine, widening is not.
    let widen = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh2),
            ("client_id", CLIENT),
            ("scope", "workspace:read document:read"),
        ],
    )
    .await;
    assert_eq!(widen.body["error"], "invalid_scope");

    // Reusing the rotated-out refresh token revokes the whole family.
    let reuse = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", CLIENT),
        ],
    )
    .await;
    assert_eq!(reuse.status, StatusCode::BAD_REQUEST);
    assert_eq!(reuse.body["error"], "invalid_grant");
    assert_eq!(
        modern_list(&env.app, &access2).await.status,
        StatusCode::UNAUTHORIZED
    );
    let dead = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh2),
            ("client_id", CLIENT),
        ],
    )
    .await;
    assert_eq!(dead.body["error"], "invalid_grant");

    // RFC 7009: revocation answers 200 whether or not the token exists.
    let (access3, refresh3) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    assert_eq!(
        form(&env.app, "/oauth/revoke", &[("token", "pfr_unknown")])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        form(&env.app, "/oauth/revoke", &[("token", &refresh3)])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        modern_list(&env.app, &access3).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn authorization_endpoint_rejects_unsafe_requests(pool: PgPool) {
    let env = setup(pool).await;
    let challenge = challenge_of(VERIFIER);

    // PKCE downgrade: redirected with an error and `iss`, and nothing is stored.
    let plain = get_plain(
        &env.app,
        &authorize_path(CLIENT, CALLBACK, "workspace:read", &challenge, "s")
            .replace("code_challenge_method=S256", "code_challenge_method=plain"),
    )
    .await;
    assert_eq!(plain.status, StatusCode::FOUND);
    let back = query_of(plain.location.as_deref().unwrap());
    assert_eq!(back["error"], "invalid_request");
    assert_eq!(back["iss"], PUBLIC);
    assert_eq!(back["state"], "s");
    let missing = get_plain(
        &env.app,
        &authorize_path(CLIENT, CALLBACK, "workspace:read", "", "s"),
    )
    .await;
    assert_eq!(
        query_of(missing.location.as_deref().unwrap())["error"],
        "invalid_request"
    );

    // Redirect mismatch and an unknown client never redirect (a Penfii error page instead).
    let wrong = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            "https://client.example/other",
            "workspace:read",
            &challenge,
            "s",
        ),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert!(wrong.location.is_none());
    let unknown = get_plain(
        &env.app,
        &authorize_path(
            "https://nobody.example/c.json",
            CALLBACK,
            "workspace:read",
            &challenge,
            "s",
        ),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert!(unknown.location.is_none());

    // Resource mismatch is invalid_target.
    let resource = get_plain(
        &env.app,
        &authorize_path(CLIENT, CALLBACK, "workspace:read", &challenge, "s").replace(
            "https%3A%2F%2Fpenfii.test%2Fmcp",
            "https%3A%2F%2Fother.example%2Fmcp",
        ),
    )
    .await;
    assert_eq!(
        query_of(resource.location.as_deref().unwrap())["error"],
        "invalid_target"
    );

    // Loopback redirect: the port is ignored (RFC 8252), nothing else is.
    let loopback = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            "http://127.0.0.1:53211/callback",
            "workspace:read",
            &challenge,
            "s",
        ),
    )
    .await;
    assert_eq!(loopback.status, StatusCode::FOUND);
    assert!(loopback.location.unwrap().starts_with("/consent?request="));
    let other_path = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            "http://127.0.0.1:53211/evil",
            "workspace:read",
            &challenge,
            "s",
        ),
    )
    .await;
    assert_eq!(other_path.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn consent_belongs_to_one_person_and_is_single_use(pool: PgPool) {
    let env = setup(pool).await;
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read ticket:read bogus:scope",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    // No session: unauthenticated.
    let anonymous = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let view = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(view.status, StatusCode::OK);
    assert_eq!(view.body["clientName"], "Test Client");
    assert_eq!(view.body["redirectHost"], "client.example");
    let offered: Vec<&str> = view.body["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["scope"].as_str().unwrap())
        .collect();
    assert_eq!(
        offered,
        ["workspace:read", "ticket:read"],
        "unknown scopes are dropped"
    );
    // The first person to load it owns it.
    let other = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.outsider_cookie),
        None,
    )
    .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    let stolen = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.outsider_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read"]})),
    )
    .await;
    assert_eq!(stolen.status, StatusCode::NOT_FOUND);
    // Scope can be removed, never added.
    let widen = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read", "member:read"]})),
    )
    .await;
    assert_eq!(widen.status, StatusCode::BAD_REQUEST);
    // The failed attempt consumed nothing that matters: a clean request is needed again.
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read ticket:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let narrowed = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read"]})),
    )
    .await;
    assert_eq!(narrowed.status, StatusCode::OK);
    let code = query_of(narrowed.body["redirectTo"].as_str().unwrap())["code"].clone();
    let token = exchange(&env, &code).await;
    assert_eq!(token.body["scope"], "workspace:read");
    let again = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read"]})),
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::NOT_FOUND,
        "a decision is single use"
    );
    // Denying redirects with access_denied and iss.
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let denied = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": false})),
    )
    .await;
    let back = query_of(denied.body["redirectTo"].as_str().unwrap());
    assert_eq!(back["error"], "access_denied");
    assert_eq!(back["iss"], PUBLIC);
    // CSRF rule holds for the decision (cookie request without the header).
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let forged = send(
        &env.app,
        Request::builder()
            .method("POST")
            .uri(format!("/v1/oauth/requests/{id}/decision"))
            .header("cookie", &env.owner_cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"approve": true, "scopes": ["workspace:read"]}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn token_endpoint_is_strict(pool: PgPool) {
    let env = setup(pool).await;
    let code = get_code(&env, &env.owner_cookie, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    // Wrong verifier / redirect / client.
    let bad_verifier = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", CALLBACK),
            ("client_id", CLIENT),
            (
                "code_verifier",
                "x-wrong-verifier-that-is-long-enough-to-pass-length-0123456789",
            ),
        ],
    )
    .await;
    assert_eq!(bad_verifier.body["error"], "invalid_grant");
    let bad_redirect = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "https://client.example/other"),
            ("client_id", CLIENT),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(bad_redirect.body["error"], "invalid_grant");
    let bad_resource = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", CALLBACK),
            ("client_id", CLIENT),
            ("code_verifier", VERIFIER),
            ("resource", "https://other.example/mcp"),
        ],
    )
    .await;
    assert_eq!(bad_resource.body["error"], "invalid_target");
    // The code still works once with the right values; a second use fails AND kills the first's tokens.
    let first = exchange(&env, &code).await;
    assert_eq!(first.status, StatusCode::OK);
    let access = first.body["access_token"].as_str().unwrap().to_string();
    assert_eq!(modern_list(&env.app, &access).await.status, StatusCode::OK);
    let second = exchange(&env, &code).await;
    assert_eq!(second.body["error"], "invalid_grant");
    assert_eq!(
        modern_list(&env.app, &access).await.status,
        StatusCode::UNAUTHORIZED,
        "code reuse revokes the tokens"
    );
    // Grant types come from the registry.
    let unknown = form(
        &env.app,
        "/oauth/token",
        &[("grant_type", "client_credentials")],
    )
    .await;
    assert_eq!(unknown.body["error"], "unsupported_grant_type");
    // Expired code.
    let code = get_code(
        &env,
        &env.owner_cookie,
        "workspace:read",
        &["workspace:read"],
    )
    .await;
    sqlx::query("UPDATE penfii.oauth_codes SET expires_at = now() - interval '1 second'")
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(exchange(&env, &code).await.body["error"], "invalid_grant");
    // Errors are never cached.
    let revoked = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", "pfr_nope"),
            ("client_id", CLIENT),
        ],
    )
    .await;
    assert_eq!(revoked.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scope_policy_and_authorization_are_checked_on_every_call(pool: PgPool) {
    let env = setup(pool).await;
    // A token with only workspace:read.
    let (narrow, _) = token_pair(&env, "workspace:read", &["workspace:read"]).await;
    // The scope does not hide a tool: what it lists is what the policy allows (here the standard
    // one, so the destructive and administrative tools are not there).
    let narrow_all = tool_names(&modern_list(&env.app, &narrow).await);
    for listed in [
        "workspace_list",
        "doc_get",
        "message_list",
        "ticket_list",
        "doc_save",
        "message_send",
        "invitation_create",
        "project_create",
    ] {
        assert!(narrow_all.contains(&listed.to_string()), "{listed}");
    }
    assert!(!narrow_all.contains(&"workflow_state_delete".to_string()));
    // Calling a listed tool without its scope is refused at call time: 403 + challenge.
    let denied = modern_call(
        &env.app,
        &narrow,
        "member_search",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert!(
        denied
            .header("www-authenticate")
            .unwrap()
            .contains("insufficient_scope")
    );
    assert_eq!(denied.body["result"]["isError"], true);
    assert_eq!(
        denied.body["result"]["structuredContent"]["error"]["code"],
        "insufficient_scope"
    );
    assert!(denied.body["result"]["_meta"]["mcp/www_authenticate"].is_array());
    let unknown = modern_call(&env.app, &narrow, "no_such_tool", json!({})).await;
    assert_eq!(unknown.body["error"]["code"], -32602);

    // search kinds follow the scope: without ticket:read an explicit kind is refused.
    let (partial, _) = token_pair(
        &env,
        "workspace:read search:read",
        &["workspace:read", "search:read"],
    )
    .await;
    let kind = modern_call(
        &env.app,
        &partial,
        "search",
        json!({"workspaceId": env.workspace_id, "query": "x", "kind": "ticket"}),
    )
    .await;
    assert_eq!(
        kind.body["result"]["structuredContent"]["error"]["code"],
        "insufficient_scope"
    );
    let all = modern_call(
        &env.app,
        &partial,
        "search",
        json!({"workspaceId": env.workspace_id, "query": "x"}),
    )
    .await;
    assert_eq!(
        all.body["result"]["structuredContent"]["kinds"],
        json!([]),
        "no readable kind, nothing searched"
    );

    // Full-scope token for the owner (a new consent replaces the grant, so this comes last).
    let (access, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    // A person outside the Workspace gets not_found, never policy_denied or the data.
    let (outsider_access, _) = {
        let code = get_code(&env, &env.outsider_cookie, ALL_SCOPE_TEXT, ALL_SCOPES).await;
        let reply = exchange(&env, &code).await;
        (reply.body["access_token"].as_str().unwrap().to_string(), ())
    };
    let hidden = modern_call(
        &env.app,
        &outsider_access,
        "workspace_overview",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(
        hidden.body["result"]["structuredContent"]["error"]["code"],
        "not_found"
    );
    let listed = modern_call(&env.app, &outsider_access, "workspace_list", json!({})).await;
    let names: Vec<_> = listed.body["result"]["structuredContent"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Other"]);

    // Workspace policy: the owner turns MCP off.
    let get = api(
        &env.app,
        "GET",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(get.body["isDefault"], true);
    assert_eq!(get.body["mcpEnabled"], true);
    assert_eq!(get.body["allowWrite"], true);
    assert_eq!(get.body["allowDestructive"], false);
    assert_eq!(get.body["allowAdministrative"], false);
    // The outsider cannot read or change it.
    let peek = api(
        &env.app,
        "GET",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.outsider_cookie),
        None,
    )
    .await;
    assert_eq!(peek.status, StatusCode::NOT_FOUND);
    let off = json!({"mcpEnabled": false, "allowWrite": false, "allowDestructive": false, "allowAdministrative": false});
    let tamper = api(
        &env.app,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.outsider_cookie),
        Some(off.clone()),
    )
    .await;
    assert_eq!(tamper.status, StatusCode::NOT_FOUND);
    let put = api(
        &env.app,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.owner_cookie),
        Some(off),
    )
    .await;
    assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
    assert_eq!(put.body["isDefault"], false);
    // The very next call is refused for a member; the audit row exists.
    let blocked = modern_call(
        &env.app,
        &access,
        "workspace_overview",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(
        blocked.body["result"]["structuredContent"]["error"]["code"],
        "policy_denied"
    );
    let hidden_workspaces = modern_call(&env.app, &access, "workspace_list", json!({})).await;
    assert_eq!(
        hidden_workspaces.body["result"]["structuredContent"]["workspaces"],
        json!([])
    );
    // tools/list hides what no reachable Workspace allows.
    assert!(tool_names(&modern_list(&env.app, &access).await).is_empty());
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'workspace.mcp_policy_changed'")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(audited, 1);
    let outcomes: Vec<String> = sqlx::query_scalar("SELECT outcome FROM penfii.mcp_call_log WHERE operation = 'workspace.overview' ORDER BY at")
        .fetch_all(&env.pool)
        .await
        .unwrap();
    assert!(outcomes.contains(&"policy_denied".to_string()));
    // Turn it back on: effect from the next call.
    let on = json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": false, "allowAdministrative": false});
    api(
        &env.app,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.owner_cookie),
        Some(on),
    )
    .await;
    let back = modern_call(
        &env.app,
        &access,
        "workspace_overview",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(back.body["result"]["isError"], false);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn users_and_grants_end_tokens_at_once(pool: PgPool) {
    let env = setup(pool).await;
    let (access, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    let grants = api(
        &env.app,
        "GET",
        "/v1/me/oauth-grants",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(grants.body["items"][0]["clientName"], "Test Client");
    let id = grants.body["items"][0]["id"].as_str().unwrap().to_string();
    // Somebody else's grant is not found.
    let foreign = api(
        &env.app,
        "DELETE",
        &format!("/v1/me/oauth-grants/{id}"),
        Some(&env.outsider_cookie),
        None,
    )
    .await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);
    // A new consent replaces the grant and kills the old tokens (one live grant per client).
    let (second, _) = token_pair(&env, "workspace:read", &["workspace:read"]).await;
    assert_eq!(
        modern_list(&env.app, &access).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(modern_list(&env.app, &second).await.status, StatusCode::OK);
    let live: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.oauth_grants WHERE revoked_at IS NULL")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(live, 1);
    // Sign out everywhere does not revoke grants; disabling the user does end them.
    api(
        &env.app,
        "DELETE",
        "/v1/me/sessions",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(modern_list(&env.app, &second).await.status, StatusCode::OK);
    sqlx::query("UPDATE penfii.users SET disabled_at = now() WHERE id = $1")
        .bind(env.owner_id)
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        modern_list(&env.app, &second).await.status,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE penfii.users SET disabled_at = NULL WHERE id = $1")
        .bind(env.owner_id)
        .execute(&env.pool)
        .await
        .unwrap();
    // The user revokes the grant: the token dies on the next request.
    let grants = api(
        &env.app,
        "GET",
        "/v1/me/oauth-grants",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let id = grants.body["items"][0]["id"].as_str().unwrap().to_string();
    let revoked = api(
        &env.app,
        "DELETE",
        &format!("/v1/me/oauth-grants/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    assert_eq!(
        modern_list(&env.app, &second).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn protocol_eras_and_headers(pool: PgPool) {
    let env = setup(pool).await;
    let (access, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    // Unsupported modern version -> 400 with the supported list (-32022).
    let mut meta = modern_meta();
    meta["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let unsupported = mcp(
        &env.app,
        Some(&access),
        &[
            ("mcp-protocol-version", "1900-01-01"),
            ("mcp-method", "tools/list"),
        ],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta}}),
    )
    .await;
    assert_eq!(unsupported.status, StatusCode::BAD_REQUEST);
    assert_eq!(unsupported.body["error"]["code"], -32022);
    assert_eq!(
        unsupported.body["error"]["data"]["supported"],
        json!(["2026-07-28", "2025-11-25"])
    );
    // Header mismatch / missing -> 400 -32020.
    let mismatch = mcp(
        &env.app,
        Some(&access),
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/list")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "workspace_list", "_meta": modern_meta()}}),
    )
    .await;
    assert_eq!(mismatch.status, StatusCode::BAD_REQUEST);
    assert_eq!(mismatch.body["error"]["code"], -32020);
    let no_name = mcp(
        &env.app,
        Some(&access),
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/call")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "workspace_list", "_meta": modern_meta()}}),
    )
    .await;
    assert_eq!(no_name.body["error"]["code"], -32020);
    // The base64 sentinel form of Mcp-Name is accepted.
    let encoded = format!(
        "=?base64?{}?=",
        base64::engine::general_purpose::STANDARD.encode("workspace_list")
    );
    let sentinel = mcp(
        &env.app,
        Some(&access),
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/call"), ("mcp-name", &encoded)],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "workspace_list", "arguments": {}, "_meta": modern_meta()}}),
    )
    .await;
    assert_eq!(sentinel.body["result"]["isError"], false);
    // Unknown modern method -> 404 -32601. Notifications -> 202 with no body.
    let unknown = mcp(
        &env.app,
        Some(&access),
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "resources/list")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list", "params": {"_meta": modern_meta()}}),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body["error"]["code"], -32601);
    let note = mcp(
        &env.app,
        Some(&access),
        &[],
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(note.status, StatusCode::ACCEPTED);
    // Garbage and batches are refused; a session id header is ignored.
    let batch = mcp(
        &env.app,
        Some(&access),
        &[],
        json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]),
    )
    .await;
    assert_eq!(batch.status, StatusCode::BAD_REQUEST);
    let with_session = mcp(
        &env.app,
        Some(&access),
        &[("mcp-session-id", "abc")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    )
    .await;
    assert_eq!(with_session.status, StatusCode::OK);
    // Every first request works on its own: tools/call with no discover, no initialize.
    let direct = modern_call(&env.app, &access, "workspace_list", json!({})).await;
    assert_eq!(direct.body["result"]["isError"], false);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn origin_and_host_are_checked_and_mcp_tokens_stay_off_v1(pool: PgPool) {
    let env = setup(pool).await;
    let (access, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let call = |origin: Option<&'static str>, host: Option<&'static str>| {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {access}"));
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        if let Some(host) = host {
            builder = builder.header("host", host);
        }
        send(&env.app, builder.body(Body::from(body.clone())).unwrap())
    };
    assert_eq!(
        call(Some("https://evil.example"), None).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(Some(PUBLIC), None).await.status, StatusCode::OK);
    // The same origin, written the way a header may be: case, a slash, spaces. And the one extra
    // origin the operator listed. Nothing near them is accepted, and no Origin at all is allowed.
    for origin in [
        "HTTPS://Penfii.Test",
        "https://penfii.test/",
        " https://penfii.test ",
        "https://console.penfii.test",
    ] {
        assert_eq!(
            call(Some(origin), None).await.status,
            StatusCode::OK,
            "{origin}"
        );
    }
    for origin in [
        "null",
        "",
        "http://penfii.test",
        "https://penfii.test:8443",
        "https://evil.penfii.test",
        "https://penfii.test.evil.example",
        "https://penfii.test@evil.example",
    ] {
        assert_eq!(
            call(Some(origin), None).await.status,
            StatusCode::FORBIDDEN,
            "{origin:?}"
        );
    }
    assert_eq!(call(None, None).await.status, StatusCode::OK);
    // A header that is not text is refused, not read as "no Origin".
    let unreadable = send(
        &env.app,
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {access}"))
            .header(
                "origin",
                axum::http::HeaderValue::from_bytes(b"https://penfii.test\xff").unwrap(),
            )
            .body(Body::from(body.clone()))
            .unwrap(),
    )
    .await;
    assert_eq!(unreadable.status, StatusCode::FORBIDDEN);
    assert_eq!(
        call(None, Some("evil.example")).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(None, Some("penfii.test")).await.status, StatusCode::OK);
    // An MCP access token is not a session; /v1 only takes the cookie.
    let as_cookie = api(
        &env.app,
        "GET",
        "/v1/me",
        Some(&format!("penfii_session={access}")),
        None,
    )
    .await;
    assert_eq!(as_cookie.status, StatusCode::UNAUTHORIZED);
    let as_bearer = send(
        &env.app,
        Request::builder()
            .uri("/v1/me")
            .header("authorization", format!("Bearer {access}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(as_bearer.status, StatusCode::UNAUTHORIZED);
    // And a session token is not an MCP token.
    let session_token = env
        .owner_cookie
        .trim_start_matches("penfii_session=")
        .to_string();
    let reversed = mcp(
        &env.app,
        Some(&session_token),
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    )
    .await;
    assert_eq!(reversed.status, StatusCode::UNAUTHORIZED);
    // A token whose resource differs is refused.
    sqlx::query("UPDATE penfii.oauth_tokens SET resource = 'https://other.example/mcp'")
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(call(None, None).await.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn client_metadata_documents_are_validated_and_dcr_is_public_only(pool: PgPool) {
    let env = setup(pool).await;
    // client_id in the document must equal the URL.
    env.clients.0.lock().unwrap().insert(
        "https://liar.example/c.json".into(),
        json!({"client_id": "https://other.example/c.json", "client_name": "Liar", "redirect_uris": [CALLBACK]}),
    );
    let liar = get_plain(
        &env.app,
        &authorize_path(
            "https://liar.example/c.json",
            CALLBACK,
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    assert_eq!(liar.status, StatusCode::BAD_REQUEST);
    // A secret-bearing or confidential document is refused.
    env.clients.0.lock().unwrap().insert(
        "https://conf.example/c.json".into(),
        json!({"client_id": "https://conf.example/c.json", "client_name": "C", "redirect_uris": [CALLBACK], "token_endpoint_auth_method": "client_secret_basic"}),
    );
    let conf = get_plain(
        &env.app,
        &authorize_path(
            "https://conf.example/c.json",
            CALLBACK,
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    assert_eq!(conf.status, StatusCode::BAD_REQUEST);
    // DCR: public clients, https / loopback redirect URIs only.
    let reg = api(&env.app, "POST", "/oauth/register", None, Some(json!({"client_name": "Dyn", "redirect_uris": ["http://127.0.0.1:1234/cb"], "token_endpoint_auth_method": "none"}))).await;
    assert_eq!(reg.status, StatusCode::CREATED, "{:?}", reg.body);
    assert!(reg.body["client_id"].as_str().unwrap().starts_with("pfi_"));
    assert!(reg.body.get("client_secret").is_none());
    let bad_uri = api(
        &env.app,
        "POST",
        "/oauth/register",
        None,
        Some(json!({"redirect_uris": ["javascript:alert(1)"]})),
    )
    .await;
    assert_eq!(bad_uri.status, StatusCode::BAD_REQUEST);
    let confidential = api(&env.app, "POST", "/oauth/register", None, Some(json!({"redirect_uris": ["https://a.example/cb"], "token_endpoint_auth_method": "client_secret_post"}))).await;
    assert_eq!(confidential.status, StatusCode::BAD_REQUEST);
    // A registered id works end to end.
    let id = reg.body["client_id"].as_str().unwrap();
    let start = get_plain(
        &env.app,
        &authorize_path(
            id,
            "http://127.0.0.1:9999/cb",
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    assert_eq!(start.status, StatusCode::FOUND);
    // ... through the token endpoint, with no client authentication.
    let code = code_for(&env, id, "http://127.0.0.1:9999/cb").await;
    let tokens = token_with(
        &env.app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "http://127.0.0.1:9999/cb"),
            ("client_id", id),
            ("code_verifier", VERIFIER),
        ],
        None,
    )
    .await;
    assert_eq!(tokens.status, StatusCode::OK, "{:?}", tokens.body);
}

pub(crate) const CHATGPT: &str = "https://chatgpt.com/oauth/client.json";
pub(crate) const CHATGPT_CALLBACK: &str = "https://chatgpt.com/connector_platform_oauth_redirect";

/// The document ChatGPT published (2026-11): a `private_key_jwt` preference, `none` among the
/// supported methods, a JWKS URL and a JWT bearer grant this server does not offer.
pub(crate) fn chatgpt_document() -> Value {
    json!({
        "client_id": CHATGPT,
        "client_uri": "https://chatgpt.com/",
        "redirect_uris": [CHATGPT_CALLBACK],
        "token_endpoint_auth_method": "private_key_jwt",
        "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
        "grant_types": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:jwt-bearer"],
        "response_types": ["code"],
        "client_name": "ChatGPT",
        "logo_uri": "https://persistent.oaistatic.com/sonic/misc/openai-logo.png",
        "token_endpoint_auth_signing_alg": "RS256",
        "jwks_uri": "https://chatgpt.com/oauth/jwks.json"
    })
}

/// Authorize + approve `workspace:read` for any client; returns the code.
pub(crate) async fn code_for(env: &Env, client: &str, redirect: &str) -> String {
    let start = get_plain(
        &env.app,
        &authorize_path(
            client,
            redirect,
            "workspace:read",
            &challenge_of(VERIFIER),
            "st-x",
        ),
    )
    .await;
    assert_eq!(start.status, StatusCode::FOUND, "{:?}", start.body);
    let id = request_id(&start.location.unwrap());
    let decision = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read"]})),
    )
    .await;
    assert_eq!(decision.status, StatusCode::OK, "{:?}", decision.body);
    let back = query_of(decision.body["redirectTo"].as_str().unwrap());
    assert!(
        decision.body["redirectTo"]
            .as_str()
            .unwrap()
            .starts_with(redirect)
    );
    assert_eq!(back["iss"], PUBLIC);
    back["code"].clone()
}

pub(crate) async fn token_with(
    app: &axum::Router,
    pairs: &[(&str, &str)],
    authorization: Option<&str>,
) -> Reply {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let mut builder = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(value) = authorization {
        builder = builder.header("authorization", value);
    }
    send(app, builder.body(Body::from(body)).unwrap()).await
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_document_listing_none_among_its_methods_connects_as_a_public_client(pool: PgPool) {
    let env = setup(pool).await;
    env.clients
        .0
        .lock()
        .unwrap()
        .insert(CHATGPT.into(), chatgpt_document());
    let code = code_for(&env, CHATGPT, CHATGPT_CALLBACK).await;
    let (name, kind): (String, String) =
        sqlx::query_as("SELECT client_name, kind FROM penfii.oauth_clients WHERE client_id = $1")
            .bind(CHATGPT)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!((name.as_str(), kind.as_str()), ("ChatGPT", "cimd"));
    let resource = format!("{PUBLIC}/mcp");
    let exchange_pairs = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", CHATGPT_CALLBACK),
        ("client_id", CHATGPT),
        ("code_verifier", VERIFIER),
        ("resource", resource.as_str()),
    ];

    // The server chose `none`: client authentication in any form is invalid_client, and the
    // refusal leaves the code unused.
    let with_param = |name: &'static str, value: &'static str| {
        let mut pairs = exchange_pairs.to_vec();
        pairs.push((name, value));
        pairs
    };
    for pairs in [
        with_param("client_secret", "s3cret"),
        with_param("client_assertion", "eyJhbGciOiJSUzI1NiJ9.e30.sig"),
        with_param(
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
        ),
    ] {
        let refused = token_with(&env.app, &pairs, None).await;
        assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{pairs:?}");
        assert_eq!(refused.body["error"], "invalid_client");
        assert!(refused.header("www-authenticate").is_none());
        assert_eq!(refused.header("cache-control"), Some("no-store"));
    }
    let basic = token_with(
        &env.app,
        &exchange_pairs,
        Some("Basic Y2hhdGdwdDpzM2NyZXQ="),
    )
    .await;
    assert_eq!(basic.status, StatusCode::UNAUTHORIZED);
    assert_eq!(basic.body["error"], "invalid_client");
    assert_eq!(
        basic.header("www-authenticate"),
        Some("Basic realm=\"penfii\"")
    );
    let odd = token_with(&env.app, &exchange_pairs, Some("Weird\"scheme x")).await;
    assert_eq!(
        odd.header("www-authenticate"),
        Some("Basic realm=\"penfii\"")
    );

    // An empty client_secret is no authentication.
    let tokens = token_with(&env.app, &with_param("client_secret", ""), None).await;
    assert_eq!(tokens.status, StatusCode::OK, "{:?}", tokens.body);
    assert_eq!(tokens.body["scope"], "workspace:read");
    let access = tokens.body["access_token"].as_str().unwrap();
    let refresh = tokens.body["refresh_token"].as_str().unwrap();
    assert_eq!(modern_list(&env.app, access).await.status, StatusCode::OK);

    // Refresh follows the same rule.
    let refresh_pairs = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", CHATGPT),
        ("resource", resource.as_str()),
    ];
    let mut assertion = refresh_pairs.to_vec();
    assertion.push(("client_assertion", "eyJhbGciOiJSUzI1NiJ9.e30.sig"));
    let refused = token_with(&env.app, &assertion, None).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(refused.body["error"], "invalid_client");
    let rotated = token_with(&env.app, &refresh_pairs, None).await;
    assert_eq!(
        rotated.status,
        StatusCode::OK,
        "the refused try did not use the token: {:?}",
        rotated.body
    );
    let access2 = rotated.body["access_token"].as_str().unwrap();
    assert_eq!(modern_list(&env.app, access2).await.status, StatusCode::OK);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn client_documents_are_checked_for_a_method_this_server_supports(pool: PgPool) {
    let env = setup(pool).await;
    let cb = "https://docs.example/callback";
    let doc = |url: &str, extra: Value| {
        let mut document = json!({"client_id": url, "client_name": "Doc", "redirect_uris": [cb, "http://127.0.0.1/callback"]});
        for (key, value) in extra.as_object().unwrap() {
            document[key] = value.clone();
        }
        document
    };
    let cases = [
        // Accepted: what Claude Code and Codex publish, no method at all, the plural list alone.
        (
            "claude",
            json!({"token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"]}),
            true,
        ),
        (
            "codex",
            json!({"application_type": "native", "token_endpoint_auth_method": "none", "token_endpoint_auth_methods_supported": ["none"]}),
            true,
        ),
        ("bare", json!({}), true),
        (
            "plural",
            json!({"token_endpoint_auth_methods_supported": ["private_key_jwt", "none"], "jwks_uri": "https://docs.example/jwks.json"}),
            true,
        ),
        // Refused: no supported method, a contradiction, a shared secret, two key sources.
        (
            "jwt-only",
            json!({"token_endpoint_auth_methods_supported": ["private_key_jwt"], "jwks_uri": "https://docs.example/jwks.json"}),
            false,
        ),
        (
            "jwt-single",
            json!({"token_endpoint_auth_method": "private_key_jwt"}),
            false,
        ),
        (
            "mismatch",
            json!({"token_endpoint_auth_method": "private_key_jwt", "token_endpoint_auth_methods_supported": ["none"]}),
            false,
        ),
        (
            "secret-listed",
            json!({"token_endpoint_auth_methods_supported": ["none", "client_secret_basic"]}),
            false,
        ),
        (
            "secret-expiry",
            json!({"token_endpoint_auth_method": "none", "client_secret_expires_at": 0}),
            false,
        ),
        (
            "two-keys",
            json!({"token_endpoint_auth_methods_supported": ["none", "private_key_jwt"], "jwks_uri": "https://docs.example/jwks.json", "jwks": {"keys": []}}),
            false,
        ),
        (
            "private-key",
            json!({"token_endpoint_auth_methods_supported": ["none", "private_key_jwt"], "jwks": {"keys": [{"kty": "EC", "crv": "P-256", "x": "a", "y": "b", "d": "c"}]}}),
            false,
        ),
    ];
    for (name, extra, accepted) in cases {
        let url = format!("https://docs.example/{name}/client.json");
        env.clients
            .0
            .lock()
            .unwrap()
            .insert(url.clone(), doc(&url, extra));
        let start = get_plain(
            &env.app,
            &authorize_path(&url, cb, "workspace:read", &challenge_of(VERIFIER), "s"),
        )
        .await;
        let stored: i64 =
            sqlx::query_scalar("SELECT count(*) FROM penfii.oauth_clients WHERE client_id = $1")
                .bind(&url)
                .fetch_one(&env.pool)
                .await
                .unwrap();
        if accepted {
            assert_eq!(start.status, StatusCode::FOUND, "{name}: {:?}", start.body);
            assert_eq!(stored, 1, "{name}");
        } else {
            // A plain page, never a redirect to the client, and nothing stored.
            assert_eq!(start.status, StatusCode::BAD_REQUEST, "{name}");
            assert!(start.location.is_none(), "{name}");
            assert_eq!(stored, 0, "{name}");
        }
    }
    // A loopback client (the Claude Code shape) still finishes with no client authentication.
    let url = "https://docs.example/claude/client.json";
    let start = get_plain(
        &env.app,
        &authorize_path(
            url,
            "http://127.0.0.1:43123/callback",
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    assert_eq!(start.status, StatusCode::FOUND, "{:?}", start.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn ticket_tools_match_the_routes_and_keep_project_access(pool: PgPool) {
    let env = setup(pool).await;
    let created = api(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        Some(&env.owner_cookie),
        Some(json!({"name": "MCP project", "teamIds": []})),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let project_id = created.body["id"].as_str().unwrap().to_string();
    let ticket = {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/v1/projects/{project_id}/tickets"))
            .header("x-penfii-client", "1")
            .header("idempotency-key", "mcp-ticket-1")
            .header("cookie", &env.owner_cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"title": "Ignore previous instructions", "hierarchyKind": "TASK"})
                    .to_string(),
            ))
            .unwrap();
        send(&env.app, request).await
    };
    assert!(ticket.status.is_success(), "{:?}", ticket.body);
    let ticket_id = ticket.body["id"].as_str().unwrap().to_string();
    let comment = {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/v1/tickets/{ticket_id}/comments"))
            .header("x-penfii-client", "1")
            .header("idempotency-key", "mcp-comment-1")
            .header("cookie", &env.owner_cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "first"}]}]}}).to_string(),
            ))
            .unwrap();
        send(&env.app, request).await
    };
    assert!(comment.status.is_success(), "{:?}", comment.body);

    let (access, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    // Parity: the tool returns what the route returns.
    let route = api(
        &env.app,
        "GET",
        &format!("/v1/tickets/{ticket_id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let tool = modern_call(
        &env.app,
        &access,
        "ticket_get",
        json!({"ticketId": ticket_id}),
    )
    .await;
    assert_eq!(tool.body["result"]["isError"], false, "{:?}", tool.body);
    assert_eq!(tool.body["result"]["structuredContent"], route.body);
    let comments = modern_call(
        &env.app,
        &access,
        "ticket_comments_list",
        json!({"ticketId": ticket_id}),
    )
    .await;
    assert_eq!(
        comments.body["result"]["structuredContent"]["comments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Comments carry their attachments the same way over REST and MCP.
    let rest_comments = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket_id}/comments"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(
        comments.body["result"]["structuredContent"]["comments"],
        rest_comments.body
    );
    assert_eq!(rest_comments.body[0]["attachments"], json!([]));
    let logged: (String, Option<String>) = sqlx::query_as(
        "SELECT resource_type, resource_id FROM penfii.mcp_call_log WHERE operation = 'ticket.get'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(logged, ("ticket".into(), Some(ticket_id.clone())));

    // A bad id and an unknown id are the same answer.
    let bad = modern_call(&env.app, &access, "ticket_get", json!({"ticketId": "nope"})).await;
    assert_eq!(
        bad.body["result"]["structuredContent"]["error"]["code"],
        "invalid_arguments"
    );
    let missing = modern_call(
        &env.app,
        &access,
        "ticket_get",
        json!({"ticketId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(
        missing.body["result"]["structuredContent"]["error"]["code"],
        "not_found"
    );

    // A Workspace member who is not in the Project gets what the web app gives: not found.
    let mate = identity::create_account(
        &env.pool,
        BootstrapInput {
            email: "mate2@penfii.test".into(),
            password: "mate-test-passphrase-1".into(),
            display_name: "Mate".into(),
            workspace_name: "Mate".into(),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, 'MEMBER', 'ACTIVE')",
    )
    .bind(env.workspace_id)
    .bind(mate.user_id)
    .execute(&env.pool)
    .await
    .unwrap();
    let mate_cookie = login(&env.app, "mate2@penfii.test", "mate-test-passphrase-1").await;
    let web = api(
        &env.app,
        "GET",
        &format!("/v1/tickets/{ticket_id}"),
        Some(&mate_cookie),
        None,
    )
    .await;
    let code = {
        let start = get_plain(
            &env.app,
            &authorize_path(
                CLIENT,
                CALLBACK,
                ALL_SCOPE_TEXT,
                &challenge_of(VERIFIER),
                "s",
            ),
        )
        .await;
        let id = request_id(&start.location.unwrap());
        let decision = api(
            &env.app,
            "POST",
            &format!("/v1/oauth/requests/{id}/decision"),
            Some(&mate_cookie),
            Some(json!({"approve": true, "scopes": ALL_SCOPES})),
        )
        .await;
        query_of(decision.body["redirectTo"].as_str().unwrap())["code"].clone()
    };
    let mate_token = exchange(&env, &code).await.body["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let via_mcp = modern_call(
        &env.app,
        &mate_token,
        "ticket_get",
        json!({"ticketId": ticket_id}),
    )
    .await;
    if web.status == StatusCode::NOT_FOUND {
        assert_eq!(
            via_mcp.body["result"]["structuredContent"]["error"]["code"],
            "not_found"
        );
    } else {
        // The web app shows it to this member, so MCP must too, and with the same content.
        assert_eq!(via_mcp.body["result"]["structuredContent"], web.body);
    }
}

pub(crate) async fn rest(
    env: &Env,
    method: &str,
    path: &str,
    cookie: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> Reply {
    send(
        &env.app,
        common::request(method, path, Some(cookie), key, body),
    )
    .await
}

pub(crate) async fn token_for(env: &Env, cookie: &str, scope: &str) -> String {
    let scopes: Vec<&str> = scope.split(' ').collect();
    let code = get_code(env, cookie, scope, &scopes).await;
    exchange(env, &code).await.body["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A token for `scope` from another MCP client (`client_id` is its metadata document URL, `name` the
/// name it shows), approved by the person signed in with `cookie`.
pub(crate) async fn token_for_client(
    env: &Env,
    cookie: &str,
    client_id: &str,
    name: &str,
    scope: &str,
) -> String {
    let callback = format!("{}/callback", client_id.rsplit_once('/').unwrap().0);
    env.clients.0.lock().unwrap().insert(
        client_id.into(),
        json!({"client_id": client_id, "client_name": name, "redirect_uris": [callback],
               "token_endpoint_auth_method": "none"}),
    );
    let start = get_plain(
        &env.app,
        &authorize_path(client_id, &callback, scope, &challenge_of(VERIFIER), "st-c"),
    )
    .await;
    assert_eq!(start.status, StatusCode::FOUND, "{:?}", start.body);
    let id = request_id(&start.location.unwrap());
    let scopes: Vec<&str> = scope.split(' ').collect();
    let decision = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(cookie),
        Some(json!({"approve": true, "scopes": scopes})),
    )
    .await;
    assert_eq!(decision.status, StatusCode::OK, "{:?}", decision.body);
    let code = query_of(decision.body["redirectTo"].as_str().unwrap())["code"].clone();
    let reply = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &callback),
            ("client_id", client_id),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    reply.body["access_token"].as_str().unwrap().to_string()
}

pub(crate) async fn add_member(env: &Env, email: &str, role: &str) -> String {
    let account = identity::create_account(
        &env.pool,
        BootstrapInput {
            email: email.into(),
            password: "member-test-passphrase".into(),
            display_name: email.split('@').next().unwrap().into(),
            workspace_name: format!("{email} home"),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, $3, 'ACTIVE')",
    )
    .bind(env.workspace_id)
    .bind(account.user_id)
    .bind(role)
    .execute(&env.pool)
    .await
    .unwrap();
    login(&env.app, email, "member-test-passphrase").await
}

pub(crate) fn structured(reply: &Reply) -> &Value {
    &reply.body["result"]["structuredContent"]
}

pub(crate) fn error_code(reply: &Reply) -> &str {
    reply.body["result"]["structuredContent"]["error"]["code"]
        .as_str()
        .unwrap_or("")
}

pub(crate) fn tool_destructive(reply: &Reply, name: &str) -> bool {
    reply.body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == name)
        .map(|tool| tool["annotations"]["destructiveHint"] == true)
        .unwrap_or(false)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn write_tools_are_ports_of_the_routes_with_the_same_effects_and_rules(pool: PgPool) {
    let env = setup(pool).await;
    let mut realtime = env.changes.subscribe();
    let project = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": "Write project", "teamIds": []})),
    )
    .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let workflow = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/workflow"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let states = workflow.body["states"].as_array().unwrap();
    let done_state = states.iter().find(|st| st["category"] == "DONE").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // The agent can learn state ids: project_workflow returns what the route returns.
    let reader = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let via_mcp = modern_call(
        &env.app,
        &reader,
        "project_workflow",
        json!({"projectId": project_id}),
    )
    .await;
    assert_eq!(
        via_mcp.body["result"]["isError"], false,
        "{:?}",
        via_mcp.body
    );
    assert_eq!(structured(&via_mcp)["states"], workflow.body["states"]);
    assert_eq!(structured(&via_mcp)["version"], workflow.body["version"]);

    let owner = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let args = json!({"projectId": project_id, "title": "From MCP", "description": "line one", "idempotencyKey": "k-1"});
    let first = modern_call(&env.app, &owner, "ticket_create", args.clone()).await;
    assert_eq!(first.body["result"]["isError"], false, "{:?}", first.body);
    let ticket_id = structured(&first)["id"].as_str().unwrap().to_string();
    let again = modern_call(&env.app, &owner, "ticket_create", args).await;
    assert_eq!(
        structured(&again)["id"],
        ticket_id.as_str(),
        "same key, same Ticket"
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.tickets WHERE title = 'From MCP'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(rows, 1);
    let mut saw_change = false;
    while let Ok(notice) = realtime.try_recv() {
        let text = format!("{notice:?}");
        saw_change |= text.contains(&project_id) || text.contains(&ticket_id);
    }
    assert!(saw_change, "a ChangeNotice follows an MCP write");
    let web = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket_id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.body["title"], "From MCP");
    let revision = web.body["revisionNo"].as_i64().unwrap();
    let actor: Option<Uuid> = sqlx::query_scalar(
        "SELECT actor_user_id FROM penfii.ticket_revisions WHERE ticket_id = $1 AND revision_no = $2",
    )
    .bind(Uuid::parse_str(&ticket_id).unwrap())
    .bind(revision)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(actor, Some(env.owner_id));
    // The revision names the call that wrote it (its command id is the call's log row), and the
    // log row names the Ticket but claims no revision of its own.
    let logged: (String, Option<String>, Option<i64>, String) = sqlx::query_as(
        "SELECT l.operation, l.resource_id, l.resource_revision, l.outcome
         FROM penfii.mcp_call_log l JOIN penfii.ticket_revisions r ON r.command_id = l.id
         WHERE r.ticket_id = $1 AND r.revision_no = $2",
    )
    .bind(Uuid::parse_str(&ticket_id).unwrap())
    .bind(revision)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        logged,
        (
            "ticket.create".into(),
            Some(ticket_id.clone()),
            None,
            "ok".into()
        )
    );

    let stale = modern_call(
        &env.app,
        &owner,
        "ticket_update",
        json!({"ticketId": ticket_id, "expectedRevision": revision - 1, "title": "x"}),
    )
    .await;
    assert_eq!(stale.body["result"]["isError"], true, "{:?}", stale.body);
    let updated = modern_call(&env.app, &owner, "ticket_update", json!({"ticketId": ticket_id, "expectedRevision": revision, "title": "Renamed", "priority": "HIGH"})).await;
    assert_eq!(
        updated.body["result"]["isError"], false,
        "{:?}",
        updated.body
    );
    assert_eq!(structured(&updated)["title"], "Renamed");
    let rev2 = structured(&updated)["revisionNo"].as_i64().unwrap();
    assert_eq!(rev2, revision + 1);
    let cleared = modern_call(
        &env.app,
        &owner,
        "ticket_update",
        json!({"ticketId": ticket_id, "expectedRevision": rev2, "priority": null}),
    )
    .await;
    assert!(
        structured(&cleared)["priority"].is_null(),
        "{:?}",
        cleared.body
    );
    let rev3 = structured(&cleared)["revisionNo"].as_i64().unwrap();
    let moved = modern_call(
        &env.app,
        &owner,
        "ticket_change_status",
        json!({"ticketId": ticket_id, "expectedRevision": rev3, "statusId": done_state}),
    )
    .await;
    assert_eq!(moved.body["result"]["isError"], false, "{:?}", moved.body);
    assert_eq!(structured(&moved)["statusId"], done_state.as_str());
    let commented = modern_call(
        &env.app,
        &owner,
        "ticket_comment",
        json!({"ticketId": ticket_id, "body": "from the assistant"}),
    )
    .await;
    assert_eq!(
        commented.body["result"]["isError"], false,
        "{:?}",
        commented.body
    );
    let comments = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket_id}/comments"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(comments.body.as_array().unwrap().len(), 1);
    assert_eq!(comments.body[0]["authorId"], env.owner_id.to_string());

    let reader = token_for(&env, &env.owner_cookie, "workspace:read ticket:read").await;
    let denied = modern_call(
        &env.app,
        &reader,
        "ticket_create",
        json!({"projectId": project_id, "title": "no"}),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&denied), "insufficient_scope");
    let policy_path = format!("/v1/workspaces/{}/mcp-policy", env.workspace_id);
    let read_only = json!({"mcpEnabled": true, "allowWrite": false, "allowDestructive": false, "allowAdministrative": false});
    assert_eq!(
        rest(
            &env,
            "PUT",
            &policy_path,
            &env.owner_cookie,
            None,
            Some(read_only)
        )
        .await
        .status,
        StatusCode::OK
    );
    let owner = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let blocked = modern_call(
        &env.app,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "blocked"}),
    )
    .await;
    assert_eq!(error_code(&blocked), "policy_denied");
    let names = tool_names(&modern_list(&env.app, &owner).await);
    assert!(
        names.contains(&"ticket_get".to_string()) && !names.contains(&"ticket_create".to_string())
    );
    assert_eq!(
        modern_call(
            &env.app,
            &owner,
            "ticket_get",
            json!({"ticketId": ticket_id})
        )
        .await
        .body["result"]["isError"],
        false
    );
    let writes_on = json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": false, "allowAdministrative": false});
    rest(
        &env,
        "PUT",
        &policy_path,
        &env.owner_cookie,
        None,
        Some(writes_on),
    )
    .await;
    let after = modern_call(
        &env.app,
        &owner,
        "ticket_comment",
        json!({"ticketId": ticket_id, "body": "again"}),
    )
    .await;
    assert_eq!(
        after.body["result"]["isError"], false,
        "policy change applies at the next call"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn writes_follow_existing_roles_for_owner_admin_member_outsider_and_disabled(pool: PgPool) {
    let env = setup(pool).await;
    let project = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": "Roles", "teamIds": []})),
    )
    .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let scopes = ALL_SCOPE_TEXT;
    let admin_cookie = add_member(&env, "admin@penfii.test", "ADMIN").await;
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let admin = token_for(&env, &admin_cookie, scopes).await;
    let member = token_for(&env, &member_cookie, scopes).await;
    let owner = token_for(&env, &env.owner_cookie, scopes).await;
    let outsider = token_for(&env, &env.outsider_cookie, scopes).await;

    for (name, cookie, token) in [
        ("owner", env.owner_cookie.clone(), owner.clone()),
        ("admin", admin_cookie.clone(), admin.clone()),
        ("member", member_cookie.clone(), member.clone()),
        ("outsider", env.outsider_cookie.clone(), outsider.clone()),
    ] {
        let web = rest(
            &env,
            "POST",
            &format!("/v1/projects/{project_id}/tickets"),
            &cookie,
            Some(&format!("web-{name}")),
            Some(json!({"title": format!("web {name}"), "hierarchyKind": "TASK"})),
        )
        .await;
        let mcp_reply = modern_call(
            &env.app,
            &token,
            "ticket_create",
            json!({"projectId": project_id, "title": format!("mcp {name}")}),
        )
        .await;
        let allowed_web = web.status.is_success();
        let allowed_mcp = mcp_reply.body["result"]["isError"] == false;
        assert_eq!(
            allowed_web,
            allowed_mcp,
            "{name}: web {} vs mcp {:?}",
            web.status,
            structured(&mcp_reply)
        );
        if !allowed_web {
            let expect = if web.status == StatusCode::NOT_FOUND {
                "not_found"
            } else {
                "forbidden"
            };
            assert_eq!(error_code(&mcp_reply), expect, "{name}");
        }
    }
    assert_eq!(
        error_code(
            &modern_call(
                &env.app,
                &outsider,
                "ticket_create",
                json!({"projectId": project_id, "title": "x"})
            )
            .await
        ),
        "not_found"
    );

    let manage = "workspace:read project:manage ticket:read";
    let admin_manage = token_for(&env, &admin_cookie, manage).await;
    let member_manage = token_for(&env, &member_cookie, manage).await;
    let owner_manage = token_for(&env, &env.owner_cookie, manage).await;
    let make = |name: &str| json!({"workspaceId": env.workspace_id, "name": name});
    assert_eq!(
        error_code(
            &modern_call(
                &env.app,
                &owner_manage,
                "project_create",
                make("Created under the standard policy")
            )
            .await
        ),
        "",
        "creating a Project is routine management (Write): the standard policy allows it"
    );
    let allow_admin = json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": false, "allowAdministrative": true});
    let policy_path = format!("/v1/workspaces/{}/mcp-policy", env.workspace_id);
    assert_eq!(
        rest(
            &env,
            "PUT",
            &policy_path,
            &member_cookie,
            None,
            Some(allow_admin.clone())
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        rest(
            &env,
            "PUT",
            &policy_path,
            &admin_cookie,
            None,
            Some(allow_admin)
        )
        .await
        .status,
        StatusCode::OK
    );
    let web_member = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &member_cookie,
        None,
        Some(json!({"name": "web member project", "teamIds": []})),
    )
    .await;
    let mcp_member = modern_call(
        &env.app,
        &member_manage,
        "project_create",
        make("mcp member project"),
    )
    .await;
    assert_eq!(
        web_member.status.is_success(),
        mcp_member.body["result"]["isError"] == false
    );
    // Core lets only an owner or admin create a Project; MCP gives the same answer as the web route.
    assert_eq!(
        web_member.status.is_success(),
        error_code(&mcp_member).is_empty()
    );
    let created = modern_call(
        &env.app,
        &admin_manage,
        "project_create",
        make("MCP project"),
    )
    .await;
    assert_eq!(
        created.body["result"]["isError"], false,
        "{:?}",
        created.body
    );
    let new_project = structured(&created)["id"].as_str().unwrap().to_string();
    let workflow = rest(
        &env,
        "GET",
        &format!("/v1/projects/{new_project}/workflow"),
        &admin_cookie,
        None,
        None,
    )
    .await;
    let version = workflow.body["version"].as_i64().unwrap();
    let added = rest(&env, "POST", &format!("/v1/projects/{new_project}/workflow/states"), &admin_cookie, None, Some(json!({"expectedWorkflowVersion": version, "name": "Spare", "category": "IN_PROGRESS", "position": 50}))).await;
    assert!(added.status.is_success(), "{:?}", added.body);
    let spare = added.body["id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| added.body["state"]["id"].as_str().unwrap().to_string());
    let version = added.body["version"]
        .as_i64()
        .or(added.body["workflowVersion"].as_i64())
        .unwrap_or(version + 1);
    let delete_args =
        json!({"projectId": new_project, "stateId": spare, "expectedWorkflowVersion": version});
    assert_eq!(
        error_code(
            &modern_call(
                &env.app,
                &admin_manage,
                "workflow_state_delete",
                delete_args.clone()
            )
            .await
        ),
        "policy_denied"
    );
    let allow_all = json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": true, "allowAdministrative": true});
    rest(
        &env,
        "PUT",
        &policy_path,
        &admin_cookie,
        None,
        Some(allow_all),
    )
    .await;
    let deleted = modern_call(
        &env.app,
        &admin_manage,
        "workflow_state_delete",
        delete_args,
    )
    .await;
    assert_eq!(
        deleted.body["result"]["isError"], false,
        "{:?}",
        deleted.body
    );
    assert!(tool_destructive(
        &modern_list(&env.app, &admin_manage).await,
        "workflow_state_delete"
    ));

    sqlx::query(
        "UPDATE penfii.users SET disabled_at = now() WHERE email_key = 'member@penfii.test'",
    )
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        modern_call(
            &env.app,
            &member,
            "ticket_create",
            json!({"projectId": project_id, "title": "ghost"})
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    let outcomes: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT outcome FROM penfii.mcp_call_log")
            .fetch_all(&env.pool)
            .await
            .unwrap();
    for wanted in ["ok", "policy_denied", "not_found"] {
        assert!(
            outcomes.contains(&wanted.to_string()),
            "{wanted} in {outcomes:?}"
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn high_risk_scopes_need_recent_authentication_at_consent(pool: PgPool) {
    let env = setup(pool).await;
    let challenge = challenge_of(VERIFIER);
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read project:manage",
            &challenge,
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let view = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let risky = view.body["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["highRisk"] == true)
        .count();
    assert_eq!(risky, 1, "project:manage is high-risk");
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '2 hours', authenticated_at = now() - interval '2 hours'")
        .execute(&env.pool)
        .await
        .unwrap();
    let body = json!({"approve": true, "scopes": ["workspace:read", "project:manage"]});
    let stale = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(body.clone()),
    )
    .await;
    assert_eq!(stale.status, StatusCode::FORBIDDEN);
    assert_eq!(stale.body["code"], "reauth_required");
    let start2 = get_plain(
        &env.app,
        &authorize_path(CLIENT, CALLBACK, "workspace:read", &challenge, "s"),
    )
    .await;
    let id2 = request_id(&start2.location.unwrap());
    let calm = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id2}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read"]})),
    )
    .await;
    assert_eq!(calm.status, StatusCode::OK);
    let proof = api(
        &env.app,
        "POST",
        "/v1/me/reauth/password",
        Some(&env.owner_cookie),
        Some(json!({"password": "owner-test-passphrase"})),
    )
    .await;
    assert_eq!(proof.status, StatusCode::NO_CONTENT);
    let start3 = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read project:manage",
            &challenge,
            "s",
        ),
    )
    .await;
    let id3 = request_id(&start3.location.unwrap());
    let fresh = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id3}/decision"),
        Some(&env.owner_cookie),
        Some(body),
    )
    .await;
    assert_eq!(fresh.status, StatusCode::OK, "{:?}", fresh.body);
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '2 hours'")
        .execute(&env.pool)
        .await
        .unwrap();
    let policy = json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": true, "allowAdministrative": false});
    let put = api(
        &env.app,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        Some(&env.owner_cookie),
        Some(policy),
    )
    .await;
    assert_eq!(put.status, StatusCode::FORBIDDEN);
    assert_eq!(put.body["code"], "reauth_required");
    let all = api(
        &env.app,
        "DELETE",
        "/v1/me/oauth-grants",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(all.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn default_consent_is_read_only_and_writes_need_an_explicit_request_and_tick(pool: PgPool) {
    let env = setup(pool).await;
    // The challenge and a request without `scope` name read scopes only.
    let anonymous = mcp(
        &env.app,
        None,
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let challenge = anonymous.header("www-authenticate").unwrap();
    assert!(challenge.contains("workspace:read"), "{challenge}");
    assert!(!challenge.contains("write"), "{challenge}");
    assert!(!challenge.contains("manage"), "{challenge}");
    let start = get_plain(
        &env.app,
        &authorize_path(CLIENT, CALLBACK, "", &challenge_of(VERIFIER), "s"),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let view = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let listed: Vec<&str> = view.body["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["scope"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&"ticket:read"));
    assert!(!listed.contains(&"ticket:write") && !listed.contains(&"project:manage"));

    // A client that asks for the write scope gets it flagged as a change, unchecked by default.
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read ticket:read ticket:write",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let view = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let flags: HashMap<String, bool> = view.body["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["scope"].as_str().unwrap().to_string(),
                s["change"] == true,
            )
        })
        .collect();
    assert!(flags["ticket:write"]);
    assert!(!flags["ticket:read"]);
    // The person ticks only the read scopes: the grant has no write, and the tool is refused
    // with a challenge that keeps what the connection has and adds what it lacks.
    let decision = api(
        &env.app,
        "POST",
        &format!("/v1/oauth/requests/{id}/decision"),
        Some(&env.owner_cookie),
        Some(json!({"approve": true, "scopes": ["workspace:read", "ticket:read"]})),
    )
    .await;
    let back = query_of(decision.body["redirectTo"].as_str().unwrap());
    let token = exchange(&env, &back["code"]).await.body["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(tool_names(&modern_list(&env.app, &token).await).contains(&"ticket_create".into()));
    let denied = modern_call(
        &env.app,
        &token,
        "ticket_create",
        json!({"projectId": Uuid::new_v4(), "title": "x"}),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let upgrade = denied.header("www-authenticate").unwrap();
    assert!(upgrade.contains("insufficient_scope"), "{upgrade}");
    assert!(
        upgrade.contains("scope=\"ticket:write workspace:read ticket:read\""),
        "{upgrade}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn activity_and_revisions_show_which_client_made_a_change_and_audit_covers_high_risk(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let project = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": "Attribution", "teamIds": []})),
    )
    .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let token = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let created = modern_call(
        &env.app,
        &token,
        "ticket_create",
        json!({"projectId": project_id, "title": "By tool"}),
    )
    .await;
    let ticket_id = structured(&created)["id"].as_str().unwrap().to_string();
    let rev1 = structured(&created)["revisionNo"].as_i64().unwrap();
    // A web edit in between is the person's own and carries no marker.
    let web_edit = rest(
        &env,
        "PATCH",
        &format!("/v1/tickets/{ticket_id}"),
        &env.owner_cookie,
        Some("web-1"),
        Some(json!({"expectedRevision": rev1, "title": "By web"})),
    )
    .await;
    assert!(web_edit.status.is_success(), "{:?}", web_edit.body);
    let rev2 = web_edit.body["revisionNo"].as_i64().unwrap();
    let updated = modern_call(
        &env.app,
        &token,
        "ticket_update",
        json!({"ticketId": ticket_id, "expectedRevision": rev2, "priority": "HIGH"}),
    )
    .await;
    let rev3 = structured(&updated)["revisionNo"].as_i64().unwrap();
    let commented = modern_call(
        &env.app,
        &token,
        "ticket_comment",
        json!({"ticketId": ticket_id, "body": "via tool"}),
    )
    .await;
    let comment_id = structured(&commented)["id"].as_str().unwrap().to_string();
    let web_comment = rest(
        &env,
        "POST",
        &format!("/v1/tickets/{ticket_id}/comments"),
        &env.owner_cookie,
        None,
        Some(json!({"body": common::doc("by hand")})),
    )
    .await;
    assert!(web_comment.status.is_success(), "{:?}", web_comment.body);
    // A stale write changes nothing and claims nothing.
    let stale = modern_call(
        &env.app,
        &token,
        "ticket_update",
        json!({"ticketId": ticket_id, "expectedRevision": rev1, "title": "stale"}),
    )
    .await;
    assert_eq!(error_code(&stale), "version_conflict");

    let activity = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket_id}/activity"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let items = activity.body["items"].as_array().unwrap();
    let via = |kind: &str, revision: Option<i64>, comment: Option<&str>| -> Option<String> {
        items
            .iter()
            .find(|item| {
                item["activityKind"] == kind
                    && (revision.is_none() || item["revisionNo"].as_i64() == revision)
                    && (comment.is_none() || item["data"]["commentId"].as_str() == comment)
            })
            .unwrap_or_else(|| panic!("no activity {kind} {revision:?}: {items:?}"))["via"]
            .as_str()
            .map(str::to_string)
    };
    let logged: Vec<(String, Option<String>, Option<i64>, String)> = sqlx::query_as(
        "SELECT operation, resource_id, resource_revision, outcome FROM penfii.mcp_call_log",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        via("ticket.created", Some(rev1), None).as_deref(),
        Some("Test Client"),
        "{logged:?} {items:?}"
    );
    let kind_of = |rev: i64| {
        items
            .iter()
            .find(|item| item["revisionNo"].as_i64() == Some(rev))
            .unwrap()["activityKind"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        via(&kind_of(rev2), Some(rev2), None),
        None,
        "the web edit is not marked"
    );
    assert_eq!(
        via(&kind_of(rev3), Some(rev3), None).as_deref(),
        Some("Test Client")
    );
    assert_eq!(
        via("ticket.commented", None, Some(&comment_id)).as_deref(),
        Some("Test Client")
    );
    let by_hand = items
        .iter()
        .filter(|item| {
            item["activityKind"] == "ticket.commented"
                && item["data"]["commentId"] != comment_id.as_str()
        })
        .count();
    assert_eq!(by_hand, 1);
    assert!(
        items
            .iter()
            .filter(|item| item["activityKind"] == "ticket.commented"
                && item["data"]["commentId"] != comment_id.as_str())
            .all(|item| item["via"].is_null())
    );
    // The same marker on the revision list and the project feed.
    let revisions = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket_id}/revisions"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let marked: Vec<(i64, bool)> = revisions.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["revisionNo"].as_i64().unwrap(), r["via"] == "Test Client"))
        .collect();
    assert!(marked.contains(&(rev1, true)));
    assert!(marked.contains(&(rev2, false)));
    assert!(marked.contains(&(rev3, true)));
    let feed = rest(
        &env,
        "GET",
        &format!("/v1/projects/{project_id}/activity"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let feed_items = feed
        .body
        .as_array()
        .or(feed.body["items"].as_array())
        .unwrap();
    assert!(
        feed_items.iter().any(|item| item["via"] == "Test Client"),
        "{feed_items:?}"
    );

    // Audit: grants are recorded, and administrative calls are recorded whether they run or not.
    let manage = token_for(&env, &env.owner_cookie, "workspace:read project:manage").await;
    // Giving someone a Project role is Administrative: off under the standard policy.
    add_member(&env, "grantee@penfii.test", "MEMBER").await;
    let grantee: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = 'grantee@penfii.test'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let args = json!({"projectId": project_id, "userId": grantee, "role": "VIEWER"});
    assert_eq!(
        error_code(&modern_call(&env.app, &manage, "project_grant_set", args.clone()).await),
        "policy_denied"
    );
    let allow = json!({"mcpEnabled": true, "allowWrite": true, "allowDestructive": false, "allowAdministrative": true});
    let put = rest(
        &env,
        "PUT",
        &format!("/v1/workspaces/{}/mcp-policy", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(allow),
    )
    .await;
    assert!(put.status.is_success(), "{:?}", put.body);
    let ok = modern_call(&env.app, &manage, "project_grant_set", args).await;
    assert_eq!(ok.body["result"]["isError"], false, "{:?}", ok.body);
    let audits: Vec<(String, String)> = sqlx::query_as(
        "SELECT action_code, outcome_code FROM penfii.audit_entries
         WHERE action_code LIKE 'mcp.%' OR action_code LIKE 'oauth.%' ORDER BY occurred_at",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert!(
        audits.contains(&("mcp.project_grant_set".into(), "POLICY_DENIED".into())),
        "{audits:?}"
    );
    assert!(
        audits.contains(&("mcp.project_grant_set".into(), "OK".into())),
        "{audits:?}"
    );
    assert!(
        audits.iter().any(|(code, _)| code == "oauth.grant_created"),
        "{audits:?}"
    );
    let ticket_calls = audits
        .iter()
        .filter(|(code, _)| code == "mcp.ticket_create")
        .count();
    assert_eq!(
        ticket_calls, 0,
        "plain writes are not Audit rows (they are Activity)"
    );
    let payload: String = sqlx::query_scalar(
        "SELECT string_agg(data::text, ' ') FROM penfii.audit_entries WHERE action_code LIKE 'mcp.%'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(!payload.contains("Audited"), "no arguments in Audit data");
    let grants = api(
        &env.app,
        "GET",
        "/v1/me/oauth-grants",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let grant = grants.body["items"][0]["id"].as_str().unwrap().to_string();
    let revoked = api(
        &env.app,
        "DELETE",
        &format!("/v1/me/oauth-grants/{grant}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    let revoked_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'oauth.grant_revoked'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(revoked_rows, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn retention_purges_what_the_oauth_server_and_gateway_accumulate(pool: PgPool) {
    use penfii_core::oauth::retention::{Retention, purge};
    let env = setup(pool).await;
    let token = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    assert_eq!(
        modern_call(&env.app, &token, "workspace_list", json!({}))
            .await
            .body["result"]["isError"],
        false
    );
    let (client_pk, grant_id): (Uuid, Uuid) =
        sqlx::query_as("SELECT client_pk, id FROM penfii.oauth_grants LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    // Seed old rows: read log 31 days, write log 31 and 366 days, a fresh read row.
    for (risk, age) in [("read", 31), ("write", 31), ("write", 366), ("read", 1)] {
        sqlx::query(
            "INSERT INTO penfii.mcp_call_log
                (id, at, user_id, grant_id, client_pk, tool, operation, risk_class, request_id, outcome)
             VALUES ($1, now() - make_interval(days => $2), $3, $4, $5, 't', 'o', $6, 'r', 'ok')",
        )
        .bind(Uuid::new_v4())
        .bind(age)
        .bind(env.owner_id)
        .bind(grant_id)
        .bind(client_pk)
        .bind(risk)
        .execute(&env.pool)
        .await
        .unwrap();
    }
    // An old revoked grant with an expired token, an expired code, a stale request, a stale DCR.
    let old_grant = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.oauth_grants (id, user_id, client_pk, scope, created_at, revoked_at, revoked_reason)
         VALUES ($1, $2, $3, '{}', now() - interval '400 days', now() - interval '370 days', 'user')",
    )
    .bind(old_grant)
    .bind(env.owner_id)
    .bind(client_pk)
    .execute(&env.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.oauth_tokens (id, grant_id, family_id, kind, token_hash, resource, expires_at)
         VALUES ($1, $2, $1, 'access', $3, 'r', now() - interval '369 days')",
    )
    .bind(Uuid::new_v4())
    .bind(old_grant)
    .bind(vec![7u8; 32])
    .execute(&env.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.oauth_codes (code_hash, grant_id, client_pk, redirect_uri, code_challenge, resource, expires_at)
         VALUES ($1, $2, $3, 'x', 'c', 'r', now() - interval '3 days')",
    )
    .bind(vec![8u8; 32])
    .bind(old_grant)
    .bind(client_pk)
    .execute(&env.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.oauth_authorization_requests (id, client_pk, redirect_uri, code_challenge, resource, expires_at)
         VALUES ($1, $2, 'x', 'c', 'r', now() - interval '3 days')",
    )
    .bind(Uuid::new_v4())
    .bind(client_pk)
    .execute(&env.pool)
    .await
    .unwrap();
    let stale_client = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.oauth_clients (id, client_id, kind, client_name, redirect_uris, fetched_at)
         VALUES ($1, 'pfi_stale', 'dcr', 'Stale', '[]', now() - interval '8 days')",
    )
    .bind(stale_client)
    .execute(&env.pool)
    .await
    .unwrap();

    let report = purge(&env.pool, Retention::from_env().unwrap())
        .await
        .unwrap();
    assert_eq!(report.call_log, 2, "{report:?}");
    assert_eq!(report.codes, 1, "{report:?}");
    assert_eq!(report.authorization_requests, 1, "{report:?}");
    assert_eq!(report.tokens, 1, "{report:?}");
    assert_eq!(report.grants, 1, "{report:?}");
    assert_eq!(report.clients, 1, "{report:?}");
    let left: Vec<(String, i64)> = sqlx::query_as(
        "SELECT risk_class, count(*) FROM penfii.mcp_call_log GROUP BY 1 ORDER BY 1",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    // The write row at 31 days stays (365-day class); the live call and the fresh read stay.
    assert_eq!(left, vec![("read".into(), 2), ("write".into(), 1)]);
    // Nothing live was touched: the token still works and a second pass finds nothing.
    assert_eq!(
        modern_call(&env.app, &token, "workspace_list", json!({}))
            .await
            .body["result"]["isError"],
        false
    );
    assert_eq!(
        purge(&env.pool, Retention::from_env().unwrap())
            .await
            .unwrap()
            .total(),
        0
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_agent_can_finish_ticket_work_with_only_what_the_tools_tell_it(pool: PgPool) {
    let env = setup(pool).await;
    // Nothing is taken from the web API: every id below comes from an earlier tool result.
    let project = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": "Agent flow", "teamIds": []})),
    )
    .await;
    assert!(project.status.is_success(), "{:?}", project.body);
    let token = token_for(&env, &env.owner_cookie, ALL_SCOPE_TEXT).await;
    let listed = modern_list(&env.app, &token).await;
    let tools = listed.body["result"]["tools"].as_array().unwrap();
    let priority = tools.iter().find(|t| t["name"] == "ticket_create").unwrap()["inputSchema"]
        ["properties"]["priority"]["enum"]
        .clone();
    assert_eq!(priority, json!(["LOW", "MEDIUM", "HIGH", "URGENT"]));
    let instructions = modern_call(&env.app, &token, "workspace_list", json!({})).await;
    let workspace = structured(&instructions)["workspaces"][0]["id"].clone();
    let overview = modern_call(
        &env.app,
        &token,
        "workspace_overview",
        json!({"workspaceId": workspace}),
    )
    .await;
    let found = structured(&overview)["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Agent flow")
        .unwrap()
        .clone();
    let workflow = modern_call(
        &env.app,
        &token,
        "project_workflow",
        json!({"projectId": found["id"]}),
    )
    .await;
    let states = structured(&workflow)["states"].as_array().unwrap().clone();
    let by_category =
        |category: &str| states.iter().find(|s| s["category"] == category).unwrap()["id"].clone();
    let created = modern_call(
        &env.app,
        &token,
        "ticket_create",
        json!({"projectId": found["id"], "title": "Agent ticket", "priority": "MEDIUM",
               "statusId": by_category("NOT_STARTED")}),
    )
    .await;
    assert_eq!(
        created.body["result"]["isError"], false,
        "{:?}",
        created.body
    );
    // The Ticket is findable by its display key, which is how people refer to it.
    let key = structured(&created)["key"].as_str().unwrap().to_string();
    let mut hit = None;
    for _ in 0..1 {
        let search = modern_call(
            &env.app,
            &token,
            "search",
            json!({"workspaceId": workspace, "query": key, "kind": "ticket"}),
        )
        .await;
        hit = structured(&search)["results"]
            .as_array()
            .and_then(|r| r.first().cloned());
        if hit.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    // Search needs the OpenSearch, which this harness does not run (the live run below searches by key); the flow does not depend on it.
    let ticket_id = hit
        .map(|h| h["id"].clone())
        .unwrap_or_else(|| structured(&created)["id"].clone());
    let got = modern_call(
        &env.app,
        &token,
        "ticket_get",
        json!({"ticketId": ticket_id}),
    )
    .await;
    let revision = structured(&got)["revisionNo"].clone();
    let moved = modern_call(
        &env.app,
        &token,
        "ticket_change_status",
        json!({"ticketId": ticket_id, "expectedRevision": revision, "statusId": by_category("IN_PROGRESS")}),
    )
    .await;
    assert_eq!(moved.body["result"]["isError"], false, "{:?}", moved.body);
    assert_eq!(structured(&moved)["statusId"], by_category("IN_PROGRESS"));
    // An unknown state id is refused with a message, not a crash.
    let bad = modern_call(
        &env.app,
        &token,
        "ticket_change_status",
        json!({"ticketId": ticket_id, "expectedRevision": structured(&moved)["revisionNo"],
               "statusId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(bad.body["result"]["isError"], true);
    // A priority outside the list is refused by the same rule the web API has.
    let wrong = modern_call(
        &env.app,
        &token,
        "ticket_update",
        json!({"ticketId": ticket_id, "expectedRevision": structured(&moved)["revisionNo"], "priority": "NOW"}),
    )
    .await;
    assert_eq!(wrong.body["result"]["isError"], true);
    // A read-only connection discovers the write tools too (calling one is refused), and has
    // project_workflow.
    let reader = token_for(&env, &env.owner_cookie, "workspace:read ticket:read").await;
    let names = tool_names(&modern_list(&env.app, &reader).await);
    assert!(names.contains(&"project_workflow".to_string()));
    assert!(names.contains(&"ticket_change_status".to_string()));
}

// ---- FEA-139: what ends which access --------------------------------------------------------------

/// A second device of the owner: its cookie.
pub(crate) async fn second_device(env: &Env) -> String {
    login(&env.app, OWNER_EMAIL, OWNER_PASSWORD).await
}

pub(crate) async fn web_alive(env: &Env, cookie: &str) -> bool {
    api(&env.app, "GET", "/v1/me", Some(cookie), None)
        .await
        .status
        == StatusCode::OK
}

pub(crate) async fn mcp_alive(env: &Env, token: &str) -> bool {
    modern_list(&env.app, token).await.status == StatusCode::OK
}

pub(crate) async fn live_grants(env: &Env) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM penfii.oauth_grants WHERE revoked_at IS NULL")
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_changed_profile_password_keeps_this_device_and_the_apps_and_ends_the_other_devices(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let (token, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    let other = second_device(&env).await;
    let changed = api(
        &env.app,
        "PATCH",
        "/v1/me",
        Some(&env.owner_cookie),
        Some(json!({
            "displayName": "Owner",
            "email": OWNER_EMAIL,
            "newPassword": "a brand new passphrase"
        })),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{:?}", changed.body);
    assert!(
        web_alive(&env, &env.owner_cookie).await,
        "this device stays"
    );
    assert!(!web_alive(&env, &other).await, "the other device ends");
    assert!(mcp_alive(&env, &token).await, "connected apps stay");
    assert_eq!(live_grants(&env).await, 1);
    // A profile change that does not touch the password ends nothing.
    let third = login(&env.app, OWNER_EMAIL, "a brand new passphrase").await;
    let renamed = api(
        &env.app,
        "PATCH",
        "/v1/me",
        Some(&env.owner_cookie),
        Some(json!({"displayName": "Owner 2", "email": OWNER_EMAIL})),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert!(web_alive(&env, &third).await);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn signing_out_one_device_or_the_others_leaves_the_apps_and_disconnecting_apps_leaves_the_devices(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let (token, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    let (b, c) = (second_device(&env).await, second_device(&env).await);
    // One chosen device.
    let sessions = api(
        &env.app,
        "GET",
        "/v1/me/sessions",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    let b_id = sessions.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["current"] == false)
        .map(|item| item["id"].as_str().unwrap().to_string())
        .unwrap();
    let ended = api(
        &env.app,
        "DELETE",
        &format!("/v1/me/sessions/{b_id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(ended.status, StatusCode::NO_CONTENT);
    assert_eq!(
        [web_alive(&env, &b).await, web_alive(&env, &c).await],
        [true, false],
        "only the chosen device (the newest other one) ends"
    );
    assert!(web_alive(&env, &env.owner_cookie).await);
    assert!(mcp_alive(&env, &token).await);
    // The other devices: this one stays.
    let others = api(
        &env.app,
        "DELETE",
        "/v1/me/sessions",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert!(others.status.is_success(), "{:?}", others.body);
    assert!(!web_alive(&env, &b).await);
    assert!(!web_alive(&env, &c).await);
    assert!(web_alive(&env, &env.owner_cookie).await);
    assert!(mcp_alive(&env, &token).await);
    // Disconnecting every app ends the apps and no device.
    let all = api(
        &env.app,
        "DELETE",
        "/v1/me/oauth-grants",
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(all.status, StatusCode::NO_CONTENT);
    assert!(!mcp_alive(&env, &token).await);
    assert!(web_alive(&env, &env.owner_cookie).await);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_password_reset_and_the_operators_password_end_every_device_and_every_app(pool: PgPool) {
    use crate::common::smtp::{FakeSmtp, Mode};
    let env = setup(pool).await;
    let smtp = FakeSmtp::start(Mode::Accept).await;
    let mailer = smtp.mailer(std::time::Duration::from_secs(5));

    // By link.
    let (token, _) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    let other = second_device(&env).await;
    identity::process_reset_request(&env.pool, &mailer, "owner@penfii.test").await;
    let mail = smtp.received().remove(0);
    let text = mail.text();
    let link = text
        .split_whitespace()
        .find(|w| w.contains("/reset-password"))
        .unwrap();
    let reset_token = link.split_once('#').unwrap().1;
    identity::confirm_password_reset(&env.pool, reset_token, "reset passphrase one two")
        .await
        .unwrap();
    assert!(!web_alive(&env, &env.owner_cookie).await);
    assert!(!web_alive(&env, &other).await);
    assert!(!mcp_alive(&env, &token).await);
    assert_eq!(live_grants(&env).await, 0);
    let reason: String =
        sqlx::query_scalar("SELECT revoked_reason FROM penfii.oauth_grants LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(reason, "password_reset");

    // By the operator.
    let cookie = login(&env.app, OWNER_EMAIL, "reset passphrase one two").await;
    let (token, _) = {
        let code = get_code(&env, &cookie, ALL_SCOPE_TEXT, ALL_SCOPES).await;
        let reply = exchange(&env, &code).await;
        (reply.body["access_token"].as_str().unwrap().to_string(), ())
    };
    assert!(mcp_alive(&env, &token).await);
    identity::admin_set_password(&env.pool, OWNER_EMAIL, "operator set passphrase")
        .await
        .unwrap();
    assert!(!web_alive(&env, &cookie).await);
    assert!(!mcp_alive(&env, &token).await);
    assert_eq!(live_grants(&env).await, 0);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn enabling_a_disabled_account_brings_back_no_session_token_or_grant(pool: PgPool) {
    let env = setup(pool).await;
    let (token, refresh) = token_pair(&env, ALL_SCOPE_TEXT, ALL_SCOPES).await;
    assert!(web_alive(&env, &env.owner_cookie).await && mcp_alive(&env, &token).await);
    // The operator's switch (what `admin-cli user-disable` / `user-enable` do).
    penfii_core::admin::operator_set_account_disabled(&env.pool, OWNER_EMAIL, true)
        .await
        .unwrap();
    assert!(!web_alive(&env, &env.owner_cookie).await);
    assert!(!mcp_alive(&env, &token).await);
    penfii_core::admin::operator_set_account_disabled(&env.pool, OWNER_EMAIL, false)
        .await
        .unwrap();
    // Nothing from before comes back.
    assert!(!web_alive(&env, &env.owner_cookie).await, "the old session");
    assert!(!mcp_alive(&env, &token).await, "the old access token");
    let refreshed = form(
        &env.app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", CLIENT),
        ],
    )
    .await;
    assert_ne!(refreshed.status, StatusCode::OK, "the old refresh token");
    assert_eq!(live_grants(&env).await, 0);
    // Signing in again works and gives a new session only.
    let again = login(&env.app, OWNER_EMAIL, OWNER_PASSWORD).await;
    assert!(web_alive(&env, &again).await);
    assert!(!mcp_alive(&env, &token).await);
}

/// FEA-143: a client whose document cannot be fetched now stays known from the stored copy, one address
/// cannot make the server fetch without bound, and a self-registered app is told apart on the consent page.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn client_lookups_have_a_budget_per_address_and_fall_back_to_the_stored_copy(pool: PgPool) {
    use penfii_core::oauth::client::{Lookup, resolve};
    let env = setup(pool).await;
    let config = OAuthConfig::new(PUBLIC, Arc::new(env.clients.clone())).unwrap();
    let mut small = penfii_core::ThrottleLimits::default();
    small.address.max_failures = 3;
    let from = |address: &'static str| Lookup {
        address: Some(address),
        throttle: &small,
    };
    let document = |url: &str| json!({"client_id": url, "client_name": "App", "redirect_uris": [CALLBACK], "token_endpoint_auth_method": "none"});
    let url = |n: u32| format!("https://app{n}.example/oauth/client.json");
    for n in 0..8 {
        env.clients
            .0
            .lock()
            .unwrap()
            .insert(url(n), document(&url(n)));
    }

    // Three lookups of unknown documents from one address are fetched; the fourth is refused, and the
    // other address is not affected.
    for n in 0..3 {
        resolve(&env.pool, &config, &url(n), from("203.0.113.7"))
            .await
            .unwrap();
    }
    assert!(matches!(
        resolve(&env.pool, &config, &url(3), from("203.0.113.7")).await,
        Err(CoreError::RateLimited { .. })
    ));
    resolve(&env.pool, &config, &url(3), from("203.0.113.8"))
        .await
        .unwrap();
    // A client already stored and fresh needs no fetch, so the used budget does not matter.
    resolve(&env.pool, &config, &url(0), from("203.0.113.7"))
        .await
        .unwrap();

    // The stored copy goes stale and the host is down (or the asker's budget is spent): the client is
    // still known from the stored copy, with its redirect URIs.
    sqlx::query("UPDATE penfii.oauth_clients SET expires_at = now() - interval '1 hour'")
        .execute(&env.pool)
        .await
        .unwrap();
    env.clients.0.lock().unwrap().remove(&url(1));
    let kept = resolve(&env.pool, &config, &url(1), from("203.0.113.9"))
        .await
        .unwrap();
    assert_eq!(kept.redirect_uris, vec![CALLBACK.to_string()]);
    let kept = resolve(&env.pool, &config, &url(2), from("203.0.113.7"))
        .await
        .unwrap();
    assert_eq!(kept.name, "App");
    // A client that was never known is not invented, and a disabled one stays refused.
    assert!(
        resolve(
            &env.pool,
            &config,
            &url(1).replace("app1", "never"),
            from("203.0.113.9")
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE penfii.oauth_clients SET disabled_at = now() WHERE client_id = $1")
        .bind(url(1))
        .execute(&env.pool)
        .await
        .unwrap();
    assert!(matches!(
        resolve(&env.pool, &config, &url(1), from("203.0.113.9")).await,
        Err(CoreError::NotFound)
    ));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_consent_page_says_where_a_client_comes_from_and_a_never_used_client_row_is_purged(
    pool: PgPool,
) {
    use penfii_core::oauth::retention::{Retention, purge};
    let env = setup(pool).await;
    let start = get_plain(
        &env.app,
        &authorize_path(
            CLIENT,
            CALLBACK,
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let view = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(view.body["clientKind"], "cimd", "{}", view.body);
    assert_eq!(view.body["clientHost"], "client.example");

    // A self-registered app is marked as such and has no host that vouches for it.
    let registered = send(
        &env.app,
        Request::builder()
            .method("POST")
            .uri("/oauth/register")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"client_name": "Self Made", "redirect_uris": [CALLBACK], "token_endpoint_auth_method": "none"}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(
        registered.status,
        StatusCode::CREATED,
        "{}",
        registered.body
    );
    let dcr = registered.body["client_id"].as_str().unwrap().to_string();
    let start = get_plain(
        &env.app,
        &authorize_path(
            &dcr,
            CALLBACK,
            "workspace:read",
            &challenge_of(VERIFIER),
            "s",
        ),
    )
    .await;
    let id = request_id(&start.location.unwrap());
    let view = api(
        &env.app,
        "GET",
        &format!("/v1/oauth/requests/{id}"),
        Some(&env.owner_cookie),
        None,
    )
    .await;
    assert_eq!(view.body["clientKind"], "dcr");
    assert!(view.body["clientHost"].is_null());

    // Document clients nobody ever connected are cleaned up after a week like unused registrations; one
    // that has a request, or a grant, stays.
    sqlx::query("UPDATE penfii.oauth_clients SET fetched_at = now() - interval '8 days'")
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.oauth_clients (id, client_id, kind, client_name, redirect_uris, fetched_at) VALUES ($1, 'https://idle.example/c.json', 'cimd', 'Idle', '[]', now() - interval '8 days')")
        .bind(Uuid::new_v4())
        .execute(&env.pool)
        .await
        .unwrap();
    purge(&env.pool, Retention::from_env().unwrap())
        .await
        .unwrap();
    let left: Vec<String> =
        sqlx::query_scalar("SELECT client_id FROM penfii.oauth_clients ORDER BY 1")
            .fetch_all(&env.pool)
            .await
            .unwrap();
    assert!(
        !left.contains(&"https://idle.example/c.json".to_string()),
        "{left:?}"
    );
    assert!(
        left.contains(&CLIENT.to_string()),
        "a client with an open request stays: {left:?}"
    );
}

/// Starting authorizations from one address has a limit of its own, larger than the sign-in limit of an
/// address, so a shared address does not run out of either because of the other.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn authorization_starts_have_their_own_larger_budget_per_address(pool: PgPool) {
    use penfii_core::oauth::client::Lookup;
    use penfii_core::oauth::server::{AuthorizeParams, AuthorizeResult, begin_authorization};
    let env = setup(pool).await;
    let config = OAuthConfig::new(PUBLIC, Arc::new(env.clients.clone())).unwrap();
    let mut small = penfii_core::ThrottleLimits::default();
    small.address.max_failures = 3;
    let lookup = Lookup {
        address: Some("203.0.113.70"),
        throttle: &small,
    };
    let start = || begin_authorization(&env.pool, &config, AuthorizeParams::default(), lookup);
    let refused = |result: &AuthorizeResult| matches!(result, AuthorizeResult::Page(text) if text.starts_with("Too many requests"));
    // Four times the sign-in limit of the address are answered (here: a page about the missing client).
    for n in 0..12 {
        let result = start().await.unwrap();
        assert!(!refused(&result), "start {n} was refused");
    }
    assert!(refused(&start().await.unwrap()));
    // The sign-in counter of that address was not touched.
    let login_ip: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.auth_throttle WHERE scope = 'login_ip'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(login_ip, 0);
}
