//! The product runs as the runtime database role: no DDL, no ownership, only row access (plus the
//! timeouts of that role). The database is made and migrated by the owner (here the superuser of the
//! test server, as in an installation from before the role split), the roles are provisioned the way
//! `admin-cli db-provision` does, and the same flows then go through a router over the runtime pool.

use super::docs::make_doc;
use super::{World, call, channel, make_ticket, new_project, workspace};
use crate::common::{self, doc, role_names, url_as};
use axum::http::StatusCode;
use serde_json::json;
use sqlx::{Connection, PgPool};
use uuid::Uuid;

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_product_flows_work_as_the_runtime_role(pool: PgPool) {
    let admin = &World::new(pool).await;
    let owner: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&admin.pool)
        .await
        .unwrap();
    let (_, runtime) = role_names();
    let mut conn = sqlx::PgConnection::connect(&common::database_url(&admin.pool))
        .await
        .unwrap();
    penfii_core::provision::provision(
        &mut conn,
        &penfii_core::provision::Provision {
            owner_role: owner,
            owner_password: None,
            runtime_role: runtime.clone(),
            runtime_password: Some("runtime-pass".into()),
        },
    )
    .await
    .unwrap();
    let app_pool = penfii_core::connect(&url_as(&admin.pool, &runtime, "runtime-pass"))
        .await
        .unwrap();
    penfii_core::require_current_schema(&app_pool)
        .await
        .unwrap();
    let world = &admin.over(app_pool.clone());

    // Chat: a channel, a message.
    let ws = workspace(world, &format!("rt-{}", Uuid::new_v4())).await;
    let channel_id = channel(world, ws, "general", "PUBLIC").await;
    let (status, body, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{channel_id}/messages"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(
            json!({"bodySchemaVersion":1,"body":doc("hello"),"threadRootId":null,"replyToId":null}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Work: a Project, a Ticket, a comment, a status change.
    let (project_id, _) = new_project(world, ws, "Runtime").await;
    let (status, ticket) = make_ticket(world, &project_id, "rt-1", json!({"title":"T"})).await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let (status, comment, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{ticket_id}/comments"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"body": doc("a comment")})),
    )
    .await;
    assert!(status.is_success(), "{status} {comment}");

    // Docs.
    let page = make_doc(world, &world.owner_cookie, ws, "A page", json!({})).await;
    assert!(page["id"].is_string(), "{page}");

    // The event outbox: the dispatcher claims, fans out and runs the Inbox and Watch consumers as the
    // runtime role (events have an identity column; the receipts are plain rows).
    let mut registry = penfii_core::consumers::registry(None, None);
    registry.workspace = Some(ws);
    penfii_core::events::run_until_idle(&app_pool, &registry)
        .await
        .unwrap();
    let dead: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.consumer_receipts WHERE state = 'DEAD'")
            .fetch_one(&app_pool)
            .await
            .unwrap();
    assert_eq!(dead, 0);
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.domain_events WHERE workspace_id = $1")
            .bind(ws)
            .fetch_one(&app_pool)
            .await
            .unwrap();
    assert!(events >= 3, "{events}");

    // Sign in and out as the runtime role, a password change (Argon2 outside the transaction).
    let cookie = common::sign_in(&world.app, common::OWNER_EMAIL, common::OWNER_PASSWORD).await;
    assert!(!cookie.is_empty());
    let (status, _, _) = call(&world.app, "GET", "/v1/me", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK);
}
