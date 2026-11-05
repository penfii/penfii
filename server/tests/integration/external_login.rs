//! External sign-in (Google / Microsoft / GitHub / generic OIDC) against a fake transport
//! that signs real RS256 ID tokens. No network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use futures_util::future::BoxFuture;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use penfii_core::CoreError;
use penfii_core::identity::provider::{
    AuthProviders, Endpoints, GithubProvider, IssuerPolicy, OidcConfig, OidcProvider, Transport,
};
use penfii_core::identity::{self, BootstrapInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Reply, call, limits};

/// A signing key generated in memory for each test run, so no private key is kept in the repository.
struct TestKey {
    pem: String,
    n: String,
}

fn test_key() -> &'static TestKey {
    use base64::Engine;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::traits::PublicKeyParts;
    static KEY: std::sync::OnceLock<TestKey> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let key = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
        TestKey {
            pem: key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
            n: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
        }
    })
}

#[derive(Default)]
struct FakeState {
    gets: HashMap<String, Value>,
    codes: HashMap<String, Value>,
    jwks_fetches: usize,
}

#[derive(Default, Clone)]
struct Fake(Arc<Mutex<FakeState>>);

impl Fake {
    fn serve_jwks(&self, url: &str, kid: &str) {
        self.0.lock().unwrap().gets.insert(
            url.into(),
            json!({"keys": [{"kty": "RSA", "kid": kid, "alg": "RS256", "use": "sig", "n": test_key().n, "e": "AQAB"}]}),
        );
    }
    fn serve(&self, url: &str, value: Value) {
        self.0.lock().unwrap().gets.insert(url.into(), value);
    }
    fn code(&self, code: &str, response: Value) {
        self.0.lock().unwrap().codes.insert(code.into(), response);
    }
    fn jwks_fetches(&self) -> usize {
        self.0.lock().unwrap().jwks_fetches
    }
}

