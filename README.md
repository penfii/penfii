# Penfii

Penfii is a self-hosted workspace that keeps **chat, projects and tickets, and documents** in one place, under one permission model.

> **Status: v0.1.0, the first public release.** Penfii is early software. The HTTP API, the database schema and the settings can still change between 0.x releases, and there is no tested upgrade path between releases. Read [Known limitations](docs/known-limitations.md) before you run it for other people.

## What it does

- **Chat**: public and private channels, direct messages, threads, reactions, read state, drafts, file attachments (S3-compatible storage), rich text.
- **Projects and tickets**: workflow states, Epic / Task / Subtask hierarchy, Board / List / Table views and saved views, labels, dependencies, Sprints, Milestones, estimates and schedules, a timeline, revision history with restore, archive (also when a Sprint is completed), a trash with administrative purge.
- **Docs**: a page tree, saved versions with restore, templates, comments, and references between pages and tickets.
- **Search** over messages, tickets, docs and files on OpenSearch with the Korean analyzer (`nori`). Results are re-checked against PostgreSQL permissions before they are shown.
- **Attention**: Inbox, Watch, Later, Reminders and notification preferences.
- **Sign-in**: password, plus optional Google, Microsoft, GitHub and generic OIDC providers. People join a workspace by invitation.
- **MCP gateway** (off by default): an OAuth authorization server and an MCP endpoint so AI clients can act as a person, under a per-workspace policy and with an audit trail.
- **Administration**: members and roles, invitations, account disabling, team membership, an audit log.

The server decides what each person may see and do, from PostgreSQL, on every request. The web client only hides what a role cannot use.

## Repository layout

| Path | Contents |
|---|---|
| [`server/`](server) | The Rust server: HTTP API, WebSockets, background workers, and the `admin-cli` operator tool. `server/api/openapi.json` is the API description. |
| [`client/`](client) | The web app (React, TypeScript, Vite), organized as an npm workspace of `apps/` and `packages/`. |
| [`database/`](database) | The PostgreSQL schema as SQL migrations (`database/migrations`). They are compiled into the server and applied by `admin-cli migrate` (in the Compose stack, the one-shot `db-setup` service); core-api only checks them. |
| [`docker/`](docker) | The Docker Compose self-hosting stack and the Dockerfiles for the server, the web app and OpenSearch. |
| [`docs/`](docs) | Documentation. |
| [`.github/`](.github) | CI, Dependabot, issue forms and the pull request template. |

## Quick start (self-hosting)

You need Docker with Compose v2 and a host with at least 4 GiB of memory. There are no published images, so Compose builds them from this checkout; the first start takes several minutes.

```sh
git clone https://git.manglix.com/penfii/penfii.git
cd penfii/docker
cp .env.example .env     # replace every CHANGE_ME, set PENFII_PUBLIC_URL
docker compose up -d --build
docker compose run --rm admin-cli bootstrap \
  --email owner@example.com --display-name Owner --workspace-name Penfii --time-zone Asia/Seoul
```

The command asks for the owner's password (15 to 128 characters; it is not shown on screen). For a script, pipe it in with `--password-stdin` instead; do not put it on the command line.

Then open <http://localhost:8080> and sign in. The full guide, including data, backups, TLS and upgrades, is in [docs/self-hosting.md](docs/self-hosting.md).

## Documentation

See [docs/README.md](docs/README.md) for the index:

- [Self-hosting](docs/self-hosting.md): install, start, back up, put a TLS proxy in front.
- [Configuration](docs/configuration.md): every setting.
- [Operations](docs/operations.md): `admin-cli`, search index, health checks.
- [Architecture](docs/architecture.md): how the pieces fit together.
- [Development](docs/development.md): run it locally, run the checks, change the schema or the API.
- [Known limitations](docs/known-limitations.md).
- [Changelog](CHANGELOG.md).

## Contributing, security and license

- Contributions are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md).
- Report vulnerabilities privately: see [SECURITY.md](SECURITY.md). Do not open a public issue for them.
- Copyright © 2026 Penfii contributors. Penfii is licensed under the **GNU General Public License v3.0 only** (`GPL-3.0-only`); see [LICENSE](LICENSE). Third-party components and their licenses are listed in [server/THIRD-PARTY-NOTICES.md](server/THIRD-PARTY-NOTICES.md) (Rust crates) and [client/THIRD-PARTY-NOTICES.md](client/THIRD-PARTY-NOTICES.md) (npm packages).
