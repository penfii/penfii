# Operations

Day-to-day tasks for someone who runs a Penfii instance. For installing and updating, see [Self-hosting](self-hosting.md); for settings, see [Configuration](configuration.md).

## `admin-cli`

`admin-cli` is the operator tool. `migrate` and `db-provision` use their own credentials. Every other command uses `DATABASE_URL` (the runtime role) and does not migrate; if the schema is behind this build, the command stops and tells you to migrate. With the Compose install, run it from `docker/`:

```sh
docker compose run --rm admin-cli <command> [options]
docker compose run --rm admin-cli --help
```

| Command | Purpose |
|---|---|
| `db-provision` | Create or update the owner role and the runtime role. Needs `PENFII_PROVISION_DATABASE_URL` (a credential that can create roles). Safe to run again. Does not move ownership of an existing installation |
| `migrate` | Apply pending migrations with `PENFII_MIGRATION_DATABASE_URL` (the owner). core-api never does this |
| `bootstrap` | Create the first account and its workspace (refuses when any user exists). Takes the password like `create-user` |
| `workspace-list` | List the workspaces with their ids, active member counts, how many of those accounts are disabled (`disabled=N`) and owners (a disabled owner is marked `(disabled)`) |
| `create-user --email (--password-stdin \| --password) --display-name --workspace <id> [--role]` | Create a new account and add it to an existing workspace (`--role` is `OWNER`, `ADMIN` or `MEMBER`, default `MEMBER`) |
| `workspace-add-member --email --workspace <id> [--role]` | Add an existing account to a workspace |
| `workspace-set-role --email --workspace <id> --role` | Change the role of an active member |
| `user-identities --email` | Show how an account can sign in (password, linked providers) |
| `user-set-password --email [--password-stdin \| --password]` | Set a password and end all sessions of the account. Written to the audit log |
| `user-disable --email`, `user-enable --email` | Disable or enable an account when no workspace owner can. Disabling lists the projects it leaves without an admin; it is never refused for that |
| `search-status`, `search-reindex [--kind]` | Inspect or rebuild the search index |
| `events-dead`, `events-retry [--consumer \| --event \| --all]` | Inspect or retry event deliveries that were given up on |
| `docs-collab-status [--page]`, `doc-collab-reset --page` | Inspect or reset the live co-editing state of docs pages |

