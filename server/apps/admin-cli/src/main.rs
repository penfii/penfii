use clap::{Parser, Subcommand};
use penfii_core::identity::{self, BootstrapInput};
use penfii_core::{apply_migrations, connect};
use sqlx::PgPool;

#[derive(Parser)]
#[command(name = "admin-cli")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the first user and an owned workspace. Refuses when any user exists.
    Bootstrap {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        workspace_name: String,
        #[arg(long)]
        time_zone: String,
    },
    /// Create another user and an owned workspace.
    CreateUser {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        workspace_name: String,
        #[arg(long)]
        time_zone: String,
    },
    /// Show how an account can sign in (password, linked providers). Recovery for a lost provider.
    UserIdentities {
        #[arg(long)]
        email: String,
    },
    /// Set or replace an account's password and end all of its sessions. Writes an audit entry.
    UserSetPassword {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
    },
    /// Disable an account: ends all its sessions and OAuth grants and blocks sign-in. For when no
    /// workspace owner can do it. Logged in the audit log of each of its workspaces.
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

/// Every command works on the database named by `DATABASE_URL`, migrated to this build.
async fn open_database() -> PgPool {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| fail("DATABASE_URL is required"));
    let pool = connect(&database_url)
        .await
        .unwrap_or_else(|error| fail(format!("database: {error}")));
    apply_migrations(&pool)
        .await
        .unwrap_or_else(|error| fail(format!("migrate: {error}")));
    pool
}

fn search_engine(pool: &PgPool) -> penfii_core::search::SearchEngine {
    let Some(config) = penfii_core::search::client::OsConfig::from_env() else {
        fail("PENFII_OPENSEARCH_URL is required");
    };
    let client = penfii_core::search::client::OsClient::new(config)
        .unwrap_or_else(|error| fail(format!("search client: {error}")));
    penfii_core::search::SearchEngine::new(pool.clone(), client)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let pool = open_database().await;
    match cli.command {
        Command::Bootstrap {
            email,
            password,
            display_name,
            workspace_name,
            time_zone,
        } => {
            let input = BootstrapInput {
                email,
                password,
                display_name,
                workspace_name,
                time_zone,
            };
            print_account(identity::bootstrap(&pool, input).await);
        }
        Command::CreateUser {
            email,
            password,
            display_name,
            workspace_name,
            time_zone,
        } => {
            let input = BootstrapInput {
                email,
                password,
                display_name,
                workspace_name,
                time_zone,
            };
            print_account(identity::create_account(&pool, input).await);
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
        Command::UserSetPassword { email, password } => {
            let user_id = identity::admin_set_password(&pool, &email, &password)
                .await
                .unwrap_or_else(|error| fail(format!("set-password: {error:?}")));
            println!("password set for {user_id}; all sessions ended");
        }
        Command::UserDisable { email } => {
            let user_id = penfii_core::admin::operator_set_account_disabled(&pool, &email, true)
                .await
                .unwrap_or_else(|error| fail(format!("user-disable: {error:?}")));
            println!("account {user_id} disabled; all sessions ended");
        }
        Command::UserEnable { email } => {
            let user_id = penfii_core::admin::operator_set_account_disabled(&pool, &email, false)
                .await
                .unwrap_or_else(|error| fail(format!("user-enable: {error:?}")));
            println!("account {user_id} enabled");
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
                let lag: i64 =
                    row.get::<i64, _>("last_seq") - row.get::<i64, _>("materialized_seq");
                println!(
                    "{} epoch={} lag={} revision={} snapshot_seq={} invalid_since={} deleted={} title={:?}",
                    row.get::<uuid::Uuid, _>("page_id"),
                    row.get::<uuid::Uuid, _>("epoch"),
                    lag,
                    row.get::<i64, _>("revision_no"),
                    row.get::<i64, _>("snapshot_seq"),
                    row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("invalid_since")
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| "-".into()),
                    row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")
                        .is_some(),
                    row.get::<String, _>("title"),
                );
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
