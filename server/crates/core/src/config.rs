//! Reading settings from the environment, strictly.
//!
//! The rules (also in `docs/configuration.md`):
//! - a variable that is not set, or is empty or only blank, is **unset**: Compose passes the optional ones as
//!   `${VAR:-}`, an empty value, and the setting then keeps its documented default or stays off;
//! - a variable that is set must be readable: anything else (a word where a number belongs, a typo in a
//!   policy name) stops the start with a message that names the variable, instead of being ignored or
//!   falling back to a default;
//! - a group of settings that belong together (a credential pair, an endpoint with its keys) is set
//!   completely or not at all.
//!
//! Every reader takes a `lookup` (the process environment in production, a closure in tests), so tests
//! never touch the environment of the process they share with other tests.

/// Where a setting is looked up: `None` is not set.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process environment.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The value, trimmed; `None` when not set or empty.
pub fn get(lookup: Lookup<'_>, name: &str) -> Option<String> {
    lookup(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// A whole number of at least `min`.
pub fn integer(lookup: Lookup<'_>, name: &str, min: i64) -> Result<Option<i64>, String> {
    let Some(raw) = get(lookup, name) else {
        return Ok(None);
    };
    match raw.parse::<i64>() {
        Ok(value) if value >= min => Ok(Some(value)),
        _ => Err(format!(
            "{name} must be a whole number of at least {min}, got {raw:?}"
        )),
    }
}

/// A switch: `1`, `true`, `yes`, `on` or `0`, `false`, `no`, `off`, in any case.
pub fn boolean(lookup: Lookup<'_>, name: &str) -> Result<Option<bool>, String> {
    let Some(raw) = get(lookup, name) else {
        return Ok(None);
    };
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(format!(
            "{name} must be one of 1, true, yes, on, 0, false, no, off, got {raw:?}"
        )),
    }
}

/// One of the words in `allowed`, exactly.
pub fn choice(
    lookup: Lookup<'_>,
    name: &str,
    allowed: &[&'static str],
) -> Result<Option<&'static str>, String> {
    let Some(raw) = get(lookup, name) else {
        return Ok(None);
    };
    allowed
        .iter()
        .find(|word| **word == raw)
        .copied()
        .map(Some)
        .ok_or_else(|| format!("{name} must be one of {}, got {raw:?}", allowed.join(", ")))
}

/// The values of a group that must be set completely or not at all: all of them, or `None` when none is set.
/// Some but not all is an error that names what is missing.
pub fn group(
    lookup: Lookup<'_>,
    what: &str,
    names: &[&str],
) -> Result<Option<Vec<String>>, String> {
    let values: Vec<Option<String>> = names.iter().map(|name| get(lookup, name)).collect();
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let missing: Vec<&str> = names
        .iter()
        .zip(&values)
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| *name)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{what} is set only in part: {} is missing",
            missing.join(", ")
        ));
    }
    Ok(Some(values.into_iter().flatten().collect()))
}

/// A comma separated list of origins (`https://host[:port]`, no path), normalised as the server compares them.
pub fn origins(lookup: Lookup<'_>, name: &str) -> Result<Vec<String>, String> {
    let Some(raw) = get(lookup, name) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for part in raw.split(',') {
        let origin = part.trim().trim_end_matches('/').to_ascii_lowercase();
        if origin.is_empty() {
            continue;
        }
        let host = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"));
        match host {
            Some(host) if !host.is_empty() && !host.contains(['/', ' ', '?', '#', '@']) => {
                out.push(origin);
            }
            _ => {
                return Err(format!(
                    "{name} must list origins like https://app.example.com (no path), got {part:?}"
                ));
            }
        }
    }
    Ok(out)
}

/// A comma separated list of host names.
pub fn hosts(lookup: Lookup<'_>, name: &str) -> Result<Vec<String>, String> {
    let Some(raw) = get(lookup, name) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for part in raw.split(',') {
        let host = part.trim().to_ascii_lowercase();
        if host.is_empty() {
            continue;
        }
        if !host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']'))
        {
            return Err(format!("{name} must list host names, got {part:?}"));
        }
        out.push(host);
    }
    Ok(out)
}

/// `PENFII_WEBHOOK_KEK`: 32 bytes as 64 hex characters. Not set: `None` (Signature webhooks then cannot be made
/// or verified); set but unreadable: an error.
pub fn webhook_kek(lookup: Lookup<'_>) -> Result<Option<[u8; 32]>, String> {
    let Some(raw) = get(lookup, "PENFII_WEBHOOK_KEK") else {
        return Ok(None);
    };
    let bad = || {
        "PENFII_WEBHOOK_KEK must be 32 bytes as 64 hex characters (openssl rand -hex 32)".to_owned()
    };
    let bytes = hex::decode(&raw).map_err(|_| bad())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| bad())?;
    Ok(Some(bytes))
}