impl Transport for Fake {
    fn get(&self, url: &str, _bearer: Option<&str>) -> BoxFuture<'_, Result<Value, CoreError>> {
        let mut state = self.0.lock().unwrap();
        if url.contains("keys") || url.contains("certs") || url.contains("jwks") {
            state.jwks_fetches += 1;
        }
        let found = state.gets.get(url).cloned();
        Box::pin(
            async move { found.ok_or_else(|| CoreError::conflict("provider_error", "no route")) },
        )
    }
    fn post_form(
        &self,
        _url: &str,
        form: Vec<(String, String)>,
    ) -> BoxFuture<'_, Result<Value, CoreError>> {
        let code = form
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.clone());
        let found = code.and_then(|code| self.0.lock().unwrap().codes.get(&code).cloned());
        Box::pin(
            async move { found.ok_or_else(|| CoreError::conflict("provider_error", "bad code")) },
        )
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn sign(kid: &str, claims: &Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.into());
    encode(
        &header,
        claims,
        &EncodingKey::from_rsa_pem(test_key().pem.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn claims(iss: &str, aud: &str, sub: &str, nonce: &str, extra: Value) -> Value {
    let mut base = json!({"iss": iss, "aud": aud, "sub": sub, "nonce": nonce,
        "exp": now() + 300, "iat": now(), "name": "Ext Person"});
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

struct Env {
    pool: PgPool,
    app: axum::Router,
    fake: Fake,
}

const GOOGLE_JWKS: &str = "https://www.googleapis.com/oauth2/v3/certs";
const GOOGLE_ISS: &str = "https://accounts.google.com";
const MS_TENANT: &str = "11111111-1111-1111-1111-111111111111";
const MS_JWKS: &str =
    "https://login.microsoftonline.com/11111111-1111-1111-1111-111111111111/discovery/v2.0/keys";
const GENERIC_ISS: &str = "https://idp.example.test";

async fn setup(pool: PgPool) -> Env {
    setup_with(pool, false).await
}

/// `google_signup` is the operator's PENFII_AUTH_GOOGLE_SIGNUP.
async fn setup_with(pool: PgPool, google_signup: bool) -> Env {
    common::setup(&pool).await;
    crate::common::migrate(&pool).await;

    let fake = Fake::default();
    fake.serve_jwks(GOOGLE_JWKS, "k1");
    fake.serve_jwks(MS_JWKS, "k1");
    fake.serve_jwks(
        "https://login.microsoftonline.com/organizations/discovery/v2.0/keys",
        "k1",
    );
    fake.serve_jwks("https://idp.example.test/keys", "k1");
    fake.serve(
        "https://idp.example.test/.well-known/openid-configuration",
        json!({"issuer": GENERIC_ISS, "authorization_endpoint": "https://idp.example.test/auth",
               "token_endpoint": "https://idp.example.test/token", "jwks_uri": "https://idp.example.test/keys"}),
    );
    let transport: Arc<dyn Transport> = Arc::new(fake.clone());
    let mut providers = AuthProviders::new(Some("https://penfii.test".into()));
    providers.add(Arc::new(
        OidcProvider::google(
            "g-client".into(),
            "S".into(),
            "secret".into(),
            None,
            transport.clone(),
        )
        .with_signup(google_signup),
    ));
    providers.add(Arc::new(OidcProvider::microsoft(
        "m-client".into(),
        "S".into(),
        "secret".into(),
        MS_TENANT.into(),
        None,
        transport.clone(),
    )));
    providers.add(Arc::new(GithubProvider::new(
        "gh-client".into(),
        "S".into(),
        "secret".into(),
        transport.clone(),
    )));
    // A generic provider that is neither Google nor Microsoft.
    providers.add(Arc::new(OidcProvider::new(
        OidcConfig {
            key: "oidc:corp".into(),
            kind: "oidc",
            display_name: "Corp".into(),
            issuer: IssuerPolicy::Fixed(GENERIC_ISS.into()),
            endpoints: None,
            client_id: "c-client".into(),
            secret_ref: "S".into(),
            client_secret: "secret".into(),
            scopes: "openid profile email".into(),
            subject_claim: "sub".into(),
            hosted_domain: None,
            email_proof_requires_pinned_tenant: false,
        },
        transport.clone(),
    )));
    let _ = Endpoints {
        authorization: String::new(),
        token: String::new(),
        jwks: String::new(),
    };
    identity::sync_providers(&pool, &providers).await.unwrap();
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        providers,
        ..penfii_http_api::RouterConfig::new(pool.clone(), limits(), false)
    });
    Env { pool, app, fake }
}

async fn password_login(env: &Env, email: &str, password: &str) -> String {
    let r = call(
        &env.app,
        "POST",
        "/v1/session/login",
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.cookie.unwrap()
}

async fn new_user(env: &Env, email: &str) -> String {
    identity::create_account(
        &env.pool,
        BootstrapInput {
            email: email.into(),
            password: "user-test-passphrase".into(),
            display_name: "U".into(),
            workspace_name: "UW".into(),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .unwrap();
    password_login(env, email, "user-test-passphrase").await
}

/// Starts an attempt and returns (state, nonce) from the provider URL.
async fn begin(
    env: &Env,
    provider: &str,
    intent: &str,
    cookie: Option<&str>,
    extra: Value,
) -> (String, String, String) {
    let mut body = json!({"intent": intent});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let r = call(
        &env.app,
        "POST",
        &format!("/v1/auth/{provider}/start"),
        cookie,
        Some(body),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let url = url::Url::parse(r.body["url"].as_str().unwrap()).unwrap();
    let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
        q.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(q.contains_key("code_challenge"));
    (
        q["state"].clone(),
        q.get("nonce").cloned().unwrap_or_default(),
        url.to_string(),
    )
}

async fn finish(env: &Env, provider: &str, code: &str, state: &str, cookie: Option<&str>) -> Reply {
    call(
        &env.app,
        "GET",
        &format!("/v1/auth/{provider}/callback?code={code}&state={state}"),
        cookie,
        None,
    )
    .await
}

fn error_of(r: &Reply) -> String {
    let location = r.location.clone().unwrap_or_default();
    url::Url::parse(&format!("https://x{location}"))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "auth_error")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

fn google_token(env: &Env, code: &str, nonce: &str, sub: &str, email: &str, verified: bool) {
    let token = sign(
        "k1",
        &claims(
            GOOGLE_ISS,
            "g-client",
            sub,
            nonce,
            json!({"email": email, "email_verified": verified}),
        ),
    );
    env.fake.code(code, json!({"id_token": token}));
}

async fn count(env: &Env, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&env.pool).await.unwrap()
}

fn google_named_token(env: &Env, code: &str, nonce: &str, sub: &str, email: &str, name: &str) {
    let token = sign(
        "k1",
        &claims(
            GOOGLE_ISS,
            "g-client",
            sub,
            nonce,
            json!({"email": email, "email_verified": true, "name": name}),
        ),
    );
    env.fake.code(code, json!({"id_token": token}));
}

/// With sign-up on, the first Google sign-in creates the user (no password, no workspace) and links the
/// identity; the next sign-in with the same identity is an ordinary login of that user.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn google_first_sign_in_creates_the_account_when_sign_up_is_on(pool: PgPool) {
    let env = setup_with(pool, true).await;
    let users = count(&env, "SELECT count(*) FROM penfii.users").await;

    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_named_token(
        &env,
        "n1",
        &nonce,
        "g-new",
        "New.Person@Penfii.test",
        "New Person",
    );
    let r = finish(&env, "google", "n1", &state, None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(error_of(&r), "");
    let cookie = r.cookie.expect("a session is issued");

    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.users").await,
        users + 1
    );
    let (email_key, name, verified): (String, String, bool) = sqlx::query_as(
        "SELECT email_key, display_name, email_verified_at IS NOT NULL FROM penfii.users
         WHERE email_key = 'new.person@penfii.test'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        (email_key.as_str(), name.as_str(), verified),
        ("new.person@penfii.test", "New Person", true)
    );
    // The account has no password and no workspace: the person creates one after signing in.
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.password_credentials p JOIN penfii.users u ON u.id = p.user_id WHERE u.email_key = 'new.person@penfii.test'").await,
        0
    );
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id WHERE u.email_key = 'new.person@penfii.test'").await,
        0
    );
    let me = call(&env.app, "GET", "/v1/me", Some(&cookie), None).await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.body["email"], "new.person@penfii.test");
    // Authentication is not membership: the instance already has a Workspace, so this person has none and
    // cannot create one; creation is closed for everyone, and no Workspace data is reachable.
    let creation = call(
        &env.app,
        "GET",
        "/v1/workspaces/creation",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(creation.body["open"], false);
    let listed = call(&env.app, "GET", "/v1/workspaces", Some(&cookie), None).await;
    assert_eq!(listed.body.as_array().unwrap().len(), 0);
    let workspace = call(
        &env.app,
        "POST",
        "/v1/workspaces",
        Some(&cookie),
        Some(json!({"name": "Mine", "timeZone": "Asia/Seoul"})),
    )
    .await;
    assert_eq!(
        workspace.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        workspace.body
    );
    assert_eq!(workspace.body["code"], "workspace_creation_closed");
    let ws: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces ORDER BY created_at LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    for path in ["space", "inbox", "favorites", "drafts"] {
        let r = call(
            &env.app,
            "GET",
            &format!("/v1/workspaces/{ws}/{path}"),
            Some(&cookie),
            None,
        )
        .await;
        assert!(
            r.status == StatusCode::NOT_FOUND || r.status == StatusCode::FORBIDDEN,
            "{path}: {} {}",
            r.status,
            r.body
        );
    }
    let invite = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(&cookie),
        Some(json!({"email": "x@penfii.test", "role": "MEMBER"})),
    )
    .await;
    assert!(!invite.status.is_success(), "{}", invite.body);
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.workspaces").await,
        1
    );

    // Sign out, then the same Google identity logs in again as the same user.
    let out = call(&env.app, "POST", "/v1/session/logout", Some(&cookie), None).await;
    assert!(out.status.is_success());
    let gone = call(&env.app, "GET", "/v1/me", Some(&cookie), None).await;
    assert_eq!(gone.status, StatusCode::UNAUTHORIZED);
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_named_token(
        &env,
        "n2",
        &nonce,
        "g-new",
        "new.person@penfii.test",
        "New Person",
    );
    let r = finish(&env, "google", "n2", &state, None).await;
    assert_eq!(error_of(&r), "");
    let again = call(&env.app, "GET", "/v1/me", r.cookie.as_deref(), None).await;
    assert_eq!(again.body["id"], me.body["id"]);
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.users").await,
        users + 1
    );
}

