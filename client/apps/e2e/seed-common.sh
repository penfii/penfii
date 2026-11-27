#!/usr/bin/env bash
# Shared by the seed commands. The launcher passes Compose as separate arguments.
setup_seed() {
  if [[ ${1:-} == --fresh ]]; then
    shift
    fresh=1
    compose_command=("$@" run --rm -T --no-deps)
  else
    fresh=0
    if [[ ${E2E_EXTERNAL:-0} != 1 ]]; then
      echo "External seeding requires E2E_EXTERNAL=1 and an explicit test Compose project" >&2
      exit 2
    fi
    : "${COMPOSE:?set COMPOSE to the disposable test project}"
    read -r -a compose_command <<< "$COMPOSE"
    compose_command+=(run --rm -T)
  fi
}

admin() { "${compose_command[@]}" admin-cli "$@"; }

add_fixture() {
  local email="$1" name="$2" workspace="$3" role="$4" output
  if output="$(printf '%s' "$PASSWORD" | admin create-user --email "$email" --password-stdin \
    --display-name "$name" --workspace "$workspace" --role "$role" 2>&1)"; then
    printf '%s\n' "$output"
  elif [[ "$fresh" == 0 && "$output" == *'create-user: an account with this e-mail already exists;'* ]]; then
    printf '%s: already exists; verify its workspace, role and password before testing\n' "$email" >&2
  else
    printf '%s\n' "$output" >&2
    return 1
  fi
}
