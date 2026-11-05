use penfii_core::{Limits, apply_migrations, connect};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let database_url = std::env::var("DATABASE_URL")?;
    let bind = std::env::var("PENFII_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let public_https = std::env::var("PENFII_PUBLIC_URL")
        .ok()
        .is_some_and(|value| value.trim().starts_with("https://"));
    // Secure is on by default behind an https public URL.
    let cookie_secure = match std::env::var("PENFII_COOKIE_SECURE") {
        Ok(value) => {
            let on = value.trim() == "1";
            if public_https && !on {
                tracing::warn!("PENFII_PUBLIC_URL is https but PENFII_COOKIE_SECURE is off");
            }
            on
        }
        Err(_) => public_https,
    };
    penfii_http_api::validate_trusted_proxies()?;
    let mut limits = Limits::from_env();
    limits.login_throttle = penfii_core::ThrottleLimits::from_env()?;
    let knowledge = penfii_core::knowledge::KnowledgeConfig::from_env()?;
    let pool = connect(&database_url).await?;
    apply_migrations(&pool).await?;
    if std::env::var("PENFII_DEV_SEED")
        .ok()
        .is_some_and(|value| value.trim() == "1")
    {
        let seeded = penfii_core::directory::seed_dev(&pool)
            .await
            .map_err(|error| format!("dev seed failed: {error:?}"))?;
        if seeded {
            println!("penfii dev directory seed applied");
        }
    }
    let objects = penfii_core::files::ObjectStore::from_env();
    if let Some(store) = &objects
        && let Err(error) = store.ensure_bucket().await
    {
        tracing::warn!("object storage bucket was not confirmed: {error}");
    }
    let search = match penfii_core::search::client::OsConfig::from_env() {
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
    let (changes, _) = tokio::sync::broadcast::channel(256);
    if std::env::var("PENFII_WORKER").ok().as_deref() != Some("0") {
        let registry = penfii_core::consumers::registry(search.as_ref(), Some(changes.clone()));
        penfii_core::events::spawn(pool.clone(), registry);
        penfii_core::reminders::spawn(pool.clone());
        penfii_core::oauth::retention::spawn(pool.clone());
        penfii_core::files::spawn_sweeper(pool.clone(), objects.clone());
        if let Some(engine) = &search {
            penfii_core::search::spawn_maintenance(engine.clone());
        }
    }
    let collab = penfii_core::docs::collab::CollabHub::new(pool.clone(), knowledge.clone());
    collab.spawn_owner(database_url.clone());
    let providers = penfii_core::identity::provider::AuthProviders::from_env(std::sync::Arc::new(
        penfii_core::identity::provider::HttpTransport::new(),
    ));
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
        ..penfii_http_api::RouterConfig::new(pool, limits, cookie_secure)
    });
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("penfii core-api listening on {bind}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown(collab))
    .await?;
    Ok(())
}

/// Waits for Ctrl-C or SIGTERM, then lets every co-editing room store what it holds (a restart must
/// not lose the last seconds of an editing session).
async fn shutdown(collab: std::sync::Arc<penfii_core::docs::collab::CollabHub>) {
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
    tracing::info!("shutting down: flushing co-editing rooms");
    collab.shutdown().await;
}
