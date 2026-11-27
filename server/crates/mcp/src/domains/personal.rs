//! MCP operations: personal work (FEA-170): Inbox, Later and reminders, favorites, drafts,
//! subscriptions, channel mutes, notification settings and the person's own profile.
//!
//! Everything here is about the signed-in person and nobody else; `core` filters on the user id.
//! Reads never change read state: reading is `inbox_list`, marking is `inbox_update` / `inbox_read_all`.
//! Left to the web app: Push registration, password, e-mail, MFA, sessions, linked sign-ins, OAuth
//! grants and the avatar upload (see `web_only` and `coverage/personal.tsv`).

use penfii_core::oauth::{Principal, RiskClass};
use penfii_core::{ChangeNotice, drafts, inbox, personal, preferences, reminders, watch};
use serde_json::{Map, Value, json};

use super::Handled;
use crate::McpContext;
use crate::catalog::{
    Operation, Toolset, WorkspaceResolver, object_schema, uuid_property, workspace_id_property,
};
use crate::tools::{
    Done, Failure, announce, datetime_arg, done, limit_arg, nullable, reachable_policies,
    string_arg, text_arg, to_value, uuid_arg,
};

const R: &[&str] = &["personal:read"];
const W: &[&str] = &["personal:write"];

const fn by(arg: &'static str, sql: &'static str) -> WorkspaceResolver {
    WorkspaceResolver::Query { arg, sql }
}

const INBOX_ITEM: WorkspaceResolver = by(
    "inboxItemId",
    "SELECT workspace_id FROM penfii.inbox_items WHERE id = $1",
);
const LATER_ITEM: WorkspaceResolver = by(
    "laterId",
    "SELECT workspace_id FROM penfii.later_items WHERE id = $1",
);
const FOLDER: WorkspaceResolver = by(
    "folderId",
    "SELECT workspace_id FROM penfii.favorite_folders WHERE id = $1",
);
const DRAFT: WorkspaceResolver = by(
    "draftId",
    "SELECT workspace_id FROM penfii.drafts WHERE id = $1",
);
const DRAFT_CONTEXT: WorkspaceResolver = by(
    "contextId",
    "SELECT workspace_id FROM penfii.conversations WHERE id = $1
     UNION ALL
     SELECT c.workspace_id FROM penfii.messages m
         JOIN penfii.conversations c ON c.id = m.conversation_id WHERE m.id = $1 LIMIT 1",
);
const SUBSCRIPTION_TARGET: WorkspaceResolver = by(
    "targetId",
    "SELECT workspace_id FROM penfii.tickets WHERE id = $1
     UNION ALL SELECT workspace_id FROM penfii.projects WHERE id = $1
     UNION ALL SELECT c.workspace_id FROM penfii.messages m
         JOIN penfii.conversations c ON c.id = m.conversation_id WHERE m.id = $1 LIMIT 1",
);
const CHANNEL: WorkspaceResolver = by(
    "channelId",
    "SELECT workspace_id FROM penfii.conversations WHERE id = $1",
);
const WS: WorkspaceResolver = WorkspaceResolver::Argument;
const NONE: WorkspaceResolver = WorkspaceResolver::None;

fn ws_only() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property()}),
        &["workspaceId"],
    )
}

fn no_args() -> Value {
    object_schema(json!({}), &[])
}

fn schema_inbox_list() -> Value {
    object_schema(
        json!({
            "workspaceId": workspace_id_property(),
            "view": {"type": "string", "enum": ["inbox", "archived"], "description": "Default inbox."},
            "filter": {"type": "string", "description": "Notification group as in the web Inbox; default all."},
            "state": {"type": "string", "enum": ["unread"], "description": "unread keeps only unread items."},
            "includeSnoozed": {"type": "boolean"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 50},
            "cursor": {"type": "string", "description": "nextCursor of the previous page."}
        }),
        &["workspaceId"],
    )
}

fn schema_inbox_update() -> Value {
    object_schema(
        json!({
            "inboxItemId": uuid_property("Item id from inbox_list."),
            "command": {"type": "string", "enum": ["read", "unread", "archive", "unarchive", "snooze"]},
            "expectedVersion": {"type": "integer", "description": "version of the item from inbox_list; a stale value is refused."},
            "until": {"type": "string", "format": "date-time", "description": "Required for snooze."}
        }),
        &["inboxItemId", "command", "expectedVersion"],
    )
}

fn schema_read_all() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "filter": {"type": "string", "description": "Only this notification group; default all."}}),
        &["workspaceId"],
    )
}

