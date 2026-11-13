//! Which browser origins a request may come from.
//!
//! Two endpoints check the `Origin` header, each against its own server-controlled list: the
//! WebSockets (here, [`origin_allowed`]) and `/mcp` (`mcp_routes`). They trust different origins
//! (the sockets also trust the server's own `Host`; `/mcp` trusts only `PENFII_PUBLIC_URL` and
//! `PENFII_MCP_ALLOWED_ORIGINS`), so the lists stay separate. What they share is how the header is
//! read ([`request_origin`]): a missing header is explicit (not a browser, it still needs a
//! credential), a header that is present but unreadable is refused, and every comparison is made on
//! [`normalize`]d text. The CORS response headers (`cors_layer`) are a separate concern: they tell
//! a browser what it may read, and decide nothing about a request.
//!
//! WebSockets:
//!
//! A WebSocket upgrade is a plain `GET` that carries the session cookie but no `X-Penfii-Client`
//! header, and a page on another site of the same registrable domain (or on another port) is
//! "same-site" for `SameSite=Lax`. So every socket checks the `Origin` the browser reports before
//! it looks at the cookie. All four sockets (`/v1/ws`, `/v1/ws/user`, `/v1/ws/projects/{id}`,
//! `/v1/docs/{id}/collab`) take [`AllowedOrigin`] before their session extractor; none has a
//! policy of its own.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};

use crate::AppState;
use crate::error::ApiError;

/// The origins of the Vite dev server, which proxies `/v1` and so arrives under another `Host`.
const DEV_ORIGINS: [&str; 2] = ["http://localhost:5173", "http://127.0.0.1:5173"];

/// `PENFII_ALLOWED_ORIGINS`: comma-separated origins (`https://penfii.example.com`) that may open the
/// socket in addition to the server's own origin. Unset means the local dev server origins only.
pub(crate) fn allowed_origins_from_env() -> Vec<String> {
    let lookup = &penfii_core::config::process_env;
    // Not set, or empty: the dev server origins. A list that cannot be read was refused when the process
    // started (`config::startup_checks`); if it gets here anyway, no origin is allowed beyond the server's own.
    match penfii_core::config::origins(lookup, "PENFII_ALLOWED_ORIGINS") {
        Ok(list) if penfii_core::config::get(lookup, "PENFII_ALLOWED_ORIGINS").is_some() => list,
        Ok(_) => DEV_ORIGINS.iter().map(|o| (*o).to_owned()).collect(),
        Err(error) => {
            tracing::error!(%error, "no extra origin is allowed");
            Vec::new()
        }
    }
}

/// The `Origin` of a request.
pub(crate) enum RequestOrigin {
    /// No `Origin` header: not a browser page (servers and command-line clients send none).
    Absent,
    /// The header, [`normalize`]d. `null` is kept as the text `null`, which no list contains.
    Present(String),
    /// A header that is not valid text. Never allowed.
    Unreadable,
}

pub(crate) fn request_origin(headers: &HeaderMap) -> RequestOrigin {
    match headers.get(axum::http::header::ORIGIN) {
        None => RequestOrigin::Absent,
        Some(value) => match value.to_str() {
            Ok(text) => RequestOrigin::Present(normalize(text)),
            Err(_) => RequestOrigin::Unreadable,
        },
    }
}

/// Trims, drops a trailing slash and lowercases (scheme and host are case-insensitive). Both the
/// header and every configured origin go through it before they are compared.
pub(crate) fn normalize(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// A browser always sends `Origin` on a WebSocket handshake, and a page cannot forge it. The
/// handshake is allowed when there is no `Origin` (not a browser; it still needs the session
/// cookie), when the origin is the server's own (its authority equals the request's `Host`), or
/// when it is listed in `allowed`. `null`, an unreadable header and anything else are refused.
pub(crate) fn origin_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    let origin = match request_origin(headers) {
        RequestOrigin::Absent => return true,
        RequestOrigin::Unreadable => return false,
        RequestOrigin::Present(origin) => origin,
    };
    if allowed.contains(&origin) {
        return true;
    }
    let authority = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"));
    match (authority, headers.get(axum::http::header::HOST)) {
        (Some(authority), Some(host)) => host
            .to_str()
            .is_ok_and(|host| host.trim().eq_ignore_ascii_case(authority)),
        _ => false,
    }
}

