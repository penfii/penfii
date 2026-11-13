# Security policy

## Supported versions

Penfii is at v0.1.0. Only the latest release and the latest commit on the default branch are supported; fixes are not backported.

## Reporting a vulnerability

**Please do not open a public issue, pull request or discussion for a security problem.**

Report it privately by e-mail to **[bang9ming9@gmail.com](mailto:bang9ming9@gmail.com)**. If the repository host offers private vulnerability reporting (on GitHub: the **Security** tab, then **Report a vulnerability**), you may use that instead. Include:

- what you found and why it matters (what an attacker gains),
- the steps, request or proof of concept needed to reproduce it,
- the commit or version you tested, and how you ran it.

What to expect: we will acknowledge a report, tell you whether we consider it a vulnerability, and keep you informed until it is fixed. No response-time commitment can be made yet because the project has no dedicated security team. Please give us a reasonable time to fix a problem before you disclose it.

## What is in scope

The server (`server/`):

- Authentication and sessions (password sign-in, external providers, session cookies, CSRF protection, re-authentication).
- Authorization: reading or changing anything across a workspace, project, private channel, direct message or doc without permission; privilege escalation between workspace roles; access that survives removing a member or disabling an account.
- The MCP gateway and OAuth authorization server (token handling, consent, scopes, workspace policy).
- Server-side request forgery, injection, unsafe file handling, and leaks of secrets or other people's data through APIs, search, logs or error messages.

The web app (`client/`):

- Cross-site scripting or HTML injection through rich text, links, file names, user names or any other user-controlled content.
- Handling of the session cookie, the CSRF header and the sign-in or consent flows in the browser.
- Showing controls or data that the signed-in role must not see (the server is the authority, but a UI that leaks data is still a bug).
- Secrets or personal data ending up in the built bundle, logs or local storage.

The Docker stack (`docker/`): defaults that leave a default installation exposed.

Out of scope: findings that need a modified or malicious server or an already compromised browser, denial of service by sending huge volumes of traffic, and missing hardening on a deployment you configured with the development settings below.

## Running it safely

These settings exist for local development and must not be used on a network you do not control:

- `make run` in `server/` and the development seed (`alice@ourspace.dev`, password `1234`, debug builds only) are for development.

For a real deployment, see [docs/self-hosting.md](docs/self-hosting.md). In particular:

- Put TLS in front of the web container and set `PENFII_PUBLIC_URL` to the `https://` origin.
- Behind a reverse proxy, set `PENFII_TRUSTED_PROXIES` and `PENFII_EDGE_PROXIES` to that proxy and nothing else; the sign-in throttle reads the client address from `X-Forwarded-For` only from the proxies you name.
- Keep OpenSearch, PostgreSQL and the object store unreachable from outside the Compose network. The bundled OpenSearch runs with its security plugin disabled.
- Replace every `CHANGE_ME` in `docker/.env` and keep that file private.
- E-mail is optional (`PENFII_SMTP_*`). Penfii checks the SMTP server's certificate and has no setting to skip that; plain SMTP (`none`) is refused together with a user name and password. Set `PENFII_PUBLIC_URL` to the real `https://` address, because the links in invitation and password-reset mail start with it. A password-reset token travels in the address fragment and is stored only as a hash; it works once for 30 minutes.
- The password rule (15 to 128 characters, not a known weak password) applies to every password that is set, and sign-in throttling is configurable (`PENFII_LOGIN_THROTTLE_*`); see [docs/configuration.md](docs/configuration.md).
- Back up PostgreSQL and the file store ([backup](docs/self-hosting.md#backup)).
