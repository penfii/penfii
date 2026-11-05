# Development

How to run Penfii from source, run the checks, and change the schema or the API. For the shape of the code, see [Architecture](architecture.md); for how to submit a change, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Prerequisites

- **Rust**, in the exact version pinned by `server/rust-toolchain.toml` (rustup installs it on first use).
- **Node.js 22** and `npm`. CI uses 22.23.3 (the version of the web image); the tests are known to behave differently on other versions.
- **Docker**, for the dependencies below and for the full test suite.
- `make`, and `python3` (used by the third-party notices check).

## Run the server

The server needs PostgreSQL, S3-compatible storage and OpenSearch with the `nori` plugin. `make run` expects three containers with these names. The credentials are for local development only.

```sh
docker run -d --name penfii-pg --restart unless-stopped \
  -e POSTGRES_DB=penfii -e POSTGRES_USER=penfii -e POSTGRES_PASSWORD=penfii \
  -v penfii-pg:/var/lib/postgresql/data -p 127.0.0.1:5432:5432 postgres:17.11-bookworm

docker run -d --name penfii-minio --restart unless-stopped \
  -e MINIO_ROOT_USER=penfii -e MINIO_ROOT_PASSWORD='<choose one, 8+ characters>' \
  -v penfii-minio:/data -p 127.0.0.1:9000:9000 -p 127.0.0.1:9001:9001 \
  pgsty/silo:RELEASE.2026-09-16T00-00-00Z server /data --console-address :9001

cd server
make opensearch     # builds docker/opensearch once and starts it on 127.0.0.1:9200
make run            # core-api on 127.0.0.1:8080
```

`make run` reads the object storage credentials from the running `penfii-minio` container, sets development values for the required limits, applies the migrations to the database in `DATABASE_URL` (default `postgres://penfii:penfii@127.0.0.1:5432/penfii`), and turns on `PENFII_DEV_RETURN_INVITE_TOKEN` and `PENFII_DEV_SEED`. If a file `server/.env.local` exists (it is git-ignored) it is loaded first, for settings such as an external sign-in provider.

**The development seed.** In a debug build, with `PENFII_DEV_SEED=1`, the server creates a demo workspace (`OurSpace`) with `alice@ourspace.dev` as owner and three more members, and a second workspace, all with the password `1234`. Set `PENFII_DEV_SEED=0` to skip it. A release build never seeds.

**Without the seed**, create the first account once:

```sh
export DATABASE_URL=postgres://penfii:penfii@127.0.0.1:5432/penfii
cargo run -p admin-cli -- bootstrap \
  --email owner@example.com --password 'choose-a-long-password' \
  --display-name Owner --workspace-name Penfii --time-zone Asia/Seoul
```

### Resetting a development database

The migration history is recorded in `penfii._sqlx_migrations`. A database made by a development build from before the schema was consolidated into `0001_initial.sql` has tables but no history, and the server refuses to start on it. To start over, recreate the database, or:

```sh
psql "$DATABASE_URL" -c 'DROP SCHEMA penfii CASCADE'
```

