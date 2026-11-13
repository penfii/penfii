#!/usr/bin/env bash
# Makes the test accounts the end-to-end tests sign in as. Local and CI use only: the password is public.
#
#   apps/e2e/seed.sh                     empty stack: bootstraps the first account and a Workspace "E2E"
#   E2E_WORKSPACE_ID=<id> apps/e2e/seed.sh   stack that already has users: adds the accounts to that Workspace
#
# Safe to run again: an account that already exists is left as it is.
set -euo pipefail
cd "$(dirname "$0")/../../../docker"

PASSWORD="${E2E_PASSWORD:-E2E-Pass-2026-local-only}"
EMAIL_A="${E2E_EMAIL_A:-e2e-a@example.test}"
EMAIL_B="${E2E_EMAIL_B:-e2e-b@example.test}"
NAME_A="${E2E_NAME_A:-E2E Alpha}"
NAME_B="${E2E_NAME_B:-E2E Bravo}"
admin() { docker compose run --rm -T admin-cli "$@"; }

if [ -z "${E2E_WORKSPACE_ID:-}" ]; then
  printf '%s' "$PASSWORD" | admin bootstrap --email "$EMAIL_A" --password-stdin --display-name "$NAME_A" --workspace-name E2E \
    || { echo "bootstrap refused: the stack already has users. Set E2E_WORKSPACE_ID to add the test accounts to a Workspace." >&2; exit 1; }
  echo "Bootstrapped $EMAIL_A. Find the Workspace id in the web app (the address of its pages) and run again with E2E_WORKSPACE_ID to add $EMAIL_B."
  exit 0
fi

for pair in "$EMAIL_A|$NAME_A" "$EMAIL_B|$NAME_B"; do
  email="${pair%%|*}"; name="${pair#*|}"
  printf '%s' "$PASSWORD" | admin create-user --email "$email" --password-stdin --display-name "$name" --workspace "$E2E_WORKSPACE_ID" --role OWNER \
    || echo "$email: not created (it may already exist)"
done
