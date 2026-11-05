# Architecture

Penfii is a PostgreSQL-backed server, a single-page web app, and the services around them. This page is an overview for contributors and operators; the code is the reference.

```
 browser ──► web (Caddy, serves the app) ──► core-api ──► PostgreSQL   (source of truth)
                  /v1  /oauth  /mcp           │  ├─────► OpenSearch   (search index, derived)
                  /.well-known /healthz       │  └─────► MinIO / S3   (file bytes)
                  /readyz                     └ background workers (events, reminders) in the same process
 AI client ─► /mcp (optional) ───────────────┘
 operator ──► admin-cli ───────────────────────► PostgreSQL, OpenSearch
```

## Repository parts

| Path | Role |
|---|---|
| `server/apps/core-api` | The server binary: HTTP API, WebSockets, event consumers, reminder scheduler. Applies migrations on start. |
| `server/apps/admin-cli` | Operator commands (see [Operations](operations.md)). |
| `server/crates/core` | Domain logic and all SQL, one module per area (identity, work, docs, search, files, ...). |
| `server/crates/http-api` | The axum router and the mapping between HTTP and the domain. |
| `server/crates/mcp` | The MCP operation catalog and tool handlers. |
| `server/crates/contracts` | Request and response types. |
| `server/api/openapi.json` | The API description, maintained by hand. A test fails when a route is missing or extra, or when a type in `crates/contracts` and the schema of the same name disagree. |
| `server/tests` | Integration tests that run the real router against PostgreSQL and OpenSearch. |
| `database/migrations` | The schema as numbered SQL files, compiled into the server binary. |
| `client/apps/web` | The Vite entry point. |
| `client/packages/client` | The React app: routes, pages, layout, feature modules (chat, work, docs, admin, ...), translations, design components. |
| `client/packages/api-client` | The `fetch` wrapper and one method per API call. |
| `client/packages/contracts` | The API types the client works with and `openapi.json`, a copy of the server's file that a test holds the types to. |
| `docker/` | Compose stack and the Dockerfiles for the server, the web app (Caddy) and OpenSearch with the Korean analyzer. |

## Rules that hold it together

- **PostgreSQL is the source of truth.** OpenSearch is derived data, rebuilt with `admin-cli search-reindex`. File bytes live in S3-compatible storage; PostgreSQL holds their metadata.
- **Permissions are decided by the server, from PostgreSQL, on every request.** There is no permission cache and the client is never the check. Things a person may not see are "not found", not "forbidden". Search results are re-checked against PostgreSQL before they are returned.
- **Commands own their transaction.** A command function in `crates/core` begins and commits its own transaction; helpers it calls must not start another (a code convention).
- **The schema is applied by the server.** On start-up core-api (and every `admin-cli` command) applies pending files from `database/migrations` under a PostgreSQL advisory lock, each in a transaction, and records them with a checksum in `penfii._sqlx_migrations`.
- **Same origin.** The web container serves the app and forwards `/v1` (HTTP and WebSocket), `/oauth`, `/mcp`, `/.well-known/*`, `/healthz` and `/readyz` to core-api, so the browser sees one origin. The client uses relative URLs and needs no configuration.
- **Event consumers.** Domain events are stored in PostgreSQL (`domain_events`) and delivered to consumers (Inbox, Watch, search) with retries; a delivery that fails too often becomes `DEAD` and can be retried by an operator.
- **Co-editing is single-instance.** Live docs rooms live in the memory of one core-api process.
