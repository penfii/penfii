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
    /// The database could not be used for this request right now, for a reason that is about its
    /// availability or about contention, not about the request or the code: a lock that was not
    /// granted in time, a deadlock, a serialization failure, no connection to be had, a connection
    /// that went away. HTTP 503 `db_unavailable`. The text is a fixed, driver-free name of the
    /// cause (for the log); nothing the database said reaches a client.
    DbUnavailable(&'static str),
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

/// What a database error means for the request, by its SQLSTATE (PostgreSQL errcodes appendix), not by
/// its text.
///
/// - `Unavailable`: availability or contention: `55P03` lock_not_available (`lock_timeout`), `40P01`
///   deadlock_detected, `40001` serialization_failure, `25P03`/`25P04` (the session was ended by
///   `idle_in_transaction_session_timeout`/`transaction_timeout`), `53300` too_many_connections,
///   `57P01` admin_shutdown, `57P03` cannot_connect_now, class `08` connection exceptions, and from
///   the pool: no connection within the acquire timeout, a closed pool, a lost connection.
/// - `Other`: everything else, among it `57014` query_canceled (the statement was cancelled: a
///   `statement_timeout` or a person; whether the query was merely slow or contended cannot be told
///   from the code, so it stays an internal error and is not an "unavailable" database), unique,
///   foreign key and check violations (the calling code decides what those mean), and syntax or
///   schema errors (our bug).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DbErrorClass {
    Unavailable(&'static str),
    Other,
}

pub(crate) fn classify(error: &sqlx::Error) -> DbErrorClass {
    use DbErrorClass::{Other, Unavailable};
    match error {
        sqlx::Error::PoolTimedOut => Unavailable("pool_timeout"),
        sqlx::Error::PoolClosed => Unavailable("pool_closed"),
        sqlx::Error::WorkerCrashed | sqlx::Error::Io(_) => Unavailable("connection_lost"),
        sqlx::Error::Database(db) => match db.code().as_deref() {
            Some("55P03") => Unavailable("lock_not_available"),
            Some("40P01") => Unavailable("deadlock_detected"),
            Some("40001") => Unavailable("serialization_failure"),
            Some("25P03" | "25P04") => Unavailable("session_timeout"),
            Some("53300") => Unavailable("too_many_connections"),
            Some("57P01") => Unavailable("admin_shutdown"),
            Some("57P03") => Unavailable("cannot_connect_now"),
            Some(code) if code.starts_with("08") => Unavailable("connection_exception"),
            _ => Other,
        },
        _ => Other,
    }
}

impl From<sqlx::Error> for CoreError {
    fn from(error: sqlx::Error) -> Self {
        match classify(&error) {
            DbErrorClass::Unavailable(cause) => Self::DbUnavailable(cause),
            DbErrorClass::Other => Self::Internal(error.to_string()),
        }
    }
}