fn schema_later_list() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "state": {"type": "string", "enum": ["ACTIVE", "DONE", "ARCHIVED"], "description": "Default ACTIVE."}}),
        &["workspaceId"],
    )
}

fn schema_saved_add() -> Value {
    object_schema(
        json!({
            "workspaceId": workspace_id_property(),
            "kind": {"type": "string", "enum": ["TICKET", "MESSAGE", "FILE"]},
            "targetId": uuid_property("Id of the Ticket, message or file."),
            "note": {"type": "string"}
        }),
        &["workspaceId", "kind", "targetId"],
    )
}

fn schema_later_update() -> Value {
    object_schema(
        json!({
            "laterId": uuid_property("Id from later_list."),
            "expectedVersion": {"type": "integer"},
            "state": {"type": "string", "enum": ["ACTIVE", "DONE", "ARCHIVED"]},
            "note": {"type": ["string", "null"], "description": "null clears the note."}
        }),
        &["laterId", "expectedVersion"],
    )
}

fn schema_later_id() -> Value {
    object_schema(
        json!({"laterId": uuid_property("Id from later_list.")}),
        &["laterId"],
    )
}

fn schema_reminder_set() -> Value {
    object_schema(
        json!({
            "laterId": uuid_property("Id from later_list."),
            "dueAt": {"type": "string", "format": "date-time"},
            "timeZone": {"type": "string", "description": "IANA zone such as Asia/Seoul."}
        }),
        &["laterId", "dueAt", "timeZone"],
    )
}

fn schema_favorite_add() -> Value {
    object_schema(
        json!({
            "workspaceId": workspace_id_property(),
            "kind": {"type": "string", "enum": ["PROJECT", "TEAM", "TICKET", "VIEW", "CHANNEL", "DM", "DOC"], "description": "What is favorited: PROJECT, TEAM, TICKET, VIEW (a saved view), CHANNEL, DM (a direct-message conversation id) or DOC (a Doc page)."},
            "targetId": uuid_property("Id of the favorited thing.")
        }),
        &["workspaceId", "kind", "targetId"],
    )
}

fn schema_favorite_order() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "items": {"type": "array", "maxItems": 500, "items": object_schema(
                   json!({"kind": {"type": "string"}, "targetId": uuid_property("Target id.")}),
                   &["kind", "targetId"])},
               "scope": {"type": "string", "enum": ["ALL", "LISTED"],
                   "description": "ALL (default): the listed favorites come first, the others keep their order after them. LISTED: only the listed favorites change places, among the places they already hold (one folder, for example)."}}),
        &["workspaceId", "items"],
    )
}

fn schema_favorite_move() -> Value {
    object_schema(
        json!({
            "workspaceId": workspace_id_property(),
            "kind": {"type": "string"},
            "targetId": uuid_property("Favorited item."),
            "folderId": {"type": ["string", "null"], "format": "uuid", "description": "null moves it to the top level."}
        }),
        &["workspaceId", "kind", "targetId", "folderId"],
    )
}

fn schema_folder_create() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "name": {"type": "string", "minLength": 1, "maxLength": 60}}),
        &["workspaceId", "name"],
    )
}

fn schema_folder_rename() -> Value {
    object_schema(
        json!({"folderId": uuid_property("Folder id from favorite_folder_list."),
               "name": {"type": "string", "minLength": 1, "maxLength": 60}}),
        &["folderId", "name"],
    )
}

fn schema_folder_order() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "folderIds": {"type": "array", "maxItems": 200, "items": uuid_property("Folder id.")}}),
        &["workspaceId", "folderIds"],
    )
}

fn schema_folder_id() -> Value {
    object_schema(
        json!({"folderId": uuid_property("Folder id from favorite_folder_list.")}),
        &["folderId"],
    )
}

fn schema_draft_save() -> Value {
    object_schema(
        json!({
            "contextKind": {"type": "string", "enum": ["CHANNEL_COMPOSE", "DM_COMPOSE", "THREAD_COMPOSE"]},
            "contextId": uuid_property("Conversation id, or the thread root message id for THREAD_COMPOSE."),
            "generation": uuid_property("Input session id: a time-ordered UUID (v7) you make when typing starts. A newer one starts over; an older or finished one is refused (draft_closed)."),
            "expectedVersion": {"type": "integer", "description": "version of the stored draft of this session; a stale value is refused (draft_conflict)."},
            "body": {"type": "object", "description": "Penfii rich-text document JSON; must not be empty."}
        }),
        &["contextKind", "contextId", "generation", "body"],
    )
}

