//! One module per product area, each owning a slice of the Operation Catalog and its handlers.
//! An operation is declared with `Operation::new` in its module's `OPS` and handled in that module's
//! `run`, which `run` below calls by the operation's domain. Argument and result helpers
//! (`uuid_arg`, `rich_text`, `Done::about`, ...) are in `crate::tools`, schema helpers in
//! `crate::catalog`, list paging and capping at the end of this file. Adding a tool also means a line
//! in `coverage/<area>.tsv` and the table in `docs/mcp-tools.md`; tests check both.

use penfii_core::oauth::Principal;
use serde_json::{Map, Value, json};

use crate::McpContext;
use crate::catalog::Operation;
use crate::tools::{Done, Failure, limit_arg, string_arg, to_value};

mod admin;
mod conversation;
mod doc;
mod file;
mod org;
mod personal;
mod plan;
mod project;
mod search;
mod tickets;

/// The operation slices of every domain module.
pub fn operations() -> Vec<&'static [Operation]> {
    vec![
        admin::OPS,
        conversation::OPS,
        doc::OPS,
        file::OPS,
        org::OPS,
        personal::OPS,
        plan::OPS,
        project::OPS,
        search::OPS,
        tickets::OPS,
    ]
}

pub(crate) type Handled = Option<Result<Done, Failure>>;

/// Runs an operation in the module of its domain. `None`: no domain module handles it (the first
/// release's operations, which `tools::run` holds).
pub async fn run(ctx: &McpContext, principal: &Principal, op: &Operation, args: &Value) -> Handled {
    match op.domain {
        "admin" => admin::run(ctx, principal, op, args).await,
        "conversation" => conversation::run(ctx, principal, op, args).await,
        "doc" => doc::run(ctx, principal, op, args).await,
        "file" => file::run(ctx, principal, op, args).await,
        "org" => org::run(ctx, principal, op, args).await,
        "personal" => personal::run(ctx, principal, op, args).await,
        "plan" => plan::run(ctx, principal, op, args).await,
        "project" => project::run(ctx, principal, op, args).await,
        "search" => search::run(ctx, principal, op, args).await,
        "ticket" => tickets::run(ctx, principal, op, args).await,
        _ => None,
    }
}

// Shared by the domains: lists the server returns whole, and answers that must stay bounded.

/// Offset paging over a list the server returns whole.
/// A scope the call needs because of its arguments, beyond the operation's own `scopes`. The
/// operation checks it itself when it runs.
pub(crate) fn argument_scope(op_id: &str, args: &Value) -> Option<&'static str> {
    match op_id {
        "search.query" => search::kind_scope(args),
        _ => None,
    }
}

pub(super) fn page(items: Vec<Value>, args: &Value) -> Result<Value, Failure> {
    let limit = limit_arg(args, 20)? as usize;
    let offset = match string_arg(args, "cursor") {
        None | Some("") => 0,
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| Failure::invalid("cursor is not valid"))?,
    };
    let total = items.len();
    let shown: Vec<Value> = items.into_iter().skip(offset).take(limit).collect();
    let next = (offset + shown.len() < total).then(|| (offset + shown.len()).to_string());
    Ok(json!({"items": shown, "total": total, "nextCursor": next}))
}

pub(super) fn items_of<T: serde::Serialize>(list: &[T]) -> Result<Vec<Value>, Failure> {
    list.iter().map(|item| to_value(item)).collect()
}

/// Caps every array in `value` at `max` items. Names what was cut in `truncated` (path -> total
/// count) on the top-level object, so a client knows the answer is partial.
pub(super) fn cap_arrays(value: &mut Value, max: usize) {
    fn walk(value: &mut Value, path: String, max: usize, cut: &mut Map<String, Value>) {
        match value {
            Value::Array(items) => {
                if items.len() > max {
                    cut.insert(path.clone(), json!(items.len()));
                    items.truncate(max);
                }
                for (index, item) in items.iter_mut().enumerate() {
                    walk(item, format!("{path}[{index}]"), max, cut);
                }
            }
            Value::Object(map) => {
                for (key, item) in map.iter_mut() {
                    let child = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    walk(item, child, max, cut);
                }
            }
            _ => {}
        }
    }
    let mut cut = Map::new();
    walk(value, String::new(), max, &mut cut);
    if !cut.is_empty()
        && let Value::Object(map) = value
    {
        map.insert("truncated".into(), Value::Object(cut));
    }
}
