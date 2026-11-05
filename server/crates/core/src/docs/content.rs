//! DocContent v1: the shape of `doc_pages.content` and `doc_templates.content`.
//!
//! The Rust validator is the authority. It is used when a page is
//! created from client content, by the materializer after it has turned the Yjs state into JSON, and
//! by the search extractor. `sanitize` returns the canonical form: unknown props and styles are
//! dropped, table cells are normalised to inline arrays, and ids are made unique in document order,
//! so the same state always gives the same JSON and the same `content_hash`.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const SCHEMA: &str = "penfii.doc";
pub const VERSION: i64 = 1;

/// Nesting deeper than this is refused so a hostile document cannot overflow the stack of the
/// validator. It is a safety bound, not a product limit.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentError(pub String);

impl ContentError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One empty paragraph: what a page starts with when neither content nor a template is given.
pub fn empty() -> Value {
    json!({
        "schema": SCHEMA,
        "version": VERSION,
        "blocks": [{"id": Uuid::new_v4().to_string(), "type": "paragraph", "props": {}, "content": [], "children": []}]
    })
}

pub fn hash(content: &Value) -> String {
    // serde_json maps are ordered by key, so the serialisation is canonical.
    hex::encode(Sha256::digest(content.to_string().as_bytes()))
}

const TEXT_BLOCKS: &[&str] = &[
    "paragraph",
    "heading",
    "bulletListItem",
    "numberedListItem",
    "checkListItem",
    "quote",
    "codeBlock",
];

const STYLES_BOOL: &[&str] = &["bold", "italic", "underline", "strike", "code"];
const STYLES_STRING: &[&str] = &["textColor", "backgroundColor"];

/// What `sanitize` found besides the clean document.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Ticket, Doc and Record references and user mentions: `(kind, id)` with the
    /// `doc_page_targets` kind names `TICKET`, `DOC`, `DATABASE_RECORD`, `USER`.
    pub targets: BTreeSet<(String, Uuid)>,
    /// Files referenced by image and file blocks.
    pub files: BTreeSet<Uuid>,
}

pub fn file_url(file_id: Uuid) -> String {
    format!("/v1/files/{file_id}/content")
}

fn file_id_of(url: &str) -> Option<Uuid> {
    let rest = url.strip_prefix("/v1/files/")?.strip_suffix("/content")?;
    Uuid::parse_str(rest).ok()
}

/// A file block the editor has inserted but not finished uploading has an empty `url`. It is not
/// part of the saved page: leaving it in would make `sanitize` refuse the whole document, so a
/// transfer that is still running or that failed would stop every other edit from being stored.
/// A block whose `url` is present but not a file path is kept, and `sanitize` still refuses it.
fn without_pending_files(input: &Value) -> Value {
    fn pending(block: &Value) -> bool {
        let kind = block["type"].as_str().unwrap_or("");
        if !matches!(kind, "image" | "video" | "audio" | "file") {
            return false;
        }
        block["props"]["url"].as_str().unwrap_or("").is_empty()
    }
    fn walk(blocks: &[Value]) -> Vec<Value> {
        blocks
            .iter()
            .filter(|block| !pending(block))
            .map(|block| {
                let mut block = block.clone();
                if let Some(children) = block["children"].as_array() {
                    block["children"] = Value::Array(walk(children));
                }
                block
            })
            .collect()
    }
    let Some(blocks) = input.get("blocks").and_then(Value::as_array) else {
        return input.clone();
    };
    let mut kept = walk(blocks);
    // An already-empty page is still invalid. A page whose only blocks are unfinished uploads
    // keeps a paragraph so the rest of the document can be stored.
    if kept.is_empty() && !blocks.is_empty() {
        // Same stand-in `filter_files` uses: a page always has one block, and the id is fixed so a
        // retry of the same pending state does not write a new hash every time.
        kept.push(json!({
            "id": "empty",
            "type": "paragraph",
            "props": {},
            "content": [],
            "children": []
        }));
    }
    let mut out = input.clone();
    out["blocks"] = Value::Array(kept);
    out
}

