#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
if [[ ${1:-} == --external ]]; then
  shift
  : "${E2E_BASE_URL:?set E2E_BASE_URL to the test stack to use}"
  E2E_RUNNER=host exec npm test -w @penfii/e2e -- "$@"
fi

repo="$(cd .. && pwd -P)"
docker_local=(env -i "PATH=$PATH" "HOME=$HOME" docker --host unix:///var/run/docker.sock)
remove_stack() {
  if "${docker_local[@]}" container inspect "$project-playwright" >/dev/null 2>&1; then
    "${docker_local[@]}" rm -f "$project-playwright" >/dev/null || return
  fi
  "${compose[@]}" down --volumes --remove-orphans || return
  local service
  for service in core-api web opensearch; do
    if "${docker_local[@]}" image inspect "$project-$service:local" >/dev/null 2>&1; then
      "${docker_local[@]}" image rm "$project-$service:local" >/dev/null || return
    fi
  done
  rm -f "$settings"
}
set_compose_command() {
  compose=("${docker_local[@]}" compose --project-name "$project" --env-file "$settings"
    -f "$repo/docker/compose.yaml" -f "$repo/docker/compose.e2e.yaml")
}
if [[ ${1:-} == --clean ]]; then
  project="${2:-}"
  [[ "$project" =~ ^penfii-e2e-[a-f0-9]{16}$ && $# == 2 ]] || {
    echo 'Usage: run.sh --clean penfii-e2e-<16 hex digits>' >&2
    exit 2
  }
  settings="$PWD/apps/e2e/runs/$project/compose.env"
  if [[ ! -f "$settings" ]] || ! grep -Fxq "E2E_PROJECT=$project" "$settings"; then
    echo "Missing saved settings for $project" >&2
    exit 2
  fi
  set_compose_command
  remove_stack
  exit
fi

mkdir -p "$PWD/apps/e2e/runs"
case "${E2E_RUNNER:-host}" in
  host|docker) ;;
  *) echo 'E2E_RUNNER must be host or docker' >&2; exit 2 ;;
esac
project="penfii-e2e-$(openssl rand -hex 8)"
artifacts="$PWD/apps/e2e/runs/$project"
mkdir -m 700 "$artifacts"
settings="$artifacts/compose.env"
install -m 600 /dev/null "$settings"
sed '/=CHANGE_ME$/d' "$repo/docker/.env.example" > "$settings"
printf '\nE2E_PROJECT=%s\n' "$project" >> "$settings"
for variable in POSTGRES_PASSWORD PENFII_DB_OWNER_PASSWORD PENFII_DB_RUNTIME_PASSWORD \
  PENFII_S3_ACCESS_KEY PENFII_S3_SECRET_KEY PENFII_S3_APP_ACCESS_KEY PENFII_S3_APP_SECRET_KEY; do
  printf '%s=%s\n' "$variable" "$(openssl rand -hex 16)" >> "$settings"
done

# Both Docker's endpoint and Compose's interpolation environment are explicit.
set_compose_command
cleanup() {
  local status=$?
  trap - EXIT
  if ((status != 0)); then
    "${compose[@]}" ps -a > "$artifacts/compose-ps.txt" 2>&1 || true
    "${compose[@]}" logs --no-color > "$artifacts/compose.log" 2>&1 || true
  fi
  if [[ "${E2E_KEEP_STACK:-0}" == 1 ]]; then
    echo "E2E stack retained: $project (settings and logs: $artifacts)"
    exit "$status"
  fi
  remove_stack || status=1
  echo "E2E artifacts: $artifacts"
  exit "$status"
}
trap cleanup EXIT
test_pid=''
interrupt() {
  if [[ -n "$test_pid" ]]; then
    kill -TERM -- "-$test_pid" 2>/dev/null || true
    wait "$test_pid" 2>/dev/null || true
  fi
  exit "$1"
}
trap 'interrupt 130' INT
trap 'interrupt 143' TERM

