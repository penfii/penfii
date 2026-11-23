//! Removing a person from a Workspace revokes what was explicitly given to them there, so that coming
//! back is a new membership (FEA-149): Team and Group memberships, direct Project grants and invited
//! Doc participants go; what they wrote, the Docs they created, their direct messages, other
//! Workspaces and the audit trail stay. Migration 0003 does the same for people removed by earlier
//! releases.

use axum::http::StatusCode;
use penfii_core::{admin, membership};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Env, Person};

fn page(text: &str) -> Value {
    json!({"schema":"penfii.doc","version":1,"blocks":[
        {"id":Uuid::new_v4().to_string(),"type":"paragraph","props":{},"content":[{"type":"text","text":text,"styles":{}}],"children":[]}]})
}

async fn project(env: &Env, name: &str) -> Uuid {
    let reply = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": name, "teamIds": []})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().parse().unwrap()
}

async fn doc(env: &Env, by: &Person, title: &str, visibility: &str, participants: Value) -> Uuid {
    let reply = env
        .call(
            by,
            "POST",
            &format!("/v1/workspaces/{}/docs", env.ws),
            Some(json!({"title": title, "visibility": visibility, "content": page(title), "participants": participants})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().parse().unwrap()
}

async fn count(pool: &PgPool, sql: &str, a: Uuid, b: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(a)
        .bind(b)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn sign_in_again(env: &Env, who: &Person) -> Person {
    let (status, cookie) = common::login(&env.app, &who.email, common::MEMBER_PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    Person {
        cookie: cookie.unwrap(),
        ..who.clone()
    }
}

async fn gets(env: &Env, who: &Person, path: &str) -> StatusCode {
    env.call(who, "GET", path, None).await.status
}

struct World {
    env: Env,
    pat: Person,
    buddy: Person,
    team: Uuid,
    group: Uuid,
    via_team: Uuid,
    direct: Uuid,
    via_group: Uuid,
    shared_doc: Uuid,
    own_doc: Uuid,
    dm: Uuid,
}

/// `pat` holds every kind of explicit access in the Workspace, and a Team in a second Workspace.
async fn world(pool: PgPool) -> World {
    let env = Env::new(pool.clone()).await;
    let pat = env.person("pat", "MEMBER").await;
    let buddy = env.person("buddy", "MEMBER").await;
    let team = {
        let reply = env
            .call(
                &env.owner,
                "POST",
                &format!("/v1/workspaces/{}/teams", env.ws),
                Some(json!({"name": "Dev", "members": [{"userId": env.owner.id, "role": "MANAGER"}]})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        reply.body["id"].as_str().unwrap().parse::<Uuid>().unwrap()
    };
    let added = env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/workspaces/{}/teams/{team}/members/{}", env.ws, pat.id),
            Some(json!({"role": "MEMBER"})),
        )
        .await;
    assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.body);
    let via_team = project(&env, "ByTeam").await;
    let direct = project(&env, "Direct").await;
    let via_group = project(&env, "ByGroup").await;
    for (path, body) in [
        (format!("/v1/projects/{via_team}/teams/{team}"), None),
        (
            format!("/v1/projects/{via_team}/teams/{team}/participation"),
            Some(json!({"role": "EDITOR"})),
        ),
        (
            format!("/v1/projects/{direct}/grants/{}", pat.id),
            Some(json!({"role": "EDITOR"})),
        ),
    ] {
        let reply = env.call(&env.owner, "PUT", &path, body).await;
        assert!(reply.status.is_success(), "{path} {}", reply.body);
    }
    let group = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.groups (id, workspace_id, name) VALUES ($1, $2, 'Leads')")
        .bind(group)
        .bind(env.ws)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.group_members (group_id, user_id) VALUES ($1, $2)")
        .bind(group)
        .bind(pat.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.project_group_grants (project_id, group_id, role) VALUES ($1, $2, 'VIEWER')")
        .bind(via_group)
        .bind(group)
        .execute(&pool)
        .await
        .unwrap();
    // A restricted Doc of buddy's shared with pat, and one pat made and shared with buddy.
    let shared_doc = doc(
        &env,
        &buddy,
        "Shared",
        "RESTRICTED",
        json!([{"userId": pat.id, "role": "EDITOR"}]),
    )
    .await;
    let own_doc = doc(
        &env,
        &pat,
        "Mine",
        "RESTRICTED",
        json!([{"userId": buddy.id, "role": "MANAGER"}]),
    )
    .await;
    let dm = env
        .call(
            &pat,
            "POST",
            &format!("/v1/workspaces/{}/dms", env.ws),
            Some(json!({"userIds": [pat.id, buddy.id]})),
        )
        .await;
    assert!(dm.status.is_success(), "{}", dm.body);
    let dm = dm.body["id"].as_str().unwrap().parse().unwrap();
    World {
        env,
        pat,
        buddy,
        team,
        group,
        via_team,
        direct,
        via_group,
        shared_doc,
        own_doc,
        dm,
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn removal_revokes_explicit_access_and_a_return_restores_none_of_it(pool: PgPool) {
    let w = world(pool.clone()).await;
    let env = &w.env;
    // A Team of pat's in a second Workspace is not touched.
    let other_ws = common::add_workspace(&pool, w.pat.id, "Other").await;
    let other_team = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.teams (id, workspace_id, name) VALUES ($1, $2, 'Elsewhere')")
        .bind(other_team)
        .bind(other_ws)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO penfii.team_members (team_id, user_id, role) VALUES ($1, $2, 'MANAGER')",
    )
    .bind(other_team)
    .bind(w.pat.id)
    .execute(&pool)
    .await
    .unwrap();
    for project in [w.via_team, w.direct, w.via_group] {
        assert_eq!(
            gets(env, &w.pat, &format!("/v1/projects/{project}/tickets")).await,
            StatusCode::OK
        );
    }
    assert_eq!(
        gets(env, &w.pat, &format!("/v1/docs/{}", w.shared_doc)).await,
        StatusCode::OK
    );
    let rows = |pool: PgPool, user: Uuid, ws: Uuid| async move {
        sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM penfii.team_members tm JOIN penfii.teams t ON t.id = tm.team_id
                      WHERE tm.user_id = $1 AND t.workspace_id = $2),
                    (SELECT count(*) FROM penfii.project_user_grants g JOIN penfii.projects p ON p.id = g.project_id
                      WHERE g.user_id = $1 AND p.workspace_id = $2),
                    (SELECT count(*) FROM penfii.group_members gm JOIN penfii.groups gr ON gr.id = gm.group_id
                      WHERE gm.user_id = $1 AND gr.workspace_id = $2),
                    (SELECT count(*) FROM penfii.doc_page_participants pp JOIN penfii.doc_pages p ON p.id = pp.page_id
                      WHERE pp.user_id = $1 AND p.workspace_id = $2),
                    (SELECT count(*) FROM penfii.dm_members WHERE user_id = $1),
                    (SELECT count(*) FROM penfii.doc_pages WHERE created_by = $1 AND workspace_id = $2)",
        )
        .bind(user)
        .bind(ws)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    // Held: Team, direct grant, Group, two Doc rows (the shared one and their own), the DM, their Doc.
    assert_eq!(
        rows(pool.clone(), w.pat.id, env.ws).await,
        (1, 1, 1, 2, 1, 1)
    );

    admin::remove_member(&pool, env.owner.id, env.ws, w.pat.id)
        .await
        .unwrap();
    // Gone: Team, grant, Group, the invited Doc row. Kept: the row on their own Doc, the DM, their Doc.
    assert_eq!(
        rows(pool.clone(), w.pat.id, env.ws).await,
        (0, 0, 0, 1, 1, 1)
    );
    // The second Workspace is untouched.
    assert_eq!(rows(pool.clone(), w.pat.id, other_ws).await.0, 1);
    // Both the Team and the Group still exist for the others.
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM penfii.teams WHERE id = $1 AND workspace_id = $2",
            w.team,
            env.ws
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM penfii.groups WHERE id = $1 AND workspace_id = $2",
            w.group,
            env.ws
        )
        .await,
        1
    );
    let audit: Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'member.removed' AND target_id = $1",
    )
    .bind(w.pat.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        audit["revoked"],
        json!({"teams": 1, "projects": 1, "groups": 1, "docShares": 1, "securityAuditor": 0, "channelAuditors": 0})
    );

    // The same person comes back, by invitation.
    let invitation = env.invite(&env.owner, &w.pat.email, "MEMBER").await;
    assert_eq!(
        invitation.status,
        StatusCode::CREATED,
        "{}",
        invitation.body
    );
    let signed_in = sign_in_again(env, &w.pat).await;
    let accepted = common::call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        Some(&signed_in.cookie),
        Some(json!({"token": invitation.body["inviteToken"], "displayName": "pat", "password": common::MEMBER_PASSWORD})),
    )
    .await;
    assert!(accepted.status.is_success(), "{}", accepted.body);
    let pat = sign_in_again(env, &w.pat).await;
    for project in [w.via_team, w.direct, w.via_group] {
        assert_eq!(
            gets(env, &pat, &format!("/v1/projects/{project}/tickets")).await,
            StatusCode::NOT_FOUND,
            "no Project access comes back"
        );
    }
    assert_eq!(
        gets(env, &pat, &format!("/v1/docs/{}", w.shared_doc)).await,
        StatusCode::NOT_FOUND,
        "the share is not restored"
    );
    assert_eq!(
        gets(env, &pat, &format!("/v1/docs/{}", w.own_doc)).await,
        StatusCode::OK,
        "the author keeps their own restricted Doc"
    );
    assert_eq!(
        gets(env, &pat, &format!("/v1/conversations/{}", w.dm)).await,
        StatusCode::OK,
        "the direct message is still theirs"
    );
    assert_eq!(
        gets(env, &w.buddy, &format!("/v1/conversations/{}", w.dm)).await,
        StatusCode::OK
    );

    // New, explicit grants work as for anyone.
    let granted = env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{}/grants/{}", w.direct, pat.id),
            Some(json!({"role": "VIEWER"})),
        )
        .await;
    assert!(granted.status.is_success(), "{}", granted.body);
    assert_eq!(
        gets(env, &pat, &format!("/v1/projects/{}/tickets", w.direct)).await,
        StatusCode::OK
    );
    let added = env
        .call(
            &env.owner,
            "PUT",
            &format!(
                "/v1/workspaces/{}/teams/{}/members/{}",
                env.ws, w.team, pat.id
            ),
            Some(json!({"role": "MEMBER"})),
        )
        .await;
    assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.body);
    assert_eq!(rows(pool.clone(), pat.id, env.ws).await.0, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn repeated_and_concurrent_removals_leave_nothing_behind(pool: PgPool) {
    let w = world(pool.clone()).await;
    let env = &w.env;
    // Two removals at once: one wins, the other finds nobody to remove.
    let (a, b) = tokio::join!(
        admin::remove_member(&pool, env.owner.id, env.ws, w.pat.id),
        admin::remove_member(&pool, env.owner.id, env.ws, w.pat.id),
    );
    assert!(
        a.is_ok() != b.is_ok(),
        "exactly one removal happens: {a:?} {b:?}"
    );
    for _ in 0..2 {
        membership::join_workspace_as_operator(&pool, env.ws, w.pat.id, "MEMBER")
            .await
            .unwrap();
        // While the person is being removed, grants and Team additions made in parallel are either
        // refused (not active) or swept up by the removal: none survives.
        let (removed, grant, team, shared) = tokio::join!(
            admin::remove_member(&pool, env.owner.id, env.ws, w.pat.id),
            penfii_core::project_access::set_grant(
                &pool,
                env.owner.id,
                w.direct,
                w.pat.id,
                "EDITOR"
            ),
            admin::add_team_member(&pool, env.owner.id, env.ws, w.team, w.pat.id, "MEMBER"),
            penfii_core::docs::set_participants(
                &pool,
                w.buddy.id,
                w.shared_doc,
                vec![(w.buddy.id, "MANAGER".into()), (w.pat.id, "EDITOR".into())]
            ),
        );
        removed.unwrap();
        let _ = (grant, team, shared);
        for sql in [
            "SELECT count(*) FROM penfii.project_user_grants WHERE user_id = $1 AND project_id = $2",
            "SELECT count(*) FROM penfii.team_members WHERE user_id = $1 AND team_id = $2",
            "SELECT count(*) FROM penfii.doc_page_participants WHERE user_id = $1 AND page_id = $2",
        ] {
            let other = if sql.contains("project_user") {
                w.direct
            } else if sql.contains("team_members") {
                w.team
            } else {
                w.shared_doc
            };
            assert_eq!(count(&pool, sql, w.pat.id, other).await, 0, "{sql}");
        }
    }
}

/// Removal revokes the manager row of the last manager too (FEA-149): the Doc keeps its content, nobody
/// manages it, and a Workspace owner or admin names a manager without being given the content.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_last_manager_is_revoked_and_the_doc_is_recovered_by_naming_a_manager(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let author = env.person("author", "MEMBER").await;
    let lead = env.person("lead", "MEMBER").await;
    let heir = env.person("heir", "MEMBER").await;
    let page = doc(
        &env,
        &author,
        "Handover",
        "RESTRICTED",
        json!([{"userId": lead.id, "role": "MANAGER"}]),
    )
    .await;
    // The author hands the management over and steps down to editor; the lead is now the only manager.
    let handed = env
        .call(
            &author,
            "PUT",
            &format!("/v1/docs/{page}/participants"),
            Some(json!({"participants": [{"userId": lead.id, "role": "MANAGER"}, {"userId": author.id, "role": "EDITOR"}]})),
        )
        .await;
    assert!(handed.status.is_success(), "{}", handed.body);
    // A Doc open to the whole Workspace is never listed as manager-less.
    let open = doc(&env, &lead, "Open", "WORKSPACE", json!([])).await;
    admin::remove_member(&pool, env.owner.id, env.ws, lead.id)
        .await
        .unwrap();
    let held =
        "SELECT count(*) FROM penfii.doc_page_participants WHERE user_id = $1 AND page_id = $2";
    assert_eq!(
        count(&pool, held, lead.id, page).await,
        0,
        "the manager row is revoked"
    );
    // Content and the other participants stay.
    assert_eq!(
        gets(&env, &author, &format!("/v1/docs/{page}")).await,
        StatusCode::OK
    );
    let author_view = env
        .call(&author, "GET", &format!("/v1/docs/{page}"), None)
        .await;
    assert_eq!(author_view.body["rights"]["manage"], json!(false));
    // The lead's own Open Doc: the creator's row and rights are kept.
    assert_eq!(
        gets(&env, &env.owner, &format!("/v1/docs/{open}")).await,
        StatusCode::OK
    );

    let list_path = format!("/v1/workspaces/{}/docs-without-manager", env.ws);
    let listed = env.call(&env.owner, "GET", &list_path, None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let items = listed.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["id"], json!(page));
    assert_eq!(items[0]["title"], "Handover");
    assert_eq!(items[0]["createdByName"], "author");
    assert!(items[0].get("content").is_none(), "metadata only");
    // Ordinary members neither list nor recover.
    assert_eq!(gets(&env, &author, &list_path).await, StatusCode::FORBIDDEN);
    let path = format!("/v1/docs/{page}/manager-recovery");
    let refused = env
        .call(&author, "POST", &path, Some(json!({"userId": author.id})))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    // The removed person is no candidate.
    let gone = env
        .call(&env.owner, "POST", &path, Some(json!({"userId": lead.id})))
        .await;
    assert_eq!(gone.status, StatusCode::BAD_REQUEST, "{}", gone.body);

    // Recovery by naming somebody else gives the owner nothing to read.
    assert_eq!(
        gets(&env, &env.owner, &format!("/v1/docs/{page}")).await,
        StatusCode::NOT_FOUND
    );
    let named = env
        .call(&env.owner, "POST", &path, Some(json!({"userId": heir.id})))
        .await;
    assert_eq!(named.status, StatusCode::OK, "{}", named.body);
    assert_eq!(named.body["newAccess"], json!(true));
    assert_eq!(
        gets(&env, &env.owner, &format!("/v1/docs/{page}")).await,
        StatusCode::NOT_FOUND
    );
    let heir_view = env
        .call(&heir, "GET", &format!("/v1/docs/{page}"), None)
        .await;
    assert_eq!(heir_view.status, StatusCode::OK);
    assert_eq!(heir_view.body["rights"]["manage"], json!(true));
    let entry: Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'doc.manager_recovered' AND target_id = $1",
    )
    .bind(page)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(entry["userId"], json!(heir.id));
    assert_eq!(entry["self"], json!(false));
    assert_eq!(entry["newAccess"], json!(true));
    // Now it has a manager: no longer listed, and a second recovery is refused.
    let listed = env.call(&env.owner, "GET", &list_path, None).await;
    assert!(listed.body["items"].as_array().unwrap().is_empty());
    let again = env
        .call(
            &env.owner,
            "POST",
            &path,
            Some(json!({"userId": env.owner.id})),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.body);
    assert_eq!(again.body["code"], "doc_has_manager");
    // A Workspace-wide Doc is not recovered this way.
    let open_try = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/docs/{open}/manager-recovery"),
            Some(json!({"userId": heir.id})),
        )
        .await;
    assert_eq!(open_try.status, StatusCode::CONFLICT);
    assert_eq!(open_try.body["code"], "doc_not_restricted");

    // Naming oneself is the one way an owner gets in, and the audit entry says so.
    admin::remove_member(&pool, env.owner.id, env.ws, heir.id)
        .await
        .unwrap();
    let own = env
        .call(
            &env.owner,
            "POST",
            &path,
            Some(json!({"userId": env.owner.id})),
        )
        .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(
        gets(&env, &env.owner, &format!("/v1/docs/{page}")).await,
        StatusCode::OK
    );
    let selfs: Vec<Value> = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'doc.manager_recovered' AND target_id = $1 ORDER BY occurred_at DESC",
    )
    .bind(page)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(selfs.len(), 2);
    assert_eq!(selfs[0]["self"], json!(true));
    assert_eq!(selfs[0]["newAccess"], json!(true));
}

