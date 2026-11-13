//! The HTTP handlers, one module per area. A handler only translates the request into a core
//! command and the outcome into a response; the rules live in `penfii_core`. Each module owns its
//! slice of the route table in `routes()`, and its handlers and request bodies stay private to it.

mod conversation;
mod directory;
mod files;
mod inbox;
mod organization;
mod personal;
mod planning;
mod project;
mod push;
mod reports;
mod scope;
mod session;
mod tickets;
mod webhooks;
mod workflow;
mod workspace;

use axum::Json;
use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use penfii_core::Outcome;
use serde::Serialize;

use crate::AppState;
use crate::error::ApiError;
use crate::extract;

/// Every route served from this module tree.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .merge(session::routes())
        .merge(workspace::routes())
        .merge(organization::routes())
        .merge(directory::routes())
        .merge(project::routes())
        .merge(workflow::routes())
        .merge(tickets::routes())
        .merge(planning::routes())
        .merge(reports::routes())
        .merge(scope::routes())
        .merge(conversation::routes())
        .merge(webhooks::routes())
        .merge(files::routes())
        .merge(personal::routes())
        .merge(inbox::routes())
        .merge(push::routes())
}

fn finish<T: Serialize>(state: &AppState, outcome: Outcome<T>) -> Result<Response, ApiError> {
    state.publish(outcome.changes);
    let status = StatusCode::from_u16(outcome.http_status).unwrap_or(StatusCode::OK);
    Ok((status, Json(outcome.value)).into_response())
}

fn key(headers: &HeaderMap) -> Result<String, ApiError> {
    extract::idempotency_key(headers)
}
