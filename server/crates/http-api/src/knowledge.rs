//! Docs (and later Databases) over HTTP. The handlers only translate JSON to core commands; every
//! rule lives in `penfii_core::docs` and `penfii_core::visibility`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_core::docs;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Body, Q, present_or_null};

use axum::Router;
use axum::routing::{delete, get, patch, post, put};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/docs/tree", get(doc_tree))
        .route("/v1/workspaces/{id}/docs", post(create_doc))
        .route(
            "/v1/docs/{id}",
            get(get_doc).patch(patch_doc).delete(delete_doc),
        )
        .route("/v1/docs/{id}/content", put(save_doc))
        .route(
            "/v1/docs/{id}/versions",
            get(list_doc_versions).post(create_doc_version),
        )
        .route("/v1/docs/{id}/versions/{versionNo}", get(get_doc_version))
        .route("/v1/docs/{id}/move", post(move_doc))
        .route("/v1/docs/{id}/visibility", put(set_doc_visibility))
        .route(
            "/v1/docs/{id}/participants",
            get(doc_participants).put(set_doc_participants),
        )
        .route(
            "/v1/docs/{id}/comments",
            get(list_doc_comments).post(create_doc_comment),
        )
        .route(
            "/v1/docs/{id}/comments/{commentId}",
            patch(edit_doc_comment).delete(delete_doc_comment),
        )
        .route("/v1/docs/{id}/files", post(attach_doc_file))
        .route(
            "/v1/workspaces/{id}/doc-templates",
            get(list_doc_templates).post(create_doc_template),
        )
        .route(
            "/v1/workspaces/{id}/doc-templates/{templateId}",
            delete(delete_doc_template),
        )
        .route(
            "/v1/workspaces/{id}/references/resolve",
            post(resolve_references),
        )
        .route("/v1/tickets/{id}/references", get(ticket_references))
        .route("/v1/docs/{id}/references", get(doc_references))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeQuery {
    project_id: Option<Uuid>,
}

async fn doc_tree(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Q(query): Q<TreeQuery>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(docs::tree(&state.pool, user, workspace_id, query.project_id).await?).into_response())
}

