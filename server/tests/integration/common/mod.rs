//! What every integration test shares: the guard against the development database, the owner
//! every test database starts with, and one way to call the router.
//!
//! Each test is a `#[sqlx::test(migrator = "crate::common::GUARD")]` function. sqlx creates a
//! database of its own for it on the server in `DATABASE_URL`, hands over a pool, and drops the
//! database when the test passes, so tests run side by side and nothing they write is seen by
//! another test.

pub mod smtp;

use std::borrow::Cow;
use std::sync::LazyLock;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use penfii_core::files::ObjectStore;
use penfii_core::identity::{self, BootstrapInput, BootstrapOutput};
use penfii_core::{Limits, apply_migrations};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use tokio::sync::Semaphore;
use tower::ServiceExt;
use uuid::Uuid;

/// Passed as `#[sqlx::test(migrator = ...)]`. sqlx reads it before it creates the test database,
/// which makes it the place to refuse the development database. It holds no migrations: [`setup`]
/// applies the real ones with [`apply_migrations`], so the history lands in
/// `penfii._sqlx_migrations` as in production.
pub static GUARD: LazyLock<Migrator> = LazyLock::new(|| {
    refuse_development_database();
    Migrator {
        migrations: Cow::Borrowed(&[]),
        locking: false,
        ..Migrator::DEFAULT
    }
});

/// The development database is PostgreSQL on port 5432 of this machine (`penfii-pg`). Tests
/// create and drop databases on the server they are given, so that server is refused.
fn refuse_development_database() {
    let url = std::env::var("DATABASE_URL").expect(
        "DATABASE_URL is not set: run `make integration`, or `make test` with a throwaway server",
    );
    let parsed = url::Url::parse(&url).expect("DATABASE_URL is a URL");
    let local = matches!(
        parsed.host_str(),
        None | Some("localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0" | "penfii-pg")
    );
    assert!(
        !(local && parsed.port().unwrap_or(5432) == 5432),
        "refusing DATABASE_URL on port 5432 of this machine: that is the development database"
    );
}

/// The URL of the test database behind `pool`, for code that opens connections of its own.
pub fn database_url(pool: &PgPool) -> String {
    let mut url = url::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(
        pool.connect_options()
            .get_database()
            .expect("test database"),
    );
    url.to_string()
}

pub const OWNER_EMAIL: &str = "owner@penfii.test";
pub const OWNER_PASSWORD: &str = "owner-test-passphrase";

/// A migration locks every table it creates until it commits, and PostgreSQL keeps those locks in one
/// table for the whole server (`max_locks_per_transaction` x `max_connections`, 6400 by default), so a
/// few dozen test databases migrating at once run out of it ("out of shared memory"). They take turns,
/// a few at a time.
pub static MIGRATING: Semaphore = Semaphore::const_new(4);

/// Applies the real migrations to a fresh test database.
pub async fn migrate(pool: &PgPool) {
    let _turn = MIGRATING
        .acquire()
        .await
        .expect("semaphore is never closed");
    apply_migrations(pool).await.expect("migrate");
}

/// Migrates the test database and makes the first account, which owns the first Workspace ("W").
pub async fn setup(pool: &PgPool) -> BootstrapOutput {
    migrate(pool).await;
    identity::bootstrap(
        pool,
        BootstrapInput {
            email: OWNER_EMAIL.into(),
            password: OWNER_PASSWORD.into(),
            display_name: "Owner".into(),
            workspace_name: "W".into(),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .expect("bootstrap")
}

/// A further Workspace owned by `owner`. The API creates only the first Workspace of an instance
/// (bootstrap), so further ones are written the way the operator paths write them.
pub async fn add_workspace(pool: &PgPool, owner: Uuid, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.workspaces (id, name, time_zone) VALUES ($1, $2, 'Asia/Seoul')",
    )
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, 'OWNER', 'ACTIVE')",
    )
    .bind(id)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    for table in ["workspace_auth_state", "workspace_graph_state"] {
        sqlx::query(&format!(
            "INSERT INTO penfii.{table} (workspace_id) VALUES ($1)"
        ))
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }
    id
}

