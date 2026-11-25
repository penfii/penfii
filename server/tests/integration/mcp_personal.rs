//! MCP personal-work operations (Inbox, Later, favorites, drafts, subscriptions, settings, profile).
//! Helpers: `crate::mcp_oauth`.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::Reply;
use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, modern_list, rest, setup, structured, token_for,
    tool_names,
};
use crate::mcp_support::set_policy;

const SCOPES: &str =
    "workspace:read personal:read personal:write ticket:read ticket:write search:read";

fn ok(reply: &Reply) -> &Value {
    assert_eq!(reply.body["result"]["isError"], false, "{:?}", reply.body);
    structured(reply)
}

async fn call(env: &Env, token: &str, tool: &str, args: Value) -> Reply {
    modern_call(&env.app, token, tool, args).await
}

async fn user_of(env: &Env, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM penfii.users WHERE email = $1")
        .bind(email)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

async fn ticket(env: &Env, token: &str) -> String {
    let project = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"name": "Personal", "teamIds": []})),
    )
    .await;
    let created = call(
        env,
        token,
        "ticket_create",
        json!({"projectId": project.body["id"], "title": "Remember me"}),
    )
    .await;
    ok(&created)["id"].as_str().unwrap().to_string()
}

/// A public channel with the owner and a member in it, and one message mentioning the member:
/// the member then has an Inbox item.
async fn channel_with_mention(env: &Env, member_cookie: &str, member: Uuid) -> String {
    let channel = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/channels", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": "general", "visibility": "PUBLIC", "teamId": null, "projectId": null})),
    )
    .await;
    let channel_id = channel.body["id"].as_str().unwrap().to_string();
    let join = rest(
        env,
        "POST",
        &format!("/v1/channels/{channel_id}/join"),
        member_cookie,
        None,
        None,
    )
    .await;
    assert!(join.status.is_success(), "{:?}", join.body);
    let body = json!({"type":"doc","content":[{"type":"paragraph","content":[
        {"type":"mention","attrs":{"occurrenceId":Uuid::new_v4(),"kind":"USER","userId":member}},
        {"type":"text","text":" please look"}]}]});
    let sent = rest(
        env,
        "POST",
        &format!("/v1/conversations/{channel_id}/messages"),
        &env.owner_cookie,
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"bodySchemaVersion":1,"body":body,"threadRootId":null,"replyToId":null})),
    )
    .await;
    assert!(sent.status.is_success(), "{:?}", sent.body);
    let mut registry = penfii_core::consumers::registry(None, None);
    registry.workspace = Some(env.workspace_id);
    registry.backoff_base_secs = 0;
    registry.max_backoff_secs = 0;
    penfii_core::events::run_until_idle(&env.pool, &registry)
        .await
        .unwrap();
    channel_id
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn inbox_reads_never_mark_read_and_marking_is_explicit_and_private(pool: PgPool) {
    let env = setup(pool).await;
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let member_id = user_of(&env, "member@penfii.test").await;
    channel_with_mention(&env, &member_cookie, member_id).await;
    let member = token_for(&env, &member_cookie, SCOPES).await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let path = format!("/v1/workspaces/{}/inbox", env.workspace_id);

    let listed = call(
        &env,
        &member,
        "inbox_list",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    let page = ok(&listed).clone();
    assert_eq!(page["unreadCount"], 1, "{page}");
    let item = &page["items"][0];
    let (item_id, version) = (
        item["id"].as_str().unwrap().to_string(),
        item["version"].as_i64().unwrap(),
    );
    // The same page the web route returns.
    let web = rest(&env, "GET", &path, &member_cookie, None, None).await;
    assert_eq!(page, web.body);
    // Listing (twice, and filtered) changed nothing.
    for args in [
        json!({"workspaceId": env.workspace_id}),
        json!({"workspaceId": env.workspace_id, "state": "unread", "limit": 5}),
    ] {
        ok(&call(&env, &member, "inbox_list", args).await);
    }
    let (state, v): (String, i64) =
        sqlx::query_as("SELECT state, version FROM penfii.inbox_items WHERE id = $1")
            .bind(Uuid::parse_str(&item_id).unwrap())
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!((state.as_str(), v), ("UNREAD", version));

    // Someone else's item is not found, and the owner's own Inbox is empty.
    let foreign = call(
        &env,
        &owner,
        "inbox_update",
        json!({"inboxItemId": item_id, "command": "read", "expectedVersion": version}),
    )
    .await;
    assert_eq!(error_code(&foreign), "not_found");
    let own = call(
        &env,
        &owner,
        "inbox_list",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert!(ok(&own)["items"].as_array().unwrap().is_empty());

    // Marking read is a separate, explicit write; a stale version is refused.
    let stale = call(
        &env,
        &member,
        "inbox_update",
        json!({"inboxItemId": item_id, "command": "read", "expectedVersion": version + 5}),
    )
    .await;
    assert_eq!(stale.body["result"]["isError"], true);
    let read = call(
        &env,
        &member,
        "inbox_update",
        json!({"inboxItemId": item_id, "command": "read", "expectedVersion": version}),
    )
    .await;
    assert_eq!(ok(&read)["state"], "READ");
    let unread = call(
        &env,
        &member,
        "inbox_update",
        json!({"inboxItemId": item_id, "command": "unread", "expectedVersion": version + 1}),
    )
    .await;
    assert_eq!(ok(&unread)["state"], "UNREAD");
    let no_until = call(
        &env,
        &member,
        "inbox_update",
        json!({"inboxItemId": item_id, "command": "snooze", "expectedVersion": version + 2}),
    )
    .await;
    assert_eq!(error_code(&no_until), "invalid_arguments");
    let all = call(
        &env,
        &member,
        "inbox_read_all",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(ok(&all)["changed"], 1);
    let after = call(
        &env,
        &member,
        "inbox_list",
        json!({"workspaceId": env.workspace_id}),
    )
    .await;
    assert_eq!(ok(&after)["unreadCount"], 0);
    let archived = call(&env, &member, "inbox_update", json!({"inboxItemId": item_id, "command": "archive", "expectedVersion": ok(&after)["items"][0]["version"]})).await;
    assert_eq!(ok(&archived)["state"], "ARCHIVED");
    let in_archive = call(
        &env,
        &member,
        "inbox_list",
        json!({"workspaceId": env.workspace_id, "view": "archived"}),
    )
    .await;
    assert_eq!(ok(&in_archive)["items"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn later_favorites_folders_and_drafts_belong_to_one_person(pool: PgPool) {
    let env = setup(pool).await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let ticket_id = ticket(&env, &owner).await;
    let ws = env.workspace_id;

    // Later: list equals the web list; versioned update; reminder; remove.
    let added = call(
        &env,
        &owner,
        "later_add",
        json!({"workspaceId": ws, "kind": "TICKET", "targetId": ticket_id, "note": "soon"}),
    )
    .await;
    let later = ok(&added).clone();
    let later_id = later["id"].as_str().unwrap().to_string();
    let listed = call(&env, &owner, "later_list", json!({"workspaceId": ws})).await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/later"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&listed)["items"], web.body);
    let stale = call(
        &env,
        &owner,
        "later_update",
        json!({"laterId": later_id, "expectedVersion": 99, "state": "DONE"}),
    )
    .await;
    assert_eq!(stale.body["result"]["isError"], true);
    let due = (chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339();
    let reminder = call(
        &env,
        &owner,
        "later_reminder_set",
        json!({"laterId": later_id, "dueAt": due, "timeZone": "Asia/Seoul"}),
    )
    .await;
    assert_eq!(
        reminder.body["result"]["isError"], false,
        "{:?}",
        reminder.body
    );
    let cancel = call(
        &env,
        &owner,
        "later_reminder_cancel",
        json!({"laterId": later_id}),
    )
    .await;
    ok(&cancel);
    let current = call(&env, &owner, "later_list", json!({"workspaceId": ws})).await;
    let version = ok(&current)["items"][0]["version"].clone();
    let done = call(
        &env,
        &owner,
        "later_update",
        json!({"laterId": later_id, "expectedVersion": version, "state": "DONE", "note": null}),
    )
    .await;
    assert_eq!(ok(&done)["state"], "DONE");
    assert!(ok(&done)["note"].is_null());

    // Another person cannot touch it.
    let member_cookie = add_member(&env, "member@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, SCOPES).await;
    let foreign = call(
        &env,
        &member,
        "later_update",
        json!({"laterId": later_id, "expectedVersion": 1, "state": "ACTIVE"}),
    )
    .await;
    assert_eq!(error_code(&foreign), "not_found");
    let foreign = call(&env, &member, "later_remove", json!({"laterId": later_id})).await;
    assert_eq!(error_code(&foreign), "not_found");
    let theirs = call(
        &env,
        &member,
        "later_list",
        json!({"workspaceId": ws, "state": "DONE"}),
    )
    .await;
    assert!(ok(&theirs)["items"].as_array().unwrap().is_empty());
    ok(&call(&env, &owner, "later_remove", json!({"laterId": later_id})).await);

    // Favorites and folders.
    ok(&call(
        &env,
        &owner,
        "favorite_add",
        json!({"workspaceId": ws, "kind": "TICKET", "targetId": ticket_id}),
    )
    .await);
    let favorites = call(&env, &owner, "favorite_list", json!({"workspaceId": ws})).await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/favorites"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&favorites)["items"], web.body);
    assert_eq!(web.body.as_array().unwrap().len(), 1);
    let folder = call(
        &env,
        &owner,
        "favorite_folder_create",
        json!({"workspaceId": ws, "name": "Work"}),
    )
    .await;
    let folder_id = ok(&folder)["id"].as_str().unwrap().to_string();
    ok(&call(
        &env,
        &owner,
        "favorite_folder_rename",
        json!({"folderId": folder_id, "name": "Work 2"}),
    )
    .await);
    ok(&call(
        &env,
        &owner,
        "favorite_move",
        json!({"workspaceId": ws, "kind": "TICKET", "targetId": ticket_id, "folderId": folder_id}),
    )
    .await);
    ok(&call(
        &env,
        &owner,
        "favorite_order",
        json!({"workspaceId": ws, "items": [{"kind": "TICKET", "targetId": ticket_id}]}),
    )
    .await);
    let folders = call(
        &env,
        &owner,
        "favorite_folder_list",
        json!({"workspaceId": ws}),
    )
    .await;
    assert_eq!(ok(&folders)["items"].as_array().unwrap().len(), 1);
    ok(&call(
        &env,
        &owner,
        "favorite_folder_order",
        json!({"workspaceId": ws, "folderIds": [folder_id]}),
    )
    .await);
    let foreign = call(
        &env,
        &member,
        "favorite_folder_delete",
        json!({"folderId": folder_id}),
    )
    .await;
    assert_eq!(error_code(&foreign), "not_found");
    let deleted = call(
        &env,
        &owner,
        "favorite_folder_delete",
        json!({"folderId": folder_id}),
    )
    .await;
    assert_eq!(ok(&deleted)["movedToTopLevel"], 1);
    ok(&call(
        &env,
        &owner,
        "favorite_move",
        json!({"workspaceId": ws, "kind": "TICKET", "targetId": ticket_id, "folderId": null}),
    )
    .await);
    ok(&call(
        &env,
        &owner,
        "favorite_remove",
        json!({"workspaceId": ws, "kind": "TICKET", "targetId": ticket_id}),
    )
    .await);

    // Drafts: private to their author, versioned, closed sessions refused.
    let member_id = user_of(&env, "member@penfii.test").await;
    let channel_id = channel_with_mention(&env, &member_cookie, member_id).await;
    let body = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"half a thought"}]}]});
    let gen1 = Uuid::from_u128(1 << 100);
    let saved = call(&env, &member, "draft_save", json!({"contextKind": "CHANNEL_COMPOSE", "contextId": channel_id, "generation": gen1, "body": body})).await;
    let draft = ok(&saved).clone();
    assert_eq!(draft["created"], true);
    let draft_id = draft["id"].as_str().unwrap().to_string();
    let conflict = call(&env, &member, "draft_save", json!({"contextKind": "CHANNEL_COMPOSE", "contextId": channel_id, "generation": gen1, "expectedVersion": 42, "body": body})).await;
    assert_eq!(error_code(&conflict), "draft_conflict");
    let empty = call(&env, &member, "draft_save", json!({"contextKind": "CHANNEL_COMPOSE", "contextId": channel_id, "generation": gen1, "body": {"type":"doc","content":[]}})).await;
    assert_eq!(error_code(&empty), "invalid_arguments");
    let mine = call(&env, &member, "draft_list", json!({"workspaceId": ws})).await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/drafts"),
        &member_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&mine)["items"], web.body);
    assert_eq!(web.body.as_array().unwrap().len(), 1);
    let owners = call(&env, &owner, "draft_list", json!({"workspaceId": ws})).await;
    assert!(ok(&owners)["items"].as_array().unwrap().is_empty());
    let foreign = call(&env, &owner, "draft_delete", json!({"draftId": draft_id})).await;
    assert_eq!(error_code(&foreign), "not_found");
    ok(&call(&env, &member, "draft_delete", json!({"draftId": draft_id})).await);
    let reused = call(&env, &member, "draft_save", json!({"contextKind": "CHANNEL_COMPOSE", "contextId": channel_id, "generation": gen1, "body": body})).await;
    assert_eq!(
        error_code(&reused),
        "draft_closed",
        "a discarded session cannot be revived"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn subscriptions_mutes_settings_and_profile(pool: PgPool) {
    let env = setup(pool).await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let ticket_id = ticket(&env, &owner).await;
    let ws = env.workspace_id;

    let before = call(
        &env,
        &owner,
        "subscription_get",
        json!({"kind": "TICKET", "targetId": ticket_id}),
    )
    .await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{ticket_id}/subscription"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&before), &web.body);
    let off = call(
        &env,
        &owner,
        "subscription_set",
        json!({"kind": "TICKET", "targetId": ticket_id, "watching": false}),
    )
    .await;
    assert_eq!(ok(&off)["watching"], false);
    let on = call(
        &env,
        &owner,
        "subscription_set",
        json!({"kind": "TICKET", "targetId": ticket_id, "watching": true}),
    )
    .await;
    assert_eq!(ok(&on)["watching"], true);
    let list = call(
        &env,
        &owner,
        "subscription_list",
        json!({"workspaceId": ws, "kind": "TICKET"}),
    )
    .await;
    assert_eq!(ok(&list)["items"].as_array().unwrap().len(), 1);
    // The same id under the wrong kind is not found, and another Workspace's person gets nothing.
    let wrong = call(
        &env,
        &owner,
        "subscription_get",
        json!({"kind": "PROJECT", "targetId": ticket_id}),
    )
    .await;
    assert_eq!(wrong.body["result"]["isError"], true);
    let outsider = token_for(&env, &env.outsider_cookie, SCOPES).await;
    let out = call(
        &env,
        &outsider,
        "subscription_set",
        json!({"kind": "TICKET", "targetId": ticket_id, "watching": true}),
    )
    .await;
    assert_eq!(out.body["result"]["isError"], true);

    // Channel mute.
    let channel = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/channels"),
        &env.owner_cookie,
        None,
        Some(json!({"name": "quiet", "visibility": "PUBLIC", "teamId": null, "projectId": null})),
    )
    .await;
    let channel_id = channel.body["id"].as_str().unwrap().to_string();
    ok(&call(
        &env,
        &owner,
        "channel_mute_set",
        json!({"channelId": channel_id, "muted": true}),
    )
    .await);
    let muted = call(&env, &owner, "channel_mute_list", json!({})).await;
    let web = rest(
        &env,
        "GET",
        "/v1/me/muted-channels",
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&muted), &web.body);
    assert_eq!(web.body["channelIds"].as_array().unwrap().len(), 1);
    ok(&call(
        &env,
        &owner,
        "channel_mute_set",
        json!({"channelId": channel_id, "muted": false}),
    )
    .await);

    // Notification settings: versioned replace, same as the web route.
    let prefs = call(&env, &owner, "notification_preferences_get", json!({})).await;
    let web = rest(
        &env,
        "GET",
        "/v1/me/notification-preferences",
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(ok(&prefs), &web.body);
    let version = ok(&prefs)["version"].as_i64().unwrap();
    let saved = call(
        &env,
        &owner,
        "notification_preferences_set",
        json!({
        "expectedVersion": version, "level": "DIRECT_ONLY", "dnd": {"enabled": false}}),
    )
    .await;
    assert_eq!(ok(&saved)["level"], "DIRECT_ONLY");
    let stale = call(
        &env,
        &owner,
        "notification_preferences_set",
        json!({
        "expectedVersion": version, "level": "ALL", "dnd": {"enabled": false}}),
    )
    .await;
    assert_eq!(stale.body["result"]["isError"], true);

    // Profile: reads what the web reads; a partial update keeps the other fields.
    let me = call(&env, &owner, "me_get", json!({})).await;
    let web = rest(&env, "GET", "/v1/me", &env.owner_cookie, None, None).await;
    assert_eq!(ok(&me), &web.body);
    let first = call(
        &env,
        &owner,
        "me_update_profile",
        json!({"jobTitle": "Lead", "statusText": "busy"}),
    )
    .await;
    assert_eq!(ok(&first)["jobTitle"], "Lead");
    let second = call(
        &env,
        &owner,
        "me_update_profile",
        json!({"displayName": "Boss"}),
    )
    .await;
    let updated = ok(&second);
    assert_eq!(
        (
            updated["displayName"].as_str(),
            updated["jobTitle"].as_str(),
            updated["statusText"].as_str()
        ),
        (Some("Boss"), Some("Lead"), Some("busy"))
    );
    assert_eq!(
        updated["email"],
        ok(&me)["email"],
        "the e-mail is never changed here"
    );
    let cleared = call(
        &env,
        &owner,
        "me_update_profile",
        json!({"statusText": null, "statusEmoji": null}),
    )
    .await;
    assert!(ok(&cleared)["statusText"].is_null());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn scopes_policy_and_the_web_only_boundary(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id;
    let reader = token_for(&env, &env.owner_cookie, "workspace:read personal:read").await;
    let names = tool_names(&modern_list(&env.app, &reader).await);
    for tool in [
        "inbox_list",
        "later_list",
        "favorite_list",
        "draft_list",
        "subscription_get",
        "me_get",
    ] {
        assert!(names.contains(&tool.to_string()), "{tool}");
    }
    for tool in [
        "inbox_update",
        "inbox_read_all",
        "later_add",
        "me_update_profile",
        "draft_save",
        "channel_mute_set",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "{tool} is discoverable before personal:write is granted"
        );
    }
    let refused = call(&env, &reader, "inbox_read_all", json!({"workspaceId": ws})).await;
    assert_eq!(error_code(&refused), "insufficient_scope");
    assert_eq!(refused.status.as_u16(), 403);

    // Nothing for account security or devices is exposed under any scope.
    let all = token_for(&env, &env.owner_cookie, SCOPES).await;
    let listed = modern_list(&env.app, &all).await;
    for tool in listed.body["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        for banned in ["password", "mfa", "push", "session", "oauth", "identity"] {
            assert!(!name.contains(banned), "{name} touches {banned}");
        }
        // Recovering something is web-only; MCP only lists what a person would recover there.
        if name.contains("recovery") {
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
        }
    }

    // The Workspace switch: with writes off, every personal write is refused and hidden, reads stay.
    set_policy(&env, false, false, false).await;
    let blocked = call(&env, &all, "inbox_read_all", json!({"workspaceId": ws})).await;
    assert_eq!(error_code(&blocked), "policy_denied");
    let profile = call(&env, &all, "me_update_profile", json!({"jobTitle": "x"})).await;
    assert_eq!(error_code(&profile), "policy_denied", "{:?}", profile.body);
    ok(&call(&env, &all, "me_get", json!({})).await);
    let names = tool_names(&modern_list(&env.app, &all).await);
    assert!(!names.contains(&"inbox_read_all".to_string()));
    assert!(names.contains(&"inbox_list".to_string()));

    // A person outside the Workspace reads nothing of it.
    set_policy(&env, true, false, false).await;
    let outsider = token_for(&env, &env.outsider_cookie, SCOPES).await;
    let out = call(&env, &outsider, "inbox_list", json!({"workspaceId": ws})).await;
    assert_eq!(out.body["result"]["isError"], true);
    let out = call(&env, &outsider, "favorite_list", json!({"workspaceId": ws})).await;
    assert_eq!(out.body["result"]["isError"], true);
    // And the calls are logged with the Workspace they targeted.
    let logged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.mcp_call_log WHERE operation = 'inbox.read_all' AND workspace_id = $1",
    )
    .bind(ws)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(logged >= 1);
}