/// Validates and canonicalises a document. `max_bytes` bounds the canonical JSON.
pub fn sanitize(input: &Value, max_bytes: usize) -> Result<(Value, Extracted), ContentError> {
    let prepared = without_pending_files(input);
    let object = prepared
        .as_object()
        .ok_or_else(|| ContentError::new("content must be an object"))?;
    if object.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(ContentError::new("content.schema must be penfii.doc"));
    }
    if object.get("version").and_then(Value::as_i64) != Some(VERSION) {
        return Err(ContentError::new("content.version must be 1"));
    }
    let blocks = object
        .get("blocks")
        .and_then(Value::as_array)
        .ok_or_else(|| ContentError::new("content.blocks must be an array"))?;
    if blocks.is_empty() {
        return Err(ContentError::new("content must have at least one block"));
    }
    let mut state = State::default();
    let blocks = blocks_of(blocks, 0, &mut state)?;
    let content = json!({"schema": SCHEMA, "version": VERSION, "blocks": blocks});
    if content.to_string().len() > max_bytes {
        return Err(ContentError::new(
            "content is larger than the configured limit",
        ));
    }
    Ok((content, state.found))
}

#[derive(Default)]
struct State {
    ids: BTreeSet<String>,
    found: Extracted,
}

fn blocks_of(
    blocks: &[Value],
    depth: usize,
    state: &mut State,
) -> Result<Vec<Value>, ContentError> {
    if depth > MAX_DEPTH {
        return Err(ContentError::new("blocks are nested too deeply"));
    }
    blocks.iter().map(|b| block_of(b, depth, state)).collect()
}

fn unique_id(raw: &str, state: &mut State) -> String {
    let base: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    let base = if base.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        base
    };
    let mut id = base.clone();
    let mut n = 1;
    while !state.ids.insert(id.clone()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn block_of(value: &Value, depth: usize, state: &mut State) -> Result<Value, ContentError> {
    let object = value
        .as_object()
        .ok_or_else(|| ContentError::new("a block must be an object"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ContentError::new("a block needs a type"))?;
    let raw_id = object.get("id").and_then(Value::as_str).unwrap_or("");
    let id = unique_id(raw_id, state);
    let props = object.get("props").and_then(Value::as_object);
    let mut out_props = Map::new();
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("type".into(), json!(kind));
    match kind {
        k if TEXT_BLOCKS.contains(&k) => {
            match k {
                "heading" => {
                    let level = props
                        .and_then(|p| p.get("level"))
                        .and_then(Value::as_i64)
                        .unwrap_or(1);
                    if !(1..=3).contains(&level) {
                        return Err(ContentError::new("heading level is 1 to 3"));
                    }
                    out_props.insert("level".into(), json!(level));
                }
                "checkListItem" => {
                    let checked = props
                        .and_then(|p| p.get("checked"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    out_props.insert("checked".into(), json!(checked));
                }
                "codeBlock" => {
                    let language = props
                        .and_then(|p| p.get("language"))
                        .and_then(Value::as_str)
                        .unwrap_or("text");
                    let language: String = language
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || "+-_#.".contains(*c))
                        .take(40)
                        .collect();
                    out_props.insert("language".into(), json!(language));
                }
                _ => {}
            }
            let inline = match object.get("content") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => inline_of(items, k == "codeBlock", state)?,
                Some(Value::String(text)) => inline_of(
                    &[json!({"type": "text", "text": text, "styles": {}})],
                    k == "codeBlock",
                    state,
                )?,
                Some(_) => return Err(ContentError::new("block content must be an array")),
            };
            out.insert("content".into(), Value::Array(inline));
        }
        "table" => {
            let table = object
                .get("content")
                .and_then(Value::as_object)
                .ok_or_else(|| ContentError::new("a table needs tableContent"))?;
            let rows = table
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| ContentError::new("a table needs rows"))?;
            if rows.is_empty() {
                return Err(ContentError::new("a table needs at least one row"));
            }
            let mut out_rows = Vec::new();
            for row in rows {
                let cells = row
                    .get("cells")
                    .and_then(Value::as_array)
                    .ok_or_else(|| ContentError::new("a table row needs cells"))?;
                if cells.is_empty() {
                    return Err(ContentError::new("a table row needs at least one cell"));
                }
                let mut out_cells = Vec::new();
                for cell in cells {
                    // BlockNote writes `{type:"tableCell", content:[...], props:{...}}`; the
                    // canonical form keeps only the inline content.
                    let inline = match cell {
                        Value::Array(items) => items.clone(),
                        Value::Object(o) => match o.get("content") {
                            Some(Value::Array(items)) => items.clone(),
                            None | Some(Value::Null) => Vec::new(),
                            Some(_) => {
                                return Err(ContentError::new(
                                    "table cell content must be an array",
                                ));
                            }
                        },
                        _ => return Err(ContentError::new("a table cell must be an array")),
                    };
                    out_cells.push(Value::Array(inline_of(&inline, false, state)?));
                }
                out_rows.push(json!({"cells": out_cells}));
            }
            out.insert(
                "content".into(),
                json!({"type": "tableContent", "rows": out_rows}),
            );
        }
        "image" | "video" | "audio" | "file" => {
            let props = props.ok_or_else(|| ContentError::new("a file block needs props"))?;
            let url = props.get("url").and_then(Value::as_str).unwrap_or("");
            let file = file_id_of(url).ok_or_else(|| {
                ContentError::new("an image or file url must be the file's download path")
            })?;
            state.found.files.insert(file);
            out_props.insert("url".into(), json!(file_url(file)));
            out_props.insert("name".into(), json!(string_prop(props, "name", 255)));
            out_props.insert("caption".into(), json!(string_prop(props, "caption", 1000)));
            if matches!(kind, "image" | "video" | "audio") {
                if let Some(width) = props.get("previewWidth").and_then(Value::as_f64)
                    && width.is_finite()
                    && (1.0..=10000.0).contains(&width)
                {
                    out_props.insert("previewWidth".into(), json!(width.round() as i64));
                }
                let show = props
                    .get("showPreview")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                out_props.insert("showPreview".into(), json!(show));
            }
        }
        "divider" => {}
        other => {
            return Err(ContentError::new(format!(
                "block type {other} is not allowed"
            )));
        }
    }
    out.insert("props".into(), Value::Object(out_props));
    let children = match object.get("children") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => blocks_of(items, depth + 1, state)?,
        Some(_) => return Err(ContentError::new("children must be an array")),
    };
    out.insert("children".into(), Value::Array(children));
    Ok(Value::Object(out))
}

