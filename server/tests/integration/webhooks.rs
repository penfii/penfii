//! Incoming webhooks: management, both credentials, and posting through the shared message path.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use crate::common::{self, Env, send};

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body.as_bytes());
    format!("v1={}", hex::encode(mac.finalize().into_bytes()))
}

async fn post_hook(
    env: &Env,
    credential: &str,
    body: &str,
    extra: &[(&str, &str)],
    peer: Option<SocketAddr>,
) -> common::Reply {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/v1/hooks/{credential}"))
        .header("content-type", "application/json");
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::from(body.to_string())).unwrap();
    if let Some(peer) = peer {
        request.extensions_mut().insert(ConnectInfo(peer));
    }
    send(&env.app, request).await
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn webhooks_are_issued_and_post_as_the_integration_actor(pool: PgPool) {
    let env = Env::new(pool).await;
    let member = env.person("member", "MEMBER").await;
    let channel = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(serde_json::json!({"name": "hooks", "visibility": "PUBLIC"})),
        )
        .await;
    assert_eq!(channel.status, StatusCode::CREATED, "{}", channel.body);
    let channel_id = channel.body["id"].as_str().unwrap();

    let denied = env
        .call(
            &member,
            "POST",
            &format!("/v1/channels/{channel_id}/webhooks"),
            Some(serde_json::json!({"name": "nope", "authMode": "URL_SECRET", "displayName": "Nope"})),
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let url = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/channels/{channel_id}/webhooks"),
            Some(serde_json::json!({
                "name": "deploys",
                "authMode": "URL_SECRET",
                "displayName": "Deploy Bot"
            })),
        )
        .await;
    assert_eq!(url.status, StatusCode::CREATED, "{}", url.body);
    let url_secret = url.body["secret"].as_str().unwrap().to_string();
    let url_hook = url.body["webhook"]["id"].as_str().unwrap().to_string();
    let actor_id = url.body["webhook"]["actorId"].as_str().unwrap().to_string();
    assert_eq!(url.body["webhook"]["authMode"], "URL_SECRET");

    let signed = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/channels/{channel_id}/webhooks"),
            Some(serde_json::json!({
                "name": "builds",
                "authMode": "SIGNATURE",
                "actorId": actor_id
            })),
        )
        .await;
    assert_eq!(signed.status, StatusCode::CREATED, "{}", signed.body);
    let sig_secret = signed.body["secret"].as_str().unwrap().to_string();
    let sig_hook = signed.body["webhook"]["id"].as_str().unwrap().to_string();
    assert_eq!(signed.body["webhook"]["actorId"], actor_id);

    let missing_kek = penfii_core::webhooks::create_webhook(
        &env.pool,
        env.owner.id,
        penfii_core::webhooks::NewWebhook {
            channel_id: channel_id.parse().unwrap(),
            name: "needs-key",
            auth_mode: "SIGNATURE",
            actor_id: None,
            display_name: Some("Other Bot"),
            kek: None,
        },
    )
    .await;
    assert!(
        matches!(missing_kek, Err(penfii_core::CoreError::Conflict { code, .. }) if code == "webhook_kek_unconfigured"),
        "{missing_kek:?}"
    );

    let list = env
        .call(
            &env.owner,
            "GET",
            &format!("/v1/channels/{channel_id}/webhooks"),
            None,
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    let listed = list.body.to_string();
    assert!(!listed.contains(&url_secret));
    assert!(!listed.contains(&sig_secret));
    assert_eq!(list.body["items"].as_array().unwrap().len(), 2);

    let body = r#"{"text":"shipped"}"#;
    let posted = post_hook(
        &env,
        &url_secret,
        body,
        &[("cookie", env.owner.cookie.as_str())],
        None,
    )
    .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    assert_eq!(posted.body["authorId"], actor_id);
    assert_eq!(posted.body["authorKind"], "INTEGRATION");
    assert_eq!(posted.body["authorDisplayName"], "Deploy Bot");
    assert_ne!(posted.body["authorId"], env.owner.id.to_string());
    let message_id = posted.body["id"].as_str().unwrap().to_string();

    let forged = post_hook(
        &env,
        &url_secret,
        r#"{"text":"nope","channelId":"00000000-0000-0000-0000-000000000000"}"#,
        &[],
        None,
    )
    .await;
    assert_eq!(forged.status, StatusCode::BAD_REQUEST, "{}", forged.body);

    let now = Utc::now().timestamp();
    let signed_body = r#"{"text":"built"}"#;
    let signature = sign(&sig_secret, now, signed_body);
    let signed_post = post_hook(
        &env,
        &sig_hook,
        signed_body,
        &[
            ("x-penfii-timestamp", &now.to_string()),
            ("x-penfii-signature", &signature),
            ("x-penfii-delivery", "delivery-1"),
        ],
        None,
    )
    .await;
    assert_eq!(
        signed_post.status,
        StatusCode::CREATED,
        "{}",
        signed_post.body
    );
    assert_eq!(signed_post.body["authorId"], actor_id);
    let replay = post_hook(
        &env,
        &sig_hook,
        signed_body,
        &[
            ("x-penfii-timestamp", &now.to_string()),
            ("x-penfii-signature", &signature),
            ("x-penfii-delivery", "delivery-1"),
        ],
        None,
    )
    .await;
    assert_eq!(replay.status, StatusCode::CREATED, "{}", replay.body);
    assert_eq!(replay.body["id"], signed_post.body["id"]);

    let other = r#"{"text":"built again"}"#;
    let other_sig = sign(&sig_secret, now, other);
    let conflict = post_hook(
        &env,
        &sig_hook,
        other,
        &[
            ("x-penfii-timestamp", &now.to_string()),
            ("x-penfii-signature", &other_sig),
            ("x-penfii-delivery", "delivery-1"),
        ],
        None,
    )
    .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.body);
    assert_eq!(conflict.body["code"], "idempotency_conflict");

    let stale = sign(&sig_secret, now - 301, signed_body);
    let stale_post = post_hook(
        &env,
        &sig_hook,
        signed_body,
        &[
            ("x-penfii-timestamp", &(now - 301).to_string()),
            ("x-penfii-signature", &stale),
        ],
        None,
    )
    .await;
    let tampered = post_hook(
        &env,
        &sig_hook,
        r#"{"text":"built "}"#,
        &[
            ("x-penfii-timestamp", &now.to_string()),
            ("x-penfii-signature", &signature),
        ],
        None,
    )
    .await;
    let unknown = post_hook(&env, "not-a-real-token", body, &[], None).await;
    assert_eq!(stale_post.status, StatusCode::UNAUTHORIZED);
    assert_eq!(tampered.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(stale_post.body, unknown.body);
    assert_eq!(tampered.body, unknown.body);
    assert!(!stale_post.body.to_string().contains(&sig_secret));

    let rotated = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/webhooks/{url_hook}/rotate"),
            None,
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    let new_secret = rotated.body["secret"].as_str().unwrap().to_string();
    assert_ne!(new_secret, url_secret);
    let old = post_hook(&env, &url_secret, body, &[], None).await;
    let fresh = post_hook(&env, &new_secret, body, &[], None).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED, "{}", old.body);
    assert_eq!(fresh.status, StatusCode::CREATED, "{}", fresh.body);
    assert_eq!(old.body, unknown.body);

    let revoked = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/webhooks/{url_hook}/revoke"),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.body);
    assert_eq!(revoked.body["revoked"], true);
    let after_revoke = post_hook(&env, &new_secret, body, &[], None).await;
    assert_eq!(after_revoke.status, StatusCode::UNAUTHORIZED);
    assert_eq!(after_revoke.body, unknown.body);

    sqlx::query("UPDATE penfii.integration_actors SET disabled_at = now() WHERE user_id = $1")
        .bind(actor_id.parse::<uuid::Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    let disabled = post_hook(
        &env,
        &sig_hook,
        signed_body,
        &[
            ("x-penfii-timestamp", &Utc::now().timestamp().to_string()),
            (
                "x-penfii-signature",
                &sign(&sig_secret, Utc::now().timestamp(), signed_body),
            ),
        ],
        None,
    )
    .await;
    assert_eq!(disabled.status, StatusCode::UNAUTHORIZED);
    assert_eq!(disabled.body, unknown.body);

    let email: String = sqlx::query_scalar("SELECT email FROM penfii.users WHERE id = $1")
        .bind(actor_id.parse::<uuid::Uuid>().unwrap())
        .fetch_one(&env.pool)
        .await
        .unwrap();
    let (login_status, cookie) = common::login(&env.app, &email, "whatever-password").await;
    assert_eq!(login_status, StatusCode::UNAUTHORIZED);
    assert!(cookie.is_none());
    let member_insert = sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role) VALUES ($1, $2, 'MEMBER')",
    )
    .bind(env.ws)
    .bind(actor_id.parse::<uuid::Uuid>().unwrap())
    .execute(&env.pool)
    .await;
    assert!(member_insert.is_err(), "integration actors cannot join");

    let human = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/conversations/{channel_id}/messages"),
            Some(serde_json::json!({
                "bodySchemaVersion": 1,
                "body": common::doc("from a person"),
                "threadRootId": null,
                "replyToId": null
            })),
        )
        .await;
    assert_eq!(human.status, StatusCode::CREATED, "{}", human.body);
    assert_eq!(human.body["authorKind"], "HUMAN");
    assert_eq!(human.body["authorId"], env.owner.id.to_string());

    let (actor_kind, webhook_id): (String, String) = sqlx::query_as(
        "SELECT actor_kind, security_context->>'webhookId'
         FROM penfii.domain_events
         WHERE event_kind = 'message.created' AND target_id = $1",
    )
    .bind(message_id.parse::<uuid::Uuid>().unwrap())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(actor_kind, "INTEGRATION");
    assert_eq!(webhook_id, url_hook);

    let changes = env
        .call(
            &env.owner,
            "GET",
            &format!("/v1/conversations/{channel_id}/changes?limit=50"),
            None,
        )
        .await;
    assert_eq!(changes.status, StatusCode::OK, "{}", changes.body);
    assert!(
        changes.body.as_array().unwrap().iter().any(|change| {
            change["messageId"] == message_id && change["changeKind"] == "message.created"
        }),
        "{}",
        changes.body
    );

    let audits: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT action_code, data FROM penfii.audit_entries
         WHERE action_code LIKE 'webhook.%' ORDER BY occurred_at",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    let actions: Vec<&str> = audits.iter().map(|(action, _)| action.as_str()).collect();
    assert!(actions.contains(&"webhook.created"));
    assert!(actions.contains(&"webhook.rotated"));
    assert!(actions.contains(&"webhook.revoked"));
    for (_, data) in &audits {
        let text = data.to_string();
        assert!(!text.contains(&url_secret));
        assert!(!text.contains(&sig_secret));
        assert!(!text.contains(&new_secret));
    }

    let dm = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/dms", env.ws),
            Some(serde_json::json!({"userIds": [env.owner.id, member.id]})),
        )
        .await;
    assert_eq!(dm.status, StatusCode::CREATED, "{}", dm.body);
    let on_dm = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/channels/{}/webhooks", dm.body["id"].as_str().unwrap()),
            Some(serde_json::json!({"name": "dm", "authMode": "URL_SECRET", "displayName": "Dm Bot"})),
        )
        .await;
    assert_eq!(on_dm.status, StatusCode::NOT_FOUND, "{}", on_dm.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_archived_team_rejects_the_post_and_the_body_is_limited(pool: PgPool) {
    let env = Env::new(pool).await;
    let team = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/teams", env.ws),
            Some(serde_json::json!({"name": "Ship", "members": [{"userId": env.owner.id, "role": "MANAGER"}]})),
        )
        .await;
    assert_eq!(team.status, StatusCode::CREATED, "{}", team.body);
    let team_id = team.body["id"].as_str().unwrap();
    let project = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(serde_json::json!({"name": "Ship", "teamIds": [team_id]})),
        )
        .await;
    assert_eq!(project.status, StatusCode::CREATED, "{}", project.body);
    let project_id = project.body["id"].as_str().unwrap();
    let channel = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(serde_json::json!({
                "name": "releases",
                "visibility": "PUBLIC",
                "projectId": project_id,
                "teamId": team_id
            })),
        )
        .await;
    assert_eq!(channel.status, StatusCode::CREATED, "{}", channel.body);
    let channel_id = channel.body["id"].as_str().unwrap();
    let created = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/channels/{channel_id}/webhooks"),
            Some(
                serde_json::json!({"name": "rel", "authMode": "URL_SECRET", "displayName": "Rel"}),
            ),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let secret = created.body["secret"].as_str().unwrap();

    let archived = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/teams/{team_id}/archive", env.ws),
            Some(serde_json::json!({})),
        )
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", archived.body);
    let blocked = post_hook(&env, secret, r#"{"text":"later"}"#, &[], None).await;
    assert_eq!(blocked.status, StatusCode::CONFLICT, "{}", blocked.body);
    assert_eq!(blocked.body["code"], "channel_read_only");

    let huge = format!(r#"{{"text":"{}"}}"#, "x".repeat(70_000));
    let limited = post_hook(&env, secret, &huge, &[], None).await;
    assert_eq!(limited.status, StatusCode::BAD_REQUEST, "{}", limited.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn posts_are_limited_per_webhook_and_auth_failures_per_address(pool: PgPool) {
    let env = Env::new(pool).await;
    let channel = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(serde_json::json!({"name": "limits", "visibility": "PUBLIC"})),
        )
        .await;
    let channel_id = channel.body["id"].as_str().unwrap();
    let created = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/channels/{channel_id}/webhooks"),
            Some(serde_json::json!({"name": "fast", "authMode": "URL_SECRET", "displayName": "Fast"})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let secret = created.body["secret"].as_str().unwrap();
    for n in 0..60 {
        let reply = post_hook(
            &env,
            secret,
            &format!(r#"{{"text":"n{n}"}}"#),
            &[("x-penfii-delivery", &format!("n{n}"))],
            None,
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{n} {}", reply.body);
    }
    let limited = post_hook(
        &env,
        secret,
        r#"{"text":"over"}"#,
        &[("x-penfii-delivery", "over")],
        None,
    )
    .await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert_eq!(limited.body["code"], "rate_limited");

    let peer: SocketAddr = "203.0.113.9:4242".parse().unwrap();
    for _ in 0..30 {
        let reply = post_hook(&env, "nope", r#"{"text":"x"}"#, &[], Some(peer)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
    }
    let locked = post_hook(&env, "nope", r#"{"text":"x"}"#, &[], Some(peer)).await;
    assert_eq!(
        locked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        locked.body
    );
    assert_eq!(locked.body["code"], "rate_limited");
}