/// Refuses the handshake with `403 origin_not_allowed` unless [`origin_allowed`] says yes
/// (against `AppState::allowed_origins`).
pub(crate) struct AllowedOrigin;

impl FromRequestParts<AppState> for AllowedOrigin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if origin_allowed(&parts.headers, &state.allowed_origins) {
            return Ok(Self);
        }
        Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "this origin may not open a live socket",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(origin: Option<&str>, host: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert("host", HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            map.insert("origin", HeaderValue::from_str(origin).unwrap());
        }
        map
    }

    fn dev() -> Vec<String> {
        DEV_ORIGINS.iter().map(|o| (*o).to_owned()).collect()
    }

    #[test]
    fn the_server_s_own_origin_and_the_listed_ones_may_connect() {
        let h = headers(Some("https://penfii.example.com"), "penfii.example.com");
        assert!(origin_allowed(&h, &[]));
        let h = headers(Some("HTTPS://Penfii.Example.com/"), "penfii.example.com");
        assert!(origin_allowed(&h, &[]));
        let h = headers(Some("http://127.0.0.1:5173"), "127.0.0.1:8080");
        assert!(
            origin_allowed(&h, &dev()),
            "the dev server is listed by default"
        );
        assert!(!origin_allowed(&h, &[]), "and only because it is listed");
        let listed = penfii_core::config::origins(
            &|_| Some(" https://a.example.com/ , https://B.example.com ,".to_owned()),
            "PENFII_ALLOWED_ORIGINS",
        )
        .unwrap();
        let h = headers(Some("https://b.example.com"), "core.internal:8080");
        assert!(origin_allowed(&h, &listed));
    }

    #[test]
    fn other_sites_are_refused_even_when_they_are_on_the_same_domain_or_host() {
        for origin in [
            "https://evil.example.org",
            "https://evil.penfii.example.com",
            "http://penfii.example.com:3000",
            "null",
            "file://",
            "https://penfii.example.com.evil.org",
        ] {
            let h = headers(Some(origin), "penfii.example.com");
            assert!(!origin_allowed(&h, &dev()), "{origin}");
        }
    }

    #[test]
    fn a_client_without_an_origin_is_not_a_browser() {
        assert!(origin_allowed(&headers(None, "penfii.example.com"), &[]));
        assert!(matches!(
            request_origin(&headers(None, "penfii.example.com")),
            RequestOrigin::Absent
        ));
    }

    #[test]
    fn an_origin_header_that_is_not_text_is_refused_not_read_as_absent() {
        let mut h = headers(None, "penfii.example.com");
        h.insert("origin", HeaderValue::from_bytes(b"https://\xff").unwrap());
        assert!(matches!(request_origin(&h), RequestOrigin::Unreadable));
        assert!(!origin_allowed(&h, &[]));
    }

    #[test]
    fn the_header_is_normalised_the_same_way_for_every_caller() {
        let h = headers(Some("  HTTPS://Penfii.Example.com/ "), "x");
        let RequestOrigin::Present(origin) = request_origin(&h) else {
            panic!("present");
        };
        assert_eq!(origin, "https://penfii.example.com");
        assert_eq!(normalize("https://penfii.example.com"), origin);
    }

    #[test]
    fn an_empty_or_null_origin_matches_nothing() {
        for origin in ["", "null", "NULL"] {
            let h = headers(Some(origin), "penfii.example.com");
            assert!(!origin_allowed(&h, &dev()), "{origin:?}");
        }
    }
}