People can reset their own password by e-mail only while the server sends e-mail ([E-mail](configuration.md#e-mail)); otherwise, and for an account that has no password, use `user-set-password`.

**Giving the password.** `bootstrap`, `create-user` and `user-set-password` read it without putting it on the command line, where it would stay in the shell history and show in the process list:

- `--password-stdin` reads one line from standard input (the line ending is dropped, everything else is kept). Use it in scripts: `printf '%s' "$PW" | docker compose run --rm -T admin-cli create-user --email ... --password-stdin ...` (`-T` lets compose pass the pipe on).
- With neither option, and a terminal on standard input, the command asks for the password without showing it and asks again to confirm. Without a terminal it fails instead of waiting.
- `--password <value>` still works but prints a warning to stderr; it is not recommended. It cannot be combined with `--password-stdin`.

Nothing here prints the password, not even in an error. The rules for a password (15 to 128 characters, not a known weak one) are the same.

### Adding people to a workspace

Find the workspace id with `workspace-list`, then:

- `create-user` makes a **new** account (its e-mail is taken as verified) and adds it to that workspace. It never makes a workspace; the only way to make one is `bootstrap`, on an empty instance. An e-mail that already has an account is refused: use `workspace-add-member` for it.
- `workspace-add-member` adds an **existing** account. A member who was removed earlier comes back as an active member with the role you give, not the role they had. A person who is already an active member is reported and left as they are (exit status 1); `workspace-set-role` changes a role. A disabled account is refused until you enable it with `user-enable`.
- `workspace-set-role` changes the role of an active member, for when no owner can do it in the web app. The last owner of a workspace cannot be changed to anything else.

These follow the same rules as accepting an invitation, and each change is written to the workspace's audit log as `member.joined` or `member.role_changed` with `via: admin-cli` and no acting person. Joining does not add the person to any team, project or channel. A person who comes back after being removed gets back their place in the organisation: their teams, what those teams give them in projects, and their direct project roles are kept while they are removed and work again once they are active. Their channel memberships are not: removing a person takes them out of every channel of the workspace, private and public, and a person who comes back is in none. A private channel has to be given to them again; a public one they read as anyone else does (the workspace-wide public channels can be read, and posting needs joining first; a project's public channels follow their current project role). Memberships left over from a removal made before this are cleared when the person comes back. Messages, tickets, read positions, mutes, favorites and notification settings are kept.

## Health checks

Both are served by core-api and are reachable through the web container.

- `GET /healthz`: liveness. The process answers; nothing else is checked.
- `GET /readyz`: readiness. `200` while PostgreSQL answers. With search configured, an unreachable or red OpenSearch is reported as `"status": "degraded"` (still `200`; chat, projects and docs keep working). `503` when PostgreSQL does not answer.

`/readyz` also says how far the event consumers are behind: `checks.workers` is `ok`, `degraded`, or `unknown` (no reading yet, in the first minute after the start), and `receipts` carries `oldestWaitingSeconds` (the oldest delivery that is due and not done, per consumer), `dead` (deliveries given up on) and `takenAt`. The instance is `degraded` when a due delivery has waited more than 5 minutes or a delivery is `DEAD`; the status code stays `200`, so a health check never restarts the container for it. The numbers are read once a minute by a background task and kept; a request does not read the receipts table (it is never emptied). Use `/readyz` as a readiness probe only, never as a liveness probe (Kubernetes: `/healthz` for liveness).

## Background workers

The workers (event consumers, reminders, Sprint archive, retention, sweepers, search upkeep, the co-editing owner) run inside core-api; `PENFII_WORKER=0` stops them in one process.

- **A panic inside one unit of work is that unit's failure.** One event that panics in a consumer is recorded like an error (`last_error_code` is `panic`), is retried with the usual delays, and becomes `DEAD` after 12 attempts (`admin-cli events-dead`); the other events and consumers go on. A panic in one pass of a reminder, Sprint-archive, retention, sweeper or search pass is logged as an error and the pass is retried at the next tick; a Sprint whose archive panics stays scheduled and does not stop the other Sprints.
- **A panic that gets out of a worker's loop ends the process.** core-api logs it as an error with the worker's name, shuts down in the usual order (HTTP, workers, co-editing, database pool, within the 10 seconds of a stop) and exits with a non-zero status. Nothing restarts a worker inside the process: the container runtime does (`restart: unless-stopped` in the Compose stack). Every panic of the process is also logged through the log with its place (`panic` at `error`).
- **What to watch.** A restarting core-api shows as a rising restart count (`docker ps`, `docker inspect`) and an error line naming the worker. If it restarts again and again, the cause is data that makes a worker's loop panic every time: read the panic line, fix or remove the data. A process started without a restart policy (a bare binary) simply stays down. Requests in flight are cut when the process ends; delivery, leases and co-editing recover on the next start by design.
- A panic in a request handler is not a worker failure: that request is answered `500 internal` and logged (the process keeps running); a panic while sending a Web Push or a password-reset mail is logged and nothing else.
- **Stopping.** The order is: HTTP stops taking connections and finishes its requests (4 s), the workers stop (3 s), the Web Pushes already on their way get 1 s and the rest are cancelled (a push is best effort; the Inbox item is there either way), the co-editing rooms are written, and the database pool closes, all inside the 10 seconds Docker gives a stop. At most 64 batches of pushes are on their way at once; a batch that comes while 64 are still being sent is dropped with a warning.

## Search

PostgreSQL is the source of truth; the OpenSearch index is derived data and can always be rebuilt (`admin-cli search-reindex`).

- **Without OpenSearch**, core-api starts, logs a warning, and content search answers `503 search_unavailable`. Finding people, teams, projects and channels comes from PostgreSQL and keeps working.
- **Keeping the index right.** Index updates are an event consumer: while OpenSearch is down, a delivery is retried up to 12 times with growing delays and then marked `DEAD`. A background task also rebuilds any index that is missing or has an old mapping. After an outage, or after changing the analyzer, run `admin-cli search-status` and, if needed, `admin-cli events-retry --consumer search` or `admin-cli search-reindex`.

### Search index prefix

Index names are `<prefix>-v<mapping>-<kind>-<time>` with an alias `<prefix>-<kind>`. The prefix is the only thing that keeps two environments apart on one OpenSearch, so give each environment its own `PENFII_OPENSEARCH_INDEX_PREFIX`.

- Each index records which database made it. If another database's indices already use the prefix, core-api refuses to start (`search_prefix_in_use`) rather than write into them. The same happens after you drop and recreate a database: delete the old indices (`curl -X DELETE '<opensearch-url>/<prefix>-*'`) and let the index build again.
- Changing the prefix leaves the old indices alone; delete them yourself when they are no longer needed. The new prefix is built from PostgreSQL on start-up.
- `admin-cli search-status` prints the prefix in use, this database's owner id and the state of each index. core-api logs the prefix at start-up.

## Database timeouts

The runtime role's sessions use `lock_timeout` 5s, `statement_timeout` 30s and `idle_in_transaction_session_timeout` 60s (`admin-cli db-provision` sets them with `ALTER ROLE`). They are an operating baseline for a pool of about ten connections, not PostgreSQL defaults: a lock wait or a runaway statement should free the connection instead of stalling every request. There is no global `transaction_timeout`. The owner role that runs migrations is not given these limits.

A statement that is supposed to run longer sets a higher limit inside its own transaction with `SET LOCAL`. The paths that hold a transaction across more than one statement today are bounded batches: event consumers take 50 receipts, the unbound-upload sweep takes 200 files, trash purge takes a limited number of tickets, and a search reindex reads one page of ids at a time. None of those is given a higher limit; a page that cannot finish in 30 seconds fails and is retried rather than holding a connection. A lock that is not granted in time (`55P03`), a deadlock (`40P01`) and a connection the pool could not get are answered to the client as HTTP 503 `db_unavailable` with `{code, message}` only: no `Retry-After`, no SQLSTATE, no table name. A statement cancelled by `statement_timeout` (`57014`) stays an internal error (HTTP 500). The server does not automatically retry requests. A command that records its own idempotency key could be retried on `40P01` or `40001` only; none is retried today, because those errors are not something the current transactions produce (writes are not `SERIALIZABLE`, and lock order is the application's).

## Event deliveries that were given up on

A consumer (Inbox, Watch, search, and `storage`, which deletes a retired file's object) tries an event up to 12 times. For a consumer that works outside the database, each claim counts, including a claim that happens because the previous worker's lease ran out without a report; a worker that keeps dying reaches `DEAD` instead of being claimed forever. A failure of the handler does not add another count on top of the claim. For a consumer whose work is only rows, the count goes up when the handler fails, inside the same transaction. After 12 the delivery is `DEAD` and the others carry on. Nothing is removed. `events-dead` prints that count.

```sh
admin-cli events-dead                         # list them
admin-cli events-retry --consumer inbox       # or --event <id>, or --all
```

`events-retry` only changes `DEAD` deliveries back to pending with new attempts, and a running core-api picks them up. Consumers are safe to run twice for the same event.

## Co-editing

The server has a live co-editing socket for docs. It keeps its rooms in the memory of one core-api process; the process that holds the PostgreSQL advisory lock owns them. A second instance answers co-editing sockets with `503 collab_unavailable`, and has the other limits listed in [Running more than one core-api](#running-more-than-one-core-api). `admin-cli docs-collab-status` shows how far each page's live state is ahead of PostgreSQL and which pages the server refused to store; `admin-cli doc-collab-reset --page <id>` starts a new live lineage from the last stored content (edits not yet written to PostgreSQL are lost). The shipped web app does not use this socket yet; see [Known limitations](known-limitations.md).

## Running more than one core-api

The supported install runs **one** core-api. A second one starts and serves requests, but these things are per process, and a person connected to the other instance feels them:

- **Live notices.** When something changes (a message, a ticket, an Inbox item, a person's channels), the process that handled it sends a notice on a broadcast in its own memory, and only the sockets connected to that process get it. A person on another instance sees the change when their client next reads (a reload, a reconnect, the window regaining focus), not live. The conversation socket repairs a gap on the next notice it does receive; the Project board socket and the user socket have no replay. An Inbox item is announced by the instance that processed its delivery, which may not be the one the person is connected to. Sticky sessions do not help for that reason.
- **Co-editing** has one owner (the instance that holds the PostgreSQL advisory lock); the others answer `503 collab_unavailable`.
- **The `/mcp` rate limit** is counted per process: with N instances a grant can make N times the calls per minute. (The sign-in throttle is kept in PostgreSQL and is shared.)
- **Workers** can run on several instances: event consumers claim deliveries with a lease, reminders, scheduled Sprint archives and the unattached-upload sweep take their rows with `SKIP LOCKED`, the trash sweep re-checks each ticket under its row lock, and the MCP retention purge only deletes what is old. Search upkeep has no lock: two instances may build the same index at once, which is harmless (writes carry versions) but does the work twice. `PENFII_WORKER=0` turns the workers off in an instance that should not run them. `/readyz` reports the receipts, which are in PostgreSQL, so every instance says the same.

Carrying the notices between processes (for example through PostgreSQL `LISTEN/NOTIFY`) is not done; it would change the sockets' receive path and the event workers' wake-up (they poll every half second today), and it is the step that would make more than one instance a supported install.

## Workspace administration

Workspace roles are `OWNER`, `ADMIN` and `MEMBER`. Owners and admins manage the workspace in the web app under *Workspace administration*: members and roles, removing members, invitations, team membership and the audit log. Ticket trash and purge are in a project's settings.

The server decides every request from the person's current membership in PostgreSQL, so a change takes effect on the next request; open sockets close when a person's access changes. Sensitive actions (removing a member, disabling an account, purging a ticket, changing MCP policy) need a recent password entry (`PENFII_REAUTH_WINDOW_SECONDS`). Disabling an account ends all its sessions and OAuth grants at once and blocks sign-in. Only an owner invites or grants `OWNER` or `ADMIN`.

### Projects and their admins

A project keeps an admin who can act. Whoever counts as an admin counts only while they are an active member of the workspace **and** their account is not disabled; the same meaning is used for the last owner of a workspace, the last manager of a team and the last manager of a private channel.

- The last such **direct** admin of a project cannot be demoted or have their grant taken away, by anyone, themself included (`409 last_project_admin`). Other changes to the project's grants are not held up by it.
- Removing a member or disabling an account is never refused for this. Before you do it, the confirmation lists the projects that would lose their last direct admin, saying whether an admin through a team or group stays (the project is still managed) or nobody would be left. `admin-cli user-disable` prints the same as warnings.
- A project can end up with no active admin by any source (directly, through a team that takes part with the admin role, or through a group). Nobody can manage it then, a workspace owner or admin included, because a workspace role gives no role in a project. Under *Workspace administration* a section **Projects without an admin** lists them, and an owner or admin can **name an admin** for one: the one named becomes its direct admin, nothing is promoted automatically, and only a project without an active admin can be recovered this way. Naming gives the owner or admin no power in the project; naming themself is allowed and is the one way they get any. The person named can read the project's content, which the confirmation says, and the audit log records it as `project.admin_recovered` with whether it opened the project to someone who could not read it.
- Enabling an account again brings its admin role back as it was, next to an admin named in the meantime. Nothing is tidied up on its own.

### Private channels

A private channel is open only to the people in it. Its creator is its first manager. **Running** a channel and **reading** it are different things:

- The channel's **manager** reads it and runs it: adds people (a workspace member, and in a Project's channel only someone who takes part in the Project), takes people out, makes a member a manager or a plain member again, turns *Let members add other people* (`allowMemberInvites`, off by default) on or off, changes the description, and manages its webhooks. The channel's *Add people* dialog does the people part.
- The **admin of its Project** runs the people and the settings the same way **without being able to read the channel**: they see the channel under *Project settings* > **Private channels** (name, description, counts; never messages or files), can put other people in, take people out, make others managers, change the switch and the description, and see and revoke its webhooks. They cannot read or post in it, cannot put themself in or make themself a manager (`403`), cannot create or rotate a webhook (a webhook posts into the channel, so only someone who may post issues one: the manager, or an admin who is in the channel), and cannot make the channel public. Every such change is recorded in the audit log (`channel.member_added`, `channel.member_removed`, `channel.member_role_changed`, `channel.allow_member_invites_changed`, `webhook.revoked`) with `actorCanRead`, so an admin who did it without reading the channel shows.
- A workspace owner or admin is not an admin of someone's private channel: they have no part in it unless they are its manager or are named by the recovery below.
- A plain member reads and posts, adds people only while the channel allows member invitations, and can leave.

The API does not let a channel lose its last manager: making the last manager a member, taking them out, or the last manager leaving, is refused with `409 last_manager`, so hand the role to someone else first. A manager counts only while they are an active member of the workspace with an account that is not disabled **and** still take part in the Project of a Project's channel, so a manager who is disabled or who lost the Project does not keep the channel run. (Such a manager can leave the channel without one.) A person who is added, made a manager, taken out or leaves sees their channel list change without reloading. The changes are also recorded as domain events (`channel.member_added`, `channel.member_role_changed`, `channel.member_removed`). Removing a person from the workspace takes their channel memberships, so it can leave a private channel without a manager; that is never refused for it. Before you do it, and before disabling an account, the confirmation lists the private channels that would be left without a manager (`admin-cli user-disable` prints them as warnings), and the audit entry `member.removed` records how many channels were cleared and which were left without a manager.

**Visibility.** Making a private channel public is the manager's alone: it shows what was said before to people who could not read it. Making a channel private is for the people who run it (a Project's admin, the manager, and a workspace owner or admin for a public workspace-wide channel); whoever does it and is not in the channel loses it themself. A change of visibility neither makes nor removes a membership: people who are in the channel stay, a public channel's other readers (its Project's people, or the workspace) read it only by the public rule, so turning it private closes it to them and turning it public opens it to them. They see the change without reloading, and the audit log records it as `channel.visibility_changed` with who did it and whether they were in the channel. A channel's Project and Team are fixed when it is created.

**Projects and the channels in them.** The access to a Project's channels follows the person's part in the Project. When a person stops taking part (their direct role is removed, the Team that gave them the role is unlinked or loses its role, or they leave the Team), their memberships of the Project's channels, private and public, and with them any manager role, are taken away in the same change; if they still take part another way they keep them. Taking part again brings none of it back: a private channel has to be opened to them again, a public one is open by the Project. A channel membership left by an older version for a person who does not take part is not a way in either: it grants no reading, writing or role, and the upgrade removes it. This can leave a private channel without a manager; it is never refused for that and the channel is listed below. The audit entry of the change (`project.grant_removed`, `project.team_unlinked`, `project.team_participation_set`, `team.member_removed`) records how many channels were cleared and which were left without a manager.

A private channel with no active manager, whatever the cause (it never had one, its managers were removed or disabled, or a public channel without a manager was made private), is listed under *Workspace administration* in **Private channels without a manager**. A workspace owner or admin can **name a manager** for one: the one named becomes its manager and can read it, which the confirmation says, nothing is promoted automatically, and only a private channel without an active manager can be recovered this way (`409 channel_has_manager` otherwise). Naming gives the owner or admin no access; naming themself is allowed and is the one way they get any. The audit log records it as `channel.manager_recovered`, with whether it opened the channel to someone who could not read it. A manager whose account is enabled again is a manager next to the one named in the meantime; nothing is tidied up. A public channel can have no manager: it is not listed and there is nothing to recover, and in it only a manager can delete other people's messages, so with none nobody can; a workspace owner or admin and a Project's admin get no moderation power there. A separate policy for moderating public channels is not written yet.

The audit log covers membership and role changes, account disabling and enabling, invitations, team membership, ticket trash, restore and purge, and project designation and OAuth entries. Invitation tokens are never written to it.
