//! Integration tests: the whole router and the core against PostgreSQL (`DATABASE_URL`, a
//! throwaway server; each test gets a database of its own, see `common`) and, for search, OpenSearch
//! with the Korean analyzer (`PENFII_TEST_OPENSEARCH_URL`). `make integration` starts both.

mod common;

mod admin;
mod api;
mod avatar;
mod capabilities;
mod channel_members;
mod external_login;
mod login_policy;
mod mcp_oauth;
mod migrations;
mod sessions;
mod ticket_files;
