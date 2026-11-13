use clap::{Parser, Subcommand};
mod password;
use password::PasswordArgs;
use penfii_core::identity::{self, BootstrapInput};
use penfii_core::membership::{self, JoinOutcome, OperatorJoin};
use penfii_core::{apply_migrations, connect, require_current_schema};
use sqlx::Connection;
use sqlx::PgPool;

#[derive(Parser)]
#[command(name = "admin-cli")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a new VAPID key pair for Web Push (PENFII_VAPID_PRIVATE_KEY and the public key). Needs no database.
    VapidKeys,
    /// Create or update the database roles: the schema owner (the migration credential) and the runtime
    /// role core-api connects as, with their privileges and the runtime role's timeouts. Needs a
    /// privileged credential in PENFII_PROVISION_DATABASE_URL (normally the PostgreSQL superuser) and
    /// does not touch an owner role that already exists. Passwords come from PENFII_DB_OWNER_PASSWORD
    /// and PENFII_DB_RUNTIME_PASSWORD, never from the command line. Safe to run again; run it again
    /// after restoring a dump (roles are not part of a database dump).
    DbProvision {
        /// The role that owns the schema and runs migrations. An existing role is left as it is.
        #[arg(long, default_value = "penfii_owner")]
        owner_role: String,
        /// The role core-api and this tool connect as.
        #[arg(long, default_value = "penfii_app")]
        runtime_role: String,
    },
    /// Apply the schema migrations of this build, with the owner's credential in
    /// PENFII_MIGRATION_DATABASE_URL (core-api and the other commands never migrate).
    Migrate,
    /// Create the first user and an owned workspace. Refuses when any user exists.
    Bootstrap {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        password: PasswordArgs,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        workspace_name: String,
        #[arg(long)]
        time_zone: String,
    },
    /// Create a new account and add it to an existing workspace (find the id with workspace-list).
    /// An e-mail that already has an account is refused; add that account with workspace-add-member.
    CreateUser {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        password: PasswordArgs,
        #[arg(long)]
        display_name: String,
        /// Id of the workspace the account joins.
        #[arg(long)]
        workspace: uuid::Uuid,
        /// OWNER, ADMIN or MEMBER.
        #[arg(long, default_value = "MEMBER")]
        role: String,
    },
    /// Add an existing account to a workspace. A removed member comes back with this role; a member
    /// who is already in is reported and left as is (change the role with workspace-set-role); a
    /// disabled account has to be enabled first.
    WorkspaceAddMember {
        #[arg(long)]
        email: String,
        /// Id of the workspace the account joins.
        #[arg(long)]
        workspace: uuid::Uuid,
        /// OWNER, ADMIN or MEMBER.
        #[arg(long, default_value = "MEMBER")]
        role: String,
    },
    /// Change the role of an active member. The last owner of a workspace cannot be changed to
    /// anything else. For when no owner can do it in the web app.
    WorkspaceSetRole {
        #[arg(long)]
        email: String,
        /// Id of the workspace.
        #[arg(long)]
        workspace: uuid::Uuid,
        /// OWNER, ADMIN or MEMBER.
        #[arg(long)]
        role: String,
    },
    /// List the workspaces with their ids, active member counts and owners.
    WorkspaceList,
    /// Show how an account can sign in (password, linked providers). Recovery for a lost provider.
    UserIdentities {
        #[arg(long)]
        email: String,
    },
    /// Set or replace an account's password and end all of its sessions. Writes an audit entry.
    UserSetPassword {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        password: PasswordArgs,
    },
    /// Disable an account: ends all its sessions and OAuth grants and blocks sign-in. For when no
    /// workspace owner can do it. Logged in the audit log of each of its workspaces. Prints the
    /// projects it leaves without an admin; it is never refused for that.
    UserDisable {
        #[arg(long)]
        email: String,
    },
    /// Enable a disabled account again (workspace membership is unchanged).
    UserEnable {
        #[arg(long)]
        email: String,
    },
    /// Show the search index state per kind and any failed event deliveries.
    SearchStatus,
    /// List deliveries that failed too often and were given up on (Inbox, Watch, search).
    EventsDead,
    /// Give dead deliveries new attempts. Name a consumer, an event, or pass --all.
    EventsRetry {
        #[arg(long)]
        consumer: Option<String>,
        #[arg(long)]
        event: Option<uuid::Uuid>,
        #[arg(long)]
        all: bool,
    },
    /// Show the co-editing state of Doc pages: lineage, how far the log is ahead of the last
    /// materialization, and pages the materializer refused. One page with --page.
    DocsCollabStatus {
        #[arg(long)]
        page: Option<uuid::Uuid>,
    },
    /// Replace a page's Yjs working state by a new lineage seeded from the last good PostgreSQL
    /// content. Connected clients are told to reconnect. Edits not yet materialized are lost.
    DocCollabReset {
        #[arg(long)]
        page: uuid::Uuid,
    },
    /// Build the search index again from PostgreSQL (all kinds, or one with --kind).
    SearchReindex {
        #[arg(long)]
        kind: Option<String>,
    },
}

