# Self-hosting

`docker/compose.yaml` is the supported way to run Penfii. It starts PostgreSQL, OpenSearch (with the Korean analyzer), MinIO (S3-compatible file storage), the server (`core-api`) and the web app, and builds the server and web images from this checkout. There are no published images.

Penfii is at v0.1.0, an early release: read [Known limitations](known-limitations.md) first.

## What you need

- Docker with Compose v2.
- Memory: OpenSearch runs with a 512 MB Java heap. On a nearly empty instance its container used about 1.4 GiB, so plan for a host with 4 GiB or more for the whole stack. These are observations from a development instance, not a sizing guide.
- On Linux hosts, OpenSearch's own documentation asks for `vm.max_map_count=262144` for production use.
- Time for the first start: it compiles the server in release mode and builds the web app, which takes several minutes. Later starts reuse the Docker cache.

## Install

```sh
git clone https://git.manglix.com/penfii/penfii.git
cd penfii/docker
cp .env.example .env
```

Edit `.env`:

- Replace every `CHANGE_ME`: the PostgreSQL password (letters and digits only, it is placed inside a connection URL) and the object storage key pair (8 or more characters).
- Set `PENFII_PUBLIC_URL` to the address people will open, with no path (`https://penfii.example.com`). The default is `http://localhost:8080`.
- Keep the limits in the file or change them; core-api needs each of them (see [Configuration](configuration.md)).

Then build and start:

```sh
docker compose up -d --build
docker compose ps        # every service should become "healthy"
```

Compose refuses to start and names the variable when a required value is missing from `.env`.

## Create the first account

There is no sign-up page. Create the first account and workspace once, with the operator tool:

```sh
docker compose run --rm admin-cli bootstrap \
  --email owner@example.com --password 'choose-a-long-password' \
  --display-name Owner --workspace-name Penfii --time-zone Asia/Seoul
```

A password must be 15 to 128 characters and must not be a known weak password. `bootstrap` refuses to run when any user already exists.

### Adding other people

Penfii does not send e-mail, and how an invitation reaches a person is not decided yet (see [Known limitations](known-limitations.md#invitations-and-e-mail)). What exists today:

- `admin-cli create-user` (same arguments as `bootstrap`) creates another account **with a workspace of its own**; it does not add the person to an existing workspace.
- Owners and admins can create invitations in the web app. The invitation link token is returned to the inviter only when the server runs with `PENFII_DEV_RETURN_INVITE_TOKEN=1`, a development switch that should not be left on for a network you do not control.

## Check that it works

Open `PENFII_PUBLIC_URL` and sign in. A quick end-to-end check: create a project and a ticket, attach a file to the ticket and download it again, then search for the ticket title.

Health probes, through the web container:

```sh
curl http://localhost:8080/healthz     # liveness
curl http://localhost:8080/readyz      # readiness: PostgreSQL and search
```

## Where the data lives

The stack keeps its data in named volumes. They survive `docker compose down` and container re-creation. Never use `docker compose down -v` unless you mean to erase everything.

| Volume | Holds | Back it up? |
|---|---|---|
| `penfii_penfii-pg` | PostgreSQL: all workspace data | **Yes** |
| `penfii_penfii-files` | MinIO: uploaded files | **Yes** |
| `penfii_penfii-os` | The OpenSearch index | No. It is derived data; rebuild it with `docker compose run --rm admin-cli search-reindex` |

### Backup

Back up PostgreSQL and the file store together; a ticket refers to files by id. For PostgreSQL:

```sh
docker compose exec -T postgres pg_dump -U penfii penfii > penfii.sql
```

For the file store, copy the `penfii_penfii-files` volume, for example:

```sh
docker run --rm -v penfii_penfii-files:/data -v "$PWD":/backup alpine \
  tar czf /backup/penfii-files.tar.gz -C /data .
```

A tested restore procedure is not written yet (see [Not written yet](README.md#not-written-yet)). Try your backups on a throwaway copy before you rely on them.

## Network and TLS

By default the web container is published on `127.0.0.1` only (`PENFII_HTTP_BIND`, `PENFII_HTTP_PORT`). PostgreSQL, MinIO, OpenSearch and core-api are not published at all; OpenSearch runs with its security plugin disabled, so it must stay unreachable from outside the Compose network.

For anything beyond your own machine, put a TLS reverse proxy in front of the web container:

- Forward every path to the web container (it forwards `/v1`, `/oauth`, `/mcp`, `/.well-known/*`, `/healthz` and `/readyz` to core-api and serves the app for the rest), and let WebSocket upgrades through.
- Set `PENFII_PUBLIC_URL` to the public `https://` origin. A value starting with `https://` also marks the session cookie `Secure`, so use it only when TLS really is in front.
- Add `Strict-Transport-Security` at the proxy; the web container leaves it to the proxy.
- Sign-in attempts are throttled per client address, taken from `X-Forwarded-For` only when the request comes from a proxy you name. The defaults trust private address ranges, which fits a proxy on the same machine or private network. If your proxy has a public address (a cloud load balancer, a CDN), list it in both `PENFII_EDGE_PROXIES` and `PENFII_TRUSTED_PROXIES`, otherwise all visitors share one throttle. The commented examples in `docker/.env.example` show the format.

To use a different port, change `PENFII_HTTP_PORT` and the port in `PENFII_PUBLIC_URL` together.

## The MCP gateway (optional)

The MCP gateway lets AI clients work in Penfii as a signed-in person, after that person approves the client in the browser. It is off by default. To turn it on, set `PENFII_MCP_ENABLED=1` in `.env` and run `docker compose up -d` again. MCP clients then connect to `PENFII_PUBLIC_URL/mcp`. `PENFII_PUBLIC_URL` must be `https://` (or `http://localhost` / `http://127.0.0.1`), otherwise core-api refuses to start. Set `PENFII_MCP_DCR_ENABLED=1` as well for clients that register themselves. The other MCP settings are in [Configuration](configuration.md#mcp-gateway).

## Updating

```sh
git pull
cd docker
docker compose up -d --build
```

core-api applies pending database migrations when it starts. This update procedure has not been tested across releases, and there is no upgrade guarantee between 0.x releases, so back up first and read the [changelog](../CHANGELOG.md). A database made by a development build from before the schema was consolidated into `database/migrations/0001_initial.sql` cannot be upgraded in place and has to be recreated.

## Stopping and removing

```sh
docker compose down        # stop and remove the containers; the volumes stay
docker compose down -v     # ALSO erases the volumes: all data is lost
```

## Your own PostgreSQL, S3 or OpenSearch

core-api takes all three through environment variables (`DATABASE_URL`, `PENFII_S3_*`, `PENFII_OPENSEARCH_*`; see [Configuration](configuration.md)), so you can remove a service from the Compose file and point core-api at yours. That is not packaged or tested here. OpenSearch must have the `analysis-nori` plugin, and the S3 store must accept path-style requests (only MinIO is exercised).

## Troubleshooting

- `docker compose logs core-api` shows why the server did not start. It refuses to start on an invalid setting (for example a malformed `PENFII_TRUSTED_PROXIES`, or MCP enabled without a suitable `PENFII_PUBLIC_URL`) and names the variable.
- Search returns 503 or `/readyz` reports `degraded`: OpenSearch is down or unhealthy. Chat, projects and docs keep working. See [Operations](operations.md#search).
- Sign-in fails with `session_ttl_unconfigured` (or another `*_unconfigured` code): a required limit is missing from the server's environment.
