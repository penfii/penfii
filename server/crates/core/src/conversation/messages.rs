//! Message send, edit, delete, reactions, and history. One transaction per command.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use penfii_contracts::{
    Message, MessageAttachment, MessageHistory, MessageVersion, Reaction, ReactionState,
};
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use crate::access::{self, ConvAccess};
use crate::error::CoreError;
use crate::files;
use crate::limits::Limits;
use crate::platform::{self, ChangeNotice};
use crate::{
    Outcome,
    conversation::richtext::{self, ParsedMention, RichTextError},
};

pub struct SendMessage {
    pub actor_id: Uuid,
    pub conversation_id: Uuid,
    pub idempotency_key: String,
    pub body_schema_version: i16,
    pub body: Value,
    pub thread_root_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub file_ids: Vec<Uuid>,
    pub raw: Value,
}

pub struct EditMessage {
    pub actor_id: Uuid,
    pub message_id: Uuid,
    pub idempotency_key: String,
    pub expected_version: i64,
    pub body: Value,
    pub raw: Value,
}

pub struct DeleteMessage {
    pub actor_id: Uuid,
    pub message_id: Uuid,
    pub idempotency_key: String,
    pub raw: Value,
}

pub struct ReactionCommand {
    pub actor_id: Uuid,
    pub message_id: Uuid,
    pub idempotency_key: String,
    pub emoji: String,
    pub raw: Value,
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: Uuid,
    conversation_id: Uuid,
    stream_seq: i64,
    author_id: Uuid,
    thread_root_id: Option<Uuid>,
    reply_to_id: Option<Uuid>,
    version: i64,
    body_schema_version: i16,
    body: Value,
    created_at: DateTime<Utc>,
    edited_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct ReactionRow {
    message_id: Uuid,
    user_id: Uuid,
    emoji: String,
}

#[derive(sqlx::FromRow)]
struct LockedConv {
    workspace_id: Uuid,
}

const MESSAGE_COLUMNS: &str = "id, conversation_id, stream_seq, author_id, thread_root_id, reply_to_id, version, body_schema_version, body, created_at, edited_at, deleted_at";

pub async fn list_messages(
    pool: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
    before_seq: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<Message>, CoreError> {
    let limit = crate::paging::require_limit(limit, 50, 100, "limit must be between 1 and 100")?;
    if before_seq.is_some_and(|seq| seq < 0) {
        return Err(CoreError::validation("beforeSeq is invalid"));
    }
    let mut tx = pool.begin().await?;
    access::require_read(&mut tx, user_id, conversation_id).await?;
    let rows = sqlx::query_as::<_, MessageRow>(&format!(
        "SELECT {MESSAGE_COLUMNS}
         FROM penfii.messages
         WHERE conversation_id = $1
           AND thread_root_id IS NULL
           AND ($2::bigint IS NULL OR stream_seq < $2)
         ORDER BY stream_seq DESC
         LIMIT $3"
    ))
    .bind(conversation_id)
    .bind(before_seq)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let messages = map_messages(&mut tx, rows).await?;
    tx.commit().await?;
    Ok(messages)
}

pub async fn list_thread(
    pool: &PgPool,
    user_id: Uuid,
    root_id: Uuid,
    after_seq: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<Message>, CoreError> {
    let limit = crate::paging::require_limit(limit, 50, 100, "limit must be between 1 and 100")?;
    if after_seq.is_some_and(|seq| seq < 0) {
        return Err(CoreError::validation("afterSeq is invalid"));
    }
    let mut tx = pool.begin().await?;
    super::require_thread_root(&mut tx, user_id, root_id).await?;
    let rows = sqlx::query_as::<_, MessageRow>(&format!(
        "SELECT {MESSAGE_COLUMNS}
         FROM penfii.messages
         WHERE (id = $1 OR thread_root_id = $1)
           AND ($2::bigint IS NULL OR stream_seq > $2)
         ORDER BY stream_seq ASC
         LIMIT $3"
    ))
    .bind(root_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let messages = map_messages(&mut tx, rows).await?;
    tx.commit().await?;
    Ok(messages)
}

pub async fn send_message(
    pool: &PgPool,
    limits: &Limits,
    cmd: SendMessage,
) -> Result<Outcome<Message>, CoreError> {
    prepare_body(limits, cmd.body_schema_version, &cmd.body)?;
    let mentions = richtext::validate(&cmd.body, false).map_err(map_rich)?;
    if cmd.file_ids.is_empty() && richtext::is_blank(&cmd.body) {
        return Err(CoreError::validation("message body is empty"));
    }
    let mut tx = pool.begin().await?;
    let locked = lock_conversation(&mut tx, cmd.conversation_id).await?;
    let access = access::require_read(&mut tx, cmd.actor_id, cmd.conversation_id).await?;
    access.require_writable()?;
    if !access.can_write() {
        return Err(CoreError::forbidden("channel membership required"));
    }
    let actor = platform::actor_key(cmd.actor_id);
    let digest = platform::fingerprint("message.send", &cmd.raw);
    if let Some(outcome) = platform::replay_outcome::<Message>(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        "message.send",
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    validate_targets(
        &mut tx,
        cmd.conversation_id,
        cmd.thread_root_id,
        cmd.reply_to_id,
    )
    .await?;
    let (message, change_seq) = persist_created_message(
        &mut tx,
        limits,
        CreatedMessage {
            workspace_id: locked.workspace_id,
            conversation_id: cmd.conversation_id,
            author_id: cmd.actor_id,
            body_schema_version: cmd.body_schema_version,
            body: cmd.body,
            thread_root_id: cmd.thread_root_id,
            reply_to_id: cmd.reply_to_id,
            mentions,
            file_ids: cmd.file_ids,
            actor_kind: "USER",
            security_context: json!({"conversationId": cmd.conversation_id.to_string()}),
        },
    )
    .await?;
    let body =
        serde_json::to_value(&message).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        201,
        &body,
    )
    .await?;
    let message_id = message.id;
    tx.commit().await?;
    Ok(Outcome {
        http_status: 201,
        value: message,
        changes: vec![ChangeNotice::Conversation {
            conversation_id: cmd.conversation_id,
            change_seq,
            message_id,
            message_version: 1,
            change_kind: "message.created".to_string(),
        }],
    })
}

pub async fn edit_message(
    pool: &PgPool,
    limits: &Limits,
    cmd: EditMessage,
) -> Result<Outcome<Message>, CoreError> {
    prepare_body(limits, 1, &cmd.body)?;
    let mentions = richtext::validate(&cmd.body, false).map_err(map_rich)?;
    if richtext::is_blank(&cmd.body) {
        return Err(CoreError::validation("message body is empty"));
    }
    let mut tx = pool.begin().await?;
    let Some(existing) = load_row(&mut tx, cmd.message_id).await? else {
        return Err(CoreError::NotFound);
    };
    let locked = lock_conversation(&mut tx, existing.conversation_id).await?;
    let current = load_row_for_update(&mut tx, cmd.message_id).await?;
    let access = access::require_read(&mut tx, cmd.actor_id, current.conversation_id).await?;
    access.require_writable()?;
    if current.author_id != cmd.actor_id {
        return Err(CoreError::forbidden("only the author can edit a message"));
    }
    let actor = platform::actor_key(cmd.actor_id);
    let digest = platform::fingerprint("message.edit", &cmd.raw);
    if let Some(outcome) = platform::replay_outcome::<Message>(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        "message.edit",
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    if current.deleted_at.is_some() {
        return Err(CoreError::conflict(
            "message_deleted",
            "deleted message cannot be edited",
        ));
    }
    if current.version != cmd.expected_version {
        return Err(version_conflict(&current));
    }
    ensure_mention_users(&mut tx, &mentions).await?;
    archive_version(&mut tx, &current, cmd.actor_id).await?;
    sqlx::query(
        "UPDATE penfii.messages
         SET body = $2, body_schema_version = 1, version = version + 1, edited_at = now()
         WHERE id = $1",
    )
    .bind(cmd.message_id)
    .bind(&cmd.body)
    .execute(&mut *tx)
    .await?;
    let previous: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM penfii.message_mentions WHERE message_id = $1 AND user_id IS NOT NULL AND removed_at IS NULL",
    )
    .bind(cmd.message_id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM penfii.message_mentions WHERE message_id = $1")
        .bind(cmd.message_id)
        .execute(&mut *tx)
        .await?;
    insert_mentions(&mut tx, cmd.message_id, &mentions).await?;
    let change_seq = bump_change(&mut tx, current.conversation_id).await?;
    let new_version = current.version + 1;
    insert_change(
        &mut tx,
        current.conversation_id,
        change_seq,
        cmd.message_id,
        new_version,
        "message.edited",
    )
    .await?;
    let added: Vec<Uuid> = mentions
        .iter()
        .map(|m| m.user_id)
        .filter(|id| !previous.contains(id))
        .collect();
    let (thread_root_id, reply_to_id): (Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT thread_root_id, reply_to_id FROM penfii.messages WHERE id = $1")
            .bind(cmd.message_id)
            .fetch_one(&mut *tx)
            .await?;
    crate::events::emit(
        &mut tx,
        crate::events::NewEvent {
            workspace_id: locked.workspace_id,
            target_kind: "MESSAGE",
            target_id: cmd.message_id,
            target_version: Some(new_version),
            kind: "message.edited",
            actor_user_id: Some(cmd.actor_id),
            command_id: Uuid::new_v4(),
            facts: json!({
                "conversationId": current.conversation_id,
                "threadRootId": thread_root_id,
                "replyToId": reply_to_id,
                "addedMentionUserIds": added,
            }),
            security_context: json!({"conversationId": current.conversation_id.to_string()}),
        },
    )
    .await?;
    let message = fetch_message(&mut tx, cmd.message_id).await?;
    let body =
        serde_json::to_value(&message).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        200,
        &body,
    )
    .await?;
    tx.commit().await?;
    Ok(Outcome {
        http_status: 200,
        value: message,
        changes: vec![ChangeNotice::Conversation {
            conversation_id: current.conversation_id,
            change_seq,
            message_id: cmd.message_id,
            message_version: new_version,
            change_kind: "message.edited".to_string(),
        }],
    })
}

pub async fn delete_message(
    pool: &PgPool,
    cmd: DeleteMessage,
) -> Result<Outcome<Message>, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(existing) = load_row(&mut tx, cmd.message_id).await? else {
        return Err(CoreError::NotFound);
    };
    let locked = lock_conversation(&mut tx, existing.conversation_id).await?;
    let current = load_row_for_update(&mut tx, cmd.message_id).await?;
    let access = access::require_read(&mut tx, cmd.actor_id, current.conversation_id).await?;
    access.require_writable()?;
    if !can_delete(&access, current.author_id, cmd.actor_id) {
        return Err(CoreError::forbidden("not allowed to delete this message"));
    }
    let actor = platform::actor_key(cmd.actor_id);
    let digest = platform::fingerprint("message.delete", &cmd.raw);
    if let Some(outcome) = platform::replay_outcome::<Message>(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        "message.delete",
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    if current.deleted_at.is_some() {
        let message = fetch_message(&mut tx, cmd.message_id).await?;
        let body = serde_json::to_value(&message)
            .map_err(|error| CoreError::Internal(error.to_string()))?;
        platform::complete(
            &mut tx,
            locked.workspace_id,
            &actor,
            &cmd.idempotency_key,
            200,
            &body,
        )
        .await?;
        tx.commit().await?;
        return Ok(Outcome {
            http_status: 200,
            value: message,
            changes: Vec::new(),
        });
    }
    archive_version(&mut tx, &current, cmd.actor_id).await?;
    let tombstone = json!({"type": "doc", "content": []});
    sqlx::query(
        "UPDATE penfii.messages
         SET body = $2, body_schema_version = 1, version = version + 1, deleted_at = now()
         WHERE id = $1",
    )
    .bind(cmd.message_id)
    .bind(&tombstone)
    .execute(&mut *tx)
    .await?;
    let change_seq = bump_change(&mut tx, current.conversation_id).await?;
    let new_version = current.version + 1;
    insert_change(
        &mut tx,
        current.conversation_id,
        change_seq,
        cmd.message_id,
        new_version,
        "message.deleted",
    )
    .await?;
    crate::events::emit(
        &mut tx,
        crate::events::NewEvent {
            workspace_id: locked.workspace_id,
            target_kind: "MESSAGE",
            target_id: cmd.message_id,
            target_version: Some(new_version),
            kind: "message.deleted",
            actor_user_id: Some(cmd.actor_id),
            command_id: Uuid::new_v4(),
            facts: json!({"conversationId": current.conversation_id}),
            security_context: json!({"conversationId": current.conversation_id.to_string()}),
        },
    )
    .await?;
    let message = fetch_message(&mut tx, cmd.message_id).await?;
    let body =
        serde_json::to_value(&message).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        200,
        &body,
    )
    .await?;
    tx.commit().await?;
    Ok(Outcome {
        http_status: 200,
        value: message,
        changes: vec![ChangeNotice::Conversation {
            conversation_id: current.conversation_id,
            change_seq,
            message_id: cmd.message_id,
            message_version: new_version,
            change_kind: "message.deleted".to_string(),
        }],
    })
}

pub async fn message_history(
    pool: &PgPool,
    user_id: Uuid,
    message_id: Uuid,
) -> Result<MessageHistory, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = load_row(&mut tx, message_id).await? else {
        return Err(CoreError::NotFound);
    };
    let access = match access::load_conversation(&mut tx, user_id, row.conversation_id).await? {
        Some(access) => access,
        None => return Err(CoreError::NotFound),
    };
    let auditor = if access.kind == "CHANNEL" {
        channel_auditor(&mut tx, row.conversation_id, user_id).await?
    } else {
        false
    };
    let allowed = (row.author_id == user_id && access.can_read()) || auditor;
    if !allowed {
        return Err(CoreError::NotFound);
    }
    let versions = sqlx::query_as::<_, VersionRow>(
        "SELECT version, body_schema_version, body, changed_by, recorded_at
         FROM penfii.message_versions
         WHERE message_id = $1
         ORDER BY version ASC",
    )
    .bind(message_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(MessageHistory {
        message_id,
        versions: versions.into_iter().map(version_from).collect(),
    })
}

pub async fn add_reaction(
    pool: &PgPool,
    cmd: ReactionCommand,
) -> Result<Outcome<ReactionState>, CoreError> {
    set_reaction(pool, cmd, true).await
}

pub async fn remove_reaction(
    pool: &PgPool,
    cmd: ReactionCommand,
) -> Result<Outcome<ReactionState>, CoreError> {
    set_reaction(pool, cmd, false).await
}

async fn set_reaction(
    pool: &PgPool,
    cmd: ReactionCommand,
    add: bool,
) -> Result<Outcome<ReactionState>, CoreError> {
    validate_emoji(&cmd.emoji)?;
    let kind = if add {
        "reaction.add"
    } else {
        "reaction.remove"
    };
    let change_kind = if add {
        "reaction.added"
    } else {
        "reaction.removed"
    };
    let mut tx = pool.begin().await?;
    let Some(existing) = load_row(&mut tx, cmd.message_id).await? else {
        return Err(CoreError::NotFound);
    };
    let locked = lock_conversation(&mut tx, existing.conversation_id).await?;
    let current = load_row_for_update(&mut tx, cmd.message_id).await?;
    let access = access::require_read(&mut tx, cmd.actor_id, current.conversation_id).await?;
    access.require_writable()?;
    if !access.can_write() {
        return Err(CoreError::forbidden("channel membership required"));
    }
    let actor = platform::actor_key(cmd.actor_id);
    let digest = platform::fingerprint(kind, &cmd.raw);
    if let Some(outcome) = platform::replay_outcome::<ReactionState>(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        kind,
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    let changed = if add {
        let inserted = sqlx::query(
            "INSERT INTO penfii.message_reactions (message_id, user_id, emoji)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(cmd.message_id)
        .bind(cmd.actor_id)
        .bind(&cmd.emoji)
        .execute(&mut *tx)
        .await?;
        inserted.rows_affected() == 1
    } else {
        let deleted = sqlx::query(
            "DELETE FROM penfii.message_reactions
             WHERE message_id = $1 AND user_id = $2 AND emoji = $3",
        )
        .bind(cmd.message_id)
        .bind(cmd.actor_id)
        .bind(&cmd.emoji)
        .execute(&mut *tx)
        .await?;
        deleted.rows_affected() == 1
    };
    let mut changes = Vec::new();
    if changed {
        let change_seq = bump_change(&mut tx, current.conversation_id).await?;
        insert_change(
            &mut tx,
            current.conversation_id,
            change_seq,
            cmd.message_id,
            current.version,
            change_kind,
        )
        .await?;
        changes.push(ChangeNotice::Conversation {
            conversation_id: current.conversation_id,
            change_seq,
            message_id: cmd.message_id,
            message_version: current.version,
            change_kind: change_kind.to_string(),
        });
    }
    let user_ids = emoji_users(&mut tx, cmd.message_id, &cmd.emoji).await?;
    let state = ReactionState {
        message_id: cmd.message_id,
        emoji: cmd.emoji,
        user_ids,
    };
    let body =
        serde_json::to_value(&state).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        locked.workspace_id,
        &actor,
        &cmd.idempotency_key,
        200,
        &body,
    )
    .await?;
    tx.commit().await?;
    Ok(Outcome {
        http_status: 200,
        value: state,
        changes,
    })
}

pub(crate) struct CreatedMessage {
    pub workspace_id: Uuid,
    pub conversation_id: Uuid,
    pub author_id: Uuid,
    pub body_schema_version: i16,
    pub body: Value,
    pub thread_root_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub mentions: Vec<ParsedMention>,
    pub file_ids: Vec<Uuid>,
    /// `USER` for a person, `INTEGRATION` for a webhook actor. Stored on `message.created`.
    pub actor_kind: &'static str,
    pub security_context: Value,
}

/// Inserts one message and emits `message.created`. The caller has already decided the author may
/// post and owns the transaction.
pub(crate) async fn persist_created_message(
    conn: &mut PgConnection,
    limits: &Limits,
    msg: CreatedMessage,
) -> Result<(Message, i64), CoreError> {
    ensure_mention_users(conn, &msg.mentions).await?;
    let (stream_seq, change_seq) = bump_message_and_change(conn, msg.conversation_id).await?;
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.messages
            (id, conversation_id, stream_seq, author_id, thread_root_id, reply_to_id,
             version, body_schema_version, body)
         VALUES ($1, $2, $3, $4, $5, $6, 1, $7, $8)",
    )
    .bind(message_id)
    .bind(msg.conversation_id)
    .bind(stream_seq)
    .bind(msg.author_id)
    .bind(msg.thread_root_id)
    .bind(msg.reply_to_id)
    .bind(msg.body_schema_version)
    .bind(&msg.body)
    .execute(&mut *conn)
    .await?;
    insert_mentions(conn, message_id, &msg.mentions).await?;
    files::attach_to_message(
        conn,
        limits,
        msg.author_id,
        msg.workspace_id,
        message_id,
        &msg.file_ids,
    )
    .await?;
    insert_change(
        conn,
        msg.conversation_id,
        change_seq,
        message_id,
        1,
        "message.created",
    )
    .await?;
    let mentioned: Vec<Uuid> = msg.mentions.iter().map(|m| m.user_id).collect();
    crate::events::emit_with_kind(
        conn,
        crate::events::NewEvent {
            workspace_id: msg.workspace_id,
            target_kind: "MESSAGE",
            target_id: message_id,
            target_version: Some(1),
            kind: "message.created",
            actor_user_id: Some(msg.author_id),
            command_id: Uuid::new_v4(),
            facts: json!({
                "conversationId": msg.conversation_id,
                "threadRootId": msg.thread_root_id,
                "replyToId": msg.reply_to_id,
                "mentionedUserIds": mentioned,
                "hasFile": !msg.file_ids.is_empty(),
            }),
            security_context: msg.security_context,
        },
        msg.actor_kind,
    )
    .await?;
    let message = fetch_message(conn, message_id).await?;
    Ok((message, change_seq))
}

pub(crate) fn prepare_body(
    limits: &Limits,
    schema_version: i16,
    body: &Value,
) -> Result<(), CoreError> {
    let max = limits.message_body_max_bytes.ok_or_else(|| {
        CoreError::conflict(
            "message_limit_unconfigured",
            "message body limit is not configured",
        )
    })?;
    if schema_version != 1 {
        return Err(CoreError::validation("bodySchemaVersion must be 1"));
    }
    let bytes =
        serde_json::to_vec(body).map_err(|error| CoreError::validation(error.to_string()))?;
    let len = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if len > max {
        return Err(CoreError::validation(
            "message body exceeds the configured maximum",
        ));
    }
    Ok(())
}

fn map_rich(error: RichTextError) -> CoreError {
    match error {
        RichTextError::MentionPolicy => CoreError::conflict(
            "mention_policy_unconfigured",
            "mention policy is not configured",
        ),
        RichTextError::Empty => CoreError::validation("message body is empty"),
        RichTextError::Invalid(reason) => {
            CoreError::validation(format!("message body is invalid: {reason}"))
        }
    }
}

fn can_delete(access: &ConvAccess, author_id: Uuid, actor_id: Uuid) -> bool {
    if author_id == actor_id {
        return true;
    }
    access.kind == "CHANNEL" && access.channel_role.as_deref() == Some("MANAGER")
}

fn version_conflict(row: &MessageRow) -> CoreError {
    let mut extra = Map::new();
    extra.insert("version".into(), json!(row.version));
    let body = if row.deleted_at.is_some() {
        Value::Null
    } else {
        row.body.clone()
    };
    extra.insert("body".into(), body);
    CoreError::conflict_extra("version_conflict", "version conflict", extra)
}

fn validate_emoji(emoji: &str) -> Result<(), CoreError> {
    if emoji.is_empty() || emoji.len() > 16 || emoji.graphemes(true).count() != 1 {
        return Err(CoreError::validation(
            "emoji must be a single emoji grapheme",
        ));
    }
    let base = emoji.chars().any(is_emoji_base);
    let keycap = emoji.chars().any(|ch| ch == '\u{20E3}')
        && emoji
            .chars()
            .any(|ch| ch.is_ascii_digit() || ch == '#' || ch == '*');
    if !base && !keycap {
        return Err(CoreError::validation(
            "emoji must be a single emoji grapheme",
        ));
    }
    Ok(())
}

fn is_emoji_base(ch: char) -> bool {
    let code = u32::from(ch);
    matches!(
        code,
        0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139
            | 0x2194..=0x2199
            | 0x21A9..=0x21AA
            | 0x231A..=0x231B
            | 0x2328
            | 0x23CF
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x24C2
            | 0x25AA..=0x25AB
            | 0x25B6
            | 0x25C0
            | 0x25FB..=0x25FE
            | 0x2600..=0x27BF
            | 0x2934..=0x2935
            | 0x2B05..=0x2B07
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x1F000..=0x1FAFF
    )
}

async fn lock_conversation(conn: &mut PgConnection, id: Uuid) -> Result<LockedConv, CoreError> {
    let row = sqlx::query_as::<_, LockedConv>(
        "SELECT workspace_id FROM penfii.conversations WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.ok_or(CoreError::NotFound)
}

async fn bump_message_and_change(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<(i64, i64), CoreError> {
    let row = sqlx::query_as::<_, (i64, i64)>(
        "UPDATE penfii.conversations
         SET last_message_seq = last_message_seq + 1,
             last_change_seq = last_change_seq + 1
         WHERE id = $1
         RETURNING last_message_seq, last_change_seq",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

async fn bump_change(conn: &mut PgConnection, id: Uuid) -> Result<i64, CoreError> {
    let seq = sqlx::query_scalar(
        "UPDATE penfii.conversations
         SET last_change_seq = last_change_seq + 1
         WHERE id = $1
         RETURNING last_change_seq",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(seq)
}

async fn insert_change(
    conn: &mut PgConnection,
    conversation_id: Uuid,
    change_seq: i64,
    message_id: Uuid,
    message_version: i64,
    change_kind: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.conversation_changes
            (conversation_id, change_seq, message_id, message_version, change_kind)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(conversation_id)
    .bind(change_seq)
    .bind(message_id)
    .bind(message_version)
    .bind(change_kind)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn validate_targets(
    conn: &mut PgConnection,
    conversation_id: Uuid,
    thread_root_id: Option<Uuid>,
    reply_to_id: Option<Uuid>,
) -> Result<(), CoreError> {
    let Some(root_id) = thread_root_id else {
        if reply_to_id.is_some() {
            return Err(CoreError::conflict(
                "reply_target_rejected",
                "reply target rejected",
            ));
        }
        return Ok(());
    };
    let Some(root) = load_row(conn, root_id).await? else {
        return Err(CoreError::conflict(
            "reply_target_rejected",
            "reply target rejected",
        ));
    };
    if root.conversation_id != conversation_id || root.thread_root_id.is_some() {
        return Err(CoreError::conflict(
            "reply_target_rejected",
            "reply target rejected",
        ));
    }
    if let Some(reply_id) = reply_to_id {
        let Some(target) = load_row(conn, reply_id).await? else {
            return Err(CoreError::conflict(
                "reply_target_rejected",
                "reply target rejected",
            ));
        };
        let in_thread = target.id == root_id || target.thread_root_id == Some(root_id);
        if target.conversation_id != conversation_id || !in_thread {
            return Err(CoreError::conflict(
                "reply_target_rejected",
                "reply target rejected",
            ));
        }
    }
    Ok(())
}

async fn ensure_mention_users(
    conn: &mut PgConnection,
    mentions: &[ParsedMention],
) -> Result<(), CoreError> {
    for mention in mentions {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.users WHERE id = $1 AND disabled_at IS NULL)",
        )
        .bind(mention.user_id)
        .fetch_one(&mut *conn)
        .await?;
        if !exists {
            return Err(CoreError::validation("mentioned user does not exist"));
        }
    }
    Ok(())
}

async fn insert_mentions(
    conn: &mut PgConnection,
    message_id: Uuid,
    mentions: &[ParsedMention],
) -> Result<(), CoreError> {
    for mention in mentions {
        sqlx::query(
            "INSERT INTO penfii.message_mentions
                (message_id, occurrence_id, mention_kind, user_id)
             VALUES ($1, $2, 'USER', $3)",
        )
        .bind(message_id)
        .bind(mention.occurrence_id)
        .bind(mention.user_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn archive_version(
    conn: &mut PgConnection,
    row: &MessageRow,
    actor_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.message_versions
            (message_id, version, body_schema_version, body, changed_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(row.id)
    .bind(row.version)
    .bind(row.body_schema_version)
    .bind(&row.body)
    .bind(actor_id)
    .execute(&mut *conn)
    .await?;
    files::copy_version_files(conn, row.id, row.version).await?;
    Ok(())
}

async fn load_row(conn: &mut PgConnection, id: Uuid) -> Result<Option<MessageRow>, CoreError> {
    let row = sqlx::query_as::<_, MessageRow>(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM penfii.messages WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

async fn load_row_for_update(conn: &mut PgConnection, id: Uuid) -> Result<MessageRow, CoreError> {
    let row = sqlx::query_as::<_, MessageRow>(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM penfii.messages WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.ok_or(CoreError::NotFound)
}

async fn fetch_message(conn: &mut PgConnection, id: Uuid) -> Result<Message, CoreError> {
    let row = load_row(conn, id).await?.ok_or(CoreError::NotFound)?;
    map_messages(conn, vec![row])
        .await?
        .into_iter()
        .next()
        .ok_or(CoreError::NotFound)
}

async fn map_messages(
    conn: &mut PgConnection,
    rows: Vec<MessageRow>,
) -> Result<Vec<Message>, CoreError> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let author_ids: Vec<Uuid> = rows.iter().map(|row| row.author_id).collect();
    let reactions = reaction_map(conn, &ids).await?;
    let authors = author_info(conn, &author_ids).await?;
    let replies = reply_counts(conn, &ids).await?;
    let mut attachments: HashMap<Uuid, Vec<MessageAttachment>> = HashMap::new();
    for (message_id, attachment) in files::attachments_for(conn, &ids).await? {
        attachments.entry(message_id).or_default().push(attachment);
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let grouped = reactions.get(&row.id).cloned().unwrap_or_default();
            let (name, kind) = authors
                .get(&row.author_id)
                .cloned()
                .unwrap_or_else(|| (String::new(), "HUMAN".to_string()));
            let reply_count = replies.get(&row.id).copied().unwrap_or(0);
            let files = attachments.remove(&row.id).unwrap_or_default();
            to_message(row, grouped, name, kind, reply_count, files)
        })
        .collect())
}

async fn author_info(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, (String, String)>, CoreError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, display_name, kind FROM penfii.users WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, kind)| (id, (name, kind)))
        .collect())
}

async fn reply_counts(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, CoreError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT thread_root_id, count(*)::bigint
         FROM penfii.messages
         WHERE thread_root_id = ANY($1) AND deleted_at IS NULL
         GROUP BY thread_root_id",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn reaction_map(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Reaction>>, CoreError> {
    let mut map: HashMap<Uuid, Vec<Reaction>> = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let rows = sqlx::query_as::<_, ReactionRow>(
        "SELECT message_id, user_id, emoji
         FROM penfii.message_reactions
         WHERE message_id = ANY($1)
         ORDER BY message_id, emoji, user_id",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    for row in rows {
        let list = map.entry(row.message_id).or_default();
        if let Some(last) = list.last_mut()
            && last.emoji == row.emoji
        {
            last.user_ids.push(row.user_id);
            continue;
        }
        list.push(Reaction {
            emoji: row.emoji,
            user_ids: vec![row.user_id],
        });
    }
    Ok(map)
}

async fn emoji_users(
    conn: &mut PgConnection,
    message_id: Uuid,
    emoji: &str,
) -> Result<Vec<Uuid>, CoreError> {
    let ids = sqlx::query_scalar(
        "SELECT user_id FROM penfii.message_reactions
         WHERE message_id = $1 AND emoji = $2
         ORDER BY user_id",
    )
    .bind(message_id)
    .bind(emoji)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids)
}

fn to_message(
    row: MessageRow,
    reactions: Vec<Reaction>,
    author_display_name: String,
    author_kind: String,
    reply_count: i64,
    attachments: Vec<MessageAttachment>,
) -> Message {
    let deleted = row.deleted_at.is_some();
    Message {
        id: row.id,
        conversation_id: row.conversation_id,
        stream_seq: row.stream_seq,
        author_id: row.author_id,
        author_display_name,
        author_kind,
        thread_root_id: row.thread_root_id,
        reply_to_id: row.reply_to_id,
        version: row.version,
        body_schema_version: row.body_schema_version,
        body: if deleted { None } else { Some(row.body) },
        deleted,
        created_at: row.created_at,
        edited_at: row.edited_at,
        reply_count,
        reactions,
        attachments,
    }
}

async fn channel_auditor(
    conn: &mut PgConnection,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<bool, CoreError> {
    let exists = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM penfii.channel_content_auditors
            WHERE channel_id = $1 AND user_id = $2
         )",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(exists)
}

#[derive(sqlx::FromRow)]
struct VersionRow {
    version: i64,
    body_schema_version: i16,
    body: Value,
    changed_by: Uuid,
    recorded_at: DateTime<Utc>,
}

fn version_from(row: VersionRow) -> MessageVersion {
    MessageVersion {
        version: row.version,
        body_schema_version: row.body_schema_version,
        body: row.body,
        changed_by: row.changed_by,
        recorded_at: row.recorded_at,
    }
}
