//! OAuth client identification: Client ID Metadata Documents (primary), compatibility
//! registrations and the redirect URI rule.

use std::collections::VecDeque;
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
/// Lookups of one host per minute. The budget above is shared by everyone, so one host (a page that
/// keeps pointing at itself, a person trying many ids on it) must not be able to use all of it up.
const FETCHES_PER_HOST_PER_MINUTE: usize = 6;
/// Hosts the per-host budget remembers at once; the oldest idle ones are forgotten beyond it.
const HOSTS_REMEMBERED: usize = 512;

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

pub use crate::net::is_public_address;

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
    by_host: Mutex<std::collections::HashMap<String, VecDeque<Instant>>>,
    private_hosts: Vec<String>,
}

impl HttpClientFetcher {
    pub fn new() -> Self {
        Self {
            recent: Mutex::new(VecDeque::new()),
            by_host: Mutex::new(std::collections::HashMap::new()),
            // Checked when the process started (`config::startup_checks`).
            private_hosts: crate::config::hosts(
                &crate::config::process_env,
                "PENFII_MCP_CIMD_PRIVATE_HOSTS",
            )
            .unwrap_or_default(),
        }
    }

    fn allow_host(&self, host: &str) -> bool {
        let mut hosts = self.by_host.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        hosts.retain(|_, times| {
            times.retain(|at| now.duration_since(*at) <= Duration::from_secs(60));
            !times.is_empty()
        });
        if hosts.len() >= HOSTS_REMEMBERED && !hosts.contains_key(host) {
            return false;
        }
        let times = hosts.entry(host.to_string()).or_default();
        if times.len() >= FETCHES_PER_HOST_PER_MINUTE {
            return false;
        }
        times.push_back(now);
        true
    }

