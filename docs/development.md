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

`make run` reads the object storage credentials from the running `penfii-minio` container, sets development values for the required limits, applies the migrations to the database in `DATABASE_URL` (default `postgres://penfii:penfii@127.0.0.1:5432/penfii`), and turns on `PENFII_DEV_SEED`; invitation links are shown in the web app (copy them), and `PENFII_SMTP_*` in `server/.env.local` turns e-mail on. If a file `server/.env.local` exists (it is git-ignored) it is loaded first, for settings such as an external sign-in provider.

**The development seed.** In a debug build, with `PENFII_DEV_SEED=1`, the server creates a demo workspace (`OurSpace`) with `alice@ourspace.dev` as owner and three more members, and a second workspace, all with the password `1234`. Set `PENFII_DEV_SEED=0` to skip it. A release build never seeds.

**Without the seed**, create the first account once:

```sh
export DATABASE_URL=postgres://penfii:penfii@127.0.0.1:5432/penfii
cargo run -p admin-cli -- bootstrap \
  --email owner@example.com --display-name Owner --workspace-name Penfii --time-zone Asia/Seoul
```

The command asks for the password (not shown); in a script use `--password-stdin`. `--password <value>` still works but leaves the password in the shell history and the process list.

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
make check              # what CI does, in a Node 22 container on a clean copy: npm ci, licenses, generated types, typecheck, lint, knip, tests, build, bundle budget, npm audit (needs Docker)
npm run contract:check  # or: make contract (see "Changing the API")
npm run budget          # or: make budget, after `npm run build`: first-load JavaScript size (see below)
make e2e-seed e2e       # end-to-end tests against a running Compose stack (see "End-to-end tests")
```

There is no formatter. Strict `tsc` is the main check; ESLint (`client/eslint.config.js`) keeps a few bug-class rules (rules of hooks, floating or misused promises, TanStack Query) and Knip (`client/knip.jsonc`) fails on unused files, exports and dependencies, including code that only tests use. Tests run under jsdom and must not open real sockets or networks. A test whose mock leaves a query unanswered fails ("Query data cannot be undefined"): make the mock return what the API returns.

**Bundle budget.** `client/scripts/check-bundle-size.mjs` (run by `make check` after the build) fails when the JavaScript the first page needs (the entry and its modulepreloads, plus the larger language catalog) grows past the budgets written in the script, or when tiptap/ProseMirror, shiki or BlockNote are in it again. Keep a library that one screen needs behind a lazy import, and import a small helper from its own file, not from a barrel that also exports an editor.

### End-to-end tests (`client/apps/e2e`)

Playwright drives a real browser against a running Compose stack and its real server; it is the only place where login, sockets, drafts and layout changes are tested together. It is **not** part of `make check` or CI's `client-check`: it needs the whole stack (PostgreSQL, OpenSearch, MinIO, core-api, web) built from the same tree, which the Node-only clean-copy container does not have, and a run changes data in the stack. Run it before merging a change to sign-in, the shell, realtime sync, drafts or the board, and add a scenario there when one of those breaks. It is deliberately a separate step; moving it into a CI job needs a runner that can start the Compose stack, and the job must not gate `deploy` until it has proved stable there.

```sh
cd docker && docker compose up -d --build        # the stack, http://127.0.0.1:8080
cd ../client && npx playwright install chromium  # once: the browser
E2E_WORKSPACE_ID=<id> make e2e-seed             # once: e2e-a@example.test and e2e-b@example.test as owners of that Workspace
                                                  # (on an empty stack, omit it: the script bootstraps one named E2E)
make e2e                                          # E2E_BASE_URL overrides the address
```

**Other browsers.** `E2E_BROWSERS=chromium,firefox,webkit make e2e` runs the same suite in more engines (`npx playwright install firefox webkit`). WebKit needs system libraries that `npx playwright install-deps webkit` installs (it needs root); without them run it in the official image:

```sh
docker run --rm --user "$(id -u):$(id -g)" --network host --ipc=host -v "$PWD":/work -w /work \
  -e E2E_BROWSERS=webkit -e E2E_BASE_URL=http://127.0.0.1:8080 -e HOME=/tmp mcr.microsoft.com/playwright:v1.65.0-noble npx playwright test -c apps/e2e
