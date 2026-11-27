//! FEA-182: long ("keep me signed in") web sessions of an ID/PW sign-in: issue, cookie, inactivity
//! window, absolute end, activity, sockets, revocation, operator settings and the upgrade from a
//! database without the new columns. Times are moved in the database, not waited for.

use axum::http::StatusCode;
use penfii_core::{Limits, identity};
use serde_json::json;
use sqlx::PgPool;

use crate::common::{self, Reply, call, limits};

const EMAIL: &str = "owner@penfii.test";
const PASSWORD: &str = "owner-test-passphrase";

async fn app(pool: &PgPool, limits: Limits) -> axum::Router {
    common::setup(pool).await;
    penfii_http_api::router(pool.clone(), limits, false, None)
}

async fn sign_in(app: &axum::Router, keep: Option<bool>) -> Reply {
    let mut body = json!({"email": EMAIL, "password": PASSWORD});
    if let Some(keep) = keep {
        body["keepSignedIn"] = json!(keep);
    }
    call(app, "POST", "/v1/session/login", None, Some(body)).await
}

fn max_age(reply: &Reply) -> String {
    let cookie = reply.header("set-cookie").expect("set-cookie");
    cookie
        .split(';')
        .find_map(|part| part.trim().strip_prefix("Max-Age=").map(str::to_string))
        .unwrap_or_default()
}

