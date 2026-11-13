use penfii_core::{Limits, connect, require_current_schema};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    // Every panic is logged with its place (a worker's panic also ends the process, see `lifecycle`).
    penfii_core::lifecycle::install_panic_hook();
    // A setting that is set and cannot be read stops the start here, with its name (see `config`): nothing is
    // ignored or replaced by a default. Empty is the same as not set.
    let lookup = &penfii_core::config::process_env;
    penfii_core::config::startup_checks(lookup)?;
    let database_url =
        penfii_core::config::get(lookup, "DATABASE_URL").ok_or("DATABASE_URL is required")?;
    let bind = penfii_core::config::get(lookup, "PENFII_BIND")
        .unwrap_or_else(|| "0.0.0.0:8080".to_string());
    let public_https = penfii_core::config::get(lookup, "PENFII_PUBLIC_URL")
        .is_some_and(|value| value.starts_with("https://"));
    // Secure is on by default behind an https public URL.
    let cookie_secure = match penfii_core::config::boolean(lookup, "PENFII_COOKIE_SECURE")? {
        Some(on) => {
            if public_https && !on {
                tracing::warn!("PENFII_PUBLIC_URL is https but PENFII_COOKIE_SECURE is off");
            }
            on
        }
        None => public_https,
    };
    penfii_http_api::validate_trusted_proxies()?;
    let webhook_kek = penfii_core::config::webhook_kek(lookup)?;
    let mut limits = Limits::from_env()?;
    limits.login_throttle = penfii_core::ThrottleLimits::from_env()?;
    let knowledge = penfii_core::knowledge::KnowledgeConfig::from_env()?;
    let pool = connect(&database_url).await?;
    // core-api never migrates: the schema is made by `admin-cli migrate` with the owner's
    // credential, and this only checks (reading the history) that it is the one this build needs.
    require_current_schema(&pool).await?;
    if penfii_core::config::boolean(lookup, "PENFII_DEV_SEED")?.unwrap_or(false) {
        let seeded = penfii_core::directory::seed_dev(&pool)
            .await
            .map_err(|error| format!("dev seed failed: {error:?}"))?;
        if seeded {
            println!("penfii dev directory seed applied");
        }
    }
    let objects = penfii_core::files::ObjectStore::from_env()?;
    if let Some(store) = &objects
        && let Err(error) = store.ensure_bucket().await
    {
        tracing::warn!("object storage bucket was not confirmed: {error}");
    }
    let search = match penfii_core::search::client::OsConfig::from_env()? {
        Some(config) => {
            let client = penfii_core::search::client::OsClient::new(config)
                .map_err(|error| format!("search client: {error}"))?;
            let prefix = client.config.prefix.clone();
            let engine = penfii_core::search::SearchEngine::with_limits(
                pool.clone(),
                client,
                penfii_core::search::docs::BuildLimits::from(&knowledge),
            );
            match engine.indexer.check_prefix().await {
                Ok(()) => tracing::info!(%prefix, "search index prefix"),
                // Starting anyway would write into another database's indices.
                Err(penfii_core::CoreError::Conflict {
                    code: "search_prefix_in_use",
                    ..
                }) => {
                    return Err(format!(
                        "OpenSearch index prefix {prefix:?} belongs to another database (or to this one before it was recreated); set PENFII_OPENSEARCH_INDEX_PREFIX, or delete the old {prefix}-* indices"
                    )
                    .into());
                }
                Err(error) => {
                    tracing::warn!(%prefix, ?error, "search index prefix was not checked")
                }
            }
            Some(engine)
        }
        None => {
            tracing::warn!("PENFII_OPENSEARCH_URL is not set: search answers 503 until it is");
            None
        }
    };
    // E-mail is off without PENFII_SMTP_HOST (or while its settings are incomplete); a value that
    // cannot be right stops the start here, like the other settings.
    let mailer = match penfii_core::mail::SmtpConfig::from_env()
        .map_err(|error| format!("SMTP configuration: {error}"))?
    {
        penfii_core::mail::MailSetup::On(config) => {
            tracing::info!(host = %config.host, port = config.port, "e-mail enabled");
            Some(std::sync::Arc::new(
                penfii_core::mail::Mailer::new(&config)
                    .map_err(|error| format!("SMTP configuration: {error}"))?,
            ))
        }
        penfii_core::mail::MailSetup::Off(Some(reason)) => {
            tracing::warn!("e-mail is off: {reason}");
            None
        }
        penfii_core::mail::MailSetup::Off(None) => {
            tracing::info!(
                "PENFII_SMTP_HOST is not set: e-mail is off, invitations are shared as links"
            );
            None
        }
    };
    let (changes, _) = tokio::sync::broadcast::channel(256);
    // Web Push is off without a VAPID key; a malformed key stops the start here.
    let push = match penfii_core::push::VapidConfig::from_env()
        .map_err(|error| format!("Web Push configuration: {error}"))?
    {
        Some(vapid) => {
            tracing::info!(public_key = %vapid.public_key(), "Web Push enabled");
            Some(
                penfii_core::push::PushService::new(pool.clone(), vapid)
                    .map_err(|error| format!("Web Push: {error}"))?,
            )
        }
        None => {
            tracing::info!("PENFII_VAPID_PRIVATE_KEY is not set: Web Push is off");
            None
        }
    };
    // Every background worker's handle, so shutdown can wait for them (see `stop_workers`).
    let mut workers: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    if penfii_core::config::boolean(lookup, "PENFII_WORKER")?.unwrap_or(true) {
        let registry = penfii_core::consumers::registry_with_push(
            search.as_ref(),
            Some(changes.clone()),
            push.clone(),
            objects.clone(),
        );
        workers.extend(penfii_core::events::spawn(pool.clone(), registry));
        workers.push(penfii_core::reminders::spawn(pool.clone()));
        workers.push(penfii_core::planning::spawn_archive_scheduler(
            pool.clone(),
            Some(changes.clone()),
        ));
        workers.push(penfii_core::oauth::retention::spawn(pool.clone()));
        workers.push(penfii_core::files::spawn_sweeper(
            pool.clone(),
            objects.clone(),
        ));
        workers.push(penfii_core::ticket_lifecycle::spawn_trash_sweeper(
            pool.clone(),
            objects.clone(),
        ));
        if let Some(engine) = &search {
            workers.push(penfii_core::search::spawn_maintenance(engine.clone()));
        }
    }
    // How far the event consumers are behind, for /readyz. Computed here even when this process runs no worker.
    let health = penfii_core::health::ReceiptHealth::new();
    workers.push(penfii_core::health::spawn(pool.clone(), health.clone()));
    let collab = penfii_core::docs::collab::CollabHub::new(pool.clone(), knowledge.clone());
    collab.spawn_owner(database_url.clone());
    let providers = penfii_core::identity::provider::AuthProviders::from_env(std::sync::Arc::new(
        penfii_core::identity::provider::HttpTransport::new(),
    ))?;
    penfii_core::identity::sync_providers(&pool, &providers)
        .await
        .map_err(|error| format!("auth provider sync failed: {error:?}"))?;
    // PENFII_MCP_ENABLED is off by default; turning it on without a valid PENFII_PUBLIC_URL stops
    // the process here.
    let oauth = penfii_core::oauth::OAuthConfig::from_env(std::sync::Arc::new(
        penfii_core::oauth::HttpClientFetcher::new(),
    ))
    .map_err(|error| format!("MCP configuration: {error}"))?;
    if let Some(config) = &oauth {
        tracing::info!(resource = %config.resource(), "MCP enabled");
    }
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        objects,
        search,
        changes: Some(changes),
        knowledge,
        collab: Some(collab.clone()),
        providers,
        oauth,
        push: push.clone(),
        mailer,
        health: Some(health),
        webhook_kek,
        ..penfii_http_api::RouterConfig::new(pool.clone(), limits, cookie_secure)
    });
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("penfii core-api listening on {bind}");
    // Shutdown runs in this order, each step bounded so the process ends inside the container's stop
    // grace period (10 s by default): the HTTP server stops taking connections and finishes the
    // requests in flight, then the background workers stop, then the co-editing rooms store what they
    // hold, and last the database pool closes. Every step that uses the database is before the last.
    let stop_http = std::sync::Arc::new(tokio::sync::Notify::new());
    let mut server = tokio::spawn({
        let stop_http = stop_http.clone();
        async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move { stop_http.notified().await })
            .await
        }
    });
    // A worker whose loop panicked asks the process to end (`lifecycle::fatal`): it shuts down in the same
    // order, then exits with an error, so the container runtime or another supervisor starts it again.
    let fatal_reason: Option<String> = tokio::select! {
        result = &mut server => {
            result??;
            None
        }
        () = wait_for_stop_signal() => {
            tracing::info!("shutting down: stopping HTTP");
            stop_http_server(&stop_http, &mut server).await?;
            None
        }
        reason = penfii_core::lifecycle::fatal() => {
            tracing::error!(%reason, "a background worker failed: shutting down so that the process is started again");
            stop_http_server(&stop_http, &mut server).await?;
            Some(reason)
        }
    };
    tracing::info!("shutting down: stopping background workers");
    stop_workers(workers).await;
    // The workers are what sends pushes, so none is started after this; the ones on their way get a
    // moment, then end, before the database pool closes (a push records its result there).
    if let Some(push) = &push {
        push.drain(PUSH_STOP).await;
    }
    tracing::info!("shutting down: flushing co-editing rooms");
    collab.shutdown().await;
    tracing::info!("shutting down: closing the database pool");
    pool.close().await;
    match fatal_reason {
        Some(reason) => Err(format!("a background worker failed: {reason}").into()),
        None => Ok(()),
    }
}

