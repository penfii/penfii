//! Rich text schema version 1. The stored value is the JSON document, never HTML.

use std::collections::HashSet;

use serde_json::{Map, Value};
use url::Url;
use uuid::Uuid;

#[derive(Debug)]
pub struct ParsedMention {
    pub occurrence_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Debug)]
pub enum RichTextError {
    Invalid(&'static str),
    MentionPolicy,
    Empty,
}

/// Matches the editor's blank-message rule while allowing a mention to count as content.
pub fn is_blank(value: &Value) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("text") => value
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(|text| text.trim().is_empty()),
        Some("mention") => false,
        _ => value
            .get("content")
            .and_then(Value::as_array)
            .is_none_or(|content| content.iter().all(is_blank)),
    }
}

pub fn validate(value: &Value, allow_empty: bool) -> Result<Vec<ParsedMention>, RichTextError> {
    let obj = object(value)?;
    require_keys(obj, &["type", "content"])?;
    if kind(obj)? != "doc" {
        return Err(RichTextError::Invalid("document type"));
    }
    let content = array(obj, "content")?;
    if content.is_empty() && !allow_empty {
        return Err(RichTextError::Empty);
    }
    let mut mentions = Vec::new();
    let mut seen = HashSet::new();
    for block in content {
        validate_block(block, &mut mentions, &mut seen)?;
    }
    Ok(mentions)
}

fn validate_block(
    value: &Value,
    mentions: &mut Vec<ParsedMention>,
    seen: &mut HashSet<Uuid>,
) -> Result<(), RichTextError> {
    let obj = object(value)?;
    match kind(obj)? {
        "paragraph" => {
            require_keys(obj, &["type", "content"])?;
            validate_inline(array(obj, "content")?, mentions, seen)
        }
        "blockquote" | "listItem" => {
            require_keys(obj, &["type", "content"])?;
            validate_flow(array(obj, "content")?, mentions, seen)
        }
        "heading" => validate_heading(obj, mentions, seen),
        "codeBlock" => validate_code_block(obj),
        "bulletList" | "orderedList" => validate_list(obj, mentions, seen),
        _ => Err(RichTextError::Invalid("unknown block")),
    }
}

fn validate_heading(
    obj: &Map<String, Value>,
    mentions: &mut Vec<ParsedMention>,
    seen: &mut HashSet<Uuid>,
) -> Result<(), RichTextError> {
    require_keys(obj, &["type", "attrs", "content"])?;
    let attrs = object(
        obj.get("attrs")
            .ok_or(RichTextError::Invalid("heading attrs"))?,
    )?;
    require_keys(attrs, &["level"])?;
    let level = attrs
        .get("level")
        .and_then(Value::as_u64)
        .ok_or(RichTextError::Invalid("heading level"))?;
    if !(1..=6).contains(&level) {
        return Err(RichTextError::Invalid("heading level"));
    }
    validate_inline(array(obj, "content")?, mentions, seen)
}

fn validate_code_block(obj: &Map<String, Value>) -> Result<(), RichTextError> {
    let mut allowed = vec!["type", "content"];
    if obj.contains_key("attrs") {
        allowed.push("attrs");
    }
    require_keys(obj, &allowed)?;
    if let Some(attrs) = obj.get("attrs") {
        let attrs = object(attrs)?;
        require_keys(attrs, &["language"])?;
        match attrs.get("language") {
            Some(Value::Null) | Some(Value::String(_)) => {}
            _ => return Err(RichTextError::Invalid("code language")),
        }
    }
    for node in array(obj, "content")? {
        let child = object(node)?;
        if kind(child)? != "text" {
            return Err(RichTextError::Invalid("code text"));
        }
        require_keys(child, &["type", "text"])?;
        if child.get("text").and_then(Value::as_str).is_none() {
            return Err(RichTextError::Invalid("text"));
        }
    }
    Ok(())
}

