# Self-hosting

`docker/compose.yaml` is the supported way to run Penfii. It starts PostgreSQL, OpenSearch (with the Korean analyzer), MinIO (S3-compatible file storage), the server (`core-api`) and the web app, and builds the server and web images from this checkout. There are no published images.

Penfii is at v0.3.1, an early release: read [Known limitations](known-limitations.md) first.

## What you need

- Git and Docker with Compose v2.
- Memory: OpenSearch runs with a 512 MB Java heap. On a nearly empty instance its container used about 1.4 GiB, so plan for a host with 4 GiB or more for the whole stack. These are observations from a development instance, not a sizing guide.
- On Linux hosts, OpenSearch's own documentation asks for `vm.max_map_count=262144` for production use.
- Time for the first start: it compiles the server in release mode and builds the web app, which takes several minutes. Later starts reuse the Docker cache.

## Install

```sh
git clone https://github.com/penfii/penfii.git
cd penfii
git checkout v0.3.1      # a release tag; leave this out to run the latest commit of the default branch
cd docker
cp .env.example .env
```

Edit `.env`:

- Replace every `CHANGE_ME`: the PostgreSQL password and the passwords of the two database roles (`PENFII_DB_OWNER_PASSWORD`, `PENFII_DB_RUNTIME_PASSWORD`; letters and digits only, they are placed inside a connection URL) and the object storage key pair (8 or more characters).
- Set `PENFII_PUBLIC_URL` to the address people will open, with no path (`https://penfii.example.com`). The default is `http://localhost:8080`.
- Keep the limits in the file or change them; core-api needs each of them (see [Configuration](configuration.md)).

Then build and start:

```sh
docker compose up -d --build
docker compose ps --all  # long-running services should be healthy; db-setup should exit with code 0
```

Compose refuses to start and names the variable when a required value is missing from `.env`.

## Create the first account

There is no sign-up page. Create the first account and workspace once, with the operator tool:

```sh
docker compose run --rm admin-cli bootstrap \
  --email owner@example.com --display-name Owner --workspace-name Penfii --time-zone Asia/Seoul
```

The command asks for the password without showing it (and again to confirm). In a script, pipe it in instead: `printf '%s' "$PW" | docker compose run --rm -T admin-cli bootstrap ... --password-stdin`. Avoid `--password <value>`: it stays in the shell history and the process list. Use 15 to 128 characters and avoid known weak passwords. `bootstrap` refuses to run when any user already exists.

### Adding other people

An invitation is a link, and sending it by e-mail is optional:

