//! Index lifecycle: creation, live updates from events, and the full (re)build.

use std::sync::Arc;

use reqwest::Method;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::client::{OsClient, OsError};
use super::docs::{self, Built};
use super::mapping::{KINDS, MAPPING_VERSION, index_body};
use crate::error::CoreError;
use crate::events::{EventRow, ExternalConsumer, HandlerFuture};

pub fn map_os(error: OsError) -> CoreError {
    CoreError::Internal(error.code())
}

#[derive(Debug, Clone)]
pub struct KindState {
    pub kind: String,
    pub mapping_version: i32,
    pub live_index: Option<String>,
    pub building_index: Option<String>,
    pub phase: String,
    pub last_error_code: Option<String>,
}

#[derive(Clone)]
pub struct Indexer {
    pub pool: PgPool,
    pub client: OsClient,
    pub limits: docs::BuildLimits,
}

impl Indexer {
    pub fn new(pool: PgPool, client: OsClient) -> Self {
        Self {
            pool,
            client,
            limits: docs::BuildLimits::default(),
        }
    }

    /// The operator's limits for what one document may hold.
    pub fn with_limits(mut self, limits: docs::BuildLimits) -> Self {
        self.limits = limits;
        self
    }

    pub async fn states(&self) -> Result<Vec<KindState>, CoreError> {
        let mut out = Vec::new();
        for kind in KINDS {
            out.push(self.state(kind).await?);
        }
        Ok(out)
    }

