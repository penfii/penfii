//! Web Push: the server's public key and the signed-in person's subscriptions.
//!
//! A subscription belongs to the person and to one browser; it is not tied to the session, so
//! signing out does not remove it. Someone else's subscription is not found.

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Body};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/push/vapid-public-key", get(public_key))
        .route(
            "/v1/me/push-subscriptions",
            get(list_subscriptions).post(subscribe),
        )
        .route("/v1/me/push-subscriptions/{id}", delete(unsubscribe))
}

async fn public_key(State(state): State<AppState>, Actor(_): Actor) -> Result<Response, ApiError> {
    Ok(Json(match &state.push {
        Some(push) => json!({"configured": true, "publicKey": push.public_key()}),
        None => json!({"configured": false, "publicKey": null}),
    })
    .into_response())
}

#[derive(serde::Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}

/// The shape `PushSubscription.toJSON()` gives, plus an optional label for the device.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeBody {
    endpoint: String,
    keys: Keys,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

async fn subscribe(
    State(state): State<AppState>,
    Actor(user_id): Actor,
    Body(body): Body<SubscribeBody>,
) -> Result<Response, ApiError> {
    if state.push.is_none() {
        return Err(ApiError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "push_not_configured",
            "Web Push is not set up on this server",
        ));
    }
    let saved = penfii_core::push::subscribe(
        &state.pool,
        user_id,
        penfii_core::push::NewSubscription {
            endpoint: &body.endpoint,
            p256dh: &body.keys.p256dh,
            auth: &body.keys.auth,
            user_agent: body.user_agent.as_deref(),
            label: body.label.as_deref(),
        },
    )
    .await?;
    Ok((StatusCode::OK, Json(saved)).into_response())
}

async fn list_subscriptions(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = penfii_core::push::list(&state.pool, user_id).await?;
    Ok(Json(json!({"items": items})).into_response())
}

async fn unsubscribe(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    penfii_core::push::unsubscribe(&state.pool, user_id, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
