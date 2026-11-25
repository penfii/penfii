#!/usr/bin/env bash
# Software bill of materials (SBOM) and a container vulnerability report for this checkout. CI runs the
# same two commands in .github/workflows/ci.yml and .forgejo/workflows/ci.yml (job `supply-chain`); run them
# here to see what CI sees.
#
#   docker/scripts/supply-chain.sh sbom OUTDIR   CycloneDX JSON SBOMs of the source tree (Cargo.lock,
#                                                package-lock.json) and of the two images, each stamped with
#                                                the commit (and tag, when there is one) and the image id,
#                                                then checked against the CycloneDX schema. Fails when an SBOM
#                                                is missing, invalid or not stamped.
#   docker/scripts/supply-chain.sh scan OUTDIR   a vulnerability report for the two images (Grype). It only
#                                                reports: findings never fail it, and a scanner that cannot run
#                                                (no network for its database) is written down as UNAVAILABLE.
#
# The images are the ones `docker compose -f docker/compose.yaml build` makes (IMAGE_CORE_API, IMAGE_WEB;
# default penfii-core-api:local and penfii-web:local); build them first. Needs Docker; the tools run from
# pinned images. OUTDIR is created. Settings: COMMIT (default: git HEAD), TAG (default: the tag on HEAD, or
# none).
#
# What this does NOT cover: images built by anything else (for example another pipeline that builds for a
# deployment); the Rust crates are listed from Cargo.lock in the source SBOM, not read from the binary.
set -euo pipefail

SYFT_IMAGE="ghcr.io/anchore/syft:v1.57.2"
GRYPE_IMAGE="ghcr.io/anchore/grype:v0.122.1"
CDX_IMAGE="cyclonedx/cyclonedx-cli:0.33.1"
PYTHON_IMAGE="python:3.13.18-slim"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# REPO: the source tree to describe (default: this checkout). The release workflow points it at the release tag's
# checkout while the script itself comes from the tooling checkout.
REPO="${REPO:-$(cd "$HERE/.." && pwd)}"
IMAGE_CORE_API="${IMAGE_CORE_API:-penfii-core-api:local}"
IMAGE_WEB="${IMAGE_WEB:-penfii-web:local}"
# Optional third image (the release workflow sets it); without it only core-api and web are covered.
IMAGE_OPENSEARCH="${IMAGE_OPENSEARCH:-}"
COMMIT="${COMMIT:-$(git -C "$REPO" rev-parse HEAD)}"
TAG="${TAG:-$(git -C "$REPO" describe --tags --exact-match HEAD 2>/dev/null || true)}"

# The tools run as root (they need the Docker socket); hand the files back to the caller.
own() { docker run --rm -v "$OUT":/out "$PYTHON_IMAGE" chown -R "$(id -u):$(id -g)" /out >/dev/null 2>&1 || true; }
die() { echo "error: $*" >&2; exit 1; }
[ "$#" -eq 2 ] || { sed -n '2,19p' "$0"; exit 2; }
MODE="$1"
mkdir -p "$2"
OUT="$(cd "$2" && pwd)"
trap own EXIT

# stamp FILE NAME VALUE... : adds properties to the SBOM's metadata (the tool reads and writes only OUT).
stamp() {
  local file="$1"; shift
  docker run --rm -v "$OUT":/out "$PYTHON_IMAGE" python -I -c '
import json, sys
path = "/out/" + sys.argv[1]
bom = json.load(open(path))
props = bom.setdefault("metadata", {}).setdefault("properties", [])
args = sys.argv[2:]
for name, value in zip(args[::2], args[1::2]):
    props.append({"name": name, "value": value})
json.dump(bom, open(path, "w"), indent=1)
' "$file" "$@"
}

sbom_one() { # FILE NAME SYFT-SOURCE [IMAGE]
  local file="$1" name="$2" source="$3" image="${4:-}" id="" extra=()
  local run=(docker run --rm -v "$OUT":/out)
  if [ -n "$image" ]; then
    docker image inspect "$image" >/dev/null 2>&1 || die "image $image does not exist: build it first (docker compose -f docker/compose.yaml build)"
    id="$(docker image inspect --format '{{.Id}}' "$image")"
    run+=(-v /var/run/docker.sock:/var/run/docker.sock)
    extra=(penfii:image "$image" penfii:image-id "$id")
  else
    run+=(-v "$REPO":/src:ro)
  fi
  "${run[@]}" "$SYFT_IMAGE" "$source" -q --source-name "$name" --source-version "$COMMIT" \
    -o "cyclonedx-json=/out/$file"
  [ -s "$OUT/$file" ] || die "$file was not written"
  stamp "$file" penfii:commit "$COMMIT" penfii:tag "${TAG:-none}" "${extra[@]}"
  docker run --rm -v "$OUT":/in:ro "$CDX_IMAGE" validate --input-file "/in/$file" --input-format json --fail-on-errors >/dev/null \
    || die "$file is not a valid CycloneDX document"
  # The commit must really be in the file, and (images) the id of the image that was described.
  grep -q "\"$COMMIT\"" "$OUT/$file" || die "$file does not carry the commit"
  [ -z "$id" ] || grep -q "$id" "$OUT/$file" || die "$file does not carry the image id"
  echo "   $file: $(grep -c '"bom-ref"' "$OUT/$file") entries, commit ${COMMIT:0:12}${TAG:+, tag $TAG}${id:+, image ${id:0:19}}"
}

