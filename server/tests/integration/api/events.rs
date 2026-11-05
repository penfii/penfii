//! The event outbox, the dispatcher and its receipts.

use super::{World, call, make_ticket, new_project, workspace};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

mod p62_probe {
    use penfii_core::CoreError;
    use penfii_core::events::{EventRow, ExternalConsumer, HandlerFuture, PgConsumer};
    use sqlx::{PgConnection, PgPool};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Writes one row per event (idempotent) and can be told to fail the first N times.
    pub struct Probe {
        pub name: &'static str,
        pub fail_first: AtomicUsize,
        pub always_fail: bool,
        pub calls: AtomicUsize,
    }

    impl Probe {
        pub fn new(name: &'static str, fail_first: usize, always_fail: bool) -> Self {
            Self {
                name,
                fail_first: AtomicUsize::new(fail_first),
                always_fail,
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl PgConsumer for Probe {
        fn name(&self) -> &'static str {
            self.name
        }
        fn accepts(&self, kind: &str) -> bool {
            kind.starts_with("ticket.")
        }
        fn handle<'a>(
            &'a self,
            conn: &'a mut PgConnection,
            event: &'a EventRow,
        ) -> HandlerFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                sqlx::query("INSERT INTO penfii.p62_probe(consumer,event_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
                    .bind(self.name)
                    .bind(event.event_id)
                    .execute(&mut *conn)
                    .await?;
                if self.always_fail {
                    return Err(CoreError::validation("probe always fails"));
                }
                if self.fail_first.load(Ordering::SeqCst) > 0 {
                    self.fail_first.fetch_sub(1, Ordering::SeqCst);
                    // The effect above must be rolled back with the failure.
                    return Err(CoreError::validation("probe fails once"));
                }
                Ok(())
            })
        }
    }

    pub struct ExternalProbe {
        pub name: &'static str,
        pub calls: AtomicUsize,
    }

    impl ExternalConsumer for ExternalProbe {
        fn name(&self) -> &'static str {
            self.name
        }
        fn accepts(&self, kind: &str) -> bool {
            kind.starts_with("ticket.")
        }
        fn handle<'a>(&'a self, pool: &'a PgPool, event: &'a EventRow) -> HandlerFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                sqlx::query("INSERT INTO penfii.p62_probe(consumer,event_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
                    .bind(self.name)
                    .bind(event.event_id)
                    .execute(pool)
                    .await?;
                Ok(())
            })
        }
    }
}

async fn probe_table(pool: &PgPool) {
    sqlx::query("CREATE TABLE IF NOT EXISTS penfii.p62_probe(consumer text NOT NULL, event_id uuid NOT NULL, PRIMARY KEY(consumer,event_id))")
        .execute(pool)
        .await
        .unwrap();
}