fn string_prop(props: &Map<String, Value>, key: &str, max_chars: usize) -> String {
    props
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(max_chars)
        .collect()
}

fn styles_of(value: Option<&Value>) -> Value {
    let mut out = Map::new();
    if let Some(map) = value.and_then(Value::as_object) {
        for key in STYLES_BOOL {
            if map.get(*key).and_then(Value::as_bool) == Some(true) {
                out.insert((*key).into(), json!(true));
            }
        }
        for key in STYLES_STRING {
            if let Some(color) = map.get(*key).and_then(Value::as_str)
                && !color.is_empty()
                && color != "default"
                && color.len() <= 32
                && color
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '#' || c == '-')
            {
                out.insert((*key).into(), json!(color));
            }
        }
    }
    Value::Object(out)
}

fn safe_href(href: &str) -> bool {
    let href = href.trim();
    if href.is_empty() || href.len() > 2048 || href.chars().any(char::is_control) {
        return false;
    }
    href.starts_with("https://")
        || href.starts_with("http://")
        || href.starts_with("mailto:")
        || (href.starts_with('/') && !href.starts_with("//"))
}

fn inline_of(items: &[Value], plain: bool, state: &mut State) -> Result<Vec<Value>, ContentError> {
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        let object = item
            .as_object()
            .ok_or_else(|| ContentError::new("inline content must be objects"))?;
        match object.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = object.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    continue;
                }
                let styles = if plain {
                    json!({})
                } else {
                    styles_of(object.get("styles"))
                };
                // Adjacent runs with equal styles are one run, so the same text always has the
                // same shape whatever order the edits arrived in.
                if let Some(last) = out.last_mut()
                    && last["type"] == "text"
                    && last["styles"] == styles
                {
                    let joined = format!("{}{}", last["text"].as_str().unwrap_or(""), text);
                    last["text"] = json!(joined);
                    continue;
                }
                out.push(json!({"type": "text", "text": text, "styles": styles}));
            }
            Some("link") if !plain => {
                let href = object.get("href").and_then(Value::as_str).unwrap_or("");
                if !safe_href(href) {
                    return Err(ContentError::new(
                        "a link must be http, https, mailto or a relative path",
                    ));
                }
                let inner = match object.get("content") {
                    Some(Value::Array(children)) => children.clone(),
                    _ => Vec::new(),
                };
                let mut texts = Vec::new();
                for child in inner {
                    if child.get("type").and_then(Value::as_str) != Some("text") {
                        return Err(ContentError::new("a link holds text only"));
                    }
                    texts.extend(inline_of(&[child], false, state)?);
                }
                out.push(json!({"type": "link", "href": href.trim(), "content": texts}));
            }
            Some("mention") if !plain => {
                let user = uuid_prop(object, "userId")?;
                state.found.targets.insert(("USER".into(), user));
                out.push(json!({"type": "mention", "props": {"userId": user}}));
            }
            Some("reference") if !plain => {
                let props = object.get("props").and_then(Value::as_object);
                let kind = props
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let id = props
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| ContentError::new("a reference needs an id"))?;
                let target = match kind {
                    "TICKET" => "TICKET",
                    "DOC" => "DOC",
                    "RECORD" => "DATABASE_RECORD",
                    _ => return Err(ContentError::new("a reference is a TICKET, DOC or RECORD")),
                };
                state.found.targets.insert((target.into(), id));
                out.push(json!({"type": "reference", "props": {"kind": kind, "id": id}}));
            }
            _ => return Err(ContentError::new("inline content type is not allowed")),
        }
    }
    Ok(out)
}

