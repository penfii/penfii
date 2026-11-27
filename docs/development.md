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

Its PostgreSQL is not published on the host, as in a real installation. Open a shell in it with `docker compose exec postgres psql -U penfii penfii`. To reach it from a tool on your machine, put a `docker/compose.override.yaml` next to the Compose file (it is git-ignored and Compose reads it on its own) with a port other than 5432, which `penfii-pg` above uses:

```yaml
services:
  postgres:
    ports: ["127.0.0.1:55432:5432"]
```

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

**Several people at once.** `tests/multi-user.spec.ts` signs five people in at the same time, each in a browser context of their own and on different engines: a workspace owner who administers the Project (Chromium), a member in the private channel (Firefox), a member outside it (WebKit), a member outside the Project (WebKit) and a workspace admin outside the Project (Chromium). It checks what each of them sees, what is refused to them, and what happens when two of them act at the same moment (chat, ticket fields, title, description, a document, archive, channel membership, Project participation, sign-out). It is opt-in, needs its own accounts and all three engines, so run it in the official image:

```sh
E2E_WORKSPACE_ID=<id> apps/e2e/seed-multi-user.sh   # once: mu-owner, mu-admin, mu-member, mu-member2, mu-outsider @example.test
docker run --rm --user "$(id -u):$(id -g)" --network host --ipc=host -v "$PWD":/work -w /work/apps/e2e -e HOME=/tmp \
  -e E2E_MULTI_USER=1 -e E2E_BASE_URL=http://127.0.0.1:8080 mcr.microsoft.com/playwright:v1.65.0-noble \
  npx playwright test tests/multi-user.spec.ts
```

