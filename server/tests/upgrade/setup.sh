#!/usr/bin/env bash
# Seeds the database of the FEA-149 upgrade proof (server/tests/integration/upgrade_rejoin.rs, run by hand).
#
# It builds the schema with the admin-cli of the release being upgraded from, makes the people the test
# signs in as (owner@up.test, active@up.test, gone@up.test, gone2@up.test, password upgrade-passphrase-1)
# and writes what a removal in that release left behind (seed.sql). Then, by hand:
#
#   psql "$DATABASE_URL" -X -f snapshot.sql             # before
#   admin-cli migrate                                    # the current build, same DATABASE_URL
#   psql "$DATABASE_URL" -X -f snapshot.sql             # after: the leftovers are gone
#   PENFII_UPGRADE_DATABASE_URL="$DATABASE_URL" \
#     cargo test --test integration upgrade_rejoin -- --ignored --nocapture
#
# Usage: OLD_ADMIN_CLI=<admin-cli built from the previous release> DATABASE_URL=<a new, empty database> \
#          server/tests/upgrade/setup.sh
# Use a throwaway PostgreSQL server, never a real installation. Needs psql.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
: "${OLD_ADMIN_CLI:?set OLD_ADMIN_CLI to an admin-cli built from the previous release}"
: "${DATABASE_URL:?set DATABASE_URL to a new, empty database}"
export DATABASE_URL
export PENFII_MIGRATION_DATABASE_URL="${PENFII_MIGRATION_DATABASE_URL:-$DATABASE_URL}"
PASSWORD=upgrade-passphrase-1

db() { psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -X -At "$@"; }

"$OLD_ADMIN_CLI" migrate
printf '%s' "$PASSWORD" | "$OLD_ADMIN_CLI" bootstrap --email owner@up.test --display-name Owner \
  --workspace-name W1 --time-zone UTC --password-stdin
w1="$(db -c "SELECT id FROM penfii.workspaces LIMIT 1")"
for name in active gone gone2; do
  printf '%s' "$PASSWORD" | "$OLD_ADMIN_CLI" create-user --workspace "$w1" --email "$name@up.test" \
    --display-name "$name" --password-stdin
done
db < "$here/seed.sql" | tail -1
echo "--- migrations applied by the previous release:"
db -c "SELECT version, description FROM penfii._sqlx_migrations ORDER BY version"
