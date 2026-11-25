#!/usr/bin/env bash
# Builds (and with PUSH=1 publishes) the three official images from a SOURCE CHECKOUT OF A RELEASE TAG.
#
#   docker/release/build-images.sh VERSION SRC [IMAGE...]
#
# VERSION is X.Y.Z (no v), SRC the checkout of tag vVERSION (the script itself may come from a newer checkout:
# the images contain only what SRC holds, never the release tooling). IMAGE is any of core-api, web, opensearch
# (default: all three). Settings (environment):
#   IMAGE_PREFIX  registry path (default ghcr.io/penfii)
#   PUSH          1 = push (needs a prior `docker login`), with a provenance and an SBOM attestation; anything
#                 else = build into the local Docker image store only, nothing is published
#   OUT           directory for digests.txt (name=sha256:...), default ./release-out
# Tags are immutable: when IMAGE_PREFIX/NAME:VERSION already exists in the registry, it is NOT built or pushed
# again. It must carry this SOURCE's commit in org.opencontainers.image.revision or the script fails, so a re-run
# can never replace a published image with one built from another source. Only the version tag is pushed
# (no `latest`). Platform: linux/amd64 only.
set -euo pipefail
[ "$#" -ge 2 ] || { sed -n '2,19p' "$0"; exit 2; }
VERSION="$1"; SRC="$(cd "$2" && pwd)"; shift 2
IMAGES=("$@"); [ "${#IMAGES[@]}" -gt 0 ] || IMAGES=(core-api web opensearch)
PREFIX="${IMAGE_PREFIX:-ghcr.io/penfii}"; OUT="${OUT:-release-out}"; PUSH="${PUSH:-0}"
TAG="v$VERSION"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "error: VERSION must be X.Y.Z" >&2; exit 2; }
SHA="$(git -C "$SRC" rev-parse "refs/tags/$TAG^{commit}")" || { echo "error: $SRC has no tag $TAG" >&2; exit 1; }
[ "$(git -C "$SRC" rev-parse HEAD)" = "$SHA" ] || { echo "error: $SRC is not checked out at $TAG ($SHA)" >&2; exit 1; }
[ -z "$(git -C "$SRC" status --porcelain)" ] || { echo "error: $SRC has local changes" >&2; exit 1; }
mkdir -p "$OUT"; : > "$OUT/digests.txt"
CREATED="$(git -C "$SRC" log -1 --format=%cI "$SHA")"   # the commit's own date: no wall-clock in the labels

if ! docker buildx inspect penfii-release >/dev/null 2>&1; then
  docker buildx create --name penfii-release --driver docker-container --driver-opt network=host >/dev/null
fi

for name in "${IMAGES[@]}"; do
  case "$name" in
    core-api) file="$SRC/docker/server/Dockerfile"; ctx="$SRC"; desc="Penfii server (core-api) and operator CLI (admin-cli)";;
    web)      file="$SRC/docker/web/Dockerfile";    ctx="$SRC"; desc="Penfii web app served by Caddy";;
    opensearch) file="$SRC/docker/opensearch/Dockerfile"; ctx="$SRC/docker/opensearch"; desc="OpenSearch with the nori Korean analyzer for Penfii";;
    *) echo "error: unknown image $name" >&2; exit 2;;
  esac
  ref="$PREFIX/$name:$VERSION"
  if [ "$PUSH" = 1 ] && docker buildx imagetools inspect "$ref" >/dev/null 2>&1; then
    have="$(docker buildx imagetools inspect "$ref" --format '{{json .Image}}' 2>/dev/null \
      | python3 -I -c 'import json,sys
d=json.load(sys.stdin)
d=d.get("linux/amd64", d) if isinstance(d, dict) and "config" not in d else d
print((d.get("config") or {}).get("Labels", {}).get("org.opencontainers.image.revision",""))' || true)"
    [ "$have" = "$SHA" ] || { echo "error: $ref already exists with revision '${have:-unknown}', not $SHA; refusing to replace it" >&2; exit 1; }
    echo "== $ref exists with revision $SHA: kept, not rebuilt"
    echo "$name=$(docker buildx imagetools inspect "$ref" --format '{{.Manifest.Digest}}')" >> "$OUT/digests.txt"
    continue
  fi
  echo "== building $ref from $TAG ($SHA)"
  args=(buildx build --builder penfii-release --platform linux/amd64 -f "$file" --tag "$ref"
    --label "org.opencontainers.image.title=penfii-$name" --label "org.opencontainers.image.description=$desc"
    --label "org.opencontainers.image.version=$VERSION" --label "org.opencontainers.image.revision=$SHA"
    --label "org.opencontainers.image.source=https://github.com/penfii/penfii" --label "org.opencontainers.image.url=https://github.com/penfii/penfii"
    --label "org.opencontainers.image.documentation=https://github.com/penfii/penfii/tree/$TAG/docs"
    --label "org.opencontainers.image.licenses=GPL-3.0-only" --label "org.opencontainers.image.created=$CREATED"
    --label "org.opencontainers.image.ref.name=$TAG"
    --metadata-file "$OUT/$name.metadata.json")
  if [ "$PUSH" = 1 ]; then args+=(--push --provenance=mode=max --sbom=true); else args+=(--load); fi
  docker "${args[@]}" "$ctx"
  if [ "$PUSH" = 1 ]; then
    python3 -I -c 'import json,sys;print(sys.argv[1]+"="+json.load(open(sys.argv[2]))["containerimage.digest"])' "$name" "$OUT/$name.metadata.json" >> "$OUT/digests.txt"
  else
    echo "$name=$(docker image inspect --format '{{.Id}}' "$ref")" >> "$OUT/digests.txt"
  fi
done
cat "$OUT/digests.txt"