/// Prints the message to stderr and exits with status 1.
fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

/// Every command but `migrate` and `db-provision` works on the database named by `DATABASE_URL` (the
/// runtime role), which must already be migrated to this build: only `migrate` changes the schema.
async fn open_database() -> PgPool {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| fail("DATABASE_URL is required"));
    let pool = connect(&database_url)
        .await
        .unwrap_or_else(|error| fail(format!("database: {error}")));
    require_current_schema(&pool)
        .await
        .unwrap_or_else(|error| fail(format!("schema: {error}")));
    pool
}

/// `admin-cli migrate`: the migration credential applies the pending migrations.
async fn run_migrate() {
    let Some(url) = penfii_core::migration_url_from_env() else {
        fail("PENFII_MIGRATION_DATABASE_URL is required (the owner role's connection string)");
    };
    let pool = connect(&url)
        .await
        .unwrap_or_else(|error| fail(format!("database: {error}")));
    apply_migrations(&pool)
        .await
        .unwrap_or_else(|error| fail(format!("migrate: {error}")));
    penfii_core::protect_history(&pool)
        .await
        .unwrap_or_else(|error| fail(format!("migrate: {error}")));
    let (count, latest): (i64, Option<i64>) =
        sqlx::query_as("SELECT count(*), max(version) FROM penfii._sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|error| fail(format!("migrate: {error}")));
    println!(
        "schema is current: {count} migrations applied, the latest is {}",
        latest.unwrap_or(0)
    );
}

/// `admin-cli db-provision`.
async fn run_db_provision(owner_role: String, runtime_role: String) {
    let url = std::env::var("PENFII_PROVISION_DATABASE_URL").unwrap_or_else(|_| {
        fail("PENFII_PROVISION_DATABASE_URL is required (a privileged connection string)")
    });
    let password = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let plan = penfii_core::provision::Provision {
        owner_role,
        owner_password: password("PENFII_DB_OWNER_PASSWORD"),
        runtime_role,
        runtime_password: password("PENFII_DB_RUNTIME_PASSWORD"),
    };
    let mut conn = sqlx::PgConnection::connect(&url)
        .await
        .unwrap_or_else(|error| fail(format!("database: {error}")));
    match penfii_core::provision::provision(&mut conn, &plan).await {
        Ok(done) => {
            println!(
                "database {}: owner role {} ({}), runtime role {} ({})",
                done.database,
                plan.owner_role,
                if done.owner_created {
                    "created"
                } else {
                    "exists, left as it is"
                },
                plan.runtime_role,
                if done.runtime_created {
                    "created"
                } else {
                    "updated"
                },
            );
        }
        Err(error) => fail(format!("db-provision: {error}")),
    }
}

fn search_engine(pool: &PgPool) -> penfii_core::search::SearchEngine {
    let config = match penfii_core::search::client::OsConfig::from_env() {
        Ok(Some(config)) => config,
        Ok(None) => fail("PENFII_OPENSEARCH_URL is required"),
        Err(error) => fail(error),
    };
    let client = penfii_core::search::client::OsClient::new(config)
        .unwrap_or_else(|error| fail(format!("search client: {error}")));
    penfii_core::search::SearchEngine::new(pool.clone(), client)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if matches!(cli.command, Command::VapidKeys) {
        let (private, public) = penfii_core::push::VapidConfig::generate();
        println!("PENFII_VAPID_PRIVATE_KEY={private}");
        println!("# public key (the server reports it to browsers): {public}");
        return;
    }
    // These two work with their own credentials, not with `DATABASE_URL`.
    match cli.command {
        Command::Migrate => {
            run_migrate().await;
            return;
        }
        Command::DbProvision {
            owner_role,
            runtime_role,
        } => {
            run_db_provision(owner_role, runtime_role).await;
            return;
        }
        _ => {}
    }
    // The password is read before the database is opened, so a bad or missing input never reaches it.
    let password = match &cli.command {
        Command::Bootstrap { password, .. }
        | Command::CreateUser { password, .. }
        | Command::UserSetPassword { password, .. } => {
            Some(password::resolve(password).unwrap_or_else(|error| fail(error)))
        }
        _ => None,
    };
    let pool = open_database().await;
    match cli.command {
        Command::VapidKeys | Command::Migrate | Command::DbProvision { .. } => {
            unreachable!("handled before the database is opened")
        }
        Command::Bootstrap {
            email,
            display_name,
            workspace_name,
            time_zone,
            ..
        } => {
            let input = BootstrapInput {
                email,
                password: password.expect("read above"),
                display_name,
                workspace_name,
                time_zone,
            };
            print_account(identity::bootstrap(&pool, input).await);
        }
        Command::CreateUser {
            email,
            display_name,
            workspace,
            role,
            ..
        } => {
            let password = password.expect("read above");
            let joined = membership::operator_create_member(
                &pool,
                workspace,
                &email,
                &password,
                &display_name,
                &role,
            )
            .await
            .unwrap_or_else(|error| fail(format!("create-user: {}", explain(&error))));
            print_join(&joined, &role);
        }
        Command::WorkspaceAddMember {
            email,
            workspace,
            role,
        } => {
            let joined = membership::operator_add_member(&pool, workspace, &email, &role)
                .await
                .unwrap_or_else(|error| fail(format!("workspace-add-member: {}", explain(&error))));
            if let JoinOutcome::AlreadyMember { role: current } = &joined.outcome {
                fail(format!(
                    "workspace-add-member: the account {} is already a member of {} as {current}; nothing was changed (workspace-set-role changes the role)",
                    joined.user_id, joined.workspace_id
                ));
            }
            print_join(&joined, &role);
        }
        Command::WorkspaceSetRole {
            email,
            workspace,
            role,
        } => {
            let change =
                penfii_core::admin::operator_set_member_role(&pool, workspace, &email, &role)
                    .await
                    .unwrap_or_else(|error| {
                        fail(format!("workspace-set-role: {}", explain(&error)))
                    });
            println!(
                "result {}",
                if change.changed {
                    "changed"
                } else {
                    "unchanged"
                }
            );
            println!("user_id {}", change.user_id);
            println!("workspace_id {workspace}");
            println!("role {}", change.after);
            if change.changed {
                println!("was {}", change.before);
            }
        }
        Command::WorkspaceList => {
            let workspaces = penfii_core::admin::operator_list_workspaces(&pool)
                .await
                .unwrap_or_else(|error| fail(format!("workspace-list: {}", explain(&error))));
            for w in &workspaces {
                // A disabled account cannot sign in, so it is marked wherever it is counted.
                let owners: Vec<String> = w
                    .owners
                    .iter()
                    .map(|email| {
                        if w.disabled_owners.contains(email) {
                            format!("{email}(disabled)")
                        } else {
                            email.clone()
                        }
                    })
                    .collect();
                println!(
                    "{} members={} disabled={} owners={} time_zone={} created={} name={:?}",
                    w.id,
                    w.active_members,
                    w.disabled_members,
                    if owners.is_empty() {
                        "-".to_string()
                    } else {
                        owners.join(",")
                    },
                    w.time_zone,
                    w.created_at.to_rfc3339(),
                    w.name
                );
            }
            println!("workspaces: {}", workspaces.len());
        }
        Command::UserIdentities { email } => {
            let found = identity::admin_identities(&pool, &email)
                .await
                .unwrap_or_else(|error| fail(format!("identities: {error:?}")));
            println!(
                "user {} password={} disabled={}",
                found.user_id, found.has_password, found.disabled
            );
            for item in found.identities {
                println!(
                    "  {} email={} linked={} last_login={}",
                    item.provider,
                    item.email.as_deref().unwrap_or("-"),
                    item.linked_at.to_rfc3339(),
                    item.last_login_at
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| "-".into())
                );
            }
        }
        Command::UserSetPassword { email, .. } => {
            let password = password.expect("read above");
            let user_id = identity::admin_set_password(&pool, &email, &password)
                .await
                .unwrap_or_else(|error| fail(format!("set-password: {error:?}")));
            println!("password set for {user_id}; all sessions ended");
        }
        Command::UserDisable { email } => {
            let change = penfii_core::admin::operator_set_account_disabled(&pool, &email, true)
                .await
                .unwrap_or_else(|error| fail(format!("user-disable: {error:?}")));
            println!("account {} disabled; all sessions ended", change.user_id);
            print_admin_impact(&change.impacts);
        }
        Command::UserEnable { email } => {
            let change = penfii_core::admin::operator_set_account_disabled(&pool, &email, false)
                .await
                .unwrap_or_else(|error| fail(format!("user-enable: {error:?}")));
            println!("account {} enabled", change.user_id);
        }
        Command::SearchStatus => {
            let engine = search_engine(&pool);
            println!(
                "prefix={} owner={}",
                engine.indexer.client.config.prefix,
                engine.indexer.owner_id().await.unwrap_or_default()
            );
            if let Err(error) = engine.indexer.check_prefix().await {
                println!("prefix check: {error:?}");
            }
            for state in engine.indexer.states().await.unwrap_or_default() {
                println!(
                    "{:8} phase={} live={} building={} error={}",
                    state.kind,
                    state.phase,
                    state.live_index.as_deref().unwrap_or("-"),
                    state.building_index.as_deref().unwrap_or("-"),
                    state.last_error_code.as_deref().unwrap_or("-")
                );
            }
            let dead: Vec<(String, i64)> = sqlx::query_as(
                "SELECT consumer, count(*) FROM penfii.consumer_receipts WHERE state='DEAD' GROUP BY consumer",
            )
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
            for (consumer, count) in dead {
                println!("dead deliveries: {consumer} {count}");
            }
        }
        Command::SearchReindex { kind } => {
            let engine = search_engine(&pool);
            let kinds: Vec<&str> = match kind.as_deref() {
                Some(one) => vec![one],
                None => penfii_core::search::mapping::KINDS.to_vec(),
            };
            for kind in kinds {
                if let Err(error) = engine.indexer.request_reindex(kind).await {
                    fail(format!("{kind}: {error:?}"));
                }
            }
            let done = engine
                .indexer
                .backfill_all()
                .await
                .unwrap_or_else(|error| fail(format!("reindex failed: {error:?}")));
            println!("rebuilt: {}", done.join(", "));
        }
        Command::DocsCollabStatus { page } => {
            let rows = sqlx::query(
                "SELECT s.page_id, p.title, s.epoch, s.last_seq, s.materialized_seq, s.snapshot_seq,
                        s.invalid_since, p.revision_no, p.materialized_at, p.deleted_at
                 FROM penfii.doc_collab_state s JOIN penfii.doc_pages p ON p.id = s.page_id
                 WHERE ($1::uuid IS NULL OR s.page_id = $1)
                 ORDER BY (s.last_seq - s.materialized_seq) DESC, s.updated_at DESC LIMIT 200",
            )
            .bind(page)
            .fetch_all(&pool)
            .await
            .unwrap_or_else(|error| fail(format!("status: {error}")));
            use sqlx::Row;
            for row in &rows {
                let line = (|| -> Result<String, sqlx::Error> {
                    let lag: i64 = row.try_get::<i64, _>("last_seq")?
                        - row.try_get::<i64, _>("materialized_seq")?;
                    Ok(format!(
                        "{} epoch={} lag={} revision={} snapshot_seq={} invalid_since={} deleted={} title={:?}",
                        row.try_get::<uuid::Uuid, _>("page_id")?,
                        row.try_get::<uuid::Uuid, _>("epoch")?,
                        lag,
                        row.try_get::<i64, _>("revision_no")?,
                        row.try_get::<i64, _>("snapshot_seq")?,
                        row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("invalid_since")?
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_else(|| "-".into()),
                        row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")?
                            .is_some(),
                        row.try_get::<String, _>("title")?,
                    ))
                })()
                .unwrap_or_else(|error| fail(format!("status: {error}")));
                println!("{line}");
            }
            println!("pages with a lineage: {}", rows.len());
        }
        Command::DocCollabReset { page } => {
            let epoch = penfii_core::docs::collab::store::reset(&pool, page)
                .await
                .unwrap_or_else(|error| fail(format!("reset failed: {error:?}")));
            println!("new lineage for {page}: epoch {epoch}");
        }
        Command::EventsDead => {
            let dead = penfii_core::events::dead_receipts(&pool, 200)
                .await
                .unwrap_or_else(|error| fail(format!("events: {error:?}")));
            println!(
                "attempts: claims so far for a consumer outside the database (a claim after an expired lease counts); handler failures for a consumer that only writes rows"
            );
            for d in &dead {
                println!(
                    "{} consumer={} kind={} attempts={} error={}",
                    d.event_id,
                    d.consumer,
                    d.event_kind,
                    d.attempts,
                    d.last_error_code.as_deref().unwrap_or("-")
                );
            }
            println!("dead deliveries: {}", dead.len());
        }
        Command::EventsRetry {
            consumer,
            event,
            all,
        } => {
            if consumer.is_none() && event.is_none() && !all {
                fail("name --consumer, --event or --all");
            }
            let count = penfii_core::events::requeue_dead(&pool, consumer.as_deref(), event)
                .await
                .unwrap_or_else(|error| fail(format!("requeue failed: {error:?}")));
            println!("requeued: {count}");
        }
    }
}

