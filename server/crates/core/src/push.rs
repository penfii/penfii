//! Web Push: subscriptions, the VAPID identity and sending.
//!
//! The Inbox decides what a person is told; this module only carries some kinds of it out of the app
//! (`PUSH_KINDS`). A subscription belongs to a person and to one browser/device. Nothing here writes
//! or reads an Inbox item except to check that it still exists before sending.
//!
//! Encryption of the message (RFC 8291) is `web-push-native`'s; the VAPID token (RFC 8292) is a plain
//! ES256 JWT signed with `jsonwebtoken`. The VAPID private key is a server secret, never stored in
//! the database and never logged.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::pkcs8::EncodePrivateKey;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use web_push_native::{Auth, WebPushBuilder};

use crate::error::CoreError;

/// The Inbox kinds that are also pushed. Anything not listed here is never pushed, including kinds
/// added to the Inbox later, until it is added on purpose. (`dependency.released` is in the Inbox's
/// direct kinds but is not pushed; neither are invitations and general activity.)
pub const PUSH_KINDS: &[&str] = &[
    "mention.user",
    "reply.direct",
    "ticket.assigned",
    "reminder.due",
    "dm.message",
    "thread.reply",
];

pub fn is_push_kind(kind: &str) -> bool {
    PUSH_KINDS.contains(&kind)
}

// ---------------------------------------------------------------------------------------------
// VAPID
// ---------------------------------------------------------------------------------------------

/// A fresh P-256 private key, drawn again in the rare case the bytes are not a valid scalar.
fn random_secret() -> p256::SecretKey {
    loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the system gives random bytes");
        if let Ok(secret) = p256::SecretKey::from_slice(&bytes) {
            return secret;
        }
    }
}

/// The server's Web Push identity: a P-256 key pair and a contact the push service can reach.
pub struct VapidConfig {
    key: EncodingKey,
    public_key: String,
    subject: String,
}

impl std::fmt::Debug for VapidConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The private key is never printed.
        f.debug_struct("VapidConfig")
            .field("public_key", &self.public_key)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct VapidClaims<'a> {
    aud: &'a str,
    exp: i64,
    sub: &'a str,
}

impl VapidConfig {
    /// `PENFII_VAPID_PRIVATE_KEY` (the 32-byte private key, base64url) and `PENFII_VAPID_SUBJECT`
    /// (`mailto:` or `https:` contact). Both unset: push is off. Anything else wrong is an error, so
    /// a typo stops the start instead of silently turning push off.
    pub fn from_env() -> Result<Option<Self>, String> {
        let private = std::env::var("PENFII_VAPID_PRIVATE_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let subject = std::env::var("PENFII_VAPID_SUBJECT")
            .ok()
            .filter(|v| !v.trim().is_empty());
        match (private, subject) {
            (None, None) => Ok(None),
            (Some(private), Some(subject)) => Self::from_parts(&private, &subject).map(Some),
            (Some(_), None) => {
                Err("PENFII_VAPID_SUBJECT is required when PENFII_VAPID_PRIVATE_KEY is set".into())
            }
            (None, Some(_)) => {
                Err("PENFII_VAPID_PRIVATE_KEY is required when PENFII_VAPID_SUBJECT is set".into())
            }
        }
    }

    pub fn from_parts(private_key: &str, subject: &str) -> Result<Self, String> {
        let subject = subject.trim();
        if !(subject.starts_with("mailto:") || subject.starts_with("https://")) {
            return Err("PENFII_VAPID_SUBJECT must start with mailto: or https://".into());
        }
        let raw = URL_SAFE_NO_PAD
            .decode(private_key.trim().trim_end_matches('='))
            .map_err(|_| "PENFII_VAPID_PRIVATE_KEY is not base64url".to_owned())?;
        let secret = p256::SecretKey::from_slice(&raw).map_err(|_| {
            "PENFII_VAPID_PRIVATE_KEY is not a P-256 private key (32 bytes)".to_owned()
        })?;
        let der = secret
            .to_pkcs8_der()
            .map_err(|_| "PENFII_VAPID_PRIVATE_KEY cannot be used for signing".to_owned())?;
        let public = secret.public_key();
        let point = p256::elliptic_curve::sec1::ToSec1Point::to_sec1_point(&public, false);
        Ok(Self {
            key: EncodingKey::from_ec_der(der.as_bytes()),
            public_key: URL_SAFE_NO_PAD.encode(point.as_bytes()),
            subject: subject.to_owned(),
        })
    }

    /// A new key pair, as the two strings to put in the environment: the private key
    /// (`PENFII_VAPID_PRIVATE_KEY`) and the public key a browser subscribes with.
    pub fn generate() -> (String, String) {
        let secret = random_secret();
        let private = URL_SAFE_NO_PAD.encode(secret.to_bytes());
        let point =
            p256::elliptic_curve::sec1::ToSec1Point::to_sec1_point(&secret.public_key(), false);
        (private, URL_SAFE_NO_PAD.encode(point.as_bytes()))
    }

    /// The application server key a browser subscribes with (65-byte uncompressed point, base64url).
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// The `Authorization` header value for a push service at `endpoint`.
    fn authorization(&self, audience: &str) -> Result<String, String> {
        let claims = VapidClaims {
            aud: audience,
            exp: (Utc::now() + chrono::Duration::hours(12)).timestamp(),
            sub: &self.subject,
        };
        let token = jsonwebtoken::encode(&Header::new(Algorithm::ES256), &claims, &self.key)
            .map_err(|e| format!("VAPID token: {e}"))?;
        Ok(format!("vapid t={token}, k={}", self.public_key))
    }
}

// ---------------------------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------------------------

/// What a stored subscription gives a transport.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: Uuid,
    pub user_id: Uuid,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// How one send ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    /// The push service says the subscription no longer exists (404/410): delete it.
    Gone,
    /// Worth another try (timeout, 429, 5xx).
    Retry(String),
    /// Will not work again as it is (other 4xx, a key that cannot be used).
    Failed(String),
}