fn schema_draft_id() -> Value {
    object_schema(
        json!({"draftId": uuid_property("Draft id from draft_list.")}),
        &["draftId"],
    )
}

fn schema_subscription_get() -> Value {
    object_schema(
        json!({"kind": {"type": "string", "enum": ["TICKET", "PROJECT", "THREAD"]},
               "targetId": uuid_property("Ticket id, Project id or the thread's root message id.")}),
        &["kind", "targetId"],
    )
}

fn schema_subscription_set() -> Value {
    object_schema(
        json!({"kind": {"type": "string", "enum": ["TICKET", "PROJECT", "THREAD"]},
               "targetId": uuid_property("Ticket id, Project id or the thread's root message id."),
               "watching": {"type": "boolean", "description": "true to follow, false to stop following (the unfollow is remembered; it is not re-followed automatically)."}}),
        &["kind", "targetId", "watching"],
    )
}

fn schema_subscription_list() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "kind": {"type": "string", "enum": ["TICKET", "PROJECT", "THREAD"], "description": "Only this kind."}}),
        &["workspaceId"],
    )
}

fn schema_mute() -> Value {
    object_schema(
        json!({"channelId": uuid_property("Channel id."),
               "muted": {"type": "boolean"}}),
        &["channelId", "muted"],
    )
}

fn schema_preferences_set() -> Value {
    object_schema(
        json!({
            "expectedVersion": {"type": "integer", "description": "version from notification_preferences_get."},
            "level": {"type": "string", "enum": ["ALL", "DIRECT_ONLY"]},
            "dnd": object_schema(json!({
                "enabled": {"type": "boolean"},
                "schedule": {"type": ["object", "null"], "description": "{days:[0-6, 0=Monday], start:'HH:MM', end:'HH:MM', timeZone}"},
                "snoozeUntil": {"type": ["string", "null"], "format": "date-time"}
            }), &["enabled"]),
            "external": {"type": "object", "description": "Reserved; leave out."}
        }),
        &["expectedVersion", "level", "dnd"],
    )
}

fn schema_profile_update() -> Value {
    object_schema(
        json!({
            "displayName": {"type": "string", "minLength": 1},
            "jobTitle": {"type": ["string", "null"]},
            "timeZone": {"type": ["string", "null"], "description": "IANA zone."},
            "statusEmoji": {"type": ["string", "null"]},
            "statusText": {"type": ["string", "null"]},
            "statusClearAt": {"type": ["string", "null"], "format": "date-time"}
        }),
        &[],
    )
}