fn print_account(created: Result<identity::BootstrapOutput, penfii_core::CoreError>) {
    let output = created.unwrap_or_else(|error| fail(format!("account command failed: {error:?}")));
    println!("user_id {}", output.user_id);
    println!("workspace_id {}", output.workspace_id);
}

/// What disabling an account left without an admin or manager, if anything. Disabling is never refused
/// for it; a workspace owner or admin names a new admin for such a project, or a manager for such a
/// private channel, in the web app (workspace administration), or the account is enabled again.
fn print_admin_impact(impacts: &[penfii_core::admin::WorkspaceImpact]) {
    for impact in impacts {
        for project in &impact.projects {
            let what = match project.kind.as_str() {
                "NO_ADMIN" => "now has no admin who can act",
                _ => "has lost its last direct admin (an admin through a team or group remains)",
            };
            println!(
                "warning: project {:?} ({}) in workspace {} {what}",
                project.name, project.project_id, impact.workspace_id
            );
        }
        for channel in &impact.channels {
            println!(
                "warning: private channel {:?} ({}) in workspace {} now has no manager who can act",
                channel.name, channel.channel_id, impact.workspace_id
            );
        }
    }
}

/// What a join did: whether the person is new or came back, and who is in which role.
fn print_join(joined: &OperatorJoin, role: &str) {
    let result = match joined.outcome {
        JoinOutcome::Joined => "joined",
        JoinOutcome::Rejoined => "rejoined",
        JoinOutcome::AlreadyMember { .. } => "already_member",
    };
    println!("result {result}");
    println!("user_id {}", joined.user_id);
    println!("workspace_id {}", joined.workspace_id);
    println!("role {role}");
}