pub type PushFuture<'a> = Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>;

/// Carries one encrypted message to one subscription. The real one speaks HTTP; tests record.
pub trait PushTransport: Send + Sync {
    fn send<'a>(&'a self, subscription: &'a Subscription, payload: &'a [u8]) -> PushFuture<'a>;
}

pub struct HttpTransport {
    client: reqwest::Client,
    vapid: VapidConfig,
}

impl HttpTransport {
    pub fn new(vapid: VapidConfig) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PublicOnlyResolver))
            .build()
            .map_err(|e| format!("push client: {e}"))?;
        Ok(Self { client, vapid })
    }

    fn request(
        &self,
        subscription: &Subscription,
        payload: &[u8],
    ) -> Result<http::Request<Vec<u8>>, Outcome> {
        let endpoint: http::Uri = subscription
            .endpoint
            .parse()
            .map_err(|_| Outcome::Failed("endpoint is not a URL".into()))?;
        let (Some(scheme), Some(authority)) = (endpoint.scheme_str(), endpoint.authority()) else {
            return Err(Outcome::Failed("endpoint is not absolute".into()));
        };
        let audience = format!("{scheme}://{authority}");
        let public = URL_SAFE_NO_PAD
            .decode(&subscription.p256dh)
            .ok()
            .and_then(|b| web_push_native::p256::PublicKey::from_sec1_bytes(&b).ok())
            .ok_or_else(|| Outcome::Failed("p256dh is not a P-256 public key".into()))?;
        let auth_bytes = URL_SAFE_NO_PAD
            .decode(&subscription.auth)
            .ok()
            .filter(|b| b.len() == 16)
            .ok_or_else(|| Outcome::Failed("auth secret is not 16 bytes".into()))?;
        let mut request =
            WebPushBuilder::new(endpoint, public, Auth::clone_from_slice(&auth_bytes))
                .build(payload.to_vec())
                .map_err(|e| Outcome::Failed(format!("encryption: {e}")))?;
        let authorization = self
            .vapid
            .authorization(&audience)
            .map_err(Outcome::Failed)?;
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&authorization)
                .map_err(|_| Outcome::Failed("authorization".into()))?,
        );
        request
            .headers_mut()
            .insert("Urgency", http::HeaderValue::from_static("high"));
        Ok(request)
    }
}

