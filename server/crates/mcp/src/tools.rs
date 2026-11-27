//! Operation handlers: ports of the matching route handlers. Each parses its
//! arguments, calls the same `core` function with the principal's user id, and maps the result.
//! Nothing here decides who may see what; `core` does, on every call.

use penfii_core::oauth::{Principal, RiskClass, policy, scopes};
use penfii_core::work::{TicketInput, TicketUpdate};
use penfii_core::{CoreError, access, directory, organization, work};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::McpContext;
use crate::catalog::{Operation, WorkspaceResolver};

/// A tool-level failure: reported as `isError: true` with a stable code, never as a protocol
/// error (the model can often act on it).
#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    /// The scopes the call needs that the connection does not have (insufficient_scope only).
    pub needed_scopes: Vec<String>,
}

impl Failure {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            needed_scopes: Vec::new(),
        }
    }

    /// The connection lacks these scopes: all of them at once, so one new consent is enough.
    pub(crate) fn insufficient_scope(missing: Vec<String>) -> Self {
        Self {
            needed_scopes: missing.clone(),
            ..Self::new(
                "insufficient_scope",
                format!("this connection was not granted {}", missing.join(", ")),
            )
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message)
    }

    pub fn outcome(&self) -> &'static str {
        match self.code {
            "insufficient_scope" => "scope_denied",
            "policy_denied" => "policy_denied",
            "forbidden" => "forbidden",
            "not_found" => "not_found",
            _ => "error",
        }
    }
}

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Self {
        match error {
            // Existence is not revealed.
            CoreError::NotFound => Self::new("not_found", "not found"),
            CoreError::Unauthenticated => Self::new("not_found", "not found"),
            CoreError::Forbidden(message) => Self::new("forbidden", message),
            CoreError::Validation(message) => Self::new("invalid_arguments", message),
            CoreError::Rule { code, message } => Self::new(code, message),
            // Facts that come with a conflict (`currentRevision`, ...) let the model read again and
            // merge, so they are part of the message.
            CoreError::Conflict {
                code,
                message,
                extra,
            } if !extra.is_empty() => {
                Self::new(code, format!("{message} {}", Value::Object(extra)))
            }
            CoreError::Conflict { code, message, .. } => Self::new(code, message),
            CoreError::ReauthRequired { .. } => {
                Self::new("reauth_required", "recent authentication is required")
            }
            CoreError::MfaRequired => Self::new("mfa_required", "a verification code is required"),
            CoreError::MfaCodeInvalid => Self::new("mfa_code_invalid", "the code is not valid"),
            CoreError::RateLimited { .. } => Self::new("rate_limited", "too many requests"),
            CoreError::DbUnavailable(cause) => {
                tracing::warn!(cause, "mcp tool: database unavailable");
                Self::new("db_unavailable", "the database is temporarily unavailable")
            }
            CoreError::Internal(message) => {
                tracing::error!(%message, "mcp tool internal error");
                Self::new("internal_error", "internal error")
            }
        }
    }
}

impl From<sqlx::Error> for Failure {
    fn from(error: sqlx::Error) -> Self {
        CoreError::from(error).into()
    }
}

/// What a successful operation hands back, plus the facts the call log records.
pub struct Done {
    pub value: Value,
    pub resource_type: Option<&'static str>,
    pub resource_id: Option<String>,
    /// Revision or version a write produced (a Doc revision, a message version).
    pub resource_revision: Option<i64>,
}

impl Done {
    /// A result about one resource: the call log records its type and id.
    pub(crate) fn about(value: Value, kind: &'static str, id: impl ToString) -> Self {
        Self {
            value,
            resource_type: Some(kind),
            resource_id: Some(id.to_string()),
            resource_revision: None,
        }
    }

    /// `about`, with the id read from the result's own `id` field (none when it has no such field).
    pub(crate) fn about_own(value: Value, kind: &'static str) -> Self {
        let id = value.get("id").and_then(Value::as_str).map(str::to_string);
        Self {
            value,
            resource_type: Some(kind),
            resource_id: id,
            resource_revision: None,
        }
    }

    /// The revision the write produced.
    pub(crate) fn revision(mut self, revision: Option<i64>) -> Self {
        self.resource_revision = revision;
        self
    }
}

