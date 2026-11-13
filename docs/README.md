# Penfii documentation

Penfii is at v0.1.0, an early release. These documents describe the repository as it is today; where something is not decided or not documented yet, the page says so.

## For people who run Penfii

| Page | What it covers |
|---|---|
| [Self-hosting](self-hosting.md) | Requirements, installing with Docker Compose, the first account, where the data lives, backups, TLS, the optional MCP gateway |
| [Configuration](configuration.md) | The `docker/.env` file and the full reference of server environment variables |
| [Operations](operations.md) | `admin-cli` commands, health checks, the search index, failed event deliveries |
| [Known limitations](known-limitations.md) | What is missing or not yet safe to rely on |

## For people who work on Penfii

| Page | What it covers |
|---|---|
| [Architecture](architecture.md) | The parts of the system and the rules that hold them together |
| [Development](development.md) | Local setup, running the server and the web app, the checks, migrations, the API description |
| [Contributing](../CONTRIBUTING.md) | How to propose and submit a change |

## Also in this repository

- [Security policy](../SECURITY.md): supported versions, how to report a vulnerability.
- [License](../LICENSE) and third-party notices ([server](../server/THIRD-PARTY-NOTICES.md), [client](../client/THIRD-PARTY-NOTICES.md)).
- [Code of Conduct](../CODE_OF_CONDUCT.md).

## Not written yet

These are planned and are not covered anywhere yet:

- An upgrade procedure between releases.
- A reference for the HTTP API beyond [`server/api/openapi.json`](../server/api/openapi.json).
- End-user documentation for the web app.
- A documented release process. Changes per release are listed in the [changelog](../CHANGELOG.md).
