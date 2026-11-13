//! Incoming webhooks. A webhook is permission to post one plain-text message into one channel as
//! one Integration Actor. It is not a member of the channel, and the request cannot choose another
//! channel or author.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use chrono::Utc;
use hmac::{Hmac, Mac};
use penfii_contracts::{Webhook, WebhookCreated, WebhookList};
use rand::RngCore;
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::conversation::{self, CreatedMessage};
use crate::error::CoreError;
use crate::identity::{self, check_address, record_address_failure};
use crate::limits::Limits;
use crate::platform::{self, ChangeNotice};

type HmacSha256 = Hmac<Sha256>;

const POSTS_PER_MINUTE: i32 = 60;
const TIMESTAMP_SKEW_SECONDS: i64 = 300;

pub struct NewWebhook<'a> {
    pub channel_id: Uuid,
    pub name: &'a str,
    pub auth_mode: &'a str,
    pub actor_id: Option<Uuid>,
    pub display_name: Option<&'a str>,
    pub kek: Option<&'a [u8; 32]>,
}

pub async fn create_webhook(
    pool: &PgPool,
    actor_id: Uuid,
    webhook: NewWebhook<'_>,
) -> Result<WebhookCreated, CoreError> {
    let NewWebhook {
        channel_id,
        name,
        auth_mode,
        actor_id: existing_actor,
        display_name,
        kek,
    } = webhook;
    let name = identity::require_label(name, "name")?;
    if auth_mode != "URL_SECRET" && auth_mode != "SIGNATURE" {
        return Err(CoreError::validation(
            "authMode must be URL_SECRET or SIGNATURE",
        ));
    }
    match (
        existing_actor,
        display_name.map(str::trim).filter(|v| !v.is_empty()),
    ) {
        (Some(_), Some(_)) => {
            return Err(CoreError::validation(
                "set actorId or displayName, not both",
            ));
        }
        (None, None) => {
            return Err(CoreError::validation("actorId or displayName is required"));
        }
        _ => {}
    }
    let display_name = match display_name {
        Some(value) => Some(identity::require_label(value, "displayName")?),
        None => None,
    };
    let mut tx = pool.begin().await?;
    let channel = conversation::require_webhook_access(&mut tx, actor_id, channel_id, true).await?;
    let actor = match existing_actor {
        Some(actor) => load_actor(&mut tx, channel.workspace_id, actor).await?,
        None => {
            create_actor(
                &mut tx,
                channel.workspace_id,
                actor_id,
                display_name.as_deref().expect("display name checked"),
            )
            .await?
        }
    };
    let webhook_id = Uuid::new_v4();
    let stored = credential(auth_mode, webhook_id, kek)?;
    sqlx::query(
        "INSERT INTO penfii.webhooks
            (id, workspace_id, channel_id, actor_user_id, name, auth_mode,
             token_hash, secret_nonce, secret_ciphertext, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(webhook_id)
    .bind(channel.workspace_id)
    .bind(channel.id)
    .bind(actor.id)
    .bind(&name)
    .bind(auth_mode)
    .bind(&stored.token_hash)
    .bind(&stored.nonce)
    .bind(&stored.ciphertext)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Audit {
            workspace_id: channel.workspace_id,
            actor_id,
            action: "webhook.created",
            webhook_id,
            channel_id: channel.id,
            auth_mode,
            integration_actor: actor.id,
            actor_can_read: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(WebhookCreated {
        webhook: Webhook {
            id: webhook_id,
            channel_id: channel.id,
            name,
            auth_mode: auth_mode.to_string(),
            actor_id: actor.id,
            actor_display_name: actor.display_name,
            revoked: false,
            created_at: Utc::now(),
            rotated_at: None,
        },
        secret: stored.secret,
    })
}

pub async fn list_webhooks(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
) -> Result<WebhookList, CoreError> {
    let mut tx = pool.begin().await?;
    conversation::require_webhook_access(&mut tx, actor_id, channel_id, false).await?;
    let items = sqlx::query_as::<_, WebhookRecord>(
        "SELECT w.id, w.channel_id, w.name, w.auth_mode,
                w.actor_user_id AS actor_id, u.display_name AS actor_display_name,
                (w.revoked_at IS NOT NULL) AS revoked, w.created_at, w.rotated_at
         FROM penfii.webhooks w
         JOIN penfii.users u ON u.id = w.actor_user_id
         WHERE w.channel_id = $1
         ORDER BY w.created_at, w.id",
    )
    .bind(channel_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(WebhookList {
        items: items.into_iter().map(Webhook::from).collect(),
    })
}

pub async fn rotate_webhook(
    pool: &PgPool,
    actor_id: Uuid,
    webhook_id: Uuid,
    kek: Option<&[u8; 32]>,
) -> Result<WebhookCreated, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = lock_webhook(&mut tx, webhook_id).await? else {
        return Err(CoreError::NotFound);
    };
    conversation::require_webhook_access(&mut tx, actor_id, row.channel_id, true).await?;
    if row.revoked_at.is_some() {
        return Err(CoreError::conflict(
            "webhook_revoked",
            "revoked webhook cannot be rotated",
        ));
    }
    if row.auth_mode == "SIGNATURE" && kek.is_none() {
        return Err(CoreError::conflict(
            "webhook_kek_unconfigured",
            "PENFII_WEBHOOK_KEK is not configured",
        ));
    }
    let stored = credential(&row.auth_mode, row.id, kek)?;
    sqlx::query(
        "UPDATE penfii.webhooks
         SET token_hash = $2, secret_nonce = $3, secret_ciphertext = $4, rotated_at = now()
         WHERE id = $1",
    )
    .bind(row.id)
    .bind(&stored.token_hash)
    .bind(&stored.nonce)
    .bind(&stored.ciphertext)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Audit {
            workspace_id: row.workspace_id,
            actor_id,
            action: "webhook.rotated",
            webhook_id: row.id,
            channel_id: row.channel_id,
            auth_mode: &row.auth_mode,
            integration_actor: row.actor_id,
            actor_can_read: None,
        },
    )
    .await?;
    let webhook = load_webhook(&mut tx, row.id).await?;
    tx.commit().await?;
    Ok(WebhookCreated {
        webhook,
        secret: stored.secret,
    })
}

pub async fn revoke_webhook(
    pool: &PgPool,
    actor_id: Uuid,
    webhook_id: Uuid,
) -> Result<Webhook, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = lock_webhook(&mut tx, webhook_id).await? else {
        return Err(CoreError::NotFound);
    };
    conversation::require_webhook_access(&mut tx, actor_id, row.channel_id, false).await?;
    let can_read = crate::access::load_conversation(&mut tx, actor_id, row.channel_id)
        .await?
        .is_some_and(|access| access.can_read());
    if row.revoked_at.is_none() {
        sqlx::query("UPDATE penfii.webhooks SET revoked_at = now() WHERE id = $1")
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            Audit {
                workspace_id: row.workspace_id,
                actor_id,
                action: "webhook.revoked",
                webhook_id: row.id,
                channel_id: row.channel_id,
                auth_mode: &row.auth_mode,
                integration_actor: row.actor_id,
                actor_can_read: Some(can_read),
            },
        )
        .await?;
    }
    let webhook = load_webhook(&mut tx, row.id).await?;
    tx.commit().await?;
    Ok(webhook)
}