cmd_sbom() {
  echo "== SBOM (CycloneDX) for commit $COMMIT${TAG:+ (tag $TAG)}"
  sbom_one source.cdx.json penfii "dir:/src"
  sbom_one core-api.cdx.json penfii-core-api "docker:$IMAGE_CORE_API" "$IMAGE_CORE_API"
  sbom_one web.cdx.json penfii-web "docker:$IMAGE_WEB" "$IMAGE_WEB"
  [ -z "$IMAGE_OPENSEARCH" ] || sbom_one opensearch.cdx.json penfii-opensearch "docker:$IMAGE_OPENSEARCH" "$IMAGE_OPENSEARCH"
  {
    echo "commit $COMMIT"
    echo "tag ${TAG:-none}"
    echo "created $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "syft $SYFT_IMAGE"
    echo "cyclonedx-cli $CDX_IMAGE"
    echo "image_core_api $IMAGE_CORE_API $(docker image inspect --format '{{.Id}}' "$IMAGE_CORE_API")"
    echo "image_web $IMAGE_WEB $(docker image inspect --format '{{.Id}}' "$IMAGE_WEB")"
    [ -z "$IMAGE_OPENSEARCH" ] || echo "image_opensearch $IMAGE_OPENSEARCH $(docker image inspect --format '{{.Id}}' "$IMAGE_OPENSEARCH")"
  } > "$OUT/sbom-manifest.txt"
  echo "== OK: $OUT"
}

# Counts by severity, and how many have a fix, from a Grype JSON report.
summarize() { # FILE
  docker run --rm -v "$OUT":/in:ro "$PYTHON_IMAGE" python -I -c '
import json, sys
matches = json.load(open("/in/" + sys.argv[1])).get("matches", [])
sev = {}
for m in matches:
    v = m["vulnerability"]
    s = v.get("severity", "Unknown")
    fixable = v.get("fix", {}).get("state") == "fixed"
    t = sev.setdefault(s, [0, 0]); t[0] += 1; t[1] += fixable
order = ["Critical", "High", "Medium", "Low", "Negligible", "Unknown"]
print("   " + (", ".join("%s %d (%d with a fix)" % (s, *sev[s]) for s in order if s in sev) or "no known vulnerabilities"))
' "$1"
}

cmd_scan() {
  echo "== vulnerability report (report only; commit $COMMIT${TAG:+, tag $TAG})"
  local name image
  local pairs=("core-api|$IMAGE_CORE_API" "web|$IMAGE_WEB")
  [ -z "$IMAGE_OPENSEARCH" ] || pairs+=("opensearch|$IMAGE_OPENSEARCH")
  for pair in "${pairs[@]}"; do
    name="${pair%%|*}"; image="${pair#*|}"
    echo "-- $name ($image)"
    if ! docker image inspect "$image" >/dev/null 2>&1; then echo "   UNAVAILABLE: image not built"; continue; fi
    if docker run --rm -v /var/run/docker.sock:/var/run/docker.sock -v "$OUT":/out "$GRYPE_IMAGE" "docker:$image" -q \
         -o "json=/out/$name.grype.json" -o "table=/out/$name.grype.txt" >"$OUT/$name.grype.log" 2>&1 \
       && [ -s "$OUT/$name.grype.json" ]; then
      summarize "$name.grype.json" || echo "   UNAVAILABLE: report could not be read"
      grep -E ' (Critical|High) ' "$OUT/$name.grype.txt" | head -40 || true
    else
      echo "   UNAVAILABLE: the scanner did not finish (see $name.grype.log)"; tail -3 "$OUT/$name.grype.log" 2>/dev/null | sed 's/^/   /' || true
    fi
  done
  echo "== done. This step reports; it does not fail on findings."
}

case "$MODE" in
  sbom) cmd_sbom;;
  scan) cmd_scan;;
  *) sed -n '2,19p' "$0"; exit 2;;
esac