pub(crate) fn uuid_arg(args: &Value, name: &str) -> Result<Uuid, Failure> {
    args.get(name)
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .ok_or_else(|| Failure::invalid(format!("{name} must be a UUID")))
}

/// A required string argument.
pub(crate) fn text_arg(args: &Value, name: &str) -> Result<String, Failure> {
    string_arg(args, name)
        .map(str::to_string)
        .ok_or_else(|| Failure::invalid(format!("{name} is required")))
}

/// A required integer argument.
pub(crate) fn int_arg(args: &Value, name: &str) -> Result<i64, Failure> {
    args.get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| Failure::invalid(format!("{name} must be an integer")))
}

/// An optional UUID argument: absent or null is `None`, anything else must be a UUID.
pub(crate) fn optional_uuid(args: &Value, name: &str) -> Result<Option<Uuid>, Failure> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => uuid_arg(args, name).map(Some),
    }
}

/// A required `YYYY-MM-DD` date argument.
pub(crate) fn date_arg(args: &Value, name: &str) -> Result<chrono::NaiveDate, Failure> {
    string_arg(args, name)
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| Failure::invalid(format!("{name} must be a date (YYYY-MM-DD)")))
}

/// A required RFC 3339 date-time argument, in UTC.
pub(crate) fn datetime_arg(
    args: &Value,
    name: &str,
) -> Result<chrono::DateTime<chrono::Utc>, Failure> {
    string_arg(args, name)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|at| at.with_timezone(&chrono::Utc))
        .ok_or_else(|| Failure::invalid(format!("{name} must be an RFC 3339 date-time")))
}

pub(crate) fn string_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

pub(crate) fn limit_arg(args: &Value, default: i64) -> Result<i64, Failure> {
    match args.get("limit") {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_i64()
            .filter(|limit| (1..=50).contains(limit))
            .ok_or_else(|| Failure::invalid("limit must be between 1 and 50")),
    }
}

/// The Workspace an operation targets, from its resolver: `None` for operations that are not tied to
/// one. No access check: callers that need one do it themselves; `call_tool` also uses it to name the
/// Workspace on the log row of a refused call.
pub async fn resolve_workspace(
    ctx: &McpContext,
    op: &Operation,
    args: &Value,
) -> Result<Option<Uuid>, Failure> {
    let workspace_id = match op.resolver {
        WorkspaceResolver::None => return Ok(None),
        WorkspaceResolver::Argument => uuid_arg(args, "workspaceId")?,
        WorkspaceResolver::Project => {
            let project_id = uuid_arg(args, "projectId")?;
            sqlx::query_scalar::<_, Uuid>("SELECT workspace_id FROM penfii.projects WHERE id = $1")
                .bind(project_id)
                .fetch_optional(&ctx.pool)
                .await?
                .ok_or_else(|| Failure::from(CoreError::NotFound))?
        }
        WorkspaceResolver::Query { arg, sql } => {
            let id = uuid_arg(args, arg)?;
            sqlx::query_scalar::<_, Uuid>(sql)
                .bind(id)
                .fetch_optional(&ctx.pool)
                .await?
                .ok_or_else(|| Failure::from(CoreError::NotFound))?
        }
        WorkspaceResolver::Ticket => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            sqlx::query_scalar::<_, Uuid>("SELECT workspace_id FROM penfii.tickets WHERE id = $1")
                .bind(ticket_id)
                .fetch_optional(&ctx.pool)
                .await?
                .ok_or_else(|| Failure::from(CoreError::NotFound))?
        }
    };
    Ok(Some(workspace_id))
}

/// The MCP policies of the Workspaces this connection reaches: the person's Workspaces, narrowed by
/// the grant's Workspace filter. Fails rather than leaving a Workspace out, so a check built on it
/// cannot be passed because a restrictive policy could not be read.
pub(crate) async fn reachable_policies(
    ctx: &McpContext,
    principal: &Principal,
) -> Result<Vec<policy::McpPolicy>, Failure> {
    let mut policies = Vec::new();
    for workspace in organization::list_workspaces(&ctx.pool, principal.user_id).await? {
        if principal
            .workspace_filter
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(&workspace.id))
        {
            continue;
        }
        policies.push(policy::effective(&ctx.pool, ctx.oauth.policy_defaults, workspace.id).await?);
    }
    Ok(policies)
}