pub struct IncomingPost {
    pub credential: String,
    pub raw_body: Vec<u8>,
    pub timestamp: Option<String>,
    pub signature: Option<String>,
    pub delivery: Option<String>,
    pub address: Option<String>,
}

pub async fn post_webhook(
    pool: &PgPool,
    limits: &Limits,
    post: IncomingPost,
    kek: Option<&[u8; 32]>,
) -> Result<crate::Outcome<penfii_contracts::Message>, CoreError> {
    let max = limits.message_body_max_bytes.ok_or_else(|| {
        CoreError::conflict(
            "message_limit_unconfigured",
            "message body limit is not configured",
        )
    })?;
    let len = i64::try_from(post.raw_body.len()).unwrap_or(i64::MAX);
    if len > max {
        return Err(CoreError::validation(
            "request body exceeds the configured maximum",
        ));
    }
    check_address(pool, "webhook", post.address.as_deref()).await?;
    let authed = match authenticate(pool, &post, kek).await? {
        Some(authed) => authed,
        None => {
            record_address_failure(
                pool,
                "webhook",
                post.address.as_deref(),
                &limits.login_throttle,
            )
            .await?;
            return Err(CoreError::Unauthenticated);
        }
    };
    let text = plain_text(&post.raw_body)?;
    let body = text_to_doc(&text);
    conversation::prepare_body(limits, 1, &body)?;
    let mentions = conversation::richtext::validate(&body, false)
        .map_err(|_| CoreError::validation("message body is invalid"))?;
    if conversation::richtext::is_blank(&body) {
        return Err(CoreError::validation("message body is empty"));
    }
    let body_text = String::from_utf8(post.raw_body.clone())
        .map_err(|_| CoreError::validation("request body is invalid"))?;
    let delivery = delivery_key(post.delivery.as_deref())?;
    let actor_key = format!("webhook:{}", authed.id);
    let digest = platform::fingerprint("webhook.post", &Value::String(body_text));

    let mut tx = pool.begin().await?;
    let Some(locked) = lock_webhook(&mut tx, authed.id).await? else {
        return Err(CoreError::Unauthenticated);
    };
    if locked.revoked_at.is_some() || actor_disabled(&mut tx, locked.actor_id).await? {
        return Err(CoreError::Unauthenticated);
    }
    if team_archived(&mut tx, locked.channel_id).await? {
        return Err(CoreError::conflict(
            "channel_read_only",
            "the Team is archived, so its channels are read-only",
        ));
    }
    sqlx::query("SELECT id FROM penfii.conversations WHERE id = $1 FOR UPDATE")
        .bind(locked.channel_id)
        .fetch_one(&mut *tx)
        .await?;
    // Replay is checked before a rate slot is taken, and only when the delivery was already seen:
    // `replay_outcome` claims a fresh key, and a rate-limited post commits, which would keep that claim.
    if command_exists(&mut tx, locked.workspace_id, &actor_key, &delivery).await?
        && let Some(outcome) = platform::replay_outcome(
            &mut tx,
            locked.workspace_id,
            &actor_key,
            &delivery,
            "webhook.post",
            &digest,
        )
        .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    let (posts, retry_after) = take_post_slot(&mut tx, locked.id).await?;
    if posts > POSTS_PER_MINUTE {
        tx.commit().await?;
        return Err(CoreError::RateLimited {
            retry_after_seconds: retry_after,
        });
    }
    if let Some(outcome) = platform::replay_outcome(
        &mut tx,
        locked.workspace_id,
        &actor_key,
        &delivery,
        "webhook.post",
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    let (message, change_seq) = conversation::persist_created_message(
        &mut tx,
        limits,
        CreatedMessage {
            workspace_id: locked.workspace_id,
            conversation_id: locked.channel_id,
            author_id: locked.actor_id,
            body_schema_version: 1,
            body,
            thread_root_id: None,
            reply_to_id: None,
            mentions,
            file_ids: Vec::new(),
            actor_kind: "INTEGRATION",
            security_context: json!({
                "conversationId": locked.channel_id.to_string(),
                "webhookId": locked.id.to_string(),
            }),
        },
    )
    .await?;
    let stored =
        serde_json::to_value(&message).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        locked.workspace_id,
        &actor_key,
        &delivery,
        201,
        &stored,
    )
    .await?;
    let message_id = message.id;
    let message_version = message.version;
    tx.commit().await?;
    Ok(crate::Outcome {
        http_status: 201,
        value: message,
        changes: vec![ChangeNotice::Conversation {
            conversation_id: locked.channel_id,
            change_seq,
            message_id,
            message_version,
            change_kind: "message.created".to_string(),
        }],
    })
}

