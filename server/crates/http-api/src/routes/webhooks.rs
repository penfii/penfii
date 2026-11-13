//! Incoming webhook management and the unauthenticated post route.
//!
//! Management follows channel settings and needs a recent sign-in. Posting does not use the
//! session or `X-Penfii-Client`; the credential in the path is the only authentication.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use penfii_contracts::CreateWebhookRequest;
use penfii_core::webhooks;
use uuid::Uuid;

use super::finish;
use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, Body, RecentActor};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/channels/{id}/webhooks",
            get(list_webhooks).post(create_webhook),
        )
        .route("/v1/webhooks/{id}/rotate", post(rotate_webhook))
        .route("/v1/webhooks/{id}/revoke", post(revoke_webhook))
        .route("/v1/hooks/{token}", post(post_webhook))
}

async fn create_webhook(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    RecentActor(user_id): RecentActor,
    Body(body): Body<CreateWebhookRequest>,
) -> Result<Response, ApiError> {
    let kek = state.webhook_kek;
    let created = webhooks::create_webhook(
        &state.pool,
        user_id,
        webhooks::NewWebhook {
            channel_id,
            name: &body.name,
            auth_mode: &body.auth_mode,
            actor_id: body.actor_id,
            display_name: body.display_name.as_deref(),
            kek: kek.as_ref(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

async fn list_webhooks(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let list = webhooks::list_webhooks(&state.pool, user_id, channel_id).await?;
    Ok(Json(list).into_response())
}

async fn rotate_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<Uuid>,
    RecentActor(user_id): RecentActor,
) -> Result<Response, ApiError> {
    let created =
        webhooks::rotate_webhook(&state.pool, user_id, webhook_id, state.webhook_kek.as_ref())
            .await?;
    Ok(Json(created).into_response())
}

async fn revoke_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<Uuid>,
    RecentActor(user_id): RecentActor,
) -> Result<Response, ApiError> {
    let webhook = webhooks::revoke_webhook(&state.pool, user_id, webhook_id).await?;
    Ok(Json(webhook).into_response())
}

async fn post_webhook(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    peer: Option<axum::extract::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let outcome = webhooks::post_webhook(
        &state.pool,
        &state.limits,
        webhooks::IncomingPost {
            credential: token,
            raw_body: body.to_vec(),
            timestamp: header("x-penfii-timestamp"),
            signature: header("x-penfii-signature"),
            delivery: header("x-penfii-delivery"),
            address: extract::client_address(peer.map(|found| found.0.0), &headers),
        },
        state.webhook_kek.as_ref(),
    )
    .await?;
    finish(&state, outcome)
}
