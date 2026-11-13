//! Outgoing e-mail: the SMTP settings, one `Mailer`, and the texts of the messages Penfii sends
//! (an invitation, a password reset).
//!
//! E-mail is optional. Without `PENFII_SMTP_HOST` there is no `Mailer`, and everything that can be
//! sent also works without it (an invitation link is copied, a password is set with `admin-cli`).
//!
//! Security rules that hold for every message:
//! - TLS is verified against the system roots, with the host name of `PENFII_SMTP_HOST`. There is no
//!   setting that turns the certificate or host name check off.
//! - Plain SMTP is only used when `PENFII_SMTP_SECURITY=none` is chosen on purpose, and never together
//!   with a user name and password (those would cross the network in the clear).
//! - The password of the SMTP account, and the secret links inside the messages, are never logged and
//!   never part of an error that is shown or stored: `Mailer::send` answers with a short fixed phrase.

use std::time::Duration;

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// Seconds one message may take, connecting and talking to the server included.
const DEFAULT_TIMEOUT_SECONDS: u64 = 10;

/// How the connection to the SMTP server is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Plain connection upgraded with STARTTLS; the upgrade is required, never optional.
    StartTls,
    /// TLS from the first byte (SMTPS, usually port 465).
    Tls,
    /// No TLS at all, for a relay inside a trusted network. Never with credentials.
    None,
}

/// What the environment says about e-mail.
#[derive(Debug, PartialEq, Eq)]
pub enum MailSetup {
    /// `PENFII_SMTP_HOST` is not set, or the settings are incomplete (the reason is in the text):
    /// e-mail is off, which is a normal state.
    Off(Option<String>),
    On(SmtpConfig),
}

#[derive(PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub credentials: Option<(String, String)>,
    pub from: String,
    /// The origin that links in the messages start with (`PENFII_PUBLIC_URL`, no trailing slash).
    pub public_url: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The password is never printed.
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("user", &self.credentials.as_ref().map(|(user, _)| user))
            .field("from", &self.from)
            .field("public_url", &self.public_url)
            .finish_non_exhaustive()
    }
}

impl SmtpConfig {
    /// `PENFII_SMTP_HOST`, `_PORT`, `_SECURITY` (`starttls` by default, `tls`, `none`), `_USER`,
    /// `_PASSWORD`, `_FROM`, `_TIMEOUT_SECONDS`, and `PENFII_PUBLIC_URL` for the links.
    ///
    /// A value that cannot be right (a bad port, an unknown security mode, a user name without a
    /// password, credentials with plain SMTP, a sender that is not an address) is an error, so a typo
    /// is found at start-up. A missing sender or public URL leaves e-mail off with the reason.
    pub fn from_env() -> Result<MailSetup, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<MailSetup, String> {
        let get = |name: &str| {
            lookup(name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let Some(host) = get("PENFII_SMTP_HOST") else {
            return Ok(MailSetup::Off(None));
        };
        if host.contains(['/', ' ', '@']) {
            return Err("PENFII_SMTP_HOST must be a host name, without a scheme or a path".into());
        }
        let security = match get("PENFII_SMTP_SECURITY").as_deref() {
            None | Some("starttls") => Security::StartTls,
            Some("tls") => Security::Tls,
            Some("none") => Security::None,
            Some(other) => {
                return Err(format!(
                    "PENFII_SMTP_SECURITY must be starttls, tls or none, got {other:?}"
                ));
            }
        };
        let port = match get("PENFII_SMTP_PORT") {
            Some(raw) => raw
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| format!("PENFII_SMTP_PORT must be a port number, got {raw:?}"))?,
            None => match security {
                Security::StartTls => 587,
                Security::Tls => 465,
                Security::None => 25,
            },
        };
        let credentials = match (get("PENFII_SMTP_USER"), get("PENFII_SMTP_PASSWORD")) {
            (None, None) => None,
            (Some(user), Some(password)) => Some((user, password)),
            _ => {
                return Err(
                    "PENFII_SMTP_USER and PENFII_SMTP_PASSWORD are set together or not at all"
                        .into(),
                );
            }
        };
        if credentials.is_some() && security == Security::None {
            return Err("PENFII_SMTP_SECURITY=none cannot be used with PENFII_SMTP_USER and PENFII_SMTP_PASSWORD: the password would cross the network in the clear; use starttls or tls".into());
        }
        let timeout = match get("PENFII_SMTP_TIMEOUT_SECONDS") {
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|seconds| (1..=120).contains(seconds))
                .ok_or_else(|| {
                    format!("PENFII_SMTP_TIMEOUT_SECONDS must be from 1 to 120, got {raw:?}")
                })?,
            None => DEFAULT_TIMEOUT_SECONDS,
        };
        let Some(from) = get("PENFII_SMTP_FROM") else {
            return Ok(MailSetup::Off(Some(
                "PENFII_SMTP_FROM is not set (the address mail is sent from)".into(),
            )));
        };
        if from.parse::<Mailbox>().is_err() {
            return Err(format!(
                "PENFII_SMTP_FROM must be an address such as penfii@example.com, got {from:?}"
            ));
        }
        let Some(public_url) = get("PENFII_PUBLIC_URL") else {
            return Ok(MailSetup::Off(Some(
                "PENFII_PUBLIC_URL is not set (links in the mail start with it)".into(),
            )));
        };
        let public_url = public_url.trim_end_matches('/').to_string();
        if !(public_url.starts_with("https://") || public_url.starts_with("http://"))
            || public_url.contains(['?', '#'])
            || public_url
                .split_once("://")
                .is_none_or(|(_, rest)| rest.is_empty() || rest.contains('/'))
        {
            return Ok(MailSetup::Off(Some(format!(
                "PENFII_PUBLIC_URL must be an origin such as https://penfii.example.com (no path), got {public_url:?}"
            ))));
        }
        Ok(MailSetup::On(SmtpConfig {
            host,
            port,
            security,
            credentials,
            from,
            public_url,
            timeout: Duration::from_secs(timeout),
        }))
    }
}

