#!/usr/bin/env bash
# Makes the accounts tests/security-flows.spec.ts signs in as (two-step verification, e-mail change, removal and rejoin,
# password change), in an existing Workspace. Local and CI use only: the password is the public end-to-end test password.
#
#   E2E_WORKSPACE_ID=<id> apps/e2e/seed-security.sh
#
# Another Compose project: COMPOSE="docker compose -p <name> --env-file <file>" apps/e2e/seed-security.sh
# Safe to run again: an account that already exists is left as it is.
set -euo pipefail
cd "$(dirname "$0")/../../../docker"

: "${E2E_WORKSPACE_ID:?set E2E_WORKSPACE_ID to the Workspace the accounts join (admin-cli workspace-list)}"
PASSWORD="${E2E_PASSWORD:-E2E-Pass-2026-local-only}"
COMPOSE="${COMPOSE:-docker compose}"
admin() { $COMPOSE run --rm -T admin-cli "$@"; }

# e2e-mfa: turns MFA on and off, and is removed from the Workspace and invited back. e2e-chg: changes its e-mail address
# and back. e2e-pwd: changes its password and back. Each is left as it was found when the test passes.
for spec in "e2e-mfa@example.test|E2E e2e-mfa" "e2e-chg@example.test|E2E e2e-chg" "e2e-pwd@example.test|E2E e2e-pwd"; do
  IFS='|' read -r email name <<<"$spec"
  printf '%s' "$PASSWORD" | admin create-user --email "$email" --password-stdin --display-name "$name" --workspace "$E2E_WORKSPACE_ID" --role MEMBER \
    || echo "$email: not created (it may already exist)"
done
