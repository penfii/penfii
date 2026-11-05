//! The two consumers behind Watch and Inbox.
//!
//! `WatchConsumer` makes a person follow what they took part in. `InboxConsumer` decides who an
//! event reaches and writes one Inbox item per person. Both read the event once or many times with
//! the same result: every write is `ON CONFLICT DO NOTHING`.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::error::CoreError;
use crate::events::{EventRow, HandlerFuture, PgConsumer};
use crate::platform::ChangeNotice;
use crate::preferences::{self, is_watcher_kind};
use crate::{visibility, watch};

/// Highest first. When several reasons reach one person for one event, the first one wins.
const PRIORITY: &[&str] = &[
    "ticket.assigned",
    "mention.user",
    "dependency.released",
    "reply.direct",
    "reminder.due",
    "thread.reply",
    "ticket.commented",
    "ticket.status_changed",
    "ticket.field_changed",
    "sprint.started",
    "sprint.closed",
];

fn uuid_at(value: &Value, key: &str) -> Option<Uuid> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn uuids_at(value: &Value, key: &str) -> Vec<Uuid> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|s| Uuid::parse_str(s).ok())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Watch
// ---------------------------------------------------------------------------------------------

pub struct WatchConsumer;

impl PgConsumer for WatchConsumer {
    fn name(&self) -> &'static str {
        "watch"
    }

    fn accepts(&self, kind: &str) -> bool {
        kind.starts_with("ticket.") || kind == "message.created"
    }

    fn handle<'a>(&'a self, conn: &'a mut PgConnection, event: &'a EventRow) -> HandlerFuture<'a> {
        Box::pin(async move {
            let ws = event.workspace_id;
            if event.kind.starts_with("ticket.") {
                let ticket = event.target_id;
                if event.kind == "ticket.created"
                    && let Some(actor) = event.actor_user_id
                {
                    watch::follow_automatically(conn, actor, ws, "TICKET", ticket).await?;
                }
                if event.kind == "ticket.commented"
                    && let Some(actor) = event.actor_user_id
                {
                    watch::follow_automatically(conn, actor, ws, "TICKET", ticket).await?;
                }
                // Being given a Ticket makes the assignee follow it.
                if let Some(assignee) = uuid_at(&event.facts["after"], "assigneeId") {
                    watch::follow_automatically(conn, assignee, ws, "TICKET", ticket).await?;
                }
            } else if event.kind == "message.created"
                && let Some(root) = uuid_at(&event.facts, "threadRootId")
            {
                // Replying joins the thread, and so does the author of the root: they started it.
                if let Some(actor) = event.actor_user_id {
                    watch::follow_automatically(conn, actor, ws, "THREAD", root).await?;
                }
                let author: Option<Uuid> =
                    sqlx::query_scalar("SELECT author_id FROM penfii.messages WHERE id=$1")
                        .bind(root)
                        .fetch_optional(&mut *conn)
                        .await?;
                if let Some(author) = author {
                    watch::follow_automatically(conn, author, ws, "THREAD", root).await?;
                }
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Inbox
// ---------------------------------------------------------------------------------------------

/// What an item is about; its visibility is asked of the shared service per person.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Subject {
    Ticket(Uuid),
    Message(Uuid),
    Project(Uuid),
    File(Uuid),
    Doc(Uuid),
}

pub(crate) struct Candidate {
    /// What the item is about when that differs from the event's own subject (a freed Ticket).
    pub(crate) subject: Option<Subject>,
    pub(crate) recipient: Uuid,
    pub(crate) kind: &'static str,
    pub(crate) logical_key: String,
    pub(crate) params: Value,
}

pub struct InboxConsumer {
    touched: Mutex<HashSet<Uuid>>,
    changes: Option<broadcast::Sender<ChangeNotice>>,
}

impl InboxConsumer {
    pub fn new(changes: Option<broadcast::Sender<ChangeNotice>>) -> Arc<Self> {
        Arc::new(Self {
            touched: Mutex::new(HashSet::new()),
            changes,
        })
    }
}

impl PgConsumer for InboxConsumer {
    fn name(&self) -> &'static str {
        "inbox"
    }

    fn accepts(&self, kind: &str) -> bool {
        kind.starts_with("ticket.")
            || matches!(
                kind,
                "doc.comment_created"
                    | "doc.comment_edited"
                    | "message.created"
                    | "message.edited"
                    | "sprint.started"
                    | "sprint.closed"
                    | "reminder.due"
            )
    }

    fn handle<'a>(&'a self, conn: &'a mut PgConnection, event: &'a EventRow) -> HandlerFuture<'a> {
        Box::pin(async move {
            let (subject, candidates) = candidates_for(conn, event).await?;
            let Some(subject) = subject else {
                return Ok(());
            };
            let delivered = deliver(conn, event, subject, candidates).await?;
            if !delivered.is_empty()
                && let Ok(mut touched) = self.touched.lock()
            {
                touched.extend(delivered);
            }
            Ok(())
        })
    }

    fn committed(&self) {
        let users: Vec<Uuid> = match self.touched.lock() {
            Ok(mut touched) => touched.drain().collect(),
            Err(_) => return,
        };
        if let Some(changes) = &self.changes {
            for user_id in users {
                let _ = changes.send(ChangeNotice::Inbox { user_id });
            }
        }
    }
}