fn uuid_prop(object: &Map<String, Value>, key: &str) -> Result<Uuid, ContentError> {
    let from_props = object
        .get("props")
        .and_then(Value::as_object)
        .and_then(|p| p.get(key))
        .and_then(Value::as_str);
    from_props
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| ContentError::new(format!("{key} must be a uuid")))
}

/// Removes image and file blocks (with their children) whose file `keep` refuses. Applied before
/// `sanitize`, so it works on raw or canonical JSON alike.
pub fn filter_files(content: &Value, keep: &dyn Fn(Uuid) -> bool) -> Value {
    fn walk(blocks: &[Value], keep: &dyn Fn(Uuid) -> bool) -> Vec<Value> {
        blocks
            .iter()
            .filter(|block| {
                let kind = block["type"].as_str().unwrap_or("");
                if !matches!(kind, "image" | "video" | "audio" | "file") {
                    return true;
                }
                block["props"]["url"]
                    .as_str()
                    .and_then(file_id_of)
                    .is_some_and(keep)
            })
            .map(|block| {
                let mut block = block.clone();
                if let Some(children) = block["children"].as_array() {
                    block["children"] = Value::Array(walk(children, keep));
                }
                block
            })
            .collect()
    }
    let mut out = content.clone();
    if let Some(blocks) = content["blocks"].as_array() {
        let mut kept = walk(blocks, keep);
        if kept.is_empty() {
            // A document keeps at least one block.
            kept.push(json!({"id": Uuid::new_v4().to_string(), "type": "paragraph", "props": {}, "content": [], "children": []}));
        }
        out["blocks"] = Value::Array(kept);
    }
    out
}

/// A template carries no attachments.
pub fn strip_files(content: &Value) -> Value {
    filter_files(content, &|_| false)
}

// ---------------------------------------------------------------------------------------------
// Plain text for search (section G.4)
// ---------------------------------------------------------------------------------------------

fn inline_text(items: &[Value], out: &mut String) {
    for item in items {
        match item["type"].as_str() {
            Some("text") => out.push_str(item["text"].as_str().unwrap_or("")),
            Some("link") => {
                if let Some(children) = item["content"].as_array() {
                    inline_text(children, out);
                }
            }
            // Mentions and references carry ids only: nothing is copied in, ever.
            _ => {}
        }
    }
}