pub static OPS: &[Operation] = &[
    Operation::new(
        Toolset::Personal,
        "inbox.list",
        "inbox_list",
        "List Inbox items",
        "List your Inbox (or the archive) in a Workspace, newest first, with unreadCount. Reading the list does NOT mark anything read; use inbox_update / inbox_read_all for that. Page with nextCursor.",
        R,
        RiskClass::Read,
        WS,
        schema_inbox_list,
    ),
    Operation::new(
        Toolset::Personal,
        "inbox.update",
        "inbox_update",
        "Change one Inbox item",
        "Mark one of your Inbox items read or unread, archive, unarchive, or snooze it (until required). Needs the item's current version from inbox_list; a stale one is refused.",
        W,
        RiskClass::Write,
        INBOX_ITEM,
        schema_inbox_update,
    ),
    Operation::new(
        Toolset::Personal,
        "inbox.read_all",
        "inbox_read_all",
        "Mark the Inbox read",
        "Mark all your unread Inbox items in a Workspace read (optionally only one notification group). Snoozed items stay unread. Returns how many changed.",
        W,
        RiskClass::Write,
        WS,
        schema_read_all,
    ),
    Operation::new(
        Toolset::Personal,
        "later.list",
        "later_list",
        "List Later items",
        "List what you saved for later in a Workspace (Tickets, messages, files) with note, state, version and reminder.",
        R,
        RiskClass::Read,
        WS,
        schema_later_list,
    ),
    Operation::new(
        Toolset::Personal,
        "later.add",
        "later_add",
        "Save for later",
        "Save a Ticket, message or file you can read to your Later list, with an optional note. Saving the same target again returns the existing item.",
        W,
        RiskClass::Write,
        WS,
        schema_saved_add,
    ),
    Operation::new(
        Toolset::Personal,
        "later.update",
        "later_update",
        "Update a Later item",
        "Change the state (ACTIVE, DONE, ARCHIVED) or note of one of your Later items. Needs the version from later_list.",
        W,
        RiskClass::Write,
        LATER_ITEM,
        schema_later_update,
    ),
    Operation::new(
        Toolset::Personal,
        "later.remove",
        "later_remove",
        "Remove a Later item",
        "Remove one of your Later items (and its reminder). Only your own bookmark goes; the target is untouched.",
        W,
        RiskClass::Write,
        LATER_ITEM,
        schema_later_id,
    ),
    Operation::new(
        Toolset::Personal,
        "later.reminder_set",
        "later_reminder_set",
        "Set a reminder on a Later item",
        "Remind you of a Later item at dueAt (in the future); a new reminder replaces the old one. The reminder arrives as an Inbox item.",
        W,
        RiskClass::Write,
        LATER_ITEM,
        schema_reminder_set,
    ),
    Operation::new(
        Toolset::Personal,
        "later.reminder_cancel",
        "later_reminder_cancel",
        "Cancel a reminder",
        "Cancel the reminder of a Later item. The item stays.",
        W,
        RiskClass::Write,
        LATER_ITEM,
        schema_later_id,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite.list",
        "favorite_list",
        "List favorites",
        "List your favorites in a Workspace in your order, with folders.",
        R,
        RiskClass::Read,
        WS,
        ws_only,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite.add",
        "favorite_add",
        "Add a favorite",
        "Add something you can read to your favorites. Adding it again changes nothing.",
        W,
        RiskClass::Write,
        WS,
        schema_favorite_add,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite.remove",
        "favorite_remove",
        "Remove a favorite",
        "Remove one thing from your favorites.",
        W,
        RiskClass::Write,
        WS,
        schema_favorite_add,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite.order",
        "favorite_order",
        "Reorder favorites",
        "Set the order of your favorites: items lists favorites (kind, targetId) in the new order; with every favorite listed it sets the whole order, and scope LISTED reorders only the listed ones among their own places. Returns the list.",
        W,
        RiskClass::Write,
        WS,
        schema_favorite_order,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite.move",
        "favorite_move",
        "Move a favorite into a folder",
        "Put a favorite into one of your folders, or to the top level with folderId null.",
        W,
        RiskClass::Write,
        WS,
        schema_favorite_move,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite_folder.list",
        "favorite_folder_list",
        "List favorite folders",
        "List your favorite folders in a Workspace.",
        R,
        RiskClass::Read,
        WS,
        ws_only,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite_folder.create",
        "favorite_folder_create",
        "Create a favorite folder",
        "Create a folder for your favorites. Retrying makes a second folder if the name is free.",
        W,
        RiskClass::Write,
        WS,
        schema_folder_create,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite_folder.rename",
        "favorite_folder_rename",
        "Rename a favorite folder",
        "Rename one of your favorite folders.",
        W,
        RiskClass::Write,
        FOLDER,
        schema_folder_rename,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite_folder.order",
        "favorite_folder_order",
        "Reorder favorite folders",
        "Set the order of your favorite folders: folderIds lists all of them in the new order.",
        W,
        RiskClass::Write,
        WS,
        schema_folder_order,
    ),
    Operation::new(
        Toolset::Personal,
        "favorite_folder.delete",
        "favorite_folder_delete",
        "Delete a favorite folder",
        "Delete one of your favorite folders; its favorites move to the top level and are kept.",
        W,
        RiskClass::Write,
        FOLDER,
        schema_folder_id,
    ),
    Operation::new(
        Toolset::Personal,
        "draft.list",
        "draft_list",
        "List your drafts",
        "List your unsent message drafts in a Workspace (only yours; drafts of conversations you can no longer write to are left out).",
        R,
        RiskClass::Read,
        WS,
        ws_only,
    ),
    Operation::new(
        Toolset::Personal,
        "draft.save",
        "draft_save",
        "Save a message draft",
        "Save your draft for a channel, DM or thread compose box. It is private to you and is not sent. Empty drafts are refused; use a new generation to start a new input session.",
        W,
        RiskClass::Write,
        DRAFT_CONTEXT,
        schema_draft_save,
    ),
    Operation::new(
        Toolset::Personal,
        "draft.delete",
        "draft_delete",
        "Discard a draft",
        "Discard one of your drafts (draftId from draft_list).",
        W,
        RiskClass::Write,
        DRAFT,
        schema_draft_id,
    ),
    Operation::new(
        Toolset::Personal,
        "subscription.get",
        "subscription_get",
        "Am I following this?",
        "Whether you follow a Ticket, a Project or a thread (its root message), and how (AUTO, EXPLICIT, MUTED).",
        R,
        RiskClass::Read,
        SUBSCRIPTION_TARGET,
        schema_subscription_get,
    ),
    Operation::new(
        Toolset::Personal,
        "subscription.set",
        "subscription_set",
        "Follow or unfollow",
        "Follow or stop following a Ticket, a Project or a thread. Following means its changes reach your Inbox.",
        W,
        RiskClass::Write,
        SUBSCRIPTION_TARGET,
        schema_subscription_set,
    ),
    Operation::new(
        Toolset::Personal,
        "subscription.list",
        "subscription_list",
        "List what you follow",
        "List the Tickets, Projects and threads you follow in a Workspace.",
        R,
        RiskClass::Read,
        WS,
        schema_subscription_list,
    ),
    Operation::new(
        Toolset::Personal,
        "channel_mute.set",
        "channel_mute_set",
        "Mute or unmute a channel",
        "Mute or unmute a channel for yourself (it stays quiet in your Inbox and badges). Does not change the channel for others.",
        W,
        RiskClass::Write,
        CHANNEL,
        schema_mute,
    ),
    Operation::new(
        Toolset::Personal,
        "channel_mute.list",
        "channel_mute_list",
        "List muted channels",
        "List the ids of the channels you muted, in all Workspaces.",
        R,
        RiskClass::Read,
        NONE,
        no_args,
    ),
    Operation::new(
        Toolset::Personal,
        "notification_preferences.get",
        "notification_preferences_get",
        "Read notification settings",
        "Your notification level (ALL or DIRECT_ONLY), quiet hours and current version. Push device registration is not here: it is done in the web app.",
        R,
        RiskClass::Read,
        NONE,
        no_args,
    ),
    Operation::new(
        Toolset::Personal,
        "notification_preferences.set",
        "notification_preferences_set",
        "Change notification settings",
        "Set your notification level and quiet hours (expectedVersion from notification_preferences_get). It replaces the whole setting. Push device registration is done in the web app.",
        W,
        RiskClass::Write,
        NONE,
        schema_preferences_set,
    ),
    Operation::new(
        Toolset::Personal,
        "me.get",
        "me_get",
        "Read my profile",
        "Your own account profile: id, name, e-mail, time zone, job title, status. Password, MFA, sign-ins and sessions are only in the web app.",
        R,
        RiskClass::Read,
        NONE,
        no_args,
    ),
    Operation::new(
        Toolset::Personal,
        "me.update_profile",
        "me_update_profile",
        "Change my public profile",
        "Change your display name, job title, time zone or status. Fields you leave out stay as they are; null clears one. E-mail, password and avatar are changed in the web app.",
        W,
        RiskClass::Write,
        NONE,
        schema_profile_update,
    ),
];