/// Stops taking connections and waits [`HTTP_DRAIN`] for the requests in flight; what is still running
/// after that is closed.
async fn stop_http_server(
    stop_http: &tokio::sync::Notify,
    server: &mut tokio::task::JoinHandle<Result<(), std::io::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    stop_http.notify_one();
    match tokio::time::timeout(HTTP_DRAIN, &mut *server).await {
        Ok(result) => result??,
        Err(_) => {
            tracing::warn!("shutdown: requests still running after {HTTP_DRAIN:?}, closing them");
            server.abort();
        }
    }
    Ok(())
}

/// How long the HTTP server may take to finish the requests it already had.
const HTTP_DRAIN: std::time::Duration = std::time::Duration::from_secs(4);
/// How long the pushes already on their way have after the workers stopped. What is left is cancelled.
/// The three bounds and the co-editing flush and pool close stay inside the container's 10 s stop grace.
const PUSH_STOP: std::time::Duration = std::time::Duration::from_secs(1);
/// How long the background workers have to finish the unit of work they are in.
const WORKER_STOP: std::time::Duration = std::time::Duration::from_secs(3);

/// Asks the background workers to stop (they finish one unit of work: a transaction or an idempotent
/// step, and start no other) and waits for them; what has not ended by the deadline is aborted. An
/// aborted worker drops its transaction, which rolls back: the outbox rows and leases it held are
/// found again by the next start.
async fn stop_workers(workers: Vec<tokio::task::JoinHandle<()>>) {
    penfii_core::lifecycle::request_stop();
    let deadline = tokio::time::Instant::now() + WORKER_STOP;
    for mut worker in workers {
        if tokio::time::timeout_at(deadline, &mut worker)
            .await
            .is_err()
        {
            tracing::warn!("shutdown: a background worker did not stop in time, aborting it");
            worker.abort();
            let _ = worker.await;
        }
    }
}

/// Waits for Ctrl-C or SIGTERM.
async fn wait_for_stop_signal() {
    let terminate = async {
        #[cfg(unix)]
        {
            if let Ok(mut signal) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                signal.recv().await;
                return;
            }
        }
        std::future::pending::<()>().await
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate => {}
    }
}
