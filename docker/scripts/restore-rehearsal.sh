#!/usr/bin/env bash
# Full-service backup and an isolated restore rehearsal for the Compose stack (docs/self-hosting.md,
# "Restore rehearsal"). It never restores into, resets or deletes the source stack; the only thing it does
# to the source is a short write pause while the backup is taken (core-api and MinIO are stopped, then
# started again and checked).
#
#   scripts/restore-rehearsal.sh backup  DIR   PostgreSQL dump + file store archive + manifest (+ source.env)
#   scripts/restore-rehearsal.sh restore DIR   a second Compose project from that backup (own volumes, network
#                                              and port), mail, push and MCP switched off
#   scripts/restore-rehearsal.sh verify  DIR   compare the restored stack with the manifest, rebuild search;
#                                              what it checked and the result go to DIR/verify.txt
#   scripts/restore-rehearsal.sh down    DIR   stop the rehearsal project and delete ITS volumes
#
# Run from docker/. DIR must be outside this repository (a backup is never something to commit).
# Settings (environment):
#   SOURCE_PROJECT    Compose project that is backed up (default penfii)
#   REHEARSAL_PROJECT Compose project that is restored into (default penfii-rehearsal; must differ, and must
#                     not exist yet: restore refuses a project that has volumes, down refuses one that this
#                     script did not create)
#   REHEARSAL_PORT    host port of the rehearsal web app, on 127.0.0.1 (default 18080)
#   REHEARSAL_SUBNET  address range of the rehearsal's `frontend` network (default 172.29.250.0/24); it must
#                     not overlap the source's PENFII_FRONTEND_SUBNET (default 172.29.240.0/24)
#   SOURCE_ENV        the .env of the source (default ./.env)
#   ALPINE_IMAGE      helper image for the file store archive (default alpine:3.24.2, a pinned version)
#
# The backup directory holds the database, the uploaded files and source.env (the passwords and
# PENFII_WEBHOOK_KEK, PENFII_MFA_ENCRYPTION_KEY and the object storage keys the restored data needs) in clear
# text, with mode 700/600. Keep it private; docs/self-hosting.md, "Protecting backups and off-site copies",
# says how to encrypt it for storage elsewhere. This script prints no setting values.
set -euo pipefail

SOURCE_PROJECT="${SOURCE_PROJECT:-penfii}"
REHEARSAL_PROJECT="${REHEARSAL_PROJECT:-penfii-rehearsal}"
REHEARSAL_PORT="${REHEARSAL_PORT:-18080}"
REHEARSAL_SUBNET="${REHEARSAL_SUBNET:-172.29.250.0/24}"
SOURCE_ENV="${SOURCE_ENV:-./.env}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # docker/
REPO="$(cd "$HERE/.." && pwd)"
COMPOSE_FILE="$HERE/compose.yaml"
MARKER_LABEL="org.penfii.restore-rehearsal"
# The helper image that archives and unpacks the file store volume: a pinned version, so a backup is not
# made by whatever `alpine:latest` is that day. Change it on purpose (ALPINE_IMAGE) when you update it.
ALPINE_IMAGE="${ALPINE_IMAGE:-alpine:3.24.2}"

die() { echo "error: $*" >&2; exit 1; }
# An interrupt goes through `exit`, so the EXIT trap (which starts the source again) always runs.
trap 'exit 130' INT TERM HUP
say() { echo "== $*"; }

# Compose project names are lower-cased by Docker, so 'Penfii' would be the project 'penfii'. Accept only
# names that are already in Docker's form, and compare those.
valid_project() { [[ "$1" =~ ^[a-z0-9][a-z0-9_-]*$ ]]; }
valid_project "$SOURCE_PROJECT" || die "SOURCE_PROJECT must be lower-case letters, digits, '-' or '_'"
valid_project "$REHEARSAL_PROJECT" || die "REHEARSAL_PROJECT must be lower-case letters, digits, '-' or '_'"
[ "$SOURCE_PROJECT" != "$REHEARSAL_PROJECT" ] || die "SOURCE_PROJECT and REHEARSAL_PROJECT must differ"
[[ "$REHEARSAL_PORT" =~ ^[0-9]+$ ]] || die "REHEARSAL_PORT must be a number"
[[ "$REHEARSAL_SUBNET" =~ ^[0-9.]+/[0-9]+$ ]] || die "REHEARSAL_SUBNET must be an IPv4 range such as 172.29.250.0/24"