#[derive(sqlx::FromRow)]
struct WebhookRecord {
    id: Uuid,
    channel_id: Uuid,
    name: String,
    auth_mode: String,
    actor_id: Uuid,
    actor_display_name: String,
    revoked: bool,
    created_at: chrono::DateTime<Utc>,
    rotated_at: Option<chrono::DateTime<Utc>>,
}

impl From<WebhookRecord> for Webhook {
    fn from(row: WebhookRecord) -> Self {
        Self {
            id: row.id,
            channel_id: row.channel_id,
            name: row.name,
            auth_mode: row.auth_mode,
            actor_id: row.actor_id,
            actor_display_name: row.actor_display_name,
            revoked: row.revoked,
            created_at: row.created_at,
            rotated_at: row.rotated_at,
        }
    }
}

struct ActorRef {
    id: Uuid,
    display_name: String,
}

#[derive(sqlx::FromRow)]
struct WebhookRow {
    id: Uuid,
    workspace_id: Uuid,
    channel_id: Uuid,
    actor_id: Uuid,
    auth_mode: String,
    revoked_at: Option<chrono::DateTime<Utc>>,
}

struct Authed {
    id: Uuid,
}

async fn authenticate(
    pool: &PgPool,
    post: &IncomingPost,
    kek: Option<&[u8; 32]>,
) -> Result<Option<Authed>, CoreError> {
    if let Ok(webhook_id) = Uuid::parse_str(post.credential.trim()) {
        return authenticate_signature(pool, webhook_id, post, kek).await;
    }
    authenticate_url(pool, post.credential.trim()).await
}