impl PushTransport for HttpTransport {
    fn send<'a>(&'a self, subscription: &'a Subscription, payload: &'a [u8]) -> PushFuture<'a> {
        Box::pin(async move {
            let request = match self.request(subscription, payload) {
                Ok(request) => request,
                Err(outcome) => return outcome,
            };
            let (parts, body) = request.into_parts();
            let response = self
                .client
                .post(parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await;
            match response {
                Ok(response) => {
                    let code = response.status().as_u16();
                    match code {
                        200..=299 => Outcome::Delivered,
                        404 | 410 => Outcome::Gone,
                        _ => {
                            // The push service says why (Apple: `{"reason":"BadJwtToken"}`); it holds
                            // nothing secret, so a short piece of it goes into the outcome for the log.
                            let body = response.text().await.unwrap_or_default();
                            let why = format!(
                                "status {code}: {}",
                                body.chars().take(200).collect::<String>()
                            );
                            if code == 429 || (500..=599).contains(&code) {
                                Outcome::Retry(why)
                            } else {
                                Outcome::Failed(why)
                            }
                        }
                    }
                }
                // Never print the URL: it identifies the person's device.
                Err(error) if error.is_timeout() || error.is_connect() => {
                    Outcome::Retry("network".into())
                }
                Err(_) => Outcome::Retry("request".into()),
            }
        })
    }
}

/// One message to send to every device of one person.
#[derive(Debug, Clone)]
pub struct PushJob {
    pub recipient: Uuid,
    /// The Inbox item this is about; the job is dropped if it does not exist (its batch rolled back).
    pub inbox_item_id: Uuid,
    pub payload: Value,
}

/// How many batches of pushes may be on their way at the same time. A batch that comes while this many are
/// still being sent is dropped with a warning: push is best effort (the Inbox item exists either way), and a
/// push service that answers slowly must not make the process pile up tasks without end.
pub const MAX_BATCHES_IN_FLIGHT: usize = 64;

/// Subscriptions, the public key and sending, for the process.
pub struct PushService {
    pool: PgPool,
    transport: Arc<dyn PushTransport>,
    public_key: String,
    /// Waits before the second and third attempt of a send that failed in a way worth retrying; then it is given up.
    retry_delays: Vec<Duration>,
    /// The batches being sent (see [`PushService::dispatch`]), kept so that a stop can wait for or end them.
    in_flight: std::sync::Mutex<tokio::task::JoinSet<()>>,
    /// Permits for [`MAX_BATCHES_IN_FLIGHT`]; a batch holds one until it has been sent.
    permits: Arc<tokio::sync::Semaphore>,
    /// Set by [`PushService::drain`]: nothing new is sent after the process started to stop.
    closed: std::sync::atomic::AtomicBool,
}

