//! Pure helpers that turn immutable Ticket revision snapshots into readable field changes.
//!
//! A snapshot is the internal record. These functions produce only the fields a person
//! reads in Activity and History, so callers never expose raw snapshot JSON.

use penfii_contracts::FieldChange;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// Fields shown in Activity and History, in display order.
pub const TRACKED_FIELDS: &[&str] = &[
    "title",
    "description",
    "status",
    "type",
    "priority",
    "assignee",
    "dueOn",
    "labels",
    "parent",
    "estimate",
    "schedule",
];

/// Fields a revision can put back into the current Ticket.
/// Status and parent follow transition and hierarchy rules and are not restored here.
pub const RESTORABLE_FIELDS: &[&str] = &[
    "title",
    "description",
    "type",
    "priority",
    "assignee",
    "dueOn",
    "labels",
    "estimate",
    "schedule",
];

const DESCRIPTION_EXCERPT_CHARS: usize = 2000;

/// A field value as compared (`key`) and as shown (`display`).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldView {
    pub field: &'static str,
    pub key: Value,
    pub display: Value,
}

pub fn description_text(doc: &Value) -> String {
    fn walk(node: &Value, out: &mut String) {
        match node {
            Value::Object(map) => {
                if let Some(Value::String(text)) = map.get("text") {
                    out.push_str(text);
                }
                if let Some(Value::Array(children)) = map.get("content") {
                    let block = map
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind != "doc" && kind != "text");
                    for child in children {
                        walk(child, out);
                    }
                    if block && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(doc, &mut out);
    out.trim().to_owned()
}

fn excerpt(text: String) -> String {
    if text.chars().count() <= DESCRIPTION_EXCERPT_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(DESCRIPTION_EXCERPT_CHARS).collect();
    cut.push('…');
    cut
}

fn text_or_null(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Value::String(text.clone()),
        _ => Value::Null,
    }
}

fn id_of(value: Option<&Value>) -> Value {
    text_or_null(value)
}

/// Normalizes a stored snapshot. `status_names` fills the status name for older snapshots
/// that stored only the status id.
pub fn view(snapshot: &Value, status_names: &HashMap<Uuid, String>) -> Vec<FieldView> {
    let get = |key: &str| snapshot.get(key);
    let status_id = id_of(get("statusId"));
    let status_name = text_or_null(get("statusName"))
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            status_id
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .and_then(|id| status_names.get(&id).cloned())
        });
    let mut labels: Vec<(String, String)> = get("labels")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item.get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        item.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    labels.sort();
    let assignee_id = id_of(get("assigneeId"));
    let assignee_display = if assignee_id.is_null() {
        Value::Null
    } else {
        text_or_null(get("assigneeName"))
            .as_str()
            .map_or(assignee_id.clone(), |n| json!(n))
    };
    let type_id = id_of(get("typeId"));
    let type_display = if type_id.is_null() {
        Value::Null
    } else {
        text_or_null(get("typeName"))
            .as_str()
            .map_or(type_id.clone(), |n| json!(n))
    };
    let parent_id = id_of(get("parentId"));
    let parent_display = if parent_id.is_null() {
        Value::Null
    } else {
        text_or_null(get("parentKey"))
            .as_str()
            .map_or(parent_id.clone(), |n| json!(n))
    };
    let description = get("description").map(description_text).unwrap_or_default();
    vec![
        FieldView {
            field: "title",
            key: text_or_null(get("title")),
            display: text_or_null(get("title")),
        },
        FieldView {
            field: "description",
            key: json!(description),
            display: if description.is_empty() {
                Value::Null
            } else {
                json!(excerpt(description))
            },
        },
        FieldView {
            field: "status",
            key: status_id.clone(),
            display: status_name.map_or(status_id, Value::String),
        },
        FieldView {
            field: "type",
            key: type_id,
            display: type_display,
        },
        FieldView {
            field: "priority",
            key: text_or_null(get("priority")),
            display: text_or_null(get("priority")),
        },
        FieldView {
            field: "assignee",
            key: assignee_id,
            display: assignee_display,
        },
        FieldView {
            field: "dueOn",
            key: text_or_null(get("dueOn")),
            display: text_or_null(get("dueOn")),
        },
        FieldView {
            field: "labels",
            key: json!(labels.iter().map(|(id, _)| id).collect::<Vec<_>>()),
            display: json!(labels.iter().map(|(_, name)| name).collect::<Vec<_>>()),
        },
        FieldView {
            field: "parent",
            key: parent_id,
            display: parent_display,
        },
        estimate_view(snapshot),
        schedule_view(snapshot),
    ]
}

/// `[value, unit]` when the snapshot holds an estimate.
fn estimate_view(snapshot: &Value) -> FieldView {
    let pair = snapshot
        .get("estimateValue")
        .and_then(Value::as_str)
        .zip(snapshot.get("estimateUnit").and_then(Value::as_str))
        .map_or(Value::Null, |(value, unit)| json!([value, unit]));
    FieldView {
        field: "estimate",
        key: pair.clone(),
        display: pair,
    }
}

