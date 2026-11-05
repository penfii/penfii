# Configuration

Penfii has two layers of configuration:

1. **`docker/.env`**: the file you edit when you self-host with Docker Compose. Compose reads it and passes selected values to the containers.
2. **Environment variables of the server** (`core-api` and `admin-cli`): what the server actually reads. The Compose file sets some of them from `docker/.env` and leaves the rest at their defaults.

The web app has no configuration of its own; it talks to the server on the same origin through relative URLs.

## `docker/.env`

Copy `docker/.env.example` to `docker/.env` and replace every `CHANGE_ME`. Never commit `.env` (it is in `.gitignore`).

| Variable | Default in `.env.example` | Meaning |
|---|---|---|
| `PENFII_PUBLIC_URL` | `http://localhost:8080` | Required. The origin people type in the browser, with no path. Sign-in redirects, the co-editing socket and the MCP gateway use it. `https://` also marks the session cookie `Secure`. Compose also passes it to the server as `PENFII_ALLOWED_ORIGINS` |
| `PENFII_HTTP_BIND`, `PENFII_HTTP_PORT` | `127.0.0.1`, `8080` | Address and port on the host where the web container is published |
| `POSTGRES_PASSWORD` | `CHANGE_ME` | Required. PostgreSQL password; letters and digits only |
| `PENFII_S3_ACCESS_KEY`, `PENFII_S3_SECRET_KEY` | `CHANGE_ME` | Required. The MinIO root key pair, also used by the server as its S3 key pair (8 or more characters) |
| `PENFII_SESSION_TTL_SECONDS` | `43200` | Required. Session lifetime |
| `PENFII_MESSAGE_BODY_MAX_BYTES` | `65536` | Required. Largest chat message body |
| `PENFII_INVITE_TTL_SECONDS` | `604800` | Required. Invitation lifetime |
| `PENFII_FILE_MAX_BYTES` | `104857600` | Required. Largest single file |
| `PENFII_MESSAGE_FILE_MAX_COUNT`, `PENFII_MESSAGE_FILE_MAX_BYTES` | `10`, `209715200` | Required. Attachments per message and their total size |
| `PENFII_EDGE_PROXIES` | `private_ranges` | Proxies in front of the web container whose `X-Forwarded-For` is believed (space-separated CIDRs, or `private_ranges`) |
| `PENFII_TRUSTED_PROXIES` | the three private ranges | The same for core-api (comma-separated CIDRs). Keep it in step with `PENFII_EDGE_PROXIES` |
| `PENFII_MCP_ENABLED`, `PENFII_MCP_DEFAULT_POLICY`, `PENFII_MCP_DCR_ENABLED` | off, `standard`, off | The MCP gateway; see [below](#mcp-gateway) |
| `PENFII_OPENSEARCH_INDEX_PREFIX` | `penfii` | Start of every OpenSearch index name. Change it only if the OpenSearch is shared |
| `PENFII_DB_MAX_CONNECTIONS` | `10` | PostgreSQL connection pool size |

The Compose file passes only the variables above (and fixes the ones that depend on the stack: the database, OpenSearch and S3 addresses, the bind address). To set any other server variable from the reference below, for example an external sign-in provider, add it under `core-api` → `environment` in `docker/compose.yaml`.

## Server environment reference

Required limits have no built-in default on purpose: when one is unset, the operation that needs it fails with `409` and a code ending in `_unconfigured` instead of guessing.

### Core

| Variable | Default | Effect |
|---|---|---|
| `DATABASE_URL` | required | PostgreSQL connection string. The schema name `penfii` is fixed; the database name is not |
| `PENFII_BIND` | `0.0.0.0:8080` | Listen address |
| `PENFII_PUBLIC_URL` | unset | The public origin of the instance (`https://penfii.example.com`, no path). Needed for external sign-in redirect URIs and for MCP. When it starts with `https://`, the session cookie gets `Secure` unless `PENFII_COOKIE_SECURE=0` |
| `PENFII_COOKIE_SECURE` | follows `PENFII_PUBLIC_URL` | `1` adds `Secure` to the `penfii_session` cookie, `0` removes it |
| `PENFII_ALLOWED_ORIGINS` | `http://localhost:5173,http://127.0.0.1:5173` | Comma-separated browser origins that may open a WebSocket and call the API cross-origin, besides the server's own origin. Set it to the public origin when a proxy rewrites `Host`; a value replaces the development defaults. A handshake from another origin is refused with `403 origin_not_allowed`; a client that sends no `Origin` (not a browser) is not affected |
| `PENFII_SESSION_TTL_SECONDS` | unset | Missing or `0` makes login return `session_ttl_unconfigured` |
| `PENFII_MESSAGE_BODY_MAX_BYTES` | unset | Missing or `0` makes send and edit return `message_limit_unconfigured` |
| `PENFII_INVITE_TTL_SECONDS` | unset | Missing makes invitation creation return `invite_ttl_unconfigured` |
| `PENFII_TRUSTED_PROXIES` | `127.0.0.0/8,::1/128` | Comma-separated CIDRs (or single addresses) of reverse proxies whose `X-Forwarded-For` is believed; it feeds the per-address sign-in throttle. A malformed entry stops start-up; an empty value trusts nobody. Do not list a network that untrusted machines can reach |
| `PENFII_DB_MAX_CONNECTIONS` | `10` | PostgreSQL pool size, 1 to 500 (anything else stops start-up). Co-editing keeps one more connection of its own. A request that waits more than 10 seconds for a connection fails |
| `PENFII_REAUTH_WINDOW_SECONDS` | `600` | How recently a person must have entered their password for sensitive actions: removing a member, disabling an account, purging a ticket, changing MCP policy |
| `PENFII_WORKER` | on | `0` stops the event consumers and the reminder scheduler in this process |
| `PENFII_DEV_RETURN_INVITE_TOKEN` | `0` | **Development switch.** `1` returns the invitation link token once, in the response that creates the invitation. Do not use it on a network you do not control |
| `PENFII_DEV_SEED` | unset | **Development switch.** `1` applies the development seed (a demo workspace with known passwords) on start-up, in debug builds only; a release build never seeds |

### Sign-in throttle

Failed password proofs are counted in PostgreSQL and lock a key for one window once the key has made too many. A failed sign-in counts against all three keys; the first one that is full refuses the next attempt, before the password is looked at. An unknown e-mail is counted and answered like a known one.

| Key | Default | Variables (`PENFII_LOGIN_THROTTLE_` + ...) |
|---|---|---|
| one e-mail from one address | 5 per 15 minutes | `ACCOUNT_ADDRESS_FAILURES`, `ACCOUNT_ADDRESS_WINDOW_SECONDS` |
| one e-mail from anywhere | 20 per 15 minutes | `ACCOUNT_FAILURES`, `ACCOUNT_WINDOW_SECONDS` |
| one address, any e-mail (also counts OAuth sign-in starts) | 30 per 15 minutes | `ADDRESS_FAILURES`, `ADDRESS_WINDOW_SECONDS` |

A value that is not a positive whole number stops start-up. A success clears the e-mail counters.

### Passwords

A password that someone sets (creating an account, changing it in the profile, accepting an invitation, `admin-cli bootstrap`, `create-user`, `user-set-password`) must be 15 to 128 characters and must not be a known weak password. There are no composition rules and no expiry. Passwords are stored as Argon2id. Signing in does not apply the rule, so a password set under an earlier rule keeps working until it is changed.

### Files

File attachments need an S3-compatible bucket that accepts path-style requests. The server creates the bucket on start. Without storage configured, uploads are unavailable.

| Variable | Default | Effect |
|---|---|---|
| `PENFII_S3_ENDPOINT`, `PENFII_S3_BUCKET`, `PENFII_S3_ACCESS_KEY`, `PENFII_S3_SECRET_KEY` | unset | All four are required to enable file storage |
| `PENFII_S3_REGION` | `us-east-1` | Signing region |
| `PENFII_FILE_MAX_BYTES` | unset | Largest single file; unset makes uploads fail with `file_limit_unconfigured` |
| `PENFII_MESSAGE_FILE_MAX_COUNT`, `PENFII_MESSAGE_FILE_MAX_BYTES` | unset | Attachments per message and their total size (ticket attachments are not limited by these) |
| `PENFII_UNBOUND_UPLOAD_GRACE_SECONDS` | `86400` | How long an upload that was never attached may wait before it is removed |

### Search

OpenSearch is a required part of the install: content search reads from it, and there is no PostgreSQL fallback (permission checks stay in PostgreSQL; ranking and Korean text analysis are OpenSearch's job). Chat, projects and docs keep running without it. It needs OpenSearch 2.x with the `analysis-nori` plugin; `docker/opensearch/Dockerfile` builds the tested image.

| Variable | Default | Effect |
|---|---|---|
| `PENFII_OPENSEARCH_URL` | unset | OpenSearch address. Unset: content search (messages, tickets, docs, files) answers `503 search_unavailable`; finding people, teams, projects and channels still works from PostgreSQL |
| `PENFII_OPENSEARCH_USER`, `PENFII_OPENSEARCH_PASSWORD` | unset | Basic-auth credentials |
| `PENFII_OPENSEARCH_INDEX_PREFIX` | `penfii-dev` (the Compose file sets `penfii`) | Start of every index name (`a-z`, `0-9`, `-`, `_`, up to 40). Give each environment that shares an OpenSearch its own; see [Operations](operations.md#search-index-prefix) |
| `PENFII_SEARCH_DOC_BODY_MAX_BYTES` | `524288` | Body text indexed per page (cut after this) |
| `PENFII_SEARCH_COMMENTS_MAX_BYTES` | `51200` | Comment text indexed per page |

### External sign-in providers (optional)

A provider is offered only when its variables are complete. The redirect URI to register with the provider is `${PENFII_PUBLIC_URL}/v1/auth/<provider>/callback` (`google`, `microsoft`, `github`, `oidc:<slug>`).

| Provider | Variables |
|---|---|
| Google | `PENFII_AUTH_GOOGLE_CLIENT_ID`, `PENFII_AUTH_GOOGLE_CLIENT_SECRET`, optional `PENFII_AUTH_GOOGLE_HOSTED_DOMAIN`, `PENFII_AUTH_GOOGLE_SIGNUP` |
| Microsoft | `PENFII_AUTH_MICROSOFT_CLIENT_ID`, `PENFII_AUTH_MICROSOFT_CLIENT_SECRET`, `PENFII_AUTH_MICROSOFT_TENANT` (default `organizations`; the multi-tenant values also need `PENFII_AUTH_MICROSOFT_ALLOWED_TENANTS`), `PENFII_AUTH_MICROSOFT_SIGNUP` |
| GitHub | `PENFII_AUTH_GITHUB_CLIENT_ID`, `PENFII_AUTH_GITHUB_CLIENT_SECRET` (accounts are created by invitation only; `PENFII_AUTH_GITHUB_SIGNUP` is ignored with a warning) |
| Generic OIDC | `PENFII_AUTH_OIDC_PROVIDERS=slug,slug` and per slug `PENFII_AUTH_OIDC_<SLUG>_ISSUER` (https), `..._CLIENT_ID`, `..._CLIENT_SECRET`, `..._NAME`, `..._SIGNUP` |

`*_SIGNUP=1` lets a first external sign-in create the account; by default an account must already exist or be invited.

### MCP gateway

Set `PENFII_MCP_ENABLED=1` together with a valid `PENFII_PUBLIC_URL` (`https://`, or `http://` on localhost); the process refuses to start otherwise. It serves `/oauth/*`, `/.well-known/oauth-*`, `/.well-known/openid-configuration` and `/mcp` on the same origin, so a reverse proxy has to forward them to core-api (the bundled web container does).

| Variable | Default | Effect |
|---|---|---|
| `PENFII_MCP_DEFAULT_POLICY` | `standard` | What a workspace without a stored policy gets: `standard` (on; writes allowed; destructive and administrative actions off), `read_only`, `closed`. Read once at start-up; an owner or admin can change a workspace's policy afterwards |
| `PENFII_MCP_ACCESS_TTL_SECONDS` | `3600` | Access token lifetime |
| `PENFII_MCP_REFRESH_IDLE_SECONDS`, `PENFII_MCP_REFRESH_ABSOLUTE_SECONDS` | 30 and 90 days | Refresh token idle and absolute lifetime |
| `PENFII_MCP_DCR_ENABLED` | off | Public-client Dynamic Client Registration (compatibility only) |
| `PENFII_MCP_ALLOWED_ORIGINS` | none | Extra browser origins `/mcp` accepts besides `PENFII_PUBLIC_URL`. Origins are compared case-insensitively without a trailing slash, as for `PENFII_ALLOWED_ORIGINS`; a request with no `Origin` is allowed (the bearer token still decides), one with an unreadable `Origin` is refused |
| `PENFII_MCP_CIMD_PRIVATE_HOSTS` | none | Hosts that may serve client metadata from a private address (otherwise such addresses are refused) |
| `PENFII_MCP_LOG_RETENTION_DAYS`, `PENFII_MCP_AUDIT_LOG_RETENTION_DAYS`, `PENFII_MCP_GRANT_RETENTION_DAYS` | 30, 365, 365 | Retention of MCP call logs (reads, changes) and revoked grants |

### Docs and co-editing

Initial values for a self-hosted install; none of them is a product limit. A value that is set must be a positive integer, and `PENFII_DOC_MATERIALIZE_MAX_WAIT_MS` must not be smaller than `PENFII_DOC_MATERIALIZE_DEBOUNCE_MS`, or core-api refuses to start and names the variable.

| Variable | Default | Meaning |
|---|---|---|
| `PENFII_DOC_MATERIALIZE_DEBOUNCE_MS` | `3000` | Quiet time after the last edit before the text is written to PostgreSQL |
| `PENFII_DOC_MATERIALIZE_MAX_WAIT_MS` | `15000` | The longest a page may go without being written while people keep editing |
| `PENFII_DOC_COLLAB_PERSIST_INTERVAL_MS` | `500` | How often live edits are appended to the durable update log |
| `PENFII_DOC_COLLAB_COMPACT_UPDATES` | `200` | Log rows after which a snapshot is written and the log is cut |
| `PENFII_DOC_COLLAB_ROOM_IDLE_SECONDS` | `60` | How long a room stays loaded after its last editor left |
| `PENFII_DOC_COLLAB_MAX_ROOMS` | `200` | Pages open for live editing at once (more are refused) |
| `PENFII_DOC_COLLAB_MAX_CONNECTIONS_PER_USER_DOC` | `5` | Sockets one person may hold on one page |
| `PENFII_DOC_COLLAB_MAX_UPDATE_BYTES` | `1048576` | One update message (larger closes that socket); must not exceed the next value |
| `PENFII_DOC_COLLAB_MAX_STATE_BYTES` | `16777216` | Encoded live state of one page |
| `PENFII_DOC_COLLAB_AUTH_TTL_SECONDS` | `5` | The longest a connection goes without being checked against PostgreSQL |
| `PENFII_DOC_MAX_CONTENT_BYTES` | `4194304` | The stored content of one page |
| `PENFII_DOC_FILE_ORPHAN_GRACE_SECONDS` | `3600` | How long an attached file that no node uses stays attached |
