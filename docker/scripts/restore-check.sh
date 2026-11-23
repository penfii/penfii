#!/usr/bin/env bash
# Backup and restore check of the PostgreSQL database, at the database level: dump the database in the
# custom format (pg_dump -Fc), restore it into a scratch database with pg_restore, and compare the two.
# It does NOT start core-api on the restored copy and touches no file storage, search index, mail or
# push service: the copy of a database is verified, nothing else is restored (see docs/self-hosting.md).
#
# Compared: the migration history (versions and checksums), the number of rows of every table in the
# schema, the number of indexes, constraints, triggers and functions, who owns the tables, and what the
# runtime role may do (it can read and write rows in the copy, and it cannot change the schema).
#
# Run it from docker/ with the stack up (the default talks to the `postgres` service), ideally while
# core-api is stopped so no row changes between the dump and the counts:
#
#   scripts/restore-check.sh
#
# Settings (environment): PG_EXEC (how to run a command in the PostgreSQL container, default
# "docker compose exec -T postgres"), PGSUPERUSER (default penfii), SOURCE_DB (default penfii),
# SCRATCH_DB (default penfii_restore_check), PENFII_DB_RUNTIME_USER (default penfii_app), DUMP_FILE
# (default ./penfii.dump; kept), KEEP_SCRATCH=1 keeps the scratch database.
#
# Safety: nothing is dumped, created or dropped until the settings pass these checks. Database and role
# names are plain lower-case identifiers (letters, digits, _). SCRATCH_DB must differ from SOURCE_DB, must
# not be postgres or a template, and must contain "restore" or "scratch" in its name. An existing database
# of that name is dropped only when an earlier run of this script made it (it carries the comment
# "penfii-restore-check scratch"); any other database of that name stops the script untouched. The dump
# holds every row of the database in clear text: it and the two comparison reports are written with mode 600
# (never over a symbolic link), so keep them private and delete them when you are done.
set -euo pipefail
umask 077

read -r -a EXEC <<< "${PG_EXEC:-docker compose exec -T postgres}"
SUPERUSER="${PGSUPERUSER:-penfii}"
SOURCE_DB="${SOURCE_DB:-penfii}"
SCRATCH_DB="${SCRATCH_DB:-penfii_restore_check}"
RUNTIME_ROLE="${PENFII_DB_RUNTIME_USER:-penfii_app}"
DUMP_FILE="${DUMP_FILE:-./penfii.dump}"

die() { echo "FAILED: $*" >&2; exit 1; }

# Refuse settings that could make the script drop or dump the wrong thing, before touching anything.
ident() { [[ "$2" =~ ^[a-z_][a-z0-9_]{0,62}$ ]] || die "$1 must be a plain lower-case identifier (letters, digits, _; at most 63 characters), got '$2'"; }
ident PGSUPERUSER "$SUPERUSER"
ident SOURCE_DB "$SOURCE_DB"
ident SCRATCH_DB "$SCRATCH_DB"
ident PENFII_DB_RUNTIME_USER "$RUNTIME_ROLE"
[ "$SCRATCH_DB" != "$SOURCE_DB" ] || die "SCRATCH_DB and SOURCE_DB are the same database ('$SOURCE_DB'); nothing was touched"
case "$SCRATCH_DB" in
  postgres|template0|template1) die "SCRATCH_DB '$SCRATCH_DB' is a system database; nothing was touched" ;;
esac
case "$SCRATCH_DB" in
  *restore*|*scratch*) ;;
  *) die "SCRATCH_DB '$SCRATCH_DB' must contain 'restore' or 'scratch' in its name, so a real database is not dropped by a typo; nothing was touched" ;;
esac
[ -n "$DUMP_FILE" ] || die "DUMP_FILE is empty"
for f in "$DUMP_FILE" "$DUMP_FILE.source.txt" "$DUMP_FILE.copy.txt"; do
  [ ! -L "$f" ] || die "$f is a symbolic link; refusing to write through it"
  [ ! -e "$f" ] || [ -f "$f" ] || die "$f exists and is not a regular file"
done
SCRATCH_MARK="penfii-restore-check scratch"

psql_in() { # database, sql: tuples only, unaligned
  "${EXEC[@]}" psql -U "$SUPERUSER" -d "$1" -v ON_ERROR_STOP=1 -At -c "$2"
}

# drop_scratch: drops the scratch database only when this script made it.
drop_scratch() {
  local mark
  mark="$(psql_in postgres "SELECT coalesce(shobj_description(oid, 'pg_database'), '') FROM pg_database WHERE datname = '$SCRATCH_DB'")" || return 1
  [ "$mark" = "$SCRATCH_MARK" ] || return 1
  psql_in postgres "DROP DATABASE \"$SCRATCH_DB\"" >/dev/null
}