async fn probe_count(pool: &PgPool, consumer: &str, workspace_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM penfii.p62_probe p JOIN penfii.domain_events e ON e.event_id=p.event_id WHERE p.consumer=$1 AND e.workspace_id=$2")
        .bind(consumer)
        .bind(workspace_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub(super) async fn receipt_states(
    pool: &PgPool,
    consumer: &str,
    workspace_id: Uuid,
) -> Vec<(String, i32)> {
    sqlx::query_as("SELECT r.state, r.attempts FROM penfii.consumer_receipts r JOIN penfii.domain_events e ON e.event_id=r.event_id WHERE r.consumer=$1 AND e.workspace_id=$2 ORDER BY e.seq")
        .bind(consumer)
        .bind(workspace_id)
        .fetch_all(pool)
        .await
        .unwrap()
}

fn fast_registry(workspace_id: Uuid) -> penfii_core::events::Registry {
    penfii_core::events::Registry {
        backoff_base_secs: 0,
        max_backoff_secs: 0,
        max_attempts: 3,
        workspace: Some(workspace_id),
        ..Default::default()
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_ticket_command_records_one_event_with_before_and_after_and_a_rolled_back_command_none(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("p62-ev-{}", Uuid::new_v4())).await;
    let (project_id, workflow) = new_project(world, workspace_id, "Events").await;
    let events = |kind: &'static str| {
        let pool = world.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM penfii.domain_events WHERE workspace_id=$1 AND event_kind=$2",
            )
            .bind(workspace_id)
            .bind(kind)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let (status, ticket) = make_ticket(
        world,
        &project_id,
        "p62-ev-1",
        json!({"title":"Event ticket"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    assert_eq!(events("ticket.created").await, 1);
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.outbox o JOIN penfii.domain_events e ON e.event_id=o.event_id WHERE e.workspace_id=$1 AND o.dispatched_at IS NULL AND e.event_kind='ticket.created'")
        .bind(workspace_id).fetch_one(&world.pool).await.unwrap();
    assert_eq!(
        outbox, 1,
        "the outbox row is written in the same transaction"
    );
    // A refused command leaves no event behind.
    let (status, _) = make_ticket(world, &project_id, "p62-ev-bad", json!({"title":""})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(events("ticket.created").await, 1);
    // A State change records the filterable fields before and after.
    let done = workflow["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "DONE")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = ticket["id"].as_str().unwrap();
    let (status, moved, _) = call(
        &world.app,
        "POST",
        &format!("/v1/tickets/{id}/commands/move"),
        Some(&world.owner_cookie),
        Some("p62-ev-move"),
        Some(json!({"expectedRevision":1,"statusId":done})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    let facts: Value = sqlx::query_scalar("SELECT facts FROM penfii.domain_events WHERE workspace_id=$1 AND event_kind='ticket.status_changed'")
        .bind(workspace_id).fetch_one(&world.pool).await.unwrap();
    assert_eq!(facts["before"]["statusCategory"], "NOT_STARTED", "{facts}");
    assert_eq!(facts["after"]["statusCategory"], "DONE", "{facts}");
    assert_eq!(facts["after"]["statusId"], done);
    assert!(
        facts["before"].get("labelIds").is_some() && facts["after"].get("assigneeId").is_some()
    );
    // Personal actions emit nothing.
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.domain_events WHERE workspace_id=$1")
            .bind(workspace_id)
            .fetch_one(&world.pool)
            .await
            .unwrap();
    let (status, _, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/workspaces/{workspace_id}/favorites"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"kind":"PROJECT","targetId":project_id})),
    )
    .await;
    assert!(status.is_success(), "favorite added: {status}");
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.domain_events WHERE workspace_id=$1")
            .bind(workspace_id)
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_dispatcher_survives_failures_duplicates_crashes_and_parallel_runs(pool: PgPool) {
    use penfii_core::events;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    let world = &World::new(pool).await;
    probe_table(&world.pool).await;
    let workspace_id = workspace(world, &format!("p62-disp-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, workspace_id, "Dispatch").await;
    for n in 0..4 {
        let (status, v) = make_ticket(
            world,
            &project_id,
            &format!("p62-d-{n}"),
            json!({"title":format!("T{n}")}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
    }
    let healthy = Arc::new(p62_probe::Probe::new("healthy", 0, false));
    let flaky = Arc::new(p62_probe::Probe::new("flaky", 1, false));
    let broken = Arc::new(p62_probe::Probe::new("broken", 0, true));
    let external = Arc::new(p62_probe::ExternalProbe {
        name: "external",
        calls: 0.into(),
    });
    let mut registry = fast_registry(workspace_id);
    registry.pg = vec![healthy.clone(), flaky.clone(), broken.clone()];
    registry.external = vec![external.clone()];

    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();

    // Isolation: the broken consumer is dead after its attempts; the others finished everything.
    assert_eq!(probe_count(&world.pool, "healthy", workspace_id).await, 4);
    assert_eq!(probe_count(&world.pool, "external", workspace_id).await, 4);
    assert!(
        receipt_states(&world.pool, "healthy", workspace_id)
            .await
            .iter()
            .all(|(s, _)| s == "DONE")
    );
    assert!(
        receipt_states(&world.pool, "broken", workspace_id)
            .await
            .iter()
            .all(|(s, a)| s == "DEAD" && *a == 3)
    );
    // A failure rolls the effect back, then the retry succeeds: one row per event, no more.
    assert_eq!(probe_count(&world.pool, "flaky", workspace_id).await, 4);
    let flaky_states = receipt_states(&world.pool, "flaky", workspace_id).await;
    assert!(
        flaky_states.iter().all(|(s, _)| s == "DONE"),
        "{flaky_states:?}"
    );
    assert_eq!(
        flaky_states.iter().map(|(_, a)| *a).sum::<i32>(),
        1,
        "exactly one failed attempt"
    );

    // Duplicate delivery: finished receipts are not run again, and a forced re-run is harmless.
    let calls = healthy.calls.load(Ordering::SeqCst);
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(healthy.calls.load(Ordering::SeqCst), calls);
    sqlx::query("UPDATE penfii.consumer_receipts r SET state='PENDING' FROM penfii.domain_events e WHERE e.event_id=r.event_id AND e.workspace_id=$1 AND r.consumer='healthy'")
        .bind(workspace_id).execute(&world.pool).await.unwrap();
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(
        probe_count(&world.pool, "healthy", workspace_id).await,
        4,
        "same result when delivered twice"
    );

    // Crash: the process died after claiming. An expired lease is taken over; a live lease is not.
    let (_, v) = make_ticket(world, &project_id, "p62-d-crash", json!({"title":"Crash"})).await;
    let ticket_id: Uuid = v["id"].as_str().unwrap().parse().unwrap();
    events::fan_out(&world.pool, &registry, 100).await.unwrap();
    sqlx::query("UPDATE penfii.consumer_receipts r SET state='RUNNING', lease_token=gen_random_uuid(), lease_until=now()+interval '1 hour' FROM penfii.domain_events e WHERE e.event_id=r.event_id AND e.target_id=$1 AND r.consumer='external'")
        .bind(ticket_id).execute(&world.pool).await.unwrap();
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(
        probe_count(&world.pool, "external", workspace_id).await,
        4,
        "a live lease is left alone"
    );
    sqlx::query("UPDATE penfii.consumer_receipts r SET lease_until=now()-interval '1 second' FROM penfii.domain_events e WHERE e.event_id=r.event_id AND e.target_id=$1 AND r.consumer='external'")
        .bind(ticket_id).execute(&world.pool).await.unwrap();
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(
        probe_count(&world.pool, "external", workspace_id).await,
        5,
        "the expired lease was taken over"
    );

    // Two dispatchers at once handle every receipt exactly once.
    for n in 0..6 {
        make_ticket(
            world,
            &project_id,
            &format!("p62-d-par-{n}"),
            json!({"title":format!("P{n}")}),
        )
        .await;
    }
    let (a, b) = tokio::join!(
        events::run_until_idle(&world.pool, &registry),
        events::run_until_idle(&world.pool, &registry)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(probe_count(&world.pool, "healthy", workspace_id).await, 11);
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.consumer_receipts r JOIN penfii.domain_events e ON e.event_id=r.event_id WHERE e.workspace_id=$1 AND r.consumer='healthy' AND r.state='DONE'")
        .bind(workspace_id).fetch_one(&world.pool).await.unwrap();
    assert_eq!(total, 11);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn dead_deliveries_are_listed_and_requeued_safely(pool: PgPool) {
    use penfii_core::events;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    let world = &World::new(pool).await;
    probe_table(&world.pool).await;
    let ws = workspace(world, &format!("p62-dead-{}", Uuid::new_v4())).await;
    let (project_id, _) = new_project(world, ws, "Dead").await;
    for n in 0..2 {
        make_ticket(
            world,
            &project_id,
            &format!("p62-dead-{n}"),
            json!({"title":format!("D{n}")}),
        )
        .await;
    }
    let revive = Arc::new(p62_probe::Probe::new("revive", 1000, false));
    let mut registry = fast_registry(ws);
    registry.pg = vec![revive.clone()];
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    let states = receipt_states(&world.pool, "revive", ws).await;
    let total = states.len();
    assert!(
        total >= 2 && states.iter().all(|(s, a)| s == "DEAD" && *a == 3),
        "{states:?}"
    );
    assert_eq!(probe_count(&world.pool, "revive", ws).await, 0);
    let listed = events::dead_receipts(&world.pool, 1000).await.unwrap();
    assert!(listed.iter().filter(|d| d.consumer == "revive").count() >= total);
    assert!(
        listed
            .iter()
            .any(|d| d.last_error_code.as_deref() == Some("validation"))
    );

    // Another consumer's name changes nothing; the right name gives fresh attempts.
    assert_eq!(
        events::requeue_dead(&world.pool, Some("nobody"), None)
            .await
            .unwrap(),
        0
    );
    assert!(
        receipt_states(&world.pool, "revive", ws)
            .await
            .iter()
            .all(|(s, _)| s == "DEAD")
    );
    revive.fail_first.store(0, Ordering::SeqCst);
    let one = events::dead_receipts(&world.pool, 1000)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.consumer == "revive")
        .unwrap();
    assert_eq!(
        events::requeue_dead(&world.pool, Some("revive"), Some(one.event_id))
            .await
            .unwrap(),
        1,
        "one event only"
    );
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(probe_count(&world.pool, "revive", ws).await, 1);
    let rest = events::requeue_dead(&world.pool, Some("revive"), None)
        .await
        .unwrap();
    assert_eq!(rest as usize, total - 1);
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(probe_count(&world.pool, "revive", ws).await as usize, total);
    assert!(
        receipt_states(&world.pool, "revive", ws)
            .await
            .iter()
            .all(|(s, _)| s == "DONE")
    );

    // Done work is never requeued, and a second run writes nothing more.
    assert_eq!(
        events::requeue_dead(&world.pool, Some("revive"), None)
            .await
            .unwrap(),
        0
    );
    events::run_until_idle(&world.pool, &registry)
        .await
        .unwrap();
    assert_eq!(probe_count(&world.pool, "revive", ws).await as usize, total);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_team_made_by_the_dev_directory_seed_records_team_created_once(pool: PgPool) {
    let world = &World::new(pool).await;
    penfii_core::directory::seed_dev(&world.pool).await.unwrap();
    penfii_core::directory::seed_dev(&world.pool).await.unwrap();
    let teams: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM penfii.teams WHERE name='Product'")
        .fetch_all(&world.pool)
        .await
        .unwrap();
    assert!(!teams.is_empty(), "the seed makes the Product Team");
    for team in teams {
        let events: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.domain_events WHERE target_kind='TEAM' AND target_id=$1 AND event_kind='team.created'").bind(team).fetch_one(&world.pool).await.unwrap();
        assert_eq!(events, 1, "team {team}");
        let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.outbox o JOIN penfii.domain_events e ON e.event_id=o.event_id WHERE e.target_id=$1").bind(team).fetch_one(&world.pool).await.unwrap();
        assert_eq!(outbox, 1);
    }
}