/// Auditor rows are an explicit grant too: removal revokes them, and coming back does not restore them.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn removal_revokes_auditor_rows_in_that_workspace_only(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let pat = env.person("pat", "MEMBER").await;
    let other_ws = common::add_workspace(&pool, pat.id, "Other").await;
    let channel = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.conversations (id, workspace_id, kind) VALUES ($1, $2, 'CHANNEL')",
    )
    .bind(channel)
    .bind(env.ws)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.channels (id, name, visibility) VALUES ($1, 'audited', 'PRIVATE')",
    )
    .bind(channel)
    .execute(&pool)
    .await
    .unwrap();
    for sql in [
        "INSERT INTO penfii.workspace_security_auditors (workspace_id, user_id) VALUES ($1, $3)",
        "INSERT INTO penfii.workspace_security_auditors (workspace_id, user_id) VALUES ($2, $3)",
        "INSERT INTO penfii.channel_content_auditors (channel_id, user_id) VALUES ($4, $3)",
    ] {
        sqlx::query(sql)
            .bind(env.ws)
            .bind(other_ws)
            .bind(pat.id)
            .bind(channel)
            .execute(&pool)
            .await
            .unwrap();
    }
    let audit_path = format!("/v1/workspaces/{}/audit", env.ws);
    assert_eq!(gets(&env, &pat, &audit_path).await, StatusCode::OK);
    admin::remove_member(&pool, env.owner.id, env.ws, pat.id)
        .await
        .unwrap();
    let rows = |sql: &'static str| {
        let pool = pool.clone();
        async move { count(&pool, sql, pat.id, env.ws).await }
    };
    assert_eq!(rows("SELECT count(*) FROM penfii.workspace_security_auditors WHERE user_id = $1 AND workspace_id = $2").await, 0);
    assert_eq!(
        rows("SELECT count(*) FROM penfii.channel_content_auditors WHERE user_id = $1").await,
        0
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM penfii.workspace_security_auditors WHERE user_id = $1 AND workspace_id = $2", pat.id, other_ws).await,
        1,
        "the other Workspace keeps its row"
    );
    let audit: Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'member.removed' AND target_id = $1",
    )
    .bind(pat.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit["revoked"]["securityAuditor"], json!(1));
    assert_eq!(audit["revoked"]["channelAuditors"], json!(1));
    // A return does not bring the audit log back.
    membership::join_workspace_as_operator(&pool, env.ws, pat.id, "MEMBER")
        .await
        .unwrap();
    let pat = sign_in_again(&env, &pat).await;
    assert_eq!(gets(&env, &pat, &audit_path).await, StatusCode::FORBIDDEN);
}

