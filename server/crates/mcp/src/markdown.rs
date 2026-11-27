//! Content input for the MCP write tools (FEA-188/189): the `{format, value}` wrapper and the
//! conversion of Markdown and plain text into the two stored shapes, rich text (tickets, chat,
//! comments) and Doc blocks.
//!
//! The wrapper is an input form only. It is turned into the canonical JSON before anything reaches
//! `core`, which validates that JSON exactly as it does for the web app; nothing here relaxes a
//! validator. What a target cannot express is kept as literal text, so no content is lost, and
//! nothing that could run or notify is produced: raw HTML stays text, an image stays its Markdown
//! source (nothing is downloaded or attached), a link the target refuses stays `[text](url)`, and
//! no mention is ever made from text.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::tools::Failure;

/// Largest string accepted for rich text (UTF-8 bytes), as for plain text before.
pub(crate) const RICH_TEXT_MAX_BYTES: usize = 20_000;
/// Largest Markdown or plain string accepted for a Doc (characters, the legacy `markdown` limit).
pub(crate) const DOC_TEXT_MAX_CHARS: usize = 200_000;
/// Containers (quotes, lists, items) nested deeper than this are refused before they are built.
const MAX_NESTING: usize = 16;
/// Blocks and list items a converted text may produce.
const MAX_BLOCKS: usize = 10_000;

/// Which stored shape a conversion produces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    RichText,
    Doc,
}

