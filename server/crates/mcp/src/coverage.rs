//! The feature coverage matrix (FEA-158): every HTTP route of `api/openapi.json` is classified in
//! `coverage/*.tsv` as an MCP `tool`, a `web` action (never exposed: see `web_only`), `excluded`
//! (for example WebSockets, which the changes feed replaces) or `other` (a line in another area's
//! table decides). The tests keep the matrix and the catalog honest.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    const TABLES: &[(&str, &str)] = &[
        ("admin", include_str!("../coverage/admin.tsv")),
        ("conversation", include_str!("../coverage/conversation.tsv")),
        ("doc", include_str!("../coverage/doc.tsv")),
        ("file", include_str!("../coverage/file.tsv")),
        ("org", include_str!("../coverage/org.tsv")),
        ("personal", include_str!("../coverage/personal.tsv")),
        ("plan", include_str!("../coverage/plan.tsv")),
        ("project", include_str!("../coverage/project.tsv")),
        ("search", include_str!("../coverage/search.tsv")),
        ("tickets", include_str!("../coverage/tickets.tsv")),
    ];
    const OPENAPI: &str = include_str!("../../../api/openapi.json");

    type Rows = BTreeMap<(String, String), Vec<(&'static str, String, String)>>;

    fn rows() -> Rows {
        let mut rows: Rows = BTreeMap::new();
        for (area, text) in TABLES {
            for line in text
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            {
                let cells: Vec<&str> = line.split('\t').collect();
                assert!(cells.len() >= 4, "{area}.tsv: malformed line {line:?}");
                assert!(
                    ["tool", "web", "excluded", "other"].contains(&cells[2]),
                    "{area}.tsv: unknown status in {line:?}"
                );
                rows.entry((cells[0].to_string(), cells[1].to_string()))
                    .or_default()
                    .push((area, cells[2].to_string(), cells[3].to_string()));
            }
        }
        rows
    }

    #[test]
    fn every_http_route_is_classified_exactly_once() {
        let spec: serde_json::Value = serde_json::from_str(OPENAPI).unwrap();
        let mut routes = BTreeSet::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            for method in item.as_object().unwrap().keys() {
                if ["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                    routes.insert((method.to_uppercase(), path.clone()));
                }
            }
        }
        let rows = rows();
        for route in &routes {
            let found = rows
                .get(route)
                .unwrap_or_else(|| panic!("{} {} is not in coverage/*.tsv", route.0, route.1));
            let statuses: BTreeSet<&str> = found.iter().map(|(_, s, _)| s.as_str()).collect();
            let decided = statuses.iter().filter(|s| **s != "other").count();
            assert!(
                decided >= 1,
                "{} {}: only 'other' rows, nobody decides",
                route.0,
                route.1
            );
            assert!(
                !(statuses.contains("tool") && statuses.contains("web")),
                "{} {} is both a tool and web-only",
                route.0,
                route.1
            );
        }
        for key in rows.keys() {
            assert!(
                routes.contains(key),
                "{} {} is not an OpenAPI route",
                key.0,
                key.1
            );
        }
    }

    #[test]
    fn every_tool_named_in_the_matrix_exists_and_web_rows_have_no_tool() {
        let tools: BTreeSet<&str> = crate::catalog::all().iter().map(|op| op.tool).collect();
        for ((method, path), found) in rows() {
            for (area, status, cell) in found {
                if status == "tool" {
                    for token in cell.split(|c: char| !(c.is_ascii_lowercase() || c == '_')) {
                        if token.contains('_') {
                            assert!(
                                tools.contains(token),
                                "{area}.tsv: {method} {path} names {token}, which is not a tool"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Prints the tool table of `docs/mcp-tools.md`:
    /// `cargo test -p penfii-mcp print_tool_table -- --ignored --nocapture`.
    #[test]
    #[ignore = "documentation generator"]
    fn print_tool_table() {
        let mut ops: Vec<_> = crate::catalog::all().to_vec();
        let bytes: usize = ops
            .iter()
            .map(|op| (op.input_schema)().to_string().len() + op.description.len())
            .sum();
        eprintln!("TOOLS {} SCHEMA+DESCRIPTION BYTES {bytes}", ops.len());
        ops.sort_by_key(|op| (op.toolset.as_str(), op.tool));
        let mut last = "";
        for op in ops {
            if op.toolset.as_str() != last {
                last = op.toolset.as_str();
                println!(
                    "\n### {last}\n\n| Tool | Scope | Risk | What it does |\n|---|---|---|---|"
                );
            }
            let first = op
                .description
                .split(". ")
                .next()
                .unwrap_or("")
                .trim_end_matches('.');
            println!(
                "| `{}` | `{}` | {} | {} |",
                op.tool,
                op.scopes.join(" "),
                op.risk.as_str(),
                first.replace('|', "/")
            );
        }
    }

    /// The tool list in `docs/mcp-tools.md` is the catalog: a tool added or renamed without
    /// regenerating the table fails here.
    #[test]
    fn the_documented_tools_are_the_catalog() {
        let doc = include_str!("../../../../docs/mcp-tools.md");
        for op in crate::catalog::all() {
            assert!(
                doc.contains(&format!("| `{}` |", op.tool)),
                "docs/mcp-tools.md does not list {} (regenerate the table, see print_tool_table)",
                op.tool
            );
        }
    }
}