async fn authenticate_url(pool: &PgPool, token: &str) -> Result<Option<Authed>, CoreError> {
    if token.is_empty() {
        return Ok(None);
    }
    let hash = identity::token_hash(token);
    let row = sqlx::query_as::<_, (Uuid,)>(
        "SELECT w.id
         FROM penfii.webhooks w
         JOIN penfii.integration_actors a ON a.user_id = w.actor_user_id
         WHERE w.auth_mode = 'URL_SECRET'
           AND w.token_hash = $1
           AND w.revoked_at IS NULL
           AND a.disabled_at IS NULL",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| Authed { id }))
}

async fn authenticate_signature(
    pool: &PgPool,
    webhook_id: Uuid,
    post: &IncomingPost,
    kek: Option<&[u8; 32]>,
) -> Result<Option<Authed>, CoreError> {
    let Some(timestamp) = post.timestamp.as_deref() else {
        return Ok(None);
    };
    let Some(signature) = post.signature.as_deref() else {
        return Ok(None);
    };
    if !timestamp_fresh(timestamp) {
        return Ok(None);
    }
    let Some(row) = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
        "SELECT w.secret_nonce, w.secret_ciphertext
         FROM penfii.webhooks w
         JOIN penfii.integration_actors a ON a.user_id = w.actor_user_id
         WHERE w.id = $1
           AND w.auth_mode = 'SIGNATURE'
           AND w.revoked_at IS NULL
           AND a.disabled_at IS NULL
           AND w.secret_nonce IS NOT NULL
           AND w.secret_ciphertext IS NOT NULL",
    )
    .bind(webhook_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let Some(kek) = kek else {
        return Ok(None);
    };
    let Some(secret) = open_secret(kek, webhook_id, &row.0, &row.1) else {
        tracing::warn!("webhook signature secret could not be decrypted");
        return Ok(None);
    };
    if verify_signature(&secret, timestamp, &post.raw_body, signature) {
        Ok(Some(Authed { id: webhook_id }))
    } else {
        Ok(None)
    }
}

fn timestamp_fresh(raw: &str) -> bool {
    let Ok(seconds) = raw.parse::<i64>() else {
        return false;
    };
    (Utc::now().timestamp() - seconds).abs() <= TIMESTAMP_SKEW_SECONDS
}

fn verify_signature(secret: &str, timestamp: &str, body: &[u8], header: &str) -> bool {
    let Some(encoded) = header.strip_prefix("v1=") else {
        return false;
    };
    let Ok(sig) = hex::decode(encoded) else {
        return false;
    };
    if sig.len() != 32 {
        return false;
    }
    let Ok(mut mac) = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

fn plain_text(raw: &[u8]) -> Result<String, CoreError> {
    let value: Value = serde_json::from_slice(raw)
        .map_err(|_| CoreError::validation("request body is invalid"))?;
    let Some(object) = value.as_object() else {
        return Err(CoreError::validation(
            "request body must be an object with text",
        ));
    };
    if object.len() != 1 || !object.contains_key("text") {
        return Err(CoreError::validation("request body must contain only text"));
    }
    object["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| CoreError::validation("text must be a string"))
}

pub(crate) fn text_to_doc(text: &str) -> Value {
    let content = text
        .split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                json!({"type": "paragraph", "content": []})
            } else {
                json!({"type": "paragraph", "content": [{"type": "text", "text": line}]})
            }
        })
        .collect::<Vec<_>>();
    json!({"type": "doc", "content": content})
}