/// An error in the operator's words.
fn explain(error: &penfii_core::CoreError) -> String {
    use penfii_core::CoreError;
    match error {
        CoreError::NotFound => {
            "not found (check the workspace id with workspace-list; the account must still be a member)".into()
        }
        CoreError::Validation(message) => message.clone(),
        CoreError::Conflict { code, message, .. } => match *code {
            "account_exists" => {
                "an account with this e-mail already exists; add it with workspace-add-member"
                    .into()
            }
            "account_disabled" => message.clone(),
            "last_owner" => {
                "the workspace would be left without an owner; make someone else an owner first"
                    .into()
            }
            _ => message.clone(),
        },
        CoreError::Rule { message, .. } => message.clone(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut all = vec!["admin-cli"];
        all.extend_from_slice(args);
        Cli::try_parse_from(all)
    }

    const WS: &str = "00000000-0000-0000-0000-000000000001";

    #[test]
    fn create_user_names_an_existing_workspace_and_defaults_to_member() {
        let cli = parse(&[
            "create-user",
            "--email",
            "a@b.c",
            "--password",
            "x",
            "--display-name",
            "A",
            "--workspace",
            WS,
        ])
        .unwrap();
        match cli.command {
            Command::CreateUser {
                role, workspace, ..
            } => {
                assert_eq!(role, "MEMBER");
                assert_eq!(workspace.to_string(), WS);
            }
            _ => panic!("not create-user"),
        }
        // The workspace is required, and the options that made a workspace are gone.
        assert!(
            parse(&[
                "create-user",
                "--email",
                "a@b.c",
                "--password",
                "x",
                "--display-name",
                "A"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "create-user",
                "--email",
                "a@b.c",
                "--password",
                "x",
                "--display-name",
                "A",
                "--workspace",
                WS,
                "--workspace-name",
                "W",
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "create-user",
                "--email",
                "a@b.c",
                "--password",
                "x",
                "--display-name",
                "A",
                "--workspace",
                WS,
                "--time-zone",
                "Asia/Seoul",
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "create-user",
                "--email",
                "a@b.c",
                "--password",
                "x",
                "--display-name",
                "A",
                "--workspace",
                "not-an-id",
            ])
            .is_err()
        );
    }

    #[test]
    fn bootstrap_still_makes_the_first_workspace() {
        assert!(
            parse(&[
                "bootstrap",
                "--email",
                "a@b.c",
                "--password",
                "x",
                "--display-name",
                "A",
                "--workspace-name",
                "W",
                "--time-zone",
                "Asia/Seoul",
            ])
            .is_ok()
        );
    }

    #[test]
    fn the_workspace_commands_take_an_id_and_a_role() {
        let add = parse(&[
            "workspace-add-member",
            "--email",
            "a@b.c",
            "--workspace",
            WS,
            "--role",
            "ADMIN",
        ])
        .unwrap();
        assert!(
            matches!(add.command, Command::WorkspaceAddMember { ref role, .. } if role == "ADMIN")
        );
        let add = parse(&[
            "workspace-add-member",
            "--email",
            "a@b.c",
            "--workspace",
            WS,
        ])
        .unwrap();
        assert!(
            matches!(add.command, Command::WorkspaceAddMember { ref role, .. } if role == "MEMBER")
        );
        assert!(parse(&["workspace-add-member", "--email", "a@b.c"]).is_err());
        // Changing a role never guesses one.
        assert!(parse(&["workspace-set-role", "--email", "a@b.c", "--workspace", WS]).is_err());
        assert!(
            parse(&[
                "workspace-set-role",
                "--email",
                "a@b.c",
                "--workspace",
                WS,
                "--role",
                "OWNER"
            ])
            .is_ok()
        );
        assert!(parse(&["workspace-list"]).is_ok());
    }

    #[test]
    fn the_password_comes_from_the_flag_or_standard_input_but_never_both() {
        for command in [
            vec![
                "bootstrap",
                "--email",
                "a@b.c",
                "--display-name",
                "A",
                "--workspace-name",
                "W",
                "--time-zone",
                "UTC",
            ],
            vec![
                "create-user",
                "--email",
                "a@b.c",
                "--display-name",
                "A",
                "--workspace",
                WS,
            ],
            vec!["user-set-password", "--email", "a@b.c"],
        ] {
            let with = |extra: &[&str]| {
                let mut all = command.clone();
                all.extend_from_slice(extra);
                parse(&all)
            };
            assert!(with(&["--password-stdin"]).is_ok(), "{command:?}");
            assert!(with(&["--password", "x"]).is_ok(), "{command:?}");
            // Without either, the password is asked for at run time (prompt), not by clap.
            assert!(with(&[]).is_ok(), "{command:?}");
            assert!(
                with(&["--password", "x", "--password-stdin"]).is_err(),
                "{command:?}"
            );
        }
    }
}
