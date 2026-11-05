//! `api/openapi.json` is the public description of the API. This test holds the Rust types in
//! `crates/contracts` (what the server actually serializes) to it, field by field: a struct and the
//! schema of the same name must have the same properties, agree on which may be null, and agree on the
//! kind of each value. It reads the source file as text, so it needs no database and no new crate.
//!
//! The web client holds its own copy of the types to the same file, see `client/packages/contracts`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

const OPENAPI: &str = include_str!("../api/openapi.json");
const CONTRACTS: &str = include_str!("../crates/contracts/src/lib.rs");

#[derive(Debug)]
struct Field {
    name: String,
    ty: String,
    skip_if_none: bool,
}

#[derive(Debug)]
struct RustStruct {
    fields: Vec<Field>,
    /// A `#[serde(flatten)]` field merges another type's properties in; those are not listed here.
    flattens: bool,
}

fn camel(name: &str) -> String {
    let mut parts = name.split('_');
    let mut out = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// `pub struct Name { pub field: Type, ... }` with the serde attributes this crate uses.
fn rust_structs() -> BTreeMap<String, RustStruct> {
    let mut structs = BTreeMap::new();
    let lines: Vec<&str> = CONTRACTS.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        let Some(rest) = line.strip_prefix("pub struct ") else {
            index += 1;
            continue;
        };
        let name = rest.trim_end_matches(" {").trim().to_owned();
        if !rest.ends_with('{') {
            index += 1;
            continue;
        }
        // The attributes right above the struct say how names are written.
        let mut back = index;
        let mut camel_case = false;
        while back > 0 && lines[back - 1].trim().starts_with("#[") {
            back -= 1;
            if lines[back].contains("rename_all = \"camelCase\"") {
                camel_case = true;
            }
        }
        let mut fields = Vec::new();
        let mut flattens = false;
        let mut pending: Vec<&str> = Vec::new();
        index += 1;
        while index < lines.len() && lines[index].trim() != "}" {
            let field = lines[index].trim();
            index += 1;
            if field.starts_with("#[") {
                pending.push(field);
                continue;
            }
            if field.starts_with("//") || field.is_empty() {
                continue;
            }
            let Some(field) = field.strip_prefix("pub ") else {
                continue;
            };
            let Some((field_name, ty)) = field.split_once(':') else {
                continue;
            };
            let attrs = pending.join(" ");
            pending.clear();
            if attrs.contains("flatten") {
                flattens = true;
                continue;
            }
            let renamed = attrs
                .split("rename = \"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_owned);
            let json_name = renamed.unwrap_or_else(|| {
                if camel_case {
                    camel(field_name.trim())
                } else {
                    field_name.trim().to_owned()
                }
            });
            fields.push(Field {
                name: json_name,
                ty: ty.trim().trim_end_matches(',').trim().to_owned(),
                skip_if_none: attrs.contains("skip_serializing_if"),
            });
        }
        structs.insert(name, RustStruct { fields, flattens });
    }
    structs
}

/// What a Rust type serializes to: JSON kinds, whether `null` is possible, and for a named type the
/// name a `$ref` has to carry.
#[derive(Debug, Default)]
struct Shape {
    kinds: BTreeSet<&'static str>,
    nullable: bool,
    named: Option<String>,
    item_named: Option<String>,
    any: bool,
}

fn shape(ty: &str) -> Shape {
    let ty = ty.trim();
    let ty = ty.strip_prefix("chrono::").unwrap_or(ty);
    if let Some(inner) = ty.strip_prefix("Option<").and_then(|t| t.strip_suffix('>')) {
        let mut inner = shape(inner);
        inner.nullable = true;
        return inner;
    }
    let mut out = Shape::default();
    if let Some(inner) = ty.strip_prefix("Vec<").and_then(|t| t.strip_suffix('>')) {
        out.kinds.insert("array");
        let item = shape(inner);
        out.item_named = item.named;
        return out;
    }
    match ty {
        "String" | "Uuid" | "DateTime<Utc>" | "NaiveDate" => {
            out.kinds.insert("string");
        }
        "i16" | "i32" | "i64" | "u16" | "u32" | "u64" | "usize" => {
            out.kinds.insert("integer");
        }
        "f32" | "f64" => {
            out.kinds.insert("number");
        }
        "bool" => {
            out.kinds.insert("boolean");
        }
        "Value" | "serde_json::Value" => out.any = true,
        _ if ty.starts_with("HashMap<") || ty.starts_with("BTreeMap<") => {
            out.kinds.insert("object");
        }
        _ => {
            out.kinds.insert("object");
            out.named = Some(ty.to_owned());
        }
    }
    out
}

fn ref_name(schema: &Value) -> Option<String> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.rsplit('/').next())
        .map(str::to_owned)
}