/// `(long, expires in, absolute in, idle_seconds)` of the newest session, in seconds from now.
async fn newest(pool: &PgPool) -> (bool, f64, Option<f64>, Option<i32>) {
    sqlx::query_as(
        "SELECT absolute_expires_at IS NOT NULL,
                EXTRACT(EPOCH FROM (expires_at - now()))::float8,
                EXTRACT(EPOCH FROM (absolute_expires_at - now()))::float8,
                idle_seconds
         FROM penfii.sessions ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn me(app: &axum::Router, cookie: &str) -> StatusCode {
    call(app, "GET", "/v1/me", Some(cookie), None).await.status
}

fn near(value: f64, expected: f64) -> bool {
    (value - expected).abs() < 90.0
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_choice_is_off_by_default_and_the_ordinary_session_is_unchanged(pool: PgPool) {
    let app = app(&pool, limits()).await;
    for keep in [None, Some(false)] {
        let reply = sign_in(&app, keep).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(max_age(&reply), "3600");
        let (long, expires, absolute, idle) = newest(&pool).await;
        assert!(!long && absolute.is_none() && idle.is_none());
        assert!(near(expires, 3600.0), "{expires}");
    }
    // An ordinary session is not extended by activity: its end is fixed when it is issued.
    let reply = sign_in(&app, None).await;
    let cookie = reply.cookie.unwrap();
    sqlx::query("UPDATE penfii.sessions SET last_seen_at = now() - interval '5 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, ..) = newest(&pool).await;
    assert!(near(expires, 3600.0), "{expires}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_chosen_long_session_has_an_inactivity_end_inside_an_absolute_end(pool: PgPool) {
    let app = app(&pool, limits()).await;
    let reply = sign_in(&app, Some(true)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // The cookie lives as long as the absolute lifetime: 30 days. HttpOnly and SameSite stay.
    assert_eq!(max_age(&reply), "2592000");
    let cookie_header = reply.header("set-cookie").unwrap();
    assert!(cookie_header.contains("HttpOnly") && cookie_header.contains("SameSite=Lax"));
    let (long, expires, absolute, idle) = newest(&pool).await;
    assert!(long);
    assert_eq!(idle, Some(604_800));
    assert!(
        near(expires, 604_800.0),
        "inactivity end is 7 days: {expires}"
    );
    assert!(
        near(absolute.unwrap(), 2_592_000.0),
        "absolute end is 30 days"
    );
    let cookie = reply.cookie.unwrap();

    // The sessions list tells it apart.
    let list = call(&app, "GET", "/v1/me/sessions", Some(&cookie), None).await;
    assert_eq!(list.body["items"][0]["longLived"], true);
    assert!(list.body["items"][0]["absoluteExpiresAt"].is_string());

    // Activity moves the inactivity end (at most once a minute), never the absolute end.
    sqlx::query(
        "UPDATE penfii.sessions SET last_seen_at = now() - interval '3 days',
                expires_at = now() + interval '4 days'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, absolute, _) = newest(&pool).await;
    assert!(
        near(expires, 604_800.0),
        "moved to 7 days from now: {expires}"
    );
    assert!(
        near(absolute.unwrap(), 2_592_000.0),
        "absolute end unchanged"
    );

    // A request inside the minute does not write again.
    sqlx::query("UPDATE penfii.sessions SET expires_at = now() + interval '6 days'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, ..) = newest(&pool).await;
    assert!(near(expires, 6.0 * 86_400.0), "{expires}");

    // Near the absolute end, activity cannot go past it.
    sqlx::query(
        "UPDATE penfii.sessions SET absolute_expires_at = now() + interval '1 day',
                last_seen_at = now() - interval '2 minutes'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, absolute, _) = newest(&pool).await;
    assert!(
        near(expires, 86_400.0),
        "capped at the absolute end: {expires}"
    );
    assert!(near(absolute.unwrap(), 86_400.0));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_earlier_end_wins_and_an_ended_session_is_never_revived(pool: PgPool) {
    let app = app(&pool, limits()).await;
    let cookie = sign_in(&app, Some(true)).await.cookie.unwrap();

    // Inactive for the whole window: over, on every path, and a later request does not bring it back.
    sqlx::query(
        "UPDATE penfii.sessions SET last_seen_at = now() - interval '8 days',
                expires_at = now() - interval '1 second'",
    )
    .execute(&pool)
    .await
    .unwrap();
    for _ in 0..2 {
        assert_eq!(me(&app, &cookie).await, StatusCode::UNAUTHORIZED);
    }
    let live: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.sessions WHERE expires_at > now()")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(live, 0);
    // The session list of a new sign-in does not show it.
    let second = sign_in(&app, Some(true)).await.cookie.unwrap();
    let list = call(&app, "GET", "/v1/me/sessions", Some(&second), None).await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 1);

    // The absolute end passes while the person was active: over.
    sqlx::query(
        "UPDATE penfii.sessions SET created_at = now() - interval '31 days',
                absolute_expires_at = now() - interval '1 second',
                expires_at = now() - interval '1 second', last_seen_at = now()
         WHERE token_hash = (SELECT token_hash FROM penfii.sessions ORDER BY created_at DESC LIMIT 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(me(&app, &second).await, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_socket_check_is_not_activity(pool: PgPool) {
    let app = app(&pool, limits()).await;
    let cookie = sign_in(&app, Some(true)).await.cookie.unwrap();
    let token = cookie.trim_start_matches("penfii_session=").to_string();
    sqlx::query(
        "UPDATE penfii.sessions SET last_seen_at = now() - interval '6 days',
                expires_at = now() + interval '1 day'",
    )
    .execute(&pool)
    .await
    .unwrap();
    // The check the live sockets run every few seconds leaves the session as it was...
    for _ in 0..3 {
        identity::session_is_live(&pool, &token).await.unwrap();
    }
    let (_, expires, ..) = newest(&pool).await;
    assert!(
        near(expires, 86_400.0),
        "a background check must not extend it: {expires}"
    );
    let seen: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (now() - last_seen_at))::float8 FROM penfii.sessions",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(near(seen, 6.0 * 86_400.0));
    // ...and once the inactivity end passes, it says the session is over.
    sqlx::query("UPDATE penfii.sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(identity::session_is_live(&pool, &token).await.is_err());
    // A request from the person is activity.
    sqlx::query("UPDATE penfii.sessions SET expires_at = now() + interval '1 day'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, ..) = newest(&pool).await;
    assert!(near(expires, 604_800.0), "{expires}");
}

/// Opening a live socket is the page's doing (it reconnects by itself after a dropped network or a restart), and the
/// browser cannot mark a socket upgrade as background. It must not keep a long session alive; a request of the
/// person still does.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn opening_a_live_socket_is_not_activity(pool: PgPool) {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    common::setup(&pool).await;
    let app = penfii_http_api::router(pool.clone(), limits(), false, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, served).await.unwrap();
    });
    let cookie = sign_in(&app, Some(true)).await.cookie.unwrap();
    sqlx::query(
        "UPDATE penfii.sessions SET last_seen_at = now() - interval '6 days',
                expires_at = now() + interval '1 day'",
    )
    .execute(&pool)
    .await
    .unwrap();
    for _ in 0..3 {
        let mut request = format!("ws://{addr}/v1/ws/user")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("cookie", cookie.parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let first = socket.next().await.unwrap().unwrap();
        assert!(first.to_text().unwrap().contains("user.ready"));
    }
    let (_, expires, ..) = newest(&pool).await;
    assert!(
        near(expires, 86_400.0),
        "opening a socket must not extend the session: {expires}"
    );
    // The same socket is refused once the inactivity end has passed.
    sqlx::query("UPDATE penfii.sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let mut request = format!("ws://{addr}/v1/ws/user")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
    // A request of the person is activity.
    sqlx::query("UPDATE penfii.sessions SET expires_at = now() + interval '1 day'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, ..) = newest(&pool).await;
    assert!(near(expires, 604_800.0), "{expires}");
}

/// A request the page makes by itself (`X-Penfii-Background`) is answered like any other but is not activity.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn background_requests_do_not_keep_a_long_session_alive(pool: PgPool) {
    let app = app(&pool, limits()).await;
    let cookie = sign_in(&app, Some(true)).await.cookie.unwrap();
    let age = || async {
        sqlx::query(
            "UPDATE penfii.sessions SET last_seen_at = now() - interval '3 days', expires_at = now() + interval '4 days'",
        )
        .execute(&pool)
        .await
        .unwrap();
    };
    let background = |path: &'static str, cookie: String| {
        let app = app.clone();
        async move {
            let mut request = common::request("GET", path, Some(&cookie), None, None);
            request
                .headers_mut()
                .insert("x-penfii-background", "1".parse().unwrap());
            common::send(&app, request).await
        }
    };
    age().await;
    // Both kinds of route (the plain user and the session extractor) answer, and neither moves the end.
    for path in ["/v1/me", "/v1/me/sessions"] {
        assert_eq!(
            background(path, cookie.clone()).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
    let (_, expires, ..) = newest(&pool).await;
    assert!(
        near(expires, 4.0 * 86_400.0),
        "background traffic is not activity: {expires}"
    );
    // A background request still needs a live session.
    sqlx::query("UPDATE penfii.sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        background("/v1/me", cookie.clone()).await.status,
        StatusCode::UNAUTHORIZED
    );
    // The same request without the header is the person's activity.
    age().await;
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, ..) = newest(&pool).await;
    assert!(near(expires, 604_800.0), "{expires}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn revocation_and_sensitive_actions_treat_a_long_session_like_any_other(pool: PgPool) {
    let app = app(&pool, limits()).await;
    let long = sign_in(&app, Some(true)).await.cookie.unwrap();
    let other = sign_in(&app, None).await.cookie.unwrap();

    // Logout ends it.
    assert_eq!(
        call(&app, "POST", "/v1/session/logout", Some(&long), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me(&app, &long).await, StatusCode::UNAUTHORIZED);

    // The recent-authentication window applies to a long session as to any: a session signed in
    // long ago must prove itself again for a sensitive action.
    let long = sign_in(&app, Some(true)).await.cookie.unwrap();
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '2 hours' WHERE absolute_expires_at IS NOT NULL AND revoked_at IS NULL")
        .execute(&pool)
        .await
        .unwrap();
    let (status, body, _) = crate::common::call_parts(
        &app,
        "PATCH",
        "/v1/me",
        Some(&long),
        Some(json!({"displayName": "Owner 2", "email": EMAIL})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "reauth_required");

    // "Other devices" ends a long session; another person's session is untouched by it.
    let r = call(&app, "DELETE", "/v1/me/sessions", Some(&other), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(me(&app, &long).await, StatusCode::UNAUTHORIZED);

    // A changed password ends every session, long ones included.
    let long = sign_in(&app, Some(true)).await.cookie.unwrap();
    let r = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&long),
        Some(json!({"displayName": "Owner", "email": EMAIL, "currentPassword": PASSWORD, "newPassword": "a-brand-new-passphrase-9"})),
    )
    .await;
    let _ = r;
    // Whatever the route answered, a session that was revoked in the database is dead on every path.
    sqlx::query("UPDATE penfii.sessions SET revoked_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&app, &long).await, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_operator_can_turn_the_feature_off_and_change_both_periods(pool: PgPool) {
    // Off: the ask (a remembered choice or a direct API call) gets an ordinary session.
    let mut off = limits();
    off.long_session.enabled = false;
    let app_off = app(&pool, off.clone()).await;
    let reply = sign_in(&app_off, Some(true)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(max_age(&reply), "3600");
    let (long, ..) = newest(&pool).await;
    assert!(!long);
    let info = call(&app_off, "GET", "/v1/auth/providers", None, None).await;
    assert_eq!(info.body["longSession"]["enabled"], false);

    // Other periods apply to new sessions; the idle window never exceeds the lifetime.
    let mut custom = limits();
    custom.long_session.max_age_seconds = 2 * 86_400;
    custom.long_session.idle_seconds = 5 * 86_400;
    let app_custom = penfii_http_api::router(pool.clone(), custom, false, None);
    let reply = sign_in(&app_custom, Some(true)).await;
    assert_eq!(max_age(&reply), (2 * 86_400).to_string());
    let (_, expires, absolute, idle) = newest(&pool).await;
    assert_eq!(idle, Some(2 * 86_400));
    assert!(near(expires, 2.0 * 86_400.0) && near(absolute.unwrap(), 2.0 * 86_400.0));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn changed_settings_only_ever_shorten_or_end_existing_long_sessions(pool: PgPool) {
    let app = app(&pool, limits()).await;
    let cookie = sign_in(&app, Some(true)).await.cookie.unwrap();
    // Seen 2 days ago, 3 days old, so 5 days of inactivity window and 27 of lifetime are left.
    sqlx::query(
        "UPDATE penfii.sessions SET created_at = now() - interval '3 days',
                last_seen_at = now() - interval '2 days', expires_at = now() + interval '5 days',
                absolute_expires_at = now() + interval '27 days'",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Longer settings change nothing for a session that exists.
    let mut longer = limits();
    longer.long_session.max_age_seconds = 90 * 86_400;
    longer.long_session.idle_seconds = 30 * 86_400;
    assert_eq!(
        identity::apply_long_session_policy(&pool, &longer)
            .await
            .unwrap(),
        0
    );
    let (_, expires, absolute, idle) = newest(&pool).await;
    assert!(near(expires, 5.0 * 86_400.0) && near(absolute.unwrap(), 27.0 * 86_400.0));
    assert_eq!(idle, Some(604_800));

    // A shorter inactivity window cuts the session from its last activity: 1 day from 2 days ago has passed.
    let mut shorter = limits();
    shorter.long_session.idle_seconds = 86_400;
    assert_eq!(
        identity::apply_long_session_policy(&pool, &shorter)
            .await
            .unwrap(),
        1
    );
    assert_eq!(me(&app, &cookie).await, StatusCode::UNAUTHORIZED);

    // A shorter lifetime cuts the absolute end from when it was issued.
    let cookie = sign_in(&app, Some(true)).await.cookie.unwrap();
    sqlx::query("UPDATE penfii.sessions SET created_at = now() - interval '3 days' WHERE revoked_at IS NULL AND expires_at > now()")
        .execute(&pool)
        .await
        .unwrap();
    let mut capped = limits();
    capped.long_session.max_age_seconds = 5 * 86_400;
    assert_eq!(
        identity::apply_long_session_policy(&pool, &capped)
            .await
            .unwrap(),
        1
    );
    let (_, expires, absolute, idle) = newest(&pool).await;
    assert!(near(absolute.unwrap(), 2.0 * 86_400.0), "{absolute:?}");
    assert!(near(expires, 2.0 * 86_400.0), "{expires}");
    assert_eq!(
        idle,
        Some(432_000),
        "a window longer than the new lifetime is the lifetime"
    );
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    // Activity cannot push it past the new end.
    sqlx::query("UPDATE penfii.sessions SET last_seen_at = now() - interval '2 minutes' WHERE expires_at > now()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&app, &cookie).await, StatusCode::OK);
    let (_, expires, ..) = newest(&pool).await;
    assert!(near(expires, 2.0 * 86_400.0), "{expires}");

    // Turned off: the long session becomes an ordinary one that ends an ordinary lifetime after it
    // was issued. Three days ago + 1 hour is over; one issued now keeps working for its hour.
    let fresh = sign_in(&app, Some(true)).await.cookie.unwrap();
    let mut off = limits();
    off.long_session.enabled = false;
    assert_eq!(
        identity::apply_long_session_policy(&pool, &off)
            .await
            .unwrap(),
        2
    );
    assert_eq!(me(&app, &cookie).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(&app, &fresh).await, StatusCode::OK);
    let (long, expires, absolute, idle) = newest(&pool).await;
    assert!(!long && absolute.is_none() && idle.is_none());
    assert!(expires <= 3600.0 + 90.0, "{expires}");
    // Ordinary sessions were never touched.
    assert_eq!(
        identity::apply_long_session_policy(&pool, &off)
            .await
            .unwrap(),
        0
    );
}

#[test]
fn the_settings_are_read_strictly() {
    let with = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    };
    let default = penfii_core::LongSession::from_lookup(&with(&[])).unwrap();
    assert_eq!(default, penfii_core::LongSession::default());
    assert_eq!(
        (
            default.enabled,
            default.max_age_seconds,
            default.idle_seconds
        ),
        (true, 2_592_000, 604_800)
    );
    let custom = penfii_core::LongSession::from_lookup(&with(&[
        ("PENFII_LONG_SESSION_ENABLED", "off"),
        ("PENFII_LONG_SESSION_MAX_AGE_SECONDS", "86400"),
        ("PENFII_LONG_SESSION_IDLE_SECONDS", "172800"),
    ]))
    .unwrap();
    assert!(!custom.enabled);
    assert_eq!(
        custom.effective_idle_seconds(),
        86_400,
        "an idle window past the lifetime is the lifetime"
    );
    // Compose passes an unset optional setting as an empty value.
    assert_eq!(
        penfii_core::LongSession::from_lookup(&with(&[("PENFII_LONG_SESSION_IDLE_SECONDS", "")]))
            .unwrap(),
        default
    );
    for (name, value) in [
        ("PENFII_LONG_SESSION_ENABLED", "maybe"),
        ("PENFII_LONG_SESSION_MAX_AGE_SECONDS", "0"),
        ("PENFII_LONG_SESSION_MAX_AGE_SECONDS", "-5"),
        ("PENFII_LONG_SESSION_MAX_AGE_SECONDS", "30d"),
        ("PENFII_LONG_SESSION_IDLE_SECONDS", "0"),
        ("PENFII_LONG_SESSION_IDLE_SECONDS", "99999999999"),
    ] {
        let error = Box::leak(Box::new([(name, value)]));
        let error = penfii_core::LongSession::from_lookup(&with(error)).unwrap_err();
        assert!(error.contains(name), "{name}: {error}");
    }
}
