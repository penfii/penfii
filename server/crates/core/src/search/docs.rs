//! Builds index documents from PostgreSQL. The same functions serve the live consumer and the
//! reindex job, so the index can never hold a shape that only one of them produces.

use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::work_history::description_text;

const COMMENT_CAP_BYTES: usize = 50 * 1024;

/// What a document may hold; the operator's values (`PENFII_SEARCH_*`).
#[derive(Debug, Clone, Copy)]
pub struct BuildLimits {
    pub doc_body_max_bytes: usize,
    pub doc_comments_max_bytes: usize,
}

impl Default for BuildLimits {
    fn default() -> Self {
        Self {
            doc_body_max_bytes: 524_288,
            doc_comments_max_bytes: 51_200,
        }
    }
}

impl From<&crate::knowledge::KnowledgeConfig> for BuildLimits {
    fn from(config: &crate::knowledge::KnowledgeConfig) -> Self {
        Self {
            doc_body_max_bytes: config.search_doc_body_max_bytes as usize,
            doc_comments_max_bytes: config.search_comments_max_bytes as usize,
        }
    }
}

pub struct Built {
    pub id: Uuid,
    /// `None` means the object must not be in the index (gone, trashed, or not indexable).
    pub doc: Option<Value>,
    /// The external version this write carries: the PostgreSQL clock (microseconds) read in the
    /// statement that opened the snapshot every read of this build used. A document built from a
    /// later snapshot always carries a larger version, whichever process writes first (H1).
    pub version: i64,
}

fn time(value: chrono::DateTime<chrono::Utc>) -> Value {
    json!(value.to_rfc3339())
}

/// Builds the documents for `ids` from one consistent PostgreSQL snapshot and stamps each with the
/// clock read at the start of that snapshot (see [`Built::version`]). Reading the clock apart from
/// the data, or after it, is what let a staler read carry a larger version.
pub async fn build(
    conn: &mut PgConnection,
    kind: &str,
    ids: &[Uuid],
    limits: &BuildLimits,
) -> Result<Vec<Built>, CoreError> {
    // A sqlx transaction, not a bare BEGIN: if this future is dropped half way the connection goes
    // back to the pool rolled back. SET TRANSACTION is the first statement, so it still applies.
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let built = build_in_snapshot(&mut tx, kind, ids, limits).await;
    // Read only: there is nothing to commit, and a rollback also clears an aborted transaction.
    tx.rollback().await?;
    built
}

async fn build_in_snapshot(
    conn: &mut PgConnection,
    kind: &str,
    ids: &[Uuid],
    limits: &BuildLimits,
) -> Result<Vec<Built>, CoreError> {
    // The first statement of a REPEATABLE READ transaction takes the snapshot, and the clock is
    // read inside that same statement, so a later snapshot carries a later clock. Two reads whose
    // snapshots and clocks fall within one statement's duration can still invert (a residual window); the next event or a reindex rewrites the document and nothing widens access.
    let version: i64 =
        sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint")
            .fetch_one(&mut *conn)
            .await?;
    let mut found = match kind {
        "message" => messages(conn, ids).await?,
        "ticket" => tickets(conn, ids).await?,
        "space" => spaces(conn, ids).await?,
        "file" => files(conn, ids).await?,
        "view" => views(conn, ids).await?,
        "doc" => doc_pages(conn, ids, limits).await?,
        _ => Vec::new(),
    };
    for built in &mut found {
        built.version = version;
    }
    for id in ids {
        if !found.iter().any(|b| b.id == *id) {
            found.push(Built {
                id: *id,
                doc: None,
                version,
            });
        }
    }
    Ok(found)
}

async fn messages(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<Built>, CoreError> {
    let rows = sqlx::query(
        "SELECT m.id, m.conversation_id, m.author_id, m.thread_root_id, m.version, m.body, m.created_at,
                COALESCE(m.edited_at, m.created_at) AS updated_at, c.workspace_id, c.kind AS conv_kind,
                EXISTS(SELECT 1 FROM penfii.message_attachments a WHERE a.message_id = m.id) AS has_file,
                COALESCE((SELECT array_agg(mm.user_id) FROM penfii.message_mentions mm
                          WHERE mm.message_id = m.id AND mm.user_id IS NOT NULL AND mm.removed_at IS NULL), '{}') AS mentioned
         FROM penfii.messages m JOIN penfii.conversations c ON c.id = m.conversation_id
         WHERE m.id = ANY($1) AND m.deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let conversation: Uuid = row.try_get("conversation_id")?;
        let conv_kind: String = row.try_get("conv_kind")?;
        let body: Value = row.try_get("body")?;
        let text = description_text(&body);
        let mentioned: Vec<Uuid> = row.try_get("mentioned")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "message", "id": id,
                "createdAt": time(row.try_get("created_at")?),
                "updatedAt": time(row.try_get("updated_at")?),
                "sourceVersion": row.try_get::<i64, _>("version")?,
                "text": text,
                "conversationId": conversation,
                "channelId": (conv_kind == "CHANNEL").then_some(conversation),
                "threadRootId": row.try_get::<Option<Uuid>, _>("thread_root_id")?,
                "authorId": row.try_get::<Uuid, _>("author_id")?,
                "mentionedUserIds": mentioned,
                "hasFile": row.try_get::<bool, _>("has_file")?,
            })),
        });
    }
    Ok(out)
}