fn collect_text(blocks: &[Value], lines: &mut Vec<String>) {
    for block in blocks {
        let kind = block["type"].as_str().unwrap_or("");
        if TEXT_BLOCKS.contains(&kind)
            && let Some(items) = block["content"].as_array()
        {
            let mut line = String::new();
            inline_text(items, &mut line);
            if !line.is_empty() {
                lines.push(line);
            }
        }
        // Table cells, image/file names and captions are attribute-held text: a known limitation.
        if let Some(children) = block["children"].as_array() {
            collect_text(children, lines);
        }
    }
}

/// Plain text of a materialized document, blocks joined by `\n`, cut at `max_bytes` on a char
/// boundary. The second value says the text was cut.
pub fn plain_text(content: &Value, max_bytes: usize) -> (String, bool) {
    let mut lines = Vec::new();
    if let Some(blocks) = content["blocks"].as_array() {
        collect_text(blocks, &mut lines);
    }
    let mut text = lines.join("\n");
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut cut = max_bytes;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    (text, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(blocks: Value) -> Value {
        json!({"schema": SCHEMA, "version": 1, "blocks": blocks})
    }

    #[test]
    fn unknown_props_and_styles_are_dropped_and_cells_are_normalised() {
        let input = doc(json!([
            {"id":"a","type":"paragraph","props":{"textAlignment":"center","textColor":"red"},
             "content":[{"type":"text","text":"안녕","styles":{"bold":true,"fontSize":"20px","textColor":"red"}},
                        {"type":"text","text":"하세요","styles":{"bold":true,"textColor":"red"}}]},
            {"id":"b","type":"table","content":{"type":"tableContent","rows":[
                {"cells":[{"type":"tableCell","content":[{"type":"text","text":"x","styles":{}}],"props":{"colspan":2}}]}]}}
        ]));
        let (clean, _) = sanitize(&input, 1 << 20).unwrap();
        let p = &clean["blocks"][0];
        assert_eq!(p["props"], json!({}));
        assert_eq!(
            p["content"].as_array().unwrap().len(),
            1,
            "equal styles are one run"
        );
        assert_eq!(p["content"][0]["text"], "안녕하세요");
        assert_eq!(
            p["content"][0]["styles"],
            json!({"bold": true, "textColor": "red"})
        );
        assert_eq!(
            clean["blocks"][1]["content"]["rows"][0]["cells"][0][0]["text"],
            "x"
        );
        // Canonical: sanitising the output changes nothing.
        let (again, _) = sanitize(&clean, 1 << 20).unwrap();
        assert_eq!(again, clean);
        assert_eq!(hash(&again), hash(&clean));
    }

    #[test]
    fn duplicate_ids_are_made_unique_in_document_order() {
        let input = doc(json!([
            {"id":"x","type":"paragraph","content":[]},
            {"id":"x","type":"paragraph","content":[],"children":[{"id":"x","type":"paragraph","content":[]}]}
        ]));
        let (clean, _) = sanitize(&input, 1 << 20).unwrap();
        assert_eq!(clean["blocks"][0]["id"], "x");
        assert_eq!(clean["blocks"][1]["id"], "x-1");
        assert_eq!(clean["blocks"][1]["children"][0]["id"], "x-2");
    }

    #[test]
    fn a_file_block_that_is_still_uploading_is_left_out_and_the_text_stays() {
        let input = doc(json!([
            {"id":"a","type":"paragraph","content":[{"type":"text","text":"hello","styles":{}}]},
            {"id":"b","type":"image","props":{"url":"","name":"shot.png"}},
            {"id":"c","type":"file","props":{"name":"notes.pdf"},"children":[
                {"id":"d","type":"paragraph","content":[{"type":"text","text":"inside","styles":{}}]}
            ]}
        ]));
        let (clean, found) = sanitize(&input, 1 << 20).unwrap();
        assert!(found.files.is_empty());
        assert_eq!(clean["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(clean["blocks"][0]["content"][0]["text"], "hello");
        // The same pending page sanitises to the same bytes, so a retry does not write again.
        let (again, _) = sanitize(&input, 1 << 20).unwrap();
        assert_eq!(again, clean);
    }

    #[test]
    fn disallowed_content_is_refused() {
        for bad in [
            json!([{"id":"a","type":"toggleListItem","content":[]}]),
            json!([{"id":"a","type":"paragraph","content":[{"type":"link","href":"javascript:alert(1)","content":[]}]}]),
            json!([{"id":"a","type":"paragraph","content":[{"type":"mention","props":{"userId":"nope"}}]}]),
            json!([{"id":"a","type":"image","props":{"url":"https://evil.example/x.png"}}]),
            json!([{"id":"a","type":"heading","props":{"level":4},"content":[]}]),
            json!([]),
        ] {
            assert!(sanitize(&doc(bad.clone()), 1 << 20).is_err(), "{bad}");
        }
    }

    #[test]
    fn references_and_files_are_collected_and_contribute_no_text() {
        let ticket = Uuid::new_v4();
        let user = Uuid::new_v4();
        let file = Uuid::new_v4();
        let input = doc(json!([
            {"id":"a","type":"paragraph","content":[
                {"type":"text","text":"before "},
                {"type":"reference","props":{"kind":"TICKET","id":ticket}},
                {"type":"mention","props":{"userId":user}},
                {"type":"link","href":"https://example.com","content":[{"type":"text","text":"링크 글자"}]}]},
            {"id":"b","type":"image","props":{"url":file_url(file),"name":"secret-name.png","caption":"secret caption"}},
            {"id":"c","type":"table","content":{"type":"tableContent","rows":[{"cells":[[{"type":"text","text":"tablecell"}]]}]}},
            {"id":"d","type":"codeBlock","props":{"language":"bash"},"content":[{"type":"text","text":"kubectl apply -f deploy.yaml"}]},
            {"id":"e","type":"divider"}
        ]));
        let (clean, found) = sanitize(&input, 1 << 20).unwrap();
        assert!(found.targets.contains(&("TICKET".into(), ticket)));
        assert!(found.targets.contains(&("USER".into(), user)));
        assert!(found.files.contains(&file));
        let (text, cut) = plain_text(&clean, 1 << 20);
        assert!(!cut);
        assert_eq!(text, "before 링크 글자\nkubectl apply -f deploy.yaml");
        assert!(!text.contains("secret") && !text.contains("tablecell"));
    }

    #[test]
    fn plain_text_is_cut_on_a_char_boundary() {
        let input = doc(
            json!([{"id":"a","type":"paragraph","content":[{"type":"text","text":"가나다라마바사"}]}]),
        );
        let (clean, _) = sanitize(&input, 1 << 20).unwrap();
        let (text, cut) = plain_text(&clean, 10);
        assert!(cut);
        assert_eq!(text, "가나다");
    }
}

#[cfg(test)]
mod media_block_tests {
    use super::*;

    #[test]
    fn video_and_audio_blocks_name_files_like_images_and_nothing_else() {
        let file = Uuid::new_v4();
        let page = |kind: &str, url: String| {
            json!({"schema":"penfii.doc","version":1,"blocks":[
                {"id":"m","type":kind,"props":{"url":url,"name":"clip","caption":"c","showPreview":true,"previewWidth":320,"onclick":"x"},"children":[]}]})
        };
        for kind in ["video", "audio"] {
            let (clean, found) = sanitize(&page(kind, file_url(file)), 1 << 20).unwrap();
            assert!(found.files.contains(&file), "{kind}");
            assert_eq!(clean["blocks"][0]["props"]["url"], file_url(file));
            assert!(clean["blocks"][0]["props"].get("onclick").is_none());
            // Only the file's own download path is a source.
            assert!(sanitize(&page(kind, "https://evil.example/x.mp4".into()), 1 << 20).is_err());
        }
        // A file the viewer may not use is dropped from the page, whatever kind of block holds it.
        let (kept, _) = sanitize(&page("video", file_url(file)), 1 << 20).unwrap();
        let filtered = filter_files(&kept, &|_| false);
        assert_eq!(filtered["blocks"][0]["type"], "paragraph");
    }
}