fn validate_list(
    obj: &Map<String, Value>,
    mentions: &mut Vec<ParsedMention>,
    seen: &mut HashSet<Uuid>,
) -> Result<(), RichTextError> {
    let ordered = kind(obj)? == "orderedList";
    let mut allowed = vec!["type", "content"];
    if ordered && obj.contains_key("attrs") {
        allowed.push("attrs");
    }
    require_keys(obj, &allowed)?;
    if ordered && let Some(attrs) = obj.get("attrs") {
        let attrs = object(attrs)?;
        require_keys(attrs, &["start"])?;
        let start = attrs
            .get("start")
            .and_then(Value::as_u64)
            .ok_or(RichTextError::Invalid("list start"))?;
        if start < 1 {
            return Err(RichTextError::Invalid("list start"));
        }
    }
    for item in array(obj, "content")? {
        let child = object(item)?;
        if kind(child)? != "listItem" {
            return Err(RichTextError::Invalid("list item"));
        }
        require_keys(child, &["type", "content"])?;
        validate_flow(array(child, "content")?, mentions, seen)?;
    }
    Ok(())
}

/// A quote or list item may hold inline text, or blocks such as a paragraph.
fn validate_flow(
    nodes: &[Value],
    mentions: &mut Vec<ParsedMention>,
    seen: &mut HashSet<Uuid>,
) -> Result<(), RichTextError> {
    for node in nodes {
        let obj = object(node)?;
        match kind(obj)? {
            "text" | "mention" => validate_inline(std::slice::from_ref(node), mentions, seen)?,
            _ => validate_block(node, mentions, seen)?,
        }
    }
    Ok(())
}

fn validate_inline(
    nodes: &[Value],
    mentions: &mut Vec<ParsedMention>,
    seen: &mut HashSet<Uuid>,
) -> Result<(), RichTextError> {
    for node in nodes {
        let obj = object(node)?;
        match kind(obj)? {
            "text" => validate_text(obj)?,
            "mention" => validate_mention(obj, mentions, seen)?,
            _ => return Err(RichTextError::Invalid("unknown inline")),
        }
    }
    Ok(())
}

fn validate_text(obj: &Map<String, Value>) -> Result<(), RichTextError> {
    if kind(obj)? != "text" {
        return Err(RichTextError::Invalid("text node"));
    }
    let mut allowed = vec!["type", "text"];
    if obj.contains_key("marks") {
        allowed.push("marks");
    }
    require_keys(obj, &allowed)?;
    if obj.get("text").and_then(Value::as_str).is_none() {
        return Err(RichTextError::Invalid("text"));
    }
    if let Some(marks) = obj.get("marks") {
        let marks = marks.as_array().ok_or(RichTextError::Invalid("marks"))?;
        for mark in marks {
            validate_mark(object(mark)?)?;
        }
    }
    Ok(())
}

fn validate_mark(obj: &Map<String, Value>) -> Result<(), RichTextError> {
    match kind(obj)? {
        "bold" | "italic" | "strike" | "code" => {
            require_keys(obj, &["type"])?;
            Ok(())
        }
        "link" => {
            require_keys(obj, &["type", "attrs"])?;
            let attrs = object(
                obj.get("attrs")
                    .ok_or(RichTextError::Invalid("link attrs"))?,
            )?;
            require_keys(attrs, &["href"])?;
            let href = attrs
                .get("href")
                .and_then(Value::as_str)
                .ok_or(RichTextError::Invalid("link href"))?;
            if !allowed_href(href) {
                return Err(RichTextError::Invalid("link href"));
            }
            Ok(())
        }
        _ => Err(RichTextError::Invalid("unknown mark")),
    }
}

fn validate_mention(
    obj: &Map<String, Value>,
    mentions: &mut Vec<ParsedMention>,
    seen: &mut HashSet<Uuid>,
) -> Result<(), RichTextError> {
    require_keys(obj, &["type", "attrs"])?;
    let attrs = object(
        obj.get("attrs")
            .ok_or(RichTextError::Invalid("mention attrs"))?,
    )?;
    let mention_kind = attrs
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(RichTextError::Invalid("mention kind"))?;
    match mention_kind {
        "HERE" | "CHANNEL" | "GROUP" | "TEAM" => return Err(RichTextError::MentionPolicy),
        "USER" => {}
        _ => return Err(RichTextError::Invalid("mention kind")),
    }
    require_keys(attrs, &["occurrenceId", "kind", "userId"])?;
    let occurrence_id = parse_uuid(attrs.get("occurrenceId"))?;
    let user_id = parse_uuid(attrs.get("userId"))?;
    if !seen.insert(occurrence_id) {
        return Err(RichTextError::Invalid("duplicate mention"));
    }
    mentions.push(ParsedMention {
        occurrence_id,
        user_id,
    });
    Ok(())
}

