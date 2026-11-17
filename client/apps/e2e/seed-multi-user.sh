#!/usr/bin/env bash
# Makes the accounts the multi-user test (tests/multi-user.spec.ts) signs in as, in an existing Workspace. Local and
# CI use only: the password is the public end-to-end test password.
#
#   E2E_WORKSPACE_ID=<id> apps/e2e/seed-multi-user.sh
#
# Another Compose project: COMPOSE="docker compose -p <name> --env-file <file>" apps/e2e/seed-multi-user.sh
# Safe to run again: an account that already exists is left as it is.
set -euo pipefail
cd "$(dirname "$0")/../../../docker"

: "${E2E_WORKSPACE_ID:?set E2E_WORKSPACE_ID to the Workspace the accounts join (admin-cli workspace-list)}"
PASSWORD="${E2E_PASSWORD:-E2E-Pass-2026-local-only}"
COMPOSE="${COMPOSE:-docker compose}"
admin() { $COMPOSE run --rm -T admin-cli "$@"; }

# email|display name|workspace role. The Project and channel roles are given by the test itself.
for spec in \
  "mu-owner@example.test|MU Owner|OWNER" \
  "mu-admin@example.test|MU Admin|ADMIN" \
  "mu-member@example.test|MU Member|MEMBER" \
  "mu-member2@example.test|MU Member Two|MEMBER" \
  "mu-outsider@example.test|MU Outsider|MEMBER"; do
  IFS='|' read -r email name role <<<"$spec"
  printf '%s' "$PASSWORD" | admin create-user --email "$email" --password-stdin --display-name "$name" --workspace "$E2E_WORKSPACE_ID" --role "$role" \
    || echo "$email: not created (it may already exist)"
done
