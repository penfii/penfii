//! Who may do what: the View / Collaborate / Manage answer of a mutation for a plain Workspace
//! member, a Project editor, a Project admin and a Workspace admin.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person};

/// The shared fixture, with the shapes this file reads: `(status, body)` pairs.
struct Env(common::Env);

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.0
    }
}

async fn setup(pool: PgPool) -> Env {
    Env(common::Env::new(pool).await)
}

impl Env {
    async fn send(
        &self,
        who: &Person,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let reply = common::call(&self.app, method, path, Some(&who.cookie), body).await;
        (reply.status, reply.body)
    }

    async fn ok(&self, who: &Person, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.send(who, method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }

    /// A Project made by the Workspace owner (only owners and admins create one); `creator` is then given
    /// the Project's admin role, the way a Project's first admin is named.
    async fn project(&self, creator: &Person, teams: &[&str]) -> String {
        let value = self
            .ok(
                &self.owner,
                "POST",
                &format!("/v1/workspaces/{}/projects", self.ws),
                Some(json!({"name": "Caps", "teamIds": teams})),
            )
            .await;
        let id = value["id"].as_str().unwrap().to_owned();
        if creator.id != self.owner.id {
            self.ok(
                &self.owner,
                "PUT",
                &format!("/v1/projects/{id}/grants/{}", creator.id),
                Some(json!({"role": "ADMIN"})),
            )
            .await;
        }
        // The Teams take part with the editor role; a link alone gives no one access.
        for team in teams {
            self.ok(
                &self.owner,
                "PUT",
                &format!("/v1/projects/{id}/teams/{team}/participation"),
                Some(json!({"role": "EDITOR"})),
            )
            .await;
        }
        id
    }

    async fn team_with(&self, members: &[&Person]) -> String {
        let team = self
            .ok(
                &self.owner,
                "POST",
                &format!("/v1/workspaces/{}/teams", self.ws),
                Some(json!({"name": "Dev", "members": [{"userId": self.owner.id, "role": "MANAGER"}]})),
            )
            .await;
        let id = team["id"].as_str().unwrap().to_owned();
        for member in members {
            self.ok(
                &self.owner,
                "PUT",
                &format!(
                    "/v1/workspaces/{}/teams/{id}/members/{}",
                    self.ws, member.id
                ),
                Some(json!({"role": "MEMBER"})),
            )
            .await;
        }
        id
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn channels_are_created_and_changed_by_the_role_that_is_shown_the_action(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("wadmin", "ADMIN").await;
    let member = env.person("plain", "MEMBER").await;
    let editor = env.person("editor", "MEMBER").await;
    let padmin = env.person("padmin", "MEMBER").await;
    let grouped = env.person("grouped", "MEMBER").await;
    let team = env.team_with(&[&editor]).await;
    // A plain member who makes a Project administers it.
    let project = env.project(&padmin, &[&team]).await;

    // The Workspace overview says who manages the Project, so a client offers its actions to them alone.
    let managing = |who: Person| {
        let env = &env;
        let project = project.clone();
        async move {
            let (_, space) = env
                .send(
                    &who,
                    "GET",
                    &format!("/v1/workspaces/{}/space", env.ws),
                    None,
                )
                .await;
            let found = space["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == project.as_str())
                .cloned()
                .unwrap();
            (
                found["participating"].as_bool(),
                found["canManage"].as_bool(),
            )
        }
    };
    assert_eq!(managing(padmin.clone()).await, (Some(true), Some(true)));
    assert_eq!(managing(editor.clone()).await, (Some(true), Some(false)));
    assert_eq!(managing(admin.clone()).await, (Some(false), Some(false)));

    let channels = format!("/v1/workspaces/{}/channels", env.ws);
    let global = json!({"name": "town-hall", "visibility": "PUBLIC"});
    let in_project = json!({"name": "build", "visibility": "PUBLIC", "projectId": project});
    // Global channels: Workspace owner/admin only.
    for (who, want) in [
        (&member, StatusCode::FORBIDDEN),
        (&editor, StatusCode::FORBIDDEN),
        (&padmin, StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            env.send(who, "POST", &channels, Some(global.clone()))
                .await
                .0,
            want
        );
    }
    assert_eq!(
        env.send(&admin, "POST", &channels, Some(global)).await.0,
        StatusCode::CREATED
    );
    // Project channels: the Project admin, not an editor, a stranger or a Workspace admin outside it.
    for who in [&member, &editor, &admin] {
        let (status, _) = env
            .send(who, "POST", &channels, Some(in_project.clone()))
            .await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "{status}"
        );
    }
    let (status, created) = env
        .send(&padmin, "POST", &channels, Some(in_project.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let channel = created["id"].as_str().unwrap().to_owned();

    // A Project ADMIN given through a group is the same answer as a direct one.
    let group = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.groups(id, workspace_id, name) VALUES($1,$2,'Leads')")
        .bind(group)
        .bind(env.ws)
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.group_members(group_id, user_id) VALUES($1,$2)")
        .bind(group)
        .bind(grouped.id)
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO penfii.project_group_grants(project_id, group_id, role) VALUES($1,$2,'ADMIN')",
    )
    .bind(project.parse::<Uuid>().unwrap())
    .bind(group)
    .execute(&env.pool)
    .await
    .unwrap();
    let second = json!({"name": "ops", "visibility": "PUBLIC", "projectId": project});
    assert_eq!(
        env.send(&grouped, "POST", &channels, Some(second)).await.0,
        StatusCode::CREATED
    );
    let path = format!("/v1/channels/{channel}");
    let change = json!({"description": "builds"});
    assert_eq!(
        env.send(&grouped, "PATCH", &path, Some(change.clone()))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        env.send(&editor, "PATCH", &path, Some(change.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&member, "PATCH", &path, Some(change)).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn labels_are_collaboration_for_a_project_editor_and_not_a_viewer(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let viewer = env.person("viewer", "MEMBER").await;
    let team = env.team_with(&[&editor]).await;
    let project = env.project(&env.owner, &[&team]).await;
    sqlx::query(
        "INSERT INTO penfii.project_user_grants(project_id, user_id, role) VALUES($1,$2,'VIEWER')",
    )
    .bind(project.parse::<Uuid>().unwrap())
    .bind(viewer.id)
    .execute(&env.pool)
    .await
    .unwrap();
    let labels = format!("/v1/projects/{project}/labels");
    let (status, _) = env
        .send(&editor, "POST", &labels, Some(json!({"name": "ux"})))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = env
        .send(&viewer, "POST", &labels, Some(json!({"name": "no"})))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_page_participant_can_be_limited_to_reading(pool: PgPool) {
    let env = setup(pool).await;
    let reader = env.person("reader", "MEMBER").await;
    let writer = env.person("writer", "MEMBER").await;
    let admin = env.person("wadmin", "ADMIN").await;
    let page = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(
                json!({"title": "Notes", "visibility": "WORKSPACE", "content": common_page("one")}),
            ),
        )
        .await;
    let id = page["id"].as_str().unwrap().to_owned();
    let other = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(
                json!({"title": "Other", "visibility": "WORKSPACE", "content": common_page("two")}),
            ),
        )
        .await;
    let other_id = other["id"].as_str().unwrap().to_owned();
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants": [
            {"userId": env.owner.id, "role": "MANAGER"},
            {"userId": reader.id, "role": "VIEWER"},
        ]})),
    )
    .await;

    let save = |title: &'static str| json!({"expectedRevisionNo": 1, "title": title, "content": common_page("changed")});
    let path = format!("/v1/docs/{id}");
    let content = format!("/v1/docs/{id}/content");
    // The viewer reads, and sees it as read-only, but writes nothing: content, title, versions, files.
    let (status, got) = env.send(&reader, "GET", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["rights"]["edit"], false);
    assert_eq!(got["rights"]["manage"], false);
    // The tree carries the same answer, so a client shows only the actions that will work.
    let (_, tree) = env
        .send(
            &reader,
            "GET",
            &format!("/v1/workspaces/{}/docs/tree", env.ws),
            None,
        )
        .await;
    let node = tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id.as_str())
        .unwrap();
    assert_eq!(node["rights"], json!({"edit": false, "manage": false}));
    assert_eq!(
        env.send(&reader, "PUT", &content, Some(save("x"))).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&reader, "PATCH", &path, Some(json!({"title": "x"})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(
            &reader,
            "POST",
            &format!("/v1/docs/{id}/versions"),
            Some(json!({"expectedRevisionNo": 1}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // Moving needs editing, and the viewer is refused it; reading and commenting stay.
    let move_to = |parent: &str| json!({"parentId": parent});
    assert_eq!(
        env.send(
            &reader,
            "POST",
            &format!("/v1/docs/{id}/move"),
            Some(move_to(&other_id))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // The co-editing socket is for people who may edit.
    assert!(
        !penfii_core::docs::collab::CollabHub::may_open(&env.pool, reader.id, id.parse().unwrap())
            .await
            .unwrap()
    );
    // Delete, sharing and participants are manage-only.
    assert_eq!(
        env.send(&reader, "DELETE", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(
            &reader,
            "PUT",
            &format!("/v1/docs/{id}/visibility"),
            Some(json!({"visibility": "WORKSPACE"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // A person with no row edits and moves a WORKSPACE page, and still cannot delete or share it.
    assert_eq!(
        env.send(&writer, "PUT", &content, Some(save("y"))).await.0,
        StatusCode::OK
    );
    assert_eq!(
        env.send(
            &writer,
            "POST",
            &format!("/v1/docs/{id}/move"),
            Some(move_to(&other_id))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        env.send(&writer, "DELETE", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    // A Workspace admin manages a WORKSPACE page, so it can also edit it.
    assert_eq!(
        env.send(&admin, "GET", &path, None).await.1["rights"]["manage"],
        true
    );
    // Another viewer role value is rejected; the stronger of two rows for one person wins.
    let (status, _) = env
        .send(
            &env.owner,
            "PUT",
            &format!("/v1/docs/{id}/participants"),
            Some(json!({"participants": [
                {"userId": env.owner.id, "role": "MANAGER"},
                {"userId": reader.id, "role": "VIEWER"},
                {"userId": reader.id, "role": "EDITOR"},
            ]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, got) = env.send(&reader, "GET", &path, None).await;
    assert_eq!(got["rights"]["edit"], true);
    let (status, _) = env
        .send(
            &env.owner,
            "PUT",
            &format!("/v1/docs/{id}/participants"),
            Some(json!({"participants": [{"userId": env.owner.id, "role": "WATCHER"}]})),
        )
        .await;
    assert_ne!(status, StatusCode::OK);
}

fn common_page(text: &str) -> Value {
    json!({"schema":"penfii.doc","version":1,"blocks":[
        {"id":"fixed-block","type":"paragraph","props":{},"content":[{"type":"text","text":text,"styles":{}}],"children":[]}]})
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn comments_need_only_the_right_to_read_the_page(pool: PgPool) {
    let env = setup(pool).await;
    let viewer = env.person("viewer", "MEMBER").await;
    let editor = env.person("editor", "MEMBER").await;
    let manager = env.person("manager", "MEMBER").await;
    let outsider = env.person("outsider", "MEMBER").await;
    let page = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(json!({"title": "Private", "visibility": "RESTRICTED", "content": common_page("one"),
                "participants": [{"userId": env.owner.id, "role": "MANAGER"}]})),
        )
        .await;
    let id = page["id"].as_str().unwrap().to_owned();
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants": [
            {"userId": env.owner.id, "role": "MANAGER"},
            {"userId": manager.id, "role": "MANAGER"},
            {"userId": editor.id, "role": "EDITOR"},
            {"userId": viewer.id, "role": "VIEWER"},
        ]})),
    )
    .await;
    let comments = format!("/v1/docs/{id}/comments");
    let body = |text: &str| json!({"body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]}});
    let mut ids = Vec::new();
    for (who, name) in [
        (&viewer, "viewer"),
        (&editor, "editor"),
        (&manager, "manager"),
    ] {
        let (status, made) = env.send(who, "POST", &comments, Some(body(name))).await;
        assert_eq!(status, StatusCode::CREATED, "{name}: {made}");
        assert_eq!(
            env.send(who, "GET", &comments, None).await.0,
            StatusCode::OK
        );
        ids.push(made["id"].as_str().unwrap().to_owned());
    }
    // Only a person who may read the page sees or writes comments.
    assert_eq!(
        env.send(&outsider, "POST", &comments, Some(body("x")))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&outsider, "GET", &comments, None).await.0,
        StatusCode::NOT_FOUND
    );
    // Delete: the author, or a MANAGER participant; an editor or viewer does not delete another's.
    let delete = |comment: &str| format!("/v1/docs/{id}/comments/{comment}");
    assert_eq!(
        env.send(&editor, "DELETE", &delete(&ids[0]), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&viewer, "DELETE", &delete(&ids[1]), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert!(
        env.send(&viewer, "DELETE", &delete(&ids[0]), None)
            .await
            .0
            .is_success()
    );
    assert!(
        env.send(&manager, "DELETE", &delete(&ids[1]), None)
            .await
            .0
            .is_success()
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_projects_calendar_and_watch_are_for_its_participants(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let stranger = env.person("stranger", "MEMBER").await;
    let team = env.team_with(&[&editor]).await;
    let project = env.project(&env.owner, &[&team]).await;
    for path in [
        format!("/v1/projects/{project}/calendar"),
        format!("/v1/projects/{project}/subscription"),
    ] {
        assert_eq!(
            env.send(&env.owner, "GET", &path, None).await.0,
            StatusCode::OK,
            "{path}"
        );
        assert_eq!(
            env.send(&editor, "GET", &path, None).await.0,
            StatusCode::OK,
            "{path}"
        );
        assert_eq!(
            env.send(&stranger, "GET", &path, None).await.0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let watch = format!("/v1/projects/{project}/subscription");
    assert_eq!(
        env.send(&editor, "PUT", &watch, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        env.send(&stranger, "PUT", &watch, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_page_is_shared_into_a_project_only_by_its_participants(pool: PgPool) {
    let env = setup(pool).await;
    let editor = env.person("editor", "MEMBER").await;
    let stranger = env.person("stranger", "MEMBER").await;
    let team = env.team_with(&[&editor]).await;
    let project = env.project(&env.owner, &[&team]).await;
    let docs = format!("/v1/workspaces/{}/docs", env.ws);
    let scoped = |title: &str| json!({"title": title, "visibility": "PROJECT", "scopeProjectId": project, "content": common_page("x")});
    assert_eq!(
        env.send(&editor, "POST", &docs, Some(scoped("mine")))
            .await
            .0,
        StatusCode::CREATED
    );
    assert_eq!(
        env.send(&stranger, "POST", &docs, Some(scoped("theirs")))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // Scoping an existing page of their own into the Project is refused the same way.
    let own = env
        .ok(
            &stranger,
            "POST",
            &docs,
            Some(json!({"title": "own", "visibility": "WORKSPACE", "content": common_page("y")})),
        )
        .await;
    let id = own["id"].as_str().unwrap();
    let share = json!({"visibility": "PROJECT", "scopeProjectId": project});
    assert_eq!(
        env.send(
            &stranger,
            "PUT",
            &format!("/v1/docs/{id}/visibility"),
            Some(share)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_ticket_comment_is_edited_by_its_author_and_deleted_by_a_manager(pool: PgPool) {
    let env = setup(pool).await;
    let author = env.person("author", "MEMBER").await;
    let other = env.person("other", "MEMBER").await;
    let viewer = env.person("viewer", "MEMBER").await;
    let stranger = env.person("stranger", "MEMBER").await;
    let team = env.team_with(&[&author, &other]).await;
    let project = env.project(&env.owner, &[&team]).await;
    sqlx::query(
        "INSERT INTO penfii.project_user_grants(project_id, user_id, role) VALUES($1,$2,'VIEWER')",
    )
    .bind(project.parse::<Uuid>().unwrap())
    .bind(viewer.id)
    .execute(&env.pool)
    .await
    .unwrap();
    let channel = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "build", "visibility": "PUBLIC", "projectId": project})),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let ticket = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "T", "hierarchyKind": "TASK", "channelId": channel})),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let comments = format!("/v1/tickets/{ticket}/comments");
    let text = |t: &str| json!({"body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": t}]}]}});
    let mut ids = Vec::new();
    for who in [&author, &viewer, &author, &author] {
        let made = env.ok(who, "POST", &comments, Some(text("hi"))).await;
        ids.push(made["id"].as_str().unwrap().to_owned());
    }
    let one = |id: &str| format!("{comments}/{id}");

    // The list says what each person may do with a comment, so a client offers just that.
    let rights = |who: Person, id: String| {
        let env = &env;
        let comments = comments.clone();
        async move {
            let listed = env.ok(&who, "GET", &comments, None).await;
            let found = listed
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id.as_str())
                .cloned()
                .unwrap();
            (
                found["canEdit"].as_bool().unwrap(),
                found["canDelete"].as_bool().unwrap(),
            )
        }
    };
    assert_eq!(rights(author.clone(), ids[0].clone()).await, (true, true));
    assert_eq!(rights(other.clone(), ids[0].clone()).await, (false, false));
    assert_eq!(rights(viewer.clone(), ids[0].clone()).await, (false, false));
    assert_eq!(
        rights(env.owner.clone(), ids[0].clone()).await,
        (false, true)
    );
    assert_eq!(rights(viewer.clone(), ids[1].clone()).await, (true, true));

    // Only the author edits, even a Project admin may not; a stranger does not see it.
    let edited = env
        .ok(&author, "PATCH", &one(&ids[0]), Some(text("changed")))
        .await;
    assert!(edited["editedAt"].is_string());
    assert!(edited["body"].to_string().contains("changed"));
    for (who, want) in [
        (&other, StatusCode::FORBIDDEN),
        (&env.owner, StatusCode::FORBIDDEN),
        (&viewer, StatusCode::FORBIDDEN),
        (&stranger, StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            env.send(who, "PATCH", &one(&ids[0]), Some(text("x")))
                .await
                .0,
            want
        );
    }
    // A Project viewer who wrote a comment edits and deletes it.
    env.ok(&viewer, "PATCH", &one(&ids[1]), Some(text("mine")))
        .await;
    assert_eq!(
        env.send(&viewer, "DELETE", &one(&ids[1]), None).await.0,
        StatusCode::NO_CONTENT
    );

    // Deleting someone else's comment: not an editor or a stranger, but the author, the Project
    // admin or a manager of the Ticket's channel.
    for (who, want) in [
        (&other, StatusCode::FORBIDDEN),
        (&viewer, StatusCode::FORBIDDEN),
        (&stranger, StatusCode::NOT_FOUND),
    ] {
        assert_eq!(env.send(who, "DELETE", &one(&ids[2]), None).await.0, want);
    }
    assert_eq!(
        env.send(&env.owner, "DELETE", &one(&ids[2]), None).await.0,
        StatusCode::NO_CONTENT
    );
    sqlx::query(
        "INSERT INTO penfii.channel_members(channel_id, user_id, role) VALUES($1,$2,'MANAGER')
         ON CONFLICT (channel_id, user_id) DO UPDATE SET role = 'MANAGER'",
    )
    .bind(channel.parse::<Uuid>().unwrap())
    .bind(other.id)
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        env.send(&other, "DELETE", &one(&ids[3]), None).await.0,
        StatusCode::NO_CONTENT
    );
    // A manager cannot edit someone else's words.
    assert_eq!(
        env.send(&other, "PATCH", &one(&ids[0]), Some(text("x")))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // Deleted comments are gone from the list and cannot be changed again.
    let listed = env.ok(&author, "GET", &comments, None).await;
    let left: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(left, [ids[0].as_str()]);
    assert_eq!(
        env.send(&env.owner, "DELETE", &one(&ids[2]), None).await.0,
        StatusCode::NOT_FOUND
    );

    // A trashed Ticket takes no comment changes.
    sqlx::query("UPDATE penfii.tickets SET trashed_at = now() WHERE id = $1")
        .bind(ticket.parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        env.send(&author, "PATCH", &one(&ids[0]), Some(text("late")))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&author, "DELETE", &one(&ids[0]), None).await.0,
        StatusCode::NOT_FOUND
    );
}

/// A Team link is an organizational relation. Only a Team's participation role lets its current members into
/// a Project, a direct grant is independent of every Team, and the highest role of the sources counts.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn team_derived_access_follows_team_membership_and_is_separate_from_direct_grants(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let wadmin = env.person("wadmin", "ADMIN").await;
    let alice = env.person("alice", "MEMBER").await;
    let bob = env.person("bob", "MEMBER").await;
    let carol = env.person("carol", "MEMBER").await;
    let team_a = env.team_with(&[&alice, &bob]).await;
    let team_b = env.team_with(&[&carol]).await;
    // Both Teams are linked; neither has a role in the Project.
    let project = env.project(&env.owner, &[&team_a, &team_b]).await;
    // `project()` gives a participation role to every linked Team; take it back to start from a bare link.
    for team in [&team_a, &team_b] {
        env.ok(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project}/teams/{team}/participation"),
            Some(json!({"role": null})),
        )
        .await;
    }
    let members = format!("/v1/projects/{project}/members");
    let ticket = |who: &Person| {
        let env = &env;
        let path = format!("/v1/projects/{project}/tickets");
        let who = who.clone();
        async move {
            env.send(
                &who,
                "POST",
                &path,
                Some(json!({"title": "T", "hierarchyKind": "TASK"})),
            )
            .await
            .0
        }
    };
    let listed = |who: &Person| {
        let env = &env;
        let project = project.clone();
        let who = who.clone();
        async move {
            let (_, space) = env
                .send(
                    &who,
                    "GET",
                    &format!("/v1/workspaces/{}/space", env.ws),
                    None,
                )
                .await;
            space["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == project.as_str())
                .map(|p| p["participating"].as_bool().unwrap())
        }
    };
    let team_path = |team: &str| format!("/v1/projects/{project}/teams/{team}/participation");

    // A link alone gives nothing; every Workspace member still finds the Project in the list.
    for who in [&alice, &bob, &carol, &wadmin] {
        assert_eq!(listed(who).await, Some(false));
        assert_eq!(
            env.send(who, "GET", &members, None).await.0,
            StatusCode::NOT_FOUND
        );
    }

    // Participation lets the Team's current members in with the role the Project gives the Team.
    env.ok(
        &env.owner,
        "PUT",
        &team_path(&team_a),
        Some(json!({"role": "VIEWER"})),
    )
    .await;
    assert_eq!(listed(&alice).await, Some(true));
    assert_eq!(
        env.send(&alice, "GET", &members, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        ticket(&alice).await,
        StatusCode::FORBIDDEN,
        "a viewer reads only"
    );
    assert_eq!(
        env.send(&carol, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND
    );

    // A direct grant is another source; the higher role of the two counts, and it stays when the Team goes.
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/grants/{}", alice.id),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    assert_eq!(ticket(&alice).await, StatusCode::CREATED);
    env.ok(
        &env.owner,
        "PUT",
        &team_path(&team_a),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    assert_eq!(ticket(&bob).await, StatusCode::CREATED, "Team editor");
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/grants/{}", alice.id),
        Some(json!({"role": "VIEWER"})),
    )
    .await;
    assert_eq!(
        ticket(&alice).await,
        StatusCode::CREATED,
        "the Team's editor role still counts"
    );

    // Removing someone from the Team ends what only the Team gave; their direct grant stays.
    let drop = |team: &str, who: &Person| {
        let env = &env;
        let path = format!("/v1/workspaces/{}/teams/{team}/members/{}", env.ws, who.id);
        async move { env.send(&env.owner, "DELETE", &path, None).await }
    };
    assert_eq!(drop(&team_a, &alice).await.0, StatusCode::NO_CONTENT);
    assert_eq!(drop(&team_a, &bob).await.0, StatusCode::NO_CONTENT);
    assert_eq!(listed(&bob).await, Some(false));
    assert_eq!(
        env.send(&bob, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        ticket(&alice).await,
        StatusCode::FORBIDDEN,
        "only the direct viewer role is left"
    );
    assert_eq!(
        env.send(&alice, "GET", &members, None).await.0,
        StatusCode::OK
    );

    // Moving to another Team carries nothing over: the new Team has to take part itself.
    env.ok(
        &env.owner,
        "PUT",
        &format!(
            "/v1/workspaces/{}/teams/{team_b}/members/{}",
            env.ws, bob.id
        ),
        Some(json!({"role": "MEMBER"})),
    )
    .await;
    assert_eq!(
        listed(&bob).await,
        Some(false),
        "Team B is linked but has no role"
    );
    env.ok(
        &env.owner,
        "PUT",
        &team_path(&team_b),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    assert_eq!(ticket(&bob).await, StatusCode::CREATED);
    assert_eq!(ticket(&carol).await, StatusCode::CREATED);
    // Taking the participation back, or unlinking the Team, ends it for everyone who had it only so.
    env.ok(
        &env.owner,
        "PUT",
        &team_path(&team_b),
        Some(json!({"role": null})),
    )
    .await;
    assert_eq!(
        env.send(&carol, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND
    );
    env.ok(
        &env.owner,
        "PUT",
        &team_path(&team_b),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    env.ok(
        &env.owner,
        "DELETE",
        &format!("/v1/projects/{project}/teams/{team_b}"),
        None,
    )
    .await;
    assert_eq!(
        env.send(&bob, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND
    );

    // Managing access is for the Project's admin. A Workspace admin who is not in the Project has no way in,
    // a participant who is not an admin is refused, and a Team has to be linked before it gets a role.
    let grant = format!("/v1/projects/{project}/grants/{}", carol.id);
    assert_eq!(
        env.send(&wadmin, "PUT", &grant, Some(json!({"role": "VIEWER"})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&alice, "PUT", &grant, Some(json!({"role": "VIEWER"})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(
            &alice,
            "PUT",
            &team_path(&team_b),
            Some(json!({"role": "EDITOR"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(
            &env.owner,
            "PUT",
            &team_path(&team_b),
            Some(json!({"role": "EDITOR"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        env.send(&env.owner, "PUT", &grant, Some(json!({"role": "BOSS"})))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        env.send(&env.owner, "PUT", &grant, Some(json!({"role": "VIEWER"})))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&carol, "GET", &members, None).await.0,
        StatusCode::OK
    );
    // A Project keeps an admin of its own.
    let own = format!("/v1/projects/{project}/grants/{}", env.owner.id);
    assert_eq!(
        env.send(&env.owner, "DELETE", &own, None).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.send(&env.owner, "PUT", &own, Some(json!({"role": "EDITOR"})))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.send(&env.owner, "DELETE", &grant, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&carol, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND
    );

    // Creating a Project is for owners and admins.
    let plain = format!("/v1/workspaces/{}/projects", env.ws);
    assert_eq!(
        env.send(
            &alice,
            "POST",
            &plain,
            Some(json!({"name": "Mine", "teamIds": []}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(
            &wadmin,
            "POST",
            &plain,
            Some(json!({"name": "Theirs", "teamIds": []}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
}

/// Inviting a whole Team follows the Team's membership; picking people out of it makes direct grants that
/// the Team no longer touches.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_whole_team_follows_membership_while_picked_members_keep_their_direct_grants(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let wadmin = env.person("wadmin", "ADMIN").await;
    let alice = env.person("alice", "MEMBER").await;
    let bob = env.person("bob", "MEMBER").await;
    let carol = env.person("carol", "MEMBER").await;
    let team = env.team_with(&[&alice, &bob]).await;
    let project = env.project(&env.owner, &[&team]).await;
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/teams/{team}/participation"),
        Some(json!({"role": null})),
    )
    .await;
    let members = format!("/v1/projects/{project}/members");
    let roster = format!("/v1/projects/{project}/teams/{team}/members");

    // The Project admin sees the linked Team's active members, by name; nobody else does.
    let (status, list) = env.send(&env.owner, "GET", &roster, None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["displayName"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 3, "{list}");
    assert!(
        list.to_string().contains(&alice.id.to_string()) && !list.to_string().contains("email")
    );
    for who in [&alice, &wadmin] {
        assert_eq!(
            env.send(who, "GET", &roster, None).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let unlinked = env.team_with(&[&carol]).await;
    assert_eq!(
        env.send(
            &env.owner,
            "GET",
            &format!("/v1/projects/{project}/teams/{unlinked}/members"),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "only a Team linked to the Project can be listed"
    );

    // Picked people: direct grants, independent of the Team afterwards.
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/grants/{}", alice.id),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    // The whole Team: its current and future members, while they stay.
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/teams/{team}/participation"),
        Some(json!({"role": "VIEWER"})),
    )
    .await;
    let team_path = |who: Uuid| format!("/v1/workspaces/{}/teams/{team}/members/{who}", env.ws);
    env.ok(
        &env.owner,
        "PUT",
        &team_path(carol.id),
        Some(json!({"role": "MEMBER"})),
    )
    .await;
    for who in [&alice, &bob, &carol] {
        assert_eq!(env.send(who, "GET", &members, None).await.0, StatusCode::OK);
    }
    // The Team role does not pick the Project role: the manager of the Team is a viewer like the others.
    let ticket = |who: &Person| {
        let (env, path, who) = (&env, format!("/v1/projects/{project}/tickets"), who.clone());
        async move {
            env.send(
                &who,
                "POST",
                &path,
                Some(json!({"title": "T", "hierarchyKind": "TASK"})),
            )
            .await
            .0
        }
    };
    assert_eq!(ticket(&bob).await, StatusCode::FORBIDDEN);
    assert_eq!(
        ticket(&alice).await,
        StatusCode::CREATED,
        "alice's direct editor role"
    );
    env.ok(
        &env.owner,
        "PUT",
        &team_path(bob.id),
        Some(json!({"role": "MANAGER"})),
    )
    .await;
    assert_eq!(
        ticket(&bob).await,
        StatusCode::FORBIDDEN,
        "MANAGER of the Team is not ADMIN or EDITOR of the Project"
    );
    // Everyone leaves the Team (a second manager first, so the first may go): only alice keeps access.
    for who in [&carol, &alice] {
        env.ok(&env.owner, "DELETE", &team_path(who.id), None).await;
    }
    assert_eq!(
        env.send(&carol, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&alice, "GET", &members, None).await.0,
        StatusCode::OK
    );
    assert_eq!(ticket(&alice).await, StatusCode::CREATED);
    // A later member of the Team gets the Team's access without anyone touching the Project.
    env.ok(
        &env.owner,
        "PUT",
        &team_path(carol.id),
        Some(json!({"role": "MEMBER"})),
    )
    .await;
    assert_eq!(
        env.send(&carol, "GET", &members, None).await.0,
        StatusCode::OK
    );
}

/// A Workspace owner or admin who creates a Project is its admin and may step down once someone else is;
/// the Workspace role never opens a Project.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_creator_may_step_down_and_a_workspace_role_opens_nothing(pool: PgPool) {
    let env = setup(pool).await;
    let wadmin = env.person("wadmin", "ADMIN").await;
    let alice = env.person("alice", "MEMBER").await;
    let created = env
        .ok(
            &wadmin,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Theirs", "teamIds": []})),
        )
        .await;
    let project = created["id"].as_str().unwrap().to_owned();
    let members = format!("/v1/projects/{project}/members");
    assert_eq!(
        env.send(&wadmin, "GET", &members, None).await.0,
        StatusCode::OK,
        "the creator starts as ADMIN"
    );
    assert_eq!(
        env.send(&env.owner, "GET", &members, None).await.0,
        StatusCode::NOT_FOUND,
        "the Workspace owner gets nothing"
    );
    let own = format!("/v1/projects/{project}/grants/{}", wadmin.id);
    assert_eq!(
        env.send(&wadmin, "DELETE", &own, None).await.0,
        StatusCode::CONFLICT,
        "still the only admin"
    );
    env.ok(
        &wadmin,
        "PUT",
        &format!("/v1/projects/{project}/grants/{}", alice.id),
        Some(json!({"role": "ADMIN"})),
    )
    .await;
    assert_eq!(
        env.send(&wadmin, "DELETE", &own, None).await.0,
        StatusCode::NO_CONTENT
    );
    for who in [&wadmin, &env.owner] {
        assert_eq!(
            env.send(who, "GET", &members, None).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        env.send(&alice, "GET", &members, None).await.0,
        StatusCode::OK
    );
    // The Project is still in the Workspace list for the one who stepped down.
    let (_, space) = env
        .send(
            &wadmin,
            "GET",
            &format!("/v1/workspaces/{}/space", env.ws),
            None,
        )
        .await;
    let row = space["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == project.as_str())
        .cloned()
        .unwrap();
    assert_eq!(row["participating"], false);
}

/// Group grants through a Team's `system_team_id` group are their own source: they neither need a Team link
/// nor are changed by linking, unlinking or giving the Team a role.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn group_grants_keep_working_beside_team_participation(pool: PgPool) {
    let env = setup(pool).await;
    let alice = env.person("alice", "MEMBER").await;
    let team = env.team_with(&[&alice]).await;
    let project = env.project(&env.owner, &[]).await;
    let (group, project_id): (Uuid, Uuid) = (Uuid::new_v4(), project.parse().unwrap());
    sqlx::query("INSERT INTO penfii.groups (id, workspace_id, name, system_team_id) VALUES ($1, $2, 'Team group', $3)")
        .bind(group).bind(env.ws).bind(team.parse::<Uuid>().unwrap()).execute(&env.pool).await.unwrap();
    sqlx::query("INSERT INTO penfii.project_group_grants (project_id, group_id, role) VALUES ($1, $2, 'VIEWER')")
        .bind(project_id).bind(group).execute(&env.pool).await.unwrap();
    let members = format!("/v1/projects/{project}/members");
    assert_eq!(
        env.send(&alice, "GET", &members, None).await.0,
        StatusCode::OK,
        "through the group, with no Team link"
    );
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/teams/{team}"),
        None,
    )
    .await;
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/teams/{team}/participation"),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    let tickets = format!("/v1/projects/{project}/tickets");
    let ticket = || {
        env.send(
            &alice,
            "POST",
            &tickets,
            Some(json!({"title": "T", "hierarchyKind": "TASK"})),
        )
    };
    assert_eq!(
        ticket().await.0,
        StatusCode::CREATED,
        "the Team's editor role and the group's viewer role: the higher counts"
    );
    env.ok(
        &env.owner,
        "DELETE",
        &format!("/v1/projects/{project}/teams/{team}"),
        None,
    )
    .await;
    assert_eq!(
        env.send(&alice, "GET", &members, None).await.0,
        StatusCode::OK,
        "the group grant is unchanged"
    );
    assert_eq!(
        ticket().await.0,
        StatusCode::FORBIDDEN,
        "back to the group's viewer role"
    );
}

/// Linking a Team opens nothing inside a Project: not its Tickets, comments, files, Channel, Docs, Labels,
/// activity or search results, for the Team's members or for a Workspace admin. A role opens them, and ending
/// the role closes them again.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_linked_team_without_a_role_sees_nothing_inside_the_project(pool: PgPool) {
    let env = setup(pool).await;
    let wadmin = env.person("wadmin", "ADMIN").await;
    let linked = env.person("linked", "MEMBER").await;
    let team = env.team_with(&[&linked]).await;
    let project = env.project(&env.owner, &[&team]).await;
    let participation = format!("/v1/projects/{project}/teams/{team}/participation");
    env.ok(
        &env.owner,
        "PUT",
        &participation,
        Some(json!({"role": null})),
    )
    .await;

    let (_, space) = env
        .send(
            &env.owner,
            "GET",
            &format!("/v1/workspaces/{}/space", env.ws),
            None,
        )
        .await;
    let channel = space["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["projectId"] == project.as_str())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    env.ok(&env.owner, "POST", &format!("/v1/conversations/{channel}/messages"), Some(json!({"body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "zebrafinch"}]}]}, "bodySchemaVersion": 1, "threadRootId": null, "replyToId": null}))).await;
    let ticket = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "zebrafinch ticket", "hierarchyKind": "TASK"})),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let doc = env.ok(&env.owner, "POST", &format!("/v1/workspaces/{}/docs", env.ws), Some(json!({"title": "zebrafinch doc", "visibility": "PROJECT", "scopeProjectId": project, "content": common_page("zebrafinch")}))).await["id"].as_str().unwrap().to_owned();
    let paths = [
        format!("/v1/projects/{project}/board"),
        format!("/v1/projects/{project}/members"),
        format!("/v1/projects/{project}/labels"),
        format!("/v1/projects/{project}/activity"),
        format!("/v1/projects/{project}/permissions"),
        format!("/v1/tickets/{ticket}"),
        format!("/v1/tickets/{ticket}/comments"),
        format!("/v1/tickets/{ticket}/attachments"),
        format!("/v1/conversations/{channel}/messages"),
        format!("/v1/docs/{doc}"),
    ];
    let closed = |who: &Person| {
        // the paths that answer anything but 404
        let (env, who, paths) = (&env, who.clone(), paths.clone());
        async move {
            let mut open = Vec::new();
            for path in &paths {
                let (status, _) = env.send(&who, "GET", path, None).await;
                if status != StatusCode::NOT_FOUND {
                    open.push(format!("{path}: {status}"));
                }
            }
            open
        }
    };
    // Positive control: the Project's admin reaches every one of them.
    assert_eq!(closed(&env.owner).await.len(), paths.len());
    for who in [&linked, &wadmin] {
        assert_eq!(
            closed(who).await,
            Vec::<String>::new(),
            "linked or Workspace admin: everything inside is 404"
        );
        let (_, space) = env
            .send(
                who,
                "GET",
                &format!("/v1/workspaces/{}/space", env.ws),
                None,
            )
            .await;
        assert!(
            space["channels"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["projectId"] != project.as_str()),
            "the Project's Channel is not listed"
        );
        let row = space["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == project.as_str())
            .cloned()
            .unwrap();
        assert_eq!(
            row["participating"], false,
            "the Project itself is in the list"
        );
    }
    // A role opens it to the Team's member only (the permissions page stays for the admin: 403, not 404).
    env.ok(
        &env.owner,
        "PUT",
        &participation,
        Some(json!({"role": "VIEWER"})),
    )
    .await;
    let reached = closed(&linked).await;
    assert_eq!(reached.len(), paths.len(), "{reached:?}");
    assert!(
        reached
            .iter()
            .all(|line| line.ends_with("200 OK") || line.contains("/permissions: 403")),
        "{reached:?}"
    );
    assert_eq!(closed(&wadmin).await, Vec::<String>::new());
    // Ending the role closes it again.
    env.ok(
        &env.owner,
        "PUT",
        &participation,
        Some(json!({"role": null})),
    )
    .await;
    assert_eq!(closed(&linked).await, Vec::<String>::new());
}

/// One role per person comes from the highest of their sources (a direct grant, a Group, a Team's
/// participation), and a direct grant stored with the legacy name `OWNER` is an `ADMIN`.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_highest_source_gives_the_project_role_and_a_legacy_owner_grant_is_an_admin(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let legacy = env.person("legacy", "MEMBER").await;
    let mixed = env.person("mixed", "MEMBER").await;
    let team_editor = env.person("teameditor", "MEMBER").await;
    let team = env.team_with(&[&team_editor]).await;
    let project = env.project(&env.owner, &[&team]).await;
    let project_id = project.parse::<Uuid>().unwrap();
    let grant = |user: Uuid, role: &'static str| {
        let pool = env.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO penfii.project_user_grants(project_id, user_id, role) VALUES($1,$2,$3)",
            )
            .bind(project_id)
            .bind(user)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    grant(legacy.id, "OWNER").await;
    // A viewer by direct grant who is also in a Group that edits.
    grant(mixed.id, "VIEWER").await;
    let group = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.groups(id, workspace_id, name) VALUES($1,$2,'Editors')")
        .bind(group)
        .bind(env.ws)
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.group_members(group_id, user_id) VALUES($1,$2)")
        .bind(group)
        .bind(mixed.id)
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO penfii.project_group_grants(project_id, group_id, role) VALUES($1,$2,'EDITOR')",
    )
    .bind(project_id)
    .bind(group)
    .execute(&env.pool)
    .await
    .unwrap();

    let permissions = format!("/v1/projects/{project}/permissions");
    let labels = format!("/v1/projects/{project}/labels");
    // OWNER: the Project admin's own page, and the roster shows the role as ADMIN.
    let (status, page) = env.send(&legacy, "GET", &permissions, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let roster = page["members"].as_array().unwrap();
    let row = roster
        .iter()
        .find(|member| member["userId"] == json!(legacy.id))
        .expect("listed");
    assert_eq!(row["role"], "ADMIN");
    // Viewer + Group editor: edits, does not administer.
    let (status, _) = env
        .send(&mixed, "POST", &labels, Some(json!({"name": "mixed"})))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = env.send(&mixed, "GET", &permissions, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // A Team's participation is an editor too.
    let (status, _) = env
        .send(&team_editor, "POST", &labels, Some(json!({"name": "team"})))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = env.send(&team_editor, "GET", &permissions, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