impl PushService {
    pub fn new(pool: PgPool, vapid: VapidConfig) -> Result<Arc<Self>, String> {
        let public_key = vapid.public_key().to_owned();
        let transport = HttpTransport::new(vapid)?;
        Ok(Arc::new(Self {
            pool,
            transport: Arc::new(transport),
            public_key,
            retry_delays: vec![Duration::from_secs(2), Duration::from_secs(10)],
            in_flight: Default::default(),
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_BATCHES_IN_FLIGHT)),
            closed: Default::default(),
        }))
    }

    /// For tests: any transport, no waiting between attempts.
    pub fn with_transport(
        pool: PgPool,
        public_key: &str,
        transport: Arc<dyn PushTransport>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            transport,
            public_key: public_key.to_owned(),
            retry_delays: vec![Duration::ZERO, Duration::ZERO],
            in_flight: Default::default(),
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_BATCHES_IN_FLIGHT)),
            closed: Default::default(),
        })
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// Sends a batch without making the caller wait (the event consumer has just committed). The batch is a
    /// task kept by the service, so [`PushService::wait_idle`] and [`PushService::drain`] can wait for it.
    /// It is dropped (with a warning) when [`MAX_BATCHES_IN_FLIGHT`] are already on their way or the
    /// service is draining.
    pub fn dispatch(self: &Arc<Self>, jobs: Vec<PushJob>) {
        if jobs.is_empty() {
            return;
        }
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!(
                jobs = jobs.len(),
                "push jobs dropped: the process is stopping"
            );
            return;
        }
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            tracing::warn!(
                jobs = jobs.len(),
                "push jobs dropped: too many sends in flight"
            );
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("push jobs dropped: no runtime");
            return;
        };
        tracing::info!(jobs = jobs.len(), "push jobs queued");
        let service = Arc::clone(self);
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        // Reap what has finished, so the set holds only batches still on their way.
        while in_flight.try_join_next().is_some() {}
        in_flight.spawn_on(
            async move {
                let _permit = permit;
                service.send_all(jobs).await;
            },
            &handle,
        );
    }

    fn take_in_flight(&self) -> tokio::task::JoinSet<()> {
        std::mem::take(&mut *self.in_flight.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Waits until the batches dispatched so far have been sent (or have failed). Tests use it instead of
    /// sleeping; the service keeps working afterwards.
    pub async fn wait_idle(&self) {
        let mut batches = self.take_in_flight();
        while let Some(result) = batches.join_next().await {
            if let Err(error) = result {
                tracing::warn!(?error, "a push batch ended abnormally");
            }
        }
    }

    /// For the end of the process: nothing new is sent, the batches on their way get `within` to finish, and
    /// the ones still running after that are cancelled (a push is best effort; the Inbox item stays). Returns
    /// how many were cancelled. Call it after the event workers have stopped (they are what dispatches) and
    /// before the database pool closes (a batch records its result there).
    pub async fn drain(&self, within: Duration) -> usize {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut batches = self.take_in_flight();
        let waited = tokio::time::timeout(within, async {
            while batches.join_next().await.is_some() {}
        })
        .await;
        if waited.is_ok() {
            return 0;
        }
        let cancelled = batches.len();
        batches.abort_all();
        while batches.join_next().await.is_some() {}
        tracing::warn!(
            cancelled,
            "push batches still on their way were cancelled at shutdown"
        );
        cancelled
    }

    /// Sends and waits. Used by `dispatch` and by tests.
    pub async fn send_all(&self, jobs: Vec<PushJob>) {
        for job in jobs {
            if let Err(error) = self.send_job(&job).await {
                tracing::warn!(?error, "push job failed");
            }
        }
    }

    async fn send_job(&self, job: &PushJob) -> Result<(), CoreError> {
        // The item exists only if its batch committed; a rolled-back batch is not pushed.
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.inbox_items WHERE id=$1)")
                .bind(job.inbox_item_id)
                .fetch_one(&self.pool)
                .await?;
        if !exists {
            return Ok(());
        }
        let subscriptions = subscriptions_for_push(&self.pool, job.recipient).await?;
        if subscriptions.is_empty() {
            tracing::info!(recipient = %job.recipient, "push skipped: no subscription");
            return Ok(());
        }
        let payload =
            serde_json::to_vec(&job.payload).map_err(|e| CoreError::Internal(e.to_string()))?;
        // One device failing does not hold up another.
        let sends = subscriptions
            .iter()
            .map(|subscription| self.send_one(subscription, &payload));
        futures_util::future::join_all(sends).await;
        Ok(())
    }

    async fn send_one(&self, subscription: &Subscription, payload: &[u8]) {
        let mut attempt = 0;
        let outcome = loop {
            let outcome = self.transport.send(subscription, payload).await;
            match (&outcome, self.retry_delays.get(attempt)) {
                (Outcome::Retry(_), Some(delay)) => {
                    attempt += 1;
                    tokio::time::sleep(*delay).await;
                }
                _ => break outcome,
            }
        };
        // Diagnosis: what the push service answered, by its host name only (the address of a
        // device is never logged).
        let host = subscription
            .endpoint
            .parse::<http::Uri>()
            .ok()
            .and_then(|uri| uri.host().map(str::to_owned))
            .unwrap_or_default();
        match &outcome {
            Outcome::Delivered => tracing::info!(push_host = %host, "push delivered"),
            Outcome::Gone => {
                tracing::info!(push_host = %host, "push subscription gone (404/410), removed")
            }
            Outcome::Retry(why) => {
                tracing::warn!(push_host = %host, %why, "push failed after retries")
            }
            Outcome::Failed(why) => tracing::warn!(push_host = %host, %why, "push rejected"),
        }
        let result = match outcome {
            Outcome::Delivered => {
                sqlx::query(
                    "UPDATE penfii.push_subscriptions SET last_success_at=now() WHERE id=$1",
                )
                .bind(subscription.id)
                .execute(&self.pool)
                .await
            }
            Outcome::Gone => {
                sqlx::query("DELETE FROM penfii.push_subscriptions WHERE id=$1")
                    .bind(subscription.id)
                    .execute(&self.pool)
                    .await
            }
            Outcome::Retry(_) | Outcome::Failed(_) => {
                sqlx::query(
                    "UPDATE penfii.push_subscriptions SET last_failure_at=now() WHERE id=$1",
                )
                .bind(subscription.id)
                .execute(&self.pool)
                .await
            }
        };
        if let Err(error) = result {
            tracing::warn!(?error, "push bookkeeping failed");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Subscriptions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionView {
    pub id: Uuid,
    pub endpoint: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
}

pub struct NewSubscription<'a> {
    pub endpoint: &'a str,
    pub p256dh: &'a str,
    pub auth: &'a str,
    pub user_agent: Option<&'a str>,
    pub label: Option<&'a str>,
}

fn validate(new: &NewSubscription<'_>) -> Result<(), CoreError> {
    let endpoint = new.endpoint.trim();
    if endpoint.is_empty() || endpoint.len() > 2048 {
        return Err(CoreError::validation("endpoint is 1 to 2048 characters"));
    }
    let uri: http::Uri = endpoint
        .parse()
        .map_err(|_| CoreError::validation("endpoint is not a URL"))?;
    if uri.scheme_str() != Some("https") || uri.host().is_none() {
        return Err(CoreError::validation("endpoint must be an https URL"));
    }
    // The server will POST to this address, so it must be a public name: no IP literal, no
    // single-label or local host. (The address a name resolves to is checked again when sending.)
    if !is_public_host(uri.host().unwrap_or_default()) {
        return Err(CoreError::validation("endpoint must be a public host name"));
    }
    let key = URL_SAFE_NO_PAD
        .decode(new.p256dh.trim_end_matches('='))
        .ok()
        .and_then(|b| web_push_native::p256::PublicKey::from_sec1_bytes(&b).ok());
    if key.is_none() {
        return Err(CoreError::validation("p256dh is not a P-256 public key"));
    }
    let auth_ok = URL_SAFE_NO_PAD
        .decode(new.auth.trim_end_matches('='))
        .is_ok_and(|b| b.len() == 16);
    if !auth_ok {
        return Err(CoreError::validation("auth is not a 16-byte secret"));
    }
    if new.label.is_some_and(|l| l.chars().count() > 100) {
        return Err(CoreError::validation("label is at most 100 characters"));
    }
    if new.user_agent.is_some_and(|l| l.chars().count() > 400) {
        return Err(CoreError::validation("userAgent is at most 400 characters"));
    }
    Ok(())
}

/// Most devices one person can have; a runaway client cannot grow the table or the sends of one person without bound.
const MAX_SUBSCRIPTIONS_PER_USER: i64 = 20;

/// A host name that can be a push service: not an IP literal, not `localhost`, not a single label,
/// not an internal suffix.
fn is_public_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return false;
    }
    if !host.contains('.') {
        return false;
    }
    ![
        "localhost",
        "local",
        "internal",
        "lan",
        "home",
        "corp",
        "intranet",
    ]
    .iter()
    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

/// Whether a resolved address is one a push service can have (not loopback, private, link-local, ...).
fn is_public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (18..=19).contains(&o[1]))
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Resolves names for the push client and refuses any answer that is not a public address, so a
/// subscription cannot make the server call something inside its own network.
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let found = tokio::net::lookup_host((name.as_str(), 443)).await?;
            let public: Vec<std::net::SocketAddr> =
                found.filter(|a| is_public_ip(a.ip())).collect();
            if public.is_empty() {
                return Err("the host does not resolve to a public address".into());
            }
            let addrs: reqwest::dns::Addrs = Box::new(public.into_iter());
            Ok(addrs)
        })
    }
}