    fn allow_fetch(&self) -> bool {
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
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
            // The host's own budget first, so that a refused one does not cost the shared one.
            if !self.allow_host(&host) || !self.allow_fetch() {
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
                .no_proxy()
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

/// The token endpoint authentication method this server uses with every client: they are public
/// clients and prove themselves with PKCE. The metadata advertises it as the only one.
pub const TOKEN_AUTH_METHOD: &str = "none";

/// Methods built on a shared secret, which a public document cannot hold (CIMD §4.1).
const SHARED_SECRET_METHODS: [&str; 3] = [
    "client_secret_basic",
    "client_secret_post",
    "client_secret_jwt",
];

/// JWK members that only a private or symmetric key has (RFC 7518 §6.2.2, §6.3.2, §6.4.1).
const PRIVATE_JWK_MEMBERS: [&str; 8] = ["d", "p", "q", "dp", "dq", "qi", "oth", "k"];

/// The token endpoint authentication methods a document declares (CIMD §5):
/// `token_endpoint_auth_methods_supported` when present, otherwise the singular
/// `token_endpoint_auth_method`, otherwise `none` (never RFC 7591's `client_secret_basic`
/// default). When both are present the singular one must be among the plural ones.
fn declared_auth_methods(document: &Value) -> Result<Vec<&str>, &'static str> {
    let single = match document.get("token_endpoint_auth_method") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or("token_endpoint_auth_method must be a string")?,
        ),
    };
    let Some(listed) = document.get("token_endpoint_auth_methods_supported") else {
        return Ok(vec![single.unwrap_or(TOKEN_AUTH_METHOD)]);
    };
    let methods = listed
        .as_array()
        .filter(|all| !all.is_empty())
        .and_then(|all| all.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
        .ok_or("token_endpoint_auth_methods_supported must be a non-empty list of names")?;
    if single.is_some_and(|single| !methods.contains(&single)) {
        return Err(
            "token_endpoint_auth_method must be one of token_endpoint_auth_methods_supported",
        );
    }
    Ok(methods)
}

/// Checks a fetched document and returns `(name, redirect_uris)`.
///
/// A document may declare methods this server does not use (such as `private_key_jwt` next to
/// `none`); the server picks `none` from them and the client is public. One that leaves `none`
/// out names no method this server supports and is refused.
pub fn parse_document(url: &str, document: &Value) -> Result<(String, Vec<String>), CoreError> {
    let bad = |message: &str| CoreError::validation(format!("client metadata: {message}"));
    if document.get("client_id").and_then(Value::as_str) != Some(url) {
        return Err(bad("client_id does not equal the document URL"));
    }
    if document.get("client_secret").is_some() || document.get("client_secret_expires_at").is_some()
    {
        return Err(bad("must not contain a secret"));
    }
    let methods = declared_auth_methods(document).map_err(bad)?;
    if methods
        .iter()
        .any(|method| SHARED_SECRET_METHODS.contains(method))
    {
        return Err(bad(
            "must not declare a shared-secret authentication method",
        ));
    }
    if !methods.contains(&TOKEN_AUTH_METHOD) {
        return Err(bad(
            "declares no token endpoint authentication method this server supports (none)",
        ));
    }
    let jwks = document.get("jwks").filter(|value| !value.is_null());
    if jwks.is_some()
        && document
            .get("jwks_uri")
            .is_some_and(|value| !value.is_null())
    {
        return Err(bad("must not hold both jwks and jwks_uri"));
    }
    if let Some(jwks) = jwks {
        let keys = jwks
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("jwks must be a JWK Set"))?;
        if keys
            .iter()
            .any(|key| PRIVATE_JWK_MEMBERS.iter().any(|m| key.get(*m).is_some()))
        {
            return Err(bad("must not contain private key material"));
        }
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

/// Who is asking, for the budget of lookups that cost a request to another server.
#[derive(Clone, Copy)]
pub struct Lookup<'a> {
    /// The client address of the (unauthenticated) request, if it is known.
    pub address: Option<&'a str>,
    pub throttle: &'a crate::limits::ThrottleLimits,
}

/// A client by its `client_id`: a CIMD URL (fetched once, cached by headers) or an id issued by
/// this server. An unknown or disabled client is `NotFound` and never reveals more. A document that
/// has gone stale is fetched again; when that cannot be done (the host is down, the asker's budget of
/// lookups is used, the shared budget is) the client that was known goes on being known from the
/// stored copy, so a busy or hostile moment does not lock out a client that was working.
pub async fn resolve(
    pool: &PgPool,
    config: &OAuthConfig,
    client_id: &str,
    lookup: Lookup<'_>,
) -> Result<Client, CoreError> {
    let cached = sqlx::query(
        "SELECT id, client_id, kind, client_name, redirect_uris, expires_at, disabled_at,
                (expires_at IS NULL OR expires_at > now()) AS fresh
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
        // Whether the cached document is still fresh is the database's clock's call.
        let fresh: bool = row.try_get("fresh")?;
        if !is_url || fresh {
            return from_row(row);
        }
    }
    if !is_url {
        return Err(CoreError::NotFound);
    }
    check_cimd_url(client_id)?;
    let fetched = fetch_within_budget(pool, config, client_id, lookup).await;
    let (document, ttl) = match (fetched, &cached) {
        (Ok(found), _) => found,
        (Err(error), Some(row)) => {
            tracing::warn!(
                client_id,
                ?error,
                "client metadata not refreshed; using the stored copy"
            );
            return from_row(row);
        }
        (Err(error), None) => return Err(error),
    };
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

/// The fetch, after the asker's own budget: every lookup of a document not held counts against their
/// address (`oauth_lookup`), so one address cannot keep the server fetching and storing.
async fn fetch_within_budget(
    pool: &PgPool,
    config: &OAuthConfig,
    client_id: &str,
    lookup: Lookup<'_>,
) -> Result<(Value, i64), CoreError> {
    crate::identity::check_address(pool, "oauth_lookup", lookup.address).await?;
    crate::identity::record_address_failure(pool, "oauth_lookup", lookup.address, lookup.throttle)
        .await?;
    config.fetcher.fetch(client_id).await
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
        let mut expiry = ok.clone();
        expiry["client_secret_expires_at"] = json!(0);
        assert!(parse_document(url, &expiry).is_err());
        let mut jwt = ok.clone();
        jwt["token_endpoint_auth_method"] = json!("private_key_jwt");
        assert!(parse_document(url, &jwt).is_err());
        let mut scheme = ok.clone();
        scheme["redirect_uris"] = json!(["javascript:alert(1)"]);
        assert!(parse_document(url, &scheme).is_err());
    }

    /// CIMD §5: the plural list is read first, the server picks `none` from it, and the
    /// methods it does not support are not an error.
    #[test]
    fn auth_methods_are_negotiated_from_the_plural_list() {
        let url = "https://app.example/oauth/client.json";
        let base = json!({"client_id": url, "client_name": "App", "redirect_uris": ["https://app.example/cb"]});
        let with = |extra: Value| {
            let mut document = base.clone();
            for (key, value) in extra.as_object().unwrap() {
                document[key] = value.clone();
            }
            parse_document(url, &document)
        };
        // Neither property: none, not RFC 7591's client_secret_basic.
        assert!(with(json!({})).is_ok());
        assert!(with(json!({"token_endpoint_auth_methods_supported": ["none"]})).is_ok());
        // A preference this server lacks, with none among the supported ones (ChatGPT's shape).
        let chatgpt = with(json!({
            "token_endpoint_auth_method": "private_key_jwt",
            "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
            "token_endpoint_auth_signing_alg": "RS256",
            "jwks_uri": "https://app.example/oauth/jwks.json",
            "grant_types": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:jwt-bearer"],
        }));
        assert!(chatgpt.is_ok(), "{chatgpt:?}");
        assert!(
            with(json!({"token_endpoint_auth_methods_supported": ["private_key_jwt", "none"]}))
                .is_ok()
        );
        // No method this server supports.
        for refused in [
            json!({"token_endpoint_auth_methods_supported": ["private_key_jwt"]}),
            json!({"token_endpoint_auth_methods_supported": ["tls_client_auth", "private_key_jwt"]}),
            json!({"token_endpoint_auth_method": "private_key_jwt"}),
            // The singular one must be among the plural ones.
            json!({"token_endpoint_auth_method": "private_key_jwt", "token_endpoint_auth_methods_supported": ["none"]}),
            // Shared secrets, alone or beside none.
            json!({"token_endpoint_auth_method": "client_secret_basic"}),
            json!({"token_endpoint_auth_methods_supported": ["none", "client_secret_post"]}),
            json!({"token_endpoint_auth_method": "none", "token_endpoint_auth_methods_supported": ["none", "client_secret_jwt"]}),
            // Malformed.
            json!({"token_endpoint_auth_methods_supported": []}),
            json!({"token_endpoint_auth_methods_supported": "none"}),
            json!({"token_endpoint_auth_methods_supported": ["none", 1]}),
            json!({"token_endpoint_auth_method": ["none"]}),
        ] {
            assert!(with(refused.clone()).is_err(), "{refused}");
        }
    }

    #[test]
    fn key_material_is_public_and_published_once() {
        let url = "https://app.example/oauth/client.json";
        let doc = |extra: Value| {
            let mut document = json!({"client_id": url, "client_name": "App", "redirect_uris": ["https://app.example/cb"], "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"]});
            for (key, value) in extra.as_object().unwrap() {
                document[key] = value.clone();
            }
            parse_document(url, &document)
        };
        let public_key = json!({"kty": "RSA", "kid": "1", "n": "AQAB", "e": "AQAB"});
        assert!(doc(json!({"jwks": {"keys": [public_key]}})).is_ok());
        assert!(doc(json!({"jwks_uri": "https://app.example/jwks.json"})).is_ok());
        assert!(
            doc(
                json!({"jwks": {"keys": [public_key]}, "jwks_uri": "https://app.example/jwks.json"})
            )
            .is_err()
        );
        assert!(
            doc(json!({"jwks": {"keys": [{"kty": "RSA", "n": "AQAB", "e": "AQAB", "d": "x"}]}}))
                .is_err()
        );
        assert!(doc(json!({"jwks": {"keys": [{"kty": "oct", "k": "c2VjcmV0"}]}})).is_err());
        assert!(doc(json!({"jwks": [public_key]})).is_err());
    }

    #[test]
    fn cache_ttl_is_clamped() {
        assert_eq!(cache_ttl(Some("public, max-age=5")), 60);
        assert_eq!(cache_ttl(Some("max-age=99999999")), 86_400);
        assert_eq!(cache_ttl(Some("max-age=600")), 600);
        assert_eq!(cache_ttl(None), 3600);
    }
}
