# web-push-native (vendored)

This is `web-push-native` 0.5.0 by Leo Gaskin (https://github.com/leotaku/web-push-native, MIT OR Apache-2.0),
changed only as follows:

- `p256` 0.13 -> 0.14, so the build no longer contains `crypto-bigint` 0.5 (RUSTSEC-2026-0387). No release of
  `web-push-native` or any other Rust Web Push crate depends on `p256` 0.14 yet.
- The `vapid` and `serialization` features (and `jwt-simple`, `serde`) are removed. Penfii signs the VAPID token
  itself (`penfii-core/src/push.rs`); this crate only encrypts (RFC 8291) and builds the request.
- Random bytes come from `getrandom` because `p256` 0.14 uses `rand_core` 0.10.

The encryption code and its RFC 8291 test vector are otherwise unchanged. Drop this directory and the
`[patch.crates-io]` entry in `server/Cargo.toml` once upstream moves to `p256` 0.14.
