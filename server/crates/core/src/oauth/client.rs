//! OAuth client identification: Client ID Metadata Documents (primary), compatibility
//! registrations and the redirect URI rule.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use url::Url;
use uuid::Uuid;

use super::OAuthConfig;
use crate::error::CoreError;

const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const MIN_TTL_SECONDS: i64 = 60;
const MAX_TTL_SECONDS: i64 = 86_400;
const FETCHES_PER_MINUTE: usize = 30;

/// The only way the Authorization Server reaches a client's metadata document. Tests inject a
/// fake; production uses [`HttpClientFetcher`].
pub trait ClientFetcher: Send + Sync {
    /// The parsed JSON document and how long it may be cached, in seconds (already clamped).
    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<(Value, i64), CoreError>>;
}

#[derive(Debug, Clone)]
pub struct Client {
    pub pk: Uuid,
    pub client_id: String,
    pub kind: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}

/// Public means a routable unicast address: not loopback, private, link-local (which holds the
/// cloud metadata address), unique-local, carrier-grade NAT, documentation, multicast or
/// unspecified.
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00 // unique local
                || first & 0xffc0 == 0xfe80 // link local
                || (first == 0x2001 && ip.segments()[1] == 0x0db8)
                || ip == Ipv6Addr::LOCALHOST)
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

/// Validates a CIMD URL before any network access.
pub fn check_cimd_url(raw: &str) -> Result<Url, CoreError> {
    let url = Url::parse(raw).map_err(|_| CoreError::validation("client_id is not a URL"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || url.fragment().is_some()
        || url.path().len() <= 1
    {
        return Err(CoreError::validation(
            "client_id must be an https URL with a path, no credentials, port 443",
        ));
    }
    match url.host() {
        Some(url::Host::Domain(_)) => Ok(url),
        _ => Err(CoreError::validation("client_id host must be a DNS name")),
    }
}

/// Production fetcher: TLS verified, DNS resolved here and pinned for the connection (no
/// rebinding), every resolved address public, no redirects, bounded size and time.
pub struct HttpClientFetcher {
    recent: Mutex<VecDeque<Instant>>,
    private_hosts: Vec<String>,
}

impl HttpClientFetcher {
    pub fn new() -> Self {
        Self {
            recent: Mutex::new(VecDeque::new()),
            private_hosts: std::env::var("PENFII_MCP_CIMD_PRIVATE_HOSTS")
                .unwrap_or_default()
                .split(',')
                .map(|host| host.trim().to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
        }
    }

    fn allow_fetch(&self) -> bool {
        let mut recent = self.recent.lock().unwrap();
        let now = Instant::now();
        while recent
            .front()
            .is_some_and(|at| now.duration_since(*at) > Duration::from_secs(60))
        {
            recent.pop_front();
        }
        if recent.len() >= FETCHES_PER_MINUTE {
            return false;
        }
        recent.push_back(now);
        true
    }
}

impl Default for HttpClientFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientFetcher for HttpClientFetcher {
    fn fetch(&self, raw: &str) -> BoxFuture<'_, Result<(Value, i64), CoreError>> {
        let raw = raw.to_string();
        Box::pin(async move {
            let url = check_cimd_url(&raw)?;
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            if !self.allow_fetch() {
                return Err(CoreError::validation("too many client lookups"));
            }
            let mut addresses: Vec<std::net::SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 443))
                    .await
                    .map_err(|_| CoreError::validation("client metadata host did not resolve"))?
                    .collect();
            let private_ok = self.private_hosts.contains(&host);
            if !private_ok {
                addresses.retain(|address| is_public_address(address.ip()));
            }
            let address = addresses
                .first()
                .copied()
                .ok_or_else(|| CoreError::validation("client metadata host is not public"))?;
            let http = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .resolve(&host, address)
                .user_agent("penfii-oauth")
                .build()
                .map_err(|e| CoreError::Internal(e.to_string()))?;
            let mut response = http
                .get(url.as_str())
                .header("accept", "application/json")
                .send()
                .await
                .map_err(|_| CoreError::validation("client metadata could not be fetched"))?;
            if !response.status().is_success() {
                return Err(CoreError::validation(
                    "client metadata could not be fetched",
                ));
            }
            let is_json = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.to_ascii_lowercase().contains("json"));
            if !is_json {
                return Err(CoreError::validation("client metadata is not JSON"));
            }
            let ttl = cache_ttl(
                response
                    .headers()
                    .get("cache-control")
                    .and_then(|value| value.to_str().ok()),
            );
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| CoreError::validation("client metadata could not be read"))?
            {
                body.extend_from_slice(&chunk);
                if body.len() > MAX_DOCUMENT_BYTES {
                    return Err(CoreError::validation("client metadata is too large"));
                }
            }
            let value = serde_json::from_slice(&body)
                .map_err(|_| CoreError::validation("client metadata is not JSON"))?;
            Ok((value, ttl))
        })
    }
}

