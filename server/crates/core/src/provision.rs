//! Database roles: `admin-cli db-provision`.
//!
//! Penfii uses two database roles. The **owner** owns the schema `penfii` and its objects and is the
//! only one that changes them: migrations run with its credential (`PENFII_MIGRATION_DATABASE_URL`,
//! `admin-cli migrate`). The **runtime** role is what core-api and the other `admin-cli` commands
//! connect as (`DATABASE_URL`): it can read and write rows and nothing else (no DDL, no
//! `CREATE ROLE`/`CREATEDB`, not a superuser, not an owner). Role creation and privilege changes need
//! a privileged credential (`PENFII_PROVISION_DATABASE_URL`, normally the bootstrap superuser of the
//! PostgreSQL server); they are not part of the schema migrations and they are not in a `pg_dump` of
//! the database, so a restored database needs this run again (it is safe to repeat).
//!
//! An installation made before the split keeps what it has: its objects stay owned by the role that
//! made them, and that role (which is the owner role then; it may be a superuser) stays the migration
//! credential. An owner role that already exists is never altered here.

use sqlx::{Connection, PgConnection};

use crate::migrate::SCHEMA;

/// What to provision.
pub struct Provision {
    /// The owner of the schema and the migration credential.
    pub owner_role: String,
    /// Set only when the owner role is created by this run.
    pub owner_password: Option<String>,
    /// The role core-api connects as.
    pub runtime_role: String,
    /// Set (or changed) on every run when given.
    pub runtime_password: Option<String>,
}

/// What a run did, for the operator.
#[derive(Debug, Default)]
pub struct Provisioned {
    pub database: String,
    pub owner_created: bool,
    pub runtime_created: bool,
    pub owner_kept_as_is: bool,
}

/// Role names are plain identifiers: they end up in `format('%I')` on the server, and this keeps a
/// name from being anything surprising on the way there.
fn valid_role_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && name.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with("pg_")
}

/// Builds a statement on the server with `format`, so identifiers and passwords are quoted by
/// PostgreSQL itself, then runs it.
async fn run(
    conn: &mut PgConnection,
    template: &str,
    arguments: &[&str],
) -> Result<(), sqlx::Error> {
    let mut query = sqlx::query_scalar::<_, String>(template);
    for argument in arguments {
        query = query.bind(*argument);
    }
    let statement = query.fetch_one(&mut *conn).await?;
    sqlx::query(&statement).execute(&mut *conn).await?;
    Ok(())
}

async fn role_exists(conn: &mut PgConnection, name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
        .bind(name)
        .fetch_one(&mut *conn)
        .await
}

/// Creates or updates the roles and privileges in the database `conn` is connected to. `conn` must
/// be a privileged credential (it creates roles and grants on objects it does not own).
pub async fn provision(conn: &mut PgConnection, plan: &Provision) -> Result<Provisioned, String> {
    for name in [&plan.owner_role, &plan.runtime_role] {
        if !valid_role_name(name) {
            return Err(format!(
                "{name:?} is not a usable role name (lower-case letters, digits and _, starting with a letter or _, at most 63 characters)"
            ));
        }
    }
    if plan.owner_role == plan.runtime_role {
        return Err("the owner role and the runtime role must be different roles".into());
    }
    apply(conn, plan).await.map_err(|error| error.to_string())
}