fn allowed_href(href: &str) -> bool {
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    url.username().is_empty() && url.password().is_none()
}

fn parse_uuid(value: Option<&Value>) -> Result<Uuid, RichTextError> {
    value
        .and_then(Value::as_str)
        .and_then(|text| Uuid::parse_str(text).ok())
        .ok_or(RichTextError::Invalid("uuid"))
}

fn object(value: &Value) -> Result<&Map<String, Value>, RichTextError> {
    value.as_object().ok_or(RichTextError::Invalid("object"))
}

fn kind(obj: &Map<String, Value>) -> Result<&str, RichTextError> {
    obj.get("type")
        .and_then(Value::as_str)
        .ok_or(RichTextError::Invalid("type"))
}

fn array<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a [Value], RichTextError> {
    obj.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or(RichTextError::Invalid("content"))
}

fn require_keys(obj: &Map<String, Value>, allowed: &[&str]) -> Result<(), RichTextError> {
    if obj.len() != allowed.len() || allowed.iter().any(|key| !obj.contains_key(*key)) {
        return Err(RichTextError::Invalid("unexpected field"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_blank, validate};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn blank_documents_match_editor_content_rules() {
        assert!(is_blank(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[]}]})
        ));
        assert!(is_blank(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"  \n\t"}]}]})
        ));
        assert!(!is_blank(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"userId":"user"}}]}]})
        ));
        assert!(!is_blank(
            &json!({"type":"doc","content":[{"type":"codeBlock","content":[{"type":"text","text":"0"}]}]})
        ));
    }

    fn paragraph(text: &str) -> serde_json::Value {
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": text}]
            }]
        })
    }

    #[test]
    fn rejects_empty_document_for_send() {
        let doc = json!({"type": "doc", "content": []});
        assert!(validate(&doc, false).is_err());
        assert!(validate(&doc, true).is_ok());
    }

    #[test]
    fn rejects_unsafe_links_and_unknown_nodes() {
        let javascript = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]
                }]
            }]
        });
        assert!(validate(&javascript, false).is_err());
        let credential = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{"type": "link", "attrs": {"href": "https://user:pass@example.com"}}]
                }]
            }]
        });
        assert!(validate(&credential, false).is_err());
        let script = json!({"type": "doc", "content": [{"type": "script"}]});
        assert!(validate(&script, false).is_err());
        assert!(validate(&paragraph("ok"), false).is_ok());
    }

    #[test]
    fn user_mention_is_inline_only() {
        let occurrence = Uuid::new_v4();
        let user = Uuid::new_v4();
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "mention",
                    "attrs": {
                        "occurrenceId": occurrence,
                        "kind": "USER",
                        "userId": user
                    }
                }]
            }]
        });
        let mentions = validate(&doc, false).unwrap();
        assert_eq!(mentions.len(), 1);
        let here = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "mention",
                    "attrs": {"kind": "HERE"}
                }]
            }]
        });
        assert!(matches!(
            validate(&here, false),
            Err(super::RichTextError::MentionPolicy)
        ));
    }

    #[test]
    fn accepts_nested_quote_list_and_optional_code_language() {
        let nested = json!({
            "type": "doc",
            "content": [
                {
                    "type": "blockquote",
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "quoted"}]
                    }]
                },
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": "item"}]
                        }]
                    }]
                },
                {
                    "type": "orderedList",
                    "attrs": {"start": 2},
                    "content": [{
                        "type": "listItem",
                        "content": [{"type": "text", "text": "legacy"}]
                    }]
                },
                {
                    "type": "codeBlock",
                    "content": [{"type": "text", "text": "plain"}]
                },
                {
                    "type": "codeBlock",
                    "attrs": {"language": null},
                    "content": [{"type": "text", "text": "also plain"}]
                },
                {
                    "type": "heading",
                    "attrs": {"level": 6},
                    "content": [{"type": "text", "text": "small"}]
                }
            ]
        });
        assert!(validate(&nested, false).is_ok());
        let heading = json!({
            "type": "doc",
            "content": [{
                "type": "heading",
                "attrs": {"level": 7},
                "content": [{"type": "text", "text": "no"}]
            }]
        });
        assert!(validate(&heading, false).is_err());
    }
}
