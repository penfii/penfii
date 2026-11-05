//! Operator configuration of Docs, co-editing and their search entries
//!. Every value is an initial default for self-hosted installs, never a
//! product limit. `from_env` refuses to start on a value that is not a positive number or that
//! breaks the ordering `MATERIALIZE_MAX_WAIT >= MATERIALIZE_DEBOUNCE`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeConfig {
    pub materialize_debounce_ms: u64,
    pub materialize_max_wait_ms: u64,
    pub collab_persist_interval_ms: u64,
    pub collab_compact_updates: u64,
    pub collab_room_idle_seconds: u64,
    pub collab_max_rooms: u64,
    pub collab_max_connections_per_user_doc: u64,
    pub collab_max_update_bytes: u64,
    pub collab_max_state_bytes: u64,
    pub collab_auth_ttl_seconds: u64,
    pub doc_max_content_bytes: u64,
    pub doc_file_orphan_grace_seconds: u64,
    pub search_doc_body_max_bytes: u64,
    pub search_comments_max_bytes: u64,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            materialize_debounce_ms: 3_000,
            materialize_max_wait_ms: 15_000,
            collab_persist_interval_ms: 500,
            collab_compact_updates: 200,
            collab_room_idle_seconds: 60,
            collab_max_rooms: 200,
            collab_max_connections_per_user_doc: 5,
            collab_max_update_bytes: 1_048_576,
            collab_max_state_bytes: 16_777_216,
            collab_auth_ttl_seconds: 5,
            doc_max_content_bytes: 4_194_304,
            doc_file_orphan_grace_seconds: 3_600,
            search_doc_body_max_bytes: 524_288,
            search_comments_max_bytes: 51_200,
        }
    }
}

impl KnowledgeConfig {
    /// Reads the environment. An unset variable keeps its default; a set one must be a positive
    /// integer.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut config = Self::default();
        let fields: [(&str, &mut u64); 14] = [
            (
                "PENFII_DOC_MATERIALIZE_DEBOUNCE_MS",
                &mut config.materialize_debounce_ms,
            ),
            (
                "PENFII_DOC_MATERIALIZE_MAX_WAIT_MS",
                &mut config.materialize_max_wait_ms,
            ),
            (
                "PENFII_DOC_COLLAB_PERSIST_INTERVAL_MS",
                &mut config.collab_persist_interval_ms,
            ),
            (
                "PENFII_DOC_COLLAB_COMPACT_UPDATES",
                &mut config.collab_compact_updates,
            ),
            (
                "PENFII_DOC_COLLAB_ROOM_IDLE_SECONDS",
                &mut config.collab_room_idle_seconds,
            ),
            ("PENFII_DOC_COLLAB_MAX_ROOMS", &mut config.collab_max_rooms),
            (
                "PENFII_DOC_COLLAB_MAX_CONNECTIONS_PER_USER_DOC",
                &mut config.collab_max_connections_per_user_doc,
            ),
            (
                "PENFII_DOC_COLLAB_MAX_UPDATE_BYTES",
                &mut config.collab_max_update_bytes,
            ),
            (
                "PENFII_DOC_COLLAB_MAX_STATE_BYTES",
                &mut config.collab_max_state_bytes,
            ),
            (
                "PENFII_DOC_COLLAB_AUTH_TTL_SECONDS",
                &mut config.collab_auth_ttl_seconds,
            ),
            (
                "PENFII_DOC_MAX_CONTENT_BYTES",
                &mut config.doc_max_content_bytes,
            ),
            (
                "PENFII_DOC_FILE_ORPHAN_GRACE_SECONDS",
                &mut config.doc_file_orphan_grace_seconds,
            ),
            (
                "PENFII_SEARCH_DOC_BODY_MAX_BYTES",
                &mut config.search_doc_body_max_bytes,
            ),
            (
                "PENFII_SEARCH_COMMENTS_MAX_BYTES",
                &mut config.search_comments_max_bytes,
            ),
        ];
        for (name, slot) in fields {
            if let Some(raw) = lookup(name) {
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                match raw.parse::<u64>() {
                    Ok(value) if value > 0 => *slot = value,
                    _ => {
                        return Err(format!("{name} must be a positive integer, got {raw:?}"));
                    }
                }
            }
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.materialize_max_wait_ms < self.materialize_debounce_ms {
            return Err(
                "PENFII_DOC_MATERIALIZE_MAX_WAIT_MS must be at least PENFII_DOC_MATERIALIZE_DEBOUNCE_MS"
                    .into(),
            );
        }
        if self.collab_max_update_bytes > self.collab_max_state_bytes {
            return Err(
                "PENFII_DOC_COLLAB_MAX_UPDATE_BYTES must not exceed PENFII_DOC_COLLAB_MAX_STATE_BYTES"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn with(pairs: &[(&str, &str)]) -> Result<KnowledgeConfig, String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        KnowledgeConfig::from_lookup(|name| map.get(name).cloned())
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let config = with(&[]).unwrap();
        assert_eq!(config, KnowledgeConfig::default());
        assert_eq!(config.materialize_debounce_ms, 3000);
        assert_eq!(config.materialize_max_wait_ms, 15000);
        assert_eq!(config.collab_auth_ttl_seconds, 5);
        assert_eq!(config.doc_max_content_bytes, 4_194_304);
    }

    #[test]
    fn a_configured_value_is_used() {
        let config = with(&[
            ("PENFII_DOC_MATERIALIZE_DEBOUNCE_MS", "200"),
            ("PENFII_DOC_MATERIALIZE_MAX_WAIT_MS", "900"),
        ])
        .unwrap();
        assert_eq!(config.materialize_debounce_ms, 200);
        assert_eq!(config.materialize_max_wait_ms, 900);
    }

    #[test]
    fn invalid_values_fail_with_the_variable_name() {
        for (name, value) in [
            ("PENFII_DOC_COLLAB_MAX_ROOMS", "0"),
            ("PENFII_SEARCH_COMMENTS_MAX_BYTES", "-3"),
            ("PENFII_SEARCH_COMMENTS_MAX_BYTES", "lots"),
        ] {
            let error = with(&[(name, value)]).unwrap_err();
            assert!(error.contains(name), "{error}");
        }
        let error = with(&[
            ("PENFII_DOC_MATERIALIZE_DEBOUNCE_MS", "5000"),
            ("PENFII_DOC_MATERIALIZE_MAX_WAIT_MS", "1000"),
        ])
        .unwrap_err();
        assert!(error.contains("MAX_WAIT"), "{error}");
        let error = with(&[("PENFII_DOC_COLLAB_MAX_UPDATE_BYTES", "99999999999")]).unwrap_err();
        assert!(error.contains("MAX_UPDATE_BYTES"), "{error}");
    }
}
