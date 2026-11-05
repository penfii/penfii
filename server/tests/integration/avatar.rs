//! Profile photos are bytes in object storage. PostgreSQL keeps the object key and the type.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use penfii_core::files::ObjectStore;
use penfii_core::identity::avatar_object_key;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, OWNER_EMAIL, OWNER_PASSWORD, Reply, limits};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0x00, 0x11];

struct Env {
    pool: PgPool,
    app: axum::Router,
    objects: ObjectStore,
    user_id: Uuid,
    cookie: String,
}

async fn setup(pool: PgPool) -> Env {
    let boot = common::setup(&pool).await;
    let objects = ObjectStore::in_memory();
    let app =
        penfii_http_api::router_with(pool.clone(), limits(), false, Some(objects.clone()), None);
    let cookie = common::sign_in(&app, OWNER_EMAIL, OWNER_PASSWORD).await;
    Env {
        pool,
        app,
        objects,
        user_id: boot.user_id,
        cookie,
    }
}

async fn put_photo(app: &axum::Router, cookie: &str, bytes: &'static [u8], mime: &str) -> Reply {
    let request = Request::builder()
        .method("PUT")
        .uri("/v1/me/avatar")
        .header("x-penfii-client", "1")
        .header("cookie", cookie)
        .header("content-type", mime)
        .body(Body::from(bytes.to_vec()))
        .unwrap();
    common::send(app, request).await
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_photo_is_stored_as_an_object_and_replaced_by_a_new_key(pool: PgPool) {
    let env = setup(pool).await;
    let put = put_photo(&env.app, &env.cookie, JPEG, "image/jpeg").await;
    assert_eq!(put.status, StatusCode::NO_CONTENT, "{}", put.body);

    let key: Option<String> =
        sqlx::query_scalar("SELECT avatar_object_key FROM penfii.users WHERE id = $1")
            .bind(env.user_id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        key.as_deref(),
        Some(avatar_object_key(env.user_id, 1).as_str())
    );
    assert_eq!(env.objects.get(key.as_ref().unwrap()).await.unwrap(), JPEG);

    let got = common::call(
        &env.app,
        "GET",
        &format!("/v1/users/{}/avatar", env.user_id),
        Some(&env.cookie),
        None,
    )
    .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.body);
    assert_eq!(got.bytes, JPEG);
    assert_eq!(got.header("content-type"), Some("image/jpeg"));

    let again = put_photo(
        &env.app,
        &env.cookie,
        &[0x89, 0x50, 0x4E, 0x47, 0x0D],
        "image/png",
    )
    .await;
    assert_eq!(again.status, StatusCode::NO_CONTENT, "{}", again.body);
    let next: String =
        sqlx::query_scalar("SELECT avatar_object_key FROM penfii.users WHERE id = $1")
            .bind(env.user_id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(next, avatar_object_key(env.user_id, 2));
    assert!(
        env.objects
            .get(&avatar_object_key(env.user_id, 1))
            .await
            .is_err()
    );

    let cleared = common::call(&env.app, "DELETE", "/v1/me/avatar", Some(&env.cookie), None).await;
    assert_eq!(cleared.status, StatusCode::NO_CONTENT, "{}", cleared.body);
    let missing = common::call(
        &env.app,
        "GET",
        &format!("/v1/users/{}/avatar", env.user_id),
        Some(&env.cookie),
        None,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(env.objects.get(&next).await.is_err());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_file_that_is_not_an_image_is_refused_and_nothing_is_stored(pool: PgPool) {
    let env = setup(pool).await;
    let put = put_photo(&env.app, &env.cookie, b"not an image", "image/png").await;
    assert_eq!(put.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", put.body);
    let key: Option<String> =
        sqlx::query_scalar("SELECT avatar_object_key FROM penfii.users WHERE id = $1")
            .bind(env.user_id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert!(key.is_none());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_photo_cannot_be_saved_when_object_storage_is_not_configured(pool: PgPool) {
    let boot = common::setup(&pool).await;
    let app = penfii_http_api::router_with(pool, limits(), false, None, None);
    let cookie = common::sign_in(&app, OWNER_EMAIL, OWNER_PASSWORD).await;
    let put = put_photo(&app, &cookie, JPEG, "image/jpeg").await;
    assert_eq!(put.status, StatusCode::CONFLICT, "{}", put.body);
    assert_eq!(put.body["code"], "storage_unconfigured");
    let _ = boot;
}