pub fn limits() -> Limits {
    Limits {
        session_ttl_seconds: Some(3600),
        message_body_max_bytes: Some(65536),
        invite_ttl_seconds: Some(604_800),
        file_max_bytes: Some(10 * 1024 * 1024),
        message_file_max_count: Some(10),
        message_file_max_bytes: Some(50 * 1024 * 1024),
        login_throttle: Default::default(),
    }
}

/// A rich-text body with one paragraph.
pub fn doc(text: &str) -> Value {
    json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

pub struct Reply {
    pub status: StatusCode,
    /// `Null` for an empty body, `{"raw": text}` for one that is not JSON.
    pub body: Value,
    pub bytes: Vec<u8>,
    /// The `name=value` part of `set-cookie`.
    pub cookie: Option<String>,
    pub location: Option<String>,
    pub headers: HeaderMap,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn parts(self) -> (StatusCode, Value, Option<String>) {
        (self.status, self.body, self.cookie)
    }
}

pub async fn send(app: &axum::Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes)}))
    };
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    Reply {
        status,
        body,
        cookie: header("set-cookie").map(|value| value.split(';').next().unwrap().to_string()),
        location: header("location"),
        bytes,
        headers,
    }
}

/// A request as the web client makes it: the client header, the session cookie and a JSON body.
pub fn request(
    method: &str,
    path: &str,
    cookie: Option<&str>,
    idempotency_key: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-penfii-client", "1");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(key) = idempotency_key {
        builder = builder.header("idempotency-key", key);
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// [`request`] and [`send`]. Creating a message or a Ticket needs an `Idempotency-Key`; those
/// requests get a fresh one.
pub async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let key = (method == "POST" && (path.ends_with("/tickets") || path.contains("/messages")))
        .then(|| Uuid::new_v4().to_string());
    send(app, request(method, path, cookie, key.as_deref(), body)).await
}

