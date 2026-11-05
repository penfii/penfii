//! Files on Tickets: body and comment attachments, access that follows the Ticket, trash and
//! restore, purge, and the rules shared with messages and Docs (a file is bound to one place).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use penfii_core::files::ObjectStore;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, Reply, doc};

/// [`common::call`], or with `bytes` an upload body.
async fn raw(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
    bytes: Option<Vec<u8>>,
) -> Reply {
    let Some(bytes) = bytes else {
        return common::call(app, method, path, cookie, body).await;
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-penfii-client", "1")
        .header("content-type", "application/octet-stream")
        .header("content-length", bytes.len());
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    common::send(app, builder.body(Body::from(bytes)).unwrap()).await
}

/// The shared fixture and the object store behind its router.
struct Env {
    base: common::Env,
    objects: ObjectStore,
}

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.base
    }
}

async fn setup(pool: PgPool) -> Env {
    setup_with(pool, ObjectStore::in_memory()).await
}

async fn setup_with(pool: PgPool, objects: ObjectStore) -> Env {
    Env {
        base: common::Env::with_objects(pool, objects.clone()).await,
        objects,
    }
}

impl Env {
    async fn send(&self, who: &Person, method: &str, path: &str, body: Option<Value>) -> Reply {
        raw(&self.app, method, path, Some(&who.cookie), body, None).await
    }

