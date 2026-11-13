//! What a JSON request body does, before and after the handlers read it straight into their types: the
//! answers for a body that is not JSON, has the wrong type, lacks a field, has an unknown field, leaves a field
//! out or sets it to `null` are the same as they were; and an object that names the same key twice, anywhere in
//! the body, is refused.

use super::{World, call, make_ticket, new_project};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

/// A request with the body exactly as written (the JSON the web client sends is never malformed or repeated).
async fn raw(
    world: &World,
    method: &str,
    path: &str,
    key: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-penfii-client", "1")
        .header("cookie", &world.owner_cookie);
    if let Some(key) = key {
        builder = builder.header("idempotency-key", key);
    }
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let reply = crate::common::send(
        &world.app,
        builder.body(Body::from(body.to_owned())).unwrap(),
    )
    .await;
    (reply.status, reply.body)
}

fn validation(reply: &(StatusCode, Value), what: &str) {
    assert_eq!(
        (reply.0, reply.1["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("validation")),
        "{what}: {}",
        reply.1
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_body_that_is_not_what_the_handler_wants_is_a_400_validation(pool: PgPool) {
    let world = &World::new(pool).await;
    let (project, _) = new_project(world, world.workspace_id, "Bodies").await;
    let path = format!("/v1/projects/{project}/tickets");
    let post = |n: u32, content_type: Option<&'static str>, body: &'static str| {
        let (path, key) = (path.clone(), format!("bodies-{n}"));
        async move { raw(world, "POST", &path, Some(&key), content_type, body).await }
    };
    let json_type = Some("application/json");

    validation(&post(1, json_type, r#"{"title": "#).await, "cut off");
    validation(&post(2, json_type, "").await, "empty");
    validation(&post(3, json_type, "not json").await, "not json");
    validation(&post(4, None, r#"{"title":"x"}"#).await, "no content type");
    validation(
        &post(5, Some("text/plain"), r#"{"title":"x"}"#).await,
        "wrong content type",
    );
    validation(
        &post(6, json_type, r#"{"title": 5}"#).await,
        "a number for a text",
    );
    validation(&post(7, json_type, "{}").await, "a missing field");
    validation(
        &post(8, json_type, r#"[1,2]"#).await,
        "an array for an object",
    );
    validation(
        &post(9, json_type, r#"{"title":"x","labelIds":["nope"]}"#).await,
        "a bad id",
    );

    // A JSON content type with parameters or a +json type is taken, as before.
    let (status, ticket) = post(
        10,
        Some("application/json; charset=utf-8"),
        r#"{"title":"charset"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let (status, ticket) = post(
        11,
        Some("application/vnd.penfii+json"),
        r#"{"title":"suffix"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn unknown_fields_defaults_and_null_against_omitted_keep_their_meaning(pool: PgPool) {
    let world = &World::new(pool).await;
    let (project, _) = new_project(world, world.workspace_id, "Meaning").await;

    // A body type without `deny_unknown_fields` ignores what it does not know ...
    let (status, label, _) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{project}/labels"),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name": "ignored extras", "colour": "teal", "anything": [1, 2]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{label}");
    assert!(label["color"].is_null(), "the unknown `colour` was ignored");

    // ... and one with it refuses (the bulk change names exactly two fields).
    let (status, refused, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{}/tickets/bulk", world.workspace_id),
        Some(&world.owner_cookie),
        Some("bodies-bulk"),
        Some(json!({"ticketIds": [], "change": {"type": "ARCHIVE"}, "surprise": true})),
    )
    .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("validation")),
        "{refused}"
    );

    // Defaults: a ticket needs only its title.
    let (status, bare) =
        make_ticket(world, &project, "meaning-bare", json!({"title": "Bare"})).await;
    assert_eq!(status, StatusCode::CREATED, "{bare}");
    assert_eq!(bare["hierarchyKind"], "TASK");
    assert_eq!(bare["labels"], json!([]));

    // `null` clears a field, leaving it out leaves it as it is (`dueOn` is such a field).
    let id = bare["id"].as_str().unwrap();
    let patch = |key: &str, body: Value| {
        let (path, key) = (format!("/v1/tickets/{id}"), key.to_owned());
        async move {
            let (status, value, _) = call(
                &world.app,
                "PATCH",
                &path,
                Some(&world.owner_cookie),
                Some(&key),
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{value}");
            value
        }
    };
    let set = patch(
        "meaning-1",
        json!({"expectedRevision": 1, "dueOn": "2026-12-24"}),
    )
    .await;
    assert_eq!(set["dueOn"], "2026-12-24");
    let kept = patch(
        "meaning-2",
        json!({"expectedRevision": set["revisionNo"], "title": "Renamed"}),
    )
    .await;
    assert_eq!(
        (kept["dueOn"].as_str(), kept["title"].as_str()),
        (Some("2026-12-24"), Some("Renamed"))
    );
    let cleared = patch(
        "meaning-3",
        json!({"expectedRevision": kept["revisionNo"], "dueOn": null}),
    )
    .await;
    assert!(cleared["dueOn"].is_null(), "{cleared}");
    assert_eq!(cleared["title"], "Renamed");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_key_named_twice_in_one_object_is_refused_wherever_it_is(pool: PgPool) {
    let world = &World::new(pool).await;
    let (project, _) = new_project(world, world.workspace_id, "Twice").await;
    let path = format!("/v1/projects/{project}/tickets");
    let json_type = Some("application/json");
    let post = |n: u32, body: &'static str| {
        let (path, key) = (path.clone(), format!("twice-{n}"));
        async move { raw(world, "POST", &path, Some(&key), json_type, body).await }
    };

    // In the object the handler reads, also when one spelling is an escape of the other.
    let top = post(1, r#"{"title": "first", "title": "second"}"#).await;
    validation(&top, "the same key twice at the top");
    assert!(
        top.1["message"]
            .as_str()
            .unwrap()
            .contains("duplicate key `title`"),
        "{}",
        top.1
    );
    validation(
        &post(2, r#"{"title": "first", "title": "second"}"#).await,
        "an escaped spelling",
    );
    // In a nested object, and in a free-form value the handler hands on as it is.
    validation(
        &post(
            3,
            r#"{"title": "x", "estimate": {"value": "1", "value": "2"}}"#,
        )
        .await,
        "a nested object of the type",
    );
    validation(
        &post(
            4,
            r#"{"title": "x", "description": {"type": "doc", "content": [], "type": "paragraph"}}"#,
        )
        .await,
        "inside a free-form value",
    );
    validation(
        &post(5, r#"{"title": "x", "description": {"type": "doc", "content": [{"a": 1}, {"b": 1, "b": 2}]}}"#).await,
        "deep inside an array",
    );
    // Nothing was made by any of them; the same body without the repetition is taken.
    let (status, ticket) = post(
        6,
        r#"{"title": "once", "description": {"type": "doc", "content": []}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let listed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.tickets WHERE project_id = $1")
            .bind(project.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(listed, 1);
    // The same key under two different objects is not a repeat.
    let (status, ticket) = post(
        7,
        r#"{"title": "fine", "description": {"type": "doc", "content": [{"type": "paragraph"}]}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_conversation_commands_that_keep_the_raw_body_refuse_a_repeated_key_too(pool: PgPool) {
    let world = &World::new(pool).await;
    let (status, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{}/channels", world.workspace_id),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name": "twice", "visibility": "PUBLIC", "teamId": null, "projectId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let path = format!(
        "/v1/conversations/{}/messages",
        channel["id"].as_str().unwrap()
    );
    let json_type = Some("application/json");
    let body = |text: &str| {
        format!(
            r#"{{"bodySchemaVersion": 1, "body": {}, "threadRootId": null, "replyToId": null}}"#,
            crate::common::doc(text)
        )
    };
    let repeated = format!(
        r#"{{"bodySchemaVersion": 1, "bodySchemaVersion": 1, "body": {}}}"#,
        crate::common::doc("hi")
    );
    validation(
        &raw(world, "POST", &path, Some("twice-m1"), json_type, &repeated).await,
        "a repeated key",
    );
    // The same message with no repetition goes through, and sending it again under the same key replays it.
    let once = body("hello");
    let (status, first) = raw(world, "POST", &path, Some("twice-m2"), json_type, &once).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let (status, again) = raw(world, "POST", &path, Some("twice-m2"), json_type, &once).await;
    assert_eq!(
        (status, &again["id"]),
        (StatusCode::CREATED, &first["id"]),
        "{again}"
    );
}