then start the server again. Also delete the matching `<prefix>-*` OpenSearch indices (see [Operations](operations.md#search-index-prefix)); the index records which database made it.

## Run the web app

```sh
cd client
npm ci
npm run dev        # http://127.0.0.1:5173
```

The dev server proxies `/v1` (including WebSocket upgrades), `/oauth`, `/mcp`, `/.well-known`, `/healthz` and `/readyz` to `http://127.0.0.1:8080`. API calls use relative URLs, so the client and the server share one origin from the browser's point of view and there is nothing to configure. `PENFII_ALLOWED_ORIGINS` defaults to the dev server's origins.

A build with `VITE_PREVIEW_CONTENT=1` fills some lists with demo content for screenshots. A real workspace never shows it; do not use it in production.

## Run the whole stack from source

The Docker Compose stack in `docker/` builds the server and web images from this checkout; see [Self-hosting](self-hosting.md). Use it to test the image build, the Caddy configuration or the stack as an operator sees it.

## Checks

Run the checks of the part you changed before you open a pull request. CI (`.github/workflows/ci.yml`) runs all of them.

### Server (`server/`)

```sh
make help          # lists the targets
make tools         # once: the pinned cargo-deny and cargo-shear
make quick         # rustfmt check, clippy (warnings are errors), licenses, unused dependencies, notices; no containers
make deny          # cargo-deny: RustSec advisories, licenses, banned and unknown sources (needs the network)
make integration   # the whole test suite in throwaway PostgreSQL 17 and OpenSearch containers
make check         # quick + deny + integration: what CI runs
```

The integration tests are one binary (`server/tests/integration/`). Each test is a `#[sqlx::test]` that gets a database of its own, created on the server named by `DATABASE_URL` and dropped when it passes; search tests use OpenSearch indices whose prefix is derived from that database. They need a server that allows `CREATE DATABASE`, so **never point them at a database you care about**. `make integration` (and `make check`) start their own containers on random ports and remove them afterwards. `make test` runs against a server you name with `DATABASE_URL` and `PENFII_TEST_OPENSEARCH_URL` and refuses port 5432.

### Client (`client/`)

```sh
make help
npm ci                  # or: make install
make quick              # typecheck, lint, knip, tests and license check with your local Node
make check              # what CI does, in a Node 22 container on a clean copy: npm ci, licenses, typecheck, lint, knip, tests, build, npm audit (needs Docker)
npm run contract:check  # or: make contract (see "Changing the API")
```

There is no formatter. Strict `tsc` is the main check; ESLint (`client/eslint.config.js`) keeps a few bug-class rules (rules of hooks, floating or misused promises, TanStack Query) and Knip (`client/knip.jsonc`) fails on unused files, exports and dependencies, including code that only tests use. Tests run under jsdom and must not open real sockets or networks.

### Repository-level

- The Compose file is validated with `docker compose --env-file docker/.env.example -f docker/compose.yaml config --quiet`, and CI also builds the three images.
- Third-party notices are generated: `make notices` in `server/` and in `client/` regenerates `THIRD-PARTY-NOTICES.md` after `Cargo.lock` or `package-lock.json` changes. `make quick` in `server/` fails when the file is out of date.

## Changing the schema

The schema is `database/migrations/*.sql`, compiled into the server by `sqlx::migrate!` (`server/crates/core/src/migrate.rs`) and applied once each, in order, at start-up.

- Add the next numbered file (`database/migrations/0002_<what>.sql`). Never edit a file that was already applied or released: its checksum is recorded and the server refuses to start on a changed file.
- Each file runs in a transaction, so it needs no `IF NOT EXISTS` guards and may use `DO $$` blocks and functions. Qualify every name with the `penfii.` schema. A change that needs existing rows converted does it in the same file.
- `server/tests/integration/migrations.rs` asserts the list of recorded versions; update it when you add a file. The test suite applies the whole history to an empty database, so a broken file fails the suite.
- `0001_initial.sql` is the whole schema of v0.1.0 as one file; later changes are new files next to it.

## Changing the API

`server/api/openapi.json` is maintained by hand and is the reference for the client. A test (`server/tests/contract.rs`) fails when a route and the file disagree, or when a type in `server/crates/contracts` and the schema of the same name disagree on properties, nullability or value kinds.

The client keeps a copy at `client/packages/contracts/openapi.json` and its types in `client/packages/contracts/src/index.ts`. After changing the server's file, run `npm run contract:sync` in `client/` (it copies `server/api/openapi.json`, or the file or URL in `$PENFII_OPENAPI`), then let `npm test` show where the types disagree. A field that only the client has belongs in the exception tables at the top of `client/packages/contracts/src/openapi.test.ts`, with the reason. CI fails when the copy differs from the server's file.

## Conventions

- **Permissions are decided in the server**, from PostgreSQL, on every request. Do not add a cache, or a client-side check that replaces one.
- **Text is translated.** Every user-visible string in the client is a key in `i18n/messages.ts` or a module's `messages.ts`, with both a Korean and an English value.
- **Configuration.** Document a new environment variable, with its default, in [Configuration](configuration.md).
- **No secrets** or real personal data in code, tests or fixtures.
- **Dependencies** must be compatible with GPL-3.0-only: `make licenses` checks Rust crates against `server/deny.toml` and npm packages with `client/scripts/check-licenses.mjs`.
