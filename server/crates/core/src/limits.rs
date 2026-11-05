/// Operator limits. Missing or non-positive session and message limits stay unset.
#[derive(Debug, Clone)]
pub struct Limits {
    pub session_ttl_seconds: Option<i64>,
    pub message_body_max_bytes: Option<i64>,
    pub invite_ttl_seconds: Option<i64>,
    pub dev_return_invite_token: bool,
    pub file_max_bytes: Option<i64>,
    pub message_file_max_count: Option<i64>,
    pub message_file_max_bytes: Option<i64>,
    pub login_throttle: ThrottleLimits,
}

impl Limits {
    pub fn from_env() -> Self {
        Self {
            session_ttl_seconds: positive_env("PENFII_SESSION_TTL_SECONDS"),
            message_body_max_bytes: positive_env("PENFII_MESSAGE_BODY_MAX_BYTES"),
            invite_ttl_seconds: present_i64("PENFII_INVITE_TTL_SECONDS"),
            dev_return_invite_token: flag_env("PENFII_DEV_RETURN_INVITE_TOKEN"),
            file_max_bytes: positive_env("PENFII_FILE_MAX_BYTES"),
            message_file_max_count: positive_env("PENFII_MESSAGE_FILE_MAX_COUNT"),
            message_file_max_bytes: positive_env("PENFII_MESSAGE_FILE_MAX_BYTES"),
            login_throttle: ThrottleLimits::default(),
        }
    }
}

fn present_i64(name: &str) -> Option<i64> {
    let Ok(raw) = std::env::var(name) else {
        return None;
    };
    raw.trim().parse::<i64>().ok()
}

fn positive_env(name: &str) -> Option<i64> {
    match present_i64(name) {
        Some(value) if value > 0 => Some(value),
        _ => None,
    }
}

fn flag_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| value.trim() == "1")
}

/// How many failed password proofs a key may make in a window before it is locked for that long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub max_failures: i32,
    pub window_seconds: i64,
}

/// The three counters of a sign-in. A failed attempt counts against all of them; the first one that
/// is full refuses the next attempt.
///
/// - `account_address`: one e-mail from one address. Small, so that guessing a password from one
///   place stops quickly, yet another address cannot lock the person out by typing their e-mail.
/// - `account`: one e-mail from anywhere. Larger, so a guess spread over many addresses still stops,
///   while a single address cannot reach it.
/// - `address`: one address for any e-mail (also counts OAuth sign-in starts). Shared addresses
///   (offices, proxies) are common, so it is the largest per attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrottleLimits {
    pub account_address: Bucket,
    pub account: Bucket,
    pub address: Bucket,
}

impl Default for ThrottleLimits {
    fn default() -> Self {
        let window_seconds = 15 * 60;
        Self {
            account_address: Bucket {
                max_failures: 5,
                window_seconds,
            },
            account: Bucket {
                max_failures: 20,
                window_seconds,
            },
            address: Bucket {
                max_failures: 30,
                window_seconds,
            },
        }
    }
}

impl ThrottleLimits {
    /// `PENFII_LOGIN_THROTTLE_{ACCOUNT_ADDRESS,ACCOUNT,ADDRESS}_{FAILURES,WINDOW_SECONDS}`. Unset keeps
    /// the default; a value that is not a positive integer is an error, not a silent default.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let defaults = Self::default();
        let read = |prefix: &str, default: Bucket| -> Result<Bucket, String> {
            let number = |suffix: &str, default: i64| -> Result<i64, String> {
                let name = format!("PENFII_LOGIN_THROTTLE_{prefix}_{suffix}");
                match lookup(&name) {
                    None => Ok(default),
                    Some(raw) => raw
                        .trim()
                        .parse::<i64>()
                        .ok()
                        .filter(|value| (1..=i64::from(i32::MAX)).contains(value))
                        .ok_or_else(|| format!("{name} must be a positive integer, got {raw:?}")),
                }
            };
            Ok(Bucket {
                max_failures: number("FAILURES", i64::from(default.max_failures))? as i32,
                window_seconds: number("WINDOW_SECONDS", default.window_seconds)?,
            })
        };
        Ok(Self {
            account_address: read("ACCOUNT_ADDRESS", defaults.account_address)?,
            account: read("ACCOUNT", defaults.account)?,
            address: read("ADDRESS", defaults.address)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn the_defaults_are_5_per_account_and_address_20_per_account_30_per_address_in_15_minutes() {
        let limits = ThrottleLimits::from_lookup(lookup(&[])).unwrap();
        assert_eq!(limits, ThrottleLimits::default());
        assert_eq!(
            (
                limits.account_address.max_failures,
                limits.account_address.window_seconds
            ),
            (5, 900)
        );
        assert_eq!(
            (limits.account.max_failures, limits.account.window_seconds),
            (20, 900)
        );
        assert_eq!(
            (limits.address.max_failures, limits.address.window_seconds),
            (30, 900)
        );
    }

    #[test]
    fn every_threshold_and_window_can_be_set() {
        let limits = ThrottleLimits::from_lookup(lookup(&[
            ("PENFII_LOGIN_THROTTLE_ACCOUNT_ADDRESS_FAILURES", "3"),
            ("PENFII_LOGIN_THROTTLE_ACCOUNT_ADDRESS_WINDOW_SECONDS", "60"),
            ("PENFII_LOGIN_THROTTLE_ACCOUNT_FAILURES", "10"),
            ("PENFII_LOGIN_THROTTLE_ACCOUNT_WINDOW_SECONDS", "120"),
            ("PENFII_LOGIN_THROTTLE_ADDRESS_FAILURES", "100"),
            ("PENFII_LOGIN_THROTTLE_ADDRESS_WINDOW_SECONDS", "3600"),
        ]))
        .unwrap();
        assert_eq!(
            limits.account_address,
            Bucket {
                max_failures: 3,
                window_seconds: 60
            }
        );
        assert_eq!(
            limits.account,
            Bucket {
                max_failures: 10,
                window_seconds: 120
            }
        );
        assert_eq!(
            limits.address,
            Bucket {
                max_failures: 100,
                window_seconds: 3600
            }
        );
    }

    #[test]
    fn a_value_that_is_not_a_positive_integer_is_an_error() {
        for bad in ["0", "-1", "five", "", "2147483648", "1.5"] {
            let error = ThrottleLimits::from_lookup(lookup(&[(
                "PENFII_LOGIN_THROTTLE_ACCOUNT_FAILURES",
                bad,
            )]))
            .unwrap_err();
            assert!(
                error.contains("PENFII_LOGIN_THROTTLE_ACCOUNT_FAILURES"),
                "{error}"
            );
        }
    }
}