/// Registers this browser for the person. The same endpoint again updates the row (a browser that
/// signs in as someone else takes the subscription over, so the earlier person stops receiving it).
pub async fn subscribe(
    pool: &PgPool,
    user_id: Uuid,
    new: NewSubscription<'_>,
) -> Result<SubscriptionView, CoreError> {
    validate(&new)?;
    let owned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.push_subscriptions WHERE user_id=$1 AND endpoint <> $2",
    )
    .bind(user_id)
    .bind(new.endpoint.trim())
    .fetch_one(pool)
    .await?;
    if owned >= MAX_SUBSCRIPTIONS_PER_USER {
        return Err(CoreError::rule(
            "push_subscription_limit",
            "too many devices: remove one first",
        ));
    }
    let row = sqlx::query(
        "INSERT INTO penfii.push_subscriptions(id, user_id, endpoint, p256dh, auth, user_agent, label)
         VALUES($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (endpoint) DO UPDATE
           SET user_id=EXCLUDED.user_id, p256dh=EXCLUDED.p256dh, auth=EXCLUDED.auth,
               user_agent=EXCLUDED.user_agent, label=EXCLUDED.label, updated_at=now(), last_failure_at=NULL
         RETURNING id, endpoint, label, user_agent, created_at, last_success_at, last_failure_at",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(new.endpoint.trim())
    .bind(new.p256dh.trim_end_matches('='))
    .bind(new.auth.trim_end_matches('='))
    .bind(new.user_agent)
    .bind(new.label)
    .fetch_one(pool)
    .await?;
    view_of(&row)
}

fn view_of(row: &sqlx::postgres::PgRow) -> Result<SubscriptionView, CoreError> {
    Ok(SubscriptionView {
        id: row.try_get("id")?,
        endpoint: row.try_get("endpoint")?,
        label: row.try_get("label")?,
        user_agent: row.try_get("user_agent")?,
        created_at: row.try_get("created_at")?,
        last_success_at: row.try_get("last_success_at")?,
        last_failure_at: row.try_get("last_failure_at")?,
    })
}

/// The person's own devices, newest first.
pub async fn list(pool: &PgPool, user_id: Uuid) -> Result<Vec<SubscriptionView>, CoreError> {
    let rows = sqlx::query(
        "SELECT id, endpoint, label, user_agent, created_at, last_success_at, last_failure_at
         FROM penfii.push_subscriptions WHERE user_id=$1 ORDER BY created_at DESC, id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(view_of).collect()
}

/// Turns one of the person's devices off. Someone else's subscription is `NotFound`.
pub async fn unsubscribe(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<(), CoreError> {
    let done = sqlx::query("DELETE FROM penfii.push_subscriptions WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// The devices a push goes to: those of people whose account is on. A disabled account keeps its
/// rows (switching it back on resumes them) but is not sent to.
async fn subscriptions_for_push(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<Subscription>, CoreError> {
    let rows = sqlx::query(
        "SELECT s.id, s.user_id, s.endpoint, s.p256dh, s.auth
         FROM penfii.push_subscriptions s JOIN penfii.users u ON u.id=s.user_id
         WHERE s.user_id=$1 AND u.disabled_at IS NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Subscription {
                id: row.try_get("id")?,
                user_id: row.try_get("user_id")?,
                endpoint: row.try_get("endpoint")?,
                p256dh: row.try_get("p256dh")?,
                auth: row.try_get("auth")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The message
// ---------------------------------------------------------------------------------------------

/// The words of a push: generic per kind, in the person's language. They name no workspace, person
/// or message text.
fn words(kind: &str, locale: &str) -> (&'static str, &'static str) {
    let korean = locale.starts_with("ko");
    match (kind, korean) {
        ("mention.user", true) => ("Penfii", "새 멘션이 있습니다"),
        ("mention.user", false) => ("Penfii", "You were mentioned"),
        ("reply.direct", true) => ("Penfii", "내 메시지에 답글이 달렸습니다"),
        ("reply.direct", false) => ("Penfii", "Someone replied to your message"),
        ("dm.message", true) => ("Penfii", "새 메시지가 있습니다"),
        ("dm.message", false) => ("Penfii", "You have a new message"),
        ("thread.reply", true) => ("Penfii", "참여 중인 스레드에 새 답글이 있습니다"),
        ("thread.reply", false) => ("Penfii", "New reply in a thread you follow"),
        ("ticket.assigned", true) => ("Penfii", "작업이 배정되었습니다"),
        ("ticket.assigned", false) => ("Penfii", "A task was assigned to you"),
        ("reminder.due", true) => ("Penfii", "알림 시간이 되었습니다"),
        _ => ("Penfii", "Your reminder is due"),
    }
}

/// Where tapping the message leads, the way the Inbox opens the same item (`inboxHref` in the app).
fn path_of(workspace_id: Uuid, subject_kind: &str, subject_id: Uuid, params: &Value) -> String {
    let base = format!("/w/{workspace_id}");
    let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_owned);
    let inbox = format!("{base}/inbox");
    match subject_kind {
        "TICKET" => text("projectId").map_or(inbox, |project| {
            format!("{base}/projects/{project}/work?panel=ticket:{subject_id}")
        }),
        "PROJECT" => format!("{base}/projects/{subject_id}"),
        "DOC" => format!("{base}/docs?doc={subject_id}"),
        "MESSAGE" | "FILE" => match (text("conversationId"), text("threadRootId")) {
            (Some(conversation), Some(root)) => format!("{base}/c/{conversation}/t/{root}"),
            (Some(conversation), None) => format!("{base}/c/{conversation}"),
            _ => inbox,
        },
        _ => inbox,
    }
}

/// The encrypted body's content: the item id, its kind, generic words and the path to open. No
/// message text, no workspace name.
pub fn payload(
    kind: &str,
    locale: &str,
    inbox_item_id: Uuid,
    workspace_id: Uuid,
    subject_kind: &str,
    subject_id: Uuid,
    params: &Value,
) -> Value {
    let (title, body) = words(kind, locale);
    json!({
        "v": 1,
        "id": inbox_item_id,
        "kind": kind,
        "title": title,
        "body": body,
        "path": path_of(workspace_id, subject_kind, subject_id, params),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::elliptic_curve::sec1::ToSec1Point;

    fn new_private_key() -> String {
        let secret = random_secret();
        URL_SAFE_NO_PAD.encode(secret.to_bytes())
    }

    #[test]
    fn a_vapid_key_gives_a_65_byte_public_key_and_a_signed_token() {
        let vapid = VapidConfig::from_parts(&new_private_key(), "mailto:ops@example.com").unwrap();
        let public = URL_SAFE_NO_PAD.decode(vapid.public_key()).unwrap();
        assert_eq!(public.len(), 65);
        assert_eq!(public[0], 4);
        let header = vapid.authorization("https://push.example.com").unwrap();
        let token = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap();
        assert_eq!(token.split('.').count(), 3);
        assert!(header.ends_with(vapid.public_key()));
        // The token verifies against the public key.
        let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
        validation.set_audience(&["https://push.example.com"]);
        validation.set_required_spec_claims(&["exp", "aud"]);
        let x = URL_SAFE_NO_PAD.encode(&public[1..33]);
        let y = URL_SAFE_NO_PAD.encode(&public[33..65]);
        let decoding = jsonwebtoken::DecodingKey::from_ec_components(&x, &y).unwrap();
        let data = jsonwebtoken::decode::<Value>(token, &decoding, &validation).unwrap();
        assert_eq!(data.claims["sub"], "mailto:ops@example.com");
    }

    #[test]
    fn a_wrong_vapid_setting_is_refused_and_never_echoed() {
        assert!(VapidConfig::from_parts("not base64 !!", "mailto:a@b.c").is_err());
        assert!(VapidConfig::from_parts("AAAA", "mailto:a@b.c").is_err());
        let key = new_private_key();
        let error = VapidConfig::from_parts(&key, "ftp://x").unwrap_err();
        assert!(!error.contains(&key));
        let shown = format!(
            "{:?}",
            VapidConfig::from_parts(&key, "mailto:a@b.c").unwrap()
        );
        assert!(!shown.contains(&key));
    }

    #[test]
    fn a_message_is_encrypted_for_the_browser_and_signed_for_the_push_service() {
        let vapid = VapidConfig::from_parts(&new_private_key(), "mailto:ops@example.com").unwrap();
        let transport = HttpTransport::new(vapid).unwrap();
        // The browser's side of a subscription.
        let browser = random_secret();
        let auth = [7u8; 16];
        let subscription = Subscription {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            endpoint: "https://push.example.com/send/abc".into(),
            p256dh: URL_SAFE_NO_PAD.encode(browser.public_key().to_sec1_point(false).as_bytes()),
            auth: URL_SAFE_NO_PAD.encode(auth),
        };
        let request = transport.request(&subscription, b"{\"v\":1}").unwrap();
        assert_eq!(request.uri(), "https://push.example.com/send/abc");
        assert_eq!(request.headers()["content-encoding"], "aes128gcm");
        assert!(
            request.headers()["authorization"]
                .to_str()
                .unwrap()
                .starts_with("vapid t=")
        );
        let plain = web_push_native::decrypt(
            request.body().clone(),
            &browser,
            &Auth::clone_from_slice(&auth),
        )
        .unwrap();
        assert_eq!(plain, b"{\"v\":1}");
    }

    #[test]
    fn only_public_hosts_and_addresses_can_be_push_services() {
        for host in [
            "fcm.googleapis.com",
            "updates.push.services.mozilla.com",
            "web.push.apple.com",
            "wns2-par02p.notify.windows.com",
        ] {
            assert!(is_public_host(host), "{host}");
        }
        for host in [
            "localhost",
            "127.0.0.1",
            "10.0.0.5",
            "intranet",
            "db.internal",
            "minio",
            "[::1]",
            "a.local",
            "169.254.169.254",
        ] {
            assert!(!is_public_host(host), "{host}");
        }
        for ip in ["8.8.8.8", "142.250.1.1", "2607:f8b0:4004::1"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:10.0.0.1",
            "224.0.0.1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    /// Reaches the real FCM endpoint with a made-up subscription: `cargo test -p penfii-core --lib -- --ignored push_egress`.
    /// It shows DNS, TLS and the public-address guard work from this machine and that the push service
    /// answers the signed request (a made-up registration is refused with 4xx, which is `Gone` or `Failed`).
    #[tokio::test]
    #[ignore = "needs outbound HTTPS"]
    async fn push_egress_reaches_a_real_push_service() {
        let (private, _) = VapidConfig::generate();
        let transport = HttpTransport::new(
            VapidConfig::from_parts(&private, "mailto:ops@example.com").unwrap(),
        )
        .unwrap();
        let browser = random_secret();
        let subscription = Subscription {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            endpoint: "https://fcm.googleapis.com/fcm/send/penfii-egress-check".into(),
            p256dh: URL_SAFE_NO_PAD.encode(browser.public_key().to_sec1_point(false).as_bytes()),
            auth: URL_SAFE_NO_PAD.encode([3u8; 16]),
        };
        let outcome = transport.send(&subscription, b"{\"v\":1}").await;
        eprintln!("push service answered: {outcome:?}");
        assert!(
            matches!(outcome, Outcome::Gone | Outcome::Failed(_)),
            "{outcome:?}"
        );
    }

    #[test]
    fn only_the_directed_kinds_are_pushed() {
        for kind in [
            "mention.user",
            "reply.direct",
            "ticket.assigned",
            "reminder.due",
            "dm.message",
            "thread.reply",
        ] {
            assert!(is_push_kind(kind), "{kind}");
        }
        for kind in [
            "dependency.released",
            "channel.invited",
            "ticket.commented",
            "ticket.status_changed",
            "ticket.field_changed",
            "sprint.started",
            "sprint.closed",
            "something.new",
        ] {
            assert!(!is_push_kind(kind), "{kind}");
        }
    }

    #[test]
    fn the_message_carries_no_text_and_no_workspace_name_and_opens_where_the_inbox_does() {
        let ws = Uuid::new_v4();
        let ticket = Uuid::new_v4();
        let value = payload(
            "ticket.assigned",
            "en-US",
            Uuid::new_v4(),
            ws,
            "TICKET",
            ticket,
            &json!({"projectId": "p1", "name": "Secret"}),
        );
        assert_eq!(
            value["path"],
            format!("/w/{ws}/projects/p1/work?panel=ticket:{ticket}")
        );
        assert_eq!(value["body"], "A task was assigned to you");
        assert!(!value.to_string().contains("Secret"));
        let thread = payload(
            "mention.user",
            "ko-KR",
            Uuid::new_v4(),
            ws,
            "MESSAGE",
            Uuid::new_v4(),
            &json!({"conversationId": "c1", "threadRootId": "m1"}),
        );
        assert_eq!(thread["path"], format!("/w/{ws}/c/c1/t/m1"));
        assert_eq!(thread["body"], "새 멘션이 있습니다");
        let orphan = payload(
            "ticket.assigned",
            "en",
            Uuid::new_v4(),
            ws,
            "TICKET",
            ticket,
            &json!({}),
        );
        assert_eq!(orphan["path"], format!("/w/{ws}/inbox"));
    }
}
