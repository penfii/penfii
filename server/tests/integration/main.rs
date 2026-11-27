//! Integration tests: the whole router and the core against PostgreSQL (`DATABASE_URL`, a
//! throwaway server; each test gets a database of its own, see `common`) and, for search, OpenSearch
//! with the Korean analyzer (`PENFII_TEST_OPENSEARCH_URL`). `make integration` starts both.

// A test that reads a column wrong should panic: `Row::get` is only barred from the server's code (clippy.toml).
#![allow(clippy::disallowed_methods)]

mod common;

mod admin;
mod api;
mod auth_audit;
mod avatar;
mod capabilities;
mod channel_admin;
mod channel_lifecycle;
mod channel_manager;
mod channel_members;
mod db_errors;
mod email_change;
mod external_login;
mod invitations;
mod login_policy;
mod long_sessions;
mod mcp_admin;
mod mcp_content;
mod mcp_conversation;
mod mcp_discovery;
mod mcp_doc;
mod mcp_e2e;
mod mcp_file;
mod mcp_oauth;
mod mcp_org;
mod mcp_personal;
mod mcp_plan;
mod mcp_project;
mod mcp_provenance;
mod mcp_risk;
mod mcp_search;
mod mcp_support;
mod mcp_tickets;
mod member_removal;
mod membership;
mod mfa;
mod migrations;
mod password_reset;
mod project_admin;
mod project_participation;
mod resource_limits;
mod roles;
mod sessions;
mod team_promotion;
mod ticket_files;
mod upgrade_rejoin;
mod webhooks;
