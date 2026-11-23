The five migration files that development builds between v0.3.1 and the next release had in place of
`database/migrations/0003_after_v0_3_1.sql`, byte for byte (commit dc7b13a). They are not migrations of
this build: `tests/integration/migrations.rs` applies them to make the history such a build recorded,
and checks that `admin-cli migrate` turns it into the consolidated 0003. Never edit them: their
SHA-384 checksums are the ones `server/crates/core/src/migrate.rs` (`SPLIT_0003`) recognises.