/// Sends mail through one SMTP server.
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    public_url: String,
    timeout: Duration,
}

impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailer")
            .field("from", &self.from.to_string())
            .finish_non_exhaustive()
    }
}

impl Mailer {
    pub fn new(config: &SmtpConfig) -> Result<Self, String> {
        let from: Mailbox = config
            .from
            .parse()
            .map_err(|_| "PENFII_SMTP_FROM is not an address".to_string())?;
        // `starttls_relay` and `relay` check the server certificate against the system roots and the
        // host name given here. Nothing below weakens that.
        let builder = match config.security {
            Security::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .map_err(|_| "PENFII_SMTP_HOST is not usable for TLS".to_string())?
            }
            Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|_| "PENFII_SMTP_HOST is not usable for TLS".to_string())?,
            // The name says "dangerous" because the connection is not encrypted: it is the explicit
            // `PENFII_SMTP_SECURITY=none` choice, and `from_lookup` has refused credentials with it.
            Security::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(config.host.as_str())
            }
        };
        let mut builder = builder.port(config.port).timeout(Some(config.timeout));
        if let Some((user, password)) = &config.credentials {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from,
            public_url: config.public_url.clone(),
            timeout: config.timeout,
        })
    }

    /// A link into this instance: the public origin and a path that starts with `/`.
    pub fn link(&self, path: &str) -> String {
        format!("{}{path}", self.public_url)
    }

    /// Sends one plain-text message. On failure the answer is a short fixed phrase that is safe to
    /// store and show: it never carries the server's reply, the address or any secret.
    pub async fn send(&self, to: &str, subject: &str, body: String) -> Result<(), &'static str> {
        let to: Mailbox = to.parse().map_err(|_| "invalid recipient address")?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(single_line(subject, 120))
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .map_err(|_| "message could not be built")?;
        // The transport has its own socket timeout; this one bounds the whole exchange.
        match tokio::time::timeout(self.timeout, self.transport.send(message)).await {
            Err(_) => Err("the SMTP server did not answer in time"),
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => {
                // The detail goes to the server log (it holds no secret: lettre does not echo the
                // credentials or the message), the caller gets a phrase.
                tracing::warn!(%error, "SMTP send failed");
                Err(describe(&error))
            }
        }
    }
}

