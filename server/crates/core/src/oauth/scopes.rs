//! Scope registry. Scopes are capabilities, not endpoints, and a scope
//! exists only while at least one operation needs it; the lint test in `penfii-mcp`
//! enforces that. A new scope is added together with the first operation that uses it.

pub struct ScopeInfo {
    pub scope: &'static str,
    /// High-risk scopes: every `*:delete`, every `*:manage`.
    pub high_risk: bool,
}

pub const REGISTRY: &[ScopeInfo] = &[
    ScopeInfo {
        scope: "workspace:read",
        high_risk: false,
    },
    ScopeInfo {
        scope: "member:read",
        high_risk: false,
    },
    ScopeInfo {
        scope: "search:read",
        high_risk: false,
    },
    ScopeInfo {
        scope: "ticket:read",
        high_risk: false,
    },
    ScopeInfo {
        scope: "ticket:write",
        high_risk: false,
    },
    // Creates Projects and changes workflow structure; a *:manage scope is high-risk.
    ScopeInfo {
        scope: "project:manage",
        high_risk: true,
    },
];

pub fn all() -> Vec<&'static str> {
    REGISTRY.iter().map(|info| info.scope).collect()
}

pub fn is_known(scope: &str) -> bool {
    REGISTRY.iter().any(|info| info.scope == scope)
}

pub fn is_high_risk(scope: &str) -> bool {
    REGISTRY
        .iter()
        .any(|info| info.scope == scope && info.high_risk)
}

/// The intent half of `domain:intent`.
fn intent(scope: &str) -> &str {
    scope.split_once(':').map_or("", |(_, intent)| intent)
}

/// Anything but `*:read` changes data. Consent starts these unchecked:
/// clicking Allow on a default request grants reading only.
pub fn is_change(scope: &str) -> bool {
    is_known(scope) && intent(scope) != "read"
}

/// What an unauthenticated challenge and a request without `scope` ask for: every read scope.
/// Real clients request exactly the scope they are told (the TypeScript SDK sent
/// `workspace:read` when the challenge named only that), so a challenge naming too little
/// leaves a connection that can list Workspaces and nothing else. Writes are never part of the
/// default; a client that needs one asks for it (an `insufficient_scope` challenge names it) and
/// the person approves it explicitly.
pub fn default_request() -> Vec<&'static str> {
    REGISTRY
        .iter()
        .filter(|info| intent(info.scope) == "read")
        .map(|info| info.scope)
        .collect()
}

/// The requested scopes intersected with the registry, in registry order and without
/// duplicates. `offline_access` is a protocol scope and is accepted but never stored.
pub fn normalize(requested: &str) -> Vec<String> {
    let wanted: Vec<&str> = requested.split_whitespace().collect();
    REGISTRY
        .iter()
        .filter(|info| wanted.contains(&info.scope))
        .map(|info| info.scope.to_string())
        .collect()
}

/// `manage ⊇ write ⊇ read` inside a domain; `delete` implies nothing.
pub fn grants(have: &[String], needed: &str) -> bool {
    if have.iter().any(|scope| scope == needed) {
        return true;
    }
    let Some((domain, intent)) = needed.split_once(':') else {
        return false;
    };
    let stronger: &[&str] = match intent {
        "read" => &["write", "manage"],
        "write" => &["manage"],
        _ => &[],
    };
    stronger.iter().any(|other| {
        have.iter()
            .any(|scope| scope == &format!("{domain}:{other}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_drops_unknown_and_keeps_registry_order() {
        let got = normalize("ticket:read bogus:admin offline_access workspace:read ticket:read");
        assert_eq!(got, vec!["workspace:read", "ticket:read"]);
        assert!(normalize("").is_empty());
    }

    #[test]
    fn implication_follows_the_scope_hierarchy() {
        let have = vec!["ticket:write".to_string(), "document:delete".to_string()];
        assert!(grants(&have, "ticket:read"));
        assert!(!grants(&have, "document:read"));
        assert!(!grants(&have, "ticket:manage"));
    }
}
