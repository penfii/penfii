//! HTTP API integration tests: the whole router against a database of each test's own (and
//! OpenSearch for search).

mod access;
mod collab;
mod docs;
mod estimates;
mod events;
mod inbox;
mod messages;
mod openapi;
mod personal;
mod planning;
mod realtime;
mod reports;
mod search;
mod tickets;

use search::SearchWorld;

use crate::common::{self, OWNER_EMAIL, OWNER_PASSWORD, doc};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

/// A fresh database with the bootstrap owner signed in.
pub(crate) struct World {
    pub(crate) pool: PgPool,
    pub(crate) app: axum::Router,
    pub(crate) owner_cookie: String,
    pub(crate) owner_id: Uuid,
    /// The bootstrap Workspace.
    pub(crate) workspace_id: Uuid,
    search: tokio::sync::OnceCell<Option<std::sync::Arc<SearchWorld>>>,
}

impl World {
    pub(crate) async fn new(pool: PgPool) -> World {
        let boot = common::setup(&pool).await;
        let app = penfii_http_api::router(pool.clone(), common::limits(), false, None);
        let owner_cookie = common::sign_in(&app, OWNER_EMAIL, OWNER_PASSWORD).await;
        World {
            pool,
            app,
            owner_cookie,
            owner_id: boot.user_id,
            workspace_id: boot.workspace_id,
            search: Default::default(),
        }
    }
}

pub(crate) async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    idempotency_key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    common::send(
        app,
        common::request(method, path, cookie, idempotency_key, body),
    )
    .await
    .parts()
}

/// An extra Workspace owned by the owner.
async fn workspace(world: &World, name: &str) -> Uuid {
    common::add_workspace(&world.pool, world.owner_id, name).await
}

async fn channel(world: &World, workspace_id: Uuid, name: &str, visibility: &str) -> Uuid {
    let (status, body, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/channels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({
            "name": name,
            "visibility": visibility,
            "teamId": null,
            "projectId": null
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().parse().unwrap()
}

async fn send(
    world: &World,
    cookie: &str,
    conversation_id: Uuid,
    text: &str,
    thread_root_id: Option<Uuid>,
    reply_to_id: Option<Uuid>,
) -> (StatusCode, Value) {
    let (status, body, _) = call(
        &world.app,
        "POST",
        &format!("/v1/conversations/{conversation_id}/messages"),
        Some(cookie),
        Some(&Uuid::new_v4().to_string()),
        Some(json!({
            "bodySchemaVersion": 1,
            "body": doc(text),
            "threadRootId": thread_root_id,
            "replyToId": reply_to_id
        })),
    )
    .await;
    (status, body)
}

async fn invite_member(world: &World, workspace_id: Uuid, role: &str) -> (Uuid, String) {
    let email = format!("{role}-{}@penfii.test", Uuid::new_v4());
    let password = "invite-member-password";
    let (status, body, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invitations"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"email": email, "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let token = body["inviteToken"]
        .as_str()
        .expect("invite token")
        .to_string();
    let (status, accepted, _) = call(
        &world.app,
        "POST",
        "/v1/invitations/accept",
        None,
        None,
        Some(json!({
            "token": token,
            "displayName": "Invited",
            "password": password
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    let user_id = accepted["user"]["id"].as_str().unwrap().parse().unwrap();
    let (status, _, cookie) = call(
        &world.app,
        "POST",
        "/v1/session/login",
        None,
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (user_id, cookie.expect("member cookie"))
}

async fn viewer_cookie(world: &World, workspace_id: Uuid, email: &str) -> String {
    let (status, invitation, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invitations"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"email":email,"role":"MEMBER"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invitation}");
    let (status, _, _) = call(&world.app,"POST","/v1/invitations/accept",None,None,Some(json!({"token":invitation["inviteToken"],"displayName":"Viewer Two","password":"viewer-two-password"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, cookie) = call(
        &world.app,
        "POST",
        "/v1/session/login",
        None,
        None,
        Some(json!({"email":email,"password":"viewer-two-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    cookie.unwrap()
}

pub(crate) async fn new_project(world: &World, workspace_id: Uuid, name: &str) -> (String, Value) {
    let (status, project, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":name,"teamIds":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let id = project["id"].as_str().unwrap().to_owned();
    let (status, workflow, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{id}/workflow"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflow}");
    (id, workflow)
}

async fn make_ticket(
    world: &World,
    project_id: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, value, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project_id}/tickets"),
        Some(&world.owner_cookie),
        Some(key),
        Some(body),
    )
    .await;
    (status, value)
}

// Work
async fn grant(world: &World, project_id: &str, user: Uuid, role: &str) {
    sqlx::query("INSERT INTO penfii.project_user_grants(project_id,user_id,role) VALUES($1,$2,$3) ON CONFLICT (project_id,user_id) DO UPDATE SET role=EXCLUDED.role")
        .bind(Uuid::parse_str(project_id).unwrap())
        .bind(user)
        .bind(role)
        .execute(&world.pool)
        .await
        .unwrap();
}

async fn user_id_of(world: &World, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM penfii.users WHERE email=$1")
        .bind(email)
        .fetch_one(&world.pool)
        .await
        .unwrap()
}

pub(crate) async fn post_as(
    world: &World,
    cookie: &str,
    path: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, value, _) = call(
        &world.app,
        "POST",
        path,
        Some(cookie),
        Some(key),
        Some(body),
    )
    .await;
    (status, value)
}

pub(crate) async fn get_as(world: &World, cookie: &str, path: &str) -> (StatusCode, Value) {
    let (status, value, _) = call(&world.app, "GET", path, Some(cookie), None, None).await;
    (status, value)
}