    pub async fn state(&self, kind: &str) -> Result<KindState, CoreError> {
        sqlx::query(
            "INSERT INTO penfii.search_index_state(kind, mapping_version) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(kind)
        .bind(MAPPING_VERSION)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query(
            "SELECT kind, mapping_version, live_index, building_index, phase, last_error_code FROM penfii.search_index_state WHERE kind=$1",
        )
        .bind(kind)
        .fetch_one(&self.pool)
        .await?;
        Ok(KindState {
            kind: row.try_get("kind")?,
            mapping_version: row.try_get("mapping_version")?,
            live_index: row.try_get("live_index")?,
            building_index: row.try_get("building_index")?,
            phase: row.try_get("phase")?,
            last_error_code: row.try_get("last_error_code")?,
        })
    }

    /// Creates what is missing and marks kinds whose mapping is out of date. Safe to call often.
    pub async fn ensure(&self) -> Result<(), CoreError> {
        for kind in KINDS {
            let state = self.state(kind).await?;
            let stale = state.live_index.is_some() && state.mapping_version != MAPPING_VERSION;
            let needs_build =
                state.building_index.is_none() && (state.live_index.is_none() || stale);
            if needs_build {
                self.start_build(kind, state.live_index.is_some()).await?;
            } else if let Some(building) = &state.building_index {
                // The index may have been deleted underneath us; recreate it.
                let (status, _) = self
                    .client
                    .admin(Method::HEAD, &format!("/{building}"), None)
                    .await
                    .map_err(map_os)?;
                if status == 404 {
                    self.create_index(kind, building).await?;
                }
            }
        }
        Ok(())
    }

    /// Names this database to the indices it creates, so another database that uses the same
    /// prefix on the same OpenSearch can be told apart from it.
    pub async fn owner_id(&self) -> Result<String, CoreError> {
        sqlx::query(
            "INSERT INTO penfii.foundation_state(key)
             SELECT 'search_owner:' || gen_random_uuid()
             WHERE NOT EXISTS (SELECT 1 FROM penfii.foundation_state WHERE key LIKE 'search_owner:%')",
        )
        .execute(&self.pool)
        .await?;
        let key: String = sqlx::query_scalar(
            "SELECT min(key) FROM penfii.foundation_state WHERE key LIKE 'search_owner:%'",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(key.trim_start_matches("search_owner:").to_owned())
    }

    /// Fails when an index under this prefix belongs to another database. Indices made before
    /// owners were recorded carry none and are accepted. Nothing is ever deleted here.
    pub async fn check_prefix(&self) -> Result<(), CoreError> {
        let ours = self.owner_id().await?;
        let (status, body) = self
            .client
            .admin(
                Method::GET,
                &format!(
                    "/{}-v*/_mapping?filter_path=*.mappings._meta",
                    self.client.config.prefix
                ),
                None,
            )
            .await
            .map_err(map_os)?;
        if status == 404 {
            return Ok(());
        }
        if !(200..300).contains(&status) {
            return Err(CoreError::Internal("search_prefix_check".into()));
        }
        let foreign = body.as_object().into_iter().flatten().any(|(_, index)| {
            index["mappings"]["_meta"]["owner"]
                .as_str()
                .is_some_and(|owner| owner != ours)
        });
        if foreign {
            return Err(CoreError::conflict(
                "search_prefix_in_use",
                "these OpenSearch indices were made by another database, or by this one before it was recreated; set PENFII_OPENSEARCH_INDEX_PREFIX to a name of its own, or delete the old indices",
            ));
        }
        Ok(())
    }

    async fn create_index(&self, kind: &str, name: &str) -> Result<(), CoreError> {
        let mut index = index_body(kind);
        index["mappings"]["_meta"] = json!({"owner": self.owner_id().await?});
        let (status, body) = self
            .client
            .admin(Method::PUT, &format!("/{name}"), Some(&index))
            .await
            .map_err(map_os)?;
        if !(200..300).contains(&status) {
            let kind_of_error = body["error"]["type"].as_str().unwrap_or("unknown");
            if kind_of_error != "resource_already_exists_exception" {
                return Err(CoreError::Internal(format!(
                    "search_index_create_{kind_of_error}"
                )));
            }
        }
        Ok(())
    }

    async fn start_build(&self, kind: &str, has_live: bool) -> Result<(), CoreError> {
        let name = format!(
            "{}-v{}-{}-{}",
            self.client.config.prefix,
            MAPPING_VERSION,
            kind,
            chrono::Utc::now().format("%Y%m%d%H%M%S%3f")
        );
        self.create_index(kind, &name).await?;
        sqlx::query(
            "UPDATE penfii.search_index_state SET building_index=$2, phase=$3, checkpoint='{}'::jsonb, last_error_code=NULL, updated_at=now() WHERE kind=$1",
        )
        .bind(kind)
        .bind(&name)
        .bind(if has_live { "STALE_MAPPING" } else { "BUILDING" })
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Starts a rebuild of a kind that is ready (used by `admin-cli search reindex`).
    pub async fn request_reindex(&self, kind: &str) -> Result<(), CoreError> {
        let state = self.state(kind).await?;
        if state.building_index.is_none() {
            self.start_build(kind, state.live_index.is_some()).await?;
        }
        Ok(())
    }

    /// Writes documents to the live index and to the index being built, if any.
    pub async fn write(&self, kind: &str, built: Vec<Built>) -> Result<(), CoreError> {
        if built.is_empty() {
            return Ok(());
        }
        let state = self.state(kind).await?;
        let targets: Vec<String> = state
            .live_index
            .iter()
            .chain(state.building_index.iter())
            .cloned()
            .collect();
        if targets.is_empty() {
            return Err(CoreError::Internal("search_index_missing".into()));
        }
        let mut lines = String::new();
        for target in &targets {
            for item in &built {
                let version = item.version;
                match &item.doc {
                    Some(doc) => {
                        lines.push_str(&json!({"index": {"_index": target, "_id": item.id, "version": version, "version_type": "external_gte"}}).to_string());
                        lines.push('\n');
                        lines.push_str(&doc.to_string());
                        lines.push('\n');
                    }
                    None => {
                        lines.push_str(&json!({"delete": {"_index": target, "_id": item.id, "version": version, "version_type": "external_gte"}}).to_string());
                        lines.push('\n');
                    }
                }
            }
        }
        let response = self.client.bulk(lines).await.map_err(map_os)?;
        let failed = bulk_failures(&response);
        if failed > 0 {
            return Err(CoreError::Internal(format!(
                "search_bulk_items_failed_{failed}"
            )));
        }
        Ok(())
    }

    /// Rebuilds the documents for a set of ids from PostgreSQL and writes them.
    pub async fn refresh(&self, kind: &str, ids: &[Uuid]) -> Result<(), CoreError> {
        let mut conn = self.pool.acquire().await?;
        let built = docs::build(&mut conn, kind, ids, &self.limits).await?;
        drop(conn);
        self.write(kind, built).await
    }

    /// What an event changes in the index: (kind, ids).
    async fn targets(&self, event: &EventRow) -> Result<Vec<(&'static str, Vec<Uuid>)>, CoreError> {
        Ok(match event.target_kind.as_str() {
            "TICKET" => {
                // Trash, restore, archive and purge change which of its files are searchable.
                let files: Vec<Uuid> = sqlx::query_scalar(
                    "SELECT file_id FROM penfii.ticket_attachments WHERE ticket_id=$1",
                )
                .bind(event.target_id)
                .fetch_all(&self.pool)
                .await?;
                let mut out = vec![("ticket", vec![event.target_id])];
                if !files.is_empty() {
                    out.push(("file", files));
                }
                out
            }
            "MESSAGE" => {
                let files: Vec<Uuid> = sqlx::query_scalar(
                    "SELECT file_id FROM penfii.message_attachments WHERE message_id=$1",
                )
                .bind(event.target_id)
                .fetch_all(&self.pool)
                .await?;
                let mut out = vec![("message", vec![event.target_id])];
                if !files.is_empty() {
                    out.push(("file", files));
                }
                out
            }
            "FILE" => vec![("file", vec![event.target_id])],
            "CHANNEL" | "PROJECT" | "TEAM" => vec![("space", vec![event.target_id])],
            "VIEW" => vec![("view", vec![event.target_id])],
            "DOC" => vec![("doc", vec![event.target_id])],
            _ => Vec::new(),
        })
    }

    pub async fn apply(&self, event: &EventRow) -> Result<(), CoreError> {
        for (kind, ids) in self.targets(event).await? {
            self.refresh(kind, &ids).await?;
        }
        Ok(())
    }

    /// Builds every kind that has an index under construction. Returns kinds finished.
    pub async fn backfill_all(&self) -> Result<Vec<String>, CoreError> {
        self.ensure().await?;
        let mut finished = Vec::new();
        for kind in KINDS {
            let state = self.state(kind).await?;
            if state.building_index.is_some() {
                match self.backfill(kind).await {
                    Ok(()) => finished.push((*kind).to_owned()),
                    Err(error) => {
                        let code = match &error {
                            CoreError::Internal(code) => code.clone(),
                            _ => "search_backfill_failed".into(),
                        };
                        sqlx::query("UPDATE penfii.search_index_state SET last_error_code=$2, updated_at=now() WHERE kind=$1")
                            .bind(kind)
                            .bind(&code)
                            .execute(&self.pool)
                            .await?;
                        return Err(error);
                    }
                }
            }
        }
        Ok(finished)
    }

    /// Scans PostgreSQL, bulk-loads the index under construction, verifies it and promotes it.
    /// Re-running is harmless: ids are deterministic and writes carry a version.
    pub async fn backfill(&self, kind: &str) -> Result<(), CoreError> {
        let state = self.state(kind).await?;
        let Some(building) = state.building_index.clone() else {
            return Ok(());
        };
        let checkpoint: Value =
            sqlx::query_scalar("SELECT checkpoint FROM penfii.search_index_state WHERE kind=$1")
                .bind(kind)
                .fetch_one(&self.pool)
                .await?;
        let mut after: Option<Uuid> = checkpoint["after"].as_str().and_then(|s| s.parse().ok());
        // The source keeps changing while we scan. The finished index must hold a count that lies
        // between the source count before the scan and after it.
        let expected_before = {
            let mut conn = self.pool.acquire().await?;
            docs::source_count(&mut conn, kind).await?
        };
        loop {
            let mut conn = self.pool.acquire().await?;
            let ids = docs::scan(&mut conn, kind, after, 500).await?;
            if ids.is_empty() {
                break;
            }
            let built = docs::build(&mut conn, kind, &ids, &self.limits).await?;
            drop(conn);
            self.write_one_index(&building, built).await?;
            after = ids.last().copied();
            sqlx::query("UPDATE penfii.search_index_state SET checkpoint=$2, updated_at=now() WHERE kind=$1")
                .bind(kind)
                .bind(json!({"after": after}))
                .execute(&self.pool)
                .await?;
        }
        // Verify: the same number of documents as indexable rows.
        self.client
            .admin(Method::POST, &format!("/{building}/_refresh"), None)
            .await
            .map_err(map_os)?;
        let (_, count) = self
            .client
            .admin(Method::GET, &format!("/{building}/_count"), None)
            .await
            .map_err(map_os)?;
        let indexed = count["count"].as_i64().unwrap_or(-1);
        let mut conn = self.pool.acquire().await?;
        let expected_after = docs::source_count(&mut conn, kind).await?;
        drop(conn);
        let (low, high) = (
            expected_before.min(expected_after),
            expected_before.max(expected_after),
        );
        if indexed < low || indexed > high {
            // Live events may land between the scan and the count; look once more.
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            self.client
                .admin(Method::POST, &format!("/{building}/_refresh"), None)
                .await
                .map_err(map_os)?;
            let (_, count) = self
                .client
                .admin(Method::GET, &format!("/{building}/_count"), None)
                .await
                .map_err(map_os)?;
            let again = count["count"].as_i64().unwrap_or(-1);
            let mut conn = self.pool.acquire().await?;
            let now = docs::source_count(&mut conn, kind).await?;
            drop(conn);
            let (low, high) = (low.min(now), high.max(now));
            if again < low || again > high {
                return Err(CoreError::Internal(format!(
                    "search_count_mismatch_{again}_{low}_{high}"
                )));
            }
        }
        self.promote(kind, &building).await
    }

    async fn write_one_index(&self, index: &str, built: Vec<Built>) -> Result<(), CoreError> {
        let mut lines = String::new();
        for item in &built {
            if let Some(doc) = &item.doc {
                let version = item.version;
                lines.push_str(&json!({"index": {"_index": index, "_id": item.id, "version": version, "version_type": "external_gte"}}).to_string());
                lines.push('\n');
                lines.push_str(&doc.to_string());
                lines.push('\n');
            }
        }
        if lines.is_empty() {
            return Ok(());
        }
        let mut last = 0;
        for _ in 0..3 {
            let response = self.client.bulk(lines.clone()).await.map_err(map_os)?;
            last = bulk_failures(&response);
            if last == 0 {
                return Ok(());
            }
        }
        Err(CoreError::Internal(format!(
            "search_bulk_items_failed_{last}"
        )))
    }

    async fn promote(&self, kind: &str, building: &str) -> Result<(), CoreError> {
        let state = self.state(kind).await?;
        let alias = self.client.alias(kind);
        let mut actions = Vec::new();
        if let Some(old) = &state.live_index {
            actions.push(json!({"remove": {"index": old, "alias": alias}}));
        }
        actions.push(json!({"add": {"index": building, "alias": alias}}));
        let (status, body) = self
            .client
            .admin(
                Method::POST,
                "/_aliases",
                Some(&json!({"actions": actions})),
            )
            .await
            .map_err(map_os)?;
        if !(200..300).contains(&status) {
            return Err(CoreError::Internal(format!(
                "search_alias_{}",
                body["error"]["type"].as_str().unwrap_or("unknown")
            )));
        }
        sqlx::query(
            "UPDATE penfii.search_index_state SET live_index=$2, building_index=NULL, phase='READY', mapping_version=$3, checkpoint='{}'::jsonb, last_error_code=NULL, updated_at=now() WHERE kind=$1",
        )
        .bind(kind)
        .bind(building)
        .bind(MAPPING_VERSION)
        .execute(&self.pool)
        .await?;
        // Keep one previous generation; remove older ones.
        let pattern = format!("{}-v*-{}-*", self.client.config.prefix, kind);
        if let Ok((200, listing)) = self
            .client
            .admin(
                Method::GET,
                &format!("/_cat/indices/{pattern}?format=json&h=index"),
                None,
            )
            .await
            && let Some(items) = listing.as_array()
        {
            let mut names: Vec<String> = items
                .iter()
                .filter_map(|i| i["index"].as_str().map(str::to_owned))
                .filter(|n| n != building && Some(n) != state.live_index.as_ref())
                .collect();
            names.sort();
            for name in names {
                let _ = self
                    .client
                    .admin(Method::DELETE, &format!("/{name}"), None)
                    .await;
            }
        }
        Ok(())
    }

    pub async fn delete_everything_for_tests(&self) {
        let _ = self
            .client
            .admin(
                Method::DELETE,
                &format!("/{}-*", self.client.config.prefix),
                None,
            )
            .await;
        let _ = sqlx::query("DELETE FROM penfii.search_index_state")
            .execute(&self.pool)
            .await;
    }
}

fn bulk_failures(response: &Value) -> usize {
    if response["errors"] != json!(true) {
        return 0;
    }
    response["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    let (_, body) = item
                        .as_object()
                        .and_then(|o| o.iter().next())
                        .unwrap_or((&String::new(), &Value::Null));
                    let status = body["status"].as_i64().unwrap_or(0);
                    !((200..300).contains(&status) || status == 404 || status == 409)
                })
                .count()
        })
        .unwrap_or(1)
}