/// The most items a list answer carries; the person's own lists are short, and this keeps one
/// answer bounded whatever is stored.
const LIST_CAP: usize = 200;

fn capped(items: Value) -> Value {
    let mut items = items;
    let mut truncated = false;
    if let Some(array) = items.as_array_mut()
        && array.len() > LIST_CAP
    {
        array.truncate(LIST_CAP);
        truncated = true;
    }
    json!({"items": items, "truncated": truncated})
}

/// Operations tied to no Workspace (the profile, notification settings, muted channels) still obey
/// the Workspace MCP policies. A read runs when at least one Workspace this connection can reach
/// allows it (the rule `tools/list` uses). A change runs only when every reachable Workspace allows
/// it: the profile and the settings are visible in all of them, so one Workspace that turned this
/// kind of MCP access off must be able to hold it back.
async fn require_policy(
    ctx: &McpContext,
    principal: &Principal,
    risk: RiskClass,
) -> Result<(), Failure> {
    let policies = reachable_policies(ctx, principal).await?;
    let ok = if risk == RiskClass::Read {
        policies.iter().any(|p| p.allows(risk))
    } else {
        !policies.is_empty() && policies.iter().all(|p| p.allows(risk))
    };
    if ok {
        Ok(())
    } else {
        Err(Failure::new(
            "policy_denied",
            "a Workspace administrator has turned this kind of MCP access off",
        ))
    }
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

pub(super) async fn run(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Handled {
    let user = principal.user_id;
    let pool = &ctx.pool;
    Some(match op.id {
        "inbox.list" => {
            async {
                let workspace_id = uuid_arg(args, "workspaceId")?;
                let page = inbox::list(
                    pool,
                    user,
                    workspace_id,
                    inbox::ListOptions {
                        view: inbox::View::parse(string_arg(args, "view"))?,
                        filter: string_arg(args, "filter").unwrap_or("all"),
                        unread_only: string_arg(args, "state") == Some("unread"),
                        include_snoozed: args
                            .get("includeSnoozed")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                    string_arg(args, "cursor"),
                    Some(limit_arg(args, 20)?),
                )
                .await?;
                Ok(done(to_value(&page)?))
            }
            .await
        }
        "inbox.update" => {
            async {
                let item = uuid_arg(args, "inboxItemId")?;
                let expected = args
                    .get("expectedVersion")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Failure::invalid("expectedVersion must be an integer"))?;
                let command = match string_arg(args, "command") {
                    Some("read") => inbox::Command::Read,
                    Some("unread") => inbox::Command::Unread,
                    Some("archive") => inbox::Command::Archive,
                    Some("unarchive") => inbox::Command::Unarchive,
                    Some("snooze") => inbox::Command::Snooze(
                        string_arg(args, "until")
                            .and_then(|raw| raw.parse().ok())
                            .ok_or_else(|| Failure::invalid("snooze needs `until`"))?,
                    ),
                    _ => return Err(Failure::invalid("unknown command")),
                };
                let value = inbox::command(pool, user, item, expected, command).await?;
                announce(ctx, Some(ChangeNotice::Inbox { user_id: user }));
                Ok(Done::about(value, "inbox_item", item))
            }
            .await
        }
        "inbox.read_all" => {
            async {
                let workspace_id = uuid_arg(args, "workspaceId")?;
                let changed = inbox::read_all(
                    pool,
                    user,
                    workspace_id,
                    string_arg(args, "filter").unwrap_or("all"),
                )
                .await?;
                announce(ctx, Some(ChangeNotice::Inbox { user_id: user }));
                Ok(done(json!({"changed": changed})))
            }
            .await
        }
        "later.list" => {
            async {
                let workspace_id = uuid_arg(args, "workspaceId")?;
                let items = personal::list_later(
                    pool,
                    user,
                    workspace_id,
                    string_arg(args, "state").unwrap_or("ACTIVE"),
                )
                .await?;
                Ok(done(capped(to_value(&items)?)))
            }
            .await
        }
        "later.add" => {
            async {
                let item = personal::add_later(
                    pool,
                    user,
                    uuid_arg(args, "workspaceId")?,
                    &text_arg(args, "kind")?,
                    uuid_arg(args, "targetId")?,
                    string_arg(args, "note").map(str::to_string),
                )
                .await?;
                Ok(done(to_value(&item)?))
            }
            .await
        }
        "later.update" => {
            async {
                let later = uuid_arg(args, "laterId")?;
                let expected = args
                    .get("expectedVersion")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Failure::invalid("expectedVersion must be an integer"))?;
                let item = personal::update_later(
                    pool,
                    user,
                    later,
                    expected,
                    string_arg(args, "state").map(str::to_string),
                    nullable(args, "note", text)?,
                )
                .await?;
                Ok(Done::about(to_value(&item)?, "later_item", later))
            }
            .await
        }
        "later.remove" => {
            async {
                let later = uuid_arg(args, "laterId")?;
                personal::remove_later(pool, user, later).await?;
                Ok(done(json!({"laterId": later, "removed": true})))
            }
            .await
        }
        "later.reminder_set" => {
            async {
                let reminder = reminders::set(
                    pool,
                    user,
                    uuid_arg(args, "laterId")?,
                    datetime_arg(args, "dueAt")?,
                    string_arg(args, "timeZone").unwrap_or(""),
                )
                .await?;
                Ok(done(to_value(&reminder)?))
            }
            .await
        }
        "later.reminder_cancel" => {
            async {
                let later = uuid_arg(args, "laterId")?;
                reminders::cancel(pool, user, later).await?;
                Ok(done(json!({"laterId": later, "reminderCanceled": true})))
            }
            .await
        }
        "favorite.list" => {
            async {
                let items =
                    personal::list_favorites(pool, user, uuid_arg(args, "workspaceId")?).await?;
                Ok(done(capped(to_value(&items)?)))
            }
            .await
        }
        "favorite.add" => {
            async {
                let item = personal::add_favorite(
                    pool,
                    user,
                    uuid_arg(args, "workspaceId")?,
                    &text_arg(args, "kind")?,
                    uuid_arg(args, "targetId")?,
                )
                .await?;
                Ok(done(to_value(&item)?))
            }
            .await
        }
        "favorite.remove" => {
            async {
                personal::remove_favorite(
                    pool,
                    user,
                    uuid_arg(args, "workspaceId")?,
                    &text_arg(args, "kind")?,
                    uuid_arg(args, "targetId")?,
                )
                .await?;
                Ok(done(json!({"removed": true})))
            }
            .await
        }
        "favorite.order" => {
            async {
                let workspace_id = uuid_arg(args, "workspaceId")?;
                let items = args
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Failure::invalid("items must be an array"))?;
                let mut order = Vec::new();
                for item in items {
                    let kind = item
                        .get("kind")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Failure::invalid("each item needs kind"))?;
                    order.push((kind.to_string(), uuid_arg(item, "targetId")?));
                }
                let scope =
                    personal::FavoriteOrderScope::parse(args.get("scope").and_then(Value::as_str))?;
                personal::order_favorites(pool, user, workspace_id, order, scope).await?;
                let items = personal::list_favorites(pool, user, workspace_id).await?;
                Ok(done(capped(to_value(&items)?)))
            }
            .await
        }
        "favorite.move" => {
            async {
                let folder = nullable(args, "folderId", crate::tools::parse_uuid)?
                    .ok_or_else(|| Failure::invalid("folderId is required (null for top level)"))?;
                personal::move_favorite(
                    pool,
                    user,
                    uuid_arg(args, "workspaceId")?,
                    &text_arg(args, "kind")?,
                    uuid_arg(args, "targetId")?,
                    folder,
                )
                .await?;
                Ok(done(json!({"moved": true})))
            }
            .await
        }
        "favorite_folder.list" => {
            async {
                let items =
                    personal::list_folders(pool, user, uuid_arg(args, "workspaceId")?).await?;
                Ok(done(capped(to_value(&items)?)))
            }
            .await
        }
        "favorite_folder.create" => {
            async {
                let folder = personal::create_folder(
                    pool,
                    user,
                    uuid_arg(args, "workspaceId")?,
                    string_arg(args, "name").unwrap_or(""),
                )
                .await?;
                Ok(done(to_value(&folder)?))
            }
            .await
        }
        "favorite_folder.rename" => {
            async {
                let folder = personal::rename_folder(
                    pool,
                    user,
                    uuid_arg(args, "folderId")?,
                    string_arg(args, "name").unwrap_or(""),
                )
                .await?;
                Ok(done(to_value(&folder)?))
            }
            .await
        }
        "favorite_folder.order" => {
            async {
                let ids = args
                    .get("folderIds")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Failure::invalid("folderIds must be an array"))?
                    .iter()
                    .map(crate::tools::parse_uuid)
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| Failure::invalid("folderIds must be UUIDs"))?;
                personal::order_folders(pool, user, uuid_arg(args, "workspaceId")?, ids).await?;
                Ok(done(json!({"ordered": true})))
            }
            .await
        }
        "favorite_folder.delete" => {
            async {
                let moved =
                    personal::delete_folder(pool, user, uuid_arg(args, "folderId")?).await?;
                Ok(done(json!({"movedToTopLevel": moved})))
            }
            .await
        }
        "draft.list" => {
            async {
                let items = drafts::list_drafts(pool, user, uuid_arg(args, "workspaceId")?).await?;
                Ok(done(capped(to_value(&items)?)))
            }
            .await
        }
        "draft.save" => {
            async {
                let kind = string_arg(args, "contextKind")
                    .ok_or_else(|| Failure::invalid("contextKind is required"))?;
                let body = args
                    .get("body")
                    .filter(|body| body.is_object())
                    .cloned()
                    .ok_or_else(|| Failure::invalid("body must be a rich-text document object"))?;
                let (draft, created) = drafts::save_draft(
                    pool,
                    user,
                    kind,
                    uuid_arg(args, "contextId")?,
                    uuid_arg(args, "generation")?,
                    args.get("expectedVersion").and_then(Value::as_i64),
                    body,
                )
                .await?;
                let mut value = to_value(&draft)?;
                value["created"] = json!(created);
                Ok(Done::about(value, "draft", draft.id))
            }
            .await
        }
        "draft.delete" => {
            async {
                let draft = uuid_arg(args, "draftId")?;
                drafts::delete_draft(pool, user, draft).await?;
                Ok(done(json!({"draftId": draft, "discarded": true})))
            }
            .await
        }
        "subscription.get" => {
            async {
                let status = watch::status(
                    pool,
                    user,
                    &text_arg(args, "kind")?,
                    uuid_arg(args, "targetId")?,
                )
                .await?;
                Ok(done(to_value(&status)?))
            }
            .await
        }
        "subscription.set" => {
            async {
                let on = args
                    .get("watching")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Failure::invalid("watching must be true or false"))?;
                let status = watch::set(
                    pool,
                    user,
                    &text_arg(args, "kind")?,
                    uuid_arg(args, "targetId")?,
                    on,
                )
                .await?;
                Ok(done(to_value(&status)?))
            }
            .await
        }
        "subscription.list" => {
            async {
                let items = watch::list(
                    pool,
                    user,
                    uuid_arg(args, "workspaceId")?,
                    string_arg(args, "kind"),
                )
                .await?;
                Ok(done(capped(to_value(&items)?)))
            }
            .await
        }
        "channel_mute.set" => {
            async {
                let channel = uuid_arg(args, "channelId")?;
                let muted = args
                    .get("muted")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Failure::invalid("muted must be true or false"))?;
                preferences::set_mute(pool, user, channel, muted).await?;
                Ok(done(json!({"channelId": channel, "muted": muted})))
            }
            .await
        }
        "channel_mute.list" => {
            async {
                require_policy(ctx, principal, op.risk).await?;
                let ids = preferences::muted_channels(pool, user).await?;
                // Only channels of Workspaces this connection may use (grant filter and policy).
                let allowed: Vec<uuid::Uuid> = reachable_policies(ctx, principal)
                    .await?
                    .into_iter()
                    .filter(|p| p.allows(op.risk))
                    .map(|p| p.workspace_id)
                    .collect();
                let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
                    "SELECT id FROM penfii.conversations WHERE id = ANY($1) AND workspace_id = ANY($2)",
                )
                .bind(&ids)
                .bind(&allowed)
                .fetch_all(pool)
                .await?;
                Ok(done(json!({"channelIds": ids})))
            }
            .await
        }
        "notification_preferences.get" => {
            async {
                require_policy(ctx, principal, op.risk).await?;
                Ok(done(to_value(&preferences::view(pool, user).await?)?))
            }
            .await
        }
        "notification_preferences.set" => {
            async {
                require_policy(ctx, principal, op.risk).await?;
                let expected = args
                    .get("expectedVersion")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Failure::invalid("expectedVersion must be an integer"))?;
                let mut settings = args.as_object().cloned().unwrap_or_default();
                settings.remove("expectedVersion");
                settings.entry("external").or_insert_with(|| json!({}));
                let preferences_in: preferences::Preferences =
                    serde_json::from_value(Value::Object(settings))
                        .map_err(|e| Failure::invalid(format!("invalid settings: {e}")))?;
                let saved = preferences::save(pool, user, preferences_in, expected).await?;
                Ok(done(to_value(&saved)?))
            }
            .await
        }
        "me.get" => {
            async {
                require_policy(ctx, principal, op.risk).await?;
                let me = penfii_core::identity::load_user(pool, user).await?;
                Ok(done(to_value(&me)?))
            }
            .await
        }
        "me.update_profile" => {
            async {
                require_policy(ctx, principal, op.risk).await?;
                let current = penfii_core::identity::load_user(pool, user).await?;
                let given: &Map<String, Value> = args
                    .as_object()
                    .ok_or_else(|| Failure::invalid("arguments must be an object"))?;
                let pick = |name: &str,
                            current: Option<String>|
                 -> Result<Option<String>, Failure> {
                    match given.get(name) {
                        None => Ok(current),
                        Some(Value::Null) => Ok(None),
                        Some(Value::String(text)) => Ok(Some(text.clone())),
                        Some(_) => Err(Failure::invalid(format!("{name} must be text or null"))),
                    }
                };
                let status_clear_at = match given.get("statusClearAt") {
                    None => current.status_clear_at,
                    Some(Value::Null) => None,
                    Some(_) => Some(datetime_arg(args, "statusClearAt")?),
                };
                let updated = penfii_core::identity::update_public_profile(
                    pool,
                    user,
                    penfii_core::identity::PublicProfile {
                        display_name: string_arg(args, "displayName")
                            .map(str::to_string)
                            .unwrap_or(current.display_name),
                        job_title: pick("jobTitle", current.job_title)?,
                        time_zone: pick("timeZone", current.time_zone)?,
                        status_emoji: pick("statusEmoji", current.status_emoji)?,
                        status_text: pick("statusText", current.status_text)?,
                        status_clear_at,
                    },
                )
                .await?;
                Ok(done(to_value(&updated)?))
            }
            .await
        }
        _ => return None,
    })
}
