#!/usr/bin/env bash
# Makes the accounts the multi-user test (tests/multi-user.spec.ts) signs in as, in an existing Workspace. Local and
# CI use only: the password is the public end-to-end test password.
# See docs/development.md for use with an existing disposable Compose project.
set -euo pipefail
seed_dir="$(cd "$(dirname "$0")" && pwd)"
cd "$seed_dir/../../../docker"

: "${E2E_WORKSPACE_ID:?set E2E_WORKSPACE_ID to the Workspace the accounts join (admin-cli workspace-list)}"
PASSWORD="${E2E_PASSWORD:-E2E-Pass-2026-local-only}"
# shellcheck source=client/apps/e2e/seed-common.sh
source "$seed_dir/seed-common.sh"
setup_seed "$@"

# email|display name|workspace role. The Project and channel roles are given by the test itself.
for spec in \
  "mu-owner@example.test|MU Owner|OWNER" \
  "mu-admin@example.test|MU Admin|ADMIN" \
  "mu-member@example.test|MU Member|MEMBER" \
  "mu-member2@example.test|MU Member Two|MEMBER" \
  "mu-outsider@example.test|MU Outsider|MEMBER"; do
  IFS='|' read -r email name role <<<"$spec"
  add_fixture "$email" "$name" "$E2E_WORKSPACE_ID" "$role"
done