/// `Cache-Control: max-age` clamped to one minute .. 24 hours; one hour without it.
pub fn cache_ttl(header: Option<&str>) -> i64 {
    let max_age = header.and_then(|value| {
        value
            .split(',')
            .filter_map(|part| part.trim().strip_prefix("max-age="))
            .find_map(|seconds| seconds.trim().parse::<i64>().ok())
    });
    max_age
        .unwrap_or(3600)
        .clamp(MIN_TTL_SECONDS, MAX_TTL_SECONDS)
}

/// A redirect URI that a client may register: `https`, or an `http` loopback address (RFC 8252).
pub fn valid_redirect_uri(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host_str().is_some(),
        "http" => is_loopback_host(&url),
        _ => false,
    }
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name == "localhost",
        None => false,
    }
}

pub fn is_loopback_redirect(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|url| url.scheme() == "http" && is_loopback_host(&url))
}

/// Exact string match. The one relaxation is an RFC 8252 loopback redirect, where the
/// port is ignored (scheme, host and path still have to match).
pub fn redirect_matches(registered: &[String], given: &str) -> bool {
    if registered.iter().any(|uri| uri == given) {
        return true;
    }
    let Ok(given_url) = Url::parse(given) else {
        return false;
    };
    if !(given_url.scheme() == "http" && is_loopback_host(&given_url)) {
        return false;
    }
    registered.iter().any(|uri| {
        Url::parse(uri).is_ok_and(|url| {
            url.scheme() == "http"
                && is_loopback_host(&url)
                && url.host() == given_url.host()
                && url.path() == given_url.path()
                && url.query() == given_url.query()
        })
    })
}

/// Checks a fetched document and returns `(name, redirect_uris)`.
pub fn parse_document(url: &str, document: &Value) -> Result<(String, Vec<String>), CoreError> {
    let bad = |message: &str| CoreError::validation(format!("client metadata: {message}"));
    if document.get("client_id").and_then(Value::as_str) != Some(url) {
        return Err(bad("client_id does not equal the document URL"));
    }
    if document.get("client_secret").is_some() {
        return Err(bad("must not contain a secret"));
    }
    if let Some(method) = document
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        && method != "none"
    {
        return Err(bad("only public clients are accepted"));
    }
    let name = document
        .get("client_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| bad("client_name is required"))?;
    let uris: Vec<String> = document
        .get("redirect_uris")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("redirect_uris is required"))?
        .iter()
        .filter_map(|uri| uri.as_str().map(str::to_string))
        .collect();
    if uris.is_empty() || uris.len() > 20 || !uris.iter().all(|uri| valid_redirect_uri(uri)) {
        return Err(bad(
            "redirect_uris must be https or loopback http addresses",
        ));
    }
    Ok((name.chars().take(100).collect(), uris))
}