# What is compared: one text report per database.
report() {
  psql_in "$1" "
    SELECT 'migration ' || version || ' ' || success || ' ' || encode(checksum, 'hex')
      FROM penfii._sqlx_migrations ORDER BY version;
    SELECT 'rows ' || table_name || ' ' ||
           (xpath('/row/c/text()', query_to_xml(format('SELECT count(*) AS c FROM %I.%I', table_schema, table_name), false, true, '')))[1]::text
      FROM information_schema.tables WHERE table_schema = 'penfii' AND table_type = 'BASE TABLE' ORDER BY table_name;
    SELECT 'indexes ' || count(*) FROM pg_indexes WHERE schemaname = 'penfii';
    SELECT 'constraints ' || count(*) FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace WHERE n.nspname = 'penfii';
    SELECT 'triggers ' || count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'penfii' AND NOT t.tgisinternal;
    SELECT 'functions ' || count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'penfii';
    SELECT 'owners ' || owner || ' ' || n FROM (
      SELECT pg_get_userbyid(c.relowner) AS owner, count(*) AS n FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'penfii' AND c.relkind = 'r' GROUP BY 1) t ORDER BY 1;
  "
}

echo "0. check the settings: source $SOURCE_DB, scratch $SCRATCH_DB"
[ "$(psql_in postgres "SELECT count(*) FROM pg_database WHERE datname = '$SOURCE_DB'")" = "1" ] \
  || die "the source database '$SOURCE_DB' does not exist on this server; nothing was touched"
if [ "$(psql_in postgres "SELECT count(*) FROM pg_database WHERE datname = '$SCRATCH_DB'")" = "1" ]; then
  # An earlier run of this script left it (KEEP_SCRATCH=1, or a crash): that one may be replaced.
  drop_scratch || die "a database named '$SCRATCH_DB' exists and was not made by this script (no '$SCRATCH_MARK' comment); not dropping it. Choose another SCRATCH_DB, or drop it yourself if you are sure"
fi

echo "1. dump $SOURCE_DB -> $DUMP_FILE (custom format, mode 600)"
rm -f "$DUMP_FILE"
"${EXEC[@]}" pg_dump -U "$SUPERUSER" -Fc "$SOURCE_DB" > "$DUMP_FILE"
chmod 600 "$DUMP_FILE"
[ -s "$DUMP_FILE" ] || { echo "the dump is empty" >&2; exit 1; }

echo "2. restore into the scratch database $SCRATCH_DB"
psql_in postgres "CREATE DATABASE \"$SCRATCH_DB\"" >/dev/null
psql_in postgres "COMMENT ON DATABASE \"$SCRATCH_DB\" IS '$SCRATCH_MARK'" >/dev/null
cleanup() {
  if [ "${KEEP_SCRATCH:-0}" != "1" ]; then
    drop_scratch || true
  fi
}
trap cleanup EXIT
# The roles the dump refers to (owner, runtime) must exist in this server: they are not in a dump.
"${EXEC[@]}" pg_restore -U "$SUPERUSER" -d "$SCRATCH_DB" --exit-on-error < "$DUMP_FILE"

echo "3. compare the source and the copy"
report "$SOURCE_DB" > "$DUMP_FILE.source.txt"
report "$SCRATCH_DB" > "$DUMP_FILE.copy.txt"
chmod 600 "$DUMP_FILE.source.txt" "$DUMP_FILE.copy.txt"
if ! diff -u "$DUMP_FILE.source.txt" "$DUMP_FILE.copy.txt"; then
  echo "FAILED: the copy differs from the source (rows written during the check? stop core-api and run it again)" >&2
  exit 1
fi
migrations="$(grep -c '^migration ' "$DUMP_FILE.copy.txt")"
tables="$(grep -c '^rows ' "$DUMP_FILE.copy.txt")"
echo "   $migrations migrations with the same checksums, $tables tables with the same row counts"

echo "4. the runtime role on the copy"
# It is a role of the server, so it exists here when it existed at the source. Without it (a server
# that was set up fresh) run `admin-cli db-provision` first, then this check again.
if [ "$(psql_in postgres "SELECT count(*) FROM pg_roles WHERE rolname = '$RUNTIME_ROLE'")" = "1" ]; then
  as_runtime() { psql_in "$SCRATCH_DB" "SET ROLE \"$RUNTIME_ROLE\"; $1"; }
  as_runtime "SELECT count(*) FROM penfii.users" >/dev/null
  as_runtime "SELECT count(*) FROM penfii._sqlx_migrations" >/dev/null
  if as_runtime "CREATE TABLE penfii.restore_check_must_fail (id int)" >/dev/null 2>&1; then
    echo "FAILED: the runtime role can change the schema of the copy" >&2
    exit 1
  fi
  if as_runtime "DELETE FROM penfii._sqlx_migrations" >/dev/null 2>&1; then
    echo "FAILED: the runtime role can write the migration history of the copy" >&2
    exit 1
  fi
  as_runtime "SELECT count(*) FROM penfii.audit_entries" >/dev/null
  if as_runtime "UPDATE penfii.audit_entries SET outcome_code = outcome_code" >/dev/null 2>&1 \
    || as_runtime "DELETE FROM penfii.audit_entries" >/dev/null 2>&1; then
    echo "FAILED: the runtime role can change or delete audit log entries of the copy" >&2
    exit 1
  fi
  echo "   $RUNTIME_ROLE reads and writes rows, cannot change the schema, the migration history or audit log entries"
else
  echo "   the role $RUNTIME_ROLE does not exist on this server: skipped (run admin-cli db-provision, then again)"
fi
echo "OK: the dump restores and matches the source"
