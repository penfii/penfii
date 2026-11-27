#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

count="${INTEGRATION_SHARD_COUNT:-1}"
index="${INTEGRATION_SHARD_INDEX:-1}"
case "$count:$index" in
  1:1|2:[12]|4:[1234]) ;;
  *) echo "invalid integration shard $index/$count (use 1, 2 or 4 shards, indexed from 1)" >&2; exit 2 ;;
esac

pg="penfii-check-pg-$$"
os="penfii-check-os-$$"
created=()
cleanup() {
  local status=$?
  trap - EXIT
  for container in "${created[@]}"; do
    if ((status != 0)); then docker logs --tail 40 "$container" >&2 || true; fi
    docker rm -f -v "$container" >/dev/null || status=1
  done
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

ready() {
  local service="$1" attempts="$2" interval="$3" i
  shift 3
  for ((i=0; i<attempts; i++)); do
    if "$@" >/dev/null 2>&1; then return; fi
    sleep "$interval"
  done
  echo "$service did not become ready" >&2
  return 1
}

: "${PG_IMAGE:?}" "${OS_IMAGE:?}"
if ((index == 1)); then
  command -v sqlx >/dev/null || { echo "sqlx is not installed: make tools" >&2; exit 1; }
fi
docker image inspect "$OS_IMAGE" >/dev/null 2>&1 || docker build -q -t "$OS_IMAGE" ../docker/opensearch >/dev/null
read -r -a pg_flags <<< "${PG_TEST_FLAGS:-}"
docker create --name "$pg" -e POSTGRES_USER=penfii -e POSTGRES_PASSWORD=penfii -e POSTGRES_DB=penfii \
  -p 127.0.0.1::5432 "$PG_IMAGE" "${pg_flags[@]}" >/dev/null
created+=("$pg")
docker create --name "$os" -e discovery.type=single-node -e DISABLE_SECURITY_PLUGIN=true \
  -e DISABLE_INSTALL_DEMO_CONFIG=true -e OPENSEARCH_JAVA_OPTS="-Xms512m -Xmx512m" \
  -p 127.0.0.1::9200 "$OS_IMAGE" >/dev/null
created+=("$os")
docker start "$pg" "$os" >/dev/null
pg_address="$(docker port "$pg" 5432/tcp)"
os_address="$(docker port "$os" 9200/tcp)"
[[ "$pg_address" == 127.0.0.1:* && "$pg_address" != 127.0.0.1:5432 && "$os_address" == 127.0.0.1:* ]]
ready PostgreSQL 60 1 docker exec "$pg" pg_isready -h 127.0.0.1 -U penfii
ready OpenSearch 90 2 curl -fsS --max-time 3 "http://$os_address/_cluster/health?wait_for_status=yellow&timeout=2s"
export DATABASE_URL="postgres://penfii:penfii@$pg_address/penfii"
export PENFII_TEST_OPENSEARCH_URL="http://$os_address"
export PENFII_MIGRATION_DATABASE_URL="$DATABASE_URL"
export SQLX_OFFLINE=true
unset PENFII_SKIP_SEARCH_TESTS

if ((index == 1)); then
  # SQLx compares its checked queries against a real, migrated schema once per suite.
  cargo run --locked -q -p admin-cli -- migrate
  env -u SQLX_OFFLINE cargo sqlx prepare --check --workspace -- --locked
fi
if ((count == 1)); then
  cargo test --locked
else
  scripts/integration-shard.sh "$index" "$count"
fi
