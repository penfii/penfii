#!/usr/bin/env bash
# Install and data checks for an unpacked install directory (the release ZIP's penfii/ directory) or for any
# directory that holds a compose.yaml and a .env, in its OWN Compose project, so it never touches another stack.
#
#   docker/release/smoke.sh env     DIR    write DIR/.env from .env.example with generated secrets (test values)
#   docker/release/smoke.sh up      DIR    pull (unless NO_PULL=1), start, wait until healthy
#   docker/release/smoke.sh seed    DIR    bootstrap the owner, sign in, make a team, project, ticket and an uploaded
#                                          file; the ids and the file's SHA-256 are written to DIR/smoke-state.json
#   docker/release/smoke.sh check   DIR    sign in again and compare everything seed made (ticket, project, file bytes)
#   docker/release/smoke.sh web     DIR    the web container answers the app shell and proxies /v1 to core-api
#   docker/release/smoke.sh restart DIR    stop and start every container (volumes kept), wait until healthy
#   docker/release/smoke.sh down    DIR    stop; with PURGE=1 also delete THIS project's volumes
#
# Settings (environment): SMOKE_PROJECT (Compose project, default penfii-smoke), SMOKE_PORT (default 18081, on
# 127.0.0.1), SMOKE_SUBNET (frontend network range, default 172.29.251.0/24). The owner is smoke-owner@example.test; the
# password is generated into DIR/.env-secrets. Test values only: nothing here is for a real installation.
set -euo pipefail
[ "$#" -eq 2 ] || { sed -n '2,19p' "$0"; exit 2; }
CMD="$1"; DIR="$(cd "$2" && pwd)"
P="${SMOKE_PROJECT:-penfii-smoke}"; PORT="${SMOKE_PORT:-18081}"; SUBNET="${SMOKE_SUBNET:-172.29.251.0/24}"
URL="http://127.0.0.1:$PORT"
dc() { docker compose -p "$P" --env-file "$DIR/.env" -f "$DIR/compose.yaml" "$@"; }
die() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }
rnd() { python3 -I -c 'import secrets,string;print("".join(secrets.choice(string.ascii_letters+string.digits) for _ in range(24)))'; }
OWNER=smoke-owner@example.test
JAR="$DIR/.smoke-cookies"