/// The data clean-up of migration 0003 (its "removed member access" section), which the tests run again
/// over rows made after the schema is in place. The tables the file makes before it exist already.
fn removed_member_cleanup() -> &'static str {
    let file = include_str!("../../../database/migrations/0003_after_v0_3_1.sql");
    let start = file
        .find("-- ===== removed member access =====")
        .expect("the section of migration 0003");
    &file[start..]
}

/// Migration 0003 also takes the last manager row of a shared page and the auditor rows of people
/// who are out now.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn migration_0003_cleans_manager_rows_and_auditors_of_removed_people(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let lead = env.person("lead", "MEMBER").await;
    let kept = env.person("kept", "MEMBER").await;
    let creator = env.person("creator", "MEMBER").await;
    let other_ws = common::add_workspace(&pool, lead.id, "Other").await;
    let orphan = doc(
        &env,
        &kept,
        "Orphan",
        "RESTRICTED",
        json!([{"userId": lead.id, "role": "MANAGER"}]),
    )
    .await;
    let mine = doc(&env, &creator, "Mine", "RESTRICTED", json!([])).await;
    sqlx::query("DELETE FROM penfii.doc_page_participants WHERE page_id = $1 AND user_id = $2")
        .bind(orphan)
        .bind(kept.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.workspace_security_auditors (workspace_id, user_id) VALUES ($1, $2), ($1, $3), ($4, $2)")
        .bind(env.ws)
        .bind(lead.id)
        .bind(kept.id)
        .bind(other_ws)
        .execute(&pool)
        .await
        .unwrap();
    // `lead` and `creator` were removed by an earlier release: only the membership flipped.
    sqlx::query("UPDATE penfii.workspace_members SET state = 'INACTIVE', deactivated_at = now() WHERE workspace_id = $1 AND user_id = ANY($2)")
        .bind(env.ws)
        .bind(vec![lead.id, creator.id])
        .execute(&pool)
        .await
        .unwrap();
    let migration = removed_member_cleanup();
    sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    let held =
        "SELECT count(*) FROM penfii.doc_page_participants WHERE user_id = $1 AND page_id = $2";
    assert_eq!(
        count(&pool, held, lead.id, orphan).await,
        0,
        "the manager row the old exception kept"
    );
    assert_eq!(
        count(&pool, held, creator.id, mine).await,
        1,
        "the creator's own row stays"
    );
    let auditors = |user: Uuid, ws: Uuid| {
        let pool = pool.clone();
        async move {
            count(&pool, "SELECT count(*) FROM penfii.workspace_security_auditors WHERE user_id = $1 AND workspace_id = $2", user, ws).await
        }
    };
    assert_eq!(auditors(lead.id, env.ws).await, 0);
    assert_eq!(
        auditors(lead.id, other_ws).await,
        1,
        "other Workspace untouched"
    );
    assert_eq!(
        auditors(kept.id, env.ws).await,
        1,
        "an active member keeps theirs"
    );
    let entries: Vec<Value> = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'member.access_cleaned'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["via"], "migration 0003");
    assert_eq!(entries[0]["revoked"]["docShares"], json!(1));
    assert_eq!(entries[0]["revoked"]["securityAuditor"], json!(1));
    // The Doc is listed for recovery, and applying the file again changes nothing.
    let listed = env
        .call(
            &env.owner,
            "GET",
            &format!("/v1/workspaces/{}/docs-without-manager", env.ws),
            None,
        )
        .await;
    assert_eq!(
        listed.body["items"].as_array().unwrap().len(),
        2,
        "Orphan and the creator's page whose manager is out: {}",
        listed.body
    );
    sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    let again: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'member.access_cleaned'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(again, 1);
}

