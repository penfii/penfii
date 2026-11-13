#!/usr/bin/env bash
# Installs the pinned cargo-deny and cargo-shear (`make tools`, and CI). On x86_64/aarch64 Linux it
# takes the release binary and checks its SHA-256; elsewhere it builds the same version with
# `cargo install --locked`. Bump a version together with its checksums.
set -euo pipefail

DENY_VERSION=0.20.2
SHEAR_VERSION=1.14.0
SQLX_VERSION=0.8.6
BIN="${CARGO_HOME:-$HOME/.cargo}/bin"

has() { command -v "$1" >/dev/null && "$1" --version 2>/dev/null | grep -qwF "$2"; }

fetch() { # name url sha256 path-in-archive
  local tmp
  tmp="$(mktemp -d)"
  curl -fsSL --retry 3 -o "$tmp/a.tar.gz" "$2"
  echo "$3  $tmp/a.tar.gz" | sha256sum -c --quiet
  tar -xzf "$tmp/a.tar.gz" -C "$tmp" "$4"
  install -D -m 0755 "$tmp/$4" "$BIN/$1"
  rm -rf "$tmp"
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    arch=x86_64
    deny_sha=9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f
    shear_sha=9004df4eb031a456026c4326ae0d0ce545321527a67be90f4524c1be729dc40e ;;
  Linux-aarch64)
    arch=aarch64
    deny_sha=995c82be0defc7a025cae49a2aa2644ce8245c9a3318fc4103907c6a285e8c7d
    shear_sha=d83f7c5b02c4c237d0454697c12df041df0bd83ffecc1aba46c6e06315fd47e1 ;;
  *) arch="" ;;
esac

if ! has cargo-deny "$DENY_VERSION"; then
  if [ -n "$arch" ]; then
    name="cargo-deny-$DENY_VERSION-$arch-unknown-linux-musl"
    fetch cargo-deny "https://github.com/EmbarkStudios/cargo-deny/releases/download/$DENY_VERSION/$name.tar.gz" "$deny_sha" "$name/cargo-deny"
  else
    cargo install --locked "cargo-deny@$DENY_VERSION"
  fi
fi
if ! has cargo-shear "$SHEAR_VERSION"; then
  if [ -n "$arch" ]; then
    fetch cargo-shear "https://github.com/Boshen/cargo-shear/releases/download/v$SHEAR_VERSION/cargo-shear-$arch-unknown-linux-musl.tar.gz" "$shear_sha" cargo-shear
  else
    cargo install --locked "cargo-shear@$SHEAR_VERSION"
  fi
fi
# sqlx-cli is the same 0.8 line as the sqlx crate. --locked builds the published lockfile.
# There is no upstream release binary with a checksum; the version pin is the contract.
if ! has sqlx "$SQLX_VERSION"; then
  cargo install --locked sqlx-cli --version "$SQLX_VERSION" --no-default-features --features postgres,rustls
fi
cargo-deny --version
cargo-shear --version
sqlx --version
