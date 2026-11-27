#!/usr/bin/env bash
# Makes the accounts tests/security-flows.spec.ts signs in as (two-step verification, e-mail change, removal and rejoin,
# password change), in an existing Workspace. Local and CI use only: the password is the public end-to-end test password.
# See docs/development.md for use with an existing disposable Compose project.
set -euo pipefail
seed_dir="$(cd "$(dirname "$0")" && pwd)"
cd "$seed_dir/../../../docker"

: "${E2E_WORKSPACE_ID:?set E2E_WORKSPACE_ID to the Workspace the accounts join (admin-cli workspace-list)}"
PASSWORD="${E2E_PASSWORD:-E2E-Pass-2026-local-only}"
# shellcheck source=client/apps/e2e/seed-common.sh
source "$seed_dir/seed-common.sh"
setup_seed "$@"

# e2e-mfa: turns MFA on and off, and is removed from the Workspace and invited back. e2e-chg: changes its e-mail address
# and back. e2e-pwd: changes its password and back. Each is left as it was found when the test passes.
for spec in "e2e-mfa@example.test|E2E e2e-mfa" "e2e-chg@example.test|E2E e2e-chg" "e2e-pwd@example.test|E2E e2e-pwd"; do
  IFS='|' read -r email name <<<"$spec"
  add_fixture "$email" "$name" "$E2E_WORKSPACE_ID" MEMBER
done
