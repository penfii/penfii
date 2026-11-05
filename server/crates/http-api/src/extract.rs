use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequestParts, Query, Request};
use axum::http::request::Parts;
use http::HeaderMap;
use penfii_contracts::User;
use penfii_core::{CoreError, identity};
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;

pub struct RawJson(pub Value);

impl<S> FromRequest<S> for RawJson
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<Value>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(error) => Err(ApiError::validation(format!("invalid json: {error}"))),
        }
    }
}

pub fn parse<T: DeserializeOwned>(value: Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(|error| ApiError::validation(error.to_string()))
}

/// Keeps "field omitted" (leave as is) apart from `null` (clear the value), which a plain
/// `Option<Option<T>>` would both read as omitted. Use with `#[serde(default, deserialize_with)]`.
pub fn present_or_null<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

pub fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::validation("Idempotency-Key is required"));
    };
    let value = value.trim();
    if value.is_empty() || value.len() > 200 {
        return Err(ApiError::validation("Idempotency-Key is required"));
    }
    Ok(value.to_string())
}

pub fn session_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(http::header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(token) = part.strip_prefix("penfii_session=")
            && !token.is_empty()
        {
            return Some(token.to_string());
        }
    }
    None
}

async fn current_user(state: &AppState, headers: &HeaderMap) -> Result<User, ApiError> {
    let token = session_token(headers).ok_or(CoreError::Unauthenticated)?;
    Ok(identity::authenticate(&state.pool, &token).await?)
}

async fn current_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<identity::AuthSession, ApiError> {
    let token = session_token(headers).ok_or(CoreError::Unauthenticated)?;
    Ok(identity::authenticate_session(&state.pool, &token).await?)
}

// Session extractors. A handler that takes one of these requires a signed-in session: without a
// cookie, or with an expired or revoked one, it answers `401 unauthenticated` before the request
// body is read. Public handlers take none (or `Option<Session>`). The live sockets put
// `origin::AllowedOrigin` in front, so a foreign origin is refused before the cookie is looked at.

/// The signed-in person's id.
pub struct Actor(pub Uuid);

impl FromRequestParts<AppState> for Actor {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(Self(current_user(state, &parts.headers).await?.id))
    }
}

/// The signed-in person's profile.
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(Self(current_user(state, &parts.headers).await?))
    }
}

/// The signed-in session itself, for routes that look at how or when it signed in.
pub struct Session(pub identity::AuthSession);

impl FromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(Self(current_session(state, &parts.headers).await?))
    }
}

/// `Option<Session>`: no cookie or a dead one is `None`; any other failure is still an error.
impl OptionalFromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, ApiError> {
        let Some(token) = session_token(&parts.headers) else {
            return Ok(None);
        };
        match identity::authenticate_session(&state.pool, &token).await {
            Ok(session) => Ok(Some(Self(session))),
            Err(CoreError::Unauthenticated) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

/// The person's id, after checking that they signed in recently (`403 reauth_required`
/// otherwise). For destructive administration.
pub struct RecentActor(pub Uuid);

impl FromRequestParts<AppState> for RecentActor {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let session = current_session(state, &parts.headers).await?;
        identity::require_recent_authentication(&state.pool, &session).await?;
        Ok(Self(session.user.id))
    }
}

pub fn session_cookie(token: &str, max_age: i64, secure: bool) -> String {
    let mut value =
        format!("penfii_session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}");
    if secure {
        value.push_str("; Secure");
    }
    value
}

pub fn clear_cookie(secure: bool) -> String {
    session_cookie("", 0, secure)
}

pub struct Q<T>(pub T);

impl<S, T> FromRequestParts<S> for Q<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(Self(value)),
            Err(error) => Err(ApiError::validation(error.body_text())),
        }
    }
}

type Ranges = Vec<(std::net::IpAddr, u8)>;