/// The Search consumer: it only learns *which object changed* and rebuilds that object's
/// document from PostgreSQL, so create, edit, restore and a repeated event are the same step.
pub struct SearchConsumer {
    pub indexer: Arc<Indexer>,
}

impl ExternalConsumer for SearchConsumer {
    fn name(&self) -> &'static str {
        "search"
    }

    fn accepts(&self, kind: &str) -> bool {
        if kind.starts_with("ticket.") {
            return !(kind.starts_with("ticket.dependency_")
                || kind.starts_with("ticket.related_")
                || kind == "ticket.sprint_changed"
                || kind == "ticket.milestones_changed");
        }
        if kind.starts_with("doc.") {
            // Visibility, participants and reference rows are decided in PostgreSQL at read time;
            // nothing in the index depends on them.
            return !matches!(
                kind,
                "doc.visibility_changed"
                    | "doc.participants_changed"
                    | "doc.targets_changed"
                    | "doc.version_created"
            );
        }
        [
            "message.", "file.", "channel.", "project.", "team.", "view.",
        ]
        .iter()
        .any(|prefix| kind.starts_with(prefix))
    }

    fn handle<'a>(&'a self, _pool: &'a PgPool, event: &'a EventRow) -> HandlerFuture<'a> {
        Box::pin(async move { self.indexer.apply(event).await })
    }
}