/// The Workspace of a refused call, when the principal is an active member of it.
pub async fn member_workspace(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Option<Uuid> {
    let workspace_id = resolve_workspace(ctx, op, args).await.ok().flatten()?;
    let mut conn = ctx.pool.acquire().await.ok()?;
    access::require_member(&mut conn, workspace_id, principal.user_id)
        .await
        .ok()
        .map(|_| workspace_id)
}

/// Steps 2 to 3 of the Authorization Model: scope, grant restriction, membership, Workspace
/// policy. Returns the Workspace the operation targets, if any.
pub async fn authorize(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Result<Option<Uuid>, Failure> {
    let lacked = |needed: &&str| !scopes::grants(&principal.scopes, needed);
    let mut missing: Vec<&str> = op.scopes.iter().copied().filter(lacked).collect();
    if !missing.is_empty() {
        // Name what the arguments need as well, so that one new consent is enough.
        missing.extend(
            crate::domains::argument_scope(op.id, args)
                .filter(lacked)
                .filter(|extra| !op.scopes.contains(extra)),
        );
        return Err(Failure::insufficient_scope(
            missing.into_iter().map(str::to_string).collect(),
        ));
    }
    let Some(workspace_id) = resolve_workspace(ctx, op, args).await? else {
        return Ok(None);
    };
    if let Some(allowed) = &principal.workspace_filter
        && !allowed.contains(&workspace_id)
    {
        return Err(CoreError::NotFound.into());
    }
    // Only an active member learns a policy exists; anyone else gets the answer core would give.
    let mut conn = ctx.pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, principal.user_id).await?;
    drop(conn);
    let effective = policy::effective(&ctx.pool, ctx.oauth.policy_defaults, workspace_id).await?;
    if !effective.allows(op.risk) {
        return Err(Failure::new(
            "policy_denied",
            "a Workspace administrator has turned this kind of MCP access off",
        ));
    }
    Ok(Some(workspace_id))
}

pub async fn run(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Result<Done, Failure> {
    let user = principal.user_id;
    if let Some(result) = crate::domains::run(ctx, principal, op, args).await {
        return result;
    }
    match op.id {
        "workspace.mcp_policy" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let policy =
                policy::get(&ctx.pool, ctx.oauth.policy_defaults, user, workspace_id).await?;
            Ok(done(to_value(&policy)?))
        }
        "workspace.web_only" => Ok(done(json!({
            "webOnly": crate::web_only::ENTRIES.iter().map(|entry| json!({
                "action": entry.action,
                "why": entry.why,
                "whereInWeb": entry.where_in_web,
            })).collect::<Vec<_>>(),
        }))),
        "workspace.list" => {
            let mut items = Vec::new();
            for workspace in penfii_core::organization::list_workspaces(&ctx.pool, user).await? {
                if principal
                    .workspace_filter
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(&workspace.id))
                {
                    continue;
                }
                let effective =
                    policy::effective(&ctx.pool, ctx.oauth.policy_defaults, workspace.id).await?;
                if effective.allows(RiskClass::Read) {
                    items.push(to_value(&workspace)?);
                }
            }
            Ok(done(json!({"workspaces": items})))
        }
        "workspace.overview" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let space = organization::space(&ctx.pool, user, workspace_id).await?;
            Ok(done(to_value(&space)?))
        }
        "member.search" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let found = directory::search(
                &ctx.pool,
                user,
                workspace_id,
                string_arg(args, "query").unwrap_or(""),
                Some("person"),
                Some(limit_arg(args, 20)?),
            )
            .await?;
            Ok(done(to_value(&found)?))
        }
        "ticket.get" => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            let ticket = work::get_ticket(&ctx.pool, user, ticket_id).await?;
            Ok(Done {
                value: to_value(&ticket)?,
                resource_type: Some("ticket"),
                resource_id: Some(ticket_id.to_string()),
                resource_revision: None,
            })
        }
        "ticket.comments" => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            let comments = work::comments(&ctx.pool, user, ticket_id).await?;
            Ok(Done {
                value: json!({"comments": to_value(&comments)?}),
                resource_type: Some("ticket"),
                resource_id: Some(ticket_id.to_string()),
                resource_revision: None,
            })
        }
        "workflow.get" => {
            let project_id = uuid_arg(args, "projectId")?;
            let (version, states) = work::workflow(&ctx.pool, project_id, user).await?;
            Ok(Done {
                value: json!({"projectId": project_id, "version": version, "states": to_value(&states)?}),
                resource_type: Some("project"),
                resource_id: Some(project_id.to_string()),
                resource_revision: None,
            })
        }
        "ticket.create" => ticket_create(ctx, principal, args).await,
        "ticket.update" => ticket_update(ctx, principal, args).await,
        "ticket.change_status" => ticket_change_status(ctx, principal, args).await,
        "ticket.comment" => {
            let ticket_id = uuid_arg(args, "ticketId")?;
            reject_markdown_argument(args, "body")?;
            let body = rich_text(args.get("body"), "body")?
                .ok_or_else(|| Failure::invalid("body is required"))?;
            let (comment, change) =
                work::create_comment(&ctx.pool, user, ticket_id, body, &[]).await?;
            announce(ctx, Some(change));
            Ok(Done {
                value: to_value(&comment)?,
                resource_type: Some("ticket_comment"),
                resource_id: Some(comment.id.to_string()),
                resource_revision: None,
            })
        }
        "project.create" => {
            let workspace_id = uuid_arg(args, "workspaceId")?;
            let name = string_arg(args, "name").unwrap_or("");
            let project = organization::create_project(
                &ctx.pool,
                user,
                workspace_id,
                name,
                &[],
                string_arg(args, "ticketPrefix"),
            )
            .await?;
            Ok(Done {
                value: to_value(&project)?,
                resource_type: Some("project"),
                resource_id: Some(project.id.to_string()),
                resource_revision: None,
            })
        }
        "workflow.delete_state" => {
            let project_id = uuid_arg(args, "projectId")?;
            let state_id = uuid_arg(args, "stateId")?;
            let version = args
                .get("expectedWorkflowVersion")
                .and_then(Value::as_i64)
                .ok_or_else(|| Failure::invalid("expectedWorkflowVersion must be an integer"))?;
            let version =
                work::delete_state(&ctx.pool, user, project_id, state_id, version).await?;
            // The route tells open boards of the Project to look again; so does this.
            announce(
                ctx,
                Some(penfii_core::platform::ChangeNotice::Project { project_id }),
            );
            Ok(Done {
                value: json!({"version": version}),
                resource_type: Some("workflow_state"),
                resource_id: Some(state_id.to_string()),
                resource_revision: None,
            })
        }
        other => Err(Failure::new(
            "internal_error",
            format!("operation {other} has no handler"),
        )),
    }
}