/// A parsed `{format, value}` wrapper.
pub(crate) enum Content<'a> {
    Plain(&'a str),
    Markdown(&'a str),
    Richtext(&'a Map<String, Value>),
}

/// Whether an object is meant as the new wrapper: it has a `format` or a `value` key. No stored
/// rich-text or Doc root has either, so existing JSON is never reinterpreted.
pub(crate) fn is_wrapper(object: &Map<String, Value>) -> bool {
    object.contains_key("format") || object.contains_key("value")
}

/// Strictly parses a wrapper: exactly the keys `format` and `value`, a lower-case format, and a
/// value of the type the format needs.
pub(crate) fn parse_wrapper<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<Content<'a>, Failure> {
    if let Some(extra) = object
        .keys()
        .find(|key| *key != "format" && *key != "value")
    {
        return Err(Failure::invalid(format!(
            "{name} has the key {extra}: an input object has exactly format and value, and is not mixed with a document root"
        )));
    }
    let format = match object.get("format") {
        Some(Value::String(format)) => format.as_str(),
        Some(_) => {
            return Err(Failure::invalid(format!(
                "{name}.format must be plain, markdown or richtext"
            )));
        }
        None => {
            return Err(Failure::invalid(format!(
                "{name}.format is required (plain, markdown or richtext)"
            )));
        }
    };
    let value = match object.get("value") {
        None | Some(Value::Null) => {
            return Err(Failure::invalid(format!("{name}.value is required")));
        }
        Some(value) => value,
    };
    match format {
        "plain" | "markdown" => {
            let text = value.as_str().ok_or_else(|| {
                Failure::invalid(format!("{name}.value must be a string for format {format}"))
            })?;
            Ok(if format == "plain" {
                Content::Plain(text)
            } else {
                Content::Markdown(text)
            })
        }
        "richtext" => match value {
            Value::Object(inner) if is_wrapper(inner) => Err(Failure::invalid(format!(
                "{name}.value must be the document JSON itself, not another format object"
            ))),
            Value::Object(inner) => Ok(Content::Richtext(inner)),
            _ => Err(Failure::invalid(format!(
                "{name}.value must be a JSON object for format richtext"
            ))),
        },
        _ => Err(Failure::invalid(format!(
            "{name}.format must be plain, markdown or richtext (lower case)"
        ))),
    }
}

/// A one-paragraph-per-blank-line document: the plain-text form of rich text.
pub(crate) fn plain_to_rich_text(text: &str) -> Value {
    json!({"type": "doc", "content": text
        .split("\n\n")
        .map(|paragraph| json!({"type": "paragraph", "content": [{"type": "text", "text": paragraph}]}))
        .collect::<Vec<_>>()})
}

/// Rich-text input from a string: shared by the legacy plain string and `format: plain`.
pub(crate) fn rich_text_from_plain(text: &str, name: &str) -> Result<Value, Failure> {
    check_rich_source(text, name)?;
    Ok(plain_to_rich_text(text))
}

pub(crate) fn rich_text_from_markdown(text: &str, name: &str) -> Result<Value, Failure> {
    check_rich_source(text, name)?;
    markdown_to_rich_text(text)
}

pub(crate) fn check_rich_source(text: &str, name: &str) -> Result<(), Failure> {
    if text.trim().is_empty() {
        return Err(Failure::invalid(format!("{name} must not be empty")));
    }
    if text.len() > RICH_TEXT_MAX_BYTES {
        return Err(Failure::invalid(format!("{name} is too long")));
    }
    Ok(())
}

/// Doc content from `format: plain`: line breaks and symbols are kept as written; blank lines
/// start a new paragraph. Empty text is the canonical empty document.
pub(crate) fn plain_to_doc(text: &str) -> Result<Value, Failure> {
    check_doc_source(text)?;
    let text = text.replace("\r\n", "\n");
    if text.trim().is_empty() {
        return Ok(empty_doc());
    }
    let blocks: Vec<Value> = text
        .split("\n\n")
        .map(|paragraph| {
            doc_block(
                "paragraph",
                json!({}),
                doc_text(paragraph, &Marks::default()),
            )
        })
        .collect();
    Ok(doc_root(blocks, &text))
}

pub(crate) fn markdown_to_doc(text: &str) -> Result<Value, Failure> {
    check_doc_source(text)?;
    let blocks = parse(text, Target::Doc)?;
    let mut out = Vec::new();
    for block in &blocks {
        doc_blocks(block, &mut out);
    }
    if out.is_empty() {
        return Ok(if text.trim().is_empty() {
            empty_doc()
        } else {
            // Nothing that renders (for example only link definitions): keep the text.
            doc_root(
                vec![doc_block(
                    "paragraph",
                    json!({}),
                    doc_text(text.trim_end(), &Marks::default()),
                )],
                text,
            )
        });
    }
    Ok(doc_root(out, text))
}

pub(crate) fn markdown_to_rich_text(text: &str) -> Result<Value, Failure> {
    let blocks = parse(text, Target::RichText)?;
    let mut out = Vec::new();
    rich_blocks(&blocks, &mut out);
    if out.is_empty() {
        out.push(rich_paragraph(vec![Run::plain(text.trim_end())]));
    }
    Ok(json!({"type": "doc", "content": out}))
}

fn check_doc_source(text: &str) -> Result<(), Failure> {
    if text.chars().count() > DOC_TEXT_MAX_CHARS {
        return Err(Failure::invalid("content value is too long"));
    }
    Ok(())
}

fn empty_doc() -> Value {
    let mut doc = penfii_core::docs::content::empty();
    identify(&mut doc, "");
    doc
}

fn doc_root(blocks: Vec<Value>, source: &str) -> Value {
    let mut doc = json!({"schema": "penfii.doc", "version": 1, "blocks": blocks});
    identify(&mut doc, source);
    doc
}

/// Gives every block an id that follows from the source text and the block's place, not from chance. The
/// same input is then the same Doc every time, which an idempotent retry needs (the request is compared as
/// converted: a call that is sent again with the same key must be the same request), and two pages made from
/// different text do not share ids.
fn identify(doc: &mut Value, source: &str) {
    use sha2::{Digest, Sha256};
    fn walk(blocks: &mut [Value], seed: &[u8], next: &mut u32) {
        for block in blocks {
            let mut hasher = Sha256::new();
            hasher.update(seed);
            hasher.update(next.to_be_bytes());
            *next += 1;
            let bytes: [u8; 16] = hasher.finalize()[..16].try_into().expect("16 bytes");
            block["id"] = json!(Uuid::from_bytes(bytes).to_string());
            if let Some(children) = block["children"].as_array_mut() {
                walk(children, seed, next);
            }
        }
    }
    let seed = Sha256::digest(source.as_bytes());
    let mut next = 0;
    if let Some(blocks) = doc["blocks"].as_array_mut() {
        walk(blocks, &seed, &mut next);
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing into a small intermediate tree
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Default, PartialEq, Eq)]
struct Marks {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    href: Option<String>,
}

#[derive(Clone)]
struct Run {
    text: String,
    marks: Marks,
}

impl Run {
    fn plain(text: &str) -> Self {
        Self {
            text: text.to_string(),
            marks: Marks::default(),
        }
    }
}

enum Block {
    Paragraph(Vec<Run>),
    Heading(u8, Vec<Run>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Code {
        language: String,
        text: String,
    },
    Rule,
    /// Source the target cannot express (a table, raw HTML), kept as written.
    Literal(String),
}

struct Item {
    checked: Option<bool>,
    blocks: Vec<Block>,
}

enum Kind {
    Root,
    Quote,
    List(Option<u64>),
    Item(Option<bool>),
}

struct Frame {
    kind: Kind,
    blocks: Vec<Block>,
    items: Vec<Item>,
}

struct Builder {
    target: Target,
    stack: Vec<Frame>,
    runs: Vec<Run>,
    heading: Option<u8>,
    code: Option<(String, String)>,
    html: Option<String>,
    in_table: bool,
    bold: u32,
    italic: u32,
    strike: u32,
    /// One entry per open link: the href it applies (`None` for a link the target refuses).
    links: Vec<Option<String>>,
    /// One entry per open image: where it points, for the closing text.
    images: Vec<String>,
    /// Closing text for a refused link, per open refused link.
    refused: Vec<String>,
    count: usize,
}

fn too_big() -> Failure {
    Failure::invalid("the Markdown has too many blocks")
}

impl Builder {
    fn new(target: Target) -> Self {
        Self {
            target,
            stack: vec![Frame {
                kind: Kind::Root,
                blocks: Vec::new(),
                items: Vec::new(),
            }],
            runs: Vec::new(),
            heading: None,
            code: None,
            html: None,
            in_table: false,
            bold: 0,
            italic: 0,
            strike: 0,
            links: Vec::new(),
            images: Vec::new(),
            refused: Vec::new(),
            count: 0,
        }
    }

    fn marks(&self) -> Marks {
        Marks {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code: false,
            href: self.links.iter().rev().find_map(Clone::clone),
        }
    }

    fn push_text(&mut self, text: &str) {
        if !text.is_empty() {
            let marks = self.marks();
            self.runs.push(Run {
                text: text.to_string(),
                marks,
            });
        }
    }

    fn push_literal_run(&mut self, text: String) {
        self.runs.push(Run {
            text,
            marks: Marks::default(),
        });
    }

    fn push_block(&mut self, block: Block) -> Result<(), Failure> {
        self.count += 1;
        if self.count > MAX_BLOCKS {
            return Err(too_big());
        }
        self.stack
            .last_mut()
            .expect("the root frame stays")
            .blocks
            .push(block);
        Ok(())
    }

    /// Text that arrived without a paragraph around it (a tight list item) becomes a paragraph.
    fn flush(&mut self) -> Result<(), Failure> {
        if self.runs.iter().any(|run| !run.text.is_empty()) {
            let runs = std::mem::take(&mut self.runs);
            self.push_block(Block::Paragraph(runs))?;
        } else {
            self.runs.clear();
        }
        Ok(())
    }

    fn open(&mut self, kind: Kind) -> Result<(), Failure> {
        if self.stack.len() > MAX_NESTING {
            return Err(Failure::invalid("the Markdown is nested too deeply"));
        }
        self.stack.push(Frame {
            kind,
            blocks: Vec::new(),
            items: Vec::new(),
        });
        Ok(())
    }

    fn close(&mut self) -> Frame {
        self.stack.pop().expect("a frame was opened")
    }
}

fn parse(source: &str, target: Target) -> Result<Vec<Block>, Failure> {
    let source = source.replace("\r\n", "\n");
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder::new(target);
    for (event, range) in Parser::new_ext(&source, options).into_offset_iter() {
        if b.in_table {
            if matches!(event, Event::End(TagEnd::Table)) {
                b.in_table = false;
            }
            continue;
        }
        match event {
            Event::Start(tag) => start(&mut b, tag, &source[range])?,
            Event::End(tag) => end(&mut b, tag)?,
            Event::Text(text) => {
                if let Some((_, code)) = &mut b.code {
                    code.push_str(&text);
                } else {
                    b.push_text(&text);
                }
            }
            Event::Code(text) => {
                let mut marks = b.marks();
                marks.code = true;
                b.runs.push(Run {
                    text: text.to_string(),
                    marks,
                });
            }
            Event::Html(html) => match &mut b.html {
                Some(buffer) => buffer.push_str(&html),
                None => b.push_literal_run(html.to_string()),
            },
            Event::InlineHtml(html) => b.push_literal_run(html.to_string()),
            Event::SoftBreak => {
                let text = if target == Target::Doc { " " } else { "\n" };
                b.push_text(text);
            }
            Event::HardBreak => b.push_text("\n"),
            Event::Rule => {
                b.flush()?;
                b.push_block(Block::Rule)?;
            }
            Event::TaskListMarker(checked) => {
                if let Some(Frame {
                    kind: Kind::Item(state),
                    ..
                }) = b.stack.last_mut()
                {
                    *state = Some(checked);
                }
            }
            // Everything else the parser can report is not enabled; keep its source if it appears.
            other => {
                let text = match other {
                    Event::FootnoteReference(text)
                    | Event::InlineMath(text)
                    | Event::DisplayMath(text) => text.to_string(),
                    _ => String::new(),
                };
                b.push_literal_run(text);
            }
        }
    }
    b.flush()?;
    // An unterminated container cannot happen with this parser; fold any left frames defensively.
    while b.stack.len() > 1 {
        let frame = b.close();
        for block in frame.blocks {
            b.push_block(block)?;
        }
    }
    Ok(b.close().blocks)
}

fn start(b: &mut Builder, tag: Tag<'_>, source: &str) -> Result<(), Failure> {
    match tag {
        Tag::Paragraph => b.flush()?,
        Tag::Heading { level, .. } => {
            b.flush()?;
            b.heading = Some(level as u8);
        }
        Tag::BlockQuote(_) => {
            b.flush()?;
            b.open(Kind::Quote)?;
        }
        Tag::CodeBlock(kind) => {
            b.flush()?;
            let language = match kind {
                CodeBlockKind::Fenced(info) => info
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                CodeBlockKind::Indented => String::new(),
            };
            b.code = Some((language, String::new()));
        }
        Tag::List(start) => {
            b.flush()?;
            b.open(Kind::List(start))?;
        }
        Tag::Item => {
            b.flush()?;
            b.open(Kind::Item(None))?;
        }
        Tag::Table(_) => {
            b.flush()?;
            b.in_table = true;
            let literal = source.trim_end().to_string();
            b.push_block(Block::Literal(literal))?;
        }
        Tag::HtmlBlock => {
            b.flush()?;
            b.html = Some(String::new());
        }
        Tag::Emphasis => b.italic += 1,
        Tag::Strong => b.bold += 1,
        Tag::Strikethrough => b.strike += 1,
        Tag::Link { dest_url, .. } => {
            let allowed = match b.target {
                Target::RichText => penfii_core::conversation::rich_text_link_allowed(&dest_url),
                Target::Doc => penfii_core::docs::content::safe_href(&dest_url),
            };
            if allowed {
                b.links.push(Some(dest_url.to_string()));
            } else {
                // The target refuses this link: the Markdown source stays as text.
                b.links.push(None);
                b.refused.push(format!("]({dest_url})"));
                b.push_literal_run("[".to_string());
            }
        }
        Tag::Image {
            dest_url, title, ..
        } => {
            let mut closing = format!("]({dest_url}");
            if !title.is_empty() {
                closing.push_str(&format!(" \"{title}\""));
            }
            closing.push(')');
            b.images.push(closing);
            b.push_literal_run("![".to_string());
        }
        // Not enabled in the options; ignored if ever reported.
        _ => {}
    }
    Ok(())
}

fn end(b: &mut Builder, tag: TagEnd) -> Result<(), Failure> {
    match tag {
        TagEnd::Paragraph => b.flush()?,
        TagEnd::Heading(_) => {
            let level = b.heading.take().unwrap_or(1);
            let runs = std::mem::take(&mut b.runs);
            b.push_block(Block::Heading(level, runs))?;
        }
        TagEnd::BlockQuote(_) => {
            b.flush()?;
            let frame = b.close();
            b.push_block(Block::Quote(frame.blocks))?;
        }
        TagEnd::CodeBlock => {
            if let Some((language, mut text)) = b.code.take() {
                if text.ends_with('\n') {
                    text.pop();
                }
                b.push_block(Block::Code { language, text })?;
            }
        }
        TagEnd::List(_) => {
            let frame = b.close();
            let Kind::List(start) = frame.kind else {
                return Ok(());
            };
            b.push_block(Block::List {
                start,
                items: frame.items,
            })?;
        }
        TagEnd::Item => {
            b.flush()?;
            let frame = b.close();
            let Kind::Item(checked) = frame.kind else {
                return Ok(());
            };
            b.count += 1;
            if b.count > MAX_BLOCKS {
                return Err(too_big());
            }
            if let Some(list) = b.stack.last_mut() {
                list.items.push(Item {
                    checked,
                    blocks: frame.blocks,
                });
            }
        }
        TagEnd::HtmlBlock => {
            if let Some(html) = b.html.take() {
                let literal = html.trim_end().to_string();
                if !literal.is_empty() {
                    b.push_block(Block::Literal(literal))?;
                }
            }
        }
        TagEnd::Emphasis => b.italic = b.italic.saturating_sub(1),
        TagEnd::Strong => b.bold = b.bold.saturating_sub(1),
        TagEnd::Strikethrough => b.strike = b.strike.saturating_sub(1),
        TagEnd::Link => {
            if let Some(None) = b.links.pop()
                && let Some(closing) = b.refused.pop()
            {
                b.push_literal_run(closing);
            }
        }
        TagEnd::Image => {
            if let Some(closing) = b.images.pop() {
                b.push_literal_run(closing);
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Rich text (tickets, chat, comments)
// ---------------------------------------------------------------------------------------------

fn rich_blocks(blocks: &[Block], out: &mut Vec<Value>) {
    for block in blocks {
        match block {
            Block::Paragraph(runs) => out.push(rich_paragraph(runs.clone())),
            Block::Heading(level, runs) => out.push(json!({
                "type": "heading", "attrs": {"level": (*level).clamp(1, 6)},
                "content": rich_inline(runs)
            })),
            Block::Quote(inner) => {
                let mut content = Vec::new();
                rich_blocks(inner, &mut content);
                if content.is_empty() {
                    content.push(rich_paragraph(Vec::new()));
                }
                out.push(json!({"type": "blockquote", "content": content}));
            }
            Block::List { start, items } => {
                let items: Vec<Value> = items.iter().map(rich_item).collect();
                out.push(match start {
                    None => json!({"type": "bulletList", "content": items}),
                    Some(n) if *n > 1 => {
                        json!({"type": "orderedList", "attrs": {"start": n}, "content": items})
                    }
                    Some(_) => json!({"type": "orderedList", "content": items}),
                });
            }
            Block::Code { language, text } => {
                let content: Vec<Value> = if text.is_empty() {
                    Vec::new()
                } else {
                    vec![json!({"type": "text", "text": text})]
                };
                let mut node = json!({"type": "codeBlock", "content": content});
                if !language.is_empty() {
                    node["attrs"] = json!({"language": language});
                }
                out.push(node);
            }
            // No divider in rich text: the source stays as text.
            Block::Rule => out.push(rich_paragraph(vec![Run::plain("---")])),
            Block::Literal(text) => out.push(rich_paragraph(vec![Run::plain(text)])),
        }
    }
}

/// A list item. A task marker is not a node of rich text, so it stays as the text `[ ] ` or `[x] `.
fn rich_item(item: &Item) -> Value {
    let mut content = Vec::new();
    rich_blocks(&item.blocks, &mut content);
    if let Some(checked) = item.checked {
        let marker = if checked { "[x] " } else { "[ ] " };
        match content.first_mut() {
            Some(first) if first["type"] == "paragraph" => {
                let mut nodes = first["content"].as_array().cloned().unwrap_or_default();
                // Into the first plain text node when there is one, so the marker is not a node of its own.
                match nodes.first_mut() {
                    Some(node) if node["type"] == "text" && node.get("marks").is_none() => {
                        let joined = format!("{marker}{}", node["text"].as_str().unwrap_or(""));
                        node["text"] = json!(joined);
                    }
                    _ => nodes.insert(0, json!({"type": "text", "text": marker})),
                }
                first["content"] = Value::Array(nodes);
            }
            _ => content.insert(0, rich_paragraph(vec![Run::plain(marker)])),
        }
    }
    if content.is_empty() {
        content.push(rich_paragraph(Vec::new()));
    }
    json!({"type": "listItem", "content": content})
}

fn rich_paragraph(runs: Vec<Run>) -> Value {
    json!({"type": "paragraph", "content": rich_inline(&runs)})
}

fn rich_inline(runs: &[Run]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut last: Option<&Marks> = None;
    for run in runs.iter().filter(|run| !run.text.is_empty()) {
        if last == Some(&run.marks)
            && let Some(node) = out.last_mut()
        {
            let joined = format!("{}{}", node["text"].as_str().unwrap_or(""), run.text);
            node["text"] = json!(joined);
            continue;
        }
        let mut node = json!({"type": "text", "text": run.text});
        let mut marks = Vec::new();
        if run.marks.bold {
            marks.push(json!({"type": "bold"}));
        }
        if run.marks.italic {
            marks.push(json!({"type": "italic"}));
        }
        if run.marks.strike {
            marks.push(json!({"type": "strike"}));
        }
        if run.marks.code {
            marks.push(json!({"type": "code"}));
        }
        if let Some(href) = &run.marks.href {
            marks.push(json!({"type": "link", "attrs": {"href": href}}));
        }
        if !marks.is_empty() {
            node["marks"] = Value::Array(marks);
        }
        out.push(node);
        last = Some(&run.marks);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Doc blocks
// ---------------------------------------------------------------------------------------------

fn doc_block(kind: &str, props: Value, content: Vec<Value>) -> Value {
    doc_block_with(kind, props, content, Vec::new())
}

fn doc_block_with(kind: &str, props: Value, content: Vec<Value>, children: Vec<Value>) -> Value {
    json!({"id": Uuid::new_v4().to_string(), "type": kind, "props": props,
           "content": content, "children": children})
}

fn style_object(marks: &Marks) -> Value {
    let mut styles = Map::new();
    if marks.bold {
        styles.insert("bold".into(), json!(true));
    }
    if marks.italic {
        styles.insert("italic".into(), json!(true));
    }
    if marks.strike {
        styles.insert("strike".into(), json!(true));
    }
    if marks.code {
        styles.insert("code".into(), json!(true));
    }
    Value::Object(styles)
}

fn doc_text(text: &str, marks: &Marks) -> Vec<Value> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![json!({"type": "text", "text": text, "styles": style_object(marks)})]
}

fn doc_inline(runs: &[Run]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut index = 0;
    let runs: Vec<&Run> = runs.iter().filter(|run| !run.text.is_empty()).collect();
    while index < runs.len() {
        let run = runs[index];
        match &run.marks.href {
            None => {
                out.extend(doc_text(&run.text, &run.marks));
                index += 1;
            }
            Some(href) => {
                let mut texts = Vec::new();
                while index < runs.len() && runs[index].marks.href.as_ref() == Some(href) {
                    texts.extend(doc_text(&runs[index].text, &runs[index].marks));
                    index += 1;
                }
                out.push(json!({"type": "link", "href": href, "content": texts}));
            }
        }
    }
    out
}

fn doc_blocks(block: &Block, out: &mut Vec<Value>) {
    match block {
        Block::Paragraph(runs) => out.push(doc_block("paragraph", json!({}), doc_inline(runs))),
        // The Doc editor has levels 1 to 3: deeper headings are level 3.
        Block::Heading(level, runs) => out.push(doc_block(
            "heading",
            json!({"level": (*level).clamp(1, 3)}),
            doc_inline(runs),
        )),
        Block::Quote(inner) => {
            for block in inner {
                match block {
                    Block::Paragraph(runs) | Block::Heading(_, runs) => {
                        out.push(doc_block("quote", json!({}), doc_inline(runs)));
                    }
                    other => doc_blocks(other, out),
                }
            }
        }
        Block::List { start, items } => {
            for item in items {
                out.push(doc_item(start.is_some(), item));
            }
        }
        Block::Code { language, text } => {
            let language = if language.is_empty() {
                "text"
            } else {
                language
            };
            out.push(doc_block(
                "codeBlock",
                json!({"language": language}),
                doc_text(text, &Marks::default()),
            ));
        }
        Block::Rule => out.push(doc_block("divider", json!({}), Vec::new())),
        Block::Literal(text) => out.push(doc_block(
            "paragraph",
            json!({}),
            doc_text(text, &Marks::default()),
        )),
    }
}

fn doc_item(ordered: bool, item: &Item) -> Value {
    let mut rest = item.blocks.iter();
    let mut content = Vec::new();
    let mut children = Vec::new();
    // The first paragraph is the item's own text, everything after it nests below.
    if let Some(Block::Paragraph(runs)) = item.blocks.first() {
        content = doc_inline(runs);
        rest.next();
    }
    for block in rest {
        doc_blocks(block, &mut children);
    }
    match item.checked {
        Some(checked) => doc_block_with(
            "checkListItem",
            json!({"checked": checked}),
            content,
            children,
        ),
        None if ordered => doc_block_with("numberedListItem", json!({}), content, children),
        None => doc_block_with("bulletListItem", json!({}), content, children),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use penfii_core::docs::content::sanitize;

    fn rich(markdown: &str) -> Value {
        markdown_to_rich_text(markdown).unwrap()
    }

    fn doc(markdown: &str) -> Value {
        markdown_to_doc(markdown).unwrap()
    }

    fn doc_kinds(content: &Value) -> Vec<String> {
        content["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|block| block["type"].as_str().unwrap().to_string())
            .collect()
    }

    fn texts(value: &Value) -> String {
        match value {
            Value::Object(map) => {
                let mut out = String::new();
                if let Some(Value::String(text)) = map.get("text") {
                    out.push_str(text);
                }
                for key in ["content", "children", "blocks"] {
                    if let Some(inner) = map.get(key) {
                        out.push_str(&texts(inner));
                    }
                }
                out
            }
            Value::Array(items) => items.iter().map(texts).collect(),
            _ => String::new(),
        }
    }

    #[test]
    fn wrapper_is_strict() {
        let parse = |value: Value| {
            let object = value.as_object().unwrap().clone();
            parse_wrapper(&object, "body")
                .map(|_| ())
                .map_err(|f| f.message)
        };
        assert!(parse(json!({"format": "markdown", "value": "# a"})).is_ok());
        assert!(parse(json!({"format": "plain", "value": "a"})).is_ok());
        assert!(
            parse(json!({"format": "richtext", "value": {"type": "doc", "content": []}})).is_ok()
        );
        for bad in [
            json!({"value": "a"}),
            json!({"format": "markdown"}),
            json!({"format": "markdown", "value": null}),
            json!({"format": "Markdown", "value": "a"}),
            json!({"format": "html", "value": "a"}),
            json!({"format": 1, "value": "a"}),
            json!({"format": "markdown", "value": {"type": "doc"}}),
            json!({"format": "plain", "value": 3}),
            json!({"format": "richtext", "value": "text"}),
            json!({"format": "richtext", "value": {"format": "markdown", "value": "x"}}),
            json!({"format": "markdown", "value": "a", "type": "doc"}),
            json!({"format": "markdown", "value": "a", "extra": 1}),
            json!({"type": "doc", "format": "markdown", "value": "a"}),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn rich_text_supports_the_common_formatting() {
        let value = rich(
            "# Title\n\nSome **bold**, *italic*, ~~gone~~ and `code` with a [link](https://example.com/a?b=1).\n\n- one\n- two\n  1. nested\n\n3. three\n4. four\n\n> quote\n\n```rust\nfn main() {}\n```",
        );
        let kinds: Vec<&str> = value["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "heading",
                "paragraph",
                "bulletList",
                "orderedList",
                "blockquote",
                "codeBlock"
            ]
        );
        assert_eq!(value["content"][0]["attrs"]["level"], 1);
        assert_eq!(value["content"][3]["attrs"]["start"], 3);
        assert_eq!(value["content"][5]["attrs"]["language"], "rust");
        let marks: Vec<String> = value["content"][1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|n| n["marks"].as_array().cloned().unwrap_or_default())
            .map(|m| m["type"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(marks, ["bold", "italic", "strike", "code", "link"]);
        // The nested list sits inside the first list's second item.
        assert_eq!(
            value["content"][2]["content"][1]["content"][1]["type"],
            "orderedList"
        );
        assert!(penfii_core::conversation::rich_text_link_allowed(
            "https://example.com/a?b=1"
        ));
    }

    #[test]
    fn rich_text_levels_one_to_six_and_combined_marks() {
        let value = rich("###### six\n\n***both*** and **bold *and italic***");
        assert_eq!(value["content"][0]["attrs"]["level"], 6);
        let first = &value["content"][1]["content"][0];
        assert_eq!(first["text"], "both");
        assert_eq!(first["marks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn rich_text_keeps_what_it_cannot_express() {
        let value = rich(
            "- [ ] todo\n- [x] done\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n<script>alert(1)</script>\n\n![alt](https://x.test/i.png)",
        );
        let text = texts(&value);
        for part in [
            "[ ] todo",
            "[x] done",
            "---",
            "| a | b |",
            "| 1 | 2 |",
            "<script>alert(1)</script>",
            "![alt](https://x.test/i.png)",
        ] {
            assert!(text.contains(part), "{part} was lost: {text}");
        }
        assert!(!value.to_string().contains("\"image\""));
    }

    #[test]
    fn dangerous_links_stay_as_text() {
        for href in [
            "javascript:alert(1)",
            "data:text/html,x",
            "https://user:pw@example.com/",
            "mailto:a@b.c",
            "/relative",
        ] {
            let value = rich(&format!("[click]({href})"));
            assert!(!value.to_string().contains("\"link\""), "{href}");
            assert_eq!(texts(&value), format!("[click]({href})"));
        }
        let value = doc(
            "[x](javascript:alert(1)) and [m](mailto:a@b.c) and [r](/p) and [h](https://e.test)",
        );
        let text = texts(&value);
        assert!(text.contains("[x](javascript:alert(1))"));
        let links = value.to_string().matches("\"type\":\"link\"").count();
        assert_eq!(links, 3);
    }

    #[test]
    fn markdown_inside_code_stays_literal() {
        let value = rich("`**not bold**` and\n\n```\n# not a heading\n- not a list\n```");
        assert_eq!(value["content"][0]["content"][0]["text"], "**not bold**");
        assert_eq!(
            value["content"][1]["content"][0]["text"],
            "# not a heading\n- not a list"
        );
        let content = doc("```md\n**x**\n```");
        assert_eq!(content["blocks"][0]["content"][0]["text"], "**x**");
        assert_eq!(content["blocks"][0]["props"]["language"], "md");
    }

    #[test]
    fn escapes_and_korean_text() {
        let value = rich("\\*not italic\\* 한글 **굵게** 😀 1\\. x");
        assert_eq!(texts(&value), "*not italic* 한글 굵게 😀 1. x");
    }

    #[test]
    fn doc_blocks_cover_the_legacy_parser() {
        let content = doc(
            "# Title\n\nHello **bold** and `code`.\n\n- a\n- [x] done\n1. one\n> quote\n---\n| a | b |\n![img](x)\n```rs\nlet x = 1;\n```",
        );
        let text = texts(&content);
        for part in [
            "Title",
            "Hello bold and code.",
            "a",
            "done",
            "one",
            "quote",
            "| a | b |",
            "![img](x)",
            "let x = 1;",
        ] {
            assert!(text.contains(part), "{part} lost: {text}");
        }
        let kinds = doc_kinds(&content);
        assert!(kinds.contains(&"heading".to_string()));
        assert!(kinds.contains(&"checkListItem".to_string()));
        assert!(kinds.contains(&"divider".to_string()));
        assert!(kinds.contains(&"codeBlock".to_string()));
        assert!(sanitize(&content, 1 << 20).is_ok());
    }

    #[test]
    fn doc_headings_are_capped_at_three_and_lists_nest_as_children() {
        let content =
            doc("##### deep\n\n- parent\n  - child\n    - grandchild\n\n1. first\n2. second");
        assert_eq!(content["blocks"][0]["props"]["level"], 3);
        let parent = &content["blocks"][1];
        assert_eq!(parent["type"], "bulletListItem");
        assert_eq!(
            parent["children"][0]["children"][0]["content"][0]["text"],
            "grandchild"
        );
        assert_eq!(content["blocks"][2]["type"], "numberedListItem");
        let (clean, _) = sanitize(&content, 1 << 20).unwrap();
        assert_eq!(clean["blocks"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn doc_inline_marks_and_links() {
        let content = doc("a **b *c*** [d **e**](https://x.test/) `f` ~~g~~");
        let inline = content["blocks"][0]["content"].as_array().unwrap();
        let link = inline.iter().find(|n| n["type"] == "link").unwrap();
        assert_eq!(link["href"], "https://x.test/");
        assert_eq!(link["content"][1]["styles"]["bold"], true);
        assert!(sanitize(&content, 1 << 20).is_ok());
    }

    #[test]
    fn empty_and_whitespace_markdown() {
        let content = doc("");
        assert_eq!(content["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(doc("  \n\n ")["blocks"][0]["type"], "paragraph");
        // Only a link definition: nothing renders, so the text is kept.
        assert!(texts(&rich("[a]: https://x.test")).contains("[a]: https://x.test"));
        assert!(texts(&doc("[a]: https://x.test")).contains("[a]: https://x.test"));
    }

    #[test]
    fn plain_doc_keeps_line_breaks_and_symbols() {
        let content =
            plain_to_doc("# not a heading\nsecond line\n\n- not a list **not bold**").unwrap();
        assert_eq!(doc_kinds(&content), ["paragraph", "paragraph"]);
        assert_eq!(
            content["blocks"][0]["content"][0]["text"],
            "# not a heading\nsecond line"
        );
        assert_eq!(
            content["blocks"][1]["content"][0]["text"],
            "- not a list **not bold**"
        );
        assert_eq!(
            plain_to_doc("  \n").unwrap()["blocks"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn hostile_input_is_bounded() {
        // Deep quotes and lists are refused, not recursed into.
        let quotes = format!("{}x", "> ".repeat(500));
        assert!(markdown_to_rich_text(&quotes).is_err());
        assert!(markdown_to_doc(&quotes).is_err());
        let mut lists = String::new();
        for depth in 0..100 {
            lists.push_str(&format!("{}- x\n", "  ".repeat(depth)));
        }
        assert!(markdown_to_doc(&lists).is_err());
        // Many blocks are refused.
        let many = "a\n\n".repeat(MAX_BLOCKS + 10);
        assert!(markdown_to_doc(&many).is_err());
        // A long single line is fine and linear.
        let long = "a *b* ".repeat(30_000);
        assert!(markdown_to_doc(&long).is_ok());
        // Unbalanced emphasis and brackets are text.
        assert_eq!(texts(&rich("**a *b [c](d")), "**a *b [c](d");
    }

    #[test]
    fn the_same_markdown_is_the_same_doc_so_a_retry_is_the_same_request() {
        let source = "# a\n\n- b\n  - c\n\ntext";
        assert_eq!(doc(source), doc(source));
        assert_eq!(markdown_to_doc("").unwrap(), markdown_to_doc("").unwrap());
        assert_eq!(
            plain_to_doc("x\n\ny").unwrap(),
            plain_to_doc("x\n\ny").unwrap()
        );
        // Different text, different ids; and ids are unique inside a page.
        assert_ne!(doc("a")["blocks"][0]["id"], doc("b")["blocks"][0]["id"]);
        let ids: std::collections::BTreeSet<String> = doc(source)["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids.len(), 3);
        assert!(sanitize(&doc(source), 1 << 20).is_ok());
    }

    #[test]
    fn size_limits_follow_each_area() {
        assert!(rich_text_from_plain(&"a".repeat(RICH_TEXT_MAX_BYTES), "body").is_ok());
        assert!(rich_text_from_plain(&"a".repeat(RICH_TEXT_MAX_BYTES + 1), "body").is_err());
        // 20,000 bytes, not characters: 7,000 Korean characters are 21,000 bytes.
        assert!(rich_text_from_markdown(&"가".repeat(7_000), "body").is_err());
        assert!(rich_text_from_markdown("   \n ", "body").is_err());
        assert!(plain_to_doc(&"a".repeat(DOC_TEXT_MAX_CHARS)).is_ok());
        assert!(plain_to_doc(&"a".repeat(DOC_TEXT_MAX_CHARS + 1)).is_err());
        assert!(markdown_to_doc(&"가".repeat(DOC_TEXT_MAX_CHARS)).is_ok());
    }
}
