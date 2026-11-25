//! The classification of deletes and of access changes rests on what Core really does. These tests
//! pin those facts (so a change in Core that makes a delete recoverable shows up here) and check
//! the permission matrix the MCP tools inherit: author, Project admin, channel manager, other member,
//! outsider.

use base64::Engine;
use penfii_core::files::ObjectStore;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{
    Env, add_member, modern_call, modern_list, rest, setup_with, token_for, tool_names,
};
use crate::mcp_support::{mcp_err, mcp_ok, set_policy, user_id};

const SCOPES: &str = "workspace:read ticket:read ticket:write doc:read doc:write doc:delete \
    conversation:read conversation:write file:read file:write project:read project:write team:write member:read";

async fn upload(env: &Env, token: &str) -> String {
    let file = mcp_ok(
        env,
        token,
        "file_create_small",
        json!({"workspaceId": env.workspace_id, "filename": "evidence.txt",
               "contentBase64": base64::engine::general_purpose::STANDARD.encode(b"evidence")}),
    )
    .await;
    file["id"].as_str().unwrap().to_string()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn comment_deletes_are_irreversible_and_message_deletes_keep_history(pool: PgPool) {
    let env = setup_with(pool, |config| {
        config.objects = Some(ObjectStore::in_memory())
    })
    .await;
    let ws = env.workspace_id;
    let author_cookie = add_member(&env, "author@penfii.test", "MEMBER").await;
    let other_cookie = add_member(&env, "other@penfii.test", "MEMBER").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let author = token_for(&env, &author_cookie, SCOPES).await;
    let other = token_for(&env, &other_cookie, SCOPES).await;
    // A Project the author edits and the other member only views; the owner administers it.
    let project = mcp_ok(
        &env,
        &owner,
        "project_create",
        json!({"workspaceId": ws, "name": "Risk"}),
    )
    .await;
    let pid = project["id"].as_str().unwrap().to_string();
    for (who, role) in [
        ("author@penfii.test", "EDITOR"),
        ("other@penfii.test", "VIEWER"),
    ] {
        let put = rest(
            &env,
            "PUT",
            &format!("/v1/projects/{pid}/grants/{}", user_id(&env, who).await),
            &env.owner_cookie,
            None,
            Some(json!({"role": role})),
        )
        .await;
        assert!(put.status.is_success(), "{:?}", put.body);
    }
    let ticket = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": pid, "title": "T"}),
    )
    .await;
    let tid = ticket["id"].as_str().unwrap().to_string();

    // ---- Ticket comment: text erased, files removed, no copy kept.
    let file = upload(&env, &author).await;
    let posted = rest(
        &env,
        "POST",
        &format!("/v1/tickets/{tid}/comments"),
        &author_cookie,
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"body": {"type": "doc", "content": [{"type": "paragraph",
                    "content": [{"type": "text", "text": "secret evidence"}]}]}, "fileIds": [file]})),
    )
    .await;
    assert!(posted.status.is_success(), "{:?}", posted.body);
    let comment_id = posted.body["id"].as_str().unwrap().to_string();
    let args = json!({"ticketId": tid, "commentId": comment_id});
    // Destructive: off under the standard policy, and absent from the tool list.
    assert_eq!(
        mcp_err(&env, &author, "ticket_comment_delete", args.clone()).await,
        "policy_denied"
    );
    assert!(
        !tool_names(&modern_list(&env.app, &author).await)
            .contains(&"ticket_comment_delete".to_string())
    );
    set_policy(&env, true, true, false).await;
    // Core's rule: the author or a Project admin; an editor/viewer who is neither may not.
    assert_eq!(
        mcp_err(&env, &other, "ticket_comment_delete", args.clone()).await,
        "forbidden"
    );
    let gone = mcp_ok(&env, &author, "ticket_comment_delete", args.clone()).await;
    assert_eq!(gone["attachmentsRemoved"], 1, "{gone}");
    let (body, deleted): (Value, bool) = sqlx::query_as(
        "SELECT body, deleted_at IS NOT NULL FROM penfii.ticket_comments WHERE id = $1",
    )
    .bind(Uuid::parse_str(&comment_id).unwrap())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(deleted);
    assert_eq!(body, json!({}), "the text is overwritten, not archived");
    let attached: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.ticket_attachments WHERE comment_id = $1")
            .bind(Uuid::parse_str(&comment_id).unwrap())
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        attached, 0,
        "the attachment row is deleted with the comment"
    );
    let copies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.ticket_revisions WHERE snapshot::text LIKE '%secret evidence%'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(copies, 0, "no revision keeps the comment text");
    let elsewhere: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM penfii.domain_events WHERE facts::text LIKE '%secret evidence%')
              + (SELECT count(*) FROM penfii.activity_entries WHERE data::text LIKE '%secret evidence%')",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        elsewhere, 0,
        "neither events nor activity keep the comment text"
    );
    let again = mcp_err(&env, &author, "ticket_comment_delete", args).await;
    assert!(
        ["not_found", "forbidden"].contains(&again.as_str()),
        "{again}"
    );
    // A Project admin may remove someone else's comment (web rule); the viewer may not.
    let second = mcp_ok(
        &env,
        &author,
        "ticket_comment",
        json!({"ticketId": tid, "body": "second"}),
    )
    .await;
    let second_args = json!({"ticketId": tid, "commentId": second["id"]});
    assert_eq!(
        mcp_err(&env, &other, "ticket_comment_delete", second_args.clone()).await,
        "forbidden"
    );
    mcp_ok(&env, &owner, "ticket_comment_delete", second_args).await;

    // ---- Doc comment: the files are removed for good.
    let doc = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "D", "markdown": "x"}),
    )
    .await;
    let did = doc["id"].as_str().unwrap().to_string();
    let dfile = upload(&env, &author).await;
    let dc = rest(
        &env,
        "POST",
        &format!("/v1/docs/{did}/comments"),
        &author_cookie,
        None,
        Some(
            json!({"body": {"type": "doc", "content": [{"type": "paragraph",
                    "content": [{"type": "text", "text": "doc evidence"}]}]}, "fileIds": [dfile]}),
        ),
    )
    .await;
    assert!(dc.status.is_success(), "{:?}", dc.body);
    let dcid = dc.body["id"].as_str().unwrap().to_string();
    let dargs = json!({"docId": did, "commentId": dcid});
    assert_eq!(
        mcp_err(&env, &other, "doc_comment_delete", dargs.clone()).await,
        "forbidden"
    );
    let removed = mcp_ok(&env, &author, "doc_comment_delete", dargs).await;
    assert_eq!(removed["attachmentsRemoved"], 1, "{removed}");
    let files_left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.doc_comment_files WHERE comment_id = $1")
            .bind(Uuid::parse_str(&dcid).unwrap())
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(files_left, 0);
    let listed = mcp_ok(&env, &owner, "doc_comment_list", json!({"docId": did})).await;
    assert!(
        !listed.to_string().contains("doc evidence"),
        "a deleted comment is not served again"
    );

    // ---- Message delete: an earlier version is kept, the call is idempotent, files stay attached.
    set_policy(&env, true, false, false).await;
    let channel = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "risk", "visibility": "PUBLIC"}),
    )
    .await;
    let cid = channel["id"].as_str().unwrap().to_string();
    let msg = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": cid, "body": "keep this text"}),
    )
    .await;
    let mid = msg["id"].as_str().unwrap().to_string();
    // Write: allowed under the standard policy (destructive is off here).
    mcp_ok(&env, &owner, "message_delete", json!({"messageId": mid})).await;
    let kept: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.message_versions WHERE message_id = $1 AND body::text LIKE '%keep this text%'",
    )
    .bind(Uuid::parse_str(&mid).unwrap())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(kept, 1, "the deleted message's text stays in its history");
    let history = mcp_ok(&env, &owner, "message_history", json!({"messageId": mid})).await;
    assert!(
        history.to_string().contains("keep this text"),
        "the author can still read the history"
    );
    let repeated = modern_call(
        &env.app,
        &owner,
        "message_delete",
        json!({"messageId": mid}),
    )
    .await;
    assert_eq!(
        repeated.body["result"]["isError"], false,
        "deleting twice is harmless"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn doc_access_changes_follow_core_manage_rights_and_report_what_widened(pool: PgPool) {
    let env = setup_with(pool, |_| {}).await;
    let ws = env.workspace_id;
    let mia_cookie = add_member(&env, "mia@penfii.test", "MEMBER").await;
    let ben_cookie = add_member(&env, "ben@penfii.test", "MEMBER").await;
    let admin_cookie = add_member(&env, "adm@penfii.test", "ADMIN").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let mia = token_for(&env, &mia_cookie, SCOPES).await;
    let ben = token_for(&env, &ben_cookie, SCOPES).await;
    let admin = token_for(&env, &admin_cookie, SCOPES).await;
    let (mia_id, ben_id) = (
        user_id(&env, "mia@penfii.test").await,
        user_id(&env, "ben@penfii.test").await,
    );

    // Mia makes a RESTRICTED page with herself as manager; Ben cannot see it.
    let doc = mcp_ok(
        &env,
        &mia,
        "doc_create",
        json!({"workspaceId": ws, "title": "Private plans", "visibility": "RESTRICTED",
               "participants": [{"userId": mia_id, "role": "MANAGER"}], "markdown": "launch date"}),
    )
    .await;
    let did = doc["id"].as_str().unwrap().to_string();
    assert_eq!(
        mcp_err(&env, &ben, "doc_get", json!({"docId": did})).await,
        "not_found"
    );
    // Neither a Workspace admin, the owner nor a non-manager can change the audience (Core: manage).
    for who in [&ben, &admin, &owner] {
        for (tool, args) in [
            (
                "doc_set_visibility",
                json!({"docId": did, "visibility": "WORKSPACE"}),
            ),
            (
                "doc_set_participants",
                json!({"docId": did, "participants": [{"userId": ben_id, "role": "MANAGER"}]}),
            ),
        ] {
            let code = mcp_err(&env, who, tool, args).await;
            assert!(
                ["not_found", "forbidden"].contains(&code.as_str()),
                "{tool}: {code}"
            );
        }
    }
    assert_eq!(
        mcp_err(&env, &ben, "doc_get", json!({"docId": did})).await,
        "not_found",
        "nothing leaked"
    );

    // The manager adds Ben as a viewer: the answer lists the change; Ben reads, but cannot widen.
    let shared = mcp_ok(
        &env,
        &mia,
        "doc_set_participants",
        json!({"docId": did, "participants": [{"userId": mia_id, "role": "MANAGER"}, {"userId": ben_id, "role": "VIEWER"}]}),
    )
    .await;
    assert_eq!(
        shared["change"]["added"],
        json!([ben_id.to_string()]),
        "{shared}"
    );
    assert_eq!(shared["change"]["removed"], json!([]));
    mcp_ok(&env, &ben, "doc_get", json!({"docId": did})).await;
    assert_eq!(
        mcp_err(
            &env,
            &ben,
            "doc_set_visibility",
            json!({"docId": did, "visibility": "WORKSPACE"})
        )
        .await,
        "forbidden"
    );
    let promote = mcp_err(
        &env,
        &ben,
        "doc_set_participants",
        json!({"docId": did, "participants": [{"userId": ben_id, "role": "MANAGER"}]}),
    )
    .await;
    assert_eq!(
        promote, "forbidden",
        "a viewer cannot make themselves manager"
    );

    // Widening is Write (the web app does it from the same dialog) but says so loudly.
    let widened = mcp_ok(
        &env,
        &mia,
        "doc_set_visibility",
        json!({"docId": did, "visibility": "WORKSPACE"}),
    )
    .await;
    assert_eq!(widened["change"]["widened"], true, "{widened}");
    assert_eq!(widened["change"]["from"]["visibility"], "RESTRICTED");
    assert_eq!(widened["change"]["to"]["visibility"], "WORKSPACE");
    let narrowed = mcp_ok(
        &env,
        &mia,
        "doc_set_visibility",
        json!({"docId": did, "visibility": "RESTRICTED"}),
    )
    .await;
    assert_eq!(narrowed["change"]["widened"], false);
    // Allowed under the standard policy: Write, not Administrative.
    let policy_names = tool_names(&modern_list(&env.app, &mia).await);
    assert!(
        policy_names.contains(&"doc_set_visibility".to_string())
            && policy_names.contains(&"doc_set_participants".to_string())
    );
    // The audience change cannot reach outside the Workspace: another Workspace's person is refused.
    let stranger = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM penfii.users WHERE email_key = 'outsider@penfii.test'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    let foreign = mcp_err(
        &env,
        &mia,
        "doc_set_participants",
        json!({"docId": did, "participants": [{"userId": mia_id, "role": "MANAGER"}, {"userId": stranger, "role": "VIEWER"}]}),
    )
    .await;
    assert!(
        ["invalid_arguments", "not_found", "forbidden"].contains(&foreign.as_str()),
        "{foreign}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn team_and_channel_member_tools_stay_write_and_keep_core_rules(pool: PgPool) {
    let env = setup_with(pool, |_| {}).await;
    let ws = env.workspace_id;
    let mia_cookie = add_member(&env, "mia@penfii.test", "MEMBER").await;
    let ben_cookie = add_member(&env, "ben@penfii.test", "MEMBER").await;
    // One consent per person: a new one replaces the grant and ends the earlier token.
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let mia = token_for(&env, &mia_cookie, SCOPES).await;
    let ben = token_for(&env, &ben_cookie, SCOPES).await;
    let (mia_id, ben_id) = (
        user_id(&env, "mia@penfii.test").await,
        user_id(&env, "ben@penfii.test").await,
    );
    let names = tool_names(&modern_list(&env.app, &owner).await);
    for tool in [
        "team_member_set",
        "channel_member_add",
        "channel_member_set_role",
        "channel_member_remove",
        "message_delete",
        "project_create",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "{tool} is listed under the standard policy"
        );
    }
    assert!(
        !names.contains(&"channel_set_visibility".to_string()),
        "making a private channel public stays Administrative"
    );

    // Team: a manager adds a member; Core audits it; a plain member may not.
    let team = mcp_ok(
        &env,
        &owner,
        "team_create",
        json!({"workspaceId": ws, "name": "Core", "members": [{"userId": env.owner_id, "role": "MANAGER"}]}),
    )
    .await;
    let team_id = team["id"].as_str().unwrap().to_string();
    let denied = mcp_err(
        &env,
        &ben,
        "team_member_set",
        json!({"workspaceId": ws, "teamId": team_id, "userId": ben_id, "role": "MEMBER"}),
    )
    .await;
    assert!(
        ["forbidden", "not_found"].contains(&denied.as_str()),
        "{denied}"
    );
    mcp_ok(
        &env,
        &owner,
        "team_member_set",
        json!({"workspaceId": ws, "teamId": team_id, "userId": mia_id, "role": "MEMBER"}),
    )
    .await;
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'team.member_added' AND workspace_id = $1")
        .bind(ws)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        audited, 1,
        "Core writes the Audit entry for the Team change"
    );

    // Channel: a private channel keeps its rules; a non-member cannot add themselves.
    // Core: only a Workspace owner or admin creates a channel; the owner makes Mia its manager.
    let channel = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "quiet", "visibility": "PRIVATE"}),
    )
    .await;
    let cid = channel["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &owner,
        "channel_member_add",
        json!({"channelId": cid, "userId": mia_id}),
    )
    .await;
    mcp_ok(
        &env,
        &owner,
        "channel_member_set_role",
        json!({"channelId": cid, "userId": mia_id, "role": "MANAGER"}),
    )
    .await;
    assert_eq!(
        mcp_err(
            &env,
            &mia,
            "channel_create",
            json!({"workspaceId": ws, "name": "mine", "visibility": "PRIVATE"})
        )
        .await,
        "forbidden",
        "a plain member may not create a channel, through MCP as on the web"
    );
    let selfadd = mcp_err(
        &env,
        &ben,
        "channel_member_add",
        json!({"channelId": cid, "userId": ben_id}),
    )
    .await;
    assert!(
        ["forbidden", "not_found"].contains(&selfadd.as_str()),
        "{selfadd}"
    );
    let public = mcp_err(
        &env,
        &ben,
        "channel_set_visibility",
        json!({"channelId": cid, "visibility": "PUBLIC"}),
    )
    .await;
    assert!(
        ["policy_denied", "not_found", "forbidden"].contains(&public.as_str()),
        "{public}"
    );
    mcp_ok(
        &env,
        &mia,
        "channel_member_add",
        json!({"channelId": cid, "userId": ben_id}),
    )
    .await;
    mcp_ok(&env, &ben, "message_list", json!({"conversationId": cid})).await;
    mcp_ok(
        &env,
        &mia,
        "channel_member_remove",
        json!({"channelId": cid, "userId": ben_id}),
    )
    .await;
    let after = mcp_err(&env, &ben, "message_list", json!({"conversationId": cid})).await;
    assert!(
        ["forbidden", "not_found"].contains(&after.as_str()),
        "removed from the channel: {after}"
    );
}