async fn tickets(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<Built>, CoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.workspace_id, t.project_id, t.channel_id, t.public_key, t.title, t.description,
                t.ticket_type_id, t.status_id, ws.category, t.assignee_id, t.created_by, t.priority,
                t.parent_id, t.due_on, t.revision_no, t.created_at, t.updated_at,
                COALESCE((SELECT array_agg(tl.label_id) FROM penfii.ticket_labels tl WHERE tl.ticket_id = t.id), '{}') AS labels
         FROM penfii.tickets t JOIN penfii.workflow_statuses ws ON ws.id = t.status_id
         WHERE t.id = ANY($1) AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let comments: Vec<Value> = sqlx::query_scalar(
            "SELECT body FROM penfii.ticket_comments WHERE ticket_id = $1 AND deleted_at IS NULL ORDER BY created_at, id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        let mut joined = String::new();
        for body in comments {
            let text = description_text(&body);
            if joined.len() + text.len() + 1 > COMMENT_CAP_BYTES {
                break;
            }
            joined.push_str(&text);
            joined.push('\n');
        }
        let due: Option<chrono::NaiveDate> = row.try_get("due_on")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "ticket", "id": id,
                "createdAt": time(row.try_get("created_at")?),
                "updatedAt": time(row.try_get("updated_at")?),
                "sourceVersion": row.try_get::<i64, _>("revision_no")?,
                "key": row.try_get::<String, _>("public_key")?,
                "title": row.try_get::<String, _>("title")?,
                "description": description_text(&row.try_get::<Value, _>("description")?),
                "comments": joined,
                "projectId": row.try_get::<Uuid, _>("project_id")?,
                "channelId": row.try_get::<Option<Uuid>, _>("channel_id")?,
                "typeId": row.try_get::<Option<Uuid>, _>("ticket_type_id")?,
                "statusId": row.try_get::<Uuid, _>("status_id")?,
                "statusCategory": row.try_get::<String, _>("category")?,
                "priority": row.try_get::<Option<String>, _>("priority")?,
                "assigneeId": row.try_get::<Option<Uuid>, _>("assignee_id")?,
                "creatorId": row.try_get::<Uuid, _>("created_by")?,
                "labelIds": row.try_get::<Vec<Uuid>, _>("labels")?,
                "parentId": row.try_get::<Option<Uuid>, _>("parent_id")?,
                "dueOn": due.map(|d| d.to_string()),
            })),
        });
    }
    Ok(out)
}

async fn spaces(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<Built>, CoreError> {
    let mut out = Vec::new();
    for row in sqlx::query(
        "SELECT id, workspace_id, name, ticket_prefix, archived_at, created_at FROM penfii.projects WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?
    {
        let id: Uuid = row.try_get("id")?;
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "space", "id": id, "spaceKind": "PROJECT",
                "createdAt": time(created), "updatedAt": time(created), "sourceVersion": 1,
                "name": row.try_get::<String, _>("name")?,
                "key": row.try_get::<Option<String>, _>("ticket_prefix")?,
                "archived": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("archived_at")?.is_some(),
            })),
        });
    }
    for row in sqlx::query(
        "SELECT id, workspace_id, name, archived_at FROM penfii.teams WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?
    {
        let id: Uuid = row.try_get("id")?;
        let now = chrono::Utc::now();
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "space", "id": id, "spaceKind": "TEAM",
                "createdAt": time(now), "updatedAt": time(now), "sourceVersion": 1,
                "name": row.try_get::<String, _>("name")?,
                "archived": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("archived_at")?.is_some(),
            })),
        });
    }
    for row in sqlx::query(
        "SELECT ch.id, c.workspace_id, ch.name, ch.description, ch.archived_at, ch.project_id, ch.team_id, c.created_at
         FROM penfii.channels ch JOIN penfii.conversations c ON c.id = ch.id WHERE ch.id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?
    {
        let id: Uuid = row.try_get("id")?;
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "space", "id": id, "spaceKind": "CHANNEL",
                "createdAt": time(created), "updatedAt": time(created), "sourceVersion": 1,
                "name": row.try_get::<String, _>("name")?,
                "description": row.try_get::<Option<String>, _>("description")?,
                "archived": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("archived_at")?.is_some(),
                "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
                "teamId": row.try_get::<Option<Uuid>, _>("team_id")?,
            })),
        });
    }
    Ok(out)
}

