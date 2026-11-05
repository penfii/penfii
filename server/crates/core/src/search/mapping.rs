//! Index settings and mappings. Changing anything here means bumping `MAPPING_VERSION`.

use serde_json::{Value, json};

pub const MAPPING_VERSION: i32 = 1;

pub const KINDS: &[&str] = &["message", "ticket", "space", "file", "view", "doc"];

fn text() -> Value {
    json!({
        "type": "text",
        "analyzer": "ko_text",
        "fields": {
            "ngram": {"type": "text", "analyzer": "ko_ngram", "search_analyzer": "ko_ngram"},
            "raw": {"type": "text", "analyzer": "raw_token"}
        }
    })
}

fn kw() -> Value {
    json!({"type": "keyword"})
}

fn date() -> Value {
    json!({"type": "date"})
}

pub fn settings() -> Value {
    json!({
        "number_of_shards": 1,
        "number_of_replicas": 0,
        "max_ngram_diff": 1,
        "analysis": {
            "tokenizer": {
                "ko_nori": {"type": "nori_tokenizer", "decompound_mode": "mixed"},
                "ngram_2_3": {"type": "ngram", "min_gram": 2, "max_gram": 3, "token_chars": ["letter", "digit"]}
            },
            "filter": {"ko_pos": {"type": "nori_part_of_speech"}},
            "normalizer": {"lc": {"type": "custom", "filter": ["lowercase"]}},
            "analyzer": {
                "ko_text": {"type": "custom", "tokenizer": "ko_nori", "filter": ["lowercase", "ko_pos"]},
                "ko_ngram": {"type": "custom", "tokenizer": "ngram_2_3", "filter": ["lowercase"]},
                "raw_token": {"type": "custom", "tokenizer": "whitespace", "filter": ["lowercase"]}
            }
        }
    })
}

fn envelope(properties: Value) -> Value {
    let mut all = json!({
        "workspaceId": kw(),
        "kind": kw(),
        "id": kw(),
        "createdAt": date(),
        "updatedAt": date(),
        "sourceVersion": {"type": "long"},
    });
    if let (Value::Object(base), Value::Object(extra)) = (&mut all, properties) {
        base.extend(extra);
    }
    json!({"dynamic": "strict", "properties": all})
}

pub fn mappings(kind: &str) -> Value {
    match kind {
        "message" => envelope(json!({
            "text": text(),
            "conversationId": kw(), "channelId": kw(), "threadRootId": kw(), "authorId": kw(),
            "mentionedUserIds": kw(), "hasFile": {"type": "boolean"},
        })),
        "ticket" => envelope(json!({
            "key": {"type": "keyword", "normalizer": "lc", "fields": {"raw": {"type": "text", "analyzer": "raw_token"}}},
            "title": text(), "description": text(), "comments": text(),
            "projectId": kw(), "channelId": kw(), "typeId": kw(), "statusId": kw(), "statusCategory": kw(),
            "priority": kw(), "assigneeId": kw(), "creatorId": kw(), "labelIds": kw(), "parentId": kw(),
            "dueOn": date(),
        })),
        "space" => envelope(json!({
            "spaceKind": kw(), "name": text(), "description": text(),
            "key": {"type": "keyword", "normalizer": "lc", "fields": {"raw": {"type": "text", "analyzer": "raw_token"}}},
            "archived": {"type": "boolean"}, "projectId": kw(), "teamId": kw(),
        })),
        "file" => envelope(json!({
            "filename": text(), "mime": kw(), "uploaderId": kw(),
            "conversationIds": kw(), "messageIds": kw(), "byteSize": {"type": "long"},
        })),
        "view" => envelope(json!({
            "name": text(), "scopeKind": kw(), "projectId": kw(), "teamId": kw(), "ownerId": kw(),
            "shared": {"type": "boolean"},
        })),
        // No visibility, participants, roles or relation targets: access is decided only in
        // PostgreSQL when a candidate is shown.
        "doc" => envelope(json!({
            "title": text(), "body": text(), "comments": text(),
            "projectId": kw(), "parentId": kw(), "creatorId": kw(),
        })),
        _ => json!({"dynamic": "strict", "properties": {}}),
    }
}

pub fn index_body(kind: &str) -> Value {
    json!({
        "settings": settings(),
        "mappings": mappings(kind),
    })
}