src() { docker compose -p "$SOURCE_PROJECT" -f "$COMPOSE_FILE" --env-file "$SOURCE_ENV" "$@"; }
# rh ENVFILE ARGS...: the rehearsal project, with that backup's rehearsal.env.
rh()  { docker compose -p "$REHEARSAL_PROJECT" -f "$COMPOSE_FILE" --env-file "$1" "${@:2}"; }
marker_volume() { echo "${REHEARSAL_PROJECT}_rehearsal-marker"; }

abs() { (cd "$1" && pwd); }

# The backup must not land in the repository, where it could be committed by accident.
outside_repo() { # absolute dir
  case "$1/" in "$REPO"/*) die "$1 is inside the repository; put backups elsewhere";; esac
}

# --- manifest: what the backup is expected to contain ---------------------------------------------------
# The state of the object storage account core-api uses (docs/self-hosting.md, "Object storage access key"):
# `works` (the app account signs in and can list the bucket), `missing`, or `not-configured` (no
# PENFII_S3_APP_ACCESS_KEY: core-api uses the root key pair). Run in the minio-setup service's environment,
# so no key is printed. Needs minio running.
s3_app_probe() { # compose command words...
  # The app key pair goes to mc in a config file in the container's tmpfs, not on its command line.
  # shellcheck disable=SC2016 # expanded by the shell in the container, from its environment
  "$@" run --rm --no-deps -T --entrypoint sh minio-setup -c '
    if [ -z "$APP_USER" ]; then echo not-configured; exit 0; fi
    esc() { r=$1 o=; while [ -n "$r" ]; do c=${r%"${r#?}"}; r=${r#?}; case $c in "\"" | \\) o="$o\\$c" ;; *) o="$o$c" ;; esac; done; printf %s "$o"; }
    umask 077; export MC_CONFIG_DIR=/tmp/mc-probe; mkdir -p "$MC_CONFIG_DIR"
    printf "{\"version\":\"10\",\"aliases\":{\"probe\":{\"url\":\"http://minio:9000\",\"accessKey\":\"%s\",\"secretKey\":\"%s\",\"api\":\"s3v4\",\"path\":\"auto\"}}}" \
      "$(esc "$APP_USER")" "$(esc "$APP_PASSWORD")" > "$MC_CONFIG_DIR/config.json"
    if mc ls "probe/$BUCKET" >/dev/null 2>&1; then echo works; else echo missing; fi' 2>/dev/null | tail -1 | tr -d '\r'
}

# `sessions` is left out: signing in to the restored copy adds rows to it.
report() { # compose command words... : prints a text report of the database
  "$@" exec -T postgres psql -U penfii -d penfii -v ON_ERROR_STOP=1 -At -c "
    SELECT 'migration ' || version || ' ' || success || ' ' || encode(checksum, 'hex')
      FROM penfii._sqlx_migrations ORDER BY version;
    SELECT 'rows ' || table_name || ' ' ||
           (xpath('/row/c/text()', query_to_xml(format('SELECT count(*) AS c FROM %I.%I', table_schema, table_name), false, true, '')))[1]::text
      FROM information_schema.tables WHERE table_schema = 'penfii' AND table_type = 'BASE TABLE'
        AND table_name <> 'sessions' ORDER BY table_name;"
}

# wait_healthy SECONDS SERVICE COMPOSE-COMMAND-WORDS...
wait_healthy() {
  local n="$1" svc="$2" id st; shift 2
  for _ in $(seq 1 "$n"); do
    id="$("$@" ps -q "$svc" 2>/dev/null || true)"
    if [ -n "$id" ]; then
      st="$(docker inspect --format '{{.State.Health.Status}}' "$id" 2>/dev/null || true)"
      [ "$st" = healthy ] && return 0
    fi
    sleep 1
  done
  return 1
}

image_id() { docker image inspect --format '{{.Id}}' "$1" 2>/dev/null | cut -c1-19; }
# The image a service's container runs (empty when there is none).
container_image() { # service compose-command-words...
  local id; id="$("${@:2}" ps -a -q "$1" 2>/dev/null | head -1)" || true
  [ -z "$id" ] || docker inspect --format '{{.Image}}' "$id" 2>/dev/null | cut -c1-19 || true
}

# The search index kinds of this checkout (server/crates/core/src/search/mapping.rs), one per line.
search_kinds() {
  local kinds
  kinds="$(sed -n 's/^pub const KINDS: &\[&str\] = &\[\(.*\)\];$/\1/p' "$REPO/server/crates/core/src/search/mapping.rs" 2>/dev/null \
    | tr -d '" ' | tr ',' '\n' | grep -E '^[a-z]+$' || true)"
  [ -n "$kinds" ] || die "cannot read the search index kinds from server/crates/core/src/search/mapping.rs"
  echo "$kinds"
}

# check_search_status KINDS < search-status output: every kind has exactly one line, READY with a live
# index, and the prefix check passed. Missing, partial or unexpected output fails.
check_search_status() {
  local status ok=0 kind lines
  status="$(cat)"
  if grep -q '^prefix check:' <<<"$status"; then echo "   FAILED: the index prefix check failed"; ok=1; fi
  for kind in $1; do
    lines="$(grep -E "^$kind +phase=" <<<"$status" || true)"
    if [ -z "$lines" ]; then echo "   FAILED: no status for search index '$kind'"; ok=1
    elif [ "$(wc -l <<<"$lines")" != 1 ]; then echo "   FAILED: more than one status line for '$kind'"; ok=1
    elif ! grep -qE "^$kind +phase=READY live=[^- ]" <<<"$lines"; then echo "   FAILED: search index '$kind' is not READY with a live index"; ok=1
    fi
  done
  [ "$ok" = 0 ] && echo "   every index READY: $(echo $1)"
  return "$ok"
}

cmd_backup() {
  local dir="${1:?usage: backup DIR (outside the repository)}"
  [ -f "$SOURCE_ENV" ] || die "no $SOURCE_ENV"
  [ ! -e "$dir" ] || die "$dir exists"
  local parent; parent="$(abs "$(dirname "$dir")")" || die "the parent of $dir does not exist"
  dir="$parent/$(basename "$dir")"
  outside_repo "$dir"
  local files_volume="${SOURCE_PROJECT}_penfii-files"
  docker volume inspect "$files_volume" >/dev/null 2>&1 || die "no volume $files_volume (is $SOURCE_PROJECT the right project?)"
  [ -n "$(src ps -q postgres 2>/dev/null)" ] || die "the postgres service of $SOURCE_PROJECT is not running"
  umask 077
  mkdir "$dir"

  # A short write pause: nothing writes to PostgreSQL or the file store without core-api, and MinIO is
  # stopped so the volume is not copied while it changes. They are started again whatever happens.
  say "pausing writes (stop core-api, minio)"
  trap 'src start minio core-api >/dev/null 2>&1 || true; echo "the source was started again; the backup in $dir is incomplete" >&2' EXIT
  src stop core-api minio >/dev/null 2>&1

  say "dump PostgreSQL (custom format)"
  src exec -T postgres pg_dump -U penfii -Fc penfii > "$dir/penfii.dump"
  [ -s "$dir/penfii.dump" ] || die "empty dump"

  say "archive the file store volume"
  # Streamed, so the archive is written by this shell (umask 077, owned by the caller), not by the container.
  docker run --rm -v "$files_volume":/data:ro "$ALPINE_IMAGE" tar czf - -C /data . > "$dir/penfii-files.tar.gz"

  say "manifest"
  {
    echo "created $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "source_project $SOURCE_PROJECT"
    echo "git_head $(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "git_describe $(git -C "$REPO" describe --tags --always --dirty 2>/dev/null || echo unknown)"
    echo "image_core_api $(docker inspect --format '{{.Image}}' "$(src ps -a -q core-api)" 2>/dev/null | cut -c1-19)"
    echo "image_web $(docker inspect --format '{{.Image}}' "$(src ps -q web)" 2>/dev/null | cut -c1-19)"
    echo "files_archive_bytes $(stat -c %s "$dir/penfii-files.tar.gz")"
    # Objects of the bucket `penfii` (one xl.meta per object); MinIO's own files outside it change while it runs.
    # Whether the source had a separate object storage account for core-api (no value is recorded).
    if grep -qE '^[[:space:]]*(export[[:space:]]+)?PENFII_S3_APP_ACCESS_KEY=.+' "$SOURCE_ENV"; then echo "s3_app_account configured"; else echo "s3_app_account root_fallback"; fi
    echo "files_objects $(tar tzf "$dir/penfii-files.tar.gz" | grep -c '^\./penfii/.*xl\.meta$' || true)"
    report src
  } > "$dir/manifest.txt"
  ( cd "$dir" && sha256sum penfii.dump penfii-files.tar.gz > SHA256SUMS )

  # The settings the restored data needs (secrets): kept with the backup, never printed.
  cp "$SOURCE_ENV" "$dir/source.env"; chmod 600 "$dir/source.env"

  say "start the source again"
  src start minio core-api >/dev/null 2>&1
  trap - EXIT
  wait_healthy 120 core-api src || die "source core-api is not healthy again: check 'docker compose -p $SOURCE_PROJECT ps'"
  wait_healthy 60 minio src || die "source minio is not healthy again"
  chmod -R go-rwx "$dir"
  say "source is writing again. Backup: $dir"
  echo "   It holds your data and secrets in clear text (mode 700). To keep a copy elsewhere, encrypt it first and keep"
  echo "   the key apart from it: docs/self-hosting.md, \"Protecting backups and off-site copies\"."
  grep -E '^(created|git_|image_|files_)' "$dir/manifest.txt"
}

# The rehearsal settings: the source settings the data needs (database role passwords, file store keys,
# webhook key, limits) with everything that talks to the outside switched off and the address changed.
make_rehearsal_env() { # dir
  local dir="$1" env="$1/rehearsal.env"
  grep -vE '^[[:space:]]*(export[[:space:]]+)?(PENFII_PUBLIC_URL|PENFII_HTTP_BIND|PENFII_HTTP_PORT|PENFII_SMTP_[A-Z_]+|PENFII_VAPID_[A-Z_]+|PENFII_MCP_[A-Z_]+|PENFII_OPENSEARCH_INDEX_PREFIX|PENFII_FRONTEND_SUBNET|PENFII_TRUSTED_PROXIES)[[:space:]]*=' \
    "$dir/source.env" > "$env" || true
  {
    echo "PENFII_PUBLIC_URL=http://127.0.0.1:$REHEARSAL_PORT"
    echo "PENFII_HTTP_BIND=127.0.0.1"
    echo "PENFII_HTTP_PORT=$REHEARSAL_PORT"
    # The source is running, so the rehearsal's network needs its own range (core-api trusts the web container on it).
    echo "PENFII_FRONTEND_SUBNET=$REHEARSAL_SUBNET"
    echo "PENFII_SMTP_HOST="
    echo "PENFII_VAPID_PRIVATE_KEY="
    echo "PENFII_VAPID_SUBJECT="
    echo "PENFII_MCP_ENABLED=0"
    echo "PENFII_MCP_DCR_ENABLED=0"
  } >> "$env"
  chmod 600 "$env"
}

cmd_restore() {
  local dir; dir="$(abs "${1:?usage: restore DIR}")"
  [ -s "$dir/penfii.dump" ] && [ -s "$dir/penfii-files.tar.gz" ] && [ -f "$dir/source.env" ] || die "$dir is not a backup made by this script"
  ( cd "$dir" && sha256sum -c SHA256SUMS >/dev/null ) || die "checksum mismatch: the backup is damaged"
  # A backup that came back from encrypted storage may have lost its modes: source.env holds every secret.
  if [ -n "$(find "$dir" -maxdepth 1 \( -name source.env -o -name rehearsal.env -o -name '*.dump' \) -perm /077 2>/dev/null)" ]; then
    echo "   warning: files in $dir are readable by other users; setting the backup directory to mode 700 and its files to 600"
    chmod 700 "$dir"; chmod 600 "$dir"/source.env "$dir"/*.dump "$dir"/*.tar.gz 2>/dev/null || true
  fi
  # The rehearsal must be new: no volume and no container of that project may exist, and the port is free.
  local existing
  existing="$(docker volume ls -q --filter "label=com.docker.compose.project=$REHEARSAL_PROJECT"; docker volume ls -q --filter "name=^${REHEARSAL_PROJECT}_")"
  [ -z "$existing" ] || die "project $REHEARSAL_PROJECT already has volumes: run '$0 down DIR' first (or choose another REHEARSAL_PROJECT)"
  [ -z "$(docker ps -aq --filter "label=com.docker.compose.project=$REHEARSAL_PROJECT")" ] || die "project $REHEARSAL_PROJECT already has containers"
  if (exec 3<>"/dev/tcp/127.0.0.1/$REHEARSAL_PORT") 2>/dev/null; then die "port $REHEARSAL_PORT is in use"; fi
  make_rehearsal_env "$dir"
  local env="$dir/rehearsal.env"
  # Marks the project as one this script made, so that `down` may delete it (and nothing else).
  docker volume create --label "$MARKER_LABEL=1" "$(marker_volume)" >/dev/null

  local want_api; want_api="$(sed -n 's/^image_core_api //p' "$dir/manifest.txt")"
  if [ -n "$want_api" ] && [ "$want_api" != "$(image_id penfii-core-api:local)" ]; then
    echo "   note: penfii-core-api:local is not the image the backup was taken with (see manifest.txt);"
    echo "         restore with the same release, or a later one whose changelog you have read"
    echo "         (verify records the images it checked in verify.txt)"
  fi

  say "1. PostgreSQL of $REHEARSAL_PROJECT"
  rh "$env" up -d --no-build postgres >/dev/null 2>&1
  wait_healthy 90 postgres rh "$env" || die "rehearsal postgres is not healthy"

  say "2. roles first (they are not in a dump), then the dump"
  rh "$env" run --rm --no-deps --entrypoint sh db-setup \
    -c 'admin-cli db-provision --owner-role "$PENFII_DB_OWNER_USER" --runtime-role "$PENFII_DB_RUNTIME_USER"' >/dev/null 2>&1
  # Before anything is replaced: this must be the new, empty database of the rehearsal project.
  local pg_project tables
  pg_project="$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project"}}' "$(rh "$env" ps -q postgres)")"
  [ "$pg_project" = "$REHEARSAL_PROJECT" ] || die "the postgres container is not the rehearsal's"
  tables="$(rh "$env" exec -T postgres psql -U penfii -d penfii -At -c "SELECT count(*) FROM pg_tables WHERE schemaname = 'penfii'")"
  [ "$tables" = 0 ] || die "the rehearsal database is not empty ($tables tables); refusing to restore over it"
  # db-provision made an empty schema `penfii`; --clean --if-exists replaces it with the dump's (same owner).
  rh "$env" exec -T postgres pg_restore -U penfii -d penfii --clean --if-exists --exit-on-error < "$dir/penfii.dump"

  say "3. file store"
  rh "$env" create --no-build minio >/dev/null 2>&1   # creates the volume without starting the service
  docker run --rm -i -v "${REHEARSAL_PROJECT}_penfii-files":/data "$ALPINE_IMAGE" tar xzf - -C /data < "$dir/penfii-files.tar.gz"

  # MinIO keeps its accounts in the volume just unpacked, so the app account must already be there, before
  # minio-setup (which would make it again) has run. A backup without it still restores: minio-setup makes it.
  rh "$env" up -d --no-build --no-deps minio >/dev/null 2>&1
  wait_healthy 90 minio rh "$env" || die "rehearsal minio is not healthy"
  local acct; acct="$(s3_app_probe rh "$env")"
  echo "$acct" > "$dir/s3-account-restored.txt"
  case "$acct" in
    works) echo "   object storage app account came back with the file store volume";;
    not-configured) echo "   no app account configured (core-api uses the root key pair)";;
    *) echo "   note: the app account is not in the restored file store; minio-setup makes it in the next step";;
  esac

  say "4. db-setup (grants, timeouts, migration check), then the services"
  rh "$env" up -d --no-build >/dev/null 2>&1
  wait_healthy 180 core-api rh "$env" || die "rehearsal core-api is not healthy: docker compose -p $REHEARSAL_PROJECT logs core-api"
  wait_healthy 60 web rh "$env" || die "rehearsal web is not healthy"
  say "restored: http://127.0.0.1:$REHEARSAL_PORT  (project $REHEARSAL_PROJECT; mail, push and MCP are off)"
}

cmd_verify() {
  local dir; dir="$(abs "${1:?usage: verify DIR}")"
  local env="$dir/rehearsal.env" fail=0
  [ -f "$env" ] || die "run restore first"

  # What was verified: the images the rehearsal runs (which may differ from the backup's, see restore),
  # recorded in verify.txt next to the manifest.
  say "versions (recorded in verify.txt)"
  local result="$dir/verify.txt" svc img want
  {
    echo "verified $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "rehearsal_project $REHEARSAL_PROJECT"
    echo "checkout_git_head $(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "checkout_git_describe $(git -C "$REPO" describe --tags --always --dirty 2>/dev/null || echo unknown)"
    echo "backup_git_describe $(sed -n 's/^git_describe //p' "$dir/manifest.txt")"
  } > "$result"
  for svc in core_api web; do
    img="$(container_image "${svc/_/-}" rh "$env")"
    want="$(sed -n "s/^image_$svc //p" "$dir/manifest.txt")"
    echo "image_$svc ${img:-none}" >> "$result"
    if [ -n "$img" ] && [ "$img" = "$want" ]; then
      echo "image_${svc}_same_as_backup yes" >> "$result"; echo "   ${svc/_/-} $img (same as the backup)"
    else
      echo "image_${svc}_same_as_backup no" >> "$result"; echo "   ${svc/_/-} ${img:-none} (the backup was taken with ${want:-unknown})"
    fi
  done

  say "readyz"
  if curl -fsS "http://127.0.0.1:$REHEARSAL_PORT/readyz" >/dev/null; then echo "   200"; else echo "   FAILED"; fail=1; fi

  say "object storage account"
  local acct; acct="$(s3_app_probe rh "$env")"
  echo "s3_app_account_after_setup ${acct:-unknown}" >> "$result"
  echo "s3_app_account_in_restored_volume $(cat "$dir/s3-account-restored.txt" 2>/dev/null || echo unknown)" >> "$result"
  case "$acct" in
    works|not-configured) echo "   $acct";;
    *) echo "   FAILED: the app account cannot use the bucket (${acct:-no answer})"; fail=1;;
  esac

  say "database against the manifest (migrations with checksums, row counts)"
  report rh "$env" > "$dir/restored-report.txt"
  grep -E '^(migration|rows) ' "$dir/manifest.txt" > "$dir/manifest-db.txt"
  if diff -u "$dir/manifest-db.txt" "$dir/restored-report.txt"; then
    echo "   same: $(grep -c '^migration ' "$dir/restored-report.txt") migrations, $(grep -c '^rows ' "$dir/restored-report.txt") tables"
  else echo "   FAILED: differs"; fail=1; fi

  say "file store: objects of the bucket against the manifest"
  local want got
  want="$(sed -n 's/^files_objects //p' "$dir/manifest.txt")"
  got="$(docker run --rm -v "${REHEARSAL_PROJECT}_penfii-files":/data:ro "$ALPINE_IMAGE" sh -c 'find /data/penfii -name xl.meta 2>/dev/null | wc -l' | tr -d ' ')"
  echo "   manifest $want, restored volume $got"
  [ "$want" = "$got" ] || { echo "   FAILED"; fail=1; }

  say "search: rebuild from PostgreSQL, then status"
  local kinds reindex status
  kinds="$(search_kinds)"
  if reindex="$(rh "$env" run --rm admin-cli search-reindex 2>&1)" && grep -q '^rebuilt:' <<<"$reindex"; then
    grep '^rebuilt:' <<<"$reindex" | sed 's/^/   /'
  else
    grep -v '^ Container ' <<<"$reindex" | tail -5; echo "   FAILED: search-reindex"; fail=1
  fi
  status="$(rh "$env" run --rm admin-cli search-status 2>/dev/null)" || { echo "   FAILED: search-status"; fail=1; }
  echo "$status" | tail -12
  if ! check_search_status "$kinds" <<<"$status"; then fail=1; fi

  say "event deliveries that were given up on"
  local dead; dead="$(rh "$env" run --rm admin-cli events-dead 2>/dev/null || true)"
  echo "$dead" | tail -3
  echo "$dead" | grep -q 'dead deliveries: 0' || { echo "   FAILED: dead deliveries"; fail=1; }

  if [ "$fail" != 0 ]; then echo "result FAILED" >> "$result"; die "verification failed (see $result)"; fi
  echo "result OK" >> "$result"
  say "OK (signing in and opening tickets, documents and files in the browser is a separate check; see docs/self-hosting.md)"
}

cmd_down() {
  local dir; dir="$(abs "${1:?usage: down DIR}")"
  local env="$dir/rehearsal.env"
  [ -f "$env" ] || die "$env does not exist"
  docker volume inspect --format "{{index .Labels \"$MARKER_LABEL\"}}" "$(marker_volume)" 2>/dev/null | grep -qx 1 \
    || die "project $REHEARSAL_PROJECT was not made by this script; refusing to delete it"
  say "stopping $REHEARSAL_PROJECT and deleting its volumes (never the source's)"
  rh "$env" down -v --remove-orphans >/dev/null 2>&1
  docker volume rm "$(marker_volume)" >/dev/null
  say "removed"
}

case "${1:-}" in
  backup)  shift; cmd_backup "$@";;
  restore) shift; cmd_restore "$@";;
  verify)  shift; cmd_verify "$@";;
  down)    shift; cmd_down "$@";;
  *) sed -n '2,24p' "$0"; exit 2;;
esac
