The two migration files that development builds between v0.4.1 and the next release had in place of
`database/migrations/0004_after_v0_4_1.sql`, byte for byte as they were applied. They are not migrations of
this build: `tests/integration/migrations.rs` applies them to make the history such a build recorded, and
checks that `admin-cli migrate` turns it into the consolidated 0004 (and keeps the rows). Never edit them:
their SHA-384 checksums are the ones `server/crates/core/src/migrate.rs` (`SPLIT_0004`) recognises.
