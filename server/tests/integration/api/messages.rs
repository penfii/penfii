//! Messages, conversations, Channels, direct messages and read state through the API.

use super::{World, call, channel, invite_member, send, workspace};
use crate::common::doc;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn count_messages(pool: &PgPool, conversation_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM penfii.messages WHERE conversation_id = $1")
        .bind(conversation_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The client turns a file over the limit away before sending it, so the server tells it the limit.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn server_info_reports_the_file_size_limit(pool: PgPool) {
    let world = &World::new(pool).await;
    let (status, body, _) = call(&world.app, "GET", "/v1/server-info", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["limits"]["fileMaxBytes"], json!(10 * 1024 * 1024));
    assert_eq!(body["limits"]["messageBodyMaxBytes"], json!(65536));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn idempotency_key_replays_and_conflicts(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("idem-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "general", "PUBLIC").await;
    let key = Uuid::new_v4().to_string();
    let payload = json!({
        "bodySchemaVersion": 1,
        "body": doc("same"),
        "threadRootId": null,
        "replyToId": null
    });
    let (status, first, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        Some(&key),
        Some(payload.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let (status, second, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        Some(&key),
        Some(payload),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_eq!(first["id"], second["id"]);
    assert_eq!(count_messages(&world.pool, conversation_id).await, 1);

    let (status, conflict, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        Some(&key),
        Some(json!({
            "bodySchemaVersion": 1,
            "body": doc("different"),
            "threadRootId": null,
            "replyToId": null
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "idempotency_conflict");
    assert_eq!(count_messages(&world.pool, conversation_id).await, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn replies_stay_flat_in_one_thread(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("thread-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "threads", "PUBLIC").await;
    let (status, root_a) = send(world, &world.owner_cookie, conversation_id, "A", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{root_a}");
    let a: Uuid = root_a["id"].as_str().unwrap().parse().unwrap();
    let (status, reply_b) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "B",
        Some(a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reply_b}");
    let (status, reply_c) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "C",
        Some(a),
        Some(a),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reply_c}");
    let c: Uuid = reply_c["id"].as_str().unwrap().parse().unwrap();
    let (status, reply_d) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "D",
        Some(a),
        Some(c),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reply_d}");
    assert_eq!(reply_d["threadRootId"].as_str().unwrap(), a.to_string());
    assert_eq!(reply_d["replyToId"].as_str().unwrap(), c.to_string());
    let b: Uuid = reply_b["id"].as_str().unwrap().parse().unwrap();
    let (status, nested) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "nested",
        Some(b),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{nested}");
    assert_eq!(nested["code"], "reply_target_rejected");

    let (status, thread, _) = call(
        &world.app,
        "GET",
        &format!("/v1/messages/{a}/thread"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    let items = thread.as_array().unwrap();
    assert_eq!(items.len(), 4);
    let seqs: Vec<i64> = items
        .iter()
        .map(|item| item["streamSeq"].as_i64().unwrap())
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort();
    assert_eq!(seqs, sorted);
    assert!(items[0]["threadRootId"].is_null());
    for item in items.iter().skip(1) {
        assert_eq!(item["threadRootId"].as_str().unwrap(), a.to_string());
    }

    let (status, other) = send(world, &world.owner_cookie, conversation_id, "E", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let e: Uuid = other["id"].as_str().unwrap().parse().unwrap();
    let (status, rejected) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "cross",
        Some(e),
        Some(c),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
    assert_eq!(rejected["code"], "reply_target_rejected");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn blank_messages_and_edits_are_rejected_by_the_api(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("blank-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "general", "PUBLIC").await;
    let blank = json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": []}]
    });

    for body in [
        blank.clone(),
        json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "  \n\t"}]}]
        }),
    ] {
        let (status, rejected, _) = call(
            &world.app,
            "POST",
            &format!("/v1/conversations/{conversation_id}/messages"),
            Some(&world.owner_cookie),
            Some(&Uuid::new_v4().to_string()),
            Some(json!({
                "bodySchemaVersion": 1,
                "body": body,
                "threadRootId": null,
                "replyToId": null
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        assert_eq!(rejected["code"], "validation");
    }
    assert_eq!(count_messages(&world.pool, conversation_id).await, 0);

    let (status, created) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "keep this body",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let message_id = created["id"].as_str().unwrap();
    let (status, rejected, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/messages/{message_id}"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"expectedVersion": 1, "body": blank})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["code"], "validation");
    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed[0]["body"]["content"][0]["content"][0]["text"],
        "keep this body"
    );
    assert_eq!(listed[0]["version"], 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn draft_file_delete_requires_its_owner_and_is_repeatable(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("file-delete-{}", Uuid::new_v4())).await;
    let (_, member_cookie) = invite_member(world, workspace_id, "MEMBER").await;
    let (status, file, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/files"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"filename": "draft.txt", "byteSize": 16})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{file}");
    let file_id = file["id"].as_str().unwrap();

    let (status, hidden, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/files/{file_id}"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");

    for _ in 0..2 {
        let (status, body, _) = call(
            &world.app,
            "DELETE",
            &format!("/v1/files/{file_id}"),
            Some(&world.owner_cookie),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    let (deleted, generation): (bool, i64) =
        sqlx::query_as("SELECT deleted_at IS NOT NULL, generation FROM penfii.files WHERE id = $1")
            .bind(Uuid::parse_str(file_id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert!(deleted);
    assert_eq!(generation, 2);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn deleted_root_hides_body_and_history(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("delete-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "gone", "PUBLIC").await;
    let (_, member_cookie) = invite_member(world, workspace_id, "MEMBER").await;
    let (status, joined, _) = call(
        &world.app,
        "POST",
        &format!("/v1/channels/{conversation_id}/join"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    let (status, root) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "root-body",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{root}");
    let root_id: Uuid = root["id"].as_str().unwrap().parse().unwrap();
    let (status, reply) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "still-here",
        Some(root_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reply}");

    let (status, deleted, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/messages/{root_id}"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["deleted"], true);
    assert!(deleted["body"].is_null());

    let (status, thread, _) = call(
        &world.app,
        "GET",
        &format!("/v1/messages/{root_id}/thread"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    let items = thread.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["deleted"], true);
    assert!(items[0]["body"].is_null());
    assert_eq!(
        items[1]["body"]["content"][0]["content"][0]["text"],
        "still-here"
    );

    let (status, hidden, _) = call(
        &world.app,
        "GET",
        &format!("/v1/messages/{root_id}/history"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
    assert_eq!(hidden["code"], "not_found");

    let (status, history, _) = call(
        &world.app,
        "GET",
        &format!("/v1/messages/{root_id}/history"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(
        history["versions"][0]["body"]["content"][0]["content"][0]["text"],
        "root-body"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn stale_auto_read_keeps_manual_unread(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("read-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "reads", "PUBLIC").await;
    let (status, _) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "one",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, cursor, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/read"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cursor}");
    assert_eq!(cursor["generation"], 1);

    let (status, marked, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation_id}/manual-unread"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"fromSeq": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{marked}");
    assert_eq!(marked["generation"], 2);
    assert_eq!(marked["manualUnreadFromSeq"], 1);

    let (status, stale, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation_id}/read"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({
            "generation": 1,
            "readThroughSeq": 1,
            "scope": "auto",
            "observed": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "stale_read");
    assert_eq!(stale["manualUnread"], true);

    let (status, after, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/read"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["manualUnreadFromSeq"], 1);
    assert_eq!(after["generation"], 2);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn edit_rejects_stale_version_then_records_history(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("edit-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "edits", "PUBLIC").await;
    let (status, created) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "original",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let message_id = created["id"].as_str().unwrap();

    let (status, conflict, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/messages/{message_id}"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"expectedVersion": 9, "body": doc("nope")})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "version_conflict");
    assert_eq!(conflict["version"], 1);
    assert_eq!(
        conflict["body"]["content"][0]["content"][0]["text"],
        "original"
    );

    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed[0]["body"]["content"][0]["content"][0]["text"],
        "original"
    );
    assert_eq!(listed[0]["version"], 1);

    let (status, edited, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/messages/{message_id}"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"expectedVersion": 1, "body": doc("revised")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["version"], 2);

    let (status, history, _) = call(
        &world.app,
        "GET",
        &format!("/v1/messages/{message_id}/history"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["versions"][0]["version"], 1);
    assert_eq!(
        history["versions"][0]["body"]["content"][0]["content"][0]["text"],
        "original"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn another_member_cannot_edit_or_delete_the_author_message(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("author-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "general", "PUBLIC").await;
    let (_, member_cookie) = invite_member(world, workspace_id, "MEMBER").await;
    let (status, joined, _) = call(
        &world.app,
        "POST",
        &format!("/v1/channels/{conversation_id}/join"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    let (status, created) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "author body",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let message_id = created["id"].as_str().unwrap();

    let (status, rejected, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/messages/{message_id}"),
        Some(&member_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"expectedVersion": 1, "body": doc("changed by another member")})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{rejected}");

    let (status, rejected, _) = call(
        &world.app,
        "DELETE",
        &format!("/v1/messages/{message_id}"),
        Some(&member_cookie),
        Some(&Uuid::new_v4().to_string()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{rejected}");
    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed[0]["body"]["content"][0]["content"][0]["text"],
        "author body"
    );
    assert_eq!(listed[0]["deleted"], false);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn private_channel_is_hidden_from_owner_non_member(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("private-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "secret", "PRIVATE").await;
    let (_, owner_cookie) = invite_member(world, workspace_id, "OWNER").await;

    let (status, hidden, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}"),
        Some(&owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
    assert_eq!(hidden["code"], "not_found");

    let (status, messages, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{messages}");
    assert_eq!(messages["code"], "not_found");

    let (status, visible, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn repeated_dm_create_reuses_the_same_conversation(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("dm-{}", Uuid::new_v4())).await;
    let (member_id, _) = invite_member(world, workspace_id, "MEMBER").await;
    let users = json!({"userIds": [world.owner_id, member_id]});
    let (status, first, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/dms"),
        Some(&world.owner_cookie),
        None,
        Some(users.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let first_id: Uuid = first["id"].as_str().unwrap().parse().unwrap();
    let (status, sent) = send(world, &world.owner_cookie, first_id, "history", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");

    let (status, second, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/dms"),
        Some(&world.owner_cookie),
        None,
        Some(users),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id: Uuid = second["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(first_id, second_id);
    assert_eq!(count_messages(&world.pool, first_id).await, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn private_history_and_reactions_are_not_found(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("react-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "quiet", "PRIVATE").await;
    let (status, created) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "secret",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let message_id = created["id"].as_str().unwrap();
    let (status, reaction, _) = call(
        &world.app,
        "PUT",
        &format!("/v1/messages/{message_id}/reactions"),
        Some(&world.owner_cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({"emoji": "👍"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reaction}");
    assert_eq!(reaction["userIds"].as_array().unwrap().len(), 1);

    let (_, outsider) = invite_member(world, workspace_id, "OWNER").await;
    let (status, history, _) = call(
        &world.app,
        "GET",
        &format!("/v1/messages/{message_id}/history"),
        Some(&outsider),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{history}");
    assert_eq!(history["code"], "not_found");

    let (status, messages, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(&outsider),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{messages}");
    assert_eq!(messages["code"], "not_found");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn project_channel_is_public_to_members_and_empty_dm_stays_addressable(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = workspace(world, &format!("project-chat-{}", Uuid::new_v4())).await;
    let (status, project, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name": "penfii", "teamIds": []})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, space, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/space"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{space}");
    let channel = space["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["projectId"].as_str() == Some(project_id))
        .expect("default channel");
    assert_eq!(channel["name"], "penfii");
    assert_eq!(channel["visibility"], "PUBLIC");
    let channel_id = channel["id"].as_str().unwrap();

    let (member_id, member_cookie) = invite_member(world, workspace_id, "MEMBER").await;
    let (status, hidden, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/space"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hidden}");
    assert!(
        hidden["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != channel_id)
    );
    let (status, missing, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{channel_id}"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    let (status, denied, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&member_cookie),
        None,
        Some(json!({
            "name": "backend",
            "visibility": "PUBLIC",
            "teamId": null,
            "projectId": project_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    sqlx::query(
        "INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, 'EDITOR')",
    )
    .bind(Uuid::parse_str(project_id).unwrap())
    .bind(member_id)
    .execute(&world.pool)
    .await
    .unwrap();
    let (status, readable, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{channel_id}/messages"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{readable}");
    let (status, sent) = send(
        world,
        &member_cookie,
        channel_id.parse().unwrap(),
        "from member",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");

    let (status, updated, _) = call(
        &world.app,
        "PATCH",
        &format!("/v1/channels/{channel_id}"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"visibility": "PRIVATE"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["visibility"], "PRIVATE");
    let (status, closed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{channel_id}"),
        Some(&member_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{closed}");

    let (status, dm, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/dms"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"userIds": [world.owner_id, member_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{dm}");
    assert_eq!(dm["lastMessageSeq"], 0);
    let dm_id = dm["id"].as_str().unwrap();
    let (status, again, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/dms"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"userIds": [member_id, world.owner_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["id"], dm_id);
    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/space"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed["dms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_str() == Some(dm_id))
        .unwrap();
    assert_eq!(row["lastMessageSeq"], 0);
    let (status, _) = send(
        world,
        &world.owner_cookie,
        dm_id.parse().unwrap(),
        "hello",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/space"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed["dms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_str() == Some(dm_id))
        .unwrap();
    assert!(row["lastMessageSeq"].as_i64().unwrap() > 0);
}

// Docs
