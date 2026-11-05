# Operations

Day-to-day tasks for someone who runs a Penfii instance. For installing and updating, see [Self-hosting](self-hosting.md); for settings, see [Configuration](configuration.md).

## `admin-cli`

`admin-cli` is the operator tool. It works on the database named by `DATABASE_URL` and applies pending migrations before it runs. With the Compose install, run it from `docker/`:

```sh
docker compose run --rm admin-cli <command> [options]
docker compose run --rm admin-cli --help
```

| Command | Purpose |
|---|---|
| `bootstrap` | Create the first account and its workspace (refuses when any user exists) |
| `create-user` | Create another account with a workspace of its own |
| `user-identities --email` | Show how an account can sign in (password, linked providers) |
| `user-set-password --email --password` | Set a password and end all sessions of the account. Written to the audit log |
| `user-disable --email`, `user-enable --email` | Disable or enable an account when no workspace owner can |
| `search-status`, `search-reindex [--kind]` | Inspect or rebuild the search index |
| `events-dead`, `events-retry [--consumer \| --event \| --all]` | Inspect or retry event deliveries that were given up on |
| `docs-collab-status [--page]`, `doc-collab-reset --page` | Inspect or reset the live co-editing state of docs pages |

There is no password reset by e-mail; use `user-set-password`.

## Health checks

Both are served by core-api and are reachable through the web container.

- `GET /healthz`: liveness. The process answers; nothing else is checked.
- `GET /readyz`: readiness. `200` while PostgreSQL answers. With search configured, an unreachable or red OpenSearch is reported as `"status": "degraded"` (still `200`; chat, projects and docs keep working). `503` when PostgreSQL does not answer.

## Search

PostgreSQL is the source of truth; the OpenSearch index is derived data and can always be rebuilt (`admin-cli search-reindex`).

- **Without OpenSearch**, core-api starts, logs a warning, and content search answers `503 search_unavailable`. Finding people, teams, projects and channels comes from PostgreSQL and keeps working.
- **Keeping the index right.** Index updates are an event consumer: while OpenSearch is down, a delivery is retried up to 12 times with growing delays and then marked `DEAD`. A background task also rebuilds any index that is missing or has an old mapping. After an outage, or after changing the analyzer, run `admin-cli search-status` and, if needed, `admin-cli events-retry --consumer search` or `admin-cli search-reindex`.

### Search index prefix

Index names are `<prefix>-v<mapping>-<kind>-<time>` with an alias `<prefix>-<kind>`. The prefix is the only thing that keeps two environments apart on one OpenSearch, so give each environment its own `PENFII_OPENSEARCH_INDEX_PREFIX`.

- Each index records which database made it. If another database's indices already use the prefix, core-api refuses to start (`search_prefix_in_use`) rather than write into them. The same happens after you drop and recreate a database: delete the old indices (`curl -X DELETE '<opensearch-url>/<prefix>-*'`) and let the index build again.
- Changing the prefix leaves the old indices alone; delete them yourself when they are no longer needed. The new prefix is built from PostgreSQL on start-up.
- `admin-cli search-status` prints the prefix in use, this database's owner id and the state of each index. core-api logs the prefix at start-up.

## Event deliveries that were given up on

A consumer (Inbox, Watch, search) tries an event up to 12 times with growing delays; after that the delivery is `DEAD` and the others carry on. Nothing is removed.

```sh
admin-cli events-dead                         # list them
admin-cli events-retry --consumer inbox       # or --event <id>, or --all
```

`events-retry` only changes `DEAD` deliveries back to pending with new attempts, and a running core-api picks them up. Consumers are safe to run twice for the same event.

## Co-editing

The server has a live co-editing socket for docs. It keeps its rooms in the memory of one core-api process; the process that holds the PostgreSQL advisory lock owns them. A second instance serves everything else and answers co-editing sockets with `503 collab_unavailable`. `admin-cli docs-collab-status` shows how far each page's live state is ahead of PostgreSQL and which pages the server refused to store; `admin-cli doc-collab-reset --page <id>` starts a new live lineage from the last stored content (edits not yet written to PostgreSQL are lost). The shipped web app does not use this socket yet; see [Known limitations](known-limitations.md).

## Workspace administration

Workspace roles are `OWNER`, `ADMIN` and `MEMBER`. Owners and admins manage the workspace in the web app under *Workspace administration*: members and roles, removing members, invitations, team membership and the audit log. Ticket trash and purge are in a project's settings.

The server decides every request from the person's current membership in PostgreSQL, so a change takes effect on the next request; open sockets close when a person's access changes. Sensitive actions (removing a member, disabling an account, purging a ticket, changing MCP policy) need a recent password entry (`PENFII_REAUTH_WINDOW_SECONDS`). Disabling an account ends all its sessions and OAuth grants at once and blocks sign-in. Only an owner invites or grants `OWNER` or `ADMIN`.

### Private channels

A private channel is open only to the people in it. Its creator is its first manager. A manager, or the admin of its Project for a Project's channel, can add people to it (a workspace member, and in a Project's channel only someone who takes part in the Project) and can make a member a manager or a plain member again; the channel's *Add people* dialog does both. The API does not let a channel lose its last manager: making the last manager a member, or the last manager leaving, is refused with `409 last_manager`, so hand the role to someone else first. (A manager whose workspace access ends is not covered; see [Known limitations](known-limitations.md).) Anyone in a private channel can leave it from the channel header; nobody can remove another person, and a public channel has no leave or role change. A person who is added, made a manager or leaves sees their channel list change without reloading. These changes are recorded as domain events (`channel.member_added`, `channel.member_role_changed`, `channel.member_removed`), not in the audit log. An administrator who has to take someone out of a channel or recover one without a manager has no screen for it yet.

The audit log covers membership and role changes, account disabling and enabling, invitations, team membership, ticket trash, restore and purge, and project designation and OAuth entries. Invitation tokens are never written to it.