/// The settings that are read later, where they are used (a request, a sweep), checked once at the start so
/// that a wrong value stops the start instead of being met by the first request. `0` where a setting says it
/// means "the default" stays allowed.
pub fn startup_checks(lookup: Lookup<'_>) -> Result<(), String> {
    for name in [
        "PENFII_TRASH_RETENTION_DAYS",
        "PENFII_UNBOUND_UPLOAD_GRACE_SECONDS",
        "PENFII_REAUTH_WINDOW_SECONDS",
        "PENFII_MCP_ACCESS_TTL_SECONDS",
        "PENFII_MCP_REFRESH_IDLE_SECONDS",
        "PENFII_MCP_REFRESH_ABSOLUTE_SECONDS",
        "PENFII_MCP_LOG_RETENTION_DAYS",
        "PENFII_MCP_AUDIT_LOG_RETENTION_DAYS",
        "PENFII_MCP_GRANT_RETENTION_DAYS",
    ] {
        integer(lookup, name, 0)?;
    }
    for name in [
        "PENFII_MCP_ENABLED",
        "PENFII_MCP_DCR_ENABLED",
        "PENFII_COOKIE_SECURE",
        "PENFII_WORKER",
        "PENFII_DEV_SEED",
    ] {
        boolean(lookup, name)?;
    }
    choice(
        lookup,
        "PENFII_MCP_DEFAULT_POLICY",
        &["standard", "read_only", "closed"],
    )?;
    origins(lookup, "PENFII_ALLOWED_ORIGINS")?;
    origins(lookup, "PENFII_MCP_ALLOWED_ORIGINS")?;
    hosts(lookup, "PENFII_MCP_CIMD_PRIVATE_HOSTS")?;
    webhook_kek(lookup)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn with<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |name| map.get(name).map(|value| (*value).to_owned())
    }

    #[test]
    fn empty_and_blank_are_unset_and_a_set_value_is_read_or_refused_by_name() {
        let env = with(&[
            ("A", ""),
            ("B", "  "),
            ("C", " 12 "),
            ("D", "twelve"),
            ("E", "-1"),
        ]);
        assert_eq!(get(&env, "A"), None);
        assert_eq!(get(&env, "B"), None);
        assert_eq!(get(&env, "missing"), None);
        assert_eq!(integer(&env, "A", 1), Ok(None));
        assert_eq!(integer(&env, "C", 1), Ok(Some(12)));
        assert_eq!(integer(&env, "C", 0), Ok(Some(12)));
        assert!(integer(&env, "D", 0).unwrap_err().starts_with("D must be"));
        assert!(integer(&env, "E", 0).unwrap_err().contains('E'));
        assert!(integer(&with(&[("Z", "0")]), "Z", 1).is_err());
        assert_eq!(integer(&with(&[("Z", "0")]), "Z", 0), Ok(Some(0)));
    }

    #[test]
    fn a_boolean_takes_every_spelling_of_yes_and_no_in_any_case_and_nothing_else() {
        for yes in ["1", "true", "TRUE", "True", "yes", "Yes", "on", "ON"] {
            assert_eq!(boolean(&with(&[("F", yes)]), "F"), Ok(Some(true)), "{yes}");
        }
        for no in ["0", "false", "False", "no", "NO", "off", "Off"] {
            assert_eq!(boolean(&with(&[("F", no)]), "F"), Ok(Some(false)), "{no}");
        }
        assert_eq!(boolean(&with(&[("F", "")]), "F"), Ok(None));
        for bad in ["2", "y", "enabled", "tru", "truee"] {
            assert!(
                boolean(&with(&[("F", bad)]), "F")
                    .unwrap_err()
                    .contains("F must be"),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_choice_never_falls_back_to_another_word() {
        let words = ["standard", "read_only", "closed"];
        assert_eq!(
            choice(&with(&[("P", "read_only")]), "P", &words),
            Ok(Some("read_only"))
        );
        assert_eq!(choice(&with(&[("P", "")]), "P", &words), Ok(None));
        for typo in ["readonly", "Read_Only", "strict", "closed "] {
            let result = choice(&with(&[("P", typo)]), "P", &words);
            if typo == "closed " {
                assert_eq!(
                    result,
                    Ok(Some("closed")),
                    "a blank around the word is trimmed"
                );
            } else {
                assert!(result.unwrap_err().contains("P must be one of"), "{typo}");
            }
        }
    }

    #[test]
    fn a_group_is_set_completely_or_not_at_all() {
        let names = ["ID", "SECRET"];
        assert_eq!(group(&with(&[]), "login", &names), Ok(None));
        assert_eq!(
            group(&with(&[("ID", ""), ("SECRET", "")]), "login", &names),
            Ok(None)
        );
        assert_eq!(
            group(&with(&[("ID", "a"), ("SECRET", "b")]), "login", &names),
            Ok(Some(vec!["a".into(), "b".into()]))
        );
        let error = group(&with(&[("ID", "a")]), "login", &names).unwrap_err();
        assert_eq!(error, "login is set only in part: SECRET is missing");
        assert!(group(&with(&[("SECRET", "b"), ("ID", " ")]), "login", &names).is_err());
    }

    #[test]
    fn origins_hosts_and_the_webhook_key_are_checked() {
        assert_eq!(
            origins(
                &with(&[("O", "https://App.Example.com/, http://localhost:5173")]),
                "O"
            ),
            Ok(vec![
                "https://app.example.com".into(),
                "http://localhost:5173".into()
            ])
        );
        assert_eq!(origins(&with(&[("O", "")]), "O"), Ok(vec![]));
        for bad in ["app.example.com", "https://", "https://a/b", "ftp://a"] {
            assert!(origins(&with(&[("O", bad)]), "O").is_err(), "{bad}");
        }
        assert_eq!(
            hosts(&with(&[("H", "a.example, B.example:8443")]), "H"),
            Ok(vec!["a.example".into(), "b.example:8443".into()])
        );
        assert!(hosts(&with(&[("H", "a b")]), "H").is_err());

        let key = "ab".repeat(32);
        assert_eq!(webhook_kek(&with(&[])), Ok(None));
        assert_eq!(webhook_kek(&with(&[("PENFII_WEBHOOK_KEK", "")])), Ok(None));
        assert_eq!(
            webhook_kek(&with(&[("PENFII_WEBHOOK_KEK", &key)])).unwrap(),
            Some([0xab; 32])
        );
        for bad in ["abcd", &"zz".repeat(32), &"ab".repeat(33)] {
            assert!(
                webhook_kek(&with(&[("PENFII_WEBHOOK_KEK", bad)]))
                    .unwrap_err()
                    .starts_with("PENFII_WEBHOOK_KEK"),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_start_checks_pass_on_what_compose_passes_and_name_what_is_wrong() {
        // Compose passes optional variables as empty values.
        let compose = with(&[
            ("PENFII_MCP_ENABLED", "0"),
            ("PENFII_MCP_DEFAULT_POLICY", "standard"),
            ("PENFII_MCP_DCR_ENABLED", "0"),
            ("PENFII_WEBHOOK_KEK", ""),
            ("PENFII_TRASH_RETENTION_DAYS", ""),
            ("PENFII_ALLOWED_ORIGINS", "http://localhost:8080"),
        ]);
        assert_eq!(startup_checks(&compose), Ok(()));
        assert_eq!(startup_checks(&with(&[])), Ok(()));

        for (name, value) in [
            ("PENFII_TRASH_RETENTION_DAYS", "thirty"),
            ("PENFII_REAUTH_WINDOW_SECONDS", "-5"),
            ("PENFII_MCP_ACCESS_TTL_SECONDS", "1h"),
            ("PENFII_MCP_ENABLED", "enabled"),
            ("PENFII_COOKIE_SECURE", "sometimes"),
            ("PENFII_WORKER", "2"),
            ("PENFII_DEV_SEED", "maybe"),
            ("PENFII_MCP_DEFAULT_POLICY", "readonly"),
            ("PENFII_ALLOWED_ORIGINS", "penfii.example.com"),
            ("PENFII_WEBHOOK_KEK", "not hex"),
        ] {
            let error = startup_checks(&with(&[(name, value)])).unwrap_err();
            assert!(error.contains(name), "{name}: {error}");
        }
    }

    // The readers built on these, with the cases the review found: a typo that used to fall back, a group
    // that used to switch itself off, a number that used to be ignored.

    #[test]
    fn limits_take_numbers_and_refuse_everything_else_by_name() {
        use crate::Limits;
        let none = Limits::from_lookup(&with(&[])).unwrap();
        assert_eq!(
            (
                none.session_ttl_seconds,
                none.file_max_bytes,
                none.invite_ttl_seconds
            ),
            (None, None, None)
        );
        let compose = Limits::from_lookup(&with(&[
            ("PENFII_SESSION_TTL_SECONDS", "43200"),
            ("PENFII_FILE_MAX_BYTES", "104857600"),
            ("PENFII_INVITE_TTL_SECONDS", "604800"),
            ("PENFII_MESSAGE_FILE_MAX_COUNT", "0"),
            ("PENFII_MESSAGE_BODY_MAX_BYTES", ""),
        ]))
        .unwrap();
        assert_eq!(
            (
                compose.session_ttl_seconds,
                compose.file_max_bytes,
                compose.invite_ttl_seconds
            ),
            (Some(43200), Some(104857600), Some(604800))
        );
        assert_eq!(
            compose.message_file_max_count, None,
            "0 stays 'unset', as the documentation says"
        );
        assert_eq!(compose.message_body_max_bytes, None);
        for (name, value) in [
            ("PENFII_FILE_MAX_BYTES", "100MB"),
            ("PENFII_SESSION_TTL_SECONDS", "-1"),
            ("PENFII_INVITE_TTL_SECONDS", "a week"),
        ] {
            let error = Limits::from_lookup(&with(&[(name, value)])).unwrap_err();
            assert!(error.starts_with(name), "{error}");
        }
    }

    #[test]
    fn a_policy_typo_is_refused_instead_of_becoming_the_policy_that_allows_most() {
        use crate::oauth::policy::PolicyDefaults;
        let standard = PolicyDefaults::from_lookup(&with(&[])).unwrap();
        assert!(standard.allow_write);
        assert!(
            PolicyDefaults::from_lookup(&with(&[("PENFII_MCP_DEFAULT_POLICY", "")]))
                .unwrap()
                .allow_write
        );
        let read_only =
            PolicyDefaults::from_lookup(&with(&[("PENFII_MCP_DEFAULT_POLICY", "read_only")]))
                .unwrap();
        assert!(!read_only.allow_write);
        let closed =
            PolicyDefaults::from_lookup(&with(&[("PENFII_MCP_DEFAULT_POLICY", "closed")])).unwrap();
        assert!(!closed.mcp_enabled && !closed.allow_write);
        for typo in ["readonly", "Read_Only", "locked"] {
            let error = PolicyDefaults::from_lookup(&with(&[("PENFII_MCP_DEFAULT_POLICY", typo)]))
                .unwrap_err();
            assert!(
                error.starts_with("PENFII_MCP_DEFAULT_POLICY must be one of"),
                "{error}"
            );
        }
        assert!(
            crate::oauth::retention::Retention::from_lookup(&with(&[(
                "PENFII_MCP_LOG_RETENTION_DAYS",
                "x"
            )]))
            .is_err()
        );
        assert_eq!(
            crate::oauth::retention::Retention::from_lookup(&with(&[(
                "PENFII_MCP_LOG_RETENTION_DAYS",
                "0"
            )]))
            .unwrap()
            .read_log_days,
            30
        );
    }

    #[test]
    fn object_storage_and_search_are_set_completely_or_not_at_all() {
        use crate::files::ObjectStore;
        use crate::search::client::OsConfig;
        assert!(ObjectStore::from_lookup(&with(&[])).unwrap().is_none());
        // The Compose file passes the S3 settings only when it has them: all four, or empty values.
        assert!(
            ObjectStore::from_lookup(&with(&[
                ("PENFII_S3_ENDPOINT", ""),
                ("PENFII_S3_BUCKET", "")
            ]))
            .unwrap()
            .is_none()
        );
        let error = ObjectStore::from_lookup(&with(&[
            ("PENFII_S3_ENDPOINT", "http://minio:9000"),
            ("PENFII_S3_BUCKET", "penfii"),
        ]))
        .err()
        .unwrap();
        assert_eq!(
            error,
            "object storage is set only in part: PENFII_S3_ACCESS_KEY, PENFII_S3_SECRET_KEY is missing"
        );
        let whole = with(&[
            ("PENFII_S3_ENDPOINT", "http://minio:9000"),
            ("PENFII_S3_BUCKET", "penfii"),
            ("PENFII_S3_ACCESS_KEY", "key"),
            ("PENFII_S3_SECRET_KEY", "secret"),
        ]);
        assert!(ObjectStore::from_lookup(&whole).unwrap().is_some());

        assert!(OsConfig::from_lookup(&with(&[])).unwrap().is_none());
        let url_only =
            OsConfig::from_lookup(&with(&[("PENFII_OPENSEARCH_URL", "http://os:9200/")]))
                .unwrap()
                .unwrap();
        assert_eq!(
            (url_only.url.as_str(), url_only.user.is_none()),
            ("http://os:9200", true)
        );
        assert!(
            OsConfig::from_lookup(&with(&[
                ("PENFII_OPENSEARCH_URL", "http://os:9200"),
                ("PENFII_OPENSEARCH_USER", "u")
            ]))
            .is_err()
        );
        assert!(
            OsConfig::from_lookup(&with(&[
                ("PENFII_OPENSEARCH_USER", "u"),
                ("PENFII_OPENSEARCH_PASSWORD", "p")
            ]))
            .is_err()
        );
        let with_login = OsConfig::from_lookup(&with(&[
            ("PENFII_OPENSEARCH_URL", "http://os:9200"),
            ("PENFII_OPENSEARCH_USER", "u"),
            ("PENFII_OPENSEARCH_PASSWORD", "p"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(with_login.user.as_deref(), Some("u"));
    }

    #[test]
    fn sign_in_providers_are_set_up_completely_or_the_start_is_refused() {
        use crate::identity::provider::{AuthProviders, HttpTransport};
        use std::sync::Arc;
        let build = |pairs: &[(&str, &str)]| {
            AuthProviders::from_lookup(Arc::new(HttpTransport::new()), &with(pairs))
        };
        assert!(build(&[]).is_ok());
        assert!(
            build(&[
                ("PENFII_AUTH_GOOGLE_CLIENT_ID", ""),
                ("PENFII_AUTH_GOOGLE_CLIENT_SECRET", "")
            ])
            .is_ok()
        );
        assert!(
            build(&[
                ("PENFII_PUBLIC_URL", "https://penfii.example.com"),
                ("PENFII_AUTH_GOOGLE_CLIENT_ID", "id"),
                ("PENFII_AUTH_GOOGLE_CLIENT_SECRET", "secret"),
                ("PENFII_AUTH_GOOGLE_SIGNUP", "Yes"),
            ])
            .is_ok()
        );
        let half = build(&[("PENFII_AUTH_GOOGLE_CLIENT_ID", "id")])
            .err()
            .unwrap();
        assert_eq!(
            half,
            "Google sign-in is set only in part: PENFII_AUTH_GOOGLE_CLIENT_SECRET is missing"
        );
        let typo = build(&[("PENFII_AUTH_GOOGLE_SIGNUP", "sometimes")])
            .err()
            .unwrap();
        assert!(
            typo.starts_with("PENFII_AUTH_GOOGLE_SIGNUP must be one of"),
            "{typo}"
        );
        // Microsoft's multi-tenant endpoints need an allow-list; without one the start is refused.
        let microsoft = [
            ("PENFII_AUTH_MICROSOFT_CLIENT_ID", "id"),
            ("PENFII_AUTH_MICROSOFT_CLIENT_SECRET", "secret"),
        ];
        assert!(
            build(&microsoft)
                .err()
                .unwrap()
                .starts_with("Microsoft sign-in is set up but cannot be used")
        );
        let mut allowed = microsoft.to_vec();
        allowed.push(("PENFII_AUTH_MICROSOFT_ALLOWED_TENANTS", "tenant-a"));
        assert!(build(&allowed).is_ok());
        // A generic OIDC provider named in the list needs its issuer and its pair.
        let listed = [("PENFII_AUTH_OIDC_PROVIDERS", "corp")];
        assert!(
            build(&listed)
                .err()
                .unwrap()
                .contains("PENFII_AUTH_OIDC_CORP_ISSUER is not set")
        );
        let mut oidc = listed.to_vec();
        oidc.push(("PENFII_AUTH_OIDC_CORP_ISSUER", "https://idp.example.com"));
        assert!(
            build(&oidc)
                .err()
                .unwrap()
                .contains("has no client id and secret")
        );
        oidc.push(("PENFII_AUTH_OIDC_CORP_CLIENT_ID", "id"));
        oidc.push(("PENFII_AUTH_OIDC_CORP_CLIENT_SECRET", "secret"));
        assert!(build(&oidc).is_ok());
        let mut plain = oidc.clone();
        plain[1] = ("PENFII_AUTH_OIDC_CORP_ISSUER", "http://idp.example.com");
        assert!(
            build(&plain)
                .err()
                .unwrap()
                .contains("must be an https:// URL")
        );
    }
}