/// Sign-up creates accounts; it never links one. A Google identity with the e-mail of an existing account (any
/// letter case) is refused exactly like an unknown identity, and an e-mail the provider does not vouch for
/// creates nothing.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn google_sign_up_never_links_or_duplicates_an_existing_account(pool: PgPool) {
    let env = setup_with(pool, true).await;
    let before = (
        count(&env, "SELECT count(*) FROM penfii.users").await,
        count(&env, "SELECT count(*) FROM penfii.external_identities").await,
        count(&env, "SELECT count(*) FROM penfii.sessions").await,
    );

    for (code, sub, email) in [
        ("s1", "g-owner", "owner@penfii.test"),
        ("s2", "g-owner-2", "OWNER@Penfii.Test"),
    ] {
        let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
        google_token(&env, code, &nonce, sub, email, true);
        let r = finish(&env, "google", code, &state, None).await;
        assert_eq!(error_of(&r), "not_linked", "{email}");
        assert!(r.cookie.is_none());
    }
    // An unverified e-mail proves nothing, so even a brand-new address is not enough.
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_token(
        &env,
        "s3",
        &nonce,
        "g-unverified",
        "stranger@penfii.test",
        false,
    );
    let r = finish(&env, "google", "s3", &state, None).await;
    assert_eq!(error_of(&r), "not_linked");
    assert!(r.cookie.is_none());

    let after = (
        count(&env, "SELECT count(*) FROM penfii.users").await,
        count(&env, "SELECT count(*) FROM penfii.external_identities").await,
        count(&env, "SELECT count(*) FROM penfii.sessions").await,
    );
    assert_eq!(before, after);
    // The owner is untouched and can still link Google deliberately (signed in, recently authenticated).
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.external_identities i JOIN penfii.users u ON u.id = i.user_id WHERE u.email_key = 'owner@penfii.test'").await,
        0
    );
}

