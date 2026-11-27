# Configuration

Penfii has two layers of configuration:

1. **`docker/.env`**: the file you edit when you self-host with Docker Compose. Compose reads it and passes selected values to the containers.
2. **Environment variables of the server** (`core-api` and `admin-cli`): what the server actually reads. The Compose file sets some of them from `docker/.env` and leaves the rest at their defaults.

The web app has no configuration of its own; it talks to the server on the same origin through relative URLs.

## How the server reads its settings

- **Not set, empty and blank are the same: unset.** Compose passes optional variables as empty values, and the setting then keeps its default or stays off.
- **A setting that is set must be readable.** A word where a number belongs (`PENFII_FILE_MAX_BYTES=100MB`), a typo in a policy name (`PENFII_MCP_DEFAULT_POLICY=readonly`), a key that is not 64 hex characters: core-api refuses to start and the message names the variable. Nothing is ignored or replaced by a default. Where a setting says `0` means "the default" or "off", `0` is still read that way.
- **Switches** take `1`, `true`, `yes`, `on` or `0`, `false`, `no`, `off`, in any case, and nothing else.
- **A group of settings is set completely or not at all:** the four `PENFII_S3_*` settings (endpoint, bucket, access key, secret key), `PENFII_OPENSEARCH_USER` with `PENFII_OPENSEARCH_PASSWORD` (and both only with `PENFII_OPENSEARCH_URL`), the client id and secret of each sign-in provider (generic OIDC providers also their issuer), the SMTP and Web Push settings as described below. One half of a group, or a provider that cannot be used as configured (Microsoft multi-tenant without `PENFII_AUTH_MICROSOFT_ALLOWED_TENANTS`), stops the start instead of switching the feature off quietly.
- Variables the server does not read are not checked.

## `docker/.env`

Copy `docker/.env.example` to `docker/.env` and replace every `CHANGE_ME`. Never commit `.env` (it is in `.gitignore`).

