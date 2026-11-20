# Penfii documentation

Penfii is at v0.3.1, an early release. These documents describe the repository as it is today; where something is not decided or not documented yet, the page says so.

## For people who use Penfii

| Page | What it covers |
|---|---|
| [User guide](user-guide.md) · [사용자 가이드](user-guide.ko.md) | Joining a workspace, channels and threads, making a ticket from a message, the Board and Table, linking tickets and documents, search, Inbox, and personal settings. The same guide in English and Korean |

## For people who run Penfii

| Page | What it covers |
|---|---|
| [Self-hosting](self-hosting.md) | Requirements, installing with Docker Compose, the first account, where the data lives, backups, TLS, the optional MCP gateway, and [upgrades](self-hosting.md#updating) |
| [Configuration](configuration.md) | The `docker/.env` file and the full reference of server environment variables |
| [Operations](operations.md) | `admin-cli` commands, health checks, the search index, failed event deliveries |
| [Known limitations](known-limitations.md) | What is missing or not yet safe to rely on |

## For people who work on Penfii

| Page | What it covers |
|---|---|
| [Architecture](architecture.md) | The parts of the system and the rules that hold them together |
| [Development](development.md) | Local setup, running the server and the web app, the checks, migrations, the API description |
| [Contributing](../CONTRIBUTING.md) | How to propose and submit a change, and what happens to a pull request |
| [Releases](development.md#releases) | How a release is numbered, tagged and recorded, and which CI checks run where |

## For people who build on Penfii

| Page | What it covers |
|---|---|
| [Using the API](api.md) | Where the API is described, how to sign in, the headers a change needs, errors, and the pointers to MCP and incoming webhooks |
| [`server/api/openapi.json`](../server/api/openapi.json) | The API description: every route with its parameters, bodies and errors |
| [MCP gateway](self-hosting.md#the-mcp-gateway-optional) · [its settings](configuration.md#mcp-gateway) | Letting AI clients act for a person who approves them |

## Also in this repository

- [Security policy](../SECURITY.md): supported versions, how to report a vulnerability.
- [License](../LICENSE) and third-party notices ([server](../server/THIRD-PARTY-NOTICES.md), [client](../client/THIRD-PARTY-NOTICES.md)).
- [Code of Conduct](../CODE_OF_CONDUCT.md).

## Not written yet

These are planned and are not covered anywhere yet:

- A hand-written reference for every HTTP endpoint. The API is described by [`server/api/openapi.json`](../server/api/openapi.json), with [Using the API](api.md) as the way in.