pub(crate) fn done(value: Value) -> Done {
    Done {
        value,
        resource_type: None,
        resource_id: None,
        resource_revision: None,
    }
}

pub(crate) fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|e| CoreError::Internal(e.to_string()).into())
}

/// Rich text input, for every tool that writes a description, message or comment.
///
/// A string is plain text (a paragraph per blank line) and a root object (`{type:"doc",...}`) goes
/// to `core`, which validates rich text exactly as it does for the web app; both are the original
/// behaviour. An object with `format` or `value` is the input wrapper `{format, value}`: `plain`
/// and `markdown` take a string, `richtext` the same root object as before. The wrapper is
/// converted here and never stored. Absent or `null` is `None`, whatever the tool makes of that.
pub(crate) fn rich_text(value: Option<&Value>, name: &str) -> Result<Option<Value>, Failure> {
    rich_text_as(value, name, false)
}

/// A ticket description. The web editor stores a description as a Doc (`penfii.doc`, blocks), and
/// older descriptions (the plain string, ProseMirror JSON) are converted to it by the browser each
/// time they are opened. So Markdown is written as the Doc it will become: task lists stay check
/// items and `---` a divider instead of the text `[ ] ` and `---`, and nothing is converted twice.
/// Plain text and rich-text JSON are kept as they were.
pub(crate) fn description_text(
    value: Option<&Value>,
    name: &str,
) -> Result<Option<Value>, Failure> {
    rich_text_as(value, name, true)
}