| Variable | Default in `.env.example` | Meaning |
|---|---|---|
| `PENFII_PUBLIC_URL` | `http://localhost:8080` | Required. The origin people type in the browser, with no path. Sign-in redirects, the co-editing socket and the MCP gateway use it. `https://` also marks the session cookie `Secure`. Compose also passes it to the server as `PENFII_ALLOWED_ORIGINS` |
| `PENFII_HTTP_BIND`, `PENFII_HTTP_PORT` | `127.0.0.1`, `8080` | Address and port on the host where the web container is published |
| `POSTGRES_PASSWORD` | `CHANGE_ME` | Required. PostgreSQL password; letters and digits only |
| `PENFII_S3_ACCESS_KEY`, `PENFII_S3_SECRET_KEY` | `CHANGE_ME` | Required. The MinIO root key pair (8 or more characters). It runs MinIO and the one-shot `minio-setup`; the server uses it as well only while no app key pair is set |
| `PENFII_S3_APP_ACCESS_KEY`, `PENFII_S3_APP_SECRET_KEY` | `CHANGE_ME` | Optional, set both or neither. The account the server connects to MinIO as, limited to objects in the bucket; `minio-setup` makes it. Without them the server keeps using the root key pair and `minio-setup` warns. See [Object storage access key](self-hosting.md#object-storage-access-key) |
| `PENFII_FRONTEND_SUBNET` | `172.29.240.0/24` | Address range of the private network that only the web and core-api containers share. Change it only if it collides with a network of yours |
| `PENFII_SESSION_TTL_SECONDS` | `43200` | Required. Session lifetime |
| `PENFII_LONG_SESSION_ENABLED`, `PENFII_LONG_SESSION_MAX_AGE_SECONDS`, `PENFII_LONG_SESSION_IDLE_SECONDS` | on, `2592000` (30 days), `604800` (7 days) | Optional. The "Keep me signed in" session, see [Long sessions](#long-sessions) |
| `PENFII_MESSAGE_BODY_MAX_BYTES` | `65536` | Required. Largest chat message body |
| `PENFII_INVITE_TTL_SECONDS` | `604800` | Required. Invitation lifetime |
| `PENFII_FILE_MAX_BYTES` | `104857600` | Required. Largest single file |
| `PENFII_MESSAGE_FILE_MAX_COUNT`, `PENFII_MESSAGE_FILE_MAX_BYTES` | `10`, `209715200` | Required. Attachments per message and their total size |
| `PENFII_EDGE_PROXIES` | `private_ranges` | Proxies in front of the web container whose `X-Forwarded-For` is believed (space-separated CIDRs, or `private_ranges`) |
| `PENFII_TRUSTED_PROXIES` | `PENFII_FRONTEND_SUBNET` | The same for core-api (comma-separated CIDRs): by default only the web container, on the private network it alone shares with core-api. That fits a proxy on the same host or no proxy. Add a proxy on another machine here, keeping the frontend range (for example `172.29.240.0/24,192.168.1.20`), or every visitor counts as the proxy's address and shares one sign-in throttle; a proxy with a public address goes in `PENFII_EDGE_PROXIES` too |
| `PENFII_MCP_ENABLED`, `PENFII_MCP_DEFAULT_POLICY`, `PENFII_MCP_DCR_ENABLED` | off, `standard`, off | The MCP gateway; see [below](#mcp-gateway) |
| `PENFII_VAPID_PRIVATE_KEY`, `PENFII_VAPID_SUBJECT` | unset | Web Push; off while unset. See [Web Push](#web-push) |
| `PENFII_SMTP_HOST`, `PENFII_SMTP_PORT`, `PENFII_SMTP_SECURITY`, `PENFII_SMTP_USER`, `PENFII_SMTP_PASSWORD`, `PENFII_SMTP_FROM` | unset | E-mail; off while the host is unset. See [E-mail](#e-mail) |
| `PENFII_WEBHOOK_KEK` | unset | 32-byte key (64 hex characters) that encrypts Signature-mode incoming-webhook secrets; URL-secret webhooks work without it. See [Incoming webhooks](#incoming-webhooks) |
| `PENFII_OPENSEARCH_INDEX_PREFIX` | `penfii` | Start of every OpenSearch index name. Change it only if the OpenSearch is shared |
| `PENFII_DB_MAX_CONNECTIONS` | `10` | PostgreSQL connection pool size |

The Compose file passes only the variables above (and fixes the ones that depend on the stack: the database, OpenSearch and S3 addresses, the bind address). To set any other server variable from the reference below, for example an external sign-in provider, add it under `core-api` → `environment` in `docker/compose.yaml`.

## Server environment reference

Required limits have no built-in default on purpose: when one is unset, the operation that needs it fails with `409` and a code ending in `_unconfigured` instead of guessing.

### Core

| Variable | Default | Effect |
|---|---|---|
| `DATABASE_URL` | required | PostgreSQL connection string for the **runtime** role (row access only). The schema name `penfii` is fixed; the database name is not. core-api and `admin-cli` (except `migrate` and `db-provision`) use this |
| `PENFII_MIGRATION_DATABASE_URL` | unset | Connection string for the schema **owner**. `admin-cli migrate` uses it and refuses to run when it is unset. The owner is not given the runtime statement timeouts |
| `PENFII_PROVISION_DATABASE_URL` | unset | A credential that can create roles (`CREATEDB` is not enough; role creation needs a superuser or a role with `CREATEROLE`). `admin-cli db-provision` uses it. A `pg_dump` does not contain roles, so a restored database needs this again |
| `PENFII_BIND` | `0.0.0.0:8080` | Listen address |
| `PENFII_PUBLIC_URL` | unset | The public origin of the instance (`https://penfii.example.com`, no path). Needed for external sign-in redirect URIs and for MCP. When it starts with `https://`, the session cookie gets `Secure` unless `PENFII_COOKIE_SECURE=0` |
| `PENFII_COOKIE_SECURE` | follows `PENFII_PUBLIC_URL` | A switch: on adds `Secure` to the `penfii_session` cookie, off removes it |
| `PENFII_ALLOWED_ORIGINS` | `http://localhost:5173,http://127.0.0.1:5173` | Comma-separated browser origins that may open a WebSocket and call the API cross-origin, besides the server's own origin. Set it to the public origin when a proxy rewrites `Host`; a value replaces the development defaults. A handshake from another origin is refused with `403 origin_not_allowed`; a client that sends no `Origin` (not a browser) is not affected |
| `PENFII_SESSION_TTL_SECONDS` | unset | Missing or `0` makes login return `session_ttl_unconfigured`. The lifetime of an ordinary session (the cookie has the same `Max-Age`); it is not extended by activity and it is not changed by the long-session settings |
| `PENFII_LONG_SESSION_ENABLED` | on | A switch. Off removes the "Keep me signed in" option and makes every sign-in an ordinary session, whatever a client asks for or a browser remembers. See [Long sessions](#long-sessions) |
| `PENFII_LONG_SESSION_MAX_AGE_SECONDS` | `2592000` | A whole number of at least 1 (at most ten years): how long after sign-in a long session ends at the latest. Activity does not extend it |
| `PENFII_LONG_SESSION_IDLE_SECONDS` | `604800` | A whole number of at least 1: how long a long session lasts without activity. A window longer than the lifetime is the lifetime |
| `PENFII_MESSAGE_BODY_MAX_BYTES` | unset | Missing or `0` makes send and edit return `message_limit_unconfigured` |
| `PENFII_INVITE_TTL_SECONDS` | unset | Missing makes invitation creation return `invite_ttl_unconfigured` |
| `PENFII_TRUSTED_PROXIES` | `127.0.0.0/8,::1/128` | Comma-separated CIDRs (or single addresses) of reverse proxies whose `X-Forwarded-For` is believed; it feeds the per-address sign-in throttle. A malformed entry stops start-up; an empty value trusts nobody. Do not list a network that untrusted machines can reach |
| `PENFII_DB_MAX_CONNECTIONS` | `10` | PostgreSQL pool size, 1 to 500. Not set or empty means `10` (the PostgreSQL default of 100 connections leaves room for the pool, the co-editing connection and `admin-cli`); any other value that is not a whole number in range stops start-up. Co-editing keeps one more connection of its own. A request that waits more than 10 seconds for a connection fails |
| `PENFII_REAUTH_WINDOW_SECONDS` | `600` | How recently a person must have entered their password for sensitive actions: removing a member, disabling an account, purging a ticket, changing MCP policy |
| `PENFII_WORKER` | on | A switch: off (`0`, `false`, ...) stops the event consumers, the reminder scheduler, the scheduled Sprint archive and the trash sweep in this process |
| `PENFII_DEV_SEED` | unset | **Development switch.** On (`1`, `true`, ...) applies the development seed (a demo workspace with known passwords) on start-up, in debug builds only; a release build never seeds |

### Long sessions

The sign-in page (ID and password, and every external provider) has a **Keep me signed in** checkbox, off the first time and then remembered by that browser. A person who ticks it gets a *long session*; everything else about the sign-in is as before (password, second factor, sign-in throttle, recent sign-in for sensitive actions). A long session is not a trusted device: the next sign-in still asks for the code.

- An ordinary session ends `PENFII_SESSION_TTL_SECONDS` after sign-in (12 hours in Compose), whatever the person does.
- A long session ends at the earlier of `PENFII_LONG_SESSION_MAX_AGE_SECONDS` after sign-in (the *absolute* end, never extended) and `PENFII_LONG_SESSION_IDLE_SECONDS` after the last activity (the *inactivity* end). Activity is a request the signed-in person's browser makes to the API (at most one write a minute). What the page does by itself is not: opening or reopening a live-update connection (chat, board, user, Doc page and co-editing sockets) and the periodic check it makes of its session, cursor and sync messages on the co-editing socket (only a change to the document there is activity), and the page's own requests after five minutes without any pointer, key, touch or wheel input (the web client marks them `X-Penfii-Background: 1`, for example the unread count's timer or a refresh after somebody else's change), are answered normally but do not move the inactivity end. An abandoned tab therefore does not keep a long session alive; a request from another API client without that header counts as activity. The server decides; the cookie's `Max-Age` is the absolute lifetime.
- Signing out, "sign out other devices", a changed or reset password, turning two-step verification on or off and disabling the account end long sessions exactly as they end any session. Connected AI apps (MCP) and invitations are not affected: accepting an invitation always starts an ordinary session.
- `PENFII_LONG_SESSION_ENABLED=0` hides the option, and a server that has it off issues only ordinary sessions even for a remembered choice or a direct `keepSignedIn` request.
- **Changing the settings** (they are read at start): when core-api starts it applies them to the long sessions that exist, and it only ever shortens or ends them. Turning the feature off turns every live long session into an ordinary one that ends `PENFII_SESSION_TTL_SECONDS` after it was issued (one older than that ends at once). A shorter lifetime or inactivity window cuts the sessions to the new bounds counted from when they were issued and last seen. A longer value changes nothing for existing sessions (they keep the end they were issued with) and applies to new ones. A value that is not a whole number of at least 1 stops the start with a message that names the variable.

### E-mail

E-mail is optional. Without `PENFII_SMTP_HOST` the server sends nothing, and every feature still works: an invitation is a link that the person who invites copies and passes on (the web app shows it right after the invitation is made or made again; it is shown only then, the server keeps only a hash of it).

| Variable | Default | Effect |
|---|---|---|
| `PENFII_SMTP_HOST` | unset | The SMTP server's host name (no scheme, no path). Setting it turns e-mail on, together with the two variables below |
| `PENFII_SMTP_FROM` | unset | The sender, `penfii@example.com` or `Penfii <penfii@example.com>`. Missing: e-mail stays off and the start-up log says why |
| `PENFII_PUBLIC_URL` | unset | Links in the mail start with it, so it must be the address people really open, with no path. Missing or not an origin: e-mail stays off and the start-up log says why |
| `PENFII_SMTP_SECURITY` | `starttls` | `starttls` (the connection is upgraded to TLS and must be), `tls` (TLS from the first byte, SMTPS) or `none` (no TLS: only for a relay on a network you trust) |
| `PENFII_SMTP_PORT` | `587` for `starttls`, `465` for `tls`, `25` for `none` | |
| `PENFII_SMTP_USER`, `PENFII_SMTP_PASSWORD` | unset | Set both or neither. Refused with `PENFII_SMTP_SECURITY=none`: the password would cross the network in the clear |
| `PENFII_SMTP_TIMEOUT_SECONDS` | `10` | How long one message may take (1 to 120). A slower server counts as a failed delivery |

The server's certificate is checked against the system's certificate authorities and the name in `PENFII_SMTP_HOST`; there is no setting that turns the check off. For a server with a private authority, add that authority to the container's system store (or use a relay that has a normal certificate). A value that cannot be right (an unknown mode, a port that is not a number, a user without a password, credentials with `none`, a sender that is not an address) stops start-up.

**Invitations.** There is one valid open invitation per e-mail address in a Workspace. Inviting an address that already has one (or choosing "New link" in the invitation list) gives that invitation a new link and a new expiry (`PENFII_INVITE_TTL_SECONDS`, 7 days in the example) and the previous link stops working at once. An invitation that has expired is not an open one: inviting the address again makes a new invitation, and the old one stays in the list as expired. "New link" on an expired invitation that nothing has replaced sends that same invitation again. When e-mail is on, the link is mailed right after the invitation is saved, in the language of the invited person's account, or the instance's default language when they have none. A mail that cannot be sent (the server is down, refuses it, or does not answer in time) never fails the invitation: the list shows the delivery as failed, the link still works and can be copied, and "New link" tries again with a fresh link. The delivery state is only about the e-mail; it does not change whether the invitation is pending, expired, accepted or revoked. The mail is sent once, while the request is answered, without a queue and without automatic reminders.

The invitation link, the SMTP password and the reply of the SMTP server are never written to the log, the audit log or the delivery state; a failed delivery is recorded as one fixed phrase such as `the SMTP server refused the message`.

**Password reset.** While e-mail is on, the login page offers "Forgot your password?". The person gives their address and is answered the same way whatever the address is: the page only says that a link is on its way if there is an account, and the server answers `202` at the same speed for an account, an address with none, a disabled account and an account that has no password (one that only signs in through a provider). Only an enabled account with a password is mailed, in the background, after the answer. The link (`/reset-password#<token>`) works once and for 30 minutes, and a newer request replaces it. The token travels in the part of the address after `#`, which a browser never sends to the server, so it is in no access log and in no Referer header; the web container also sends `Referrer-Policy: no-referrer` for that page. Choosing a new password follows the usual rule (15 to 128 characters, not a known weak one), signs the account out of every session and does not sign anyone in. The token and the passwords are never logged or written to the audit log (`user.password_reset_requested` and `user.password_reset` name only the account). Without e-mail there is no reset page, and an operator sets the password with `admin-cli user-set-password`.

**Changing the e-mail address.** A person changes the address of their account on the account page. The server mails a link (`/confirm-email#<token>`, once, 24 hours) to the **new** address, after a recent authentication; the address changes only when that link is opened, and until then the account keeps signing in and recovering with its current address. Opening the link makes the new address the account's verified address, closes the account's open reset links and tells the old address. At most 5 requests per account and 3 mails per address per hour. Without e-mail (`PENFII_SMTP_*` unset) the page says the address cannot be changed there and the operator uses `admin-cli user-set-email`, which leaves the new address unverified: no password reset is mailed to an unverified address, and the person confirms it from the account page once e-mail is on. Accounts that sign in through a provider keep their provider link when the address changes.

**Accepting an invitation as an existing account.** An invitation to an address that already has an account is accepted by signing in as that account and opening the link; the invitation never asks for the account's password, so a link cannot be used to guess it. An invitation to `OWNER` or `ADMIN` also asks for a recent authentication. A new address sets a password (or uses a provider) as before.

**Sign-in and session audit.** The audit log records a sign-in (`user.login`, with whether a second factor was used), ending one's own sessions (`user.session_revoked`, `user.sessions_revoked` with a count), removing a way to sign in (`user.sign_in_method_removed`) and, for an existing account, the moment a sign-in, second-factor or reauthentication counter locks (`user.login_locked`, `user.mfa_login_locked`, `user.reauth_locked`). Failed attempts are not logged one by one, and failures against an unknown e-mail or from one address never reach the audit log, so strangers cannot grow it: at most one lock entry per account and lock window. No entry holds a password, token, code, address or session cookie.

**Two-step verification (TOTP).** A person can turn on an authenticator app (Account, "Two-step verification"; codes of 6 digits every 30 seconds, SHA-1, as every authenticator app expects) and gets ten single-use recovery codes, shown once. It is off by default and there is no way to force it. Once it is on, **signing in with the account's own password also needs a current code or an unused recovery code**: `POST /v1/session/login` answers `401 mfa_required` for a right password without `code`, and the same request with `"code"` signs in; a wrong password, a wrong code and an unknown account answer alike (`401 unauthenticated`). A sign-in through an external provider is that provider's own authentication and is not asked for a code. Turning it on needs a recent authentication and a code of the new secret; turning it off and getting new recovery codes need a recent authentication **and** a current code or recovery code; every other sensitive action keeps its ordinary proof (the password, or the provider). A code is good once (the step of a used code is spent, a recovery code is marked used in the statement that finds it). Wrong codes are counted before they are checked: the code step of a sign-in uses the sign-in counters plus a per-account one (10 wrong codes in 15 minutes lock it), and the signed-in person's own attempts have a per-account counter of the same size. Turning it on or off signs the other devices out and keeps this one and the connected apps (MCP). If someone loses both the authenticator and the recovery codes, the operator runs `admin-cli user-mfa-reset --email ...`, which turns it off and ends **every** session and connected app of that account (written to the audit log as `user.mfa_reset`); enabling, disabling and replacing codes are audited too (`user.mfa_enabled`, `user.mfa_disabled`, `user.mfa_recovery_regenerated`) and never record a code or a secret. The authenticator secret is stored encrypted (AES-256-GCM); see [the encryption key](#two-step-verification-key) and, for backups, [Operations](operations.md#two-step-verification-and-backups).

| Variable | Default | Effect |
|---|---|---|
| `PENFII_MFA_ENCRYPTION_KEY` | unset | Optional. 32 bytes as 64 hex characters (`openssl rand -hex 32`) that encrypt authenticator secrets. Unset: a random key is made on first use and kept in the database (`penfii.mfa_instance_key`), so enabling two-step verification needs no setting. Set before the first person enables it, or later: secrets are sealed under the key they name, new ones use this key and old ones keep opening with the database key |

<a id="two-step-verification-key"></a>**The encryption key.** The key kept in the database protects the secrets from a copy of one table or from a log, not from a copy of the whole database, because a database backup then holds the key and the secrets together. For that, set `PENFII_MFA_ENCRYPTION_KEY` (and keep it with the other secrets, outside the backup of the database). A value that is set but is not 64 hex characters stops the start.

**Sessions and connected apps.** Changing the password on the account page keeps the current device signed in, signs the other devices out and leaves connected apps (MCP) connected. A password reset by link, `admin-cli user-set-password` and disabling an account end every session and every connected app. "Sign out other devices" ends web sessions only, and "Disconnect all apps" ends apps only. Open live sockets (chat, Project board, user, Doc page and co-editing) are closed with code 4401 within about 15 seconds of the end of the session they were opened with.

Reset requests are counted on counters of their own, with the same keys and limits as [the sign-in throttle](#sign-in-throttle) (an address and an account together, an account, an address; `PENFII_LOGIN_THROTTLE_*` changes both): every request counts, not only failures, and a counter that is full lets nothing through, silently, until its window is over. Signing in and asking for a reset do not use each other's budget.

### Sign-in throttle

Failed password proofs are counted in PostgreSQL and lock a key for one window once the key has made too many. Every sign-in attempt is counted in all three keys *before* the password is looked at, in one locked step, so requests made at the same moment cannot all get past a counter that has room for one; the first counter that is full refuses the attempt (`429 rate_limited` with the retry time), whether the password is right or wrong. A right password then clears the e-mail counters and gives the address counter its attempt back. Re-authentication uses the same step. An unknown e-mail is counted and answered like a known one.

| Key | Default | Variables (`PENFII_LOGIN_THROTTLE_` + ...) |
|---|---|---|
| one e-mail from one address | 5 per 15 minutes | `ACCOUNT_ADDRESS_FAILURES`, `ACCOUNT_ADDRESS_WINDOW_SECONDS` |
| one e-mail from anywhere | 20 per 15 minutes | `ACCOUNT_FAILURES`, `ACCOUNT_WINDOW_SECONDS` |
| one address, any e-mail (also counts OAuth sign-in starts) | 30 per 15 minutes | `ADDRESS_FAILURES`, `ADDRESS_WINDOW_SECONDS` |

A value that is not a positive whole number stops start-up.

### Passwords

A password that someone sets (creating an account, changing it in the profile, accepting an invitation, `admin-cli bootstrap`, `create-user`, `user-set-password`) must be 15 to 128 characters and must not be a known weak password. There are no composition rules and no expiry. Passwords are stored as Argon2id. Signing in does not apply the rule, so a password set under an earlier rule keeps working until it is changed.

### Files

File attachments need an S3-compatible bucket that accepts path-style requests. The server creates the bucket on start. Without storage configured, uploads are unavailable.

| Variable | Default | Effect |
|---|---|---|
| `PENFII_S3_ENDPOINT`, `PENFII_S3_BUCKET`, `PENFII_S3_ACCESS_KEY`, `PENFII_S3_SECRET_KEY` | unset | All four are required to enable file storage |
| `PENFII_S3_REGION` | `us-east-1` | Signing region |
| `PENFII_S3_ENSURE_BUCKET` | on | On, the server tries to create the bucket when it starts (development, a hand-made MinIO). The Compose file sets it to `0`: its `minio-setup` service makes the bucket, so the app account needs no right to |
| `PENFII_FILE_MAX_BYTES` | unset | Largest single file; unset makes uploads fail with `file_limit_unconfigured` |
| `PENFII_MESSAGE_FILE_MAX_COUNT`, `PENFII_MESSAGE_FILE_MAX_BYTES` | unset | Attachments per message and their total size (ticket attachments are not limited by these) |
| `PENFII_UNBOUND_UPLOAD_GRACE_SECONDS` | `86400` | How long an upload that was never attached may wait before it is removed |
| `PENFII_TRASH_RETENTION_DAYS` | unset (off) | Days a ticket stays in the trash before it is purged automatically. Unset, empty or `0` keeps the automatic purge **off** (anything else that is not a whole number of at least 0 stops the start): trashed tickets stay until a Project admin restores or purges them. When set, the purge runs hourly and shortly after start (so a server that was down catches up); a ticket that still has children or dependencies is kept and tried again |

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

Clients are public: they authenticate at the token endpoint with `none` and PKCE, never with a secret. A client identified by a Client ID Metadata Document (an `https://` `client_id`) is accepted when its `token_endpoint_auth_methods_supported`, or the singular `token_endpoint_auth_method` when the list is absent, includes `none`; other methods may be listed beside it (ChatGPT lists `private_key_jwt`). A document without either is taken as `none`. A token request that authenticates the client anyway is answered `401 invalid_client`. When a client is refused, the person sees "This application could not be identified" and core-api logs the reason (`WARN oauth client refused`).

### Web Push

Web Push delivers four kinds of Inbox item to the browsers of the person they are for, even when Penfii is closed: mentions, direct replies, Task assignments and reminders that came due. Nothing else is pushed, quiet hours (Do Not Disturb) send nothing, and a push carries no message text and no workspace name.

| Variable | Default | Effect |
|---|---|---|
| `PENFII_VAPID_PRIVATE_KEY` | unset | The server's VAPID private key (32 bytes, base64url). Unset together with `PENFII_VAPID_SUBJECT`: Web Push is off, the server starts normally and the settings screen hides the switch. One set without the other, or a key that is not valid, stops start-up. It is a secret: keep it in the environment or a secret store, never in the database, and do not change it casually (browsers subscribed with the old public key stop receiving pushes) |
| `PENFII_VAPID_SUBJECT` | unset | A contact the push services can reach: `mailto:ops@example.com` or `https://...` |

Make a key pair with `admin-cli vapid-keys` (it needs no database). It prints `PENFII_VAPID_PRIVATE_KEY=...` and the matching public key. The browser gets the public key from the API (`GET /v1/push/vapid-public-key`). Browsers need `https://` (or `localhost`) to subscribe. On iPhone and iPad, Web Push works only for the app added to the Home Screen (iOS 16.4 or later). The server must be able to reach the browsers' push services over HTTPS (for example `fcm.googleapis.com`, `updates.push.services.mozilla.com`, `web.push.apple.com`).

### Incoming webhooks

A channel manager can add an incoming webhook to a channel (channel settings, recent sign-in required). Each one posts plain-text messages (`POST /v1/hooks/<credential>` with `{"text": "..."}`) into that one channel as an Integration Actor, which cannot sign in or join anything. A webhook is either **URL secret** (the secret is the path) or **Signature** (HMAC-SHA256 of `<X-Penfii-Timestamp>.<body>` in `X-Penfii-Signature: v1=<hex>`, the webhook id in the path). The secret is shown only when the webhook is created or rotated.

| Variable | Default | Effect |
|---|---|---|
| `PENFII_WEBHOOK_KEK` | unset | 32 bytes as 64 hex characters (for example `openssl rand -hex 32`). Signature webhooks keep their secret encrypted with it, so while it is unset they cannot be created, rotated or verified; URL-secret webhooks are unaffected. It is a secret: keep it in the environment or a secret store. Losing or changing it makes existing Signature webhooks fail until each is rotated |

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