fn describe(error: &lettre::transport::smtp::Error) -> &'static str {
    if error.is_timeout() {
        "the SMTP server did not answer in time"
    } else if error.is_tls() {
        "TLS to the SMTP server failed"
    } else if error.is_permanent() {
        "the SMTP server refused the message"
    } else if error.is_transient() {
        "the SMTP server could not take the message now"
    } else {
        "the SMTP server could not be reached"
    }
}

/// A header value cannot hold a line break; text from people (a Workspace name) is flattened and cut.
fn single_line(text: &str, max_chars: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------------------------
// Texts
// ---------------------------------------------------------------------------------------------

/// A message in one language: the person's own `locale`, else the server's default. Korean unless the
/// locale is English.
fn english(locale: &str) -> bool {
    locale.to_ascii_lowercase().starts_with("en")
}

/// `(subject, body)` of an invitation. `inviter` and `workspace` are text written by people: they
/// only appear inside the body and the subject is cut to one line.
pub fn invitation_message(
    locale: &str,
    workspace: &str,
    inviter: &str,
    link: &str,
    expires_days: i64,
) -> (String, String) {
    let workspace = single_line(workspace, 80);
    let inviter = single_line(inviter, 80);
    if english(locale) {
        (
            format!("{inviter} invited you to {workspace} on Penfii"),
            format!(
                "{inviter} invited you to join {workspace} on Penfii.\n\nOpen this link to accept the invitation:\n{link}\n\nThe link works for {expires_days} days and only for this e-mail address. If you did not expect this, ignore the message.\n"
            ),
        )
    } else {
        (
            format!("{inviter}님이 Penfii의 {workspace}에 초대했습니다"),
            format!(
                "{inviter}님이 Penfii의 {workspace}에 초대했습니다.\n\n아래 링크를 열어 초대를 수락하세요:\n{link}\n\n링크는 {expires_days}일 동안, 이 이메일 주소로만 사용할 수 있습니다. 짐작 가는 바가 없다면 이 메일을 무시하세요.\n"
            ),
        )
    }
}

/// `(subject, body)` of a password reset.
pub fn password_reset_message(locale: &str, link: &str, valid_minutes: i64) -> (String, String) {
    if english(locale) {
        (
            "Reset your Penfii password".to_string(),
            format!(
                "Someone asked to reset the password of your Penfii account.\n\nOpen this link to choose a new password:\n{link}\n\nThe link works once and for {valid_minutes} minutes. Choosing a new password signs you out everywhere. If it was not you, ignore this message: your password stays as it is.\n"
            ),
        )
    } else {
        (
            "Penfii 비밀번호 재설정".to_string(),
            format!(
                "Penfii 계정의 비밀번호 재설정이 요청되었습니다.\n\n아래 링크를 열어 새 비밀번호를 정하세요:\n{link}\n\n링크는 한 번만, {valid_minutes}분 동안 사용할 수 있습니다. 새 비밀번호를 정하면 모든 기기에서 로그아웃됩니다. 본인이 요청한 것이 아니라면 이 메일을 무시하세요. 비밀번호는 그대로입니다.\n"
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(pairs: &[(&str, &str)]) -> Result<MailSetup, String> {
        SmtpConfig::from_lookup(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        })
    }

    const BASE: &[(&str, &str)] = &[
        ("PENFII_SMTP_HOST", "smtp.example.com"),
        ("PENFII_SMTP_FROM", "Penfii <penfii@example.com>"),
        ("PENFII_PUBLIC_URL", "https://penfii.example.com/"),
    ];

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut all: Vec<_> = BASE.to_vec();
        all.extend_from_slice(extra);
        all
    }

    #[test]
    fn no_host_means_mail_is_off_without_a_complaint() {
        assert_eq!(setup(&[]).unwrap(), MailSetup::Off(None));
    }

    #[test]
    fn defaults_are_starttls_on_587_and_the_public_url_loses_its_slash() {
        let MailSetup::On(config) = setup(BASE).unwrap() else {
            panic!("mail should be on");
        };
        assert_eq!(config.security, Security::StartTls);
        assert_eq!(config.port, 587);
        assert_eq!(config.public_url, "https://penfii.example.com");
        assert_eq!(config.timeout, Duration::from_secs(10));
    }

    #[test]
    fn tls_defaults_to_465_and_a_relay_without_tls_to_25() {
        let MailSetup::On(config) = setup(&with(&[("PENFII_SMTP_SECURITY", "tls")])).unwrap()
        else {
            panic!()
        };
        assert_eq!(config.port, 465);
        let MailSetup::On(config) = setup(&with(&[("PENFII_SMTP_SECURITY", "none")])).unwrap()
        else {
            panic!()
        };
        assert_eq!((config.security, config.port), (Security::None, 25));
    }

    #[test]
    fn credentials_with_plain_smtp_are_refused() {
        let error = setup(&with(&[
            ("PENFII_SMTP_SECURITY", "none"),
            ("PENFII_SMTP_USER", "mailer"),
            ("PENFII_SMTP_PASSWORD", "secret"),
        ]))
        .unwrap_err();
        assert!(error.contains("in the clear"));
        assert!(!error.contains("secret"));
    }

    #[test]
    fn a_user_without_a_password_or_the_reverse_is_an_error() {
        assert!(setup(&with(&[("PENFII_SMTP_USER", "mailer")])).is_err());
        assert!(setup(&with(&[("PENFII_SMTP_PASSWORD", "secret")])).is_err());
    }

    #[test]
    fn typos_are_errors_and_missing_pieces_leave_mail_off() {
        assert!(setup(&with(&[("PENFII_SMTP_SECURITY", "ssl")])).is_err());
        assert!(setup(&with(&[("PENFII_SMTP_PORT", "0")])).is_err());
        assert!(setup(&with(&[("PENFII_SMTP_PORT", "smtp")])).is_err());
        assert!(setup(&with(&[("PENFII_SMTP_TIMEOUT_SECONDS", "0")])).is_err());
        assert!(
            setup(&[
                ("PENFII_SMTP_HOST", "smtp://x"),
                ("PENFII_SMTP_FROM", "a@b.c")
            ])
            .is_err()
        );
        assert!(
            setup(&[
                ("PENFII_SMTP_HOST", "smtp.example.com"),
                ("PENFII_SMTP_FROM", "not an address"),
                ("PENFII_PUBLIC_URL", "https://x.example"),
            ])
            .is_err()
        );
        assert!(matches!(
            setup(&[("PENFII_SMTP_HOST", "smtp.example.com")]).unwrap(),
            MailSetup::Off(Some(_))
        ));
        assert!(matches!(
            setup(&[
                ("PENFII_SMTP_HOST", "smtp.example.com"),
                ("PENFII_SMTP_FROM", "penfii@example.com")
            ])
            .unwrap(),
            MailSetup::Off(Some(_))
        ));
        assert!(matches!(
            setup(&[
                ("PENFII_SMTP_HOST", "smtp.example.com"),
                ("PENFII_SMTP_FROM", "penfii@example.com"),
                ("PENFII_PUBLIC_URL", "https://x.example/app"),
            ])
            .unwrap(),
            MailSetup::Off(Some(_))
        ));
    }

    #[test]
    fn debug_output_hides_the_password() {
        let MailSetup::On(config) = setup(&with(&[
            ("PENFII_SMTP_USER", "mailer"),
            ("PENFII_SMTP_PASSWORD", "hunter2-hunter2"),
        ]))
        .unwrap() else {
            panic!()
        };
        assert!(!format!("{config:?}").contains("hunter2"));
    }

    #[test]
    fn the_subject_is_one_line_and_the_language_follows_the_locale() {
        let (subject, body) = invitation_message(
            "en-US",
            "Evil\r\nBcc: x@y.z",
            "Ann",
            "https://p/invite/t",
            7,
        );
        assert!(!subject.contains('\n') && !subject.contains('\r'));
        assert!(body.contains("https://p/invite/t") && body.contains("invited you"));
        let (subject, _) = invitation_message("ko-KR", "팀", "민", "https://p/invite/t", 7);
        assert!(subject.contains("초대"));
        let (subject, body) = password_reset_message("fr", "https://p/reset/t", 30);
        assert!(subject.contains("재설정") && body.contains("30분"));
    }
}