fn rich_text_as(
    value: Option<&Value>,
    name: &str,
    markdown_as_doc: bool,
) -> Result<Option<Value>, Failure> {
    use crate::markdown::{
        Content, check_rich_source, is_wrapper, markdown_to_doc, parse_wrapper,
        rich_text_from_markdown, rich_text_from_plain,
    };
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => rich_text_from_plain(text, name).map(Some),
        Some(Value::Object(object)) if is_wrapper(object) => match parse_wrapper(object, name)? {
            Content::Plain(text) => rich_text_from_plain(text, name).map(Some),
            Content::Markdown(text) if markdown_as_doc => {
                check_rich_source(text, name)?;
                markdown_to_doc(text).map(Some)
            }
            Content::Markdown(text) => rich_text_from_markdown(text, name).map(Some),
            Content::Richtext(root) => Ok(Some(Value::Object(root.clone()))),
        },
        Some(object @ Value::Object(_)) => Ok(Some(object.clone())),
        Some(_) => Err(Failure::invalid(format!(
            "{name} must be text, a document object or a {{format, value}} object"
        ))),
    }
}

/// `markdown` is not an argument of the tools that take `{format, value}` content. The MCP layer
/// does not reject unknown arguments, so without this a call that only passes `markdown` would
/// look like a successful no-op.
pub(crate) fn reject_markdown_argument(args: &Value, field: &str) -> Result<(), Failure> {
    if args.get("markdown").is_some() {
        return Err(Failure::invalid(format!(
            "markdown is not an argument of this tool: pass {field}: {{\"format\": \"markdown\", \"value\": \"...\"}}"
        )));
    }
    Ok(())
}

/// Retry key: the caller's, else a fresh one (no replay protection, and the tool says so).
pub(crate) fn idempotency(args: &Value) -> Result<String, Failure> {
    match args.get("idempotencyKey") {
        None | Some(Value::Null) => Ok(format!("mcp-{}", Uuid::new_v4())),
        Some(Value::String(key)) if !key.trim().is_empty() && key.len() <= 200 => {
            Ok(format!("mcp:{}", key.trim()))
        }
        Some(_) => Err(Failure::invalid("idempotencyKey must be 1..200 characters")),
    }
}

/// `absent` leaves a field alone, `null` clears it, anything else sets it.
pub(crate) fn nullable<T>(
    args: &Value,
    name: &str,
    parse: impl Fn(&Value) -> Option<T>,
) -> Result<Option<Option<T>>, Failure> {
    match args.get(name) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => parse(value)
            .map(|parsed| Some(Some(parsed)))
            .ok_or_else(|| Failure::invalid(format!("{name} has the wrong type"))),
    }
}

pub(crate) fn parse_uuid(value: &Value) -> Option<Uuid> {
    value.as_str().and_then(|raw| Uuid::parse_str(raw).ok())
}

/// An array of UUIDs: absent is `None`, anything else must be an array of UUID strings.
pub(crate) fn uuid_list(args: &Value, name: &str) -> Result<Option<Vec<Uuid>>, Failure> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(parse_uuid)
            .collect::<Option<Vec<_>>>()
            .map(Some)
            .ok_or_else(|| Failure::invalid(format!("{name} must be an array of UUIDs"))),
        Some(_) => Err(Failure::invalid(format!(
            "{name} must be an array of UUIDs"
        ))),
    }
}

pub(crate) fn parse_estimate(value: &Value) -> Option<penfii_core::plan_fields::EstimateInput> {
    let object = value.as_object()?;
    Some(penfii_core::plan_fields::EstimateInput {
        value: object.get("value")?.as_str()?.to_string(),
        unit: match object.get("unit") {
            None | Some(Value::Null) => None,
            Some(unit) => Some(unit.as_str()?.to_string()),
        },
    })
}

pub(crate) fn parse_schedule(value: &Value) -> Option<penfii_core::plan_fields::ScheduleInput> {
    let object = value.as_object()?;
    Some(penfii_core::plan_fields::ScheduleInput {
        planned_start_at: match object.get("plannedStartAt") {
            None | Some(Value::Null) => None,
            Some(at) => Some(at.as_str()?.parse::<chrono::DateTime<chrono::Utc>>().ok()?),
        },
        duration_minutes: match object.get("durationMinutes") {
            None | Some(Value::Null) => None,
            Some(minutes) => Some(minutes.as_i64()?),
        },
        is_schedule_milestone: match object.get("isScheduleMilestone") {
            None | Some(Value::Null) => false,
            Some(flag) => flag.as_bool()?,
        },
    })
}

pub(crate) fn parse_date(value: &Value) -> Option<chrono::NaiveDate> {
    value.as_str().and_then(|raw| raw.parse().ok())
}