async fn apply(conn: &mut PgConnection, plan: &Provision) -> Result<Provisioned, sqlx::Error> {
    let mut done = Provisioned {
        database: sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *conn)
            .await?,
        ..Default::default()
    };
    let (owner, runtime) = (plan.owner_role.as_str(), plan.runtime_role.as_str());
    let database = done.database.clone();

    // One transaction: a failed run leaves the roles as they were.
    let mut tx = conn.begin().await?;

    // The owner: created without any special right when it does not exist; an existing role (an
    // installation from before the split owns its objects with the role it has) is left alone.
    if role_exists(&mut tx, owner).await? {
        done.owner_kept_as_is = true;
    } else {
        let template = if plan.owner_password.is_some() {
            "SELECT format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION PASSWORD %L', $1::text, $2::text)"
        } else {
            "SELECT format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION', $1::text)"
        };
        let mut arguments = vec![owner];
        if let Some(password) = &plan.owner_password {
            arguments.push(password);
        }
        run(&mut tx, template, &arguments).await?;
        done.owner_created = true;
    }
    // CREATE on the database is what `CREATE SCHEMA` needs, and the migration runner issues it.
    run(
        &mut tx,
        "SELECT format('GRANT CONNECT, CREATE ON DATABASE %I TO %I', $1::text, $2::text)",
        &[&database, owner],
    )
    .await?;
    // The schema belongs to the owner from the start, so the default privileges below are in place
    // before the first table exists.
    run(
        &mut tx,
        "SELECT format('CREATE SCHEMA IF NOT EXISTS %I AUTHORIZATION %I', $1::text, $2::text)",
        &[SCHEMA, owner],
    )
    .await?;

    // The runtime role: no special right, and the timeouts of its sessions (the values are the
    // baseline in docs/operations.md; a statement that needs longer sets its own with SET LOCAL).
    if role_exists(&mut tx, runtime).await? {
        let template = if plan.runtime_password.is_some() {
            "SELECT format('ALTER ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION PASSWORD %L', $1::text, $2::text)"
        } else {
            "SELECT format('ALTER ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION', $1::text)"
        };
        let mut arguments = vec![runtime];
        if let Some(password) = &plan.runtime_password {
            arguments.push(password);
        }
        run(&mut tx, template, &arguments).await?;
    } else {
        let template = if plan.runtime_password.is_some() {
            "SELECT format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION PASSWORD %L', $1::text, $2::text)"
        } else {
            "SELECT format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION', $1::text)"
        };
        let mut arguments = vec![runtime];
        if let Some(password) = &plan.runtime_password {
            arguments.push(password);
        }
        run(&mut tx, template, &arguments).await?;
        done.runtime_created = true;
    }
    run(
        &mut tx,
        "SELECT format('GRANT CONNECT ON DATABASE %I TO %I', $1::text, $2::text)",
        &[&database, runtime],
    )
    .await?;
    for (setting, value) in RUNTIME_SETTINGS {
        run(
            &mut tx,
            &format!("SELECT format('ALTER ROLE %I SET {setting} = %L', $1::text, $2::text)"),
            &[runtime, value],
        )
        .await?;
    }

    // What the runtime role may do in the schema: use it, and read and write rows. The default
    // privileges (of the owner's future objects) keep that true for tables a later migration makes.
    for template in [
        "SELECT format('GRANT USAGE ON SCHEMA %I TO %I', $1::text, $2::text)",
        "SELECT format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA %I TO %I', $1::text, $2::text)",
    ] {
        run(&mut tx, template, &[SCHEMA, runtime]).await?;
    }
    run(
        &mut tx,
        "SELECT format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA %I GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I', $1::text, $2::text, $3::text)",
        &[owner, SCHEMA, runtime],
    )
    .await?;
    // The migration history is read, never written, by the runtime role. When it exists already (an
    // installation from before the split) the rights it got above are taken back here; the rest of
    // the time `admin-cli migrate` does the same after it ran.
    let history: bool =
        sqlx::query_scalar("SELECT to_regclass('penfii._sqlx_migrations') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if history {
        run(
            &mut tx,
            "SELECT format('REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON %I._sqlx_migrations FROM %I', $1::text, $2::text)",
            &[SCHEMA, runtime],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(done)
}

/// The session settings of the runtime role: `lock_timeout`, `statement_timeout` and
/// `idle_in_transaction_session_timeout` (see docs/operations.md, "Database timeouts").
pub const RUNTIME_SETTINGS: [(&str, &str); 3] = [
    ("lock_timeout", "5s"),
    ("statement_timeout", "30s"),
    ("idle_in_transaction_session_timeout", "60s"),
];

#[cfg(test)]
mod tests {
    use super::valid_role_name;

    #[test]
    fn role_names_are_plain_identifiers() {
        for good in ["penfii", "penfii_app", "_x", "a1"] {
            assert!(valid_role_name(good), "{good}");
        }
        for bad in [
            "",
            "Penfii",
            "1a",
            "a-b",
            "a b",
            "a\"b",
            "pg_admin",
            &"a".repeat(64),
        ] {
            assert!(!valid_role_name(bad), "{bad}");
        }
    }
}