#[derive(serde::Deserialize)]
struct ParticipantBody {
    #[serde(rename = "userId")]
    user_id: Uuid,
    role: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateDocBody {
    title: String,
    parent_id: Option<Uuid>,
    project_id: Option<Uuid>,
    visibility: String,
    scope_project_id: Option<Uuid>,
    scope_team_id: Option<Uuid>,
    #[serde(default)]
    participants: Vec<ParticipantBody>,
    template_id: Option<Uuid>,
    content: Option<Value>,
}

async fn create_doc(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<CreateDocBody>,
) -> Result<Response, ApiError> {
    let page = docs::create(
        &state.pool,
        &state.knowledge,
        user,
        workspace_id,
        docs::NewPage {
            title: body.title,
            parent_id: body.parent_id,
            project_id: body.project_id,
            visibility: body.visibility,
            scope_project_id: body.scope_project_id,
            scope_team_id: body.scope_team_id,
            participants: body
                .participants
                .into_iter()
                .map(|p| (p.user_id, p.role))
                .collect(),
            template_id: body.template_id,
            content: body.content,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(page)).into_response())
}

async fn get_doc(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    let live = state.collab.as_ref().is_some_and(|hub| hub.is_open(id));
    Ok(Json(docs::get(&state.pool, user, id, live).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchDocBody {
    title: Option<String>,
    #[serde(default, deserialize_with = "present_or_null")]
    project_id: Option<Option<Uuid>>,
}

async fn patch_doc(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<PatchDocBody>,
) -> Result<Response, ApiError> {
    let page = docs::patch(&state.pool, user, id, body.title, body.project_id).await?;
    if let Some(hub) = &state.collab {
        hub.page_meta_changed(id, page["title"].as_str().unwrap_or(""));
    }
    Ok(Json(page).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveDocBody {
    expected_revision_no: i64,
    title: String,
    content: Value,
}

/// Save: replaces the title and content with the person's draft if `expectedRevisionNo` is current.
async fn save_doc(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<SaveDocBody>,
) -> Result<Response, ApiError> {
    let page = docs::history::save(
        &state.pool,
        &state.knowledge,
        user,
        id,
        body.expected_revision_no,
        &body.title,
        &body.content,
    )
    .await?;
    if let Some(hub) = &state.collab {
        hub.page_meta_changed(id, page["title"].as_str().unwrap_or(""));
    }
    Ok(Json(page).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateVersionBody {
    expected_revision_no: i64,
}

async fn create_doc_version(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<CreateVersionBody>,
) -> Result<Response, ApiError> {
    let version =
        docs::history::create_version(&state.pool, user, id, body.expected_revision_no).await?;
    Ok((StatusCode::CREATED, Json(version)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsQuery {
    before: Option<i32>,
    limit: Option<i64>,
}

async fn list_doc_versions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<VersionsQuery>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(
        Json(docs::history::list_versions(&state.pool, user, id, query.before, query.limit).await?)
            .into_response(),
    )
}

async fn get_doc_version(
    State(state): State<AppState>,
    Path((id, version_no)): Path<(Uuid, i32)>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(docs::history::get_version(&state.pool, user, id, version_no).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveDocBody {
    parent_id: Option<Uuid>,
    before_id: Option<Uuid>,
}

async fn move_doc(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<MoveDocBody>,
) -> Result<Response, ApiError> {
    docs::move_page(&state.pool, user, id, body.parent_id, body.before_id).await?;
    Ok(Json(json!({"ok": true})).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisibilityBody {
    visibility: String,
    scope_project_id: Option<Uuid>,
    scope_team_id: Option<Uuid>,
}

async fn set_doc_visibility(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<VisibilityBody>,
) -> Result<Response, ApiError> {
    let result = docs::set_visibility(
        &state.pool,
        user,
        id,
        &body.visibility,
        body.scope_project_id,
        body.scope_team_id,
    )
    .await;
    // The page may have been closed to the very person who changed it; sockets are rechecked
    // against PostgreSQL whatever the answer to them was.
    if let Some(hub) = &state.collab {
        hub.recheck(id).await;
    }
    Ok(Json(result?).into_response())
}

async fn doc_participants(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(docs::participants(&state.pool, user, id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ParticipantsBody {
    participants: Vec<ParticipantBody>,
}

async fn set_doc_participants(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<ParticipantsBody>,
) -> Result<Response, ApiError> {
    let result = docs::set_participants(
        &state.pool,
        user,
        id,
        body.participants
            .into_iter()
            .map(|p| (p.user_id, p.role))
            .collect(),
    )
    .await;
    if let Some(hub) = &state.collab {
        hub.recheck(id).await;
    }
    Ok(Json(result?).into_response())
}

async fn delete_doc(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    let deleted = docs::delete(&state.pool, user, id).await?;
    if let Some(hub) = &state.collab {
        for page in deleted {
            hub.page_deleted(page).await;
        }
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentsQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_doc_comments(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<CommentsQuery>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(
        Json(docs::list_comments(&state.pool, user, id, query.cursor, query.limit).await?)
            .into_response(),
    )
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentBody {
    body: Value,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCommentBody {
    body: Value,
    #[serde(default)]
    file_ids: Vec<Uuid>,
}

async fn create_doc_comment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<CreateCommentBody>,
) -> Result<Response, ApiError> {
    let comment = docs::create_comment(&state.pool, user, id, body.body, &body.file_ids).await?;
    if let Some(hub) = &state.collab {
        hub.comments_changed(id);
    }
    Ok((StatusCode::CREATED, Json(comment)).into_response())
}

async fn edit_doc_comment(
    State(state): State<AppState>,
    Path((id, comment_id)): Path<(Uuid, Uuid)>,
    Actor(user): Actor,
    Body(body): Body<CommentBody>,
) -> Result<Response, ApiError> {
    let comment = docs::edit_comment(&state.pool, user, id, comment_id, body.body).await?;
    if let Some(hub) = &state.collab {
        hub.comments_changed(id);
    }
    Ok(Json(comment).into_response())
}

async fn delete_doc_comment(
    State(state): State<AppState>,
    Path((id, comment_id)): Path<(Uuid, Uuid)>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    docs::delete_comment(&state.pool, state.objects.as_ref(), user, id, comment_id).await?;
    if let Some(hub) = &state.collab {
        hub.comments_changed(id);
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_doc_templates(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(docs::list_templates(&state.pool, user, workspace_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TemplateBody {
    name: String,
    from_doc_id: Uuid,
}

async fn create_doc_template(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<TemplateBody>,
) -> Result<Response, ApiError> {
    let template = docs::create_template(
        &state.pool,
        &state.knowledge,
        user,
        workspace_id,
        &body.name,
        body.from_doc_id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(template)).into_response())
}

async fn delete_doc_template(
    State(state): State<AppState>,
    Path((workspace_id, template_id)): Path<(Uuid, Uuid)>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    docs::delete_template(&state.pool, user, workspace_id, template_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachBody {
    file_id: Uuid,
}

async fn attach_doc_file(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<AttachBody>,
) -> Result<Response, ApiError> {
    docs::attach_file(&state.pool, user, id, body.file_id).await?;
    Ok(Json(json!({"ok": true})).into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveItem {
    kind: String,
    id: Uuid,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveBody {
    items: Vec<ResolveItem>,
}

async fn resolve_references(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user): Actor,
    Body(body): Body<ResolveBody>,
) -> Result<Response, ApiError> {
    let items = body.items.into_iter().map(|i| (i.kind, i.id)).collect();
    Ok(
        Json(docs::resolve_references(&state.pool, user, workspace_id, items).await?)
            .into_response(),
    )
}

// ---------------------------------------------------------------------------------------------
// Referenced by
// ---------------------------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn ticket_references(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<ChangesQuery>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(
        penfii_core::references::for_ticket(&state.pool, user, id, query.cursor, query.limit)
            .await?,
    )
    .into_response())
}

async fn doc_references(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<ChangesQuery>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(
        penfii_core::references::for_doc(&state.pool, user, id, query.cursor, query.limit).await?,
    )
    .into_response())
}