async fn candidates_for(
    conn: &mut PgConnection,
    event: &EventRow,
) -> Result<(Option<Subject>, Vec<Candidate>), CoreError> {
    let kind = event.kind.as_str();
    if kind == "message.created" || kind == "message.edited" {
        return message_candidates(conn, event).await;
    }
    if kind == "doc.comment_created" || kind == "doc.comment_edited" {
        return Ok(doc_comment_candidates(event));
    }
    if kind == "sprint.started" || kind == "sprint.closed" {
        return sprint_candidates(conn, event).await;
    }
    if kind == "reminder.due" {
        return crate::reminders::due_candidates(conn, event).await;
    }
    ticket_candidates(conn, event).await
}

async fn message_candidates(
    conn: &mut PgConnection,
    event: &EventRow,
) -> Result<(Option<Subject>, Vec<Candidate>), CoreError> {
    let message = event.target_id;
    let facts = &event.facts;
    let conversation = uuid_at(facts, "conversationId");
    let mut out = Vec::new();
    let mentioned = if event.kind == "message.created" {
        uuids_at(facts, "mentionedUserIds")
    } else {
        uuids_at(facts, "addedMentionUserIds")
    };
    let params = json!({"conversationId": conversation, "messageId": message, "threadRootId": facts.get("threadRootId")});
    for user in mentioned {
        out.push(Candidate {
            subject: None,
            recipient: user,
            kind: "mention.user",
            logical_key: format!("mention:{message}:{user}"),
            params: params.clone(),
        });
    }
    if event.kind == "message.created" {
        if let Some(reply_to) = uuid_at(facts, "replyToId") {
            let author: Option<Uuid> =
                sqlx::query_scalar("SELECT author_id FROM penfii.messages WHERE id=$1")
                    .bind(reply_to)
                    .fetch_optional(&mut *conn)
                    .await?;
            if let Some(author) = author {
                out.push(Candidate {
                    subject: None,
                    recipient: author,
                    kind: "reply.direct",
                    logical_key: format!("reply:{message}:{author}"),
                    params: params.clone(),
                });
            }
        }
        if let Some(root) = uuid_at(facts, "threadRootId") {
            for user in watch::followers(conn, "THREAD", root).await? {
                out.push(Candidate {
                    subject: None,
                    recipient: user,
                    kind: "thread.reply",
                    logical_key: format!("thread:{message}:{user}"),
                    params: params.clone(),
                });
            }
        }
    }
    Ok((Some(Subject::Message(message)), out))
}

/// A comment on a Doc page notifies the people it mentions. Nothing else about a Doc reaches
/// the Inbox: mentions inside the page body do not notify. Whether a mentioned person may
/// read the page is asked in `deliver`, at handling time.
fn doc_comment_candidates(event: &EventRow) -> (Option<Subject>, Vec<Candidate>) {
    let comment = uuid_at(&event.facts, "commentId");
    let mentioned = if event.kind == "doc.comment_created" {
        uuids_at(&event.facts, "mentionedUserIds")
    } else {
        uuids_at(&event.facts, "addedMentionIds")
    };
    let page = event.target_id;
    let params = json!({"docId": page, "commentId": comment});
    let out = mentioned
        .into_iter()
        .map(|user| Candidate {
            subject: None,
            recipient: user,
            kind: "mention.user",
            logical_key: format!(
                "doc-mention:{}:{user}",
                comment.map(|c| c.to_string()).unwrap_or_default()
            ),
            params: params.clone(),
        })
        .collect();
    (Some(Subject::Doc(page)), out)
}