pub(crate) fn revision_arg(args: &Value) -> Result<i64, Failure> {
    args.get("expectedRevision")
        .and_then(Value::as_i64)
        .ok_or_else(|| Failure::invalid("expectedRevision must be an integer"))
}

/// The same post-commit effect the route applies: open browsers learn of the change. Takes one
/// notice (`Some`/`None`) or a list of them; nobody listening is not an error.
pub(crate) fn announce(
    ctx: &McpContext,
    changes: impl IntoIterator<Item = penfii_core::ChangeNotice>,
) {
    for change in changes {
        let _ = ctx.changes.send(change);
    }
}

/// A Ticket command's result. The log names the Ticket but no revision: the revision a call wrote
/// names the call itself (its `command_id`, see `penfii_core::origin`), and a call that wrote
/// nothing (a replay, a move that kept the revision) must not be credited with someone else's.
pub(crate) fn ticket_done(
    ctx: &McpContext,
    ticket: penfii_contracts::Ticket,
    change: Option<penfii_core::ChangeNotice>,
) -> Result<Done, Failure> {
    announce(ctx, change);
    Ok(Done::about(to_value(&ticket)?, "ticket", ticket.id))
}

async fn ticket_create(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let project_id = uuid_arg(args, "projectId")?;
    reject_markdown_argument(args, "description")?;
    let title = string_arg(args, "title").ok_or_else(|| Failure::invalid("title is required"))?;
    let input = TicketInput {
        title: title.to_string(),
        description: description_text(args.get("description"), "description")?
            .unwrap_or(Value::Null),
        hierarchy_kind: string_arg(args, "hierarchyKind")
            .unwrap_or("TASK")
            .to_string(),
        parent_id: nullable(args, "parentId", parse_uuid)?.flatten(),
        channel_id: nullable(args, "channelId", parse_uuid)?.flatten(),
        status_id: nullable(args, "statusId", parse_uuid)?.flatten(),
        assignee_id: nullable(args, "assigneeId", parse_uuid)?.flatten(),
        priority: nullable(args, "priority", |v| v.as_str().map(str::to_string))?.flatten(),
        due_on: nullable(args, "dueOn", parse_date)?.flatten(),
        label_ids: uuid_list(args, "labelIds")?.unwrap_or_default(),
        type_id: nullable(args, "typeId", parse_uuid)?.flatten(),
        estimate: nullable(args, "estimate", parse_estimate)?.flatten(),
        schedule: nullable(args, "schedule", parse_schedule)?.flatten(),
        file_ids: uuid_list(args, "fileIds")?.unwrap_or_default(),
    };
    let (ticket, change) = work::create_ticket(
        &ctx.pool,
        principal.user_id,
        project_id,
        input,
        &idempotency(args)?,
    )
    .await?;
    ticket_done(ctx, ticket, change)
}

async fn ticket_update(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let ticket_id = uuid_arg(args, "ticketId")?;
    reject_markdown_argument(args, "description")?;
    let update = TicketUpdate {
        title: string_arg(args, "title").map(str::to_string),
        description: description_text(args.get("description"), "description")?,
        assignee_id: nullable(args, "assigneeId", parse_uuid)?,
        priority: nullable(args, "priority", |v| v.as_str().map(str::to_string))?,
        due_on: nullable(args, "dueOn", parse_date)?,
        label_ids: uuid_list(args, "labelIds")?,
        parent_id: nullable(args, "parentId", parse_uuid)?,
        type_id: nullable(args, "typeId", parse_uuid)?,
        estimate: nullable(args, "estimate", parse_estimate)?,
        schedule: nullable(args, "schedule", parse_schedule)?,
    };
    let expected = revision_arg(args)?;
    let (ticket, change) = work::update_ticket(
        &ctx.pool,
        principal.user_id,
        ticket_id,
        expected,
        update,
        &idempotency(args)?,
    )
    .await?;
    ticket_done(ctx, ticket, change)
}

async fn ticket_change_status(
    ctx: &McpContext,
    principal: &Principal,
    args: &Value,
) -> Result<Done, Failure> {
    let expected = revision_arg(args)?;
    let (ticket, change) = work::change_status(
        &ctx.pool,
        principal.user_id,
        uuid_arg(args, "ticketId")?,
        expected,
        uuid_arg(args, "statusId")?,
        &idempotency(args)?,
    )
    .await?;
    ticket_done(ctx, ticket, change)
}
