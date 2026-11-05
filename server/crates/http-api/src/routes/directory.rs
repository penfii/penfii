//! The Space, the directory and Workspace search.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_contracts::SearchQuery;
use penfii_core::CoreError;
use penfii_core::directory;
use penfii_core::organization;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Q};

use axum::Router;
use axum::routing::get;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/space", get(space))
        .route("/v1/workspaces/{id}/search", get(search_directory))
        .route(
            "/v1/workspaces/{id}/organizations/{organization_id}",
            get(organization_directory),
        )
}

async fn space(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(organization::space(&state.pool, user_id, id).await?).into_response())
}

async fn search_directory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<SearchQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    if let Some(kind) = query.kind.clone() {
        return unified_search(&state, user_id, id, kind, query).await;
    }
    let results = directory::search(
        &state.pool,
        user_id,
        id,
        query.q.as_deref().unwrap_or(""),
        query.types.as_deref(),
        query.limit,
    )
    .await?;
    Ok(Json(results).into_response())
}

/// Search across objects. People come from the directory; everything else from the index, after
/// the server has re-checked access to each hit.
async fn unified_search(
    state: &AppState,
    user_id: Uuid,
    workspace_id: Uuid,
    kind: String,
    query: SearchQuery,
) -> Result<Response, ApiError> {
    use penfii_core::search::{self, SearchError, SearchParams};
    let q = query.q.clone().unwrap_or_default();
    let mut people = Vec::new();
    if kind == "person" || (kind == "all" && query.cursor.is_none()) {
        let found = directory::search(
            &state.pool,
            user_id,
            workspace_id,
            &q,
            Some("person"),
            Some(if kind == "person" {
                query.limit.unwrap_or(20)
            } else {
                5
            }),
        )
        .await?;
        people = found
            .results
            .into_iter()
            .map(|hit| {
                serde_json::json!({
                    "kind": "person", "id": hit.id, "title": hit.title, "subtitle": hit.subtitle,
                    "snippet": [], "createdAt": null, "fields": {"person": hit.person},
                })
            })
            .collect();
    }
    if kind == "person" {
        return Ok(Json(serde_json::json!({
            "query": q, "normalized": {}, "results": people, "nextCursor": null,
            "status": {"ready": true, "unavailableKinds": [], "stale": false}
        }))
        .into_response());
    }
    let state_ids = match query.state_ids.as_deref() {
        None | Some("") => Vec::new(),
        Some(text) => text
            .split(',')
            .map(|part| Uuid::parse_str(part.trim()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ApiError::validation("stateIds must be a comma separated list of ids"))?,
    };
    let params = SearchParams {
        q: q.clone(),
        kind: kind.clone(),
        state_ids,
        limit: query.limit,
        cursor: query.cursor.clone(),
        tz: query.tz.clone(),
    };
    match search::search(
        &state.pool,
        state.search.as_ref(),
        user_id,
        workspace_id,
        params,
    )
    .await
    {
        Ok(response) => {
            let mut value = serde_json::to_value(&response)
                .map_err(|e| ApiError::from_core(CoreError::Internal(e.to_string())))?;
            if !people.is_empty()
                && let Some(results) = value["results"].as_array_mut()
            {
                let mut merged = people;
                merged.append(results);
                *results = merged;
            }
            Ok(Json(value).into_response())
        }
        Err(SearchError::Invalid(invalid)) => Err(ApiError::coded_with(
            StatusCode::BAD_REQUEST,
            invalid.code,
            invalid.message,
            serde_json::json!({"token": invalid.token}),
        )),
        Err(SearchError::Ambiguous { token, candidates }) => Err(ApiError::coded_with(
            StatusCode::BAD_REQUEST,
            "ambiguous_filter",
            "more than one match; choose one",
            serde_json::json!({
                "token": token,
                "candidates": candidates.into_iter().map(|(id, name)| serde_json::json!({"id": id, "name": name})).collect::<Vec<_>>(),
            }),
        )),
        Err(SearchError::Unavailable) => Err(ApiError::coded_with(
            StatusCode::SERVICE_UNAVAILABLE,
            "search_unavailable",
            "search is not available right now",
            serde_json::json!({"retryable": true}),
        )),
        Err(SearchError::Core(error)) => Err(ApiError::from_core(error)),
    }
}

async fn organization_directory(
    State(state): State<AppState>,
    Path((id, organization_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let directory = directory::organization(&state.pool, user_id, id, organization_id).await?;
    Ok(Json(directory).into_response())
}
