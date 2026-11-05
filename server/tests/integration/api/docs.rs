//! Docs (the wiki): the tree, restricted pages, comments, versions and references.

use super::inbox::{attention_now, inbox, kinds, member};
use super::search::SearchWorld;
use super::{World, call, invite_member, workspace};
use crate::common::doc;
use axum::http::StatusCode;
use penfii_core::knowledge::KnowledgeConfig;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub(super) async fn api(
    world: &World,
    method: &str,
    cookie: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, value, _) = call(&world.app, method, path, Some(cookie), None, body).await;
    (status, value)
}

pub(super) fn page_content(text: &str) -> Value {
    json!({"schema":"penfii.doc","version":1,"blocks":[
        {"id":Uuid::new_v4().to_string(),"type":"paragraph","props":{},"content":[{"type":"text","text":text,"styles":{}}],"children":[]}]})
}

pub(super) async fn make_doc(
    world: &World,
    cookie: &str,
    ws: Uuid,
    title: &str,
    extra: Value,
) -> Value {
    let mut body =
        json!({"title": title, "visibility": "WORKSPACE", "content": page_content(title)});
    if let (Some(map), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    let (s, v) = api(
        world,
        "POST",
        cookie,
        &format!("/v1/workspaces/{ws}/docs"),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

pub(super) fn node_ids(tree: &Value) -> Vec<String> {
    tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t44_t46_docs_tree_refuses_cycles_deletes_subtrees_and_templates_are_copies(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-tree-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let a = make_doc(world, &c, ws, "A", json!({})).await;
    let a_id = a["id"].as_str().unwrap().to_owned();
    let b = make_doc(world, &c, ws, "B", json!({"parentId": a_id})).await;
    let b_id = b["id"].as_str().unwrap().to_owned();
    let cc = make_doc(world, &c, ws, "C", json!({"parentId": b_id})).await;
    let c_id = cc["id"].as_str().unwrap().to_owned();

    // Moving A under its own descendant is refused as a rule, and nothing changes.
    for target in [&b_id, &c_id, &a_id] {
        let (s, v) = api(
            world,
            "POST",
            &c,
            &format!("/v1/docs/{a_id}/move"),
            Some(json!({"parentId": target})),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert_eq!(v["code"], "doc_cycle");
    }
    let (_, tree) = api(
        world,
        "GET",
        &c,
        &format!("/v1/workspaces/{ws}/docs/tree"),
        None,
    )
    .await;
    let node = |id: &str| {
        tree["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(node(&a_id)["parentId"], Value::Null);
    assert_eq!(node(&b_id)["parentId"], json!(a_id));
    // A legal move: C goes to the top, before A.
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{c_id}/move"),
        Some(json!({"parentId": null, "beforeId": a_id})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, tree) = api(
        world,
        "GET",
        &c,
        &format!("/v1/workspaces/{ws}/docs/tree"),
        None,
    )
    .await;
    let node = |id: &str| {
        tree["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(node(&c_id)["parentId"], Value::Null);
    assert!(node(&c_id)["position"].as_i64() < node(&a_id)["position"].as_i64());

    // Deleting A removes A and B from the tree at once; C was moved out and stays.
    let (s, _) = api(world, "DELETE", &c, &format!("/v1/docs/{a_id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, tree) = api(
        world,
        "GET",
        &c,
        &format!("/v1/workspaces/{ws}/docs/tree"),
        None,
    )
    .await;
    assert_eq!(node_ids(&tree), vec![c_id.clone()]);
    for gone in [&a_id, &b_id] {
        let (s, _) = api(world, "GET", &c, &format!("/v1/docs/{gone}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
    let deleted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.doc_pages WHERE workspace_id=$1 AND deleted_at IS NOT NULL",
    )
    .bind(ws)
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(deleted, 2, "soft delete keeps the rows");

    // T46: a template is a copy of the content at that moment.
    let (s, tpl) = api(
        world,
        "POST",
        &c,
        &format!("/v1/workspaces/{ws}/doc-templates"),
        Some(json!({"name":"주간 회의", "fromDocId": c_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{tpl}");
    let tpl_id = tpl["id"].as_str().unwrap().to_owned();
    let (s, page) = api(
        world,
        "POST",
        &c,
        &format!("/v1/workspaces/{ws}/docs"),
        Some(json!({"title":"From template","visibility":"WORKSPACE","templateId": tpl_id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{page}");
    assert_eq!(page["content"]["blocks"][0]["content"][0]["text"], "C");
    // Changing the source page's stored content later changes neither the template nor the copy.
    sqlx::query("UPDATE penfii.doc_pages SET content = jsonb_set(content, '{blocks,0,content,0,text}', '\"changed\"') WHERE id=$1")
        .bind(Uuid::parse_str(&c_id).unwrap()).execute(&world.pool).await.unwrap();
    let stored: Value = sqlx::query_scalar("SELECT content FROM penfii.doc_templates WHERE id=$1")
        .bind(Uuid::parse_str(&tpl_id).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(stored["blocks"][0]["content"][0]["text"], "C");
    let (_, again) = api(
        world,
        "GET",
        &c,
        &format!("/v1/docs/{}", page["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(again["content"]["blocks"][0]["content"][0]["text"], "C");
    let (s, list) = api(
        world,
        "GET",
        &c,
        &format!("/v1/workspaces/{ws}/doc-templates"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    // template xor content
    let (s, _) = api(world, "POST", &c, &format!("/v1/workspaces/{ws}/docs"), Some(json!({"title":"x","visibility":"WORKSPACE","templateId": tpl_id, "content": page_content("y")}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Bad content is a rule violation with the sanitizer's reason.
    let (s, v) = api(world, "POST", &c, &format!("/v1/workspaces/{ws}/docs"), Some(json!({"title":"x","visibility":"WORKSPACE","content": {"schema":"penfii.doc","version":1,"blocks":[{"id":"a","type":"toggleListItem"}]}}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "doc_content_invalid");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t8_t9_t10_restricted_docs_are_visible_to_participants_only(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-restricted-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (admin_id, admin) = invite_member(world, ws, "ADMIN").await;
    let (bob_id, bob) = member(world, ws, "bob").await;
    let (carol_id, carol) = member(world, ws, "carol").await;
    let _ = (admin_id, carol_id);
    // A member creates a restricted page and takes bob in.
    let page = make_doc(
        world,
        &carol,
        ws,
        "비공개 계획",
        json!({
        "visibility": "RESTRICTED", "participants": [{"userId": bob_id, "role": "EDITOR"}]}),
    )
    .await;
    let id = page["id"].as_str().unwrap().to_owned();
    assert_eq!(page["myRole"], "MANAGER");
    for (who, cookie) in [("carol", &carol), ("bob", &bob)] {
        let (s, v) = api(world, "GET", cookie, &format!("/v1/docs/{id}"), None).await;
        assert_eq!(s, StatusCode::OK, "{who} {v}");
    }
    // Not a participant: a plain member, the Workspace admin and the Workspace owner all get 404.
    for (who, cookie) in [("owner", &owner), ("admin", &admin)] {
        let (s, _) = api(world, "GET", cookie, &format!("/v1/docs/{id}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{who}");
        let (_, tree) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/workspaces/{ws}/docs/tree"),
            None,
        )
        .await;
        assert!(
            !node_ids(&tree).contains(&id),
            "{who} must not see it in the tree"
        );
        let (s, _) = api(
            world,
            "GET",
            cookie,
            &format!("/v1/docs/{id}/comments"),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = api(
            world,
            "PUT",
            cookie,
            &format!("/v1/docs/{id}/participants"),
            Some(json!({"participants":[{"userId": admin_id, "role":"MANAGER"}]})),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::NOT_FOUND,
            "{who} cannot take over a restricted page"
        );
        let (s, _) = api(world, "DELETE", cookie, &format!("/v1/docs/{id}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
    // A reference to it resolves to nothing for them and to a title for participants.
    let (_, resolved) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/workspaces/{ws}/references/resolve"),
        Some(json!({"items":[{"kind":"DOC","id": id}]})),
    )
    .await;
    assert_eq!(
        resolved["items"][0],
        json!({"kind":"DOC","id": id,"available": false})
    );
    let (_, resolved) = api(
        world,
        "POST",
        &bob,
        &format!("/v1/workspaces/{ws}/references/resolve"),
        Some(json!({"items":[{"kind":"DOC","id": id}]})),
    )
    .await;
    assert_eq!(resolved["items"][0]["title"], "비공개 계획");

    // T9: removing bob takes effect at once.
    let (s, v) = api(
        world,
        "PUT",
        &carol,
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants":[{"userId": carol_id, "role":"MANAGER"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = api(world, "GET", &bob, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // A restricted page must keep a manager.
    let (s, v) = api(
        world,
        "PUT",
        &carol,
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants":[{"userId": bob_id, "role":"EDITOR"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "doc_manager_required");

    // A page that is open to the Workspace is managed by the owner/admin too, and closing it to
    // RESTRICTED needs a manager participant. Members read it; a stranger from another
    // Workspace never does.
    let open = make_doc(world, &carol, ws, "열린 문서", json!({})).await;
    let open_id = open["id"].as_str().unwrap().to_owned();
    let (s, _) = api(world, "GET", &bob, &format!("/v1/docs/{open_id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = api(
        world,
        "PUT",
        &admin,
        &format!("/v1/docs/{open_id}/visibility"),
        Some(json!({"visibility":"RESTRICTED"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "admin manages an open page: {v}");
    // The admin just closed it on carol (the creator is a MANAGER participant); admin is out now.
    let (s, _) = api(world, "GET", &admin, &format!("/v1/docs/{open_id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "no automatic read after closing");
    let (s, _) = api(world, "GET", &bob, &format!("/v1/docs/{open_id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let other_ws = workspace(world, &format!("p63-other-{}", Uuid::new_v4())).await;
    let (s, _) = api(
        world,
        "GET",
        &owner,
        &format!("/v1/workspaces/{other_ws}/docs/tree"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, other_tree) = api(
        world,
        "GET",
        &owner,
        &format!("/v1/workspaces/{other_ws}/docs/tree"),
        None,
    )
    .await;
    assert!(node_ids(&other_tree).is_empty());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t4_t33_page_metadata_never_fails_on_a_version_and_content_has_no_rest_write(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-meta-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let page = make_doc(world, &c, ws, "Draft", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    assert_eq!(page["revisionNo"], 1);
    // Two renames, both from a client that knows nothing about the current state.
    let (s, v) = api(
        world,
        "PATCH",
        &c,
        &format!("/v1/docs/{id}"),
        Some(json!({"title":"One"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = api(
        world,
        "PATCH",
        &c,
        &format!("/v1/docs/{id}"),
        Some(json!({"title":"Two"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["revisionNo"], 3,
        "each changing title is one materialized revision"
    );
    // The same title again changes nothing and bumps nothing.
    let (_, v) = api(
        world,
        "PATCH",
        &c,
        &format!("/v1/docs/{id}"),
        Some(json!({"title":"Two"})),
    )
    .await;
    assert_eq!(v["revisionNo"], 3);
    // Moving and visibility changes do not touch revision_no.
    let (_, other) = (0, make_doc(world, &c, ws, "Other", json!({})).await);
    api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/move"),
        Some(json!({"parentId": other["id"]})),
    )
    .await;
    let (_, v) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(v["revisionNo"], 3);
    // There is no content write, and an expectedRevision is not an accepted field.
    for body in [
        json!({"content": page_content("x")}),
        json!({"title":"x","expectedRevision": 1}),
        json!({"revisionNo": 1}),
    ] {
        let (s, v) = api(world, "PATCH", &c, &format!("/v1/docs/{id}"), Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    let (s, _) = api(
        world,
        "PUT",
        &c,
        &format!("/v1/docs/{id}"),
        Some(json!({"content": page_content("x")})),
    )
    .await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t24_a_doc_comment_notifies_only_mentioned_people_who_can_read_the_page(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-comment-{}", Uuid::new_v4())).await;
    let (alice_id, alice) = member(world, ws, "alice").await;
    let (bob_id, bob) = member(world, ws, "bob").await;
    let (carol_id, carol) = member(world, ws, "carol").await;
    let page = make_doc(
        world,
        &alice,
        ws,
        "회의록",
        json!({
        "visibility": "RESTRICTED", "participants": [{"userId": bob_id, "role": "EDITOR"}]}),
    )
    .await;
    let id = page["id"].as_str().unwrap().to_owned();
    // carol cannot read the page, so she cannot comment on it.
    let (s, _) = api(
        world,
        "POST",
        &carol,
        &format!("/v1/docs/{id}/comments"),
        Some(json!({"body": doc("hi")})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // alice mentions bob (can read), carol (cannot) and herself.
    let body = json!({"type":"doc","content":[{"type":"paragraph","content":[
        {"type":"mention","attrs":{"occurrenceId":Uuid::new_v4(),"kind":"USER","userId":bob_id}},
        {"type":"mention","attrs":{"occurrenceId":Uuid::new_v4(),"kind":"USER","userId":carol_id}},
        {"type":"mention","attrs":{"occurrenceId":Uuid::new_v4(),"kind":"USER","userId":alice_id}},
        {"type":"text","text":" 확인 부탁"}]}]});
    let (s, comment) = api(
        world,
        "POST",
        &alice,
        &format!("/v1/docs/{id}/comments"),
        Some(json!({"body": body})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{comment}");
    attention_now(ws, world).await;
    let bob_inbox = inbox(world, &bob, ws, "").await;
    assert_eq!(kinds(&bob_inbox), vec!["mention.user"], "{bob_inbox}");
    assert_eq!(bob_inbox["items"][0]["subject"]["kind"], "DOC");
    assert_eq!(bob_inbox["items"][0]["subject"]["title"], "회의록");
    assert_eq!(bob_inbox["items"][0]["link"]["docId"], json!(id));
    assert!(
        kinds(&inbox(world, &carol, ws, "").await).is_empty(),
        "carol cannot read the page"
    );
    assert!(
        kinds(&inbox(world, &alice, ws, "").await).is_empty(),
        "no notice for yourself"
    );
    // Handling the event again adds nothing.
    sqlx::query("UPDATE penfii.consumer_receipts SET state='PENDING' WHERE consumer='inbox' AND event_id IN (SELECT event_id FROM penfii.domain_events WHERE workspace_id=$1 AND event_kind='doc.comment_created')").bind(ws).execute(&world.pool).await.unwrap();
    attention_now(ws, world).await;
    assert_eq!(kinds(&inbox(world, &bob, ws, "").await).len(), 1);
    // Listing, editing and deleting.
    let (s, list) = api(world, "GET", &bob, &format!("/v1/docs/{id}/comments"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    let cid = comment["id"].as_str().unwrap().to_owned();
    let (s, _) = api(
        world,
        "PATCH",
        &bob,
        &format!("/v1/docs/{id}/comments/{cid}"),
        Some(json!({"body": doc("hijack")})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "only the author edits");
    let (s, _) = api(
        world,
        "PATCH",
        &alice,
        &format!("/v1/docs/{id}/comments/{cid}"),
        Some(doc("edited")),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "the body must be wrapped as {{body}}"
    );
    let (s, v) = api(
        world,
        "PATCH",
        &alice,
        &format!("/v1/docs/{id}/comments/{cid}"),
        Some(json!({"body": doc("edited")})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = api(
        world,
        "DELETE",
        &bob,
        &format!("/v1/docs/{id}/comments/{cid}"),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a participant who is not a manager cannot delete"
    );
    let (s, _) = api(
        world,
        "DELETE",
        &alice,
        &format!("/v1/docs/{id}/comments/{cid}"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list) = api(world, "GET", &bob, &format!("/v1/docs/{id}/comments"), None).await;
    assert!(list["items"].as_array().unwrap().is_empty());
}

//  the `doc` kind
fn doc_with(blocks: Value) -> Value {
    json!({"schema":"penfii.doc","version":1,"blocks": blocks})
}

pub(super) fn para(text: &str) -> Value {
    json!({"id": Uuid::new_v4().to_string(), "type":"paragraph", "props":{}, "content":[{"type":"text","text":text,"styles":{}}], "children":[]})
}

pub(super) async fn make_doc_with(
    world: &World,
    cookie: &str,
    ws: Uuid,
    title: &str,
    blocks: Value,
    extra: Value,
) -> String {
    let mut body = json!({"title": title, "visibility": "WORKSPACE", "content": doc_with(blocks)});
    if let (Some(map), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    let (s, v) = api(
        world,
        "POST",
        cookie,
        &format!("/v1/workspaces/{ws}/docs"),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_owned()
}

pub(super) async fn indexed_count(sw: &SearchWorld, kind: &str, workspace_id: Uuid) -> i64 {
    penfii_core::search::refresh_all(&sw.engine).await.unwrap();
    let (_, body) = sw
        .engine
        .indexer
        .client
        .admin(
            penfii_core::search::client::Method::GET,
            &format!("/{}/_count", sw.engine.indexer.client.alias(kind)),
            Some(&json!({"query":{"term":{"workspaceId": workspace_id}}})),
        )
        .await
        .unwrap();
    body["count"].as_i64().unwrap_or(-1)
}

pub(super) async fn events_of(pool: &PgPool, target: Uuid) -> Vec<penfii_core::events::EventRow> {
    use sqlx::Row;
    sqlx::query("SELECT event_id,seq,workspace_id,target_kind,target_id,target_version,event_kind,actor_user_id,command_id,facts,security_context,occurred_at FROM penfii.domain_events WHERE target_id=$1 ORDER BY seq")
        .bind(target)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| penfii_core::events::EventRow {
            event_id: r.get("event_id"), seq: r.get("seq"), workspace_id: r.get("workspace_id"),
            target_kind: r.get("target_kind"), target_id: r.get("target_id"), target_version: r.get("target_version"),
            kind: r.get("event_kind"), actor_user_id: r.get("actor_user_id"), command_id: r.get("command_id"),
            facts: r.get("facts"), security_context: r.get("security_context"), occurred_at: r.get("occurred_at"),
        })
        .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_doc_lists_readable_docs_that_reference_it(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("doc-refs-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (carol_id, carol) = member(world, ws, "carol").await;
    let (_, bob) = member(world, ws, "bob").await;
    let target = make_doc(world, &owner, ws, "대상", json!({})).await;
    let target_id = target["id"].as_str().unwrap();
    let link = || {
        json!({"content": doc_with(json!([{
            "id":"r","type":"paragraph","props":{},
            "content":[{"type":"text","text":"see ","styles":{}},{"type":"reference","props":{"kind":"DOC","id": target_id}}],
            "children":[]
        }]))})
    };
    let public_source = make_doc(world, &owner, ws, "공개 출처", link()).await;
    let hidden = make_doc(
        world,
        &carol,
        ws,
        "숨은 출처",
        json!({
            "visibility": "RESTRICTED",
            "participants": [{"userId": carol_id, "role": "EDITOR"}],
            "content": link()["content"].clone(),
        }),
    )
    .await;
    let _ = make_doc(
        world,
        &owner,
        ws,
        "티켓만",
        json!({"content": doc_with(json!([para("no doc link")]))}),
    )
    .await;

    let (s, listed) = api(
        world,
        "GET",
        &owner,
        &format!("/v1/docs/{target_id}/references"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    let titles: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["공개 출처"], "{listed}");
    assert_eq!(listed["items"][0]["sourceId"], public_source["id"]);
    assert_eq!(listed["items"][0]["sourceKind"], "DOC");

    let (s, seen) = api(
        world,
        "GET",
        &carol,
        &format!("/v1/docs/{target_id}/references"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{seen}");
    let carol_titles: Vec<&str> = seen["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert!(carol_titles.contains(&"공개 출처"), "{seen}");
    assert!(carol_titles.contains(&"숨은 출처"), "{seen}");
    assert!(!titles.contains(&"숨은 출처"));

    let secret = make_doc(
        world,
        &carol,
        ws,
        "비공개 대상",
        json!({"visibility":"RESTRICTED","participants":[{"userId": carol_id, "role":"MANAGER"}]}),
    )
    .await;
    let secret_id = secret["id"].as_str().unwrap();
    let (s, _) = api(
        world,
        "GET",
        &bob,
        &format!("/v1/docs/{secret_id}/references"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let _ = hidden;
}

// Wiki Save and Versions
pub(super) async fn save_doc(
    world: &World,
    cookie: &str,
    id: &str,
    revision: i64,
    title: &str,
    text: &str,
) -> (StatusCode, Value) {
    api(
        world,
        "PUT",
        cookie,
        &format!("/v1/docs/{id}/content"),
        Some(
            json!({"expectedRevisionNo": revision, "title": title, "content": {"schema":"penfii.doc","version":1,"blocks":[
                {"id":"fixed-block","type":"paragraph","props":{},"content":[{"type":"text","text":text,"styles":{}}],"children":[]}]}}),
        ),
    )
    .await
}

fn first_text(content: &Value) -> String {
    content["blocks"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_title_is_required_to_create_rename_and_save(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-title-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    for title in ["", "   "] {
        let (s, v) = api(
            world,
            "POST",
            &c,
            &format!("/v1/workspaces/{ws}/docs"),
            Some(json!({"title": title, "visibility": "WORKSPACE"})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    let page = make_doc(world, &c, ws, "Named", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let (s, _) = api(
        world,
        "PATCH",
        &c,
        &format!("/v1/docs/{id}"),
        Some(json!({"title": " "})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = save_doc(world, &c, &id, 1, "  ", "x").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(
        (v["title"].as_str(), v["revisionNo"].as_i64()),
        (Some("Named"), Some(1)),
        "a refused Save changes nothing"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_save_replaces_the_page_checks_the_revision_and_creates_no_version(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-save-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let (_, bob_cookie) = invite_member(world, ws, "MEMBER").await;
    let page = make_doc(world, &c, ws, "Plan", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    assert_eq!(page["revisionNo"], 1);
    assert_eq!(page["latestVersionNo"], Value::Null);
    assert_eq!(page["hasUnversionedChanges"], true);

    let (s, v) = save_doc(world, &c, &id, 1, "Plan v2", "first").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["title"].as_str(), v["revisionNo"].as_i64()),
        (Some("Plan v2"), Some(2))
    );
    assert_eq!(first_text(&v["content"]), "first");
    // Saving the same thing again is a no-op: no revision, no change.
    let (_, v) = save_doc(world, &c, &id, 2, "Plan v2", "first").await;
    assert_eq!(v["revisionNo"], 2);
    // A second person who still holds revision 1 is refused and told the current one.
    let (s, v) = save_doc(world, &bob_cookie, &id, 1, "Plan v2", "bob").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "version_conflict");
    assert_eq!(v["currentRevision"], 2);
    let (_, v) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(first_text(&v["content"]), "first");
    // Saves are not History.
    let (s, v) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["items"].as_array().unwrap().is_empty(), "{v}");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.doc_versions WHERE page_id=$1")
            .bind(Uuid::parse_str(&id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);

    // The co-editing working copy follows the saved page (a new lineage seeded from it).
    let loaded =
        penfii_core::docs::collab::store::load_or_seed(&world.pool, Uuid::parse_str(&id).unwrap())
            .await
            .ok()
            .expect("lineage");
    let (collab_content, _) = penfii_core::docs::yjs::materialize(&loaded.doc, 1 << 20).unwrap();
    assert_eq!(first_text(&collab_content), "first");

    // A person who cannot read the page learns nothing; the title and content are untouched.
    let restricted = make_doc(world, &c, ws, "Secret", json!({"visibility":"RESTRICTED"})).await;
    let rid = restricted["id"].as_str().unwrap().to_owned();
    let (s, _) = save_doc(world, &bob_cookie, &rid, 1, "Hacked", "x").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = api(
        world,
        "POST",
        &bob_cookie,
        &format!("/v1/docs/{rid}/versions"),
        Some(json!({"expectedRevisionNo": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_versions_are_made_only_on_request_and_never_change(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-ver-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let page = make_doc(world, &c, ws, "Spec", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let pid = Uuid::parse_str(&id).unwrap();

    // The first Version of a page that never had one (also what every existing page looks like).
    let (s, v1) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v1}");
    assert_eq!(v1["versionNo"], 1);
    assert_eq!(v1["isCurrent"], true);
    // Nothing changed since: a second Version would be a duplicate.
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "doc_version_unchanged");
    let (_, p) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(
        (
            p["latestVersionNo"].as_i64(),
            p["hasUnversionedChanges"].as_bool()
        ),
        (Some(1), Some(false))
    );

    // Several Saves, no Version; the saved page now differs from Version 1.
    for (rev, text) in [(1, "a"), (2, "ab"), (3, "abc")] {
        let (s, _) = save_doc(world, &c, &id, rev, "Spec", text).await;
        assert_eq!(s, StatusCode::OK);
    }
    let (_, p) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(
        (
            p["revisionNo"].as_i64(),
            p["hasUnversionedChanges"].as_bool()
        ),
        (Some(4), Some(true))
    );
    let (_, list) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions"), None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    // A stale revision cannot freeze a page it has not seen.
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": 2})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (s, v2) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": 4})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v2}");
    assert_eq!(v2["versionNo"], 2);

    // History: newest first; each Version keeps the title and content it froze.
    let (_, list) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions"), None).await;
    let numbers: Vec<i64> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["versionNo"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, vec![2, 1]);
    assert_eq!(list["items"][0]["isCurrent"], true);
    assert_eq!(list["items"][1]["isCurrent"], false);
    let (_, one) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions/1"), None).await;
    assert_eq!(first_text(&one["content"]), "Spec");
    let (_, two) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions/2"), None).await;
    assert_eq!(first_text(&two["content"]), "abc");
    let (s, _) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions/9"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Restore = read a Version, then Save it as the next state. History is only ever added to.
    let (s, v) = save_doc(world, &c, &id, 4, "Spec", &first_text(&one["content"])).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v3) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": 5})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v3}");
    assert_eq!(v3["versionNo"], 3);
    // The restored content equals Version 1's, yet it is a new Version because Version 2 differs.
    let (_, one_again) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions/1"), None).await;
    assert_eq!(one_again, one);

    // Immutable in the database as well.
    for statement in [
        "UPDATE penfii.doc_versions SET title='x' WHERE page_id=$1",
        "DELETE FROM penfii.doc_versions WHERE page_id=$1",
    ] {
        assert!(
            sqlx::query(statement)
                .bind(pid)
                .execute(&world.pool)
                .await
                .is_err(),
            "{statement}"
        );
    }
}

/// Makes a page look like one the old model left behind: an empty title. The title trigger refuses
/// new blank titles, so the row is written with triggers off for this session only.
async fn make_untitled(world: &World, page_id: Uuid) {
    let mut tx = world.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE penfii.doc_pages SET title='' WHERE id=$1")
        .bind(page_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_existing_pages_without_versions_or_with_empty_titles_still_work(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-legacy-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let page = make_doc(world, &c, ws, "Old", json!({})).await;
    let other = make_doc(world, &c, ws, "Other", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let pid = Uuid::parse_str(&id).unwrap();
    make_untitled(world, pid).await;
    let (_, v) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(v["title"], "");
    assert_eq!(v["latestVersionNo"], Value::Null);
    // The old row stays as it is: unrelated changes (move, visibility) are not blocked by its title.
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/move"),
        Some(json!({"parentId": other["id"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = api(
        world,
        "PUT",
        &c,
        &format!("/v1/docs/{id}/visibility"),
        Some(json!({"visibility":"RESTRICTED"})),
    )
    .await;
    assert!(s.is_success(), "{v}");
    // It cannot be saved, renamed or versioned without a title.
    let (s, _) = save_doc(world, &c, &id, 1, "", "x").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = api(
        world,
        "PATCH",
        &c,
        &format!("/v1/docs/{id}"),
        Some(json!({"title":""})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, cur) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    let rev = cur["revisionNo"].as_i64().unwrap();
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": rev})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "doc_title_required");
    // With a title everything works like on a new page.
    let (s, v) = save_doc(world, &c, &id, rev, "Titled now", "x").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": rev + 1})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_the_database_refuses_a_blank_title_but_not_unrelated_changes_to_an_old_one(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-trigger-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let page = make_doc(world, &c, ws, "Named", json!({})).await;
    let pid = Uuid::parse_str(page["id"].as_str().unwrap()).unwrap();
    assert!(
        sqlx::query("UPDATE penfii.doc_pages SET title='  ' WHERE id=$1")
            .bind(pid)
            .execute(&world.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO penfii.doc_pages(id,workspace_id,title,visibility,content,content_schema_version,content_hash,created_by) SELECT $1,workspace_id,'',visibility,content,1,content_hash,created_by FROM penfii.doc_pages WHERE id=$2")
        .bind(Uuid::new_v4()).bind(pid).execute(&world.pool).await.is_err());
    make_untitled(world, pid).await;
    sqlx::query(
        "UPDATE penfii.doc_pages SET position = position + 1, updated_at = now() WHERE id=$1",
    )
    .bind(pid)
    .execute(&world.pool)
    .await
    .expect("an old untitled row can still be updated");
    sqlx::query("UPDATE penfii.doc_pages SET title='' WHERE id=$1")
        .bind(pid)
        .execute(&world.pool)
        .await
        .expect("setting the same blank title is not a change");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_a_room_from_before_a_save_cannot_write_its_old_state_over_it(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-epoch-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let page = make_doc(world, &c, ws, "Race", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let pid = Uuid::parse_str(&id).unwrap();
    // A room opened before the Save holds the lineage of that moment.
    let before = penfii_core::docs::collab::store::load_or_seed(&world.pool, pid)
        .await
        .ok()
        .unwrap();
    let (s, _) = save_doc(world, &c, &id, 1, "Race", "saved text").await;
    assert_eq!(s, StatusCode::OK);
    let cfg = KnowledgeConfig::default();
    let old_state = page_content("old room text");
    let applied = penfii_core::docs::materialize::apply(
        &world.pool,
        &cfg,
        pid,
        &old_state,
        &[world.owner_id],
        Some(before.epoch),
    )
    .await
    .unwrap();
    assert!(
        matches!(applied, penfii_core::docs::materialize::Applied::Stale),
        "{applied:?}"
    );
    let (_, v) = api(world, "GET", &c, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(first_text(&v["content"]), "saved text");
    assert_eq!(v["revisionNo"], 2);
    // A room opened after the Save writes normally.
    let now = penfii_core::docs::collab::store::load_or_seed(&world.pool, pid)
        .await
        .ok()
        .unwrap();
    let applied = penfii_core::docs::materialize::apply(
        &world.pool,
        &cfg,
        pid,
        &page_content("new room text"),
        &[world.owner_id],
        Some(now.epoch),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            applied,
            penfii_core::docs::materialize::Applied::Written { .. }
        ),
        "{applied:?}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn wiki_a_version_keeps_the_files_it_names_so_a_restore_can_bring_them_back(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("wiki-files-{}", Uuid::new_v4())).await;
    let c = world.owner_cookie.clone();
    let page = make_doc(world, &c, ws, "Figure", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let pid = Uuid::parse_str(&id).unwrap();
    let file_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'도면.png',5,'image/png','AVAILABLE',$4)")
        .bind(file_id).bind(ws).bind(format!("wiki/{file_id}")).bind(world.owner_id).execute(&world.pool).await.unwrap();
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/files"),
        Some(json!({"fileId": file_id})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let with_image = |text: &str| {
        json!({"schema":"penfii.doc","version":1,"blocks":[
        {"id":"3f1c1d0e-6b7a-4c3a-9d55-0a1b2c3d4e5f","type":"image","props":{"url": penfii_core::docs::content::file_url(file_id),"name":"도면.png"},"children":[]},
        {"id":"4f1c1d0e-6b7a-4c3a-9d55-0a1b2c3d4e5f","type":"paragraph","props":{},"content":[{"type":"text","text":text,"styles":{}}],"children":[]}]})
    };
    let path = format!("/v1/docs/{id}/content");
    let put = |rev: i64, content: Value| {
        let path = path.clone();
        let c = c.clone();
        async move {
            api(
                world,
                "PUT",
                &c,
                &path,
                Some(json!({"expectedRevisionNo": rev, "title":"Figure","content": content})),
            )
            .await
        }
    };
    let (s, v) = put(1, with_image("a")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = api(
        world,
        "POST",
        &c,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": 2})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.doc_version_files WHERE page_id=$1 AND version_no=1 AND file_id=$2").bind(pid).bind(file_id).fetch_one(&world.pool).await.unwrap();
    assert_eq!(kept, 1);
    // The page stops using the file and the grace period passes.
    let (s, _) = put(2, doc_with(json!([para("no image now")]))).await;
    assert_eq!(s, StatusCode::OK);
    sqlx::query("UPDATE penfii.doc_page_files SET attached_at = now() - interval '30 days', unreferenced_since = now() - interval '30 days' WHERE page_id=$1").bind(pid).execute(&world.pool).await.unwrap();
    let (s, _) = put(3, doc_with(json!([para("still no image")]))).await;
    assert_eq!(s, StatusCode::OK);
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.doc_page_files WHERE page_id=$1 AND file_id=$2",
    )
    .bind(pid)
    .bind(file_id)
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(
        left, 1,
        "a Version still names the file, so it stays attached"
    );
    // Restore = read the Version, Save it: the image is still there.
    let (_, v1) = api(world, "GET", &c, &format!("/v1/docs/{id}/versions/1"), None).await;
    let (s, v) = put(4, v1["content"].clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["content"]["blocks"][0]["type"], "image",
        "the file node survived the Restore"
    );
}
