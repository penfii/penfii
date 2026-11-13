//! Chat-slice domain. HTTP types stay in `penfii-http-api`.
//!
//! Domain modules: identity, organization, access, conversation, platform.
//! A command function begins and commits its own transaction. Module operations
//! borrowed by that command must not begin or commit another transaction.

#![forbid(unsafe_code)]

mod error;
pub mod lifecycle;
mod limits;
pub mod mail;
mod migrate;
pub mod provision;

pub mod access;
pub mod admin;
pub mod calendar;
pub mod config;
pub mod consumers;
pub mod conversation;
pub mod directory;
pub mod docs;
pub mod drafts;
pub mod events;
pub mod files;
pub mod health;
pub mod identity;
pub mod inbox;
pub mod knowledge;
pub mod membership;
pub mod notify;
pub mod oauth;
pub mod organization;
mod paging;
pub mod personal;
pub mod plan_fields;
pub mod planning;
pub mod platform;
pub mod preferences;
pub mod project_access;
mod project_role;
pub mod project_schedule;
pub mod push;
pub mod references;
pub mod reminders;
pub mod reports;
pub mod scope;
pub mod search;
pub mod ticket_files;
pub mod ticket_key;
pub mod ticket_lifecycle;
pub mod visibility;
pub mod watch;
pub mod webhooks;
pub mod work;
pub mod work_history;

pub use error::CoreError;
pub use limits::{Bucket, Limits, ThrottleLimits};
pub use migrate::{
    apply_migrations, connect, migration_url_from_env, protect_history, require_current_schema,
    run_migrator,
};
pub use platform::ChangeNotice;

#[derive(Debug)]
pub struct Outcome<T> {
    pub http_status: u16,
    pub value: T,
    pub changes: Vec<ChangeNotice>,
}