async fn files(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<Built>, CoreError> {
    let rows = sqlx::query(
        "SELECT f.id, f.workspace_id, f.filename, f.detected_mime, f.byte_size, f.created_by, f.created_at,
                array_agg(DISTINCT m.conversation_id) AS conversations, array_agg(DISTINCT m.id) AS messages
         FROM penfii.files f
         JOIN penfii.message_attachments a ON a.file_id = f.id
         JOIN penfii.messages m ON m.id = a.message_id AND m.deleted_at IS NULL
         WHERE f.id = ANY($1) AND f.state = 'AVAILABLE' AND f.deleted_at IS NULL
         GROUP BY f.id",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    // Files on a live Ticket have no conversation; who may see them is decided when results are
    // filtered, as for every file, and the Ticket is looked up when a result is shown (the mapping
    // stays as it is, so no reindex is needed).
    let ticket_rows = sqlx::query(
        "SELECT f.id, f.workspace_id, f.filename, f.detected_mime, f.byte_size, f.created_by, f.created_at,
                a.ticket_id
         FROM penfii.files f
         JOIN penfii.ticket_attachments a ON a.file_id = f.id
         JOIN penfii.tickets t ON t.id = a.ticket_id
              AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
         WHERE f.id = ANY($1) AND f.state = 'AVAILABLE' AND f.deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in ticket_rows {
        let id: Uuid = row.try_get("id")?;
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "file", "id": id,
                "createdAt": time(created), "updatedAt": time(created), "sourceVersion": 1,
                "filename": row.try_get::<String, _>("filename")?,
                "mime": row.try_get::<Option<String>, _>("detected_mime")?,
                "byteSize": row.try_get::<i64, _>("byte_size")?,
                "uploaderId": row.try_get::<Uuid, _>("created_by")?,
                "conversationIds": Vec::<Uuid>::new(),
                "messageIds": Vec::<Uuid>::new(),
            })),
        });
    }
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "file", "id": id,
                "createdAt": time(created), "updatedAt": time(created), "sourceVersion": 1,
                "filename": row.try_get::<String, _>("filename")?,
                "mime": row.try_get::<Option<String>, _>("detected_mime")?,
                "byteSize": row.try_get::<i64, _>("byte_size")?,
                "uploaderId": row.try_get::<Uuid, _>("created_by")?,
                "conversationIds": row.try_get::<Vec<Uuid>, _>("conversations")?,
                "messageIds": row.try_get::<Vec<Uuid>, _>("messages")?,
            })),
        });
    }
    Ok(out)
}

async fn views(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<Built>, CoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, name, scope, target_kind, project_id, team_id, owner_id, created_at, updated_at
         FROM penfii.project_views WHERE id = ANY($1) AND deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "view", "id": id,
                "createdAt": time(row.try_get("created_at")?),
                "updatedAt": time(row.try_get("updated_at")?),
                "sourceVersion": 1,
                "name": row.try_get::<String, _>("name")?,
                "scopeKind": row.try_get::<String, _>("target_kind")?,
                "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
                "teamId": row.try_get::<Option<Uuid>, _>("team_id")?,
                "ownerId": row.try_get::<Uuid, _>("owner_id")?,
                "shared": row.try_get::<String, _>("scope")? == "PROJECT",
            })),
        });
    }
    Ok(out)
}