fn delivery_key(raw: Option<&str>) -> Result<String, CoreError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(Uuid::new_v4().to_string()),
        Some(value) if value.len() <= 200 => Ok(value.to_string()),
        Some(_) => Err(CoreError::validation("X-Penfii-Delivery is invalid")),
    }
}

struct StoredCredential {
    secret: String,
    token_hash: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
    ciphertext: Option<Vec<u8>>,
}

fn credential(
    auth_mode: &str,
    webhook_id: Uuid,
    kek: Option<&[u8; 32]>,
) -> Result<StoredCredential, CoreError> {
    let (secret, hash) = identity::new_secret();
    if auth_mode == "URL_SECRET" {
        return Ok(StoredCredential {
            secret,
            token_hash: Some(hash),
            nonce: None,
            ciphertext: None,
        });
    }
    let kek = kek.ok_or_else(|| {
        CoreError::conflict(
            "webhook_kek_unconfigured",
            "PENFII_WEBHOOK_KEK is not configured",
        )
    })?;
    let (nonce, ciphertext) = seal(kek, webhook_id, &secret)?;
    Ok(StoredCredential {
        secret,
        token_hash: None,
        nonce: Some(nonce),
        ciphertext: Some(ciphertext),
    })
}

fn seal(kek: &[u8; 32], webhook_id: Uuid, secret: &str) -> Result<(Vec<u8>, Vec<u8>), CoreError> {
    let cipher =
        Aes256Gcm::new_from_slice(kek).map_err(|error| CoreError::Internal(error.to_string()))?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret.as_bytes(),
                aad: webhook_id.as_bytes(),
            },
        )
        .map_err(|error| CoreError::Internal(error.to_string()))?;
    Ok((nonce.to_vec(), ciphertext))
}

fn open_secret(
    kek: &[u8; 32],
    webhook_id: Uuid,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Option<String> {
    if nonce.len() != 12 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(kek).ok()?;
    let plain = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: webhook_id.as_bytes(),
            },
        )
        .ok()?;
    String::from_utf8(plain).ok()
}

async fn create_actor(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    created_by: Uuid,
    display_name: &str,
) -> Result<ActorRef, CoreError> {
    let id = Uuid::new_v4();
    let email = format!("integration.{id}@actors.invalid");
    sqlx::query(
        "INSERT INTO penfii.users (id, email, email_key, display_name, kind)
         VALUES ($1, $2, $2, $3, 'INTEGRATION')",
    )
    .bind(id)
    .bind(&email)
    .bind(display_name)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.integration_actors (user_id, workspace_id, created_by)
         VALUES ($1, $2, $3)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(created_by)
    .execute(&mut *conn)
    .await?;
    Ok(ActorRef {
        id,
        display_name: display_name.to_string(),
    })
}

async fn load_actor(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor_id: Uuid,
) -> Result<ActorRef, CoreError> {
    let row = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT a.user_id, u.display_name
         FROM penfii.integration_actors a
         JOIN penfii.users u ON u.id = a.user_id
         WHERE a.user_id = $1 AND a.workspace_id = $2 AND u.kind = 'INTEGRATION'",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some((id, display_name)) => Ok(ActorRef { id, display_name }),
        None => Err(CoreError::validation(
            "actorId is not an integration actor in this workspace",
        )),
    }
}