    /// A finished upload by `who`.
    async fn upload(&self, who: &Person, name: &str, content: &[u8]) -> Uuid {
        let created = self
            .send(
                who,
                "POST",
                &format!("/v1/workspaces/{}/files", self.ws),
                Some(json!({"filename": name, "byteSize": content.len()})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let id = created.body["id"].as_str().unwrap().to_string();
        let put = raw(
            &self.app,
            "PUT",
            &format!("/v1/files/{id}/content"),
            Some(&who.cookie),
            None,
            Some(content.to_vec()),
        )
        .await;
        assert_eq!(put.status, StatusCode::OK, "{}", put.body);
        assert_eq!(put.body["state"], "AVAILABLE");
        id.parse().unwrap()
    }

    async fn ticket(&self, project: &str, title: &str) -> Uuid {
        let created = self
            .send(
                &self.owner,
                "POST",
                &format!("/v1/projects/{project}/tickets"),
                Some(json!({"title": title, "hierarchyKind": "TASK"})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["id"].as_str().unwrap().parse().unwrap()
    }

    async fn file_row(&self, file: Uuid) -> (bool, i64) {
        sqlx::query_as("SELECT deleted_at IS NOT NULL, generation FROM penfii.files WHERE id = $1")
            .bind(file)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn has_object(&self, file: Uuid) -> bool {
        let key: String = sqlx::query_scalar("SELECT storage_key FROM penfii.files WHERE id = $1")
            .bind(file)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        self.objects.get(&key).await.is_ok()
    }
}

async fn project_with_editor(env: &Env, editor: &Person) -> String {
    let team = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/teams", env.ws),
            Some(json!({"name": "Dev", "members": [{"userId": editor.id, "role": "MANAGER"}]})),
        )
        .await;
    let team_id = team.body["id"].as_str().unwrap().to_string();
    let added = env
        .send(
            &env.owner,
            "PUT",
            &format!(
                "/v1/workspaces/{}/teams/{team_id}/members/{}",
                env.ws, editor.id
            ),
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert_eq!(added.status, StatusCode::NO_CONTENT);
    let project = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Files", "teamIds": [team_id]})),
        )
        .await;
    assert_eq!(project.status, StatusCode::CREATED, "{}", project.body);
    let id = project.body["id"].as_str().unwrap().to_string();
    let taking_part = env
        .send(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{id}/teams/{team_id}/participation"),
            Some(json!({"role": "EDITOR"})),
        )
        .await;
    assert_eq!(taking_part.status, StatusCode::NO_CONTENT);
    id
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn body_and_comment_files_bind_once_and_follow_the_ticket(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let outsider = env.outsider().await;
    let project = project_with_editor(&env, &editor).await;
    let ticket = env.ticket(&project, "with files").await;
    let other_ticket = env.ticket(&project, "another").await;
    let attachments = format!("/v1/tickets/{ticket}/attachments");

    // The body: several files, repeating a call changes nothing, a file is bound to one place.
    let notes = env.upload(&editor, "notes.txt", b"plain notes").await;
    let report = env.upload(&env.owner, "report.pdf", b"%PDF-1.4 fake").await;
    let first = env
        .send(
            &editor,
            "POST",
            &attachments,
            Some(json!({"fileId": notes})),
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    assert_eq!(first.body["filename"], "notes.txt");
    assert!(first.body["commentId"].is_null());
    let again = env
        .send(
            &editor,
            "POST",
            &attachments,
            Some(json!({"fileId": notes})),
        )
        .await;
    assert_eq!(again.status, StatusCode::CREATED);
    assert_eq!(again.body["id"], first.body["id"]);
    let second = env
        .send(
            &env.owner,
            "POST",
            &attachments,
            Some(json!({"fileId": report})),
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.body);
    let listed = env.send(&editor, "GET", &attachments, None).await;
    assert_eq!(listed.body.as_array().unwrap().len(), 2);
    let taken = env
        .send(
            &editor,
            "POST",
            &format!("/v1/tickets/{other_ticket}/attachments"),
            Some(json!({"fileId": notes})),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.body);
    assert_eq!(taken.body["code"], "file_attached");
    // Somebody else's upload is not available to bind.
    let theirs = env
        .send(
            &editor,
            "POST",
            &format!("/v1/tickets/{other_ticket}/attachments"),
            Some(json!({"fileId": report})),
        )
        .await;
    assert_ne!(theirs.status, StatusCode::CREATED);

    // A bound file is no longer a loose upload: it cannot be read or deleted that way.
    let loose = env
        .send(&editor, "GET", &format!("/v1/files/{notes}/content"), None)
        .await;
    assert_eq!(loose.status, StatusCode::NOT_FOUND);
    let delete = env
        .send(&editor, "DELETE", &format!("/v1/files/{notes}"), None)
        .await;
    assert_eq!(delete.status, StatusCode::CONFLICT, "{}", delete.body);
    assert_eq!(delete.body["code"], "file_attached");
    assert!(!env.file_row(notes).await.0);

    // Comments carry their files in the same request; a file already used fails the whole comment.
    let photo = env
        .upload(&editor, "shot.png", b"\x89PNG\r\n\x1a\nrest")
        .await;
    let comments = format!("/v1/tickets/{ticket}/comments");
    let comment = env
        .send(
            &editor,
            "POST",
            &comments,
            Some(json!({"body": doc("see attached"), "fileIds": [photo]})),
        )
        .await;
    assert_eq!(comment.status, StatusCode::CREATED, "{}", comment.body);
    assert_eq!(comment.body["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(comment.body["attachments"][0]["filename"], "shot.png");
    assert_eq!(
        comment.body["attachments"][0]["commentId"],
        comment.body["id"]
    );
    let reused = env
        .send(
            &editor,
            "POST",
            &comments,
            Some(json!({"body": doc("again"), "fileIds": [photo]})),
        )
        .await;
    assert_eq!(reused.status, StatusCode::CONFLICT, "{}", reused.body);
    let on_body = env
        .send(
            &editor,
            "POST",
            &comments,
            Some(json!({"body": doc("steal"), "fileIds": [notes]})),
        )
        .await;
    assert_eq!(on_body.status, StatusCode::CONFLICT);
    let listed = env.send(&editor, "GET", &comments, None).await;
    assert_eq!(
        listed.body.as_array().unwrap().len(),
        1,
        "failed comments are not kept"
    );
    assert_eq!(
        listed.body[0]["attachments"][0]["id"],
        comment.body["attachments"][0]["id"]
    );
    let plain = env
        .send(
            &editor,
            "POST",
            &comments,
            Some(json!({"body": doc("no files")})),
        )
        .await;
    assert_eq!(plain.status, StatusCode::CREATED);
    assert_eq!(plain.body["attachments"], json!([]));

    // Downloads: bytes, image inline, other files as downloads; strangers see nothing.
    let body_id = first.body["id"].as_str().unwrap();
    let comment_att = comment.body["attachments"][0]["id"].as_str().unwrap();
    let download = env
        .send(
            &env.owner,
            "GET",
            &format!("/v1/attachments/{body_id}/content"),
            None,
        )
        .await;
    assert_eq!(download.status, StatusCode::OK);
    assert_eq!(download.bytes, b"plain notes");
    assert!(
        download
            .header("content-disposition")
            .unwrap()
            .starts_with("attachment")
    );
    let image = env
        .send(
            &env.owner,
            "GET",
            &format!("/v1/attachments/{comment_att}/content"),
            None,
        )
        .await;
    assert_eq!(image.status, StatusCode::OK);
    assert!(
        image
            .header("content-disposition")
            .unwrap()
            .starts_with("inline")
    );
    for id in [body_id, comment_att] {
        let hidden = env
            .send(
                &outsider,
                "GET",
                &format!("/v1/attachments/{id}/content"),
                None,
            )
            .await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        env.send(&outsider, "GET", &attachments, None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(
            &outsider,
            "POST",
            &comments,
            Some(json!({"body": doc("x"), "fileIds": []}))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // Removing: the person who attached it (or a Project admin), never a comment's file.
    let not_theirs = env
        .send(
            &editor,
            "DELETE",
            &format!("{attachments}/{}", second.body["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(
        not_theirs.status,
        StatusCode::FORBIDDEN,
        "{}",
        not_theirs.body
    );
    let comment_file = env
        .send(
            &env.owner,
            "DELETE",
            &format!("{attachments}/{comment_att}"),
            None,
        )
        .await;
    assert_eq!(
        comment_file.status,
        StatusCode::CONFLICT,
        "{}",
        comment_file.body
    );
    assert_eq!(comment_file.body["code"], "comment_attachment");
    let removed = env
        .send(&editor, "DELETE", &format!("{attachments}/{body_id}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(env.file_row(notes).await.0, "the file is retired");
    assert!(!env.has_object(notes).await, "and its object removed");
    assert_eq!(
        env.send(
            &env.owner,
            "GET",
            &format!("/v1/attachments/{body_id}/content"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let removed_admin = env
        .send(
            &env.owner,
            "DELETE",
            &format!("{attachments}/{}", second.body["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(
        removed_admin.status,
        StatusCode::NO_CONTENT,
        "a Project admin may remove any"
    );

    // Deleting a comment takes its files with it: the file is retired, its object removed and the
    // download is gone. An edit leaves them.
    let comment_id = comment.body["id"].as_str().unwrap();
    let edited = env
        .send(
            &editor,
            "PATCH",
            &format!("{comments}/{comment_id}"),
            Some(json!({"body": doc("reworded")})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(edited.body["attachments"].as_array().unwrap().len(), 1);
    assert!(!env.file_row(photo).await.0);
    let gone = env
        .send(&editor, "DELETE", &format!("{comments}/{comment_id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert!(env.file_row(photo).await.0, "the comment's file is retired");
    assert!(!env.has_object(photo).await, "and its object removed");
    assert_eq!(
        env.send(
            &env.owner,
            "GET",
            &format!("/v1/attachments/{comment_att}/content"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn trash_hides_files_restore_returns_them_and_purge_erases_them(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let ticket = env.ticket(&project, "doomed").await;
    let body_file = env.upload(&env.owner, "plan.txt", b"the plan").await;
    let comment_file = env.upload(&env.owner, "proof.txt", b"the proof").await;
    let body = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/tickets/{ticket}/attachments"),
            Some(json!({"fileId": body_file})),
        )
        .await;
    assert_eq!(body.status, StatusCode::CREATED, "{}", body.body);
    let comment = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/tickets/{ticket}/comments"),
            Some(json!({"body": doc("proof"), "fileIds": [comment_file]})),
        )
        .await;
    assert_eq!(comment.status, StatusCode::CREATED, "{}", comment.body);
    let body_att = body.body["id"].as_str().unwrap().to_string();
    let comment_att = comment.body["attachments"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let download = |id: String, who: Person| {
        let env = &env;
        async move {
            env.send(&who, "GET", &format!("/v1/attachments/{id}/content"), None)
                .await
                .status
        }
    };
    assert_eq!(
        download(body_att.clone(), editor.clone()).await,
        StatusCode::OK
    );

    // In the trash nobody can reach the files; the comment listing shows none either.
    let cmd = |name: &str| format!("/v1/tickets/{ticket}/commands/{name}");
    assert_eq!(
        env.send(&env.owner, "POST", &cmd("trash"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    for id in [&body_att, &comment_att] {
        assert_eq!(
            download(id.clone(), editor.clone()).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            download(id.clone(), env.owner.clone()).await,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        env.send(
            &editor,
            "GET",
            &format!("/v1/tickets/{ticket}/attachments"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let in_trash = env
        .send(
            &editor,
            "GET",
            &format!("/v1/tickets/{ticket}/comments"),
            None,
        )
        .await;
    if in_trash.status == StatusCode::OK {
        assert_eq!(in_trash.body[0]["attachments"], json!([]));
    }

    // Restored, they are back, bytes intact.
    assert_eq!(
        env.send(&env.owner, "POST", &cmd("restore-from-trash"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    for id in [&body_att, &comment_att] {
        assert_eq!(download(id.clone(), editor.clone()).await, StatusCode::OK);
    }
    let again = env
        .send(
            &editor,
            "GET",
            &format!("/v1/tickets/{ticket}/comments"),
            None,
        )
        .await;
    assert_eq!(again.body[0]["attachments"].as_array().unwrap().len(), 1);

    // Purge needs the trash first, then erases rows, files and objects.
    assert_eq!(
        env.send(&env.owner, "POST", &cmd("purge"), None)
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.send(&env.owner, "POST", &cmd("trash"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(env.has_object(body_file).await && env.has_object(comment_file).await);
    let purged = env.send(&env.owner, "POST", &cmd("purge"), None).await;
    assert_eq!(purged.status, StatusCode::NO_CONTENT, "{}", purged.body);
    for file in [body_file, comment_file] {
        assert!(env.file_row(file).await.0, "file retired");
        assert!(!env.has_object(file).await, "object removed");
    }
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.ticket_attachments WHERE ticket_id = $1")
            .bind(ticket)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(left, 0);
    for id in [&body_att, &comment_att] {
        assert_eq!(
            download(id.clone(), env.owner.clone()).await,
            StatusCode::NOT_FOUND
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_purge_does_not_depend_on_the_objects_still_existing(pool: PgPool) {
    // Deleting an object that is already gone succeeds, so a purge repeated after a failure (or
    // an object lost some other way) still goes through and retires the file.
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let ticket = env.ticket(&project, "gone").await;
    let file = env.upload(&env.owner, "a.txt", b"a").await;
    let attached = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/tickets/{ticket}/attachments"),
            Some(json!({"fileId": file})),
        )
        .await;
    assert_eq!(attached.status, StatusCode::CREATED);
    let key: String = sqlx::query_scalar("SELECT storage_key FROM penfii.files WHERE id = $1")
        .bind(file)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    env.objects.delete(&key).await.unwrap();
    let cmd = |name: &str| format!("/v1/tickets/{ticket}/commands/{name}");
    assert_eq!(
        env.send(&env.owner, "POST", &cmd("trash"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&env.owner, "POST", &cmd("purge"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(env.file_row(file).await.0);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_file_bound_to_a_ticket_message_or_page_is_not_free_for_another(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let ticket = env.ticket(&project, "shared").await;
    let page = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(json!({"title": "Page", "visibility": "WORKSPACE", "content": {"schema": "penfii.doc", "version": 1, "blocks": [
                {"id": Uuid::new_v4().to_string(), "type": "paragraph", "props": {}, "content": [{"type": "text", "text": "Page", "styles": {}}], "children": []}
            ]}})),
        )
        .await;
    assert_eq!(page.status, StatusCode::CREATED, "{}", page.body);
    let page = page.body["id"].as_str().unwrap().to_string();
    let channel = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "general", "visibility": "PUBLIC", "teamId": null, "projectId": null})),
        )
        .await;
    assert_eq!(channel.status, StatusCode::CREATED, "{}", channel.body);

    // A file on a Ticket cannot also go into a Doc page or a message.
    let on_ticket = env.upload(&env.owner, "t.txt", b"ticket").await;
    let bound = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/tickets/{ticket}/attachments"),
            Some(json!({"fileId": on_ticket})),
        )
        .await;
    assert_eq!(bound.status, StatusCode::CREATED, "{}", bound.body);
    let doc_try = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/docs/{page}/files"),
            Some(json!({"fileId": on_ticket})),
        )
        .await;
    assert_eq!(doc_try.status, StatusCode::NOT_FOUND, "{}", doc_try.body);
    let message = env
        .send(
            &env.owner,
            "POST",
            &format!(
                "/v1/conversations/{}/messages",
                channel.body["conversationId"]
                    .as_str()
                    .unwrap_or_else(|| channel.body["id"].as_str().unwrap())
            ),
            Some(json!({"bodySchemaVersion": 1, "body": doc("hi"), "fileIds": [on_ticket]})),
        )
        .await;
    assert_ne!(message.status, StatusCode::CREATED, "{}", message.body);

    // A file in a Doc page cannot go onto a Ticket, nor be deleted as a loose upload.
    let in_page = env.upload(&env.owner, "p.txt", b"page").await;
    let attached = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/docs/{page}/files"),
            Some(json!({"fileId": in_page})),
        )
        .await;
    assert_eq!(attached.status, StatusCode::OK, "{}", attached.body);
    let on_page = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/tickets/{ticket}/attachments"),
            Some(json!({"fileId": in_page})),
        )
        .await;
    assert_eq!(on_page.status, StatusCode::CONFLICT, "{}", on_page.body);
    assert_eq!(on_page.body["code"], "file_attached");
    let delete = env
        .send(&env.owner, "DELETE", &format!("/v1/files/{in_page}"), None)
        .await;
    assert_eq!(delete.status, StatusCode::CONFLICT, "{}", delete.body);
}

/// A GET with a `Range` header.
async fn ranged(env: &Env, who: &Person, path: &str, range: &str) -> Reply {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("x-penfii-client", "1")
        .header("cookie", &who.cookie)
        .header("range", range)
        .body(Body::empty())
        .unwrap();
    common::send(&env.app, request).await
}

fn header<'a>(reply: &'a Reply, name: &str) -> &'a str {
    reply
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_ticket_is_created_with_its_files_and_media_is_served_in_ranges(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;

    // Files named when the Ticket is created are its body attachments, in the same request.
    let mut video = b"\0\0\0\x18ftypisom\0\0\x02\0".to_vec();
    video.extend((0..200u8).collect::<Vec<_>>());
    let clip = env.upload(&editor, "clip.mp4", &video).await;
    let page = env
        .upload(&editor, "page.html", b"<script>alert(1)</script>")
        .await;
    let created = env
        .send(
            &editor,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "with files", "hierarchyKind": "TASK", "fileIds": [clip, page]})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let ticket = created.body["id"].as_str().unwrap().to_string();
    let listed = env
        .send(
            &editor,
            "GET",
            &format!("/v1/tickets/{ticket}/attachments"),
            None,
        )
        .await;
    let items = listed.body.as_array().unwrap();
    assert_eq!(items.len(), 2, "{}", listed.body);
    let by_name = |name: &str| items.iter().find(|i| i["filename"] == name).unwrap();
    assert_eq!(by_name("clip.mp4")["mime"], "video/mp4");
    assert_eq!(by_name("page.html")["mime"], "application/octet-stream");

    // A file somebody else already bound cannot be taken again.
    let again = env
        .send(
            &editor,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "second", "hierarchyKind": "TASK", "fileIds": [clip]})),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.body);

    // Video is shown in place, active content is a download, and neither can run anything.
    let clip_url = format!(
        "/v1/attachments/{}/content",
        by_name("clip.mp4")["id"].as_str().unwrap()
    );
    let page_url = format!(
        "/v1/attachments/{}/content",
        by_name("page.html")["id"].as_str().unwrap()
    );
    let whole = env.send(&editor, "GET", &clip_url, None).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(whole.bytes, video);
    assert_eq!(header(&whole, "accept-ranges"), "bytes");
    assert!(header(&whole, "content-disposition").starts_with("inline"));
    assert_eq!(header(&whole, "content-type"), "video/mp4");
    let page_reply = env.send(&editor, "GET", &page_url, None).await;
    assert!(header(&page_reply, "content-disposition").starts_with("attachment"));
    assert_eq!(
        header(&page_reply, "content-type"),
        "application/octet-stream"
    );
    assert!(header(&page_reply, "content-security-policy").contains("sandbox"));
    assert_eq!(header(&page_reply, "x-content-type-options"), "nosniff");

    // Ranges: a slice, an open end, the last bytes, and one past the end.
    let size = video.len();
    let slice = ranged(&env, &editor, &clip_url, "bytes=4-19").await;
    assert_eq!(slice.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(slice.bytes, video[4..20]);
    assert_eq!(
        header(&slice, "content-range"),
        format!("bytes 4-19/{size}")
    );
    assert_eq!(header(&slice, "content-length"), "16");
    let tail = ranged(&env, &editor, &clip_url, "bytes=100-").await;
    assert_eq!(tail.bytes, video[100..]);
    let last = ranged(&env, &editor, &clip_url, "bytes=-10").await;
    assert_eq!(last.bytes, video[size - 10..]);
    let clamped = ranged(&env, &editor, &clip_url, &format!("bytes=10-{}", size * 4)).await;
    assert_eq!(clamped.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(clamped.bytes, video[10..]);
    let beyond = ranged(&env, &editor, &clip_url, &format!("bytes={size}-")).await;
    assert_eq!(beyond.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(header(&beyond, "content-range"), format!("bytes */{size}"));

    // Somebody who cannot open the Ticket gets nothing, ranged or not.
    let outsider = env.outsider().await;
    let denied = ranged(&env, &outsider, &clip_url, "bytes=0-3").await;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn doc_comments_carry_files_that_follow_the_page_and_the_comment(pool: PgPool) {
    let env = setup(pool).await;
    let reader = env.person("reader", "MEMBER").await;
    let outsider = env.outsider().await;
    let page = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(json!({"title": "Notes", "visibility": "RESTRICTED", "content":
                {"schema":"penfii.doc","version":1,"blocks":[
                    {"id":"b1","type":"paragraph","props":{},"content":[{"type":"text","text":"x","styles":{}}],"children":[]}]}})),
        )
        .await;
    assert_eq!(page.status, StatusCode::CREATED, "{}", page.body);
    let page = page.body["id"].as_str().unwrap().to_string();
    let comments = format!("/v1/docs/{page}/comments");

    // Files alone make a comment; text alone still has to say something.
    let shot = env
        .upload(&env.owner, "shot.png", b"\x89PNG\r\n\x1a\n")
        .await;
    let sent = env
        .send(
            &env.owner,
            "POST",
            &comments,
            Some(json!({"body": {"type":"doc","content":[]}, "fileIds": [shot]})),
        )
        .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    assert_eq!(sent.body["attachments"][0]["filename"], "shot.png");
    assert_eq!(sent.body["attachments"][0]["mime"], "image/png");
    let empty = env
        .send(
            &env.owner,
            "POST",
            &comments,
            Some(json!({"body": {"type":"doc","content":[]}})),
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.body);

    // The file is bound to this comment only.
    let twice = env
        .send(
            &env.owner,
            "POST",
            &comments,
            Some(json!({"body": doc("again"), "fileIds": [shot]})),
        )
        .await;
    assert_eq!(twice.status, StatusCode::CONFLICT, "{}", twice.body);
    let listed = env.send(&env.owner, "GET", &comments, None).await;
    assert_eq!(
        listed.body["items"][0]["attachments"][0]["id"],
        sent.body["attachments"][0]["id"]
    );

    // Whoever cannot read the page cannot open the file; the author can.
    let url = format!(
        "/v1/attachments/{}/content",
        sent.body["attachments"][0]["id"].as_str().unwrap()
    );
    assert_eq!(
        env.send(&env.owner, "GET", &url, None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        env.send(&reader, "GET", &url, None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&outsider, "GET", &url, None).await.status,
        StatusCode::NOT_FOUND
    );

    // Deleting the comment retires the file and removes its bytes.
    assert!(env.has_object(shot).await);
    let comment = sent.body["id"].as_str().unwrap();
    let removed = env
        .send(&env.owner, "DELETE", &format!("{comments}/{comment}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(env.file_row(shot).await.0);
    assert!(!env.has_object(shot).await);
    assert_eq!(
        env.send(&env.owner, "GET", &url, None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn uploads_nobody_attached_are_swept_and_attached_ones_stay(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let ticket = env.ticket(&project, "keeps files").await;
    let stray = env.upload(&editor, "stray.txt", b"nobody sends this").await;
    let kept = env.upload(&editor, "kept.txt", b"attached").await;
    let attached = env
        .send(
            &editor,
            "POST",
            &format!("/v1/tickets/{ticket}/attachments"),
            Some(json!({"fileId": kept})),
        )
        .await;
    assert_eq!(attached.status, StatusCode::CREATED);

    // Too recent: left alone, so a form that is still open keeps its files.
    assert_eq!(
        penfii_core::files::sweep_unbound(&env.pool, Some(&env.objects), 3600)
            .await
            .unwrap(),
        0
    );
    assert!(!env.file_row(stray).await.0);

    sqlx::query("UPDATE penfii.files SET created_at = now() - interval '2 days'")
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        penfii_core::files::sweep_unbound(&env.pool, Some(&env.objects), 3600)
            .await
            .unwrap(),
        1
    );
    assert!(env.file_row(stray).await.0);
    assert!(!env.has_object(stray).await);
    assert!(!env.file_row(kept).await.0);
    assert!(env.has_object(kept).await);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_description_or_comment_body_that_is_not_a_document_is_refused(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let tickets = format!("/v1/projects/{project}/tickets");

    // `openapi.json` says object: a string, a number or a list would be stored but never shown.
    for bad in [json!("plain text"), json!(7), json!(["a"])] {
        let created = env
            .send(
                &editor,
                "POST",
                &tickets,
                Some(json!({"title": "bad", "hierarchyKind": "TASK", "description": bad})),
            )
            .await;
        assert_eq!(
            created.status,
            StatusCode::BAD_REQUEST,
            "{bad}: {}",
            created.body
        );
    }
    let listed = env.send(&editor, "GET", &tickets, None).await;
    assert_eq!(
        listed.body.as_array().map_or(0, Vec::len),
        0,
        "nothing was stored"
    );
    for ok in [json!({"type": "doc", "content": []}), json!(null)] {
        let created = env
            .send(
                &editor,
                "POST",
                &tickets,
                Some(json!({"title": "ok", "hierarchyKind": "TASK", "description": ok})),
            )
            .await;
        assert_eq!(
            created.status,
            StatusCode::CREATED,
            "{ok}: {}",
            created.body
        );
    }
    let omitted = env
        .send(
            &editor,
            "POST",
            &tickets,
            Some(json!({"title": "no description", "hierarchyKind": "TASK"})),
        )
        .await;
    assert_eq!(omitted.status, StatusCode::CREATED, "{}", omitted.body);

    let ticket = env.ticket(&project, "edited").await;
    let path = format!("/v1/tickets/{ticket}");
    let revision = env.send(&editor, "GET", &path, None).await.body["revisionNo"].clone();
    // A Ticket edit needs an Idempotency-Key, which `send` does not add.
    let patch = |key: &'static str, body: Value| {
        common::send(
            &env.app,
            common::request("PATCH", &path, Some(&editor.cookie), Some(key), Some(body)),
        )
    };
    let refused = patch(
        "refused-edit",
        json!({"expectedRevision": revision, "description": "plain text"}),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    let unchanged = env.send(&editor, "GET", &path, None).await;
    assert_eq!(
        unchanged.body["revisionNo"], revision,
        "a refused edit is no revision"
    );
    let accepted = patch(
        "accepted-edit",
        json!({"expectedRevision": revision, "description": doc("fine")}),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);

    let comments = format!("{path}/comments");
    for bad in [json!("plain text"), json!(null), json!(["a"])] {
        let refused = env
            .send(&editor, "POST", &comments, Some(json!({"body": bad})))
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{bad}: {}",
            refused.body
        );
    }
    let comment = env
        .send(
            &editor,
            "POST",
            &comments,
            Some(json!({"body": doc("hello")})),
        )
        .await;
    assert_eq!(comment.status, StatusCode::CREATED, "{}", comment.body);
    let edited = env
        .send(
            &editor,
            "PATCH",
            &format!("{comments}/{}", comment.body["id"].as_str().unwrap()),
            Some(json!({"body": "plain text"})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::BAD_REQUEST, "{}", edited.body);
    let list = env.send(&editor, "GET", &comments, None).await;
    assert_eq!(list.body.as_array().map_or(0, Vec::len), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_start_of_an_upload_refuses_fields_the_api_does_not_describe(pool: PgPool) {
    let env = setup(pool).await;
    let path = format!("/v1/workspaces/{}/files", env.ws);
    let extra = env
        .send(
            &env.owner,
            "POST",
            &path,
            Some(json!({"filename": "a.txt", "byteSize": 5, "evil": true})),
        )
        .await;
    assert_eq!(extra.status, StatusCode::BAD_REQUEST, "{}", extra.body);
    let plain = env
        .send(
            &env.owner,
            "POST",
            &path,
            Some(json!({"filename": "a.txt", "byteSize": 5})),
        )
        .await;
    assert_eq!(plain.status, StatusCode::CREATED, "{}", plain.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_comment_tells_the_projects_open_screens_to_look_again(pool: PgPool) {
    use penfii_core::ChangeNotice;

    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let project_id: Uuid = project.parse().unwrap();
    let ticket = env.ticket(&project, "watched").await;

    let kind_and_seq = |notice: ChangeNotice| match notice {
        ChangeNotice::Ticket {
            project_id: notified,
            ticket_id,
            change_kind,
            change_seq,
            ..
        } => {
            assert_eq!(notified, project_id);
            assert_eq!(ticket_id, ticket);
            (change_kind, change_seq)
        }
        other => panic!("not a Ticket notice: {other:?}"),
    };

    let (comment, created) =
        penfii_core::work::create_comment(&env.pool, editor.id, ticket, doc("one"), &[])
            .await
            .unwrap();
    let (kind, first) = kind_and_seq(created);
    assert_eq!(kind, "ticket.commented");
    let (_, edited) = penfii_core::work::edit_comment(
        &env.pool,
        editor.id,
        ticket,
        comment.id,
        doc("one, edited"),
    )
    .await
    .unwrap();
    let (kind, second) = kind_and_seq(edited);
    assert_eq!(kind, "ticket.comment_edited");
    let removed = penfii_core::work::delete_comment(
        &env.pool,
        Some(&env.objects),
        editor.id,
        ticket,
        comment.id,
    )
    .await
    .unwrap();
    let (kind, third) = kind_and_seq(removed);
    assert_eq!(kind, "ticket.comment_deleted");
    assert!(first < second && second < third, "{first} {second} {third}");

    // The change feed holds them, and the Ticket revision did not move.
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.project_ticket_changes
         WHERE project_id = $1 AND ticket_id = $2 AND change_kind LIKE 'ticket.comment%'",
    )
    .bind(project_id)
    .bind(ticket)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows, 3);
    let revision = env
        .send(&editor, "GET", &format!("/v1/tickets/{ticket}"), None)
        .await;
    assert_eq!(revision.body["revisionNo"], 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_file_on_the_body_tells_the_projects_open_screens_to_look_again(pool: PgPool) {
    use penfii_core::ChangeNotice;

    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let project = project_with_editor(&env, &editor).await;
    let project_id: Uuid = project.parse().unwrap();
    let ticket = env.ticket(&project, "watched").await;
    let kind_and_seq = |notice: ChangeNotice| match notice {
        ChangeNotice::Ticket {
            project_id: notified,
            ticket_id,
            change_kind,
            change_seq,
            ..
        } => {
            assert_eq!(notified, project_id);
            assert_eq!(ticket_id, ticket);
            (change_kind, change_seq)
        }
        other => panic!("not a Ticket notice: {other:?}"),
    };

    let file = env.upload(&editor, "notes.txt", b"plain notes").await;
    let (attached, change) =
        penfii_core::ticket_files::attach_body(&env.pool, editor.id, ticket, file)
            .await
            .unwrap();
    let (kind, first) = kind_and_seq(change.expect("a new file is news"));
    assert_eq!(kind, "ticket.file_attached");
    // Repeating the call binds nothing new, so it says nothing.
    let (_, repeated) = penfii_core::ticket_files::attach_body(&env.pool, editor.id, ticket, file)
        .await
        .unwrap();
    assert!(repeated.is_none());
    let removed = penfii_core::ticket_files::detach_body(
        &env.pool,
        Some(&env.objects),
        editor.id,
        ticket,
        attached.id,
    )
    .await
    .unwrap();
    let (kind, second) = kind_and_seq(removed);
    assert_eq!(kind, "ticket.file_detached");
    assert!(first < second, "{first} {second}");

    // The change feed holds both, and the Ticket revision did not move.
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.project_ticket_changes
         WHERE project_id = $1 AND ticket_id = $2 AND change_kind LIKE 'ticket.file_%'",
    )
    .bind(project_id)
    .bind(ticket)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows, 2);
    let revision = env
        .send(&editor, "GET", &format!("/v1/tickets/{ticket}"), None)
        .await;
    assert_eq!(revision.body["revisionNo"], 1);
}

/// Two requests send the same upload at once (a retry that overlaps the first try). Both are inside
/// the object write when the first finishes the file; the second must not delete the object the
/// first now stands on. Writes are held for a moment so the two really overlap.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_upload_sent_twice_at_once_keeps_its_bytes(pool: PgPool) {
    let env = setup_with(
        pool,
        ObjectStore::in_memory_slow_writes(std::time::Duration::from_millis(400)),
    )
    .await;
    let content = b"sent twice at the same time";
    let created = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/files", env.ws),
            Some(json!({"filename": "twice.txt", "byteSize": content.len()})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = created.body["id"].as_str().unwrap().to_string();
    let path = format!("/v1/files/{id}/content");
    let put = || {
        raw(
            &env.app,
            "PUT",
            &path,
            Some(&env.owner.cookie),
            None,
            Some(content.to_vec()),
        )
    };
    let (first, second) = tokio::join!(put(), put());

    let statuses = [first.status, second.status];
    assert!(
        statuses.contains(&StatusCode::OK) && statuses.contains(&StatusCode::CONFLICT),
        "one request finishes the upload and the other is told it is no longer current: {statuses:?}"
    );
    let file: Uuid = id.parse().unwrap();
    assert!(
        env.has_object(file).await,
        "a finished file keeps its bytes"
    );
}
