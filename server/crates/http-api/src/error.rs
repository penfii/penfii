use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_core::CoreError;
use serde_json::{Map, Value, json};

pub struct ApiError {
    status: StatusCode,
    body: Value,
}

impl ApiError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::coded(StatusCode::BAD_REQUEST, "validation", message)
    }

    pub fn csrf() -> Self {
        Self::coded(StatusCode::FORBIDDEN, "csrf", "missing X-Penfii-Client")
    }

    pub fn coded_with(
        status: StatusCode,
        code: &str,
        message: impl Into<String>,
        extra: Value,
    ) -> Self {
        let mut body = json!({"code": code, "message": message.into()});
        if let (Some(map), Value::Object(extra)) = (body.as_object_mut(), extra) {
            map.extend(extra);
        }
        Self { status, body }
    }

    pub fn coded(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: json!({
                "code": code,
                "message": message.into(),
            }),
        }
    }

    pub fn from_core(error: CoreError) -> Self {
        match error {
            CoreError::NotFound => Self::coded(StatusCode::NOT_FOUND, "not_found", "not found"),
            CoreError::Unauthenticated => Self::coded(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "unauthenticated",
            ),
            CoreError::Validation(message) => {
                Self::coded(StatusCode::BAD_REQUEST, "validation", message)
            }
            CoreError::Forbidden(message) => {
                Self::coded(StatusCode::FORBIDDEN, "forbidden", message)
            }
            CoreError::Conflict {
                code,
                message,
                extra,
            } => {
                let mut body = Map::new();
                body.insert("code".into(), json!(code));
                body.insert("message".into(), json!(message));
                for (key, value) in extra {
                    body.insert(key, value);
                }
                Self {
                    status: StatusCode::CONFLICT,
                    body: Value::Object(body),
                }
            }
            CoreError::Rule { code, message } => {
                Self::coded(StatusCode::UNPROCESSABLE_ENTITY, code, message)
            }
            CoreError::ReauthRequired { methods } => Self::coded_with(
                StatusCode::FORBIDDEN,
                "reauth_required",
                "recent authentication is required",
                json!({"methods": methods}),
            ),
            CoreError::RateLimited {
                retry_after_seconds,
            } => Self::coded_with(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many attempts, try again later",
                json!({"retryAfterSeconds": retry_after_seconds}),
            ),
            CoreError::Internal(message) => {
                tracing::error!("{message}");
                Self::coded(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        }
    }
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        Self::from_core(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