- Owners and admins invite people in the web app (Administration, Invitations). The app shows the invitation link right away; copy it and pass it on. This works without any e-mail setup. The link is not stored and cannot be shown again: "New link" in the list makes a new one, and the previous one stops working.
- To have Penfii mail the link as well, set the `PENFII_SMTP_*` variables in `docker/.env` (see [E-mail](configuration.md#e-mail)) and set `PENFII_PUBLIC_URL` to the address people really open: the links in the mail start with it. A mail that fails to send does not fail the invitation.
- With e-mail on, people who forget their password reset it themselves from the login page; without e-mail an operator sets it with `admin-cli user-set-password`.
- `admin-cli create-user` creates a new account and adds it to an existing workspace (`--workspace <id>`, from `admin-cli workspace-list`; `--role` defaults to `MEMBER`). For an account that already exists, use `admin-cli workspace-add-member`. See [Operations](operations.md#adding-people-to-a-workspace).

## Check that it works

Open `PENFII_PUBLIC_URL` and sign in. A quick end-to-end check: create a project and a ticket, attach a file to the ticket and download it again, then search for the ticket title.

Health probes, through the web container:

```sh
curl http://localhost:8080/healthz     # liveness
curl http://localhost:8080/readyz      # readiness: PostgreSQL, search, and background workers
```

## Where the data lives

The stack keeps its data in named volumes. They survive `docker compose down` and container re-creation. Never use `docker compose down -v` unless you mean to erase everything.

| Volume | Holds | Back it up? |
|---|---|---|
| `penfii_penfii-pg` | PostgreSQL: all workspace data | **Yes** |
| `penfii_penfii-files` | MinIO: uploaded files | **Yes** |
| `penfii_penfii-os` | The OpenSearch index | No. It is derived data; rebuild it with `docker compose run --rm admin-cli search-reindex` |

### Backup

Back up PostgreSQL and the file store together; a ticket refers to files by id. PostgreSQL is a custom-format dump (roles are not in it):

```sh
docker compose exec -T postgres pg_dump -U penfii -Fc -f /tmp/penfii.dump penfii
docker compose cp postgres:/tmp/penfii.dump ./penfii.dump
```

`penfii` here is the bootstrap superuser (`POSTGRES_USER`), which can read every table. The runtime role cannot.

For the file store, copy the `penfii_penfii-files` volume, for example:

```sh
docker run --rm -v penfii_penfii-files:/data -v "$PWD":/backup alpine \
  tar czf /backup/penfii-files.tar.gz -C /data .
```

File objects are removed after the database commit, by a background consumer, and only for a bucket that does not use S3 Versioning. A dump taken in that gap can name a file whose object is still in the volume; restoring both from the same moment is the consistent pair. A failed delete is retried and can be given up (`admin-cli events-dead`, consumer `storage`) and then retried (`events-retry`).

### Restore

`restore-check.sh` (below) checks that the database itself comes back. It does not start core-api against the restored data. The [restore rehearsal](#restore-rehearsal) does: it starts the whole service on a copy, with mail and push switched off.

Roles are not in the dump. On the cluster you restore into, create them first (`admin-cli db-provision` with `PENFII_PROVISION_DATABASE_URL`), or restore into a scratch database that already has the owner role. `pg_restore` keeps owners when those roles exist; a missing owner role makes it restore objects as the user who runs the restore. Re-run provisioning after a restore so the runtime role's grants and timeouts exist. The script `docker/scripts/restore-check.sh` does this against a scratch server and checks migration history, row counts and that the runtime role can read.

Point-in-time recovery (WAL archiving) is not set up here.

### Restore rehearsal

A backup you have never restored is a guess. `docker/scripts/restore-rehearsal.sh` takes a backup of a running stack and restores it into a **second Compose project** that has its own volumes, network and port, then checks the copy. It does not restore into, delete or reset the stack it backs up. Run it from `docker/`, with a backup directory **outside the repository** (the script refuses one inside it) whose parent exists:

```sh
mkdir -p ~/penfii-backups
scripts/restore-rehearsal.sh backup  ~/penfii-backups/2026-11-20   # PostgreSQL dump, file store archive, manifest
scripts/restore-rehearsal.sh restore ~/penfii-backups/2026-11-20   # project penfii-rehearsal on http://127.0.0.1:18080
scripts/restore-rehearsal.sh verify  ~/penfii-backups/2026-11-20   # readyz, database and file counts, search rebuild
scripts/restore-rehearsal.sh down    ~/penfii-backups/2026-11-20   # remove the rehearsal project and its volumes
```

`SOURCE_PROJECT` (default `penfii`), `SOURCE_ENV` (default `./.env`), `REHEARSAL_PROJECT` (default `penfii-rehearsal`) and `REHEARSAL_PORT` (default `18080`) change the defaults. What it does, and what you need to know before you use it:

- **A short write pause.** `backup` stops core-api and MinIO while it dumps PostgreSQL and archives the file store volume, so the two come from the same moment, then starts them again and waits until they are healthy. Requests fail during that time (about 20 seconds for a small installation). If the backup fails or is interrupted, the two are started again and the script says the backup is incomplete; delete that directory.
- **The backup holds everything in clear text**: the database, the uploaded files, and `source.env`, a copy of your `.env` with the passwords and `PENFII_WEBHOOK_KEK` the restored data needs (without the key, Signature webhooks stop verifying). The directory is mode 700 and its files 600. Keep it private. The script prints no setting values.
- **It only ever restores into a new project.** `restore` refuses a project that already has volumes or containers (so it never touches the source or another stack), refuses a port in use, and checks that the database it is about to fill is the rehearsal's and is empty. Roles are not in a dump, so it runs `admin-cli db-provision` first, restores the dump over the empty schema that made, then lets `db-setup` run again for the grants, timeouts and the migration check. `down` deletes only a project that `restore` created (it leaves a marker volume for that). A failed `restore` is cleaned up with `down` and run again.
- **Nothing reaches the outside.** The rehearsal's `rehearsal.env` empties the SMTP and Web Push settings and turns the MCP gateway off, and the public address is `http://127.0.0.1:<port>`, so no mail, push or OAuth fetch is sent from a copy of your data. Incoming webhooks only receive.
- **Same release.** Restore with the images the backup was taken with (they are built from your checkout), or with a later release whose changelog you have read. The manifest records the commit, the image ids and the migration checksums, and `restore` says when the image differs. That is a warning, not a stop: `verify` writes the image ids the rehearsal actually ran next to the backup's, and its result, to `verify.txt` in the backup directory. Moving between releases is [Updating](#updating); there is no way back to an older release other than a backup.
- **Search is rebuilt, not restored.** The new OpenSearch is empty. core-api notices that the indices PostgreSQL names are not there and builds them again in the background (within about a minute for a small installation); until then search says it is still indexing. `verify` also runs `admin-cli search-reindex` and checks that every index kind is listed by `search-status` and is `READY` with a live index; a kind that is missing from the output fails the check.
- **What `verify` proves**: `/readyz` answers, the migration history (with checksums) and the row count of every table except `sessions` equal the manifest, the number of objects in the file bucket equals the archive, every search index is `READY`, and no event delivery is dead. It does not sign in. Do that in the browser at the rehearsal address with a real account: open a ticket and a document, open a file that was uploaded before the backup, upload a new one, and search for something you know is there. A person who may not see a thing must still not find it.

The script is a rehearsal tool and not a backup service: it does not schedule anything, keep generations or copy the backup off the machine. The restore is checked on the data of the stack it ran against; it says nothing about a backup of another installation until you run it there.

### Upgrading an installation that used one database role

Compose's `db-setup` does not move ownership. For a volume created before the owner and runtime roles existed, set `PENFII_DB_OWNER_USER=penfii` and `PENFII_DB_OWNER_PASSWORD` to `POSTGRES_PASSWORD`, set a new `PENFII_DB_RUNTIME_PASSWORD`, and start as usual. `db-provision` leaves the existing owner role as it is (it may still be a superuser) and adds the runtime role. Moving ownership off the superuser (`REASSIGN OWNED`) is optional and is not done by Compose.

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

Back up first ([Backup](#backup)), then check out the release and rebuild:

```sh
git fetch --tags
git checkout v0.3.1
cd docker
docker compose up -d --build
```

`docker compose up` runs the one-shot `db-setup` service again before core-api starts (it is started again when its image is unchanged and re-created when the image changed), and `db-setup` applies pending database migrations with the schema owner's credential. core-api itself never migrates: it checks the history and refuses to start when it is behind. There is no upgrade guarantee between 0.x releases: read the [changelog](../CHANGELOG.md) of every release you move across before you start.

### From v0.2.0, v0.2.1, v0.2.2 or v0.3.0 to v0.3.1

v0.2.1 and v0.2.2 fix defects, v0.3.0 adds features, and v0.3.1 fixes search recovery and improves English matching. None of them has a database migration or a new or changed setting: check out `v0.3.1` and run `docker compose up -d --build` as above, with the same `docker/.env`. `db-setup` finds the schema current and core-api starts on the v0.2.0 database. Coming from v0.1.0, follow the next section first.

v0.3.1 changes how search analyzes text (search mapping version 2). On its first start, core-api builds every search index again from PostgreSQL in the background. Search keeps answering from the old indices until each new one is ready, so nothing has to be done by hand. Until each kind is rebuilt, OpenSearch needs room for the new index next to the current one (and next to a previous generation, if an earlier rebuild left one); after the switch the current index is kept as the previous generation and anything older is removed. `admin-cli search-status` shows when every kind is `READY` again. To check a backup before or after upgrading, see the [restore rehearsal](#restore-rehearsal).

### From v0.1.0 to v0.2.0

v0.2.0 runs the database as two roles and adds the `db-setup` service, so `docker/.env` needs three more lines before the first `up`. The v0.1.0 database stays owned by `penfii`, the PostgreSQL user it was made with:

```sh
PENFII_DB_OWNER_USER=penfii
PENFII_DB_OWNER_PASSWORD=<the value of POSTGRES_PASSWORD>
PENFII_DB_RUNTIME_PASSWORD=<a new password, letters and digits only>
```

`db-setup` then makes the runtime role and applies migration `0002`, which keeps the data of v0.1.0 (see [Upgrading an installation that used one database role](#upgrading-an-installation-that-used-one-database-role)). Settings are checked strictly from v0.2.0 on: a value that is set but cannot be read stops core-api and `docker compose logs core-api` names it. The changelog lists what changed.

A database made by a development build between v0.1.0 and v0.2.0 (migration history `0001` to `0008`), or from before the schema was consolidated into `database/migrations/0001_initial.sql`, cannot be upgraded in place and has to be recreated.

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