/// What `PENFII_TRUSTED_PROXIES` means when it is not set: only a proxy on the same machine.
/// Anything else (a reverse proxy in another container, a load balancer) has to be named.
const DEFAULT_TRUSTED_PROXIES: &str = "127.0.0.0/8,::1/128";

/// Parses comma-separated CIDRs (a bare address means one host). An empty value trusts nobody.
/// A malformed entry is an error, not something to skip: a typo would silently change who may
/// set `X-Forwarded-For`.
pub fn parse_trusted_proxies(raw: &str) -> Result<Ranges, String> {
    let mut ranges = Vec::new();
    for part in raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (addr, bits) = part.split_once('/').unwrap_or((part, ""));
        let addr: std::net::IpAddr = addr
            .parse()
            .map_err(|_| format!("{part:?} is not an IP address or CIDR"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let bits = if bits.is_empty() {
            max
        } else {
            bits.parse::<u8>()
                .ok()
                .filter(|bits| *bits <= max)
                .ok_or_else(|| format!("{part:?} has an invalid prefix length"))?
        };
        ranges.push((addr, bits));
    }
    Ok(ranges)
}

fn configured_trusted_proxies() -> Result<Ranges, String> {
    match std::env::var("PENFII_TRUSTED_PROXIES") {
        Ok(raw) => parse_trusted_proxies(&raw),
        Err(_) => parse_trusted_proxies(DEFAULT_TRUSTED_PROXIES),
    }
}

/// Fails when `PENFII_TRUSTED_PROXIES` is malformed. core-api calls it before it listens.
pub fn validate_trusted_proxies() -> Result<(), String> {
    configured_trusted_proxies()
        .map(|_| ())
        .map_err(|error| format!("PENFII_TRUSTED_PROXIES: {error}"))
}

fn trusted_ranges() -> &'static Ranges {
    static RANGES: std::sync::OnceLock<Ranges> = std::sync::OnceLock::new();
    RANGES.get_or_init(|| {
        // A bad value was already refused at start-up; trusting nobody is the safe fallback.
        configured_trusted_proxies().unwrap_or_default()
    })
}

fn is_trusted(ranges: &Ranges, ip: std::net::IpAddr) -> bool {
    ranges.iter().any(|(net, bits)| match (ip, net) {
        (std::net::IpAddr::V4(ip), std::net::IpAddr::V4(net)) => {
            let mask = if *bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            u32::from(ip) & mask == u32::from(*net) & mask
        }
        (std::net::IpAddr::V6(ip), std::net::IpAddr::V6(net)) => {
            let mask = if *bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            u128::from(ip) & mask == u128::from(*net) & mask
        }
        _ => false,
    })
}

/// The client address for throttling. `X-Forwarded-For` counts only when the TCP peer is a
/// trusted proxy; then the rightmost entry that is not itself a trusted proxy is the client.
/// `None` when the peer is unknown.
pub fn client_address(peer: Option<std::net::SocketAddr>, headers: &HeaderMap) -> Option<String> {
    client_address_with(trusted_ranges(), peer, headers)
}

fn client_address_with(
    ranges: &Ranges,
    peer: Option<std::net::SocketAddr>,
    headers: &HeaderMap,
) -> Option<String> {
    let peer = peer?.ip();
    if is_trusted(ranges, peer)
        && let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
    {
        for entry in forwarded.rsplit(',') {
            if let Ok(ip) = entry.trim().parse::<std::net::IpAddr>()
                && !is_trusted(ranges, ip)
            {
                return Some(ip.to_string());
            }
        }
    }
    Some(peer.to_string())
}

