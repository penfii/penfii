//! MCP operations: audit, webhooks (list only), trash, archive and orphaned-resource lists (FEA-171).
//!
//! Read-only on purpose. Creating, rotating or revoking a webhook, recovering a Project/Doc/Channel
//! manager, permanently deleting a Ticket and changing the MCP policy all need a recent sign-in in
//! the web app; they are in `web_only` and not exposed here.

use penfii_core::oauth::{Principal, RiskClass};
use penfii_core::{admin, conversation, docs, project_access, ticket_lifecycle, webhooks};
use serde_json::{Value, json};

use super::{Handled, cap_arrays, items_of, page};
use crate::McpContext;
use crate::catalog::{
    Operation, Toolset, WorkspaceResolver, object_schema, uuid_property, workspace_id_property,
};
use crate::tools::{Done, Failure, done, limit_arg, string_arg, to_value, uuid_arg};

const ADMIN_R: &[&str] = &["admin:read"];
const ARG: WorkspaceResolver = WorkspaceResolver::Argument;
const CHANNEL: WorkspaceResolver = WorkspaceResolver::Query {
    arg: "channelId",
    sql: "SELECT workspace_id FROM penfii.conversations WHERE id = $1",
};

fn schema_audit() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "action": {"type": "string", "description": "Exact action code, or a prefix ending in a dot such as `invitation.`."},
               "limit": {"type": "integer", "minimum": 1, "maximum": 50},
               "cursor": {"type": "string", "description": "nextCursor of the previous page."}}),
        &["workspaceId"],
    )
}
fn schema_webhooks() -> Value {
    object_schema(
        json!({"channelId": uuid_property("Channel id from workspace_overview.")}),
        &["channelId"],
    )
}
fn schema_project_paged() -> Value {
    object_schema(
        json!({"projectId": uuid_property("Project id from workspace_overview."),
               "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "Default 20."},
               "cursor": {"type": "string"}}),
        &["projectId"],
    )
}
fn schema_ws() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property()}),
        &["workspaceId"],
    )
}

pub static OPS: &[Operation] = &[
    Operation::new(
        Toolset::Admin,
        "audit.list",
        "audit_list",
        "Workspace audit log",
        "Audit entries of a Workspace, newest first, paged (limit up to 50, cursor), filtered by `action`. Only an owner, admin or security auditor may read it (Core decides). Entries made through MCP carry via=mcp, the client and grant. Read-only.",
        ADMIN_R,
        RiskClass::Read,
        ARG,
        schema_audit,
    ),
    Operation::new(
        Toolset::Admin,
        "webhook.list",
        "webhook_list",
        "List a channel's webhooks",
        "Webhooks of a channel: id, name, auth mode, acting user, revoked flag and dates. Never includes secrets or URLs with tokens. Creating, rotating and revoking webhooks is web-only (workspace_web_only). Read-only.",
        ADMIN_R,
        RiskClass::Read,
        CHANNEL,
        schema_webhooks,
    ),
    Operation::new(
        Toolset::Admin,
        "trash.list",
        "trash_list",
        "Tickets in a Project's trash",
        "Trashed Tickets of a Project (key, title, deleted time and who deleted them) for a Project admin. Paged. Read-only. Restore a Ticket with the ticket tools; permanent deletion is web-only.",
        ADMIN_R,
        RiskClass::Read,
        WorkspaceResolver::Project,
        schema_project_paged,
    ),
    Operation::new(
        Toolset::Admin,
        "archive.list",
        "archive_list",
        "Archived Tickets of a Project",
        "Archived Tickets of a Project the person can read. Paged. Read-only.",
        ADMIN_R,
        RiskClass::Read,
        WorkspaceResolver::Project,
        schema_project_paged,
    ),
    Operation::new(
        Toolset::Admin,
        "recovery.projects_without_admin",
        "recovery_projects_without_admin",
        "Projects that have no admin",
        "Projects of a Workspace with no active admin (owner/admin per Core). Read-only. Giving them an admin is done in the web app (re-authentication required).",
        ADMIN_R,
        RiskClass::Read,
        ARG,
        schema_ws,
    ),
    Operation::new(
        Toolset::Admin,
        "recovery.docs_without_manager",
        "recovery_docs_without_manager",
        "Docs that have no manager",
        "Docs with no active manager. Read-only. Recovery is web-only.",
        ADMIN_R,
        RiskClass::Read,
        ARG,
        schema_ws,
    ),
    Operation::new(
        Toolset::Admin,
        "recovery.channels_without_manager",
        "recovery_channels_without_manager",
        "Channels that have no manager",
        "Channels with no active manager. Read-only. Recovery is web-only.",
        ADMIN_R,
        RiskClass::Read,
        ARG,
        schema_ws,
    ),
];

pub(super) async fn run(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Handled {
    Some(run_admin(ctx, principal, op, args).await)
}

async fn run_admin(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Result<Done, Failure> {
    let (user, pool) = (principal.user_id, &ctx.pool);
    match op.id {
        "audit.list" => {
            let workspace = uuid_arg(args, "workspaceId")?;
            let page = admin::list_audit(
                pool,
                user,
                workspace,
                string_arg(args, "action"),
                Some(limit_arg(args, 20)?),
                string_arg(args, "cursor"),
            )
            .await?;
            let mut value = to_value(&page)?;
            cap_arrays(&mut value, 50);
            Ok(done(value))
        }
        "webhook.list" => {
            let channel = uuid_arg(args, "channelId")?;
            let list = webhooks::list_webhooks(pool, user, channel).await?;
            Ok(Done::about(to_value(&list)?, "channel", channel))
        }
        "trash.list" => {
            let project = uuid_arg(args, "projectId")?;
            let items = ticket_lifecycle::list_trash(pool, user, project).await?;
            Ok(done(page(items_of(&items)?, args)?))
        }
        "archive.list" => {
            let project = uuid_arg(args, "projectId")?;
            let items = ticket_lifecycle::list_archived(pool, user, project).await?;
            Ok(done(page(items_of(&items)?, args)?))
        }
        "recovery.projects_without_admin" => {
            let items =
                project_access::projects_without_admin(pool, user, uuid_arg(args, "workspaceId")?)
                    .await?;
            Ok(done(
                json!({"items": to_value(&items)?, "recovery": "web-only (re-authentication required)"}),
            ))
        }
        "recovery.docs_without_manager" => {
            let items =
                docs::recovery::docs_without_manager(pool, user, uuid_arg(args, "workspaceId")?)
                    .await?;
            Ok(done(
                json!({"items": to_value(&items)?, "recovery": "web-only (re-authentication required)"}),
            ))
        }
        "recovery.channels_without_manager" => {
            let items = conversation::recovery::channels_without_manager(
                pool,
                user,
                uuid_arg(args, "workspaceId")?,
            )
            .await?;
            Ok(done(
                json!({"items": to_value(&items)?, "recovery": "web-only (re-authentication required)"}),
            ))
        }
        other => Err(Failure::new(
            "internal_error",
            format!("operation {other} has no handler"),
        )),
    }
}
