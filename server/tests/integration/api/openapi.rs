//! The API contract against the routes the router serves.

use serde_json::Value;

#[test]
fn every_route_is_described_in_openapi_and_the_other_way_round() {
    let openapi: Value = serde_json::from_str(include_str!("../../../api/openapi.json")).unwrap();
    let paths = openapi["paths"].as_object().unwrap();
    // Every area of the router lists its routes next to its handlers, so read all of its sources.
    let mut sources = Vec::new();
    collect_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/http-api/src"),
        &mut sources,
    );
    let mut routes: std::collections::BTreeSet<String> = Default::default();
    for source in &sources {
        for chunk in source.split(".route(").skip(1) {
            let path = chunk.split('"').nth(1).unwrap().to_owned();
            // A route ends at the next `.route(`; its methods are the verbs in it.
            // The last route of a table ends where its function does.
            let call = chunk.split("\n}").next().unwrap();
            for verb in ["get", "post", "put", "patch", "delete"] {
                let marker = format!("{verb}(");
                let mut rest = call;
                while let Some(at) = rest.find(&marker) {
                    // A verb is a call of its own, not the tail of another name (`mcp_get(`).
                    let own = rest[..at]
                        .chars()
                        .next_back()
                        .is_none_or(|before| !(before.is_alphanumeric() || before == '_'));
                    if own {
                        routes.insert(format!("{} {}", verb.to_uppercase(), path));
                    }
                    rest = &rest[at + marker.len()..];
                }
            }
        }
    }
    assert!(routes.len() > 150, "the route tables were not found");
    let mut documented: std::collections::BTreeSet<String> = Default::default();
    for (path, item) in paths {
        for verb in ["get", "post", "put", "patch", "delete"] {
            if item.get(verb).is_some() {
                documented.insert(format!("{} {}", verb.to_uppercase(), path));
            }
        }
    }
    // Older routes that were never described. The list may shrink; it must not grow.
    // Every route is described. If an older route must be left out, name it here; the list may shrink and must not grow.
    let undocumented_before_p62: std::collections::BTreeSet<&str> =
        std::collections::BTreeSet::new();
    let missing: Vec<&String> = routes
        .iter()
        .filter(|r| !documented.contains(*r) && !undocumented_before_p62.contains(r.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "routes without an OpenAPI entry: {missing:#?}"
    );
    let stale: Vec<&String> = documented.iter().filter(|d| !routes.contains(*d)).collect();
    assert!(
        stale.is_empty(),
        "OpenAPI entries without a route: {stale:#?}"
    );
}

fn collect_sources(dir: &std::path::Path, into: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_sources(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(std::fs::read_to_string(&path).unwrap());
        }
    }
}
