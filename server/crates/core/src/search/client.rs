//! A small OpenSearch HTTP client. Nothing outside `search` knows about index names or the query DSL.

use std::time::Duration;

pub use reqwest::Method;
use serde_json::Value;

#[derive(Debug)]
pub enum OsError {
    /// Could not connect, timed out, or the server answered 5xx.
    Unavailable(String),
    /// The server understood and refused (4xx). Carries the status and the error type only.
    Rejected(u16, String),
}

impl OsError {
    pub fn code(&self) -> String {
        match self {
            OsError::Unavailable(_) => "search_unavailable".into(),
            OsError::Rejected(status, kind) => format!("search_rejected_{status}_{kind}"),
        }
    }
}

/// What Local use gets when `PENFII_OPENSEARCH_INDEX_PREFIX` is not set. Every other
/// environment sets its own, so two environments never share index names.
pub const DEFAULT_PREFIX: &str = "penfii-dev";

/// Lower case letters, digits, `-` and `_`, starting with a letter or digit: valid in an index name.
pub fn valid_prefix(prefix: &str) -> bool {
    let mut chars = prefix.chars();
    prefix.len() <= 40
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[derive(Clone)]
pub struct OsConfig {
    pub url: String,
    pub user: Option<String>,
    pub password: Option<String>,
    pub prefix: String,
    pub search_timeout: Duration,
    pub bulk_timeout: Duration,
}

impl OsConfig {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("PENFII_OPENSEARCH_URL").ok()?;
        let url = url.trim().trim_end_matches('/').to_owned();
        if url.is_empty() {
            return None;
        }
        Some(Self {
            url,
            user: std::env::var("PENFII_OPENSEARCH_USER").ok(),
            password: std::env::var("PENFII_OPENSEARCH_PASSWORD").ok(),
            prefix: std::env::var("PENFII_OPENSEARCH_INDEX_PREFIX")
                .ok()
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_PREFIX.into()),
            search_timeout: Duration::from_secs(5),
            bulk_timeout: Duration::from_secs(30),
        })
    }

    pub fn with_prefix(url: &str, prefix: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_owned(),
            user: None,
            password: None,
            prefix: prefix.to_owned(),
            search_timeout: Duration::from_secs(5),
            bulk_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone)]
pub struct OsClient {
    http: reqwest::Client,
    pub config: OsConfig,
}

impl OsClient {
    pub fn new(config: OsConfig) -> Result<Self, String> {
        if !valid_prefix(&config.prefix) {
            return Err(format!(
                "PENFII_OPENSEARCH_INDEX_PREFIX {:?} must be 1 to 40 characters of a-z, 0-9, '-' or '_'",
                config.prefix
            ));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { http, config })
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(u16, Value), OsError> {
        self.send(
            method,
            path,
            body.map(|b| b.to_string()),
            "application/json",
            timeout,
        )
        .await
    }

    pub async fn search_request(&self, path: &str, body: &Value) -> Result<(u16, Value), OsError> {
        self.request(Method::POST, path, Some(body), self.config.search_timeout)
            .await
    }

    pub async fn admin(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), OsError> {
        self.request(method, path, body, self.config.bulk_timeout)
            .await
    }

    pub async fn bulk(&self, ndjson: String) -> Result<Value, OsError> {
        let (_, value) = self
            .send(
                Method::POST,
                "/_bulk",
                Some(ndjson),
                "application/x-ndjson",
                self.config.bulk_timeout,
            )
            .await?;
        Ok(value)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        content_type: &str,
        timeout: Duration,
    ) -> Result<(u16, Value), OsError> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.config.url))
            .timeout(timeout);
        if let (Some(user), Some(password)) = (&self.config.user, &self.config.password) {
            request = request.basic_auth(user, Some(password));
        }
        if let Some(body) = body {
            request = request.header("content-type", content_type).body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| OsError::Unavailable(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|e| OsError::Unavailable(e.to_string()))?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status >= 500 {
            return Err(OsError::Unavailable(format!("status {status}")));
        }
        Ok((status, value))
    }

    pub fn alias(&self, kind: &str) -> String {
        format!("{}-{kind}", self.config.prefix)
    }
}
