//! Session metadata, recent authentication, session management, login throttle.

use axum::http::StatusCode;
use penfii_core::apply_migrations;
use penfii_core::identity;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{self, call_parts as call, limits, login};

async fn setup(pool: &PgPool) -> axum::Router {
    common::setup(pool).await;
    // Applying twice must be harmless.
    apply_migrations(pool).await.expect("migrate again");
    penfii_http_api::router(pool.clone(), limits(), false, None)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn recent_authentication_sessions_and_throttle(pool: PgPool) {
    let app = setup(&pool).await;

    // Fresh sign-in is recent: profile change works with no password in the request.
    let (status, cookie) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.unwrap();
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(json!({"displayName": "Owner 2", "email": "owner@penfii.test"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Age the session: the same call now needs reauthentication and names the usable methods.
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(json!({"displayName": "Owner 3", "email": "owner@penfii.test"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "reauth_required");
    assert_eq!(body["methods"], json!(["password"]));

    // The old client contract still works: current password = inline reauth.
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(json!({"displayName": "Owner 3", "email": "owner@penfii.test", "currentPassword": "owner-test-passphrase"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Explicit reauth: wrong password is 401, right password restores the window.
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, ..) = call(
        &app,
        "POST",
        "/v1/me/reauth/password",
        Some(&cookie),
        Some(json!({"password": "wrong"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, ..) = call(
        &app,
        "POST",
        "/v1/me/reauth/password",
        Some(&cookie),
        Some(json!({"password": "owner-test-passphrase"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A second session; listing marks the current one; revoking another person's id is 404.
    let (_, other_cookie) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    let other_cookie = other_cookie.unwrap();
    let (status, list, _) = call(&app, "GET", "/v1/me/sessions", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items.iter().filter(|s| s["current"] == true).count(), 1);
    assert_eq!(list["reauthMethods"], json!(["password"]));
    let (status, ..) = call(
        &app,
        "DELETE",
        &format!("/v1/me/sessions/{}", uuid::Uuid::new_v4()),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Revoking all others needs a recent proof, then ends the other session at once.
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, body, _) = call(&app, "DELETE", "/v1/me/sessions", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() WHERE true")
        .execute(&pool)
        .await
        .unwrap();
    let (status, body, _) = call(&app, "DELETE", "/v1/me/sessions", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revoked"], 1);
    let (status, ..) = call(&app, "GET", "/v1/me", Some(&other_cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, ..) = call(&app, "GET", "/v1/me", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);

    // Throttle: five wrong passwords lock the e-mail; a correct one is refused while locked, and
    // an unknown address behaves identically.
    for _ in 0..5 {
        let (status, _) = login(&app, "owner@penfii.test", "nope").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    for _ in 0..5 {
        let (status, _) = login(&app, "ghost@penfii.test", "nope").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = login(&app, "ghost@penfii.test", "nope").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // The lock expires; success clears the counter.
    sqlx::query("UPDATE penfii.auth_throttle SET locked_until = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    assert_eq!(status, StatusCode::OK);

    // Per-address throttle: many failures from one address lock that address, whatever the
    // account; another address is unaffected; the label is stored on the session.
    let limits = limits();
    for i in 0..30 {
        let r = identity::login_from(
            &pool,
            &limits,
            &format!("nobody{i}@penfii.test"),
            "x",
            Some("203.0.113.5"),
            None,
        )
        .await;
        assert!(matches!(r, Err(penfii_core::CoreError::Unauthenticated)));
    }
    let locked = identity::login_from(
        &pool,
        &limits,
        "owner@penfii.test",
        "owner-test-passphrase",
        Some("203.0.113.5"),
        None,
    )
    .await;
    assert!(matches!(
        locked,
        Err(penfii_core::CoreError::RateLimited { .. })
    ));
    let ok = identity::login_from(
        &pool,
        &limits,
        "owner@penfii.test",
        "owner-test-passphrase",
        Some("203.0.113.6"),
        Some("Chrome on Linux"),
    )
    .await;
    assert!(ok.is_ok());
    let label: Option<String> = sqlx::query_scalar(
        "SELECT device_label FROM penfii.sessions WHERE device_label IS NOT NULL",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(label.as_deref(), Some("Chrome on Linux"));
}

// ---- FEA-139: a socket that is already open follows its session --------------------------------

/// The router served on a real port, with sockets that look at their session every 100 ms.
async fn serve_with_fast_recheck(pool: &PgPool) -> (axum::Router, std::net::SocketAddr) {
    common::setup(pool).await;
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        ws_session_recheck: std::time::Duration::from_millis(100),
        ..penfii_http_api::RouterConfig::new(pool.clone(), limits(), false)
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, served).await.unwrap();
    });
    (app, addr)
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open_user_socket(addr: std::net::SocketAddr, cookie: &str) -> Socket {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = format!("ws://{addr}/v1/ws/user")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    // `user.ready` first.
    use futures_util::StreamExt;
    let first = socket.next().await.unwrap().unwrap();
    assert!(first.to_text().unwrap().contains("user.ready"));
    socket
}

/// What ends the socket: the close code, or `None` when it is still open after `wait`.
async fn closed_within(socket: &mut Socket, wait: std::time::Duration) -> Option<u16> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, socket.next()).await {
            Err(_) => return None,
            Ok(None) | Ok(Some(Err(_))) => return Some(1006),
            Ok(Some(Ok(Message::Close(frame)))) => {
                return Some(frame.map(|f| u16::from(f.code)).unwrap_or(1005));
            }
            Ok(Some(Ok(_))) => continue,
        }
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_open_socket_ends_with_its_session_and_only_then(pool: PgPool) {
    let (app, addr) = serve_with_fast_recheck(&pool).await;
    let (_, here) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    let (_, there) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    let (here, there) = (here.unwrap(), there.unwrap());
    let mut here_socket = open_user_socket(addr, &here).await;
    let mut there_socket = open_user_socket(addr, &there).await;

    // A live session keeps its socket across many checks.
    assert_eq!(
        closed_within(&mut here_socket, std::time::Duration::from_millis(600)).await,
        None
    );

    // Signing the other device out ends that socket (4401, "session ended"), not this one.
    let (_, listed, _) = call(&app, "GET", "/v1/me/sessions", Some(&here), None).await;
    let other = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["current"] == false)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/v1/me/sessions/{other}"),
        Some(&here),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        closed_within(&mut there_socket, std::time::Duration::from_secs(5)).await,
        Some(4401)
    );
    assert_eq!(
        closed_within(&mut here_socket, std::time::Duration::from_millis(400)).await,
        None
    );

    // A password reset (here: the operator's) ends every web session, and so this socket.
    identity::admin_set_password(&pool, "owner@penfii.test", "operator set passphrase")
        .await
        .unwrap();
    assert_eq!(
        closed_within(&mut here_socket, std::time::Duration::from_secs(5)).await,
        Some(4401)
    );
}