```

The accounts and their public password are test data (`apps/e2e/support/accounts.ts`, override with `E2E_EMAIL_A`, `E2E_EMAIL_B`, `E2E_PASSWORD`). Each test makes its own Project, channel or document, so runs can be repeated; a test fails on any console error, uncaught page error or 5xx answer it did not announce (`allowProblem`). Wait for a condition, never a fixed time.

### Repository-level

- The Compose file is validated with `docker compose --env-file docker/.env.example -f docker/compose.yaml config --quiet`, and CI also builds the three images.
- Third-party notices are generated: `make notices` in `server/` and in `client/` regenerates `THIRD-PARTY-NOTICES.md` after `Cargo.lock` or `package-lock.json` changes. `make quick` in `server/` fails when the file is out of date.

## Changing the schema

The schema is `database/migrations/*.sql`, compiled into the server by `sqlx::migrate!` (`server/crates/core/src/migrate.rs`) and applied once each, in order, by `admin-cli migrate` (the migration credential). core-api does not migrate; it refuses to start when the recorded history is behind this build.

- Add the next numbered file (`database/migrations/0009_<what>.sql`). Never edit a file that was already applied or released: its checksum is recorded and the server refuses to start on a changed file. One file, one change.
- Each file runs in a transaction, so it needs no `IF NOT EXISTS` guards and may use `DO $$` blocks and functions. Qualify every name with the `penfii.` schema. A change that needs existing rows converted does it in the same file. Before writing a migration that updates or deletes rows, count how many rows it will touch. A change that would lock a large table for a long time is written in batches, or, for an index, outside a transaction.
- `CREATE INDEX CONCURRENTLY` cannot run inside a transaction. Put `-- no-transaction` on the first line of that file. The runner does support it (sqlx 0.8): the same statement is refused in an ordinary file and applied in a no-transaction file. sqlx sends the file as one script, and PostgreSQL runs a multi-statement script as one transaction, so a script that fails does not keep its earlier statements. What does stay is a crash after the script has committed and before its history row is written, and an `INVALID` index left by a failed `CREATE INDEX CONCURRENTLY`. Write the file so a second run is safe (`IF NOT EXISTS`, and drop an invalid index before creating it again). There is no down migration: rolling back a release is restoring a backup ([Self-hosting](self-hosting.md#restore)).
- A foreign key or an index that is added later is decided from a measurement on real data, and the reason is written in the migration's comment.
- `server/tests/integration/migrations.rs` asserts the list of recorded versions; update it when you add a file. The test suite applies the whole history to an empty database, so a broken file fails the suite. It also checks that a no-transaction file keeps a partial apply and can be run again.
- Read columns with `row.try_get("column")?`, never `row.get(..)`: `get` panics when a column is missing or of another type, and `server/clippy.toml` makes clippy refuse it in the server's code (integration tests may use it). A handler that panics anyway is answered `500 internal` (CORS headers included) and the process goes on; see [Operations](operations.md#background-workers) for the workers.
- Some queries are checked at compile time (`query!`, `query_as!`, `query_scalar!` in `identity/throttle.rs`, `identity/reset.rs`, `identity/session.rs` and `events.rs`). The checked data is `server/.sqlx`. After changing one of those queries, or a column it reads, refresh it from a migrated database and commit the result:

```sh
export DATABASE_URL=postgres://penfii:penfii@127.0.0.1:5432/penfii   # already migrated
cargo sqlx prepare --workspace -- --locked
```

`cargo sqlx prepare --check` fails when `.sqlx` is stale. `make integration` runs that check. Builds that set `DATABASE_URL` to a database without the schema (the test runner's empty `penfii` database, the Docker build) set `SQLX_OFFLINE=true` so they use `.sqlx` instead of asking the database. A query assembled with `format!` cannot use these macros; leave it as `sqlx::query`.
- `0001_initial.sql` is the whole schema of v0.1.0 as one file; later changes are new files next to it.

## Changing the API

`server/api/openapi.json` is maintained by hand and is the reference for the client. A test (`server/tests/contract.rs`) fails when a route and the file disagree, or when a type in `server/crates/contracts` and the schema of the same name disagree on properties, nullability or value kinds.

The client keeps a copy at `client/packages/contracts/openapi.json` and its types in `client/packages/contracts/src/index.ts`. After changing the server's file, run `npm run contract:sync` in `client/` (it copies `server/api/openapi.json`, or the file or URL in `$PENFII_OPENAPI`) and then `npm run contract:types` (it rewrites `client/packages/contracts/src/generated.ts`, the TypeScript types of that copy, and fails on a key written twice in one object), then let `npm run typecheck` and `npm test` show where the types disagree. A model whose schema is complete is an alias of its generated type in `index.ts`, so a server change to it is a type error; the others are still written by hand until their schemas say what the server sends (every required field, closed sets of values as `enum`). `make check` fails when `generated.ts` is not what the copy produces. A field that only the client has belongs in the exception tables at the top of `client/packages/contracts/src/openapi.test.ts`, with the reason. CI fails when the copy differs from the server's file.

## Database use

- A transaction holds a pool connection until it commits or rolls back. Inside it, do only database work. Password hashing, HTTP, SMTP, S3 and any other wait happen before the transaction opens or after it commits. Do not take a second connection from the pool while one transaction is open.
- Use the sqlx transaction API. Do not send `BEGIN`, `COMMIT` or `ROLLBACK` as SQL text.
- A time that is stored, or that decides whether a database row is expired, locked or inside a window, comes from PostgreSQL `now()`. A time on an event, a signature or an external call can come from the process clock.
- On shutdown, core-api stops accepting HTTP requests and finishes the ones in flight, stops background workers, flushes co-editing rooms (they use the database), then closes the pool. Each step has a time limit.
- File objects are deleted after the commit that retires the file, by the `storage` event consumer. The request does not wait for the object store. Buckets are assumed not to use S3 Versioning; a versioned bucket can keep older versions, and this cleanup does not remove them.

## Conventions

- **Permissions are decided in the server**, from PostgreSQL, on every request. Do not add a cache, or a client-side check that replaces one.
- **Text is translated.** Every user-visible string in the client is a key in a catalog, with both a Korean and an English value. A language is its own file so that the one not in use is not downloaded: Korean in `i18n/messages.ko.ts` and each module's `messages.ts` (or `*Messages.ts`), English in the `.en.ts` file next to it. A new module catalog is spread into both `i18n/messages.ko.ts` and `i18n/messages.en.ts`.
- **A dialog is a `Dialog` or `Sheet`** from `client/packages/client/src/design/ui` (native `<dialog>` with `showModal`), not a hand-made overlay: that gives focus handling, Escape and an inert page behind it. A dialog opened from a menu item returns focus to something that still exists when it closes.
- **Writes go through a domain hook, not a call written into the screen.** New and changed code follows this; the older direct `api.*` calls move over when their screen is changed for another reason (there is no bulk rewrite).
  - *Where:* next to the domain, in its module (`modules/work/useTicketEditor.ts`, `modules/admin/useAdminAction.ts`), named `use…`.
  - *What it exposes:* `busy` (a second submit while one is running does nothing) and a readable `error` string, never the raw error. A hook that ends with the screen gone must not set state afterwards.
  - *Error text:* mapped once in the hook from the error code, through `ui/errorText` (or the domain's own key map such as `moveErrorKey`), not by `if (code === …)` in each screen.
  - *Invalidation belongs to the domain module:* the hook, or the module's `refresh.ts`, says which queries a write makes stale (`refreshTicket`, `refreshProject`), using the keys in `queryKeys.ts` (a key written inline in a screen fails `queryKeys.test.ts`). A write you made yourself is told to the refresh code (`noteOwnChange`) so its own echo from the socket does not read the same data again.
  - *Optimistic updates* (the Board move): hold the refresh of what is changed (`holdBoardRefresh`), cancel its reads, keep the value before, write the new one, and on failure put the old value back and say why; after success read the server's value once. Nothing optimistic without a rollback.
  - *A call that may need a recent sign-in* goes through `useAdminAction`, which re-asks for the password when the server says so, shows a readable failure and refreshes the lists.
- **Configuration.** Document a new environment variable, with its default, in [Configuration](configuration.md).
- **No secrets** or real personal data in code, tests or fixtures.
- **Dependencies** must be compatible with GPL-3.0-only: `make licenses` checks Rust crates against `server/deny.toml` and npm packages with `client/scripts/check-licenses.mjs`.