/// `[start, duration, remaining, milestone]` as text, an empty string where a part is not set.
/// The key is the stored object itself, so any change in any part is a change.
fn schedule_view(snapshot: &Value) -> FieldView {
    let Some(object) = snapshot.get("schedule").filter(|v| !v.is_null()) else {
        return FieldView {
            field: "schedule",
            key: Value::Null,
            display: Value::Null,
        };
    };
    let text = |key: &str| match object.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    };
    let milestone = object
        .get("isScheduleMilestone")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    FieldView {
        field: "schedule",
        key: json!([text("plannedStartAt"), text("durationMinutes"), milestone]),
        display: json!([
            text("plannedStartAt"),
            text("durationMinutes"),
            if milestone { "milestone" } else { "" }
        ]),
    }
}

/// Fields whose value differs between two snapshots, with readable before and after values.
pub fn diff(from: &[FieldView], to: &[FieldView]) -> Vec<FieldChange> {
    to.iter()
        .filter_map(|after| {
            let before = from.iter().find(|item| item.field == after.field)?;
            (before.key != after.key).then(|| FieldChange {
                field: after.field.to_owned(),
                from: before.display.clone(),
                to: after.display.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Value {
        json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]})
    }

    #[test]
    fn description_text_keeps_paragraph_breaks() {
        let value = json!({"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"first"}]},
            {"type":"paragraph","content":[{"type":"text","text":"second"}]}]});
        assert_eq!(description_text(&value), "first\nsecond");
        assert_eq!(description_text(&json!({})), "");
    }

    #[test]
    fn diff_reports_only_changed_fields_with_readable_values() {
        let a = json!({"title":"A","description":doc("x"),"statusId":"11111111-1111-1111-1111-111111111111","statusName":"Todo","priority":"MEDIUM","labels":[],"assigneeId":null});
        let b = json!({"title":"A","description":doc("x"),"statusId":"22222222-2222-2222-2222-222222222222","statusName":"In Progress","priority":"HIGH","labels":[{"id":"l1","name":"backend"}],"assigneeId":"u1","assigneeName":"John"});
        let names = HashMap::new();
        let changes = diff(&view(&a, &names), &view(&b, &names));
        let fields: Vec<_> = changes.iter().map(|c| c.field.as_str()).collect();
        assert_eq!(fields, ["status", "priority", "assignee", "labels"]);
        assert_eq!(changes[0].from, json!("Todo"));
        assert_eq!(changes[0].to, json!("In Progress"));
        assert_eq!(changes[2].to, json!("John"));
        assert_eq!(changes[3].to, json!(["backend"]));
    }

    #[test]
    fn older_snapshots_without_names_use_the_workflow_lookup() {
        let id = Uuid::new_v4();
        let snapshot = json!({"title":"A","statusId":id.to_string()});
        let mut names = HashMap::new();
        names.insert(id, "Todo".to_owned());
        let fields = view(&snapshot, &names);
        let status = fields.iter().find(|f| f.field == "status").unwrap();
        assert_eq!(status.display, json!("Todo"));
    }

    #[test]
    fn estimate_and_schedule_are_fields_with_readable_values() {
        let names = HashMap::new();
        let before = json!({"title":"A"});
        let after = json!({"title":"A","estimateValue":"5","estimateUnit":"POINTS",
            "schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":480,"isScheduleMilestone":false}});
        let changes = diff(&view(&before, &names), &view(&after, &names));
        let fields: Vec<_> = changes.iter().map(|c| c.field.as_str()).collect();
        assert_eq!(fields, ["estimate", "schedule"]);
        assert_eq!(changes[0].from, Value::Null);
        assert_eq!(changes[0].to, json!(["5", "POINTS"]));
        assert_eq!(changes[1].to, json!(["2026-11-02T00:00:00Z", "480", ""]));
        // Any part of the schedule counts as a change, a milestone flag included.
        let milestone = json!({"title":"A","schedule":{"plannedStartAt":"2026-11-02T00:00:00Z","durationMinutes":480,"isScheduleMilestone":true}});
        let again = diff(&view(&after, &names), &view(&milestone, &names));
        assert_eq!(
            again.iter().map(|c| c.field.as_str()).collect::<Vec<_>>(),
            ["estimate", "schedule"]
        );
        // A snapshot written before planning values existed has neither field, which reads as unestimated and unscheduled.
        assert!(
            diff(
                &view(&before, &names),
                &view(
                    &json!({"title":"A","estimateValue":null,"schedule":null}),
                    &names
                )
            )
            .is_empty()
        );
    }

    #[test]
    fn identical_snapshots_have_no_changes_and_label_order_is_ignored() {
        let a = json!({"title":"A","labels":[{"id":"l1","name":"a"},{"id":"l2","name":"b"}]});
        let b = json!({"title":"A","labels":[{"id":"l2","name":"b"},{"id":"l1","name":"a"}]});
        let names = HashMap::new();
        assert!(diff(&view(&a, &names), &view(&b, &names)).is_empty());
    }
}
