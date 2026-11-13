//! Integration tests: the whole router and the core against PostgreSQL (`DATABASE_URL`, a
//! throwaway server; each test gets a database of its own, see `common`) and, for search, OpenSearch
//! with the Korean analyzer (`PENFII_TEST_OPENSEARCH_URL`). `make integration` starts both.

// A test that reads a column wrong should panic: `Row::get` is only barred from the server's code (clippy.toml).
#![allow(clippy::disallowed_methods)]

mod common;

mod admin;
mod api;
mod avatar;
mod capabilities;
mod channel_admin;
mod channel_lifecycle;
mod channel_manager;
mod channel_members;
mod db_errors;
mod external_login;
mod invitations;
mod login_policy;
mod mcp_oauth;
mod membership;
mod migrations;
mod password_reset;
mod project_admin;
mod project_participation;
mod roles;
mod sessions;
mod ticket_files;
mod webhooks;
