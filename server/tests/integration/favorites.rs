//! Favorites (FEA-199): the order of mixed kinds, folders and hidden shortcuts, saving again, two
//! changes at once, and the DM and Doc kinds with who may save and see them.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Env, Person, Reply};

fn page(text: &str) -> Value {
    json!({"schema":"penfii.doc","version":1,"blocks":[
        {"id":Uuid::new_v4().to_string(),"type":"paragraph","props":{},"content":[{"type":"text","text":text,"styles":{}}],"children":[]}]})
}

async fn project(env: &Env, by: &Person, name: &str) -> String {
    let reply = env
        .call(
            by,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": name, "teamIds": []})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn channel(env: &Env, by: &Person, name: &str, visibility: &str) -> String {
    let reply = env
        .call(
            by,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": name, "visibility": visibility, "projectId": null})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn doc(env: &Env, by: &Person, title: &str, visibility: &str, extra: Value) -> String {
    let mut body = json!({"title": title, "visibility": visibility, "content": page(title)});
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    let reply = env
        .call(
            by,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(body),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn dm(env: &Env, by: &Person, with: &[&Person]) -> String {
    let mut ids = vec![by.id];
    ids.extend(with.iter().map(|p| p.id));
    let reply = env
        .call(
            by,
            "POST",
            &format!("/v1/workspaces/{}/dms", env.ws),
            Some(json!({"userIds": ids})),
        )
        .await;
    assert!(reply.status.is_success(), "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn add(env: &Env, who: &Person, kind: &str, target: &str) -> Reply {
    env.call(
        who,
        "PUT",
        &format!("/v1/workspaces/{}/favorites", env.ws),
        Some(json!({"kind": kind, "targetId": target})),
    )
    .await
}

async fn list(env: &Env, who: &Person) -> Vec<Value> {
    let reply = env
        .call(
            who,
            "GET",
            &format!("/v1/workspaces/{}/favorites", env.ws),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.body.as_array().unwrap().clone()
}

fn ids(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|item| item["targetId"].as_str().unwrap().to_owned())
        .collect()
}

async fn order(env: &Env, who: &Person, items: &[(&str, &str)], scope: Option<&str>) -> Reply {
    let items: Vec<Value> = items
        .iter()
        .map(|(kind, id)| json!({"kind": kind, "targetId": id}))
        .collect();
    let mut body = json!({"items": items});
    if let Some(scope) = scope {
        body["scope"] = json!(scope);
    }
    env.call(
        who,
        "PUT",
        &format!("/v1/workspaces/{}/favorites/order", env.ws),
        Some(body),
    )
    .await
}

async fn folder(env: &Env, who: &Person, name: &str) -> String {
    let reply = env
        .call(
            who,
            "POST",
            &format!("/v1/workspaces/{}/favorite-folders", env.ws),
            Some(json!({"name": name})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn move_to(env: &Env, who: &Person, kind: &str, target: &str, folder: Option<&str>) {
    let reply = env
        .call(
            who,
            "PUT",
            &format!("/v1/workspaces/{}/favorites/{kind}/{target}/folder", env.ws),
            Some(json!({"folderId": folder})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
}

async fn positions(pool: &PgPool, user: Uuid) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT target_id, position FROM penfii.favorites WHERE user_id=$1 ORDER BY position, created_at, target_id",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(id, position)| (id.to_string(), position))
    .collect()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn saving_again_keeps_the_folder_and_ordering_keeps_other_groups_and_hidden_ones(
    pool: PgPool,
) {
    let env = Env::new(pool).await;
    let me = &env.owner;
    let p = [
        project(&env, me, "Alpha").await,
        project(&env, me, "Beta").await,
        project(&env, me, "Gamma").await,
    ];
    let general = channel(&env, me, "fav-general", "PUBLIC").await;
    let secret = channel(&env, me, "fav-secret", "PRIVATE").await;
    for (kind, id) in [
        ("PROJECT", &p[0]),
        ("CHANNEL", &general),
        ("PROJECT", &p[1]),
        ("CHANNEL", &secret),
        ("PROJECT", &p[2]),
    ] {
        let reply = add(&env, me, kind, id).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert!(reply.body["folderId"].is_null());
    }
    let daily = folder(&env, me, "Daily").await;
    move_to(&env, me, "PROJECT", &p[1], Some(&daily)).await;
    move_to(&env, me, "PROJECT", &p[2], Some(&daily)).await;

    // Saving again answers with the place and the folder it really has.
    let before = list(&env, me).await;
    let again = add(&env, me, "PROJECT", &p[2]).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.body["folderId"], daily.as_str());
    let kept = before
        .iter()
        .find(|f| f["targetId"] == p[2].as_str())
        .unwrap();
    assert_eq!(again.body["position"], kept["position"]);
    assert_eq!(again.body["createdAt"], kept["createdAt"]);

    // The private channel becomes one the owner cannot open: hidden from the list, kept on the server.
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(Uuid::parse_str(&secret).unwrap())
        .bind(me.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let visible = list(&env, me).await;
    assert_eq!(
        ids(&visible),
        vec![p[0].clone(), general.clone(), p[1].clone(), p[2].clone()]
    );

    // Moving inside the folder (what Saved sends): only the folder's two change places; the hidden
    // channel between them and every top-level one keep theirs.
    let reply = order(
        &env,
        me,
        &[("PROJECT", &p[2]), ("PROJECT", &p[1])],
        Some("LISTED"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        ids(reply.body.as_array().unwrap()),
        vec![p[0].clone(), general.clone(), p[2].clone(), p[1].clone()]
    );
    let stored: Vec<String> = positions(&env.pool, me.id)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        stored,
        vec![
            p[0].clone(),
            general.clone(),
            p[2].clone(),
            secret.clone(),
            p[1].clone()
        ],
        "the hidden one keeps its slot"
    );
    let folders: Vec<Value> = list(&env, me)
        .await
        .into_iter()
        .filter(|f| f["folderId"] == daily.as_str())
        .collect();
    assert_eq!(folders.len(), 2, "folders are not touched by an order");

    // The whole-list order keeps its meaning: the named first, the rest after in their order, and
    // the positions are distinct (no tie with one not named).
    let reply = order(
        &env,
        me,
        &[
            ("CHANNEL", &general),
            ("PROJECT", &p[1]),
            ("CHANNEL", &general),
            ("PROJECT", &Uuid::new_v4().to_string()),
        ],
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        ids(reply.body.as_array().unwrap()),
        vec![general.clone(), p[1].clone(), p[0].clone(), p[2].clone()]
    );
    let stored = positions(&env.pool, me.id).await;
    let mut distinct: Vec<i64> = stored.iter().map(|(_, n)| *n).collect();
    distinct.dedup();
    assert_eq!(distinct.len(), stored.len(), "{stored:?}");
    assert_eq!(
        stored[4].0, secret,
        "the hidden one keeps its relative order"
    );

    // Bad requests.
    let reply = order(&env, me, &[], Some("SOME")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    let many: Vec<(String, String)> = (0..501)
        .map(|_| ("PROJECT".to_owned(), Uuid::new_v4().to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = many.iter().map(|(k, i)| (k.as_str(), i.as_str())).collect();
    let reply = order(&env, me, &refs, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);

    // Another person's order never touches mine, and cannot read my shortcuts.
    let other = env.person("fav-other", "MEMBER").await;
    let reply = order(
        &env,
        &other,
        &[("PROJECT", &p[2]), ("CHANNEL", &general)],
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.as_array().unwrap().is_empty());
    assert_eq!(positions(&env.pool, me.id).await, stored);

    // Deleting the folder puts its Favorites after every other one, never on a used position.
    let reply = env
        .call(me, "DELETE", &format!("/v1/favorite-folders/{daily}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let stored = positions(&env.pool, me.id).await;
    let mut numbers: Vec<i64> = stored.iter().map(|(_, n)| *n).collect();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(numbers.len(), stored.len(), "{stored:?}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn changes_at_once_never_share_a_position(pool: PgPool) {
    let env = Env::new(pool).await;
    let me = &env.owner;
    let mut projects = Vec::new();
    for n in 0..8 {
        projects.push(project(&env, me, &format!("Race {n}")).await);
    }
    let adds = projects
        .iter()
        .map(|id| add(&env, me, "PROJECT", id))
        .collect::<Vec<_>>();
    let last = [("PROJECT", projects[7].as_str())];
    let order_now = order(&env, me, &last, None);
    let (replies, _) = tokio::join!(futures_util::future::join_all(adds), order_now);
    assert!(replies.iter().all(|r| r.status == StatusCode::OK));
    let stored = positions(&env.pool, me.id).await;
    assert_eq!(stored.len(), 8);
    let mut numbers: Vec<i64> = stored.iter().map(|(_, n)| *n).collect();
    numbers.dedup();
    assert_eq!(numbers.len(), 8, "{stored:?}");
    // Two orders at once: one of them wins whole.
    let first: Vec<(&str, &str)> = projects.iter().map(|p| ("PROJECT", p.as_str())).collect();
    let second: Vec<(&str, &str)> = projects
        .iter()
        .rev()
        .map(|p| ("PROJECT", p.as_str()))
        .collect();
    let (a, b) = tokio::join!(
        order(&env, me, &first, None),
        order(&env, me, &second, None)
    );
    assert!(a.status.is_success() && b.status.is_success());
    let now = ids(&list(&env, me).await);
    let forward: Vec<String> = projects.clone();
    let backward: Vec<String> = projects.iter().rev().cloned().collect();
    assert!(now == forward || now == backward, "{now:?}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_dm_is_saved_by_its_people_only_and_named_like_the_side_list(pool: PgPool) {
    let env = Env::new(pool).await;
    let me = &env.owner;
    let ann = env.person("ann", "MEMBER").await;
    let bob = env.person("bob", "MEMBER").await;
    let outsider = env.outsider().await;
    let pair = dm(&env, me, &[&ann]).await;
    let group = dm(&env, me, &[&ann, &bob]).await;

    let saved = add(&env, me, "DM", &pair).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert_eq!(saved.body["title"], "ann");
    assert_eq!(saved.body["conversationId"], pair.as_str());
    assert!(saved.body["projectId"].is_null());
    let again = add(&env, me, "DM", &pair).await;
    assert_eq!(again.body["position"], saved.body["position"]);
    let saved = add(&env, me, "DM", &group).await;
    assert_eq!(saved.body["title"], "ann, bob");
    // Ann sees the pair under the owner's name.
    let theirs = add(&env, &ann, "DM", &pair).await;
    assert_eq!(theirs.status, StatusCode::OK);
    assert_eq!(theirs.body["title"], "Owner");

    // Not one of its people, another Workspace, the wrong kind: not found, and nothing is told.
    let reply = add(&env, &bob, "DM", &pair).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
    let reply = add(&env, &outsider, "DM", &pair).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
    let reply = add(&env, me, "CHANNEL", &pair).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "a DM is no channel");
    let general = channel(&env, me, "fav-dm-general", "PUBLIC").await;
    let reply = add(&env, me, "DM", &general).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "a channel is no DM");

    // It takes part in folders, order and removal like every other kind.
    let place = folder(&env, me, "People").await;
    move_to(&env, me, "DM", &group, Some(&place)).await;
    let reply = order(&env, me, &[("DM", &group), ("DM", &pair)], None).await;
    assert_eq!(
        ids(reply.body.as_array().unwrap()),
        vec![group.clone(), pair.clone()]
    );
    // A new name shows on the next read.
    sqlx::query("UPDATE penfii.users SET display_name='Ann Lee' WHERE id=$1")
        .bind(ann.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let now = list(&env, me).await;
    assert_eq!(now[1]["title"], "Ann Lee");
    assert_eq!(now[0]["folderId"], place.as_str());
    // Ann leaves the Workspace: her saved DM is hidden from her (she cannot read it any more).
    sqlx::query("UPDATE penfii.workspace_members SET state='INACTIVE' WHERE user_id=$1")
        .bind(ann.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.favorites WHERE user_id=$1 AND kind='DM'")
            .bind(ann.id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(rows, 1, "the row is kept");
    let reply = env
        .call(
            me,
            "DELETE",
            &format!("/v1/workspaces/{}/favorites/DM/{pair}", env.ws),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(ids(&list(&env, me).await), vec![group.clone()]);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_doc_is_saved_by_whoever_may_read_it_and_goes_when_that_ends(pool: PgPool) {
    let env = Env::new(pool).await;
    let me = &env.owner;
    let reader = env.person("reader", "MEMBER").await;
    let outsider = env.outsider().await;
    let open = doc(&env, me, "Handbook", "WORKSPACE", json!({})).await;
    let closed = doc(&env, me, "Salaries", "RESTRICTED", json!({})).await;
    let shared = doc(
        &env,
        me,
        "Shared notes",
        "RESTRICTED",
        json!({"participants": [{"userId": reader.id, "role": "VIEWER"}]}),
    )
    .await;
    let home = project(&env, me, "Docs home").await;
    let in_project = doc(
        &env,
        me,
        "Project page",
        "WORKSPACE",
        json!({"projectId": home}),
    )
    .await;

    let saved = add(&env, &reader, "DOC", &open).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert_eq!(saved.body["title"], "Handbook");
    let reply = add(&env, &reader, "DOC", &shared).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "a viewer may save: {}",
        reply.body
    );
    let reply = add(&env, &reader, "DOC", &closed).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
    let reply = add(&env, &outsider, "DOC", &open).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = add(&env, &reader, "DOC", &Uuid::new_v4().to_string()).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    // A page of a Project the reader is not in: readable (Workspace-wide), but the Project is not named.
    let theirs = add(&env, &reader, "DOC", &in_project).await;
    assert_eq!(theirs.status, StatusCode::OK, "{}", theirs.body);
    assert!(theirs.body["projectId"].is_null());
    assert!(theirs.body["subtitle"].is_null());
    let mine = add(&env, me, "DOC", &in_project).await;
    assert_eq!(mine.body["projectId"], home.as_str());
    assert_eq!(mine.body["subtitle"], "Docs home");

    // Saving changes nothing of the page: no revision, no participant.
    let revision: i64 =
        sqlx::query_scalar("SELECT revision_no FROM penfii.doc_pages WHERE id=$1::uuid")
            .bind(&shared)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(revision, 1);
    let participants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.doc_page_participants WHERE page_id=$1::uuid AND user_id=$2",
    )
    .bind(&shared)
    .bind(reader.id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(participants, 1, "still the one share it had");

    // A new title shows; losing the share or a deleted page hides it, with no title left behind.
    let reply = env
        .call(
            me,
            "PATCH",
            &format!("/v1/docs/{open}"),
            Some(json!({"title": "Handbook 2"})),
        )
        .await;
    assert!(reply.status.is_success(), "{}", reply.body);
    let now = list(&env, &reader).await;
    assert_eq!(now[0]["title"], "Handbook 2");
    sqlx::query("DELETE FROM penfii.doc_page_participants WHERE page_id=$1::uuid AND user_id=$2")
        .bind(&shared)
        .bind(reader.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let reply = env
        .call(me, "DELETE", &format!("/v1/docs/{open}"), None)
        .await;
    assert!(reply.status.is_success(), "{}", reply.body);
    let now = list(&env, &reader).await;
    assert_eq!(ids(&now), vec![in_project.clone()]);
    assert!(!Value::Array(now).to_string().contains("Shared notes"));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn team_and_workspace_views_are_their_owners_and_an_archived_team_stays(pool: PgPool) {
    let env = Env::new(pool).await;
    let me = &env.owner;
    let other = env.person("viewer", "MEMBER").await;
    let team = env
        .call(
            me,
            "POST",
            &format!("/v1/workspaces/{}/teams", env.ws),
            Some(json!({"name": "Ops", "members": [{"userId": me.id, "role": "MANAGER"}]})),
        )
        .await;
    assert_eq!(team.status, StatusCode::CREATED, "{}", team.body);
    let team = team.body["id"].as_str().unwrap().to_owned();
    let view = common::send(
        &env.app,
        common::request(
            "POST",
            &format!("/v1/workspaces/{}/views", env.ws),
            Some(&me.cookie),
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"name": "My ops", "targetKind": "TEAM", "teamId": team})),
        ),
    )
    .await;
    assert_eq!(view.status, StatusCode::CREATED, "{}", view.body);
    let view = view.body["id"].as_str().unwrap().to_owned();
    let saved = add(&env, me, "VIEW", &view).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert_eq!(saved.body["viewTarget"], "TEAM");
    assert_eq!(saved.body["teamId"], team.as_str());
    let reply = add(&env, &other, "VIEW", &view).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "another person's view");

    // Any member may read a Team; an archived one is still readable and stays saved.
    let reply = add(&env, &other, "TEAM", &team).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = env
        .call(
            me,
            "POST",
            &format!("/v1/workspaces/{}/teams/{team}/archive", env.ws),
            Some(json!({})),
        )
        .await;
    assert!(reply.status.is_success(), "{}", reply.body);
    assert_eq!(ids(&list(&env, &other).await), vec![team.clone()]);
    let reply = add(&env, &other, "TEAM", &team).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}