async fn lock_webhook(
    conn: &mut PgConnection,
    webhook_id: Uuid,
) -> Result<Option<WebhookRow>, CoreError> {
    let row = sqlx::query_as::<_, WebhookRow>(
        "SELECT w.id, w.workspace_id, w.channel_id, w.actor_user_id AS actor_id, w.auth_mode,
                w.revoked_at
         FROM penfii.webhooks w
         WHERE w.id = $1
         FOR UPDATE",
    )
    .bind(webhook_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

async fn load_webhook(conn: &mut PgConnection, webhook_id: Uuid) -> Result<Webhook, CoreError> {
    let row = sqlx::query_as::<_, WebhookRecord>(
        "SELECT w.id, w.channel_id, w.name, w.auth_mode,
                w.actor_user_id AS actor_id, u.display_name AS actor_display_name,
                (w.revoked_at IS NOT NULL) AS revoked, w.created_at, w.rotated_at
         FROM penfii.webhooks w
         JOIN penfii.users u ON u.id = w.actor_user_id
         WHERE w.id = $1",
    )
    .bind(webhook_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row.into())
}

async fn actor_disabled(conn: &mut PgConnection, actor_id: Uuid) -> Result<bool, CoreError> {
    let disabled = sqlx::query_scalar(
        "SELECT disabled_at IS NOT NULL FROM penfii.integration_actors WHERE user_id = $1",
    )
    .bind(actor_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(disabled)
}

async fn team_archived(conn: &mut PgConnection, channel_id: Uuid) -> Result<bool, CoreError> {
    let archived = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM penfii.channels ch
            JOIN penfii.teams t ON t.id = ch.team_id
            WHERE ch.id = $1 AND t.archived_at IS NOT NULL
         )",
    )
    .bind(channel_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(archived)
}

async fn command_exists(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor_key: &str,
    idempotency_key: &str,
) -> Result<bool, CoreError> {
    let exists = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM penfii.commands
            WHERE workspace_id = $1 AND actor_key = $2 AND idempotency_key = $3
         )",
    )
    .bind(workspace_id)
    .bind(actor_key)
    .bind(idempotency_key)
    .fetch_one(&mut *conn)
    .await?;
    Ok(exists)
}

async fn take_post_slot(
    conn: &mut PgConnection,
    webhook_id: Uuid,
) -> Result<(i32, i64), CoreError> {
    let row = sqlx::query_as::<_, (i32, i64)>(
        "INSERT INTO penfii.webhook_post_windows (webhook_id, window_start, posts)
         VALUES ($1, now(), 1)
         ON CONFLICT (webhook_id) DO UPDATE SET
             posts = CASE
                 WHEN penfii.webhook_post_windows.window_start <= now() - interval '60 seconds'
                 THEN 1
                 ELSE penfii.webhook_post_windows.posts + 1
             END,
             window_start = CASE
                 WHEN penfii.webhook_post_windows.window_start <= now() - interval '60 seconds'
                 THEN now()
                 ELSE penfii.webhook_post_windows.window_start
             END
         RETURNING posts,
                   GREATEST(
                       1,
                       EXTRACT(EPOCH FROM (window_start + interval '60 seconds' - now()))
                   )::bigint",
    )
    .bind(webhook_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

struct Audit<'a> {
    workspace_id: Uuid,
    actor_id: Uuid,
    action: &'a str,
    webhook_id: Uuid,
    channel_id: Uuid,
    auth_mode: &'a str,
    integration_actor: Uuid,
    /// Whether the actor could read the channel; recorded where an admin who cannot acts (revoke).
    actor_can_read: Option<bool>,
}

async fn audit(conn: &mut PgConnection, event: Audit<'_>) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, $2, $3, $4, 'OK', 'WEBHOOK', $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(event.workspace_id)
    .bind(event.actor_id)
    .bind(event.action)
    .bind(event.webhook_id)
    .bind(json!({
        "channelId": event.channel_id,
        "authMode": event.auth_mode,
        "actorId": event.integration_actor,
        "actorCanRead": event.actor_can_read,
    }))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_accepts_the_raw_body_and_rejects_a_different_one() {
        let secret = "abc";
        let body = br#"{"text":"hello"}"#;
        let ts = "1700000000";
        let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(ts.as_bytes());
        mac.update(b".");
        mac.update(body);
        let header = format!("v1={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature(secret, ts, body, &header));
        assert!(!verify_signature(
            secret,
            ts,
            br#"{"text":"hello "} "#,
            &header
        ));
        assert!(!verify_signature(secret, "1700000001", body, &header));
    }
}
