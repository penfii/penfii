#!/usr/bin/env bash
# Transfer the exact core-api and web images between GitHub jobs in the same workflow run.
set -euo pipefail
: "${COMMIT:?set COMMIT to the checkout being verified}"

mode="${1:?use save or load}"
bundle="${2:?set the image bundle directory}"
tag="${CI_IMAGE_TAG:-local}"
images=("penfii-ci-core-api:$tag" "penfii-ci-web:$tag")
manifest() {
  printf 'commit %s\n' "$COMMIT"
  for image in "${images[@]}"; do
    id="$(docker image inspect --format '{{.Id}}' "$image")" || return
    printf '%s %s\n' "$image" "$id"
  done
}

case "$mode" in
  save)
    mkdir -p "$bundle"
    manifest > "$bundle/images.txt"
    docker image save "${images[@]}" | gzip -1 > "$bundle/images.tar.gz"
    ;;
  load)
    if [[ ! -s "$bundle/images.tar.gz" || ! -s "$bundle/images.txt" ]]; then
      echo "Incomplete CI image bundle in $bundle" >&2
      exit 1
    fi
    [[ "$(head -n 1 "$bundle/images.txt")" == "commit $COMMIT" ]] || {
      echo "CI image bundle belongs to a different commit" >&2
      exit 1
    }
    docker image load --input "$bundle/images.tar.gz"
    actual="$(manifest)"
    if [[ "$actual" != "$(cat "$bundle/images.txt")" ]]; then
      echo "Loaded CI image IDs differ from the build manifest" >&2
      exit 1
    fi
    printf '%s\n' "$actual"
    ;;
  *) echo "unknown mode: $mode (use save or load)" >&2; exit 2 ;;
esac