/// The kinds an OpenAPI property allows; `None` when it does not say (any value).
fn documented_kinds(schema: &Value) -> Option<(BTreeSet<String>, bool)> {
    if ref_name(schema).is_some() {
        return Some((["object".to_owned()].into(), false));
    }
    let mut kinds = BTreeSet::new();
    let mut nullable = false;
    let mut add = |kind: &str| {
        if kind == "null" {
            nullable = true;
        } else {
            kinds.insert(kind.to_owned());
        }
    };
    match schema.get("type") {
        Some(Value::String(kind)) => add(kind),
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).for_each(&mut add),
        _ => {
            if schema.get("enum").is_some() {
                add("string");
            } else {
                return None;
            }
        }
    }
    Some((kinds, nullable))
}

#[test]
fn the_rust_contract_types_match_their_openapi_schemas() {
    let openapi: Value = serde_json::from_str(OPENAPI).unwrap();
    let schemas = openapi["components"]["schemas"].as_object().unwrap();
    let structs = rust_structs();
    let mut compared = 0;
    let mut problems: Vec<String> = Vec::new();

    for (name, schema) in schemas {
        let Some(rust) = structs.get(name) else {
            continue;
        };
        compared += 1;
        let properties = schema["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let required: BTreeSet<&str> = schema["required"]
            .as_array()
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let rust_names: BTreeSet<&str> = rust.fields.iter().map(|f| f.name.as_str()).collect();
        let documented: BTreeSet<&str> = properties.keys().map(String::as_str).collect();

        if !rust.flattens {
            for missing in rust_names.difference(&documented) {
                problems.push(format!(
                    "{name}.{missing}: the server sends it, openapi.json does not describe it"
                ));
            }
            for extra in documented.difference(&rust_names) {
                problems.push(format!(
                    "{name}.{extra}: openapi.json describes it, the server does not send it"
                ));
            }
        }
        for field in &rust.fields {
            let Some(property) = properties.get(&field.name) else {
                continue;
            };
            let sent = shape(&field.ty);
            let at = format!("{name}.{}", field.name);
            if let Some((kinds, nullable)) = documented_kinds(property) {
                if nullable && !sent.nullable {
                    problems.push(format!(
                        "{at}: openapi.json allows null, the Rust type cannot be null"
                    ));
                }
                if !nullable
                    && sent.nullable
                    && !field.skip_if_none
                    && required.contains(field.name.as_str())
                {
                    problems.push(format!("{at}: the Rust type can be null, openapi.json requires it and does not allow null"));
                }
                if !sent.any {
                    for kind in &sent.kinds {
                        let fits = kinds.contains(*kind)
                            || (*kind == "integer" && kinds.contains("number"));
                        if !fits {
                            problems.push(format!(
                                "{at}: the server sends {kind}, openapi.json says {kinds:?}"
                            ));
                        }
                    }
                }
            }
            if let Some(expected) = &sent.named
                && let Some(reference) = ref_name(property)
                && &reference != expected
            {
                problems.push(format!(
                    "{at}: the server sends {expected}, openapi.json refers to {reference}"
                ));
            }
            if let Some(expected) = &sent.item_named
                && let Some(reference) = property.get("items").and_then(ref_name)
                && &reference != expected
            {
                problems.push(format!(
                    "{at}[]: the server sends {expected}, openapi.json refers to {reference}"
                ));
            }
            if field.skip_if_none && required.contains(field.name.as_str()) {
                problems.push(format!(
                    "{at}: the server leaves it out when empty, openapi.json requires it"
                ));
            }
        }
        for name_required in &required {
            if !rust.flattens && !rust_names.contains(name_required) {
                problems.push(format!(
                    "{name}.{name_required}: required in openapi.json, never sent"
                ));
            }
        }
    }

    assert!(
        compared >= 55,
        "only {compared} schemas were matched to a Rust struct; has the parser stopped seeing them?"
    );
    assert!(
        problems.is_empty(),
        "openapi.json and crates/contracts disagree:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_comparison_notices_a_difference() {
    // A guard for the test above: it must fail on a drifted copy, not pass because it sees nothing.
    let drifted = OPENAPI.replacen("\"lastChangeSeq\"", "\"lastChangeSeqX\"", 1);
    assert_ne!(drifted, OPENAPI);
    let openapi: Value = serde_json::from_str(&drifted).unwrap();
    let schemas = openapi["components"]["schemas"].as_object().unwrap();
    let structs = rust_structs();
    let found = schemas.iter().any(|(name, schema)| {
        structs.get(name).is_some_and(|rust| {
            let documented: BTreeSet<&str> = schema["properties"]
                .as_object()
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            rust.fields
                .iter()
                .any(|f| !documented.contains(f.name.as_str()))
        })
    });
    assert!(found, "a renamed property went unnoticed");
}