/// Signs in and returns the session cookie.
pub async fn sign_in(app: &axum::Router, email: &str, password: &str) -> String {
    let reply = call(
        app,
        "POST",
        "/v1/session/login",
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.cookie.expect("session cookie")
}

/// [`call`] as the `(status, body, cookie)` triple most tests read.
pub async fn call_parts(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    call(app, method, path, cookie, body).await.parts()
}

/// One sign-in attempt: the status, and the session cookie when it worked.
pub async fn login(
    app: &axum::Router,
    email: &str,
    password: &str,
) -> (StatusCode, Option<String>) {
    let (status, _, cookie) = call_parts(
        app,
        "POST",
        "/v1/session/login",
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await;
    (status, cookie)
}

/// The password every [`Env::person`] gets.
pub const MEMBER_PASSWORD: &str = "member-test-passphrase";

/// Someone signed in: who they are and the session cookie that speaks for them.
#[derive(Clone)]
pub struct Person {
    pub id: Uuid,
    pub email: String,
    pub cookie: String,
}

/// A migrated database with the bootstrap owner signed in, and the router over it. A test file that
/// needs more (a Project helper, a different reply shape) wraps this in a type of its own and
/// dereferences to it; what is the same everywhere is here.
pub struct Env {
    pub pool: PgPool,
    pub app: axum::Router,
    /// The bootstrap Workspace.
    pub ws: Uuid,
    pub owner: Person,
}

impl Env {
    /// A router with no object storage (files and photos are refused).
    pub async fn new(pool: PgPool) -> Env {
        Env::build(pool, None).await
    }

    /// A router over `objects`.
    pub async fn with_objects(pool: PgPool, objects: ObjectStore) -> Env {
        Env::build(pool, Some(objects)).await
    }

    /// A router that sends e-mail through `mailer`.
    pub async fn with_mailer(pool: PgPool, mailer: penfii_core::mail::Mailer) -> Env {
        Env::build_with(pool, None, Some(std::sync::Arc::new(mailer))).await
    }

    async fn build(pool: PgPool, objects: Option<ObjectStore>) -> Env {
        Env::build_with(pool, objects, None).await
    }

    async fn build_with(
        pool: PgPool,
        objects: Option<ObjectStore>,
        mailer: Option<std::sync::Arc<penfii_core::mail::Mailer>>,
    ) -> Env {
        let boot = setup(&pool).await;
        let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
            objects,
            mailer,
            webhook_kek: Some(webhook_kek()),
            ..penfii_http_api::RouterConfig::new(pool.clone(), limits(), false)
        });
        let cookie = sign_in(&app, OWNER_EMAIL, OWNER_PASSWORD).await;
        Env {
            pool,
            app,
            ws: boot.workspace_id,
            owner: Person {
                id: boot.user_id,
                email: OWNER_EMAIL.into(),
                cookie,
            },
        }
    }

    pub async fn call(&self, who: &Person, method: &str, path: &str, body: Option<Value>) -> Reply {
        call(&self.app, method, path, Some(&who.cookie), body).await
    }

    /// `by` invites `email` to the Workspace with `role`.
    pub async fn invite(&self, by: &Person, email: &str, role: &str) -> Reply {
        self.call(
            by,
            "POST",
            &format!("/v1/workspaces/{}/invitations", self.ws),
            Some(json!({"email": email, "role": role})),
        )
        .await
    }

    /// The owner invites `<name>@penfii.test`, the invitation is accepted, and the person signs in.
    pub async fn person(&self, name: &str, role: &str) -> Person {
        let email = format!("{name}@penfii.test");
        let invitation = self.invite(&self.owner, &email, role).await;
        assert_eq!(
            invitation.status,
            StatusCode::CREATED,
            "{}",
            invitation.body
        );
        let accepted = call(
            &self.app,
            "POST",
            "/v1/invitations/accept",
            None,
            Some(json!({
                "token": invitation.body["inviteToken"],
                "displayName": name,
                "password": MEMBER_PASSWORD
            })),
        )
        .await;
        assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
        let cookie = sign_in(&self.app, &email, MEMBER_PASSWORD).await;
        Person {
            id: accepted.body["user"]["id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            email,
            cookie,
        }
    }

    /// A person who owns a second, unrelated Workspace called "Other" and is in no other.
    pub async fn outsider(&self) -> Person {
        let person = self.person("outsider", "MEMBER").await;
        sqlx::query("DELETE FROM penfii.workspace_members WHERE user_id = $1")
            .bind(person.id)
            .execute(&self.pool)
            .await
            .unwrap();
        add_workspace(&self.pool, person.id, "Other").await;
        person
    }
}

/// Two database role names no other test uses (roles belong to the server, not to a test database).
pub fn role_names() -> (String, String) {
    let id = Uuid::new_v4().simple().to_string();
    (format!("own_{}", &id[..12]), format!("app_{}", &id[..12]))
}

/// The URL of the test database behind `pool`, signed in as `role`.
pub fn url_as(pool: &PgPool, role: &str, password: &str) -> String {
    let mut url = url::Url::parse(&database_url(pool)).unwrap();
    url.set_username(role).unwrap();
    url.set_password(Some(password)).unwrap();
    url.to_string()
}

/// Provisions an owner role and a runtime role in the database behind `pool` (as the privileged test
/// connection does in `admin-cli db-provision`). Returns the plan that was used.
pub async fn provision_roles(
    pool: &PgPool,
    owner: &str,
    runtime: &str,
) -> penfii_core::provision::Provisioned {
    use sqlx::Connection;
    let mut conn = sqlx::PgConnection::connect(&database_url(pool))
        .await
        .expect("privileged connection");
    penfii_core::provision::provision(
        &mut conn,
        &penfii_core::provision::Provision {
            owner_role: owner.to_string(),
            owner_password: Some("owner-pass".into()),
            runtime_role: runtime.to_string(),
            runtime_password: Some("runtime-pass".into()),
        },
    )
    .await
    .expect("provision")
}

/// The key the test routers encrypt Signature-webhook secrets with (the server reads `PENFII_WEBHOOK_KEK` once, at start).
pub const WEBHOOK_KEK_HEX: &str =
    "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

pub fn webhook_kek() -> [u8; 32] {
    penfii_core::config::webhook_kek(&|name| {
        (name == "PENFII_WEBHOOK_KEK").then(|| WEBHOOK_KEK_HEX.to_owned())
    })
    .unwrap()
    .unwrap()
}