/// A client by its `client_id`: a CIMD URL (fetched once, cached by headers) or an id issued by
/// this server. An unknown or disabled client is `NotFound` and never reveals more.
pub async fn resolve(
    pool: &PgPool,
    config: &OAuthConfig,
    client_id: &str,
) -> Result<Client, CoreError> {
    let cached = sqlx::query(
        "SELECT id, client_id, kind, client_name, redirect_uris, expires_at, disabled_at
         FROM penfii.oauth_clients WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await?;
    let is_url = client_id.starts_with("https://");
    if let Some(row) = &cached {
        let disabled: Option<chrono::DateTime<chrono::Utc>> = row.try_get("disabled_at")?;
        if disabled.is_some() {
            return Err(CoreError::NotFound);
        }
        let expires: Option<chrono::DateTime<chrono::Utc>> = row.try_get("expires_at")?;
        if !is_url || expires.is_none_or(|at| at > chrono::Utc::now()) {
            return from_row(row);
        }
    }
    if !is_url {
        return Err(CoreError::NotFound);
    }
    check_cimd_url(client_id)?;
    let (document, ttl) = config.fetcher.fetch(client_id).await?;
    let (name, uris) = parse_document(client_id, &document)?;
    let hash = Sha256::digest(document.to_string().as_bytes()).to_vec();
    let row = sqlx::query(
        "INSERT INTO penfii.oauth_clients
            (id, client_id, kind, client_name, redirect_uris, metadata_hash, fetched_at, expires_at)
         VALUES ($1, $2, 'cimd', $3, $4, $5, now(), now() + make_interval(secs => $6))
         ON CONFLICT (client_id) DO UPDATE SET
            client_name = EXCLUDED.client_name, redirect_uris = EXCLUDED.redirect_uris,
            metadata_hash = EXCLUDED.metadata_hash, fetched_at = now(),
            expires_at = EXCLUDED.expires_at
         RETURNING id, client_id, kind, client_name, redirect_uris, expires_at, disabled_at",
    )
    .bind(Uuid::new_v4())
    .bind(client_id)
    .bind(&name)
    .bind(serde_json::to_value(&uris).unwrap_or_default())
    .bind(hash)
    .bind(ttl as f64)
    .fetch_one(pool)
    .await?;
    from_row(&row)
}

fn from_row(row: &sqlx::postgres::PgRow) -> Result<Client, CoreError> {
    let uris: Value = row.try_get("redirect_uris")?;
    Ok(Client {
        pk: row.try_get("id")?,
        client_id: row.try_get("client_id")?,
        kind: row.try_get("kind")?,
        name: row.try_get("client_name")?,
        redirect_uris: uris
            .as_array()
            .map(|all| {
                all.iter()
                    .filter_map(|uri| uri.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn private_and_special_addresses_are_refused() {
        for blocked in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
            "224.0.0.1",
        ] {
            assert!(!is_public_address(blocked.parse().unwrap()), "{blocked}");
        }
        for open in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_address(open.parse().unwrap()), "{open}");
        }
    }

    #[test]
    fn cimd_url_rules() {
        assert!(check_cimd_url("https://chatgpt.com/oauth/client.json").is_ok());
        for bad in [
            "http://example.com/client.json",
            "https://example.com",
            "https://user:pw@example.com/c.json",
            "https://example.com:8443/c.json",
            "https://127.0.0.1/c.json",
            "https://[::1]/c.json",
            "https://example.com/c.json#x",
        ] {
            assert!(check_cimd_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn redirect_matching_is_exact_with_a_loopback_port_exemption() {
        let registered = vec![
            "https://claude.ai/api/mcp/auth_callback".to_string(),
            "http://127.0.0.1/callback".to_string(),
        ];
        assert!(redirect_matches(
            &registered,
            "https://claude.ai/api/mcp/auth_callback"
        ));
        assert!(!redirect_matches(
            &registered,
            "https://claude.ai/api/mcp/auth_callback/"
        ));
        assert!(!redirect_matches(
            &registered,
            "https://claude.ai:8443/api/mcp/auth_callback"
        ));
        assert!(redirect_matches(
            &registered,
            "http://127.0.0.1:53211/callback"
        ));
        assert!(!redirect_matches(
            &registered,
            "http://127.0.0.1:53211/other"
        ));
        assert!(!redirect_matches(
            &registered,
            "http://localhost:53211/callback"
        ));
        assert!(!redirect_matches(
            &registered,
            "http://evil.example/callback"
        ));
    }

    #[test]
    fn document_must_name_itself_and_hold_no_secret() {
        let url = "https://app.example/oauth/client.json";
        let ok = json!({"client_id": url, "client_name": "App", "redirect_uris": ["https://app.example/cb"], "token_endpoint_auth_method": "none"});
        assert!(parse_document(url, &ok).is_ok());
        let mut wrong = ok.clone();
        wrong["client_id"] = json!("https://other.example/x.json");
        assert!(parse_document(url, &wrong).is_err());
        let mut secret = ok.clone();
        secret["client_secret"] = json!("x");
        assert!(parse_document(url, &secret).is_err());
        let mut jwt = ok.clone();
        jwt["token_endpoint_auth_method"] = json!("private_key_jwt");
        assert!(parse_document(url, &jwt).is_err());
        let mut scheme = ok.clone();
        scheme["redirect_uris"] = json!(["javascript:alert(1)"]);
        assert!(parse_document(url, &scheme).is_err());
    }

    #[test]
    fn cache_ttl_is_clamped() {
        assert_eq!(cache_ttl(Some("public, max-age=5")), 60);
        assert_eq!(cache_ttl(Some("max-age=99999999")), 86_400);
        assert_eq!(cache_ttl(Some("max-age=600")), 600);
        assert_eq!(cache_ttl(None), 3600);
    }
}
