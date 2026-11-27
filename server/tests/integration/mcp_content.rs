//! `{format, value}` text input of the MCP write tools (FEA-188..192): every tool that writes a
//! description, message, comment or Doc body takes plain, markdown and richtext, keeps the earlier
//! string and JSON forms, and refuses a malformed object before anything is written.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mcp_oauth::{Env, modern_call, modern_list, rest, setup, token_for};
use crate::mcp_support::{mcp_err, mcp_ok};

const SCOPES: &str = "workspace:read member:read ticket:read ticket:write project:read project:write conversation:read conversation:write doc:read doc:write";

async fn new_project(env: &Env, name: &str) -> String {
    let project = rest(
        env,
        "POST",
        &format!("/v1/workspaces/{}/projects", env.workspace_id),
        &env.owner_cookie,
        None,
        Some(json!({"name": name, "teamIds": []})),
    )
    .await;
    project.body["id"].as_str().unwrap().to_string()
}

fn kinds(rich: &Value) -> Vec<String> {
    rich["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["type"].as_str().unwrap().to_string())
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

fn md(value: &str) -> Value {
    json!({"format": "markdown", "value": value})
}

/// No wrapper is ever stored: nothing in a read-back has a `format` key.
fn no_wrapper(value: &Value) {
    assert!(!value.to_string().contains("\"format\""), "{value}");
}

async fn ticket_count(env: &Env, project_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM penfii.tickets WHERE project_id = $1")
        .bind(Uuid::parse_str(project_id).unwrap())
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

const BAD_OBJECTS: &[&str] = &[
    r#"{"format": "markdown"}"#,
    r#"{"value": "x"}"#,
    r#"{"format": "markdown", "value": null}"#,
    r#"{"format": "html", "value": "x"}"#,
    r#"{"format": "Markdown", "value": "x"}"#,
    r#"{"format": "markdown", "value": 3}"#,
    r#"{"format": "markdown", "value": "   "}"#,
    r#"{"format": "plain", "value": ""}"#,
    r#"{"format": "markdown", "value": "x", "type": "doc"}"#,
    r#"{"format": "markdown", "value": "x", "markdown": "y"}"#,
    r#"{"format": "richtext", "value": "x"}"#,
    r#"{"format": "richtext", "value": {"format": "markdown", "value": "x"}}"#,
    r#"{"type": "doc", "content": [{"type": "paragraph", "content": []}], "format": "plain", "value": "x"}"#,
];

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn ticket_descriptions_and_comments_take_the_three_formats(pool: PgPool) {
    let env = setup(pool).await;
    let project_id = new_project(&env, "Formats").await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;

    // Markdown becomes the rich text the editor stores; the wrapper is not stored.
    let ticket = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "md",
               "description": md("## 목표\n- **굵게** 항목\n- `code`\n\n1. one\n\n> quote\n\n```rust\nlet x = 1;\n```\n\n[docs](https://example.com/a)")}),
    )
    .await;
    let id = ticket["id"].as_str().unwrap().to_string();
    let got = mcp_ok(&env, &owner, "ticket_get", json!({"ticketId": id})).await;
    let description = &got["description"];
    // A description is written as the Doc the web editor stores (and converts older shapes to), not as chat rich text.
    let blocks: Vec<&str> = description["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        blocks,
        [
            "heading",
            "bulletListItem",
            "bulletListItem",
            "numberedListItem",
            "quote",
            "codeBlock",
            "paragraph"
        ]
    );
    assert_eq!(description["schema"], "penfii.doc");
    assert_eq!(description["blocks"][0]["props"]["level"], 2);
    no_wrapper(description);
    // The web route returns the same stored JSON.
    let web = rest(
        &env,
        "GET",
        &format!("/v1/tickets/{id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.body["description"], *description);

    // Task lists and dividers keep their meaning (check items, a divider), instead of the text `[ ]` and `---`.
    let tasks = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "tasks", "description": md("- [ ] todo\n- [x] done\n\n---\n\n##### deep")}),
    )
    .await;
    let kinds_of: Vec<&str> = tasks["description"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds_of,
        ["checkListItem", "checkListItem", "divider", "heading"]
    );
    assert_eq!(tasks["description"]["blocks"][1]["props"]["checked"], true);
    assert_eq!(tasks["description"]["blocks"][3]["props"]["level"], 3);

    // plain keeps the text literal; a bare string behaves as before.
    let plain = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "plain",
               "description": {"format": "plain", "value": "# not a heading\n\n**not bold**"}}),
    )
    .await;
    assert_eq!(kinds(&plain["description"]), ["paragraph", "paragraph"]);
    assert_eq!(
        plain["description"]["content"][0]["content"][0]["text"],
        "# not a heading"
    );
    let bare = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "bare", "description": "# not a heading\n\n**not bold**"}),
    )
    .await;
    assert_eq!(bare["description"], plain["description"]);

    // richtext and the old JSON form are the same document.
    let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "json", "marks": [{"type": "bold"}]}]}]});
    let wrapped = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "rt", "description": {"format": "richtext", "value": doc.clone()}}),
    )
    .await;
    let legacy = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "legacy", "description": doc.clone()}),
    )
    .await;
    assert_eq!(wrapped["description"], doc);
    assert_eq!(legacy["description"], doc);

    // A description left out is no description; update replaces the whole text and keeps it when
    // it is left out.
    let none = mcp_ok(
        &env,
        &owner,
        "ticket_create",
        json!({"projectId": project_id, "title": "none"}),
    )
    .await;
    assert!(none["description"].is_null() || kinds(&none["description"]).is_empty());
    let revision = got["revisionNo"].as_i64().unwrap();
    let updated = mcp_ok(
        &env,
        &owner,
        "ticket_update",
        json!({"ticketId": id, "expectedRevision": revision, "description": md("replaced **text**")}),
    )
    .await;
    assert_eq!(texts(&updated["description"]), "replaced text");
    let kept = mcp_ok(
        &env,
        &owner,
        "ticket_update",
        json!({"ticketId": id, "expectedRevision": updated["revisionNo"], "title": "renamed"}),
    )
    .await;
    assert_eq!(texts(&kept["description"]), "replaced text");

    // Bad objects write nothing: no ticket is created, the revision does not move.
    let before = ticket_count(&env, &project_id).await;
    let current = mcp_ok(&env, &owner, "ticket_get", json!({"ticketId": id})).await;
    for bad in BAD_OBJECTS {
        let object: Value = serde_json::from_str(bad).unwrap();
        let code = mcp_err(
            &env,
            &owner,
            "ticket_create",
            json!({"projectId": project_id, "title": "bad", "description": object.clone()}),
        )
        .await;
        assert_eq!(code, "invalid_arguments", "{bad}");
        let code = mcp_err(
            &env,
            &owner,
            "ticket_update",
            json!({"ticketId": id, "expectedRevision": current["revisionNo"], "description": object}),
        )
        .await;
        assert_eq!(code, "invalid_arguments", "{bad}");
    }
    assert_eq!(ticket_count(&env, &project_id).await, before);
    let after = mcp_ok(&env, &owner, "ticket_get", json!({"ticketId": id})).await;
    assert_eq!(after["revisionNo"], current["revisionNo"]);
    assert_eq!(after["description"], current["description"]);

    // A top-level markdown is not an argument: refused, never a silent no-op.
    for tool_args in [
        json!({"ticketId": id, "expectedRevision": current["revisionNo"], "markdown": "# lost"}),
        json!({"ticketId": id, "expectedRevision": current["revisionNo"], "title": "t", "markdown": "# lost"}),
    ] {
        assert_eq!(
            mcp_err(&env, &owner, "ticket_update", tool_args).await,
            "invalid_arguments"
        );
    }
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_create",
            json!({"projectId": project_id, "title": "x", "markdown": "# x"})
        )
        .await,
        "invalid_arguments"
    );
    assert_eq!(ticket_count(&env, &project_id).await, before);

    // Comments: markdown, plain, JSON; edit replaces; a missing or bad body writes nothing.
    let comment = mcp_ok(
        &env,
        &owner,
        "ticket_comment",
        json!({"ticketId": id, "body": md("- a\n- b")}),
    )
    .await;
    assert_eq!(kinds(&comment["body"]), ["bulletList"]);
    no_wrapper(&comment["body"]);
    let comment_id = comment["id"].as_str().unwrap().to_string();
    let edited = mcp_ok(
        &env,
        &owner,
        "ticket_comment_edit",
        json!({"ticketId": id, "commentId": comment_id, "body": {"format": "plain", "value": "- literal"}}),
    )
    .await;
    assert_eq!(texts(&edited["body"]), "- literal");
    assert_eq!(kinds(&edited["body"]), ["paragraph"]);
    for bad in BAD_OBJECTS {
        let object: Value = serde_json::from_str(bad).unwrap();
        assert_eq!(
            mcp_err(
                &env,
                &owner,
                "ticket_comment",
                json!({"ticketId": id, "body": object.clone()})
            )
            .await,
            "invalid_arguments",
            "{bad}"
        );
        assert_eq!(
            mcp_err(
                &env,
                &owner,
                "ticket_comment_edit",
                json!({"ticketId": id, "commentId": comment_id, "body": object})
            )
            .await,
            "invalid_arguments",
            "{bad}"
        );
    }
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "ticket_comment",
            json!({"ticketId": id, "body": md("x"), "markdown": "y"})
        )
        .await,
        "invalid_arguments"
    );
    assert_eq!(
        mcp_err(&env, &owner, "ticket_comment", json!({"ticketId": id})).await,
        "invalid_arguments"
    );
    let comments = mcp_ok(
        &env,
        &owner,
        "ticket_comments_list",
        json!({"ticketId": id}),
    )
    .await;
    assert_eq!(comments["comments"].as_array().unwrap().len(), 1);

    // A retry with the same key and the same request returns the first ticket; the same key with
    // another request is a conflict, not a second ticket.
    let args = json!({"projectId": project_id, "title": "once", "description": md("# once"), "idempotencyKey": "fmt-1"});
    let first = mcp_ok(&env, &owner, "ticket_create", args.clone()).await;
    let second = mcp_ok(&env, &owner, "ticket_create", args).await;
    assert_eq!(first["id"], second["id"]);
    let before = ticket_count(&env, &project_id).await;
    let other = json!({"projectId": project_id, "title": "once", "description": md("# other"), "idempotencyKey": "fmt-1"});
    let reply = modern_call(&env.app, &owner, "ticket_create", other).await;
    assert_eq!(reply.body["result"]["isError"], true, "{:?}", reply.body);
    assert_eq!(ticket_count(&env, &project_id).await, before);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn messages_threads_and_edits_take_the_three_formats(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let channel = mcp_ok(
        &env,
        &owner,
        "channel_create",
        json!({"workspaceId": ws, "name": "fmt-talk", "visibility": "PUBLIC"}),
    )
    .await;
    let channel_id = channel["id"].as_str().unwrap().to_string();

    let message = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": channel_id, "body": md("**done**\n\n- a\n- b")}),
    )
    .await;
    assert_eq!(kinds(&message["body"]), ["paragraph", "bulletList"]);
    no_wrapper(&message["body"]);
    let root_id = message["id"].as_str().unwrap().to_string();

    // A thread reply and a quoted reply go through the same input.
    let reply = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": channel_id, "threadRootId": root_id, "body": md("> quoted\n\nthanks")}),
    )
    .await;
    assert_eq!(kinds(&reply["body"]), ["blockquote", "paragraph"]);
    assert_eq!(reply["threadRootId"], root_id);
    let quoted = mcp_ok(
        &env,
        &owner,
        "message_send",
        json!({"conversationId": channel_id, "threadRootId": root_id, "replyToId": reply["id"], "body": {"format": "plain", "value": "# literal"}}),
    )
    .await;
    assert_eq!(texts(&quoted["body"]), "# literal");

    // Edit replaces the whole body with the version check.
    let version = message["version"].as_i64().unwrap();
    let edited = mcp_ok(
        &env,
        &owner,
        "message_edit",
        json!({"messageId": root_id, "expectedVersion": version, "body": md("## replaced")}),
    )
    .await;
    assert_eq!(kinds(&edited["body"]), ["heading"]);
    let stale = mcp_err(
        &env,
        &owner,
        "message_edit",
        json!({"messageId": root_id, "expectedVersion": version, "body": md("stale")}),
    )
    .await;
    assert_eq!(stale, "version_conflict");

    // Malformed objects write nothing and notify nobody.
    let count = |env: &Env| {
        let pool = env.pool.clone();
        let id = Uuid::parse_str(&channel_id).unwrap();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM penfii.messages WHERE conversation_id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let messages = count(&env).await;
    let current = edited["version"].as_i64().unwrap();
    for bad in BAD_OBJECTS {
        let object: Value = serde_json::from_str(bad).unwrap();
        assert_eq!(
            mcp_err(
                &env,
                &owner,
                "message_send",
                json!({"conversationId": channel_id, "body": object.clone()})
            )
            .await,
            "invalid_arguments",
            "{bad}"
        );
        assert_eq!(
            mcp_err(
                &env,
                &owner,
                "message_edit",
                json!({"messageId": root_id, "expectedVersion": current, "body": object})
            )
            .await,
            "invalid_arguments",
            "{bad}"
        );
    }
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "message_send",
            json!({"conversationId": channel_id, "markdown": "# x"})
        )
        .await,
        "invalid_arguments"
    );
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "message_send",
            json!({"conversationId": channel_id, "body": md("x"), "markdown": "# x"})
        )
        .await,
        "invalid_arguments"
    );
    // Message bodies are checked against the chat rich-text schema: a node it has no place for
    // (a table) is refused whichever way it arrives.
    let table = json!({"type": "doc", "content": [{"type": "table"}]});
    for body in [table.clone(), json!({"format": "richtext", "value": table})] {
        assert!(
            !mcp_err(
                &env,
                &owner,
                "message_send",
                json!({"conversationId": channel_id, "body": body})
            )
            .await
            .is_empty()
        );
    }
    assert_eq!(count(&env).await, messages);
    let unchanged = mcp_ok(
        &env,
        &owner,
        "message_history",
        json!({"messageId": root_id}),
    )
    .await;
    assert!(unchanged.to_string().contains("done") && !unchanged.to_string().contains("stale"));
    // The same key with the same request is a retry, with another request a conflict.
    let args =
        json!({"conversationId": channel_id, "body": md("# keyed"), "idempotencyKey": "k-1"});
    let first = mcp_ok(&env, &owner, "message_send", args.clone()).await;
    assert_eq!(
        first["id"],
        mcp_ok(&env, &owner, "message_send", args).await["id"]
    );
    let other =
        json!({"conversationId": channel_id, "body": md("# different"), "idempotencyKey": "k-1"});
    assert_eq!(
        modern_call(&env.app, &owner, "message_send", other)
            .await
            .body["result"]["isError"],
        true
    );
    assert_eq!(count(&env).await, messages + 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn docs_take_the_three_formats_and_keep_the_legacy_inputs(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id.to_string();
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let blocks = |page: &Value| -> Vec<String> {
        page["content"]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|block| block["type"].as_str().unwrap().to_string())
            .collect()
    };

    let created = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Guide",
               "content": md("# 운영 가이드\n\n- 하나\n- [x] 완료\n  - 중첩\n\n```sh\nls\n```\n\n---\n\n표:\n\n| a | b |\n|---|---|\n| 1 | 2 |")}),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let got = mcp_ok(&env, &owner, "doc_get", json!({"docId": id})).await;
    assert_eq!(
        blocks(&got),
        [
            "heading",
            "bulletListItem",
            "checkListItem",
            "codeBlock",
            "divider",
            "paragraph",
            "paragraph"
        ]
    );
    assert_eq!(
        got["content"]["blocks"][2]["children"][0]["content"][0]["text"],
        "중첩"
    );
    assert!(texts(&got["content"]).contains("| a | b |"));
    no_wrapper(&got["content"]);
    let web = rest(
        &env,
        "GET",
        &format!("/v1/docs/{id}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(web.body["content"], got["content"]);

    // plain keeps symbols and line breaks; richtext and the bare document are the same.
    let plain = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Plain", "content": {"format": "plain", "value": "# x\nsecond\n\n- y"}}),
    )
    .await;
    assert_eq!(blocks(&plain), ["paragraph", "paragraph"]);
    assert_eq!(
        plain["content"]["blocks"][0]["content"][0]["text"],
        "# x\nsecond"
    );
    let document = json!({"schema": "penfii.doc", "version": 1, "blocks": [
        {"id": "b1", "type": "paragraph", "props": {}, "children": [],
         "content": [{"type": "text", "text": "exact", "styles": {"bold": true}}]}]});
    let rich = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Rich", "content": {"format": "richtext", "value": document.clone()}}),
    )
    .await;
    let bare = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Bare", "content": document.clone()}),
    )
    .await;
    assert_eq!(
        rich["content"]["blocks"][0]["content"],
        bare["content"]["blocks"][0]["content"]
    );
    assert_eq!(
        rich["content"]["blocks"][0]["content"][0]["styles"]["bold"],
        true
    );

    // Legacy top-level markdown still creates a page; an empty one is the canonical empty page.
    let legacy = mcp_ok(
        &env,
        &owner,
        "doc_create",
        json!({"workspaceId": ws, "title": "Legacy", "markdown": "# Goal\n\n- one\n- two"}),
    )
    .await;
    assert_eq!(
        blocks(&legacy),
        ["heading", "bulletListItem", "bulletListItem"]
    );
    for empty in [
        json!({"workspaceId": ws, "title": "E1", "markdown": ""}),
        json!({"workspaceId": ws, "title": "E2", "content": {"format": "markdown", "value": ""}}),
        json!({"workspaceId": ws, "title": "E3", "content": {"format": "plain", "value": ""}}),
        json!({"workspaceId": ws, "title": "E4"}),
    ] {
        let page = mcp_ok(&env, &owner, "doc_create", empty).await;
        assert_eq!(blocks(&page), ["paragraph"]);
        assert!(
            page["content"]["blocks"][0]["content"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    // More than one of content, markdown and templateId is an error, and so are bad objects or
    // types; no page is created.
    let pages = |env: &Env| {
        let pool = env.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM penfii.doc_pages")
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let before = pages(&env).await;
    let template = Uuid::new_v4().to_string();
    for args in [
        json!({"workspaceId": ws, "title": "x", "content": md("a"), "markdown": "b"}),
        json!({"workspaceId": ws, "title": "x", "content": md("a"), "markdown": ""}),
        json!({"workspaceId": ws, "title": "x", "markdown": "", "templateId": template}),
        json!({"workspaceId": ws, "title": "x", "content": md("a"), "templateId": template}),
        json!({"workspaceId": ws, "title": "x", "content": document.clone(), "markdown": "b"}),
        json!({"workspaceId": ws, "title": "x", "markdown": 3}),
        json!({"workspaceId": ws, "title": "x", "content": "bare string"}),
        json!({"workspaceId": ws, "title": "x", "content": {"format": "richtext", "value": {"type": "doc", "content": []}}}),
    ] {
        let code = mcp_err(&env, &owner, "doc_create", args.clone()).await;
        assert!(
            code == "invalid_arguments" || code == "doc_content_invalid",
            "{args} -> {code}"
        );
    }
    for bad in BAD_OBJECTS {
        // An empty plain or markdown value is an explicit empty page for a Doc, so those two are not errors here.
        if bad.contains(r#""value": "   ""#) || bad.contains(r#""format": "plain", "value": """#) {
            continue;
        }
        let object: Value = serde_json::from_str(bad).unwrap();
        let code = mcp_err(
            &env,
            &owner,
            "doc_create",
            json!({"workspaceId": ws, "title": "x", "content": object}),
        )
        .await;
        assert_eq!(code, "invalid_arguments", "{bad}");
    }
    assert_eq!(pages(&env).await, before);

    // doc_save: the three formats, an empty body as an explicit replacement, the revision check.
    let mut revision = got["revisionNo"].as_i64().unwrap();
    let saved = mcp_ok(
        &env,
        &owner,
        "doc_save",
        json!({"docId": id, "expectedRevision": revision, "content": md("## Second\n\ntext")}),
    )
    .await;
    assert_eq!(blocks(&saved), ["heading", "paragraph"]);
    assert_eq!(saved["title"], "Guide");
    let stale = mcp_err(
        &env,
        &owner,
        "doc_save",
        json!({"docId": id, "expectedRevision": revision, "content": md("lost")}),
    )
    .await;
    assert_eq!(stale, "version_conflict");
    revision = saved["revisionNo"].as_i64().unwrap();
    let saved = mcp_ok(
        &env,
        &owner,
        "doc_save",
        json!({"docId": id, "expectedRevision": revision, "content": {"format": "richtext", "value": document.clone()}}),
    )
    .await;
    assert_eq!(texts(&saved["content"]), "exact");
    revision = saved["revisionNo"].as_i64().unwrap();
    let saved = mcp_ok(
        &env,
        &owner,
        "doc_save",
        json!({"docId": id, "expectedRevision": revision, "content": {"format": "plain", "value": "a < b & c"}}),
    )
    .await;
    assert_eq!(texts(&saved["content"]), "a < b & c");
    revision = saved["revisionNo"].as_i64().unwrap();
    let emptied = mcp_ok(
        &env,
        &owner,
        "doc_save",
        json!({"docId": id, "expectedRevision": revision, "content": md("")}),
    )
    .await;
    assert_eq!(blocks(&emptied), ["paragraph"]);
    assert_eq!(texts(&emptied["content"]), "");
    revision = emptied["revisionNo"].as_i64().unwrap();

    // What is refused leaves the page and its revision alone.
    for args in [
        json!({"docId": id, "expectedRevision": revision}),
        json!({"docId": id, "expectedRevision": revision, "content": null}),
        json!({"docId": id, "expectedRevision": revision, "markdown": "# x"}),
        json!({"docId": id, "expectedRevision": revision, "content": md("x"), "markdown": "# x"}),
        json!({"docId": id, "expectedRevision": revision, "content": "bare string"}),
        json!({"docId": id, "expectedRevision": revision, "content": {"format": "markdown", "value": null}}),
        json!({"docId": id, "expectedRevision": revision, "content": {"format": "richtext", "value": {"type": "doc", "content": [{"type": "paragraph", "content": []}]}}}),
        json!({"docId": id, "expectedRevision": revision, "content": {"format": "richtext", "value": {"format": "markdown", "value": "x"}}}),
    ] {
        let code = mcp_err(&env, &owner, "doc_save", args.clone()).await;
        assert!(
            code == "invalid_arguments" || code == "doc_content_invalid",
            "{args} -> {code}"
        );
    }
    let after = mcp_ok(&env, &owner, "doc_get", json!({"docId": id})).await;
    assert_eq!(after["revisionNo"], revision);

    // Doc comments take the same input.
    let comment = mcp_ok(
        &env,
        &owner,
        "doc_comment_create",
        json!({"docId": id, "body": md("**note**\n\n- a")}),
    )
    .await;
    assert_eq!(kinds(&comment["body"]), ["paragraph", "bulletList"]);
    let comment_id = comment["id"].as_str().unwrap().to_string();
    let edited = mcp_ok(
        &env,
        &owner,
        "doc_comment_edit",
        json!({"docId": id, "commentId": comment_id, "body": {"format": "plain", "value": "- literal"}}),
    )
    .await;
    assert_eq!(texts(&edited["body"]), "- literal");
    for bad in BAD_OBJECTS {
        let object: Value = serde_json::from_str(bad).unwrap();
        assert_eq!(
            mcp_err(
                &env,
                &owner,
                "doc_comment_create",
                json!({"docId": id, "body": object.clone()})
            )
            .await,
            "invalid_arguments",
            "{bad}"
        );
        assert_eq!(
            mcp_err(
                &env,
                &owner,
                "doc_comment_edit",
                json!({"docId": id, "commentId": comment_id, "body": object})
            )
            .await,
            "invalid_arguments",
            "{bad}"
        );
    }
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "doc_comment_create",
            json!({"docId": id, "body": md("x"), "markdown": "y"})
        )
        .await,
        "invalid_arguments"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_listed_schemas_describe_the_same_input_the_server_accepts(pool: PgPool) {
    let env = setup(pool).await;
    let owner = token_for(&env, &env.owner_cookie, SCOPES).await;
    let listed = modern_list(&env.app, &owner).await;
    let tools = listed.body["result"]["tools"].as_array().unwrap();
    let find = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    for (tool, field) in [
        ("ticket_create", "description"),
        ("ticket_update", "description"),
        ("ticket_comment", "body"),
        ("ticket_comment_edit", "body"),
        ("message_send", "body"),
        ("message_edit", "body"),
        ("doc_comment_create", "body"),
        ("doc_comment_edit", "body"),
        ("doc_create", "content"),
        ("doc_save", "content"),
    ] {
        let schema = &find(tool)["inputSchema"]["properties"][field];
        let branches = schema["oneOf"]
            .as_array()
            .unwrap_or_else(|| panic!("{tool}.{field}: {schema}"));
        // The wrapper branch has exactly format and value and nothing else.
        let wrapper = branches
            .iter()
            .find(|branch| branch["properties"]["format"].is_object())
            .unwrap_or_else(|| panic!("{tool}.{field} has no wrapper branch"));
        assert_eq!(
            wrapper["properties"]["format"]["enum"],
            json!(["plain", "markdown", "richtext"])
        );
        assert_eq!(wrapper["required"], json!(["format", "value"]));
        assert_eq!(wrapper["additionalProperties"], false);
        // The other object branch cannot also match a wrapper: it requires a root key a wrapper
        // does not have, so exactly one branch is valid for any input.
        let root = branches
            .iter()
            .filter(|branch| {
                branch["type"] == "object" && !branch["properties"]["format"].is_object()
            })
            .collect::<Vec<_>>();
        assert_eq!(root.len(), 1, "{tool}.{field}");
        assert!(!root[0]["required"].as_array().unwrap().is_empty());
        assert!(
            schema["description"].as_str().unwrap().contains("markdown"),
            "{tool}.{field}"
        );
        // No top-level markdown property was added, except doc_create's legacy one.
        let top = &find(tool)["inputSchema"]["properties"]["markdown"];
        assert_eq!(top.is_object(), tool == "doc_create", "{tool}");
        let description = find(tool)["description"].as_str().unwrap();
        assert!(description.contains("format"), "{tool}: {description}");
    }
    // The server instructions carry the rule after the unchanged opening.
    let init = crate::mcp_oauth::mcp(
        &env.app,
        Some(&owner),
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
    )
    .await;
    let instructions = init.body["result"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("Penfii is a work-management workspace"));
    let rule = instructions.find("Writing text:").unwrap();
    assert!(
        rule > 512,
        "the rule must not displace the first 512 characters"
    );
    assert!(instructions.contains("\"format\":\"markdown\""));
}