echo "E2E project: $project (artifacts: $artifacts)"
"${compose[@]}" build core-api web opensearch
# Let Docker allocate two loopback ports and a subnet atomically. Caddy can start before its upstream.
"${compose[@]}" up -d --no-deps --no-build web mailpit
web="$("${compose[@]}" port web 8080)"
mail="$("${compose[@]}" port mailpit 8025)"
subnet="$("${docker_local[@]}" network inspect --format '{{(index .IPAM.Config 0).Subnet}}' "${project}_frontend")"
[[ "$web" == 127.0.0.1:* && "$mail" == 127.0.0.1:* && -n "$subnet" ]]
printf 'project=%s\nweb=%s\nmail=%s\nfrontend=%s\n' "$project" "$web" "$mail" "$subnet" | tee "$artifacts/stack.txt"
app_origin="http://$web"
mail_origin="http://$mail"
if [[ "${E2E_RUNNER:-host}" == docker ]]; then
  # The browser container shares web's network namespace, including its loopback.
  app_origin=http://127.0.0.1:8080
  mail_origin=http://mailpit:8025
fi
printf '\nPENFII_PUBLIC_URL=%s\nPENFII_TRUSTED_PROXIES=%s\nPENFII_VAPID_SUBJECT=mailto:e2e@example.test\n' "$app_origin" "$subnet" >> "$settings"
"${compose[@]}" run --rm -T --no-deps admin-cli vapid-keys >> "$settings"
"${compose[@]}" up -d --no-build --wait --wait-timeout 300

seed=(env -i "PATH=$PATH" "HOME=$HOME")
"${seed[@]}" apps/e2e/seed.sh --fresh "${compose[@]}" | tee "$artifacts/seed.txt"
workspace="$(awk '$1 == "workspace_id" {print $2; exit}' "$artifacts/seed.txt")"
[[ "$workspace" =~ ^[a-f0-9-]{36}$ ]]
for family in security multi-user; do
  "${seed[@]}" "E2E_WORKSPACE_ID=$workspace" "apps/e2e/seed-$family.sh" --fresh "${compose[@]}" |
    tee -a "$artifacts/seed.txt"
done
"${compose[@]}" exec -T postgres psql -X -U penfii -d penfii -v ON_ERROR_STOP=1 -Atc '
  SELECT (SELECT count(*) FROM penfii.workspaces) AS workspaces,
         (SELECT count(*) FROM penfii.users) AS users,
         (SELECT count(*) FROM penfii.projects) AS projects,
         (SELECT count(*) FROM penfii.files) AS files;
  SELECT role, count(*) FROM penfii.workspace_members GROUP BY role ORDER BY role;
' > "$artifacts/seed-state.txt"

# Account overrides and endpoints from a previous (external) run must not reach these fixtures.
test_env=("E2E_RUNNER=${E2E_RUNNER:-host}" "E2E_BASE_URL=$app_origin" "E2E_MAILPIT=$mail_origin" "E2E_EMAIL_MEMBER=mu-member@example.test"
  "E2E_BROWSERS=${E2E_BROWSERS:-chromium}" "E2E_MULTI_USER=${E2E_MULTI_USER:-}"
  "E2E_MU_ENGINES=${E2E_MU_ENGINES:-}" "E2E_MU_LOG=$artifacts/multi-user.jsonl")
if [[ "${E2E_RUNNER:-host}" == docker ]]; then
  docker_env=()
  for setting in "${test_env[@]}"; do docker_env+=(-e "$setting"); done
  web_container="$("${compose[@]}" ps -q web)"
  runner=("${docker_local[@]}" run --rm --init --name "$project-playwright" --network "container:$web_container"
    --shm-size 1g --user "$(id -u):$(id -g)"
    -v "$PWD:$PWD" -w "$PWD" -e HOME=/tmp "${docker_env[@]}" mcr.microsoft.com/playwright:v1.65.0-noble)
else
  runner=(env -i "PATH=$PATH" "HOME=$HOME" "${test_env[@]}")
  if [[ -v PLAYWRIGHT_BROWSERS_PATH ]]; then
    runner+=("PLAYWRIGHT_BROWSERS_PATH=$PLAYWRIGHT_BROWSERS_PATH")
  fi
fi
# The child shell expands the log path and arguments; setsid lets us cancel that job alone.
# shellcheck disable=SC2016
setsid bash -o pipefail -c 'log=$1; shift; "$@" | tee "$log"' e2e "$artifacts/playwright.log" \
  "${runner[@]}" npm test -w @penfii/e2e -- --output "$artifacts/playwright" "$@" &
test_pid=$!
wait "$test_pid"
