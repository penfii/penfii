//! A small SMTP server for the tests: it speaks just enough plain SMTP for `Mailer` (security
//! `none`), keeps what it was given, and can refuse a message or never answer.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use penfii_core::mail::{Mailer, Security, SmtpConfig};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Takes every message (250).
    Accept,
    /// Answers the message itself with a permanent refusal (550).
    Refuse,
    /// Answers the greeting and then nothing.
    Hang,
}

#[derive(Debug, Clone)]
pub struct Received {
    pub to: String,
    /// The whole message as it came in (headers and body), line breaks as `\n`.
    pub data: String,
}

impl Received {
    /// The text of the message: its body, decoded when the sender encoded it (lettre sends
    /// non-ASCII text as base64), line breaks as `\n`.
    pub fn text(&self) -> String {
        use base64::Engine as _;
        let (head, body) = self.data.split_once("\n\n").unwrap_or((&self.data, ""));
        if head
            .to_ascii_lowercase()
            .contains("content-transfer-encoding: base64")
        {
            let joined: String = body.split_whitespace().collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(joined)
                .expect("base64 body");
            String::from_utf8(bytes).unwrap().replace("\r\n", "\n")
        } else {
            body.to_string()
        }
    }
}

pub struct FakeSmtp {
    pub port: u16,
    modes: Arc<Mutex<Vec<Mode>>>,
    fallback: Arc<Mutex<Mode>>,
    received: Arc<Mutex<Vec<Received>>>,
}

impl FakeSmtp {
    pub async fn start(mode: Mode) -> FakeSmtp {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = FakeSmtp {
            port,
            modes: Arc::default(),
            fallback: Arc::new(Mutex::new(mode)),
            received: Arc::default(),
        };
        let modes = server.modes.clone();
        let fallback = server.fallback.clone();
        let received = server.received.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                // One-off modes are used first, one per connection, then the standing one.
                let mode = {
                    let mut queued = modes.lock().unwrap();
                    if queued.is_empty() {
                        *fallback.lock().unwrap()
                    } else {
                        queued.remove(0)
                    }
                };
                let received = received.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, mode, received).await;
                });
            }
        });
        server
    }

    /// The next connection behaves like `mode`; later ones follow the standing mode again.
    pub fn next(&self, mode: Mode) {
        self.modes.lock().unwrap().push(mode);
    }

    pub fn set(&self, mode: Mode) {
        *self.fallback.lock().unwrap() = mode;
    }

    pub fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    /// A `Mailer` that sends to this server, with links that start with `https://penfii.example.com`.
    pub fn mailer(&self, timeout: Duration) -> Mailer {
        Mailer::new(&SmtpConfig {
            host: "127.0.0.1".into(),
            port: self.port,
            security: Security::None,
            credentials: None,
            from: "Penfii <penfii@example.com>".into(),
            public_url: "https://penfii.example.com".into(),
            timeout,
        })
        .unwrap()
    }
}

async fn serve(
    stream: TcpStream,
    mode: Mode,
    received: Arc<Mutex<Vec<Received>>>,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"220 fake ESMTP\r\n").await?;
    if mode == Mode::Hang {
        tokio::time::sleep(Duration::from_secs(30)).await;
        return Ok(());
    }
    let mut to = String::new();
    while let Some(line) = lines.next_line().await? {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            write.write_all(b"250 fake\r\n").await?;
        } else if upper.starts_with("MAIL FROM") || upper.starts_with("RSET") {
            write.write_all(b"250 ok\r\n").await?;
        } else if upper.starts_with("RCPT TO") {
            to = line
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>'))
                .map(|(address, _)| address.to_string())
                .unwrap_or_default();
            write.write_all(b"250 ok\r\n").await?;
        } else if upper.starts_with("DATA") {
            write.write_all(b"354 go ahead\r\n").await?;
            let mut data = String::new();
            while let Some(row) = lines.next_line().await? {
                if row == "." {
                    break;
                }
                data.push_str(&row);
                data.push('\n');
            }
            if mode == Mode::Refuse {
                write.write_all(b"550 refused\r\n").await?;
            } else {
                received.lock().unwrap().push(Received {
                    to: to.clone(),
                    data,
                });
                write.write_all(b"250 queued\r\n").await?;
            }
        } else if upper.starts_with("QUIT") {
            write.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else {
            write.write_all(b"500 what\r\n").await?;
        }
    }
    Ok(())
}
