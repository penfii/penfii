# Changelog

## Unreleased

### Added

- **Official Docker install from a release ZIP and prebuilt images** (FEA-154, FEA-155, FEA-156): a release now comes with `penfii-vX.Y.Z-docker.zip` (a `compose.yaml` that pulls `ghcr.io/penfii/core-api`, `web` and `opensearch` at the release tag, `.env.example`, `minio/setup.sh`, a README) and a `SHA256SUMS` file; no source checkout, Rust or Node is needed. The stack is the same as `docker/compose.yaml` (database roles, MinIO app account, networks, non-root read-only containers); a check keeps the two files equal except for the images. Images are `linux/amd64`. They carry OCI labels (version, source commit), a provenance and an SBOM attestation, and the release has CycloneDX SBOMs and a report-only Grype scan. See [Self-hosting](docs/self-hosting.md#install-from-the-release-zip).
- `.github/workflows/release.yml` (started by hand) builds the images from the checkout of an existing release tag, never from newer code on `main`, publishes them without replacing an existing tag, installs from the ZIP with the published images and creates a draft GitHub Release.

## 0.4.1

MCP gateway release, with an updated OpenSearch image. No database migration and no new setting.

### Upgrading from 0.4.0 (or 0.3.1, 0.3.0, 0.2.2, 0.2.1, 0.2.0)

- From 0.4.0 nothing has to be changed: check out `v0.4.1` and run `docker compose up -d --build` with the same `docker/.env`. There is no migration, so `db-setup` finds the schema current. Coming from an older release, follow the 0.4.0 section below as well.
- MCP clients that already have a connection keep the scopes they had. To use the new tools (Docs, chat, plans, ...) a client asks for the new scopes and the person approves them again in the browser.

### Security

- **The bundled OpenSearch is 2.19.8 (was 2.19.2)**, and the image no longer contains the performance analyzer, ML Commons and skills plugins, which Penfii does not use. A vulnerability scan of the image went from 19 Critical and 308 High findings to none Critical and 5 High; those 5 are libraries inside OpenSearch's security plugin (switched off here) and job scheduler. OpenSearch stays reachable only on the internal `data` network. The data volume is compatible (same 2.19 line): `docker compose up -d --build` builds the new image and keeps your indices. Search is derived data, `admin-cli search-reindex` rebuilds it from PostgreSQL if you ever need to.

### Added

- **The MCP gateway covers the work people do in the web app:** Tickets (fields, parents, revisions, trash, archive), Docs (create, save with conflict detection, versions, comments, sharing, templates), Channels, DMs, messages, threads and reactions, search across tickets, docs, messages, files, views and people, Projects, workflow states and labels, boards, views, sprints, milestones, dependencies and reports, Teams, departments, MEMBER invitations, files (up to 256 KiB), Inbox, Later, favorites, drafts, subscriptions and notification settings, audit and trash lists. About 200 tools, listed in [docs/mcp-tools.md](docs/mcp-tools.md). New OAuth scopes (`doc:*`, `conversation:*`, `plan:*`, ...) need a new consent; existing connections keep what they had.
- `workspace_mcp_policy` and `workspace_web_only` tell an AI client what the Workspace allows and which actions stay in the web app.

### Changed

- **The MCP tool list no longer depends on the connection's scopes** (FEA-174): `tools/list` shows what the Workspace policy allows, each tool with the scopes it needs, so a client can find a tool and ask for its permission. Calls are checked as before. An `insufficient_scope` answer now names every missing scope at once and carries `error_description`, and tool descriptors have `securitySchemes` at the top level as well as in `_meta`. A connection with few scopes now receives the whole list.
- **Changing a member's role to or from OWNER or ADMIN now needs a recent sign-in** in the web app (`403 reauth_required` otherwise). Other role changes are unchanged.
- `project_create` is a Write tool (`project:write`), no longer Administrative with `project:manage`: creating a Project is routine management, so the standard Workspace policy allows it.
- MCP calls refused for scope or policy reasons, and calls on a Ticket, Project, Doc or other indirect resource, now record the Workspace they aimed at in the call log and the audit entry.
- Deleting a Ticket or Doc comment through MCP is a Destructive tool (it cannot be undone): the standard Workspace policy leaves it off until an administrator allows it.
- `doc_set_visibility` and `doc_set_participants` report what changed (`change`: added, removed, widened) and MCP results that are lists come back as `{"items": [...]}`.
- The member actions menu on the Workspace admin Members page is no longer cut off by its table (the Make Admin and Remove items were hidden for the last rows).
- Every MCP tool reports a version conflict with the facts that come with it (for example the current revision), as the Doc tools already did, so a client can read again and merge.
- Search through MCP accepts the kinds `message`, `doc`, `file`, `space` and `view` besides `ticket` and `person`.
- **What an MCP client changes names the client everywhere:** each Ticket of `ticket_bulk_update` shows *via &lt;client&gt;* in History and Activity like a single edit, and audit entries made through MCP (invitations, trash, channel members, ...) carry `via: mcp` with the client and grant, shown next to the person in the web app's audit log (`via` in `GET /v1/workspaces/{id}/audit`). Changes made in the web app name no client.
- **Channel names are not unique, and a retry is told apart by its key:** `channel_create` takes `idempotencyKey` and `POST /v1/workspaces/{id}/channels` an optional `Idempotency-Key`; the same key with the same request returns the first channel. The web app sends one, so trying again after no answer does not make a second channel.
- **Boards, My Work and a Ticket's children give each schedule its planned end** (`plannedEndAt`), the same as the Ticket read on its own; it was null in lists.
- A list's planned ends (boards, My Work, the Gantt page) are worked out with a few queries for the whole list instead of one per scheduled Ticket.

## 0.4.0

Security hardening from the v0.3.1 audit (FEA-133, FEA-147). Back up before you upgrade.

### Upgrading from 0.3.1 (or 0.3.0, 0.2.2, 0.2.1, 0.2.0)

- **Database migration 0003** runs in `db-setup` on the first start, in one transaction. It adds e-mail change, external sign-in binding and two-step verification tables. It also removes the Team, Project, Group, Doc share and auditor rights still held by people already removed from a workspace (written to the audit log as `member.access_cleaned`, one entry per person and workspace).
- **web listens on 8080 inside its container** and runs as a non-root user. The published port (`PENFII_HTTP_PORT`) is unchanged. A hand-written Compose file or health check that targets the container's port 80 must change.
- **The Compose networks change:** a private `frontend` network with a fixed range (`PENFII_FRONTEND_SUBNET`) and an internal `data` network. Run `docker compose down` then `up -d` once so Docker creates them again (see [Self-hosting](docs/self-hosting.md#upgrading-to-the-hardened-compose-file)).
- **core-api now trusts `X-Forwarded-For` only from the web container** (`PENFII_TRUSTED_PROXIES` defaults to `PENFII_FRONTEND_SUBNET`, before: every private range). A reverse proxy on another machine must be added to `PENFII_TRUSTED_PROXIES`, or every visitor counts as the proxy's address and shares one sign-in throttle. core-api logs a warning when it sees such a proxy.
- **New settings are optional.** `PENFII_S3_APP_ACCESS_KEY` and `PENFII_S3_APP_SECRET_KEY` (MinIO app account) and `PENFII_MFA_ENCRYPTION_KEY` can stay unset; without the app keys the server keeps the root key pair and `minio-setup` warns.
- **The runtime database role can no longer change or delete audit entries.** This applies after `migrate` or `db-provision`, on installs with separate owner and runtime roles.
- **Changing an e-mail address in the profile now needs a confirmation** sent to the new address, and is not offered without e-mail. `PATCH /v1/me` with a different address answers `409 email_change_requires_verification`.

### Added

- **Optional two-step verification** with an authenticator app (TOTP, with a QR code) and single-use recovery codes. Password sign-in asks for a code once it is on; sign-in through an external provider follows the provider's policy. Turning it off or making new recovery codes needs a current code. An operator can reset it with `admin-cli user-mfa-reset`, which ends every session and MCP grant.
- **E-mail change confirmed from the new address.** The old address keeps working until the link is opened. Operators use `admin-cli user-set-email`, which leaves the new address unverified.
- **Recovery of Docs without a manager:** workspace owners and admins see their title and creator (not their content) and can name a manager, written to the audit log as `doc.manager_recovered`.
- **A `minio-setup` service** that makes the bucket and an app account limited to it.
- **CycloneDX SBOMs and a report-only container vulnerability scan** (`docker/scripts/supply-chain.sh`, a `supply-chain` CI job). Findings never fail the build.
- **Audit entries** for sign-ins, lockouts, session ends, password changes and sign-in method changes. Failed sign-ins for unknown addresses write nothing.

### Changed

- **An existing account accepts an invitation only while signed in to it.** A member invitation needs nothing more; an admin or owner invitation needs a recent sign-in. The invitation page no longer takes a password.
- **Credential changes end other access.**
  - A password change in the profile keeps this device, signs out the others and keeps MCP grants.
  - A reset by e-mail or by `admin-cli user-set-password` ends every session and MCP grant.
  - Open chat, Project and user sockets close when their session ends.
- **Removing someone from a workspace takes away their explicit access** there: Teams, direct Project roles, Groups, Doc shares (also as the last manager), security and channel auditor roles and private channels. Their own Docs, DMs, history and other workspaces stay.
- **Adding yourself to a Team, or accepting your own invitation into it, no longer makes you admin of a Project you did not already administer.** The Team page shows the Project roles a Team gives.
- **Compose containers drop capabilities, use `no-new-privileges` and, where they can, read-only root file systems.** core-api and web run as uid 10001. Build inputs and tool images are pinned (`cargo build --locked`). The web image uses Caddy 2.11.9, and the core-api image uses Debian trixie with current security updates.

### Fixed

- **Trashed Tickets:** their comments, children, activity and revisions are no longer readable, and Sprint reports no longer list them to people who are not Project admins. Sprint figures are unchanged.
- **Sign-in throttling** counts attempts before the password is checked, so parallel requests cannot get past it. The invitation page could be used to try passwords without a limit.
- **Uploads** are refused before their body is read when they cannot be finished, share a memory budget and time out when stalled. Socket messages, comments and descriptions have size bounds; existing documents stay readable.
- **External sign-in is bound to the browser that started it.** Outgoing requests (push, OAuth client metadata) refuse more internal address forms and proxies.
- **Search snippets** no longer show deleted comments. Doc comment attachments are visible to readers of the page. A channel auditor counts only while they are an active member.
- **`restore-check.sh`** refuses a scratch database that could be the source, and only drops one it created itself.

## 0.3.1

A maintenance release: search recovery and English matching, a restore rehearsal, and user and contributor documentation. Back up before you upgrade.

### Upgrading from 0.3.0, 0.2.2, 0.2.1 or 0.2.0

- No database migration, no API change and no new or changed setting. Check out `v0.3.1` and rebuild with the same `docker/.env` (see [Self-hosting](docs/self-hosting.md#from-v020-v021-v022-or-v030-to-v031)); `db-setup` finds the schema current.
- The search mapping version is now 2. On its first start core-api builds every search index again from PostgreSQL in the background; search keeps answering from the old indices until each new one is ready. Meanwhile OpenSearch needs room for the new index next to the current one (and a previous generation, if one is left from an earlier rebuild), and `admin-cli search-status` shows when every kind is `READY`.

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