async fn sprint_candidates(
    conn: &mut PgConnection,
    event: &EventRow,
) -> Result<(Option<Subject>, Vec<Candidate>), CoreError> {
    let Some(project) = uuid_at(&event.facts, "projectId") else {
        return Ok((None, Vec::new()));
    };
    let kind: &'static str = if event.kind == "sprint.started" {
        "sprint.started"
    } else {
        "sprint.closed"
    };
    let params =
        json!({"projectId": project, "sprintId": event.target_id, "name": event.facts.get("name")});
    let out = watch::followers(conn, "PROJECT", project)
        .await?
        .into_iter()
        .map(|user| Candidate {
            subject: None,
            recipient: user,
            kind,
            logical_key: format!("{kind}:{}:{user}", event.target_id),
            params: params.clone(),
        })
        .collect();
    Ok((Some(Subject::Project(project)), out))
}

async fn ticket_candidates(
    conn: &mut PgConnection,
    event: &EventRow,
) -> Result<(Option<Subject>, Vec<Candidate>), CoreError> {
    let ticket = event.target_id;
    let facts = &event.facts;
    let (before, after) = (&facts["before"], &facts["after"]);
    let project = uuid_at(after, "projectId");
    let base = json!({"ticketId": ticket, "projectId": project});
    let mut out = Vec::new();
    let watchers = watch::followers(conn, "TICKET", ticket).await?;

    // Being given the Ticket reaches the assignee whether or not they follow.
    let new_assignee = uuid_at(after, "assigneeId");
    let assigned = new_assignee.is_some()
        && (event.kind == "ticket.created" || uuid_at(before, "assigneeId") != new_assignee);
    if assigned && let Some(user) = new_assignee {
        out.push(Candidate {
            subject: None,
            recipient: user,
            kind: "ticket.assigned",
            logical_key: format!("assigned:{}:{user}", event.event_id),
            params: base.clone(),
        });
    }
    if event.kind == "ticket.commented" {
        let mut params = base.clone();
        params["commentId"] = facts.get("commentId").cloned().unwrap_or(Value::Null);
        for user in &watchers {
            out.push(Candidate {
                subject: None,
                recipient: *user,
                kind: "ticket.commented",
                logical_key: format!("commented:{}:{user}", event.event_id),
                params: params.clone(),
            });
        }
    }
    if before.is_object() && after.is_object() && event.kind != "ticket.created" {
        if before.get("statusId") != after.get("statusId") && after.get("statusId").is_some() {
            let mut params = base.clone();
            params["fromStatusId"] = before["statusId"].clone();
            params["toStatusId"] = after["statusId"].clone();
            params["toCategory"] = after["statusCategory"].clone();
            for user in &watchers {
                out.push(Candidate {
                    subject: None,
                    recipient: *user,
                    kind: "ticket.status_changed",
                    logical_key: format!("status:{}:{user}", event.event_id),
                    params: params.clone(),
                });
            }
        }
        let changed: Vec<&str> = ["dueOn", "priority", "title", "typeId"]
            .into_iter()
            .filter(|field| before.get(*field) != after.get(*field))
            .collect();
        if !changed.is_empty() {
            let mut params = base.clone();
            params["fields"] = json!(changed);
            for user in &watchers {
                out.push(Candidate {
                    subject: None,
                    recipient: *user,
                    kind: "ticket.field_changed",
                    logical_key: format!("fields:{}:{user}", event.event_id),
                    params: params.clone(),
                });
            }
        }
        // The last unmet blocker finishing frees the Tickets waiting on it.
        if after["statusCategory"] == "DONE" && before["statusCategory"] != "DONE" {
            let freed = sqlx::query(
                "SELECT d.successor_id, t.assignee_id
                 FROM penfii.ticket_dependencies d JOIN penfii.tickets t ON t.id = d.successor_id
                 WHERE d.predecessor_id = $1 AND d.kind = 'BLOCKS'
                   AND t.trashed_at IS NULL AND t.purged_at IS NULL AND t.archived_at IS NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM penfii.ticket_dependencies o
                     JOIN penfii.tickets p ON p.id = o.predecessor_id
                     JOIN penfii.workflow_statuses ps ON ps.id = p.status_id
                     WHERE o.successor_id = d.successor_id AND o.kind = 'BLOCKS'
                       AND o.waived_at IS NULL AND ps.category <> 'DONE')",
            )
            .bind(ticket)
            .fetch_all(&mut *conn)
            .await?;
            for row in freed {
                let successor: Uuid = row.try_get("successor_id")?;
                let assignee: Option<Uuid> = row.try_get("assignee_id")?;
                let mut recipients: Vec<Uuid> = watch::followers(conn, "TICKET", successor).await?;
                recipients.extend(assignee);
                let params =
                    json!({"ticketId": successor, "predecessorId": ticket, "projectId": project});
                for user in recipients {
                    out.push(Candidate {
                        // The item is about the Ticket that was freed, so that is what each
                        // recipient must be able to see.
                        subject: Some(Subject::Ticket(successor)),
                        recipient: user,
                        kind: "dependency.released",
                        logical_key: format!("released:{successor}:{ticket}:{user}"),
                        params: params.clone(),
                    });
                }
            }
        }
    }
    Ok((Some(Subject::Ticket(ticket)), out))
}