/// The first Google sign-in on an empty instance ends in an error redirect, never in a half-created account
/// or a session, whether the identity is unknown (nothing is linked yet) or the provider cannot be reached
/// (the token exchange fails, as it does when the server has no outbound network).
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn first_sign_in_without_account_or_provider_creates_nothing(pool: PgPool) {
    let env = setup(pool).await;
    let users = count(&env, "SELECT count(*) FROM penfii.users").await;
    let identities = count(&env, "SELECT count(*) FROM penfii.external_identities").await;
    let sessions = count(&env, "SELECT count(*) FROM penfii.sessions").await;

    // Provider unreachable: the code is not accepted, so there is no profile to map.
    let (state, _nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    let r = finish(&env, "google", "no-such-code", &state, None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(error_of(&r), "provider_error");
    assert!(r.cookie.is_none());

    // Provider answers, but no Penfii user is linked to the identity.
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_token(&env, "first", &nonce, "g-first", "first@penfii.test", true);
    let r = finish(&env, "google", "first", &state, None).await;
    assert_eq!(error_of(&r), "not_linked");
    assert!(r.cookie.is_none());

    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.users").await,
        users
    );
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.external_identities").await,
        identities
    );
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.sessions").await,
        sessions
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn external_sign_in_flows(pool: PgPool) {
    let env = setup(pool).await;

    // Offered providers, and the secret is never in the database.
    let r = call(&env.app, "GET", "/v1/auth/providers", None, None).await;
    let keys: Vec<_> = r.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(keys, ["github", "google", "microsoft", "oidc:corp"]);
    let cfg: String = sqlx::query_scalar(
        "SELECT string_agg(config::text || coalesce(secret_ref,''), '') FROM penfii.auth_providers",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(!cfg.contains("secret\""));

    let alice = new_user(&env, "alice@penfii.test").await;

    // Login with an identity nobody linked: no account, no session, nothing created.
    let users_before = count(&env, "SELECT count(*) FROM penfii.users").await;
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_token(&env, "c1", &nonce, "g-alice", "alice@penfii.test", true);
    let r = finish(&env, "google", "c1", &state, None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(error_of(&r), "not_linked");
    assert!(r.cookie.is_none());
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.users").await,
        users_before
    );
    assert!(!r.location.as_deref().unwrap().contains("c1"));

    // Same e-mail is never linked automatically; the attempt is single use.
    let r = finish(&env, "google", "c1", &state, None).await;
    assert_eq!(error_of(&r), "invalid_attempt");

    // Link: requires a session; a stale session needs reauth first.
    let r = call(
        &env.app,
        "POST",
        "/v1/auth/google/start",
        None,
        Some(json!({"intent": "link"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&env.pool)
        .await
        .unwrap();
    let r = call(
        &env.app,
        "POST",
        "/v1/auth/google/start",
        Some(&alice),
        Some(json!({"intent": "link"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["code"], "reauth_required");
    let r = call(
        &env.app,
        "POST",
        "/v1/me/reauth/password",
        Some(&alice),
        Some(json!({"password": "user-test-passphrase"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let (state, nonce, _) = begin(
        &env,
        "google",
        "link",
        Some(&alice),
        json!({"returnTo": "/account"}),
    )
    .await;
    google_token(&env, "c2", &nonce, "g-alice", "alice@penfii.test", true);
    // A callback without the session that started it fails.
    let r = finish(&env, "google", "c2", &state, None).await;
    assert_eq!(error_of(&r), "unauthenticated");
    let (state, nonce, _) = begin(
        &env,
        "google",
        "link",
        Some(&alice),
        json!({"returnTo": "/account"}),
    )
    .await;
    google_token(&env, "c2", &nonce, "g-alice", "alice@penfii.test", true);
    let r = finish(&env, "google", "c2", &state, Some(&alice)).await;
    assert_eq!(r.location.as_deref(), Some("/account?auth=linked"));

    // Another user cannot take that identity.
    let bob = new_user(&env, "bob@penfii.test").await;
    let (state, nonce, _) = begin(&env, "google", "link", Some(&bob), json!({})).await;
    google_token(&env, "c3", &nonce, "g-alice", "alice@penfii.test", true);
    let r = finish(&env, "google", "c3", &state, Some(&bob)).await;
    assert_eq!(error_of(&r), "identity_linked_elsewhere");
    // One identity per provider per user.
    let (state, nonce, _) = begin(&env, "google", "link", Some(&alice), json!({})).await;
    google_token(&env, "c4", &nonce, "g-alice-2", "a2@penfii.test", true);
    let r = finish(&env, "google", "c4", &state, Some(&alice)).await;
    assert_eq!(error_of(&r), "provider_already_linked");

    // Login now works; the presented cookie is revoked; the session records how it was made.
    let (state, nonce, _) = begin(
        &env,
        "google",
        "login",
        None,
        json!({"returnTo": "//evil.example"}),
    )
    .await;
    google_token(&env, "c5", &nonce, "g-alice", "alice@penfii.test", true);
    let r = finish(&env, "google", "c5", &state, Some(&bob)).await;
    assert_eq!(
        r.location.as_deref(),
        Some("/"),
        "open redirect must collapse to /"
    );
    let alice_google = r.cookie.clone().unwrap();
    let me = call(&env.app, "GET", "/v1/me", Some(&alice_google), None).await;
    assert_eq!(me.body["email"], "alice@penfii.test");
    assert_eq!(
        call(&env.app, "GET", "/v1/me", Some(&bob), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let method: String = sqlx::query_scalar(
        "SELECT auth_method FROM penfii.sessions ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(method, "google");
    let bob = password_login(&env, "bob@penfii.test", "user-test-passphrase").await;

    // Tampering: nonce, audience, expiry, unknown key, wrong provider's state, provider error.
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "nonce",
            claims(GOOGLE_ISS, "g-client", "g-alice", "other", json!({})),
            "id_token_invalid",
        ),
        (
            "aud",
            claims(GOOGLE_ISS, "someone-else", "g-alice", "N", json!({})),
            "id_token_invalid",
        ),
        (
            "iss",
            claims(
                "https://evil.example",
                "g-client",
                "g-alice",
                "N",
                json!({}),
            ),
            "id_token_invalid",
        ),
        (
            "exp",
            claims(
                GOOGLE_ISS,
                "g-client",
                "g-alice",
                "N",
                json!({"exp": now() - 600}),
            ),
            "id_token_invalid",
        ),
        (
            "azp",
            claims(
                GOOGLE_ISS,
                "g-client",
                "g-alice",
                "N",
                json!({"azp": "other"}),
            ),
            "id_token_invalid",
        ),
    ];
    for (i, (name, token_claims, expected)) in cases.into_iter().enumerate() {
        let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
        let mut token_claims = token_claims;
        if token_claims["nonce"] == "N" {
            token_claims["nonce"] = json!(nonce);
        }
        let code = format!("t{i}");
        env.fake
            .code(&code, json!({"id_token": sign("k1", &token_claims)}));
        let r = finish(&env, "google", &code, &state, None).await;
        assert_eq!(error_of(&r), expected, "case {name}");
        assert!(r.cookie.is_none(), "case {name}");
    }
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    env.fake.code("kid", json!({"id_token": sign("unknown", &claims(GOOGLE_ISS, "g-client", "g-alice", &nonce, json!({})))}));
    assert_eq!(
        error_of(&finish(&env, "google", "kid", &state, None).await),
        "id_token_invalid"
    );
    assert!(
        env.fake.jwks_fetches() >= 2,
        "an unknown kid refreshes the key set once"
    );
    // A state that belongs to another provider.
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_token(&env, "x1", &nonce, "g-alice", "alice@penfii.test", true);
    assert_eq!(
        error_of(&finish(&env, "microsoft", "x1", &state, None).await),
        "invalid_attempt"
    );
    // The provider said no.
    let (state, _, _) = begin(&env, "google", "login", None, json!({})).await;
    let r = call(
        &env.app,
        "GET",
        &format!("/v1/auth/google/callback?error=access_denied&state={state}"),
        None,
        None,
    )
    .await;
    assert_eq!(error_of(&r), "provider_denied");

    // Reauth with the provider: only the user's own identity counts.
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&env.pool)
        .await
        .unwrap();
    let r = call(
        &env.app,
        "PATCH",
        "/v1/me",
        Some(&alice_google),
        Some(json!({"displayName": "A", "email": "alice@penfii.test"})),
    )
    .await;
    assert_eq!(r.body["code"], "reauth_required");
    assert!(
        r.body["methods"]
            .as_array()
            .unwrap()
            .contains(&json!("provider:google"))
    );
    let (state, nonce, _) = begin_reauth(&env, "google", &alice_google).await;
    google_token(
        &env,
        "r1",
        &nonce,
        "g-someone-else",
        "alice@penfii.test",
        true,
    );
    assert_eq!(
        error_of(&finish(&env, "google", "r1", &state, Some(&alice_google)).await),
        "wrong_account"
    );
    let (state, nonce, url) = begin_reauth(&env, "google", &alice_google).await;
    assert!(url.contains("prompt=login") && url.contains("max_age=0"));
    google_token(&env, "r2", &nonce, "g-alice", "alice@penfii.test", true);
    let r = finish(&env, "google", "r2", &state, Some(&alice_google)).await;
    assert!(r.location.unwrap().contains("auth=reauthenticated"));
    let r = call(
        &env.app,
        "PATCH",
        "/v1/me",
        Some(&alice_google),
        Some(json!({"displayName": "A", "email": "alice@penfii.test"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    // Unlink needs recent auth and respects the last-method rule (password + google here).
    let r = call(
        &env.app,
        "GET",
        "/v1/me/identities",
        Some(&alice_google),
        None,
    )
    .await;
    assert_eq!(r.body["items"][0]["provider"], "google");
    assert_eq!(r.body["hasPassword"], true);

    // ---- Invitation through a provider ----
    let ws: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces ORDER BY created_at LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let owner = password_login(&env, "owner@penfii.test", "owner-test-passphrase").await;
    async fn invite(env: &Env, owner: &str, ws: Uuid, email: &str) -> String {
        let r = call(
            &env.app,
            "POST",
            &format!("/v1/workspaces/{ws}/invitations"),
            Some(owner),
            Some(json!({"email": email, "role": "MEMBER"})),
        )
        .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
        r.body["inviteToken"].as_str().unwrap().to_string()
    }
    // Unverified e-mail and a different e-mail are refused; nothing is created.
    let token = invite(&env, &owner, ws, "carol@penfii.test").await;
    let before = count(&env, "SELECT count(*) FROM penfii.users").await;
    for (code, email, verified) in [
        ("i1", "carol@penfii.test", false),
        ("i2", "mallory@penfii.test", true),
    ] {
        let (state, nonce, _) = begin(
            &env,
            "google",
            "invite",
            None,
            json!({"invitationToken": token}),
        )
        .await;
        google_token(&env, code, &nonce, "g-carol", email, verified);
        assert_eq!(
            error_of(&finish(&env, "google", code, &state, None).await),
            "email_mismatch"
        );
    }
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.users").await,
        before
    );
    // A bad invitation token cannot even start.
    let r = call(
        &env.app,
        "POST",
        "/v1/auth/google/start",
        None,
        Some(json!({"intent": "invite", "invitationToken": "nope"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // Verified and matching: account, identity, membership, one session, invitation consumed.
    let (state, nonce, _) = begin(
        &env,
        "google",
        "invite",
        None,
        json!({"invitationToken": token}),
    )
    .await;
    google_token(&env, "i3", &nonce, "g-carol", "Carol@Penfii.test", true);
    let r = finish(&env, "google", "i3", &state, None).await;
    assert!(r.location.as_deref() == Some("/"), "{:?}", r.location);
    let carol = r.cookie.unwrap();
    let me = call(&env.app, "GET", "/v1/me", Some(&carol), None).await;
    assert_eq!(me.body["email"], "carol@penfii.test");
    assert_eq!(count(&env, &format!("SELECT count(*) FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id WHERE m.workspace_id = '{ws}' AND u.email_key = 'carol@penfii.test'")).await, 1);
    assert_eq!(count(&env, "SELECT count(*) FROM penfii.password_credentials p JOIN penfii.users u ON u.id = p.user_id WHERE u.email_key = 'carol@penfii.test'").await, 0);
    // The token cannot be used again.
    let r = call(
        &env.app,
        "POST",
        "/v1/auth/google/start",
        None,
        Some(json!({"intent": "invite", "invitationToken": token})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // Carol has only Google: she cannot unlink it or remove a password she does not have.
    let r = call(
        &env.app,
        "DELETE",
        "/v1/me/identities/google",
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["code"], "last_method");
    // She can add a password (update_profile with a new password), then unlink google, then not remove the password.
    let r = call(&env.app, "PATCH", "/v1/me", Some(&carol), Some(json!({"displayName": "Carol", "email": "carol@penfii.test", "newPassword": "carol-test-passphrase"}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let r = call(
        &env.app,
        "DELETE",
        "/v1/me/identities/google",
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(&env.app, "DELETE", "/v1/me/password", Some(&carol), None).await;
    assert_eq!(r.body["code"], "last_method");

    // An existing account cannot be attached by an external invitation sign-in.
    let token = invite(&env, &owner, ws, "bob@penfii.test").await;
    let (state, nonce, _) = begin(
        &env,
        "google",
        "invite",
        None,
        json!({"invitationToken": token}),
    )
    .await;
    google_token(&env, "i4", &nonce, "g-bob", "bob@penfii.test", true);
    assert_eq!(
        error_of(&finish(&env, "google", "i4", &state, None).await),
        "account_exists_sign_in"
    );
    // Bob accepts as himself: his session, no password in the request, the token alone is not enough.
    let r = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": token, "displayName": "Bob"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        Some(&alice_google),
        Some(json!({"token": token, "displayName": "Bob"})),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::CONFLICT,
        "another person's session does not count"
    );
    let r = call(
        &env.app,
        "POST",
        "/v1/me/reauth/password",
        Some(&bob),
        Some(json!({"password": "user-test-passphrase"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        Some(&bob),
        Some(json!({"token": token, "displayName": "Bob"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);

    // Concurrent unlink of the last two methods: exactly one succeeds.
    let dave = new_user(&env, "dave@penfii.test").await;
    let (state, nonce, _) = begin(&env, "google", "link", Some(&dave), json!({})).await;
    google_token(&env, "d1", &nonce, "g-dave", "dave@penfii.test", true);
    finish(&env, "google", "d1", &state, Some(&dave)).await;
    let session =
        identity::authenticate_session(&env.pool, dave.trim_start_matches("penfii_session="))
            .await
            .unwrap();
    let (a, b) = tokio::join!(
        identity::unlink_identity(&env.pool, &session, "google"),
        identity::remove_password(&env.pool, &session)
    );
    assert!(
        a.is_ok() ^ b.is_ok(),
        "exactly one removal may succeed: {a:?} {b:?}"
    );
    assert_eq!(count(&env, &format!("SELECT (SELECT count(*) FROM penfii.password_credentials WHERE user_id = '{}') + (SELECT count(*) FROM penfii.external_identities WHERE user_id = '{}')", session.user.id, session.user.id)).await, 1);

    // A disabled user cannot sign in with a provider.
    sqlx::query(
        "UPDATE penfii.users SET disabled_at = now() WHERE email_key = 'alice@penfii.test'",
    )
    .execute(&env.pool)
    .await
    .unwrap();
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_token(&env, "z1", &nonce, "g-alice", "alice@penfii.test", true);
    assert_eq!(
        error_of(&finish(&env, "google", "z1", &state, None).await),
        "not_linked"
    );
    assert_eq!(
        call(&env.app, "GET", "/v1/me", Some(&alice_google), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // Admin recovery: the account's methods are visible, a new password ends every session and is audited.
    let found = identity::admin_identities(&env.pool, "Alice@penfii.test")
        .await
        .unwrap();
    assert!(found.disabled && found.has_password && found.identities.len() == 1);
    identity::admin_set_password(&env.pool, "alice@penfii.test", "recovered-test-passphrase")
        .await
        .unwrap();
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.sessions s JOIN penfii.users u ON u.id = s.user_id WHERE u.email_key = 'alice@penfii.test' AND s.revoked_at IS NULL").await,
        0
    );
    assert_eq!(
        count(
            &env,
            "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'user.password_set'"
        )
        .await,
        1
    );
    assert!(matches!(
        identity::admin_set_password(&env.pool, "nobody@penfii.test", "recovered-test-passphrase")
            .await,
        Err(CoreError::NotFound)
    ));
}

async fn begin_reauth(env: &Env, provider: &str, cookie: &str) -> (String, String, String) {
    let r = call(
        &env.app,
        "POST",
        &format!("/v1/me/reauth/{provider}/start"),
        Some(cookie),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let url = url::Url::parse(r.body["url"].as_str().unwrap()).unwrap();
    let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
    (
        q["state"].clone(),
        q.get("nonce").cloned().unwrap_or_default(),
        url.to_string(),
    )
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn microsoft_github_and_generic_oidc(pool: PgPool) {
    let env = setup(pool).await;
    let ws: Uuid = sqlx::query_scalar("SELECT id FROM penfii.workspaces LIMIT 1")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    let owner = password_login(&env, "owner@penfii.test", "owner-test-passphrase").await;
    let ms_iss = format!("https://login.microsoftonline.com/{MS_TENANT}/v2.0");

    // Microsoft, pinned tenant: the e-mail claim may accept an invitation; identity is (tenant issuer, sub).
    let r = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(&owner),
        Some(json!({"email": "erin@penfii.test", "role": "MEMBER"})),
    )
    .await;
    let token = r.body["inviteToken"].as_str().unwrap().to_string();
    let (state, nonce, _) = begin(
        &env,
        "microsoft",
        "invite",
        None,
        json!({"invitationToken": token}),
    )
    .await;
    let t = sign(
        "k1",
        &claims(
            &ms_iss,
            "m-client",
            "ms-erin",
            &nonce,
            json!({"email": "erin@penfii.test", "tid": MS_TENANT, "oid": "oid-1"}),
        ),
    );
    env.fake.code("m1", json!({"id_token": t}));
    let r = finish(&env, "microsoft", "m1", &state, None).await;
    assert!(r.cookie.is_some(), "{:?}", r.location);
    let (issuer, claims_json): (String, Value) = sqlx::query_as(
        "SELECT issuer, claims FROM penfii.external_identities WHERE subject = 'ms-erin'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(issuer, ms_iss);
    assert_eq!(claims_json["oid"], "oid-1");
    // Wrong tenant in the token fails the issuer rule.
    let (state, nonce, _) = begin(&env, "microsoft", "login", None, json!({})).await;
    let t = sign(
        "k1",
        &claims(
            "https://login.microsoftonline.com/22222222-2222-2222-2222-222222222222/v2.0",
            "m-client",
            "ms-erin",
            &nonce,
            json!({"tid": "22222222-2222-2222-2222-222222222222"}),
        ),
    );
    env.fake.code("m2", json!({"id_token": t}));
    assert_eq!(
        error_of(&finish(&env, "microsoft", "m2", &state, None).await),
        "id_token_invalid"
    );
    // Login by (issuer, sub) works.
    let (state, nonce, _) = begin(&env, "microsoft", "login", None, json!({})).await;
    let t = sign(
        "k1",
        &claims(
            &ms_iss,
            "m-client",
            "ms-erin",
            &nonce,
            json!({"tid": MS_TENANT}),
        ),
    );
    env.fake.code("m3", json!({"id_token": t}));
    assert!(
        finish(&env, "microsoft", "m3", &state, None)
            .await
            .cookie
            .is_some()
    );

    // GitHub: identity is the numeric id; e-mail comes from /user/emails (primary + verified).
    env.fake.serve(
        "https://api.github.com/user",
        json!({"id": 4242, "login": "gina", "name": "Gina"}),
    );
    env.fake.serve(
        "https://api.github.com/user/emails",
        json!([
        {"email": "x@other.test", "primary": false, "verified": true},
        {"email": "gina@penfii.test", "primary": true, "verified": true}]),
    );
    env.fake.code("gh1", json!({"access_token": "gho_x"}));
    let r = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(&owner),
        Some(json!({"email": "gina@penfii.test", "role": "MEMBER"})),
    )
    .await;
    let token = r.body["inviteToken"].as_str().unwrap().to_string();
    let (state, nonce, url) = begin(
        &env,
        "github",
        "invite",
        None,
        json!({"invitationToken": token}),
    )
    .await;
    assert!(nonce.is_empty() && url.contains("scope=user%3Aemail"));
    assert!(
        finish(&env, "github", "gh1", &state, None)
            .await
            .cookie
            .is_some()
    );
    let (issuer, subject): (String, String) = sqlx::query_as(
        "SELECT issuer, subject FROM penfii.external_identities WHERE subject = '4242'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        (issuer.as_str(), subject.as_str()),
        ("https://github.com", "4242")
    );
    // Plain sign-in asks for no scope and logs the same person in.
    let (state, _, url) = begin(&env, "github", "login", None, json!({})).await;
    assert!(!url.contains("scope="));
    assert!(
        finish(&env, "github", "gh1", &state, None)
            .await
            .cookie
            .is_some()
    );

    // Generic OIDC through discovery; a discovery document for another issuer is refused.
    let (state, nonce, url) = begin(&env, "oidc:corp", "login", None, json!({})).await;
    assert!(url.starts_with("https://idp.example.test/auth"));
    env.fake.code("o1", json!({"id_token": sign("k1", &claims(GENERIC_ISS, "c-client", "corp-1", &nonce, json!({})))}));
    assert_eq!(
        error_of(&finish(&env, "oidc:corp", "o1", &state, None).await),
        "not_linked"
    );

    // Multi-tenant Microsoft: allowed only with an allow-list, and the e-mail claim proves nothing.
    {
        let transport: Arc<dyn Transport> = Arc::new(env.fake.clone());
        let mut providers = AuthProviders::new(Some("https://penfii.test".into()));
        providers.add(Arc::new(OidcProvider::microsoft(
            "m-client".into(),
            "S".into(),
            "x".into(),
            "organizations".into(),
            Some(vec![MS_TENANT.into()]),
            transport,
        )));
        identity::sync_providers(&env.pool, &providers)
            .await
            .unwrap();
        let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
            providers,
            ..penfii_http_api::RouterConfig::new(env.pool.clone(), limits(), false)
        });
        let multi = Env {
            pool: env.pool.clone(),
            app,
            fake: env.fake.clone(),
        };
        let r = call(
            &env.app,
            "POST",
            &format!("/v1/workspaces/{ws}/invitations"),
            Some(&owner),
            Some(json!({"email": "fay@penfii.test", "role": "MEMBER"})),
        )
        .await;
        let token = r.body["inviteToken"].as_str().unwrap().to_string();
        let (state, nonce, _) = begin(
            &multi,
            "microsoft",
            "invite",
            None,
            json!({"invitationToken": token}),
        )
        .await;
        let t = sign(
            "k1",
            &claims(
                &ms_iss,
                "m-client",
                "ms-fay",
                &nonce,
                json!({"email": "fay@penfii.test", "tid": MS_TENANT}),
            ),
        );
        multi.fake.code("m4", json!({"id_token": t}));
        assert_eq!(
            error_of(&finish(&multi, "microsoft", "m4", &state, None).await),
            "email_mismatch"
        );
        // Re-sync the original set so later providers keep working.
    }
}

/// An empty instance has nobody who could invite: the first signed-in person creates the first Workspace
/// and becomes its OWNER; after that creation is closed, also for a second concurrent attempt.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn first_google_user_bootstraps_the_first_workspace(pool: PgPool) {
    let env = setup_with(pool, true).await;
    sqlx::query("TRUNCATE penfii.workspaces CASCADE")
        .execute(&env.pool)
        .await
        .unwrap();

    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_named_token(
        &env,
        "b1",
        &nonce,
        "g-first",
        "first.google@penfii.test",
        "First",
    );
    let r = finish(&env, "google", "b1", &state, None).await;
    assert_eq!(error_of(&r), "");
    let cookie = r.cookie.expect("a session is issued");
    assert_eq!(
        call(
            &env.app,
            "GET",
            "/v1/workspaces/creation",
            Some(&cookie),
            None
        )
        .await
        .body["open"],
        true
    );
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_named_token(
        &env,
        "b2",
        &nonce,
        "g-second",
        "second.google@penfii.test",
        "Second",
    );
    let other = finish(&env, "google", "b2", &state, None)
        .await
        .cookie
        .unwrap();
    let body = json!({"name": "Home", "timeZone": "Asia/Seoul"});
    let (a, b) = tokio::join!(
        call(
            &env.app,
            "POST",
            "/v1/workspaces",
            Some(&cookie),
            Some(body.clone())
        ),
        call(
            &env.app,
            "POST",
            "/v1/workspaces",
            Some(&other),
            Some(body.clone())
        ),
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::UNPROCESSABLE_ENTITY],
        "{} / {}",
        a.body,
        b.body
    );
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.workspaces").await,
        1
    );
    let created = if a.status == StatusCode::CREATED {
        &a
    } else {
        &b
    };
    assert_eq!(created.body["role"], "OWNER");
    assert_eq!(
        call(
            &env.app,
            "GET",
            "/v1/workspaces/creation",
            Some(&cookie),
            None
        )
        .await
        .body["open"],
        false
    );
}

/// An invitation works whether the person signed in with Google before or after it was sent: the account that
/// the first sign-in created accepts as itself (its own fresh session), and nobody else's session does.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn invitation_is_accepted_by_an_account_created_by_google_sign_in(pool: PgPool) {
    let env = setup_with(pool, true).await;
    let ws: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces ORDER BY created_at LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let owner = password_login(&env, "owner@penfii.test", "owner-test-passphrase").await;

    // Signs in first: user without membership.
    let (state, nonce, _) = begin(&env, "google", "login", None, json!({})).await;
    google_named_token(&env, "d1", &nonce, "g-dana", "dana@penfii.test", "Dana");
    let r = finish(&env, "google", "d1", &state, None).await;
    assert_eq!(error_of(&r), "");
    let dana = r.cookie.expect("session");
    assert_eq!(
        call(
            &env.app,
            "GET",
            &format!("/v1/workspaces/{ws}/space"),
            Some(&dana),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // The invitation arrives afterwards.
    let sent = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(&owner),
        Some(json!({"email": "dana@penfii.test", "role": "MEMBER"})),
    )
    .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    let token = sent.body["inviteToken"].as_str().unwrap().to_string();

    // The link tells its holder who it is for and that this person must sign in (no session needed).
    let preview = call(
        &env.app,
        "POST",
        "/v1/invitations/preview",
        None,
        Some(json!({"token": token})),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.body);
    assert_eq!(preview.body["email"], "dana@penfii.test");
    assert_eq!(preview.body["accountExists"], true);
    assert_eq!(preview.body["role"], "MEMBER");
    let nobody = call(
        &env.app,
        "POST",
        "/v1/invitations/preview",
        None,
        Some(json!({"token": "nope"})),
    )
    .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);
    let fresh = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(&owner),
        Some(json!({"email": "nobody-yet@penfii.test", "role": "MEMBER"})),
    )
    .await;
    let fresh_token = fresh.body["inviteToken"].as_str().unwrap().to_string();
    let preview = call(
        &env.app,
        "POST",
        "/v1/invitations/preview",
        None,
        Some(json!({"token": fresh_token})),
    )
    .await;
    assert_eq!(preview.body["accountExists"], false);

    // Someone else's session cannot accept it, and signing in alone gives no membership.
    let eve = new_user(&env, "eve@penfii.test").await;
    let wrong = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        Some(&eve),
        Some(json!({"token": token, "displayName": "Eve"})),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::CONFLICT, "{}", wrong.body);
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id WHERE u.email_key = 'dana@penfii.test'").await,
        0
    );

    // The `invite` intent cannot create a second account for the e-mail.
    let (state, nonce, _) = begin(
        &env,
        "google",
        "invite",
        None,
        json!({"invitationToken": token}),
    )
    .await;
    google_token(&env, "d2", &nonce, "g-dana-2", "dana@penfii.test", true);
    assert_eq!(
        error_of(&finish(&env, "google", "d2", &state, None).await),
        "account_exists_sign_in"
    );
    // The token alone, or someone else's session, is not enough.
    let bare = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": token, "displayName": "Dana"})),
    )
    .await;
    assert_eq!(bare.status, StatusCode::CONFLICT);
    // Her own session accepts.
    let accepted = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        Some(&dana),
        Some(json!({"token": token, "displayName": "Dana"})),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    let listed = call(&env.app, "GET", "/v1/workspaces", Some(&dana), None).await;
    assert_eq!(listed.body.as_array().unwrap().len(), 1);
    assert_eq!(listed.body[0]["role"], "MEMBER");
    assert_eq!(
        call(
            &env.app,
            "GET",
            &format!("/v1/workspaces/{ws}/space"),
            Some(&dana),
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        count(
            &env,
            "SELECT count(*) FROM penfii.users WHERE email_key = 'dana@penfii.test'"
        )
        .await,
        1
    );
}

/// Accepting an invitation through a provider applies the invitation's Team step and says how it went in
/// the redirect, for an active, an archived and a vanished Team.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_provider_invitation_with_a_team_reports_the_outcome_in_the_redirect(pool: PgPool) {
    let env = setup(pool).await;
    let ws: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces ORDER BY created_at LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let owner = password_login(&env, "owner@penfii.test", "owner-test-passphrase").await;
    let owner_id: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = 'owner@penfii.test'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let team = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/teams"),
        Some(&owner),
        Some(json!({"name": "Core & Co", "members": [{"userId": owner_id, "role": "MANAGER"}]})),
    )
    .await;
    assert_eq!(team.status, StatusCode::CREATED, "{}", team.body);
    let team_id = team.body["id"].as_str().unwrap().to_string();
    let invite = |email: &'static str| {
        let (env, owner, team_id) = (&env, owner.clone(), team_id.clone());
        async move {
            let r = call(
                &env.app,
                "POST",
                &format!("/v1/workspaces/{ws}/invitations"),
                Some(&owner),
                Some(json!({"email": email, "role": "MEMBER", "teamId": team_id})),
            )
            .await;
            assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
            r.body["inviteToken"].as_str().unwrap().to_string()
        }
    };
    let accept = |token: String, code: &'static str, sub: &'static str, email: &'static str| {
        let env = &env;
        async move {
            let (state, nonce, _) = begin(
                env,
                "google",
                "invite",
                None,
                json!({"invitationToken": token}),
            )
            .await;
            google_token(env, code, &nonce, sub, email, true);
            let r = finish(env, "google", code, &state, None).await;
            url::Url::parse(&format!("https://x{}", r.location.clone().unwrap()))
                .unwrap()
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect::<Vec<_>>()
        }
    };
    let outcome = |pairs: &[(String, String)], key: &str| {
        pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };

    // Active: joined, and the name travels intact.
    let token = invite("dora@penfii.test").await;
    let pairs = accept(token, "t1", "g-dora", "dora@penfii.test").await;
    assert_eq!(outcome(&pairs, "teamOutcome").as_deref(), Some("JOINED"));
    assert_eq!(outcome(&pairs, "teamName").as_deref(), Some("Core & Co"));
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.team_members tm JOIN penfii.users u ON u.id = tm.user_id WHERE u.email_key = 'dora@penfii.test'").await,
        1
    );

    // Archived after the invitation: Workspace only.
    let token = invite("evan@penfii.test").await;
    sqlx::query("UPDATE penfii.teams SET archived_at = now() WHERE id = $1")
        .bind(team_id.parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    let pairs = accept(token, "t2", "g-evan", "evan@penfii.test").await;
    assert_eq!(outcome(&pairs, "teamOutcome").as_deref(), Some("ARCHIVED"));
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.team_members tm JOIN penfii.users u ON u.id = tm.user_id WHERE u.email_key = 'evan@penfii.test'").await,
        0
    );
    assert_eq!(
        count(&env, "SELECT count(*) FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id WHERE u.email_key = 'evan@penfii.test'").await,
        1
    );

    // An invitation without a Team keeps the plain redirect.
    let r = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(&owner),
        Some(json!({"email": "fay@penfii.test", "role": "MEMBER"})),
    )
    .await;
    let plain = r.body["inviteToken"].as_str().unwrap().to_string();
    let pairs = accept(plain, "t3", "g-fay", "fay@penfii.test").await;
    assert!(pairs.is_empty(), "{pairs:?}");

    // The Team is gone: the name recorded with the invitation is still reported.
    let token = invite_gone(&env, &owner, ws).await;
    let pairs = accept(token, "t4", "g-gus", "gus@penfii.test").await;
    assert_eq!(outcome(&pairs, "teamOutcome").as_deref(), Some("GONE"));
    assert_eq!(outcome(&pairs, "teamName").as_deref(), Some("Vanished"));
}

/// An invitation whose Team row no longer exists, as `ON DELETE SET NULL` leaves it.
async fn invite_gone(env: &Env, owner: &str, ws: Uuid) -> String {
    let r = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        Some(owner),
        Some(json!({"email": "gus@penfii.test", "role": "MEMBER"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    sqlx::query("UPDATE penfii.workspace_invitations SET team_name = 'Vanished' WHERE email_key = 'gus@penfii.test'")
        .execute(&env.pool)
        .await
        .unwrap();
    r.body["inviteToken"].as_str().unwrap().to_string()
}
