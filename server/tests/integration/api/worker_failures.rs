//! A panic inside one event is that event's failure (attempts, DEAD), not the worker's end; and what
//! `/readyz` says about a backlog of receipts.

use super::{World, call, make_ticket, new_project};
use axum::http::StatusCode;
use penfii_core::CoreError;
use penfii_core::events::{EventRow, ExternalConsumer, HandlerFuture, PgConsumer, Registry};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

/// Panics for the event of one ticket, handles every other one.
struct PanicsFor {
    name: &'static str,
    poison: Uuid,
    handled: AtomicUsize,
}

impl PanicsFor {
    fn new(name: &'static str, poison: Uuid) -> Arc<Self> {
        Arc::new(Self {
            name,
            poison,
            handled: AtomicUsize::new(0),
        })
    }
}

impl PgConsumer for PanicsFor {
    fn name(&self) -> &'static str {
        self.name
    }
    fn accepts(&self, kind: &str) -> bool {
        kind == "ticket.created"
    }
    fn handle<'a>(&'a self, _conn: &'a mut PgConnection, event: &'a EventRow) -> HandlerFuture<'a> {
        Box::pin(async move {
            if event.target_id == self.poison {
                panic!("this event always panics");
            }
            self.handled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

impl ExternalConsumer for PanicsFor {
    fn name(&self) -> &'static str {
        self.name
    }
    fn accepts(&self, kind: &str) -> bool {
        kind == "ticket.created"
    }
    fn handle<'a>(&'a self, _pool: &'a PgPool, event: &'a EventRow) -> HandlerFuture<'a> {
        Box::pin(async move {
            if event.target_id == self.poison {
                panic!("this event always panics");
            }
            self.handled.fetch_add(1, Ordering::SeqCst);
            Ok::<(), CoreError>(())
        })
    }
}

/// Two tickets in the world's Workspace: `(poison, fine)`.
async fn two_tickets(world: &World) -> (Uuid, Uuid) {
    let (project, _) = new_project(world, world.workspace_id, "Panics").await;
    let mut ids = Vec::new();
    for key in ["wf-a", "wf-b"] {
        let (status, value) = make_ticket(world, &project, key, json!({"title": key})).await;
        assert_eq!(status, StatusCode::CREATED, "{value}");
        ids.push(value["id"].as_str().unwrap().parse::<Uuid>().unwrap());
    }
    (ids[0], ids[1])
}

fn registry(world: &World) -> Registry {
    Registry {
        workspace: Some(world.workspace_id),
        backoff_base_secs: 0,
        max_backoff_secs: 0,
        max_attempts: 3,
        ..Registry::default()
    }
}

async fn receipt(world: &World, consumer: &str, ticket: Uuid) -> (String, i32, Option<String>) {
    sqlx::query_as(
        "SELECT r.state, r.attempts, r.last_error_code FROM penfii.consumer_receipts r
         JOIN penfii.domain_events e ON e.event_id = r.event_id
         WHERE r.consumer = $1 AND e.target_id = $2 AND e.event_kind = 'ticket.created'",
    )
    .bind(consumer)
    .bind(ticket)
    .fetch_one(&world.pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_panic_in_a_database_consumer_is_that_events_failure(pool: PgPool) {
    let world = &World::new(pool).await;
    let (poison, fine) = two_tickets(world).await;
    let panics = PanicsFor::new("wf_pg_panics", poison);
    let steady = PanicsFor::new("wf_pg_steady", Uuid::nil());
    let mut registry = registry(world);
    registry.pg.push(panics.clone());
    registry.pg.push(steady.clone());

    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();

    // The event that panics is counted like any failure and ends as DEAD; the code says why.
    assert_eq!(
        receipt(world, "wf_pg_panics", poison).await,
        ("DEAD".into(), 3, Some("panic".into()))
    );
    // The other event of the same consumer and the other consumer were not held back.
    assert_eq!(receipt(world, "wf_pg_panics", fine).await.0, "DONE");
    assert_eq!(panics.handled.load(Ordering::SeqCst), 1);
    assert_eq!(receipt(world, "wf_pg_steady", poison).await.0, "DONE");
    assert_eq!(receipt(world, "wf_pg_steady", fine).await.0, "DONE");
    assert_eq!(steady.handled.load(Ordering::SeqCst), 2);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_panic_in_an_external_consumer_is_that_events_failure(pool: PgPool) {
    let world = &World::new(pool).await;
    let (poison, fine) = two_tickets(world).await;
    let panics = PanicsFor::new("wf_ext_panics", poison);
    let mut registry = registry(world);
    registry.external.push(panics.clone());

    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();

    assert_eq!(
        receipt(world, "wf_ext_panics", poison).await,
        ("DEAD".into(), 3, Some("panic".into()))
    );
    assert_eq!(receipt(world, "wf_ext_panics", fine).await.0, "DONE");
    assert_eq!(panics.handled.load(Ordering::SeqCst), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn readyz_reports_a_backlog_as_degraded_and_keeps_answering_200(pool: PgPool) {
    let world = &World::new(pool).await;
    let health = penfii_core::health::ReceiptHealth::new();
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        health: Some(health.clone()),
        ..penfii_http_api::RouterConfig::new(world.pool.clone(), crate::common::limits(), false)
    });

    // Before the first reading the process says it does not know yet, and is not degraded for it.
    let (status, body, _) = call(&app, "GET", "/readyz", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["checks"]["workers"], "unknown");

    // Nothing waits: ok, with the numbers.
    health.refresh(&world.pool).await.unwrap();
    let (status, body, _) = call(&app, "GET", "/readyz", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["status"].as_str(), body["checks"]["workers"].as_str()),
        (Some("ok"), Some("ok"))
    );
    assert_eq!(body["receipts"]["dead"], 0);

    // A delivery that was given up on: degraded, still 200.
    let (poison, _) = two_tickets(world).await;
    let mut registry = registry(world);
    registry.pg.push(PanicsFor::new("wf_health", poison));
    penfii_core::events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    // The reading is the last one taken: the request does not read the receipts itself.
    let (_, body, _) = call(&app, "GET", "/readyz", None, None, None).await;
    assert_eq!(body["receipts"]["dead"], 0);
    health.refresh(&world.pool).await.unwrap();
    let (status, body, _) = call(&app, "GET", "/readyz", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["checks"]["workers"], "degraded");
    assert_eq!(body["receipts"]["dead"], 1);

    // A due receipt that has waited too long: degraded too, with the consumer named.
    sqlx::query("UPDATE penfii.consumer_receipts SET state = 'PENDING', attempts = 0, available_at = now() - interval '1 hour' WHERE consumer = 'wf_health' AND state = 'DEAD'")
        .execute(&world.pool)
        .await
        .unwrap();
    health.refresh(&world.pool).await.unwrap();
    let (status, body, _) = call(&app, "GET", "/readyz", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["receipts"]["dead"], 0);
    assert_eq!(body["receipts"]["consumers"][0]["consumer"], "wf_health");
    assert!(
        body["receipts"]["oldestWaitingSeconds"].as_i64().unwrap() >= 3500,
        "{body}"
    );
}