/// A Doc page and its comments are one document. The body is plain text taken from the
/// materialized `doc_pages.content`, never from the live Yjs state.
async fn doc_pages(
    conn: &mut PgConnection,
    ids: &[Uuid],
    limits: &BuildLimits,
) -> Result<Vec<Built>, CoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, project_id, parent_id, title, content, revision_no, created_by, created_at, updated_at
         FROM penfii.doc_pages WHERE id = ANY($1) AND deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let content: Value = row.try_get("content")?;
        let (body, cut) = crate::docs::content::plain_text(&content, limits.doc_body_max_bytes);
        if cut {
            tracing::info!(page = %id, "search_doc_body_truncated");
        }
        let comments: Vec<Value> = sqlx::query_scalar(
            "SELECT body FROM penfii.doc_comments WHERE page_id = $1 AND deleted_at IS NULL ORDER BY created_at, id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        let mut joined = String::new();
        for body in comments {
            let text = description_text(&body);
            if joined.len() + text.len() + 1 > limits.doc_comments_max_bytes {
                break;
            }
            joined.push_str(&text);
            joined.push('\n');
        }
        out.push(Built {
            id,
            version: 0,
            doc: Some(json!({
                "workspaceId": row.try_get::<Uuid, _>("workspace_id")?,
                "kind": "doc", "id": id,
                "createdAt": time(row.try_get("created_at")?),
                "updatedAt": time(row.try_get("updated_at")?),
                "sourceVersion": row.try_get::<i64, _>("revision_no")?,
                "title": row.try_get::<String, _>("title")?,
                "body": body,
                "comments": joined,
                "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
                "parentId": row.try_get::<Option<Uuid>, _>("parent_id")?,
                "creatorId": row.try_get::<Uuid, _>("created_by")?,
            })),
        });
    }
    Ok(out)
}

/// Ids of every indexable object of a kind after `after`, in id order, for the reindex scan.
pub async fn scan(
    conn: &mut PgConnection,
    kind: &str,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Uuid>, CoreError> {
    let after = after.unwrap_or(Uuid::nil());
    let sql = match kind {
        "message" => {
            "SELECT id FROM penfii.messages WHERE deleted_at IS NULL AND id > $1 ORDER BY id LIMIT $2"
        }
        "ticket" => {
            "SELECT id FROM penfii.tickets WHERE archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL AND id > $1 ORDER BY id LIMIT $2"
        }
        "space" => {
            "SELECT id FROM (SELECT id FROM penfii.projects UNION ALL SELECT id FROM penfii.teams UNION ALL SELECT id FROM penfii.channels) s WHERE id > $1 ORDER BY id LIMIT $2"
        }
        "file" => {
            "SELECT f.id FROM penfii.files f WHERE f.state='AVAILABLE' AND f.deleted_at IS NULL AND f.id > $1
             AND (EXISTS(SELECT 1 FROM penfii.message_attachments a JOIN penfii.messages m ON m.id=a.message_id AND m.deleted_at IS NULL WHERE a.file_id=f.id)
                 OR EXISTS(SELECT 1 FROM penfii.ticket_attachments a JOIN penfii.tickets t ON t.id=a.ticket_id AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL WHERE a.file_id=f.id)) ORDER BY f.id LIMIT $2"
        }
        "view" => {
            "SELECT id FROM penfii.project_views WHERE deleted_at IS NULL AND id > $1 ORDER BY id LIMIT $2"
        }
        "doc" => {
            "SELECT id FROM penfii.doc_pages WHERE deleted_at IS NULL AND id > $1 ORDER BY id LIMIT $2"
        }
        _ => return Ok(Vec::new()),
    };
    Ok(sqlx::query_scalar(sql)
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?)
}

/// How many documents a complete index of this kind holds, for the reindex verification.
pub async fn source_count(conn: &mut PgConnection, kind: &str) -> Result<i64, CoreError> {
    let sql = match kind {
        "message" => "SELECT count(*) FROM penfii.messages WHERE deleted_at IS NULL",
        "ticket" => {
            "SELECT count(*) FROM penfii.tickets WHERE archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL"
        }
        "space" => {
            "SELECT (SELECT count(*) FROM penfii.projects) + (SELECT count(*) FROM penfii.teams) + (SELECT count(*) FROM penfii.channels)"
        }
        "file" => {
            "SELECT count(*) FROM penfii.files f WHERE f.state='AVAILABLE' AND f.deleted_at IS NULL
             AND (EXISTS(SELECT 1 FROM penfii.message_attachments a JOIN penfii.messages m ON m.id=a.message_id AND m.deleted_at IS NULL WHERE a.file_id=f.id)
                 OR EXISTS(SELECT 1 FROM penfii.ticket_attachments a JOIN penfii.tickets t ON t.id=a.ticket_id AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL WHERE a.file_id=f.id))"
        }
        "view" => "SELECT count(*) FROM penfii.project_views WHERE deleted_at IS NULL",
        "doc" => "SELECT count(*) FROM penfii.doc_pages WHERE deleted_at IS NULL",
        _ => return Ok(0),
    };
    Ok(sqlx::query_scalar(sql).fetch_one(&mut *conn).await?)
}
