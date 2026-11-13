//! Command receipts. The command function owns the surrounding transaction.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::CoreError;

#[derive(Debug, Clone)]
pub enum ChangeNotice {
    Conversation {
        conversation_id: Uuid,
        change_seq: i64,
        message_id: Uuid,
        message_version: i64,
        change_kind: String,
    },
    /// A person's Inbox changed; the user socket tells their client to look again.
    Inbox { user_id: Uuid },
    /// A person lost (or may have lost) access: sockets they hold close, and reopen only if the
    /// next access check passes.
    Access { user_id: Uuid },
    /// The channels a person can see changed (they were put into one, or left one): the user
    /// socket tells their client to read its Space again.
    Space { user_id: Uuid },
    /// Many Tickets of a Project changed at once (a Sprint's finished work was archived): the board
    /// of that Project looks again. It carries nobody's data, so it is checked like a Project change.
    Project { project_id: Uuid },
    Ticket {
        project_id: Uuid,
        change_seq: i64,
        ticket_id: Uuid,
        ticket_revision_no: i64,
        change_kind: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredResult {
    status: u16,
    body: Value,
}

pub enum Claim {
    Fresh,
    Replayed { status: u16, body: Value },
}

pub fn actor_key(user_id: Uuid) -> String {
    format!("user:{user_id}")
}

pub fn fingerprint(command_kind: &str, body: &Value) -> Vec<u8> {
    let mut material = canonical_json(body).into_bytes();
    material.push(0);
    material.extend_from_slice(command_kind.as_bytes());
    Sha256::digest(&material).to_vec()
}

pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => {
            let inner = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{inner}]")
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner = keys
                .into_iter()
                .map(|key| {
                    let encoded = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{encoded}:{}", canonical_json(&map[key]))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
    }
}

pub async fn claim(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: &str,
    idempotency_key: &str,
    command_kind: &str,
    digest: &[u8],
) -> Result<Claim, CoreError> {
    if let Some(existing) = load_receipt(conn, workspace_id, actor, idempotency_key).await? {
        return decide_existing(existing, digest);
    }

    // ON CONFLICT instead of catching the violation: a failed statement would abort the caller's
    // transaction, so two requests with one key arriving together could not read the receipt.
    let inserted = sqlx::query(
        "INSERT INTO penfii.commands
            (id, workspace_id, actor_key, idempotency_key, command_kind, request_fingerprint)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (workspace_id, actor_key, idempotency_key) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(workspace_id)
    .bind(actor)
    .bind(idempotency_key)
    .bind(command_kind)
    .bind(digest)
    .execute(&mut *conn)
    .await?;
    if inserted.rows_affected() == 1 {
        return Ok(Claim::Fresh);
    }
    let existing = load_receipt(conn, workspace_id, actor, idempotency_key)
        .await?
        .ok_or_else(|| CoreError::Internal("idempotency row missing".into()))?;
    decide_existing(existing, digest)
}

pub async fn complete(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: &str,
    idempotency_key: &str,
    status: u16,
    body: &Value,
) -> Result<(), CoreError> {
    let stored = StoredResult {
        status,
        body: body.clone(),
    };
    let refs =
        serde_json::to_value(&stored).map_err(|error| CoreError::Internal(error.to_string()))?;
    sqlx::query(
        "UPDATE penfii.commands
         SET result_refs = $4, result_schema_version = 1, completed_at = now()
         WHERE workspace_id = $1 AND actor_key = $2 AND idempotency_key = $3",
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(idempotency_key)
    .bind(refs)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

struct Receipt {
    request_fingerprint: Vec<u8>,
    result_refs: Value,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn load_receipt(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: &str,
    idempotency_key: &str,
) -> Result<Option<Receipt>, CoreError> {
    let row = sqlx::query_as::<_, Receipt>(
        "SELECT request_fingerprint, result_refs, completed_at
         FROM penfii.commands
         WHERE workspace_id = $1 AND actor_key = $2 AND idempotency_key = $3
         FOR UPDATE",
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(idempotency_key)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

fn decide_existing(existing: Receipt, digest: &[u8]) -> Result<Claim, CoreError> {
    if existing.request_fingerprint.as_slice() != digest {
        return Err(CoreError::conflict(
            "idempotency_conflict",
            "idempotency key was already used with a different request",
        ));
    }
    if existing.completed_at.is_none() {
        return Err(CoreError::Internal(
            "idempotency receipt is incomplete".into(),
        ));
    }
    let stored: StoredResult = serde_json::from_value(existing.result_refs)
        .map_err(|error| CoreError::Internal(error.to_string()))?;
    Ok(Claim::Replayed {
        status: stored.status,
        body: stored.body,
    })
}

/// The stored result of a command that was already done under this idempotency key, or `None` when the
/// key is fresh and the command should run (and end with [`finish`]). The same key with a different
/// request is a conflict. The caller owns the transaction and commits it before returning the replay.
pub(crate) async fn replay<T: serde::de::DeserializeOwned>(
    conn: &mut PgConnection,
    workspace: Uuid,
    actor: Uuid,
    idempotency_key: &str,
    command_kind: &str,
    body: &Value,
) -> Result<Option<T>, CoreError> {
    let digest = fingerprint(command_kind, body);
    match claim(
        conn,
        workspace,
        &actor_key(actor),
        idempotency_key,
        command_kind,
        &digest,
    )
    .await?
    {
        Claim::Fresh => Ok(None),
        Claim::Replayed { body, .. } => Ok(Some(
            serde_json::from_value(body).map_err(|error| CoreError::Internal(error.to_string()))?,
        )),
    }
}

/// Records the result a command returns, so [`replay`] can return it again.
pub(crate) async fn finish<T: Serialize>(
    conn: &mut PgConnection,
    workspace: Uuid,
    actor: Uuid,
    idempotency_key: &str,
    status: u16,
    value: &T,
) -> Result<(), CoreError> {
    let body =
        serde_json::to_value(value).map_err(|error| CoreError::Internal(error.to_string()))?;
    complete(
        conn,
        workspace,
        &actor_key(actor),
        idempotency_key,
        status,
        &body,
    )
    .await
}

/// [`replay`] for the commands that return an [`crate::Outcome`] and may have stored an error: a stored
/// status of 400 or more is returned as the same error again, any other as the same outcome with no
/// change notices (the first run already sent them). `digest` is the request's [`fingerprint`].
pub(crate) async fn replay_outcome<T: serde::de::DeserializeOwned>(
    conn: &mut PgConnection,
    workspace: Uuid,
    actor: &str,
    idempotency_key: &str,
    command_kind: &str,
    digest: &[u8],
) -> Result<Option<crate::Outcome<T>>, CoreError> {
    match claim(
        conn,
        workspace,
        actor,
        idempotency_key,
        command_kind,
        digest,
    )
    .await?
    {
        Claim::Fresh => Ok(None),
        Claim::Replayed { status, body } => {
            if status >= 400 {
                return Err(replay_error(&body));
            }
            let value = serde_json::from_value(body)
                .map_err(|error| CoreError::Internal(error.to_string()))?;
            Ok(Some(crate::Outcome {
                http_status: status,
                value,
                changes: Vec::new(),
            }))
        }
    }
}

pub fn replay_error(body: &Value) -> CoreError {
    use serde_json::Map;
    let code = body.get("code").and_then(Value::as_str).unwrap_or("");
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("conflict");
    let static_code = match code {
        "draft_conflict" => "draft_conflict",
        "idempotency_conflict" => "idempotency_conflict",
        "version_conflict" => "version_conflict",
        "stale_read" => "stale_read",
        "read_not_observed" => "read_not_observed",
        "message_deleted" => "message_deleted",
        "mention_policy_unconfigured" => "mention_policy_unconfigured",
        "reply_target_rejected" => "reply_target_rejected",
        _ => "conflict",
    };
    let mut extra = Map::new();
    if let Some(object) = body.as_object() {
        for (key, value) in object {
            if key != "code" && key != "message" {
                extra.insert(key.clone(), value.clone());
            }
        }
    }
    CoreError::conflict_extra(static_code, message, extra)
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for Receipt {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            request_fingerprint: row.try_get("request_fingerprint")?,
            result_refs: row.try_get("result_refs")?,
            completed_at: row.try_get("completed_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_json;
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_object_keys() {
        let left = json!({"b": 1, "a": {"d": true, "c": "x"}});
        let right = json!({"a": {"c": "x", "d": true}, "b": 1});
        assert_eq!(canonical_json(&left), canonical_json(&right));
        assert_eq!(
            canonical_json(&left),
            "{\"a\":{\"c\":\"x\",\"d\":true},\"b\":1}"
        );
    }
}
