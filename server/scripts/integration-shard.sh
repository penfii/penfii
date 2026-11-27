#!/usr/bin/env bash
# Derive the partition from libtest itself, including ignored tests. The sorted list is
# independent of filesystem order and new tests are automatically part of one shard.
set -euo pipefail
index="$1"
count="$2"
export LC_ALL=C
listing="$(cargo test --locked -p core-api --test integration -- --list --format terse)"
mapfile -t names < <(sed -En 's/: (test|benchmark)$//p' <<< "$listing" | sort)
((${#names[@]} >= count))
printf '%s\n' "${names[@]}" | sort -cu
selected=()
excluded=()
for i in "${!names[@]}"; do
  if ((i % count + 1 == index)); then
    selected+=("${names[i]}")
  else
    excluded+=(--skip "${names[i]}")
  fi
done

printf 'Integration shard %s/%s (%s of %s tests):\n' "$index" "$count" "${#selected[@]}" "${#names[@]}"
printf '  %s\n' "${selected[@]}"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT
if ((index == 1)); then
  # All other binaries and the doctests run once. Exact matching keeps a skip from
  # also excluding any test whose name merely contains the selected integration name.
  cargo test --locked -- --format pretty --exact "${excluded[@]}" | tee "$log"
else
  cargo test --locked -p core-api --test integration -- --format pretty --exact "${selected[@]}" | tee "$log"
fi

# Keep only names in the integration suite. Shard 1 also reports unit, contract and doc tests.
# Do not deduplicate the observed names: an unexpected second execution must fail the comparison.
observed="$(sed -En 's/^test ([^[:space:]]+) \.\.\. .*/\1/p' "$log" |
  grep -Fx -f <(printf '%s\n' "${names[@]}") | sort)"
diff -u <(printf '%s\n' "${selected[@]}") <(printf '%s\n' "$observed")

# CI uploads the observed test names, including ignored tests, after a successful run.
if [[ -n "${INTEGRATION_SHARD_PROOF:-}" ]]; then
  mkdir -p "$INTEGRATION_SHARD_PROOF"
  printf '%s\n' "${names[@]}" > "$INTEGRATION_SHARD_PROOF/all.txt"
  printf '%s\n' "$observed" > "$INTEGRATION_SHARD_PROOF/selected.txt"
fi
