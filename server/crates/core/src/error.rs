use serde_json::{Map, Value};

#[derive(Debug)]
pub enum CoreError {
    NotFound,
    Unauthenticated,
    Validation(String),
    Forbidden(String),
    Conflict {
        code: &'static str,
        message: String,
        extra: Map<String, Value>,
    },
    /// A rule of the domain was broken (HTTP 422): the request was understood and is not allowed
    /// by the data as it is, which is not a version conflict.
    Rule {
        code: &'static str,
        message: String,
    },
    /// The action needs a recent proof of identity (HTTP 403 `reauth_required`); `methods` lists
    /// what this account can use to give it.
    ReauthRequired {
        methods: Vec<String>,
    },
    /// Too many failed attempts (HTTP 429).
    RateLimited {
        retry_after_seconds: i64,
    },
    Internal(String),
}

impl CoreError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn rule(code: &'static str, message: impl Into<String>) -> Self {
        Self::Rule {
            code,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::conflict_extra(code, message, Map::new())
    }

    pub fn conflict_extra(
        code: &'static str,
        message: impl Into<String>,
        extra: Map<String, Value>,
    ) -> Self {
        Self::Conflict {
            code,
            message: message.into(),
            extra,
        }
    }
}

impl From<sqlx::Error> for CoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}