api() { # METHOD PATH [JSON]  -> body on stdout, fails on a non-2xx answer
  local m="$1" path="$2" body="${3:-}" out code
  out="$(mktemp)"
  code="$(curl -sS -o "$out" -w '%{http_code}' -b "$JAR" -c "$JAR" -X "$m" "$URL$path" \
    -H 'X-Penfii-Client: 1' -H "Idempotency-Key: $(python3 -I -c 'import uuid;print(uuid.uuid4())')" \
    ${body:+-H 'Content-Type: application/json' --data "$body"})"
  if [[ "$code" != 2* ]]; then echo "FAIL: $m $path -> $code: $(head -c 300 "$out")" >&2; rm -f "$out"; return 1; fi
  cat "$out"; rm -f "$out"
}
jget() { python3 -I -c 'import json,sys;d=json.load(sys.stdin)
for k in sys.argv[1].split("."): d=d[int(k)] if k.isdigit() else d[k]
print(d)' "$1"; }
login() {
  : > "$JAR"
  api POST /v1/session/login "$(python3 -I -c 'import json,sys;print(json.dumps({"email":sys.argv[1],"password":sys.argv[2]}))' "$OWNER" "$(cat "$DIR/.env-secrets")")" >/dev/null
}

case "$CMD" in
  env)
    [ -f "$DIR/.env.example" ] || die "no .env.example in $DIR"
    [ ! -e "$DIR/.env" ] || die "$DIR/.env exists; refusing to overwrite"
    umask 077
    pw="$(rnd)"; printf '%s' "Smoke-${pw}-Pw1" > "$DIR/.env-secrets"
    cp "$DIR/.env.example" "$DIR/.env"
    for k in POSTGRES_PASSWORD PENFII_DB_OWNER_PASSWORD PENFII_DB_RUNTIME_PASSWORD PENFII_S3_SECRET_KEY PENFII_S3_APP_SECRET_KEY; do
      sed -i "s/^$k=CHANGE_ME/$k=$(rnd)/" "$DIR/.env"
    done
    sed -i "s/^PENFII_S3_ACCESS_KEY=CHANGE_ME/PENFII_S3_ACCESS_KEY=root$(rnd)/; s/^PENFII_S3_APP_ACCESS_KEY=CHANGE_ME/PENFII_S3_APP_ACCESS_KEY=app$(rnd)/" "$DIR/.env"
    sed -i "s#^PENFII_HTTP_PORT=.*#PENFII_HTTP_PORT=$PORT#; s#^PENFII_PUBLIC_URL=.*#PENFII_PUBLIC_URL=$URL#" "$DIR/.env"
    printf '\nPENFII_FRONTEND_SUBNET=%s\n' "$SUBNET" >> "$DIR/.env"
    ! grep -q '^[^#]*CHANGE_ME' "$DIR/.env" || die "a CHANGE_ME is left in .env"
    ok ".env written (project $P, $URL, subnet $SUBNET)";;
  up)
    [ "${NO_PULL:-0}" = 1 ] || dc pull --quiet
    dc up -d --wait --wait-timeout 300
    dc ps --all --format '{{.Service}} {{.State}} {{.Health}} exit={{.ExitCode}}'
    for j in db-setup minio-setup; do
      [ "$(dc ps --all --format '{{.Service}} {{.ExitCode}}' | awk -v s=$j '$1==s{print $2}')" = 0 ] || die "$j did not exit 0"
    done
    ok "stack is up; db-setup and minio-setup exited 0";;
  seed)
    printf '%s' "$(cat "$DIR/.env-secrets")" | dc run --rm -T admin-cli bootstrap --email "$OWNER" --display-name "Smoke Owner" \
      --workspace-name "Smoke WS" --time-zone Asia/Seoul --password-stdin >/dev/null
    ok "bootstrap"
    login; ok "login"
    me="$(api GET /v1/me)"; uid="$(echo "$me" | jget id)"
    ws="$(api GET /v1/workspaces | jget 0.id)"
    team="$(api POST /v1/workspaces/$ws/teams "{\"name\":\"Smoke Team\",\"members\":[{\"userId\":\"$uid\",\"role\":\"MANAGER\"}]}" | jget id)" || die "team"
    proj="$(api POST /v1/workspaces/$ws/projects "{\"name\":\"Smoke Project\",\"teamIds\":[\"$team\"]}")"; pid="$(echo "$proj" | jget id)"
    ok "project $(echo "$proj" | jget ticketPrefix)"
    tk="$(api POST /v1/projects/$pid/tickets '{"title":"Smoke ticket","hierarchyKind":"TASK"}')"; tid="$(echo "$tk" | jget id)"
    ok "ticket $(echo "$tk" | jget key)"
    f="$DIR/.smoke-upload.bin"; head -c 300000 /dev/urandom > "$f"; sum="$(sha256sum "$f" | cut -d' ' -f1)"
    up="$(api POST /v1/workspaces/$ws/files "{\"filename\":\"smoke.bin\",\"byteSize\":300000}")"; fid="$(echo "$up" | jget id)"
    curl -sS -f -b "$JAR" -X PUT "$URL/v1/files/$fid/content" -H 'X-Penfii-Client: 1' -H 'Content-Type: application/octet-stream' \
      --data-binary "@$f" -o /dev/null || die "file bytes upload"
    api POST /v1/tickets/$tid/attachments "{\"fileId\":\"$fid\"}" >/dev/null
    ok "file uploaded and attached ($sum)"
    python3 -I -c 'import json,sys;json.dump(dict(zip(["ws","uid","team","project","ticket","file","sha256"],sys.argv[1:])),open(sys.argv[8],"w"))' \
      "$ws" "$uid" "$team" "$pid" "$tid" "$fid" "$sum" "$DIR/smoke-state.json";;
  check)
    [ -f "$DIR/smoke-state.json" ] || die "no smoke-state.json: run seed first"
    st() { python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$DIR/smoke-state.json" "$1"; }
    login; ok "login after restart/upgrade"
    [ "$(api GET /v1/tickets/$(st ticket) | jget title)" = "Smoke ticket" ] || die "ticket lost"
    api GET /v1/projects/$(st project)/tickets | grep -q "Smoke ticket" || die "ticket not listed"
    ok "project and ticket are there"
    att="$(api GET /v1/tickets/$(st ticket)/attachments | jget 0.id)" || die "attachment lost"
    curl -sS -f -b "$JAR" "$URL/v1/attachments/$att/content" -o "$DIR/.smoke-download.bin" || die "file download"
    [ "$(sha256sum "$DIR/.smoke-download.bin" | cut -d' ' -f1)" = "$(st sha256)" ] || die "file bytes differ"
    ok "uploaded file downloads with identical SHA-256"
    api GET "/v1/workspaces/$(st ws)/search?q=Smoke" >/dev/null 2>&1 && ok "search endpoint answers" || echo "note: search endpoint not checked";;
  web)
    code="$(curl -sS -o "$DIR/.smoke-index.html" -w '%{http_code}' "$URL/")"; [ "$code" = 200 ] || die "GET / -> $code"
    grep -qi '<div id="root"' "$DIR/.smoke-index.html" || grep -qi '<script' "$DIR/.smoke-index.html" || die "no app shell"
    [ "$(curl -sS -o /dev/null -w '%{http_code}' "$URL/readyz")" = 200 ] || die "/readyz"
    [ "$(curl -sS -o /dev/null -w '%{http_code}' "$URL/v1/me")" = 401 ] || die "/v1/me should answer 401 without a session"
    ok "web serves the app, /readyz 200, /v1 reaches core-api (401 without a session)";;
  restart)
    dc restart; dc up -d --wait --wait-timeout 300; ok "restarted, healthy again";;
  down)
    if [ "${PURGE:-0}" = 1 ]; then dc down -v; else dc down; fi; ok "down${PURGE:+ (volumes of $P deleted)}";;
  *) sed -n '2,19p' "$0"; exit 2;;
esac