/// What an earlier release left for a removed person, and what the migration does to it.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn migration_0003_cleans_the_leftovers_of_people_removed_before_it(pool: PgPool) {
    let w = world(pool.clone()).await;
    let env = &w.env;
    let other_ws = common::add_workspace(&pool, w.pat.id, "Other").await;
    let other_project = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.projects (id, workspace_id, name, ticket_prefix) VALUES ($1, $2, 'Elsewhere', 'ELS')")
        .bind(other_project)
        .bind(other_ws)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, 'ADMIN')")
        .bind(other_project)
        .bind(w.pat.id)
        .execute(&pool)
        .await
        .unwrap();
    // A Project pat alone administered directly, then pat was removed the way v0.3.1 did: only the
    // membership flips, every grant stays.
    let solo = project(env, "Solo").await;
    for (who, role) in [(w.pat.id, "ADMIN")] {
        sqlx::query("INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(solo)
            .bind(who)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM penfii.project_user_grants WHERE project_id = $1 AND user_id = $2")
        .bind(solo)
        .bind(env.owner.id)
        .execute(&pool)
        .await
        .unwrap();
    // A second person was removed and has since been taken back: their access is current and stays.
    let back = env.person("back", "MEMBER").await;
    sqlx::query("INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, 'EDITOR')")
        .bind(w.direct)
        .bind(back.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE penfii.workspace_members SET state = 'INACTIVE', deactivated_at = now() WHERE workspace_id = $1 AND user_id = $2")
        .bind(env.ws)
        .bind(w.pat.id)
        .execute(&pool)
        .await
        .unwrap();
    let before_audit: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    let snapshot = |pool: PgPool| async move {
        sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM penfii.team_members),
                    (SELECT count(*) FROM penfii.project_user_grants),
                    (SELECT count(*) FROM penfii.group_members),
                    (SELECT count(*) FROM penfii.doc_page_participants),
                    (SELECT count(*) FROM penfii.dm_members)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let (teams, grants, groups, doc_rows, dms) = snapshot(pool.clone()).await;

    let migration = removed_member_cleanup();
    sqlx::raw_sql(migration).execute(&pool).await.unwrap();

    // pat: one Team row, three grants in this Workspace (direct, solo, ...) and one in Other stays,
    // one Group row, one invited Doc row go. The row on their own Doc, the DM and the other Workspace stay.
    let (teams_after, grants_after, groups_after, doc_rows_after, dms_after) =
        snapshot(pool.clone()).await;
    assert_eq!(teams - teams_after, 1);
    assert_eq!(
        grants - grants_after,
        2,
        "the direct grant and the Solo grant, not Other's"
    );
    assert_eq!(groups - groups_after, 1);
    assert_eq!(doc_rows - doc_rows_after, 1);
    assert_eq!(dms, dms_after);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM penfii.project_user_grants WHERE user_id = $1 AND project_id = $2", w.pat.id, other_project).await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM penfii.doc_page_participants WHERE user_id = $1 AND page_id = $2",
            w.pat.id,
            w.own_doc
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM penfii.project_user_grants WHERE user_id = $1 AND project_id = $2", back.id, w.direct).await,
        1,
        "an active member's access is not touched"
    );
    let entries: Vec<Value> = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'member.access_cleaned'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["userId"], json!(w.pat.id));
    assert_eq!(
        entries[0]["revoked"],
        json!({"teams": 1, "projects": 2, "groups": 1, "docShares": 1, "securityAuditor": 0, "channelAuditors": 0})
    );
    assert_eq!(entries[0]["directAdminOf"], json!([solo]));
    // The Project pat alone administered is now one without an admin, and the list still says who held it.
    let listed = env
        .call(
            &env.owner,
            "GET",
            &format!("/v1/workspaces/{}/projects-without-admin", env.ws),
            None,
        )
        .await;
    let projects = listed.body["items"].as_array().unwrap();
    let row = projects
        .iter()
        .find(|p| p["id"] == json!(solo))
        .expect("Solo is listed");
    assert_eq!(row["formerAdmins"][0]["displayName"], "pat");
    // The Workspace owner recovers it as before.
    let recovered = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/projects/{solo}/admin-recovery"),
            Some(json!({"userId": env.owner.id})),
        )
        .await;
    assert!(recovered.status.is_success(), "{}", recovered.body);

    // Applying it again changes nothing, adds no audit entry, and the membership rows are as they were.
    sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    assert_eq!(
        snapshot(pool.clone()).await,
        (
            teams_after,
            grants_after + 1,
            groups_after,
            doc_rows_after,
            dms_after
        )
    );
    let after_audit: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        after_audit,
        before_audit + 1 + 1,
        "one cleanup entry and the recovery entry"
    );
}
