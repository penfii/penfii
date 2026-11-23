//! Upgrade proof for FEA-149, run by hand (it is `#[ignore]`d, it needs a database that a v0.3.1
//! build made and the current `admin-cli migrate` upgraded): removed people come back as new members.
//!
//! The database is named by `PENFII_UPGRADE_DATABASE_URL`; the people and the data are the ones the
//! seed of the upgrade rehearsal wrote (`owner@up.test`, `active@up.test`, `gone@up.test`,
//! `gone2@up.test`, all with the password `upgrade-passphrase-1`). The seed and the steps are in
//! `server/tests/upgrade/` (`setup.sh`, `seed.sql`, `snapshot.sql`). Nothing here runs in CI.

use axum::http::StatusCode;
use penfii_core::membership;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::common::{self, Person};

const PASSWORD: &str = "upgrade-passphrase-1";

struct Ctx {
    app: axum::Router,
    pool: PgPool,
}

impl Ctx {
    async fn get(&self, who: &Person, path: &str) -> (StatusCode, Value) {
        let reply = common::call(&self.app, "GET", path, Some(&who.cookie), None).await;
        (reply.status, reply.body)
    }
    async fn status(&self, who: &Person, path: &str) -> StatusCode {
        self.get(who, path).await.0
    }
    async fn person(&self, name: &str) -> Person {
        let email = format!("{name}@up.test");
        let id: Uuid = sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = $1")
            .bind(&email)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        let cookie = common::sign_in(&self.app, &email, PASSWORD).await;
        Person { id, email, cookie }
    }
    async fn id(&self, sql: &str, name: &str) -> Uuid {
        sqlx::query_scalar(sql)
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn check(label: &str, got: impl std::fmt::Debug, want: impl std::fmt::Debug) {
    let (got, want) = (format!("{got:?}"), format!("{want:?}"));
    println!(
        "{} {label}: {got}",
        if got == want { "PASS" } else { "FAIL" }
    );
    assert_eq!(got, want, "{label}");
}

#[tokio::test]
#[ignore = "needs the upgraded rehearsal database (PENFII_UPGRADE_DATABASE_URL)"]
async fn removed_people_who_return_after_the_upgrade_get_none_of_their_old_rights_back() {
    let url = std::env::var("PENFII_UPGRADE_DATABASE_URL").expect("PENFII_UPGRADE_DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap();
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        webhook_kek: Some(common::webhook_kek()),
        ..penfii_http_api::RouterConfig::new(pool.clone(), common::limits(), false)
    });
    let c = Ctx {
        app,
        pool: pool.clone(),
    };
    let w1: Uuid = c
        .id("SELECT id FROM penfii.workspaces WHERE name = $1", "W1")
        .await;
    let w2: Uuid = c
        .id("SELECT id FROM penfii.workspaces WHERE name = $1", "W2")
        .await;
    let q = "SELECT id FROM penfii.projects WHERE name = $1";
    let (p1, p2, p3, p4) = (
        c.id(q, "ByGrant").await,
        c.id(q, "ByTeam").await,
        c.id(q, "ByGroup").await,
        c.id(q, "OtherWs").await,
    );
    let q = "SELECT id FROM penfii.doc_pages WHERE title LIKE $1 || '%'";
    let (d1, d2, d3, d4, d6) = (
        c.id(q, "D1").await,
        c.id(q, "D2").await,
        c.id(q, "D3").await,
        c.id(q, "D4").await,
        c.id(q, "D6").await,
    );
    let team: Uuid = c
        .id("SELECT id FROM penfii.teams WHERE name = $1", "T1")
        .await;
    let channel: Uuid = c
        .id("SELECT id FROM penfii.channels WHERE name = $1", "secret")
        .await;
    let dm: Uuid = sqlx::query_scalar("SELECT id FROM penfii.conversations WHERE kind = 'DM'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let secret_message: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.messages WHERE conversation_id = $1")
            .bind(channel)
            .fetch_one(&pool)
            .await
            .unwrap();
    let owner = c.person("owner").await;
    let active = c.person("active").await;
    let gone = c.person("gone").await;
    let gone2 = c.person("gone2").await;
    let tickets = |p: Uuid| format!("/v1/projects/{p}/tickets");
    let docp = |d: Uuid| format!("/v1/docs/{d}");
    let audit = |w: Uuid| format!("/v1/workspaces/{w}/audit");
    let history = format!("/v1/messages/{secret_message}/history");

    println!("== after the upgrade, before anyone returns");
    for (who, name) in [(gone.id, "gone"), (gone2.id, "gone2")] {
        for (table, sql) in [
            (
                "team_members",
                "SELECT count(*) FROM penfii.team_members tm JOIN penfii.teams t ON t.id=tm.team_id WHERE tm.user_id=$1 AND t.workspace_id=$2",
            ),
            (
                "project_user_grants",
                "SELECT count(*) FROM penfii.project_user_grants g JOIN penfii.projects p ON p.id=g.project_id WHERE g.user_id=$1 AND p.workspace_id=$2",
            ),
            (
                "group_members",
                "SELECT count(*) FROM penfii.group_members g JOIN penfii.groups x ON x.id=g.group_id WHERE g.user_id=$1 AND x.workspace_id=$2",
            ),
            (
                "doc_page_participants (not own)",
                "SELECT count(*) FROM penfii.doc_page_participants pp JOIN penfii.doc_pages p ON p.id=pp.page_id WHERE pp.user_id=$1 AND p.workspace_id=$2 AND p.created_by<>pp.user_id",
            ),
            (
                "workspace_security_auditors",
                "SELECT count(*) FROM penfii.workspace_security_auditors WHERE user_id=$1 AND workspace_id=$2",
            ),
        ] {
            let n: i64 = sqlx::query_scalar(sql)
                .bind(who)
                .bind(w1)
                .fetch_one(&pool)
                .await
                .unwrap();
            check(&format!("{name} {table} rows in W1"), n, 0);
        }
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.channel_content_auditors WHERE user_id=$1",
        )
        .bind(who)
        .fetch_one(&pool)
        .await
        .unwrap();
        check(&format!("{name} channel_content_auditors rows"), n, 0);
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.doc_page_participants WHERE page_id=$1 AND user_id=$2",
    )
    .bind(d3)
    .bind(gone.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    check(
        "gone's participant row on their OWN doc D3 (author-based) is kept",
        n,
        1,
    );
    let listed = c
        .get(&owner, &format!("/v1/workspaces/{w1}/docs-without-manager"))
        .await
        .1;
    let mut titles: Vec<String> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    titles.sort();
    check(
        "docs without a manager (admin list, metadata only)",
        titles,
        [
            "D1 last manager gone",
            "D3 authored by gone",
            "D4 last manager gone2",
        ],
    );
    check(
        "a plain member cannot list them",
        c.status(
            &active,
            &format!("/v1/workspaces/{w1}/docs-without-manager"),
        )
        .await,
        StatusCode::FORBIDDEN,
    );
    println!("== other Workspace (W2) rights before the return");
    check(
        "gone reads W2 project",
        c.status(&gone, &tickets(p4)).await,
        StatusCode::OK,
    );
    check(
        "gone reads W2 doc D6 (shared)",
        c.status(&gone, &docp(d6)).await,
        StatusCode::OK,
    );
    check(
        "gone reads W2 audit log (security auditor)",
        c.status(&gone, &audit(w2)).await,
        StatusCode::OK,
    );

    println!(
        "== gone returns through the real invitation flow; gone2 through the operator add-member function"
    );
    let invite = common::call(
        &c.app,
        "POST",
        &format!("/v1/workspaces/{w1}/invitations"),
        Some(&owner.cookie),
        Some(json!({"email": gone.email, "role": "MEMBER"})),
    )
    .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let accepted = common::call(&c.app, "POST", "/v1/invitations/accept", Some(&gone.cookie), Some(json!({"token": invite.body["inviteToken"], "displayName": "gone", "password": PASSWORD}))).await;
    assert!(accepted.status.is_success(), "{}", accepted.body);
    let outcome = membership::operator_add_member(&pool, w1, &gone2.email, "MEMBER")
        .await
        .unwrap();
    check(
        "gone2 operator outcome",
        format!("{:?}", outcome.outcome),
        "Rejoined",
    );
    let gone = c.person("gone").await;
    let gone2 = c.person("gone2").await;
    let state: String = sqlx::query_scalar(
        "SELECT state FROM penfii.workspace_members WHERE workspace_id=$1 AND user_id=$2",
    )
    .bind(w1)
    .bind(gone.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    check(
        "gone is an ACTIVE member of W1 again",
        state,
        "ACTIVE".to_owned(),
    );

    println!("== after the return: old rights must not be back");
    for (who, name) in [(&gone, "gone"), (&gone2, "gone2")] {
        check(
            &format!("{name}: Project via direct ADMIN/EDITOR grant (P1)"),
            c.status(who, &tickets(p1)).await,
            StatusCode::NOT_FOUND,
        );
        check(
            &format!("{name}: Project via Team T1 (P2)"),
            c.status(who, &tickets(p2)).await,
            StatusCode::NOT_FOUND,
        );
        check(
            &format!("{name}: Project via Group G1 (P3)"),
            c.status(who, &tickets(p3)).await,
            StatusCode::NOT_FOUND,
        );
        check(
            &format!("{name}: W1 audit log (security auditor)"),
            c.status(who, &audit(w1)).await,
            StatusCode::FORBIDDEN,
        );
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.team_members tm JOIN penfii.teams t ON t.id=tm.team_id WHERE tm.user_id=$1 AND t.workspace_id=$2").bind(who.id).bind(w1).fetch_one(&pool).await.unwrap();
        check(&format!("{name}: Team rows in W1"), n, 0);
    }
    check(
        "gone: restricted Doc D1 where they were the last manager",
        c.status(&gone, &docp(d1)).await,
        StatusCode::NOT_FOUND,
    );
    check(
        "gone: restricted Doc D2 shared with them",
        c.status(&gone, &docp(d2)).await,
        StatusCode::NOT_FOUND,
    );
    check(
        "gone2: restricted Doc D4 where they were the last manager",
        c.status(&gone2, &docp(d4)).await,
        StatusCode::NOT_FOUND,
    );
    check(
        "gone: channel auditor view of the private channel message history",
        c.status(&gone, &history).await,
        StatusCode::NOT_FOUND,
    );
    check(
        "gone: leftover membership of the private channel",
        c.status(&gone, &format!("/v1/conversations/{channel}"))
            .await,
        StatusCode::NOT_FOUND,
    );
    let team_members = c
        .get(&owner, &format!("/v1/workspaces/{w1}/teams/{team}/members"))
        .await
        .1;
    let names: Vec<String> = team_members["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["displayName"].as_str().unwrap().to_owned())
        .collect();
    check("Team T1 roster (admin view)", names, ["active"]);

    println!("== after the return: what must be intact");
    let (st, own) = c.get(&gone, &docp(d3)).await;
    check(
        "gone: Doc D3 they authored (author-based access)",
        st,
        StatusCode::OK,
    );
    check(
        "gone: manages D3 (their own row)",
        own["rights"]["manage"].clone(),
        json!(true),
    );
    check(
        "gone: their DM",
        c.status(&gone, &format!("/v1/conversations/{dm}")).await,
        StatusCode::OK,
    );
    let msgs = c
        .get(&gone, &format!("/v1/conversations/{dm}/messages"))
        .await
        .1
        .to_string();
    check(
        "gone: DM history keeps both messages",
        (
            msgs.contains("gone wrote this in the dm"),
            msgs.contains("active replied in the dm"),
        ),
        (true, true),
    );
    check(
        "active (never removed): P1",
        c.status(&active, &tickets(p1)).await,
        StatusCode::OK,
    );
    check(
        "active: P2 via Team T1",
        c.status(&active, &tickets(p2)).await,
        StatusCode::OK,
    );
    check(
        "active: P3 via Group G1",
        c.status(&active, &tickets(p3)).await,
        StatusCode::OK,
    );
    for (label, d) in [("D1", d1), ("D2", d2), ("D3", d3), ("D4", d4)] {
        check(
            &format!("active: Doc {label}"),
            c.status(&active, &docp(d)).await,
            StatusCode::OK,
        );
    }
    check(
        "active: W1 audit log (security auditor kept)",
        c.status(&active, &audit(w1)).await,
        StatusCode::OK,
    );
    check(
        "active: channel auditor history kept",
        c.status(&active, &history).await,
        StatusCode::OK,
    );
    for (who, name) in [(&gone, "gone"), (&gone2, "gone2")] {
        check(
            &format!("{name}: W2 project still readable"),
            c.status(who, &tickets(p4)).await,
            StatusCode::OK,
        );
        check(
            &format!("{name}: W2 audit log (security auditor) still readable"),
            c.status(who, &audit(w2)).await,
            StatusCode::OK,
        );
    }
    check(
        "gone: W2 doc D6 still shared",
        c.status(&gone, &docp(d6)).await,
        StatusCode::OK,
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.team_members tm JOIN penfii.teams t ON t.id=tm.team_id WHERE tm.user_id=$1 AND t.workspace_id=$2").bind(gone.id).bind(w2).fetch_one(&pool).await.unwrap();
    check("gone: W2 Team row", n, 1);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.group_members g JOIN penfii.groups x ON x.id=g.group_id WHERE g.user_id=$1 AND x.workspace_id=$2").bind(gone.id).bind(w2).fetch_one(&pool).await.unwrap();
    check("gone: W2 Group row", n, 1);

    println!("== audit trail and manager-less docs after the return");
    let removed: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code='member.removed' AND workspace_id=$1").bind(w1).fetch_one(&pool).await.unwrap();
    check("the earlier member.removed entries are kept", removed, 2);
    let rows = sqlx::query("SELECT target_id, data->>'via' AS via, data->'revoked' AS revoked FROM penfii.audit_entries WHERE action_code='member.access_cleaned' ORDER BY occurred_at, via").fetch_all(&pool).await.unwrap();
    for r in &rows {
        println!(
            "INFO member.access_cleaned user={} via={} revoked={}",
            r.get::<Uuid, _>("target_id"),
            r.get::<String, _>("via"),
            r.get::<Value, _>("revoked")
        );
    }
    let joined: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code='member.joined' AND data->>'rejoined'='true'").fetch_one(&pool).await.unwrap();
    check("member.joined (rejoined) entries", joined, 2);
    let listed = c
        .get(&owner, &format!("/v1/workspaces/{w1}/docs-without-manager"))
        .await
        .1;
    let mut titles: Vec<String> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    titles.sort();
    check(
        "docs without a manager now (D3 has its author back)",
        titles,
        ["D1 last manager gone", "D4 last manager gone2"],
    );
    check(
        "the admin cannot read D1 without being named",
        c.status(&owner, &docp(d1)).await,
        StatusCode::NOT_FOUND,
    );
    let named = common::call(
        &c.app,
        "POST",
        &format!("/v1/docs/{d1}/manager-recovery"),
        Some(&owner.cookie),
        Some(json!({"userId": active.id})),
    )
    .await;
    check(
        "recovery names `active` manager of D1 (200, newAccess=false: they could read it)",
        (named.status, named.body["newAccess"].clone()),
        (StatusCode::OK, json!(false)),
    );
    check(
        "owner still cannot read D1",
        c.status(&owner, &docp(d1)).await,
        StatusCode::NOT_FOUND,
    );
    check(
        "rejoined gone still cannot read D1",
        c.status(&gone, &docp(d1)).await,
        StatusCode::NOT_FOUND,
    );

    println!("== a new explicit grant works");
    let g = common::call(
        &c.app,
        "PUT",
        &format!("/v1/projects/{p1}/grants/{}", gone.id),
        Some(&owner.cookie),
        Some(json!({"role": "VIEWER"})),
    )
    .await;
    assert!(g.status.is_success(), "{}", g.body);
    check(
        "gone reads P1 only after the new VIEWER grant",
        c.status(&gone, &tickets(p1)).await,
        StatusCode::OK,
    );
}
