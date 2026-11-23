#!/bin/sh
# One-shot object storage setup, run by the `minio-setup` service on every `docker compose up`, before
# core-api starts. It uses the storage root key pair (the only place it is used besides MinIO itself):
#   1. makes the bucket (core-api no longer needs the right to), and
#   2. when PENFII_S3_APP_ACCESS_KEY / PENFII_S3_APP_SECRET_KEY are set, makes (or updates) a user that
#      may only read, write and delete objects in that bucket, which is what core-api then connects as.
# Without them it only makes the bucket and warns: core-api keeps the root key pair, as installations
# from before this setup always did (docs/self-hosting.md, "Object storage access key"). Safe to run again.
# It prints no key or secret, and puts none on a command line (where `ps` would show it): mc reads the root
# key pair from a config file in the container's tmpfs, and the app secret from its standard input.
set -eu
umask 077

: "${ROOT_USER:?}" "${ROOT_PASSWORD:?}" "${BUCKET:?}"
APP_USER="${APP_USER:-}"
APP_PASSWORD="${APP_PASSWORD:-}"
ENDPOINT="${ENDPOINT:-http://minio:9000}"
POLICY=penfii-app

if [ -n "$APP_USER" ] && [ -z "$APP_PASSWORD" ]; then
  echo "minio-setup: set both PENFII_S3_APP_ACCESS_KEY and PENFII_S3_APP_SECRET_KEY, or neither" >&2
  exit 1
fi
if [ -z "$APP_USER" ] && [ -n "$APP_PASSWORD" ]; then
  echo "minio-setup: set both PENFII_S3_APP_ACCESS_KEY and PENFII_S3_APP_SECRET_KEY, or neither" >&2
  exit 1
fi
if [ "$APP_USER" = CHANGE_ME ] || [ "$APP_PASSWORD" = CHANGE_ME ]; then
  echo "minio-setup: PENFII_S3_APP_ACCESS_KEY / PENFII_S3_APP_SECRET_KEY still hold the CHANGE_ME placeholder; set real values, or remove both lines" >&2
  exit 1
fi
if [ -n "$APP_USER" ] && [ "$APP_USER" = "$ROOT_USER" ] && [ "$APP_PASSWORD" != "$ROOT_PASSWORD" ]; then
  echo "minio-setup: PENFII_S3_APP_ACCESS_KEY equals the root key but PENFII_S3_APP_SECRET_KEY is not the root secret, so core-api could not sign in; use another app key, or remove both lines" >&2
  exit 1
fi

# The text as a JSON string body (backslash and double quote escaped).
json_escape() {
  rest=$1 out=
  while [ -n "$rest" ]; do
    char=${rest%"${rest#?}"}
    rest=${rest#?}
    case $char in
      '"' | \\) out="$out\\$char" ;;
      *) out="$out$char" ;;
    esac
  done
  printf '%s' "$out"
}

MC_CONFIG_DIR=/tmp/mc
export MC_CONFIG_DIR
mkdir -p "$MC_CONFIG_DIR"
printf '{"version":"10","aliases":{"local":{"url":"%s","accessKey":"%s","secretKey":"%s","api":"s3v4","path":"auto"}}}\n' \
  "$(json_escape "$ENDPOINT")" "$(json_escape "$ROOT_USER")" "$(json_escape "$ROOT_PASSWORD")" > "$MC_CONFIG_DIR/config.json"

tries=0
until mc ls local >/dev/null 2>&1; do
  tries=$((tries + 1))
  [ "$tries" -lt 60 ] || { echo "minio-setup: cannot sign in to $ENDPOINT with the storage root key pair" >&2; exit 1; }
  sleep 1
done

# `docker compose run --rm --no-deps minio-setup mc ARGS...`: an operator's mc command with the root key
# pair as the alias `local` (for example `mc admin user rm local OLD-APP-KEY`), and nothing else.
if [ "${1:-}" = mc ]; then
  shift
  exec mc "$@"
fi

mc mb --ignore-existing "local/$BUCKET" >/dev/null
echo "minio-setup: bucket $BUCKET is there"

if [ -z "$APP_USER" ]; then
  echo "minio-setup: WARNING: PENFII_S3_APP_ACCESS_KEY is not set, so core-api uses the storage ROOT key pair." >&2
  echo "minio-setup: WARNING: it works, but set an app key pair to limit core-api to the bucket (docs/self-hosting.md)." >&2
  exit 0
fi
if [ "$APP_USER" = "$ROOT_USER" ]; then
  echo "minio-setup: WARNING: PENFII_S3_APP_ACCESS_KEY equals the root key; core-api uses the storage ROOT key pair." >&2
  exit 0
fi

cat > /tmp/policy.json <<P
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:GetBucketLocation", "s3:ListBucket", "s3:ListBucketMultipartUploads"],
      "Resource": ["arn:aws:s3:::$BUCKET"]
    },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
      "Resource": ["arn:aws:s3:::$BUCKET/*"]
    }
  ]
}
P
mc admin policy create local "$POLICY" /tmp/policy.json >/dev/null
rm -f /tmp/policy.json
# Adding an existing user sets its secret again, so changing the secret in .env takes effect on the next start.
# Without the secret argument, mc reads it from standard input.
printf '%s\n' "$APP_PASSWORD" | mc admin user add local "$APP_USER" >/dev/null
mc admin policy attach local "$POLICY" --user "$APP_USER" >/dev/null 2>&1 || true
case "$(mc admin user info local "$APP_USER")" in
  *"$POLICY"*) ;;
  *) echo "minio-setup: policy $POLICY is not attached to the app user" >&2; exit 1;;
esac
echo "minio-setup: app user is ready, limited to bucket $BUCKET"