/// A coarse "Browser on OS" label for the session list; never the raw User-Agent.
pub fn device_label(headers: &HeaderMap) -> Option<String> {
    let agent = headers.get(http::header::USER_AGENT)?.to_str().ok()?;
    let browser = ["Edg/", "OPR/", "Firefox/", "Chrome/", "Safari/"]
        .iter()
        .zip(["Edge", "Opera", "Firefox", "Chrome", "Safari"])
        .find(|(needle, _)| agent.contains(**needle))
        .map(|(_, name)| name)
        .unwrap_or("Browser");
    let os = [
        ("Android", "Android"),
        ("iPhone", "iOS"),
        ("iPad", "iOS"),
        ("Windows", "Windows"),
        ("Mac OS X", "macOS"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(needle, _)| agent.contains(needle))
    .map(|(_, name)| *name);
    Some(match os {
        Some(os) => format!("{browser} on {os}"),
        None => browser.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    fn ranges(raw: &str) -> Ranges {
        parse_trusted_proxies(raw).unwrap()
    }

    #[test]
    fn forwarded_for_counts_only_behind_a_trusted_proxy() {
        let proxies = ranges("10.0.0.0/8");
        let behind = Some("10.1.2.3:5000".parse().unwrap());
        let spoof = headers(&[("x-forwarded-for", "198.51.100.9, 203.0.113.7, 10.1.2.4")]);
        // Rightmost entry that is not a trusted proxy.
        assert_eq!(
            client_address_with(&proxies, behind, &spoof).as_deref(),
            Some("203.0.113.7")
        );
        // A public peer cannot choose its own address.
        let direct = Some("203.0.113.50:5000".parse().unwrap());
        assert_eq!(
            client_address_with(&proxies, direct, &spoof).as_deref(),
            Some("203.0.113.50")
        );
        assert_eq!(client_address_with(&proxies, None, &spoof), None);
        // Nothing forwarded: the peer itself.
        assert_eq!(
            client_address_with(&proxies, behind, &HeaderMap::new()).as_deref(),
            Some("10.1.2.3")
        );
    }

    #[test]
    fn by_default_only_loopback_may_set_forwarded_for() {
        let proxies = ranges(DEFAULT_TRUSTED_PROXIES);
        let spoof = headers(&[("x-forwarded-for", "198.51.100.9")]);
        // A neighbour on a private network (another container, another machine on the LAN) cannot
        // pick the address the throttle sees.
        for peer in [
            "10.1.2.3:5000",
            "172.18.0.5:5000",
            "192.168.1.9:5000",
            "[fd00::1]:5000",
        ] {
            assert_eq!(
                client_address_with(&proxies, Some(peer.parse().unwrap()), &spoof).as_deref(),
                Some(peer.rsplit_once(':').unwrap().0.trim_matches(['[', ']'])),
                "{peer}"
            );
        }
        // The proxy on the same machine is believed.
        for peer in ["127.0.0.1:5000", "[::1]:5000"] {
            assert_eq!(
                client_address_with(&proxies, Some(peer.parse().unwrap()), &spoof).as_deref(),
                Some("198.51.100.9"),
                "{peer}"
            );
        }
    }

    #[test]
    fn an_empty_list_trusts_nobody() {
        let proxies = ranges("");
        let spoof = headers(&[("x-forwarded-for", "198.51.100.9")]);
        assert_eq!(
            client_address_with(&proxies, Some("127.0.0.1:1".parse().unwrap()), &spoof).as_deref(),
            Some("127.0.0.1")
        );
    }

    #[test]
    fn a_malformed_proxy_list_is_an_error() {
        for bad in [
            "10.0.0.0/33",
            "::1/129",
            "10.0.0.0/x",
            "proxy.local",
            "10.0.0.0/8,oops",
            "10.0.0.0/-1",
        ] {
            assert!(parse_trusted_proxies(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_trusted_proxies(" 10.0.0.0/8 , 192.0.2.7 ,, fc00::/7 ")
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn device_label_is_coarse() {
        let chrome = headers(&[(
            "user-agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36",
        )]);
        assert_eq!(device_label(&chrome).as_deref(), Some("Chrome on Windows"));
        assert_eq!(device_label(&HeaderMap::new()), None);
    }
}