Every action, console message, refused request and socket error is written with the person, the engine and the time to `apps/e2e/test-results/multi-user/*.jsonl` (`E2E_MU_LOG` names another file); match it against `docker compose logs core-api` by time. `E2E_MU_ENGINES=owner=webkit,member=chromium,member2=firefox` moves people to other engines, to tell an engine's problem from the application's. The findings MU-1 to MU-4 named in its comments are explained in [the v0.2.1 audit](https://github.com/penfii/penfii/blob/v0.2.1/docs/audits/v0.2.1-release-audit.md).

**Mail, recovery and the phone layout.** `tests/release-flows.spec.ts` covers what the other specs leave out: an invitation and a password reset through the mailed link, recovering a private channel nobody manages, a Ticket's archive, trash, restore and purge, a channel's incoming webhook (with the sign-in check in between), the Web Push switch, the phone layout and a menu whose trigger leaves the window. The mail tests need an SMTP catcher the stack sends to: add a [Mailpit](https://mailpit.axllent.org/) service to the stack, point `PENFII_SMTP_HOST`/`PENFII_SMTP_PORT` at it with `PENFII_SMTP_SECURITY=none`, and give the tests its HTTP API as `E2E_MAILPIT=http://127.0.0.1:8025`; without it they are skipped. The Web Push test is skipped while the stack has no VAPID key. `E2E_EMAIL_MEMBER` (a plain MEMBER) runs the read-only Channel Settings check in `channel-ticket-ux.spec.ts`.

**Two-step verification, e-mail change, removal and rejoin, password change.** `tests/security-flows.spec.ts` drives them as a person does, with a TOTP code computed from the key the setup screen shows: set up MFA with a wrong and then a right code, sign in with the password alone (refused, no session), with a wrong code, a right code and a recovery code (once only), turn it off; change the e-mail address through the mailed link (the old address keeps working until then, the link works once, the old address is told) and change it back; remove a member in the admin screen, see the person shut out at once and invited back through the mailed link without the old private channel; change the password and see the other devices end while this one stays, and sign a device out from the list. Seed its three accounts with `E2E_WORKSPACE_ID=<id> apps/e2e/seed-security.sh`; the mail parts need `E2E_MAILPIT` as above, and the mails are in the instance's default language. The server allows 5 e-mail change requests an hour per account and 3 an hour per target address, and one run makes two, so run that test at most twice an hour against the same stack (a 429 is the limit, not a failure of the feature).

The accounts and their public password are test data (`apps/e2e/support/accounts.ts`, override with `E2E_EMAIL_A`, `E2E_EMAIL_B`, `E2E_PASSWORD`). Each test makes its own Project, channel or document, so runs can be repeated; a test fails on any console error, uncaught page error or 5xx answer it did not announce (`allowProblem`). Wait for a condition, never a fixed time.

### Repository-level

- The Compose file is validated with `docker compose --env-file docker/.env.example -f docker/compose.yaml config --quiet`, and CI also builds the three images.
- Third-party notices are generated: `make notices` in `server/` and in `client/` regenerates `THIRD-PARTY-NOTICES.md` after `Cargo.lock` or `package-lock.json` changes. `make quick` in `server/` fails when the file is out of date.

## Changing the schema

The schema is `database/migrations/*.sql`, compiled into the server by `sqlx::migrate!` (`server/crates/core/src/migrate.rs`) and applied once each, in order, by `admin-cli migrate` (the migration credential). core-api does not migrate; it refuses to start when the recorded history is behind this build.

- Add the next numbered file (`database/migrations/0005_<what>.sql`). Never edit a file that was already applied or released: its checksum is recorded and the server refuses to start on a changed file. One file, one change.
- A release ships the changes made since the previous one as one file (`0002_after_v0_1_0.sql`, `0003_after_v0_3_1.sql`, `0004_after_v0_4_1.sql`), so a new installation and an upgraded one run the same SQL. When the files of a development cycle are merged into one before the release, a database that already applied them has a history the new build does not know: recreate it, or teach `admin-cli migrate` to rewrite that exact history, as `consolidate_split` in `server/crates/core/src/migrate.rs` does for the split 0003-0007 of the v0.3.1 cycle and the split 0004-0005 of the v0.4.1 cycle (one `Split` entry each; it recognises the files by their checksums, checks the objects they made, and refuses anything partial).
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
- `0001_initial.sql` is the whole schema of v0.1.0 as one file, byte for byte as released (a v0.1.0 database has its checksum recorded). `0002_after_v0_1_0.sql` is every change made before v0.2.0, so a v0.1.0 database upgrades in place. Later changes are new files next to them. `migrations.rs` checks that a database with the v0.1.0 history and data upgrades and keeps its rows.

## Changing the API

To use the API from another program, start with [Using the API](api.md). `server/api/openapi.json` is maintained by hand and is the reference for the client. A test (`server/tests/contract.rs`) fails when a route and the file disagree, or when a type in `server/crates/contracts` and the schema of the same name disagree on properties, nullability or value kinds.

The client keeps a copy at `client/packages/contracts/openapi.json` and its types in `client/packages/contracts/src/index.ts`. After changing the server's file, run `npm run contract:sync` in `client/` (it copies `server/api/openapi.json`, or the file or URL in `$PENFII_OPENAPI`) and then `npm run contract:types` (it rewrites `client/packages/contracts/src/generated.ts`, the TypeScript types of that copy, and fails on a key written twice in one object), then let `npm run typecheck` and `npm test` show where the types disagree. A model whose schema is complete is an alias of its generated type in `index.ts`, so a server change to it is a type error; the others are still written by hand until their schemas say what the server sends (every required field, closed sets of values as `enum`). `make check` fails when `generated.ts` is not what the copy produces. A field that only the client has belongs in the exception tables at the top of `client/packages/contracts/src/openapi.test.ts`, with the reason. CI fails when the copy differs from the server's file.

## Releases

What follows is what the repository itself shows about how Penfii is released. It says nothing about who may merge or tag, and a change that only edits documentation never tags, pushes or deploys anything.

**Versions and tags.** Releases are numbered `MAJOR.MINOR.PATCH` and are all `0.x` for now: the API, the schema and the settings can change between them ([Known limitations](known-limitations.md#stability)). A release is an annotated Git tag `vX.Y.Z` (`v0.1.0` to `v0.3.1` so far) on a commit of `main`; from `v0.2.0` on that commit is named `Release vX.Y.Z`. The public repository on GitHub is a mirror that carries `main` and those tags (the working branch `dev` is not mirrored). The release notes are the [changelog](../CHANGELOG.md). People install from a tag ([Self-hosting](self-hosting.md#install)).

**What a release commit changes.** Look at any `Release vX.Y.Z` commit (`git show --stat v0.3.0`):

- `CHANGELOG.md` gets a `## X.Y.Z` section: an *Upgrading from …* part first (anything an operator has to do, such as a migration or a setting; "none" is said when none), then *Added*, *Changed*, *Fixed*, *Security* or *Development* as they apply.
- The version is set in the manifest of every crate under `server/apps/` and `server/crates/` and in `Cargo.lock`, and in every `package.json` under `client/` and `package-lock.json`.
- The sentence that names the current release is updated in `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, `docs/README.md`, `docs/known-limitations.md` and `docs/self-hosting.md`, which also gets a *From … to …* section under [Updating](self-hosting.md#updating) and the new tag in its commands.

**Publishing a release (images, install ZIP).** The tag is made and pushed first, as above; publishing never changes it. `.github/workflows/release.yml` is started by hand with the tag (`publish: false` is a dry run) and takes the *tag's* checkout as the only source of what goes into an image or the ZIP's files, so a later commit to `main` (including this tooling) can never end up in a released image. It refuses a tag that is not annotated, not on `main`, whose manifests or changelog do not say that version, or whose commit has no successful CI run. It then builds `ghcr.io/penfii/core-api`, `web` and `opensearch` (`linux/amd64`, labels with version and commit, provenance and SBOM attestation) and pushes only the version tag, never `latest`: a tag that already exists in GHCR is kept, and the run fails if it was built from another commit. After that it pulls the images anonymously, checks the labels, makes the SBOMs and the report-only vulnerability report (`docker/scripts/supply-chain.sh`), installs from the ZIP with the published images and runs the smoke test (`docker/release/smoke.sh`: first account, sign-in, project, Ticket, file upload, restart, data kept), and creates a **draft** GitHub Release with the ZIP, `SHA256SUMS`, the SBOMs and notes drafted from the changelog. A person publishes the draft. A new GHCR package is private: make it public once in the package settings. The pieces are in `docker/release/` (`build-images.sh`, `package.sh`, `check-install-compose.py`, `smoke.sh`, `release-notes.py`) and the ZIP's own files in `docker/install/`; `docker/install/compose.yaml` must stay equal to `docker/compose.yaml` except for the images (CI checks it), so a change to one is made in the other.

**Checks.** A release commit is held to the same checks as any change: `make check` in [Server](#server-server) and [Client](#client-client), and the CI workflow below. A migration that is part of a release is never edited afterwards ([Changing the schema](#changing-the-schema)). There is no down migration, so going back to an older release means restoring a backup ([Backup](self-hosting.md#backup) and [Restore](self-hosting.md#restore)); those procedures are kept there, not repeated here.

**CI.** `.github/workflows/ci.yml` runs on every pull request and on pushes to `dev` and `main`. Its jobs are `server-quick` (rustfmt, clippy, licences, unused dependencies, notices), `server-deny` (RustSec advisories and licences), `server-test` (the whole server suite on PostgreSQL 17 and OpenSearch with `nori`, containing the API contract test), `client-check` (licences, typecheck, lint, knip, tests, build), `client-audit` (`npm audit` of what ships to the browser), `contract` (the client's `openapi.json` equals the server's), `docker` (the Compose file is valid and the three images build), `gitleaks` and `supply-chain` (a CycloneDX SBOM of the source and of the core-api and web images, stamped with the commit and the image id and validated, plus a report-only Grype vulnerability report that never fails the job; both are kept as a build artifact; run it locally with `docker/scripts/supply-chain.sh sbom|scan OUTDIR` after `docker compose build`. It describes the images built from this checkout, not images another pipeline builds for a deployment; the SBOM carries the commit, and the tag only when the script runs on a checkout that has one, because CI does not run on tag pushes). Dependabot proposes cargo, npm, GitHub Actions and Docker base image updates weekly (`.github/dependabot.yml`). The maintainers' original repository is on their own Forgejo server and GitHub carries a mirror of `main` and the release tags, so GitHub Actions is the CI that runs: on a push of `main` it runs once for that commit, and a pushed tag starts nothing. `.forgejo/workflows/ci.yml` is an older copy for a Forgejo runner that no longer exists; it is not in use and nothing depends on it. Deploying a build to the maintainers' development environment is a separate, manual procedure in a private repository, not part of CI. Whether a given mirror push has started the workflow is visible on the repository's Actions page; it is not something a contributor has to set up.

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
