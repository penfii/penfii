# Changelog

## 0.3.1

A maintenance release: search recovery and English matching, a restore rehearsal, and user and contributor documentation. Back up before you upgrade.

### Upgrading from 0.3.0, 0.2.2, 0.2.1 or 0.2.0

- No database migration, no API change and no new or changed setting. Check out `v0.3.1` and rebuild with the same `docker/.env` (see [Self-hosting](docs/self-hosting.md#from-v020-v021-v022-or-v030-to-v031)); `db-setup` finds the schema current.
- The search mapping version is now 2. On its first start core-api builds every search index again from PostgreSQL in the background; search keeps answering from the old indices until each new one is ready. OpenSearch needs room for both generations meanwhile, and `admin-cli search-status` shows when every kind is `READY`.

### Added

- `docker/scripts/restore-rehearsal.sh`: a backup of the running Compose stack (PostgreSQL dump and file store archive from the same moment, with a manifest) and a rehearsal of restoring it into a second Compose project with mail, push and MCP switched off, then checking it. See [Self-hosting](docs/self-hosting.md#restore-rehearsal).
- User guides in English and Korean (`docs/user-guide.md`, `docs/user-guide.ko.md`), an entry point for API integrators (`docs/api.md`) and the release flow in `docs/development.md`.
- English word forms (`reports` and `reported`, `deploying` and `deployed`) now match in search through an additional light-stemming field (search mapping version 2; see Upgrading).

### Fixed

- Search after a restore or a lost search volume: when PostgreSQL named a live index that OpenSearch no longer had, `admin-cli search-reindex` failed with `search_alias_index_not_found_exception` and search found nothing for that kind. core-api now notices it (also when a write in the meantime made OpenSearch create an index of that name on the fly, without Penfii's mapping) and builds the kind again.
- A rebuild whose index under construction was lost halfway (OpenSearch lost its data, or the index was deleted) failed for ever with `search_count_mismatch`, because it resumed from a checkpoint into an empty index. It now starts over.

### Development

- Integration tests for search index recovery: a lost OpenSearch, a single lost index, an index made up by a write, and a rebuild lost halfway.
- End-to-end: `tests/search-quality.spec.ts` checks English word forms, Korean particles, an exact key, a project filter and an outsider in the real app. `tests/multi-user.spec.ts` follows the v0.3.0 Projects page (My projects by default, other Projects under All projects).

## 0.3.0

A feature release. Penfii is still early software: back up before you upgrade.

### Upgrading from 0.2.2, 0.2.1 or 0.2.0

- No database migration and no new or changed setting. Check out `v0.3.0` and rebuild with the same `docker/.env` (see [Self-hosting](docs/self-hosting.md#from-v020-v021-v022-or-v030-to-v031)); `db-setup` finds the schema current.
- An MCP client that sends a client credential to the token endpoint is now refused with `401 invalid_client` (see Fixed); before, the credential was ignored.
- A Ticket whose parent is archived can no longer be moved to a State that is not Completed or Canceled: the request is refused with `409 parent_archived`. Unarchive the parent first.

### Added

- Bulk archive from the Board and the Table: Board columns have a three-state check, the bar says how many of the chosen Tickets can really be archived and why the others cannot, and archiving asks first and can be undone. The bulk change `ARCHIVE` takes an optional `onlyFinished`; with it, a Ticket that is no longer Completed or Canceled when its turn comes is refused for that Ticket alone (`not_finished`), and the rest are archived. Without it the bulk archive behaves as before.
- A month calendar in the Project Timeline beside the Gantt. A due date is a flag on its day; the planned period is a named bar shown on request. Crowded days list the rest behind More, a phone gets a day-by-day list, and Tasks and Subtasks with neither a due date nor a drawable plan are listed under No date yet, following the same filter as the month. The calendar is a table with weekday headers and a full date for screen readers in every day cell.
- Gantt and Calendar share one day in the address (`date=YYYY-MM-DD`); switching keeps the place, and the Gantt writes back the day in its centre once it stops moving. An old `month=` link still opens its month.
- My projects (the default Projects page, favorites first), All projects with search and a participation filter, and All teams. A Project switcher on the Project name keeps the section; on a phone it shows as a chip.
- From a member card or a Team page, the Team can be found in the org chart. Member search keeps its text when going back.

### Changed

- The Work List is now the Table's compact density. Layout, density and columns are each person's own display, kept per account in this browser, and no longer mark a shared View as changed.
- The Projects navigation shows shortcuts and favorites instead of every Project and Team; the Teams navigation puts the org chart, all Teams and all members above the person's own Teams.
- A parent's panel and child list say how many of its children are open again after it was completed. Reopening a child never changes the parent.
- Creating a ticket keeps what was typed: closing the panel (Esc, Close, Cancel, Back) keeps a draft in this browser for 14 days, per person, Workspace, Project and where Create was opened, with "Reopen" in the notice; opening the same Create again restores it, with "Discard draft". Attachments are not kept, so closing with files attached asks first. A ticket made from a Project screen opens at once; one made from a parent or a chat message is a press away in a notice.
- The profile says what is not saved yet: changed fields are marked, a save bar counts them (fixed to the bottom of the screen on a phone), and leaving with changes asks "Keep editing / Discard and leave / Save and leave". A profile changed elsewhere (the status menu, a photo, another device) no longer replaces what is being typed; the same field changed on both sides shows both values, and Save reads the profile again first and does not overwrite a newer value silently.
- Deleting a document says what goes with it before Delete is enabled: the subpages deleted with it, the ones kept (and moved up) because they cannot be managed, and that a deleted document cannot be restored. Nothing is deleted when that cannot be checked or when the structure changed since; the result notice counts what was really deleted.
- The main chat column reads in a centred block at most 800px wide on wide screens, and the text typed in the composer starts where message text starts. A reader at the bottom stays there when a thread opens or the window narrows.
- A new channel's name is checked on Create and explained under the field in the screen's language; Create sends one request however often it is pressed, and an answer that did not arrive says the channel may exist instead of inviting a retry.
- Search with no results lists the conditions in effect with ways to remove them; changing the kind keeps the text and the Project state, also on a phone.

### Fixed

- Search in the All tab with `project:` or `team:` left out every ticket that has no channel (and every ticket of a Project without channels); the Tickets tab found them.
- An MCP client whose Client ID Metadata Document prefers a token endpoint authentication method Penfii lacks, but lists `none` among the ones it supports (ChatGPT: `private_key_jwt` preferred, `none` supported), was refused with "This application could not be identified". As the CIMD draft asks, the methods now come from `token_endpoint_auth_methods_supported` when it is present, Penfii picks `none` from them, and the client connects as a public client with PKCE. A document that leaves `none` out, whose singular method is not among its listed ones, that names a shared-secret method or holds `client_secret_expires_at`, both `jwks` and `jwks_uri`, or private key material is refused.
- A token request that authenticates the client (an `Authorization` header, `client_secret` or `client_assertion`) is answered `401 invalid_client`; every client here uses `none`. Before, the credential was ignored.
- A client refused at `/oauth/authorize` is logged (`WARN oauth client refused`) with the reason; before, the log showed only the `400`.
- A child of an archived parent could be reopened, leaving open work under an archived Ticket. Every path (State change, drag, bulk State) now refuses it with `parent_archived`, and the child's panel explains why and offers the parent's Unarchive to a Project admin.
- A Ticket reopened while a bulk archive of finished work was running could be archived anyway. The screens now send `onlyFinished`, and the server checks the State under the archive's own row lock.
- Changing a child seen only in its parent's child list did not refresh the parent's progress.

### Security

- `lettre` 0.11.24 (outgoing e-mail) fixes six advisories in 0.11.23: RUSTSEC-2026-0560 to 0565 (STARTTLS response injection, missing dot-stuffing, panics on some EHLO names and display names, a recipient address that could change, unbounded buffering of SMTP replies).

## 0.2.2

A fix release. Penfii is still early software: back up before you upgrade.

### Upgrading from 0.2.1 or 0.2.0

- No database migration and no new or changed setting. Check out `v0.2.2` and run `docker compose up -d --build` from `docker/` with the same `.env`. `db-setup` finds the schema current.

### Fixed

- The same bulk change sent twice at once (one `Idempotency-Key`) could answer a Ticket as failed (`idempotency_conflict`) although it was changed, once, as asked. The copy that comes second now answers with what the first recorded.

### Development

- An integration test of two first saves of one Draft at once expected both to be stored, but the older session is rightly refused when the newer one lands first; it failed now and then in CI.

## 0.2.1

A fix release. Penfii is still early software: back up before you upgrade.

### Upgrading from 0.2.0

- No database migration and no new or changed setting. Check out `v0.2.1` and run `docker compose up -d --build` from `docker/` with the same `.env`. `db-setup` finds the schema current.

### Fixed

- Creating or replacing an incoming webhook secret after the "Confirm it is you" step closed the webhook dialog, so the secret, shown only once, was lost. A dialog opened from inside another no longer closes its parent when it closes or when Escape is pressed.
- Revoking an incoming webhook or replacing its secret now asks first; both stop the current secret at once.
- A person removed from a Project (direct role removed, Team unlinked or its participation ended, removed from the Team) who read its channels only through that participation kept the channel open on screen, and messages written there were refused as "Not sent". Their sockets are now closed and their channel list read again, so the page says they no longer have access.
- An open Ticket did not show a change made while its Project's live connection was being opened (the Board beside it did). The first connection now reads the Project's open Tickets again.
- The header of a private Project channel counted and pictured the Project's members instead of the channel's.
- The chat sidebar's "Move to section" menu could run past the bottom of the window when the window became shorter while it was open.

### Development

- End-to-end tests for several people at once on Chromium, Firefox and WebKit (`tests/multi-user.spec.ts`, opt-in), and for invitations and password reset by e-mail, recovery, a Ticket's archive and trash, webhooks, the Web Push switch and the phone layout (`tests/release-flows.spec.ts`). See [Development](docs/development.md#end-to-end-tests-clientappse2e).
- The verification of this release: [docs/audits/v0.2.1-release-audit.md](https://github.com/penfii/penfii/blob/v0.2.1/docs/audits/v0.2.1-release-audit.md).

## 0.2.0

Penfii is still early software: the HTTP API, the database schema and the settings can change between 0.x releases. Back up before you upgrade.

### Upgrading from 0.1.0

- **A v0.1.0 database upgrades in place.** `0001_initial.sql` is the file v0.1.0 shipped, unchanged, and every schema and data change since is `0002_after_v0_1_0.sql`, applied by `admin-cli migrate` (the Compose `db-setup` service). It also removes the channel memberships of people who no longer take part in the channel's Project, and revokes all but the newest open invitation per e-mail address. A database made by a development build between v0.1.0 and this release (migration history `0001` to `0008`) cannot take this history and has to be recreated.
- **The database runs as two roles, and core-api no longer migrates.** The new one-shot `db-setup` service makes a schema owner and a runtime role (`admin-cli db-provision`) and applies the migrations (`admin-cli migrate`) before core-api starts; core-api and `admin-cli` connect as the runtime role and refuse to start on a schema behind the build. `docker/.env` needs `PENFII_DB_OWNER_PASSWORD` and `PENFII_DB_RUNTIME_PASSWORD`; for a v0.1.0 volume also set `PENFII_DB_OWNER_USER=penfii` and `PENFII_DB_OWNER_PASSWORD` to `POSTGRES_PASSWORD`. See [Self-hosting](docs/self-hosting.md#from-v010-to-v020).
- **Settings that cannot be read now stop the start.** Before, a value that was set but wrong was ignored or replaced by a default: a limit such as `PENFII_FILE_MAX_BYTES=100MB`, `PENFII_WEBHOOK_KEK`, the day counts of the trash and MCP retention, the MCP lifetimes, and `PENFII_REAUTH_WINDOW_SECONDS` became "not set", a `PENFII_MCP_DEFAULT_POLICY` that was not `read_only` or `closed` (a typo included) became `standard`, which allows writes, and a switch other than its one accepted spelling was off. core-api now refuses to start and names the variable. Empty values still count as not set. Check `docker/.env` (and any other environment) before updating; see [Configuration](docs/configuration.md#how-the-server-reads-its-settings).
- **Settings that belong together must be set together.** The four `PENFII_S3_*` settings, `PENFII_OPENSEARCH_USER` with `_PASSWORD`, and a sign-in provider's client id with its secret used to switch the feature off quietly when one was missing; now one without the others stops the start. A Microsoft provider that is multi-tenant and has no `PENFII_AUTH_MICROSOFT_ALLOWED_TENANTS`, and a generic OIDC provider listed without its issuer or pair, are refused too (they were skipped with a warning).
- **Switches take `1`/`true`/`yes`/`on` and `0`/`false`/`no`/`off`** in any case, everywhere (`PENFII_COOKIE_SECURE=true` now works); other words stop the start. An empty `PENFII_ALLOWED_ORIGINS` now means "not set" (the dev server origins), as for every other setting, instead of "no extra origin".
- **A JSON request body that names the same key twice in one object, anywhere in it (inside a free-form value too), is refused with `400 validation`.** The last one used to win silently. Handlers now read the body straight into their type, so the wording of a JSON error can differ (it may carry a line and column); the code and status are the same. The conversation commands (messages, reactions, read state), whose idempotency key is a hash of the body as sent, still read the body as a document, with the same refusal of a repeated key.
- `PENFII_DEV_RETURN_INVITE_TOKEN` is removed: the inviter always gets the invitation link.
- Channel access follows the current Project participation. A person who stops taking part in a Project (grant removed, Team unlinked or left) loses its channels, and taking part again restores none.

### Added

- Incoming webhooks that post into a channel as an integration actor, with a URL secret or an HMAC signature (`PENFII_WEBHOOK_KEK` for the signature mode).
- Ticket archive, archiving a Sprint's finished work when the Sprint is completed (now or at a chosen time), bulk archive and trash on the Board, and an automatic trash purge (`PENFII_TRASH_RETENTION_DAYS`, off by default).
- Optional e-mail (`PENFII_SMTP_*`): invitation links are mailed, and people reset a forgotten password from the login page. One open invitation per address; inviting again issues a new link.
- Web Push (`PENFII_VAPID_*`, `admin-cli vapid-keys`): mentions, direct messages, replies in followed threads, Task assignments and due reminders reach the browser while Penfii is closed. The push settings show each device's last delivery.
- A phone layout (640 px and below) with tabs, sheets, and a list view of Work with bulk changes.
- Recovery for a Project without an active admin and a private channel without an active manager; removing a member or disabling an account warns which ones lose their last one.
- Private channel management for Project admins without reading the channel.
- `admin-cli`: `db-provision`, `migrate`, `vapid-keys`, `create-user` into an existing workspace, `workspace-list`, `workspace-add-member`, `workspace-set-role`. Passwords are read from a hidden prompt or `--password-stdin`.
- `docker/scripts/restore-check.sh`, a restore check against a scratch server, and backup/restore notes for the two roles.

### Changed

- `/readyz` also reports the event consumers' backlog (`checks.workers`, `receipts`) and is `degraded` when a delivery waits more than 5 minutes or is `DEAD`; the status code stays `200`. A panic in a background worker's loop ends core-api so the container is restarted; a panic inside one event or pass is that unit's failure (see [Operations](docs/operations.md#background-workers)).
- Board sockets now carry archive, trash, restore and purge of a ticket and a Sprint's archive; a request handler that panics answers `500 internal`.
- A lock, deadlock or pool timeout is answered `503 db_unavailable` instead of `500 internal`.
- One result notice (with Undo where it applies) for every flow; a reworked ticket panel; Administration split into pages; save states and Retry for documents.
- Ticket descriptions use the documents editor (slash menu, mentions, references).
- Offline, the web app only reads: no write is queued or replayed.
- The first load of the web app is about 0.7 MB instead of 1.6 MB.

### Fixed

- Transactions hold only database work; uploaded files are deleted after the commit through the outbox, never inside a transaction.
- External event consumers are fenced by their lease, so a stale worker cannot overwrite a newer result.
- Push subscriptions on Safari and Home Screen apps are no longer ended by the browser after notifications that were not shown.
- Focus, IME Enter and dialog fixes in the web app.

## 0.1.0

The first public release. Penfii is early software: the HTTP API, the database schema and the settings can still change between 0.x releases, and there is no tested upgrade path. See [Known limitations](docs/known-limitations.md).

Included: chat, projects and tickets, docs, search (OpenSearch with `nori`), the attention features (Inbox, Watch, Later, Reminders), password and external sign-in, the optional MCP gateway, administration and the `admin-cli` operator tool, and a Docker Compose self-hosting stack (PostgreSQL 17). See the [README](README.md) for what each covers.
