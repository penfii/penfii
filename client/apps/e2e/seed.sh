#!/usr/bin/env bash
# Makes the test accounts the end-to-end tests sign in as. Local and CI use only: the password is public.
# See docs/development.md for use with an existing disposable Compose project.
set -euo pipefail
seed_dir="$(cd "$(dirname "$0")" && pwd)"
cd "$seed_dir/../../../docker"

PASSWORD="${E2E_PASSWORD:-E2E-Pass-2026-local-only}"
EMAIL_A="${E2E_EMAIL_A:-e2e-a@example.test}"
EMAIL_B="${E2E_EMAIL_B:-e2e-b@example.test}"
NAME_A="${E2E_NAME_A:-E2E Alpha}"
NAME_B="${E2E_NAME_B:-E2E Bravo}"
# shellcheck source=client/apps/e2e/seed-common.sh
source "$seed_dir/seed-common.sh"
setup_seed "$@"

if [[ -z "${E2E_WORKSPACE_ID:-}" ]]; then
  bootstrap="$(printf '%s' "$PASSWORD" | admin bootstrap --email "$EMAIL_A" --password-stdin \
    --display-name "$NAME_A" --workspace-name E2E --time-zone "${E2E_TIME_ZONE:-UTC}")"
  printf '%s\n' "$bootstrap"
  workspace="$(awk '$1 == "workspace_id" {print $2; exit}' <<< "$bootstrap")"
  [[ "$workspace" =~ ^[a-f0-9-]{36}$ ]]
  add_fixture "$EMAIL_B" "$NAME_B" "$workspace" OWNER
  exit
fi

for pair in "$EMAIL_A|$NAME_A" "$EMAIL_B|$NAME_B"; do
  email="${pair%%|*}"; name="${pair#*|}"
  add_fixture "$email" "$name" "$E2E_WORKSPACE_ID" OWNER
done