/// Applies the rules every kind shares and writes the items. Returns the people who got one.
async fn deliver(
    conn: &mut PgConnection,
    event: &EventRow,
    subject: Subject,
    candidates: Vec<Candidate>,
) -> Result<Vec<Uuid>, CoreError> {
    // One reason per person: the strongest.
    let mut best: BTreeMap<Uuid, Candidate> = BTreeMap::new();
    for candidate in candidates {
        // A person's own action does not notify them (a Reminder is theirs by design).
        if candidate.kind != "reminder.due" && Some(candidate.recipient) == event.actor_user_id {
            continue;
        }
        let rank = |kind: &str| {
            PRIORITY
                .iter()
                .position(|k| *k == kind)
                .unwrap_or(usize::MAX)
        };
        match best.get(&candidate.recipient) {
            Some(existing) if rank(existing.kind) <= rank(candidate.kind) => {}
            _ => {
                best.insert(candidate.recipient, candidate);
            }
        }
    }
    let conversation = match subject {
        Subject::Message(id) => {
            sqlx::query_scalar::<_, Uuid>("SELECT conversation_id FROM penfii.messages WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        _ => None,
    };
    let mut delivered = Vec::new();
    for (recipient, candidate) in best {
        if is_watcher_kind(candidate.kind) {
            let (prefs, _) = preferences::load(conn, recipient).await?;
            if prefs.level == "DIRECT_ONLY" {
                continue;
            }
            if candidate.kind == "thread.reply"
                && let Some(channel) = conversation
                && preferences::is_muted(conn, recipient, channel).await?
            {
                continue;
            }
        }
        // Permission is asked again here, now, for this person.
        let subject = candidate.subject.unwrap_or(subject);
        let visible = match subject {
            Subject::Ticket(id) => visibility::tickets(conn, recipient, &[id])
                .await?
                .contains(&id),
            Subject::Message(id) => visibility::messages(conn, recipient, &[id])
                .await?
                .contains(&id),
            Subject::Project(id) => visibility::projects(conn, recipient, &[id])
                .await?
                .contains(&id),
            Subject::File(id) => visibility::files(conn, recipient, &[id])
                .await?
                .contains(&id),
            Subject::Doc(id) => visibility::docs(conn, recipient, &[id])
                .await?
                .contains(&id),
        };
        if !visible {
            continue;
        }
        let mut decision = candidate.params.clone();
        decision["actorId"] = json!(event.actor_user_id);
        decision["subject"] = match subject {
            Subject::Ticket(id) => json!({"kind": "TICKET", "id": id}),
            Subject::Message(id) => json!({"kind": "MESSAGE", "id": id}),
            Subject::Project(id) => json!({"kind": "PROJECT", "id": id}),
            Subject::File(id) => json!({"kind": "FILE", "id": id}),
            Subject::Doc(id) => json!({"kind": "DOC", "id": id}),
        };
        sqlx::query(
            "INSERT INTO penfii.event_recipients(event_id, recipient_id, notification_kind, logical_key, decision)
             VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
        )
        .bind(event.event_id)
        .bind(recipient)
        .bind(candidate.kind)
        .bind(&candidate.logical_key)
        .bind(&decision)
        .execute(&mut *conn)
        .await?;
        let inserted = sqlx::query(
            "INSERT INTO penfii.inbox_items(id, workspace_id, event_id, recipient_id, notification_kind, logical_key)
             VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(event.workspace_id)
        .bind(event.event_id)
        .bind(recipient)
        .bind(candidate.kind)
        .bind(&candidate.logical_key)
        .execute(&mut *conn)
        .await?;
        if inserted.rows_affected() > 0 {
            delivered.push(recipient);
        }
    }
    Ok(delivered)
}
