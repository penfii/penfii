//! MCP OAuth Authorization Server and Workspace MCP Policy. Penfii is the Authorization Server for
//! exactly one Resource Server, itself: `${PENFII_PUBLIC_URL}/mcp`.
//!
//! Nothing here reads provider tokens and nothing here decides what a person may
//! see; the token only carries identity and a scope ceiling.

pub mod client;
pub mod policy;
pub mod retention;
pub mod scopes;
pub mod server;

use std::sync::Arc;

use chrono::Duration;
use serde_json::{Value, json};

pub use client::{ClientFetcher, HttpClientFetcher};
pub use policy::{McpPolicy, PolicyDefaults, PolicyInput, RiskClass};
pub use server::{
    AuthorizeParams, AuthorizeResult, ConsentView, Decision, GrantInfo, OAuthError, Principal,
    TokenRequest, TokenResponse,
};

/// Everything the OAuth endpoints and `/mcp` need to know about this instance.
#[derive(Clone)]
pub struct OAuthConfig {
    /// `PENFII_PUBLIC_URL` without a trailing slash; the issuer and the base of every URL.
    pub public_url: String,
    pub fetcher: Arc<dyn ClientFetcher>,
    pub access_ttl: Duration,
    pub refresh_idle_ttl: Duration,
    pub refresh_absolute_ttl: Duration,
    /// Public-client Dynamic Client Registration, compatibility only. Off by default.
    pub dcr_enabled: bool,
    pub policy_defaults: PolicyDefaults,
    /// Extra browser origins `/mcp` accepts (`PENFII_MCP_ALLOWED_ORIGINS`).
    pub allowed_origins: Vec<String>,
}

impl OAuthConfig {
    pub fn new(public_url: &str, fetcher: Arc<dyn ClientFetcher>) -> Result<Self, String> {
        Ok(Self {
            public_url: validate_public_url(public_url)?,
            fetcher,
            access_ttl: Duration::seconds(env_seconds("PENFII_MCP_ACCESS_TTL_SECONDS", 3600)),
            refresh_idle_ttl: Duration::seconds(env_seconds(
                "PENFII_MCP_REFRESH_IDLE_SECONDS",
                30 * 86_400,
            )),
            refresh_absolute_ttl: Duration::seconds(env_seconds(
                "PENFII_MCP_REFRESH_ABSOLUTE_SECONDS",
                90 * 86_400,
            )),
            dcr_enabled: env_flag("PENFII_MCP_DCR_ENABLED"),
            policy_defaults: PolicyDefaults::from_env(),
            allowed_origins: std::env::var("PENFII_MCP_ALLOWED_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(|part| part.trim().trim_end_matches('/').to_string())
                .filter(|part| !part.is_empty())
                .collect(),
        })
    }

    /// `None` while `PENFII_MCP_ENABLED` is off. Turning it on without a usable public URL is an
    /// error the process must not start with.
    pub fn from_env(fetcher: Arc<dyn ClientFetcher>) -> Result<Option<Self>, String> {
        if !env_flag("PENFII_MCP_ENABLED") {
            return Ok(None);
        }
        let public_url = std::env::var("PENFII_PUBLIC_URL")
            .map_err(|_| "PENFII_MCP_ENABLED needs PENFII_PUBLIC_URL".to_string())?;
        Self::new(&public_url, fetcher).map(Some)
    }

    pub fn issuer(&self) -> &str {
        &self.public_url
    }

    /// The one resource.
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.public_url)
    }

    pub fn resource_metadata_url(&self) -> String {
        format!(
            "{}/.well-known/oauth-protected-resource/mcp",
            self.public_url
        )
    }

    /// RFC 9728 document.
    pub fn protected_resource_metadata(&self) -> Value {
        json!({
            "resource": self.resource(),
            "authorization_servers": [self.issuer()],
            "scopes_supported": scopes::all(),
            "bearer_methods_supported": ["header"],
            "resource_name": "Penfii",
        })
    }

    /// RFC 8414 document; every field is fixed by the specification.
    pub fn authorization_server_metadata(&self) -> Value {
        let base = &self.public_url;
        let mut scopes_supported: Vec<&str> = scopes::all();
        scopes_supported.push("offline_access");
        let mut metadata = json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/oauth/authorize"),
            "token_endpoint": format!("{base}/oauth/token"),
            "revocation_endpoint": format!("{base}/oauth/revoke"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": true,
            "authorization_response_iss_parameter_supported": true,
            "scopes_supported": scopes_supported,
        });
        if self.dcr_enabled {
            metadata["registration_endpoint"] = json!(format!("{base}/oauth/register"));
        }
        metadata
    }

    /// The `WWW-Authenticate` challenge for a missing or bad token.
    pub fn challenge(&self, error: Option<&str>, scope: Option<&str>) -> String {
        let mut value = String::from("Bearer");
        let mut parts = Vec::new();
        if let Some(error) = error {
            parts.push(format!("error=\"{error}\""));
        }
        parts.push(format!(
            "resource_metadata=\"{}\"",
            self.resource_metadata_url()
        ));
        let scope = scope
            .map(str::to_string)
            .unwrap_or_else(|| scopes::default_request().join(" "));
        parts.push(format!("scope=\"{scope}\""));
        value.push(' ');
        value.push_str(&parts.join(", "));
        value
    }
}

/// Scheme + host (+ port), no path or trailing slash; `http` only for loopback names.
pub fn validate_public_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let url = url::Url::parse(trimmed).map_err(|_| "PENFII_PUBLIC_URL is not a URL".to_string())?;
    if url.path() != "/" && !url.path().is_empty() || url.query().is_some() {
        return Err("PENFII_PUBLIC_URL must have no path".into());
    }
    let host = url.host_str().unwrap_or_default().to_string();
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err("PENFII_PUBLIC_URL must be https (http only for localhost)".into()),
    }
    Ok(trimmed.to_string())
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref().map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

fn env_seconds(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}
