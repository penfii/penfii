# Self-hosting

`docker/compose.yaml` is the supported way to run Penfii. It starts PostgreSQL, OpenSearch (with the Korean analyzer), MinIO (S3-compatible file storage), the server (`core-api`) and the web app, and builds the server and web images from this checkout. There are no published images.

Penfii is at v0.4.1, an early release: read [Known limitations](known-limitations.md) first.

## What you need

- Git and Docker with Compose v2.
- Memory: OpenSearch runs with a 512 MB Java heap. On a nearly empty instance its container used about 1.4 GiB, so plan for a host with 4 GiB or more for the whole stack. These are observations from a development instance, not a sizing guide.
- On Linux hosts, OpenSearch's own documentation asks for `vm.max_map_count=262144` for production use.
- Time for the first start: it compiles the server in release mode and builds the web app, which takes several minutes. Later starts reuse the Docker cache.

## Install

```sh
git clone https://github.com/penfii/penfii.git
cd penfii
git checkout v0.4.1      # a release tag; leave this out to run the latest commit of the default branch
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
| `penfii_penfii-files` | MinIO: uploaded files, and MinIO's own accounts (including the app account, see [Object storage access key](#object-storage-access-key)) | **Yes** |
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
docker run --rm -v penfii_penfii-files:/data -v "$PWD":/backup alpine:3.24.2 \
  tar czf /backup/penfii-files.tar.gz -C /data .
```

File objects are removed after the database commit, by a background consumer, and only for a bucket that does not use S3 Versioning. A dump taken in that gap can name a file whose object is still in the volume; restoring both from the same moment is the consistent pair. A failed delete is retried and can be given up (`admin-cli events-dead`, consumer `storage`) and then retried (`events-retry`).

### Restore

`restore-check.sh` (below) checks that the database itself comes back. It does not start core-api against the restored data. The [restore rehearsal](#restore-rehearsal) does: it starts the whole service on a copy, with mail and push switched off.

Roles are not in the dump. On the cluster you restore into, create them first (`admin-cli db-provision` with `PENFII_PROVISION_DATABASE_URL`), or restore into a scratch database that already has the owner role. `pg_restore` keeps owners when those roles exist; a missing owner role makes it restore objects as the user who runs the restore. Re-run provisioning after a restore so the runtime role's grants and timeouts exist. The script `docker/scripts/restore-check.sh` does this against a scratch server and checks migration history, row counts, that the runtime role can read and write rows and that it cannot change the schema, the migration history or an audit log entry. It refuses, before it dumps or drops anything, a `SCRATCH_DB` that is the same as `SOURCE_DB`, a system database, a name without `restore` or `scratch` in it, or a name that is not a plain lower-case identifier; it also refuses to drop an existing database of that name unless an earlier run of the script made it. The dump and the comparison files are written with mode 600 and never through a symbolic link; they hold your data in clear text, so delete them when the check is done.

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
- **The backup holds everything in clear text**: the database, the uploaded files, and `source.env`, a copy of your `.env` with every password and key the restored data needs (the database and MinIO passwords, `PENFII_WEBHOOK_KEK`: without it Signature webhooks stop verifying, `PENFII_MFA_ENCRYPTION_KEY` when set, the SMTP password and the Web Push key). The directory is mode 700 and its files 600 (the script sets them again at the end of `backup`, and `restore` warns and corrects them when a copy came back with looser modes). Keep it private, and read [Protecting backups and off-site copies](#protecting-backups-and-off-site-copies) before a copy leaves this machine. The script prints no setting values.
- **It only ever restores into a new project.** `restore` refuses a project that already has volumes or containers (so it never touches the source or another stack), refuses a port in use, and checks that the database it is about to fill is the rehearsal's and is empty. Roles are not in a dump, so it runs `admin-cli db-provision` first, restores the dump over the empty schema that made, then lets `db-setup` run again for the grants, timeouts and the migration check. `down` deletes only a project that `restore` created (it leaves a marker volume for that). A failed `restore` is cleaned up with `down` and run again.
- **Nothing reaches the outside.** The rehearsal's `rehearsal.env` empties the SMTP and Web Push settings and turns the MCP gateway off, and the public address is `http://127.0.0.1:<port>`, so no mail, push or OAuth fetch is sent from a copy of your data. Incoming webhooks only receive.
- **Same release.** Restore with the images the backup was taken with (they are built from your checkout), or with a later release whose changelog you have read. The manifest records the commit, the image ids and the migration checksums, and `restore` says when the image differs. That is a warning, not a stop: `verify` writes the image ids the rehearsal actually ran next to the backup's, and its result, to `verify.txt` in the backup directory. Moving between releases is [Updating](#updating); there is no way back to an older release other than a backup.
- **The rehearsal has its own network range.** The source and the rehearsal run side by side, and two Compose projects cannot share an address range, so the rehearsal's `frontend` network uses `REHEARSAL_SUBNET` (default `172.29.250.0/24`; it must not overlap the source's `PENFII_FRONTEND_SUBNET`).
- **The object storage account is checked.** MinIO keeps its accounts in the file store volume, so the archive carries the app account. `restore` checks, before `minio-setup` runs, that the app account came back with the volume (`s3-account-restored.txt`), and `verify` checks that it can use the bucket afterwards (both are recorded in `verify.txt`). With no app account configured both say `not-configured`.
- **Search is rebuilt, not restored.** The new OpenSearch is empty. core-api notices that the indices PostgreSQL names are not there and builds them again in the background (within about a minute for a small installation); until then search says it is still indexing. `verify` also runs `admin-cli search-reindex` and checks that every index kind is listed by `search-status` and is `READY` with a live index; a kind that is missing from the output fails the check.
- **What `verify` proves**: `/readyz` answers, the migration history (with checksums) and the row count of every table except `sessions` equal the manifest, the number of objects in the file bucket equals the archive, every search index is `READY`, and no event delivery is dead. It does not sign in. Do that in the browser at the rehearsal address with a real account: open a ticket and a document, open a file that was uploaded before the backup, upload a new one, and search for something you know is there. A person who may not see a thing must still not find it.

The script is a rehearsal tool and not a backup service: it does not schedule anything, keep generations or copy the backup off the machine. The restore is checked on the data of the stack it ran against; it says nothing about a backup of another installation until you run it there.

### Protecting backups and off-site copies

A backup is a copy of everything the service can do. Treat the directory `restore-rehearsal.sh backup` makes (and any dump or volume archive you make by hand) like the production secrets, because it is.

**What is in it**

| File | Holds | Reading it gives |
|---|---|---|
| `penfii.dump` | the whole database | all workspaces, messages, documents and tickets, every password hash, session and sign-in record, the sealed authenticator secrets and recovery-code hashes. With no `PENFII_MFA_ENCRYPTION_KEY` it also holds the key that opens those secrets |
| `penfii-files.tar.gz` | the file store volume | every uploaded file, and MinIO's accounts with their keys |
| `source.env` (and `rehearsal.env`) | a copy of `.env` | the PostgreSQL, owner, runtime and MinIO root and app passwords, `PENFII_WEBHOOK_KEK`, `PENFII_MFA_ENCRYPTION_KEY` if set, `PENFII_SMTP_PASSWORD`, `PENFII_VAPID_PRIVATE_KEY` |

Anyone who has the dump and `source.env` together has the data and the keys to read it. Keep that in mind when you choose where a copy goes.

**Rules for a copy that leaves this machine**

1. **Encrypt it before it leaves** with a tool you trust, such as [age](https://age-encryption.org) or GnuPG. Penfii does not encrypt backups itself and has no backup service or key store of its own; do not invent an encryption format.
2. **Keep the key apart from the data.** The passphrase or private key must not sit in the same bucket, drive or folder as the encrypted file (a password manager, a printed copy in a safe, another person). A key next to the data protects nothing.
3. **Know what losing the key means.** An encrypted backup whose key is lost is unrecoverable, and so is a database whose `PENFII_MFA_ENCRYPTION_KEY` is lost (authenticator secrets cannot be read; people use a recovery code or you run `user-mfa-reset`, see [operations](operations.md#two-step-verification-and-backups)). Keep the key in at least two places that cannot be lost together.
4. **If you set `PENFII_MFA_ENCRYPTION_KEY`, keep it apart from the dump** (so a stolen dump does not also carry it), and back it up separately. `source.env` copies it, so for an off-site copy either encrypt `source.env` under a different key from the dump's, or keep the off-site copy without `source.env` and store the secrets separately in a password manager.
5. **Rotate after exposure.** If a backup, a `.env` or a log with secrets may have been seen by someone else, change the passwords in it (PostgreSQL roles, MinIO keys, SMTP, webhook key) and tell people to reset theirs if the dump leaked. A copy of the dump cannot be recalled.
6. **Do not put secrets in version control, tickets, chat or CI logs.** Never commit `.env` or a backup directory (the script refuses a backup inside the repository). `git` history is forever; the repository's gitleaks check is a last net, not a plan.
7. **Delete what you no longer need**, above all decrypted copies and the clear-text backup directory once an encrypted copy exists and has been tested.

**An example with age** (`age` for the key pair, `tar` keeps the 700/600 modes):

```sh
# once: make a key pair. Store penfii-backup.key in a password manager or print it; NOT next to the backups.
age-keygen -o penfii-backup.key          # prints the public key, "age1..."
# each backup:
scripts/restore-rehearsal.sh backup ~/penfii-backups/2026-11-20
tar -C ~/penfii-backups -cf - 2026-11-20 | age -r age1...yourpublickey -o 2026-11-20.tar.age
# now 2026-11-20.tar.age may be copied off the machine; delete the clear-text directory when you no longer need it
```

With GnuPG instead of age, the encryption step is `tar -C ~/penfii-backups -cf - 2026-11-20 | gpg --symmetric --cipher-algo AES256 -o 2026-11-20.tar.gpg` (it asks for a passphrase; store that like the age key) and decryption is `gpg --decrypt 2026-11-20.tar.gpg | tar -x -C ~/penfii-restore`.

**Prove that you can get it back** before you rely on it. The same rehearsal does it, from the decrypted copy, on a machine that has the checkout (and the images: `docker compose build`):

```sh
mkdir -p ~/penfii-restore
age -d -i penfii-backup.key 2026-11-20.tar.age | tar -x -C ~/penfii-restore
scripts/restore-rehearsal.sh restore ~/penfii-restore/2026-11-20
scripts/restore-rehearsal.sh verify  ~/penfii-restore/2026-11-20
scripts/restore-rehearsal.sh down    ~/penfii-restore/2026-11-20
```

A restore from an encrypted copy that was decrypted with a wrong key fails at the decrypt step, before anything is restored. Delete `~/penfii-restore` afterwards.

### Upgrading an installation that used one database role

Compose's `db-setup` does not move ownership. For a volume created before the owner and runtime roles existed, set `PENFII_DB_OWNER_USER=penfii` and `PENFII_DB_OWNER_PASSWORD` to `POSTGRES_PASSWORD`, set a new `PENFII_DB_RUNTIME_PASSWORD`, and start as usual. `db-provision` leaves the existing owner role as it is (it may still be a superuser) and adds the runtime role. Moving ownership off the superuser (`REASSIGN OWNED`) is optional and is not done by Compose.

## Network and TLS

By default the web container is published on `127.0.0.1` only (`PENFII_HTTP_BIND`, `PENFII_HTTP_PORT`). PostgreSQL, MinIO, OpenSearch and core-api are not published at all; OpenSearch runs with its security plugin disabled, so it must stay unreachable from outside the Compose network.

For anything beyond your own machine, put a TLS reverse proxy in front of the web container:

- Forward every path to the web container (it forwards `/v1`, `/oauth`, `/mcp`, `/.well-known/*`, `/healthz` and `/readyz` to core-api and serves the app for the rest), and let WebSocket upgrades through.
- Set `PENFII_PUBLIC_URL` to the public `https://` origin. A value starting with `https://` also marks the session cookie `Secure`, so use it only when TLS really is in front.
- Add `Strict-Transport-Security` at the proxy; the web container leaves it to the proxy.
- Sign-in attempts are throttled per client address, taken from `X-Forwarded-For` only when the request comes from a proxy you name. The web container trusts private address ranges (`PENFII_EDGE_PROXIES`); core-api trusts only the web container, which is the only other container on the private `frontend` network (`PENFII_FRONTEND_SUBNET`, `PENFII_TRUSTED_PROXIES`), so nothing else on the host's Docker networks can forge an address for it. This fits a proxy on the same machine as Penfii, or no proxy at all. A proxy on **another** machine, private or public (a LAN proxy, a cloud load balancer, a CDN), must be added to `PENFII_TRUSTED_PROXIES` (keeping the frontend subnet), and a public one to `PENFII_EDGE_PROXIES` as well, otherwise all visitors appear to come from the proxy and share one throttle; core-api logs a warning naming such a proxy's private address. The commented examples in `docker/.env.example` show the format.
- Inside Compose, the containers are separated. Only the web container is on the `frontend` network with core-api, and of the two only core-api needs a route out (mail, external sign-in); the data services have none. PostgreSQL, OpenSearch and MinIO are on an `internal` network with no route out. The containers drop every Linux capability they do not need and cannot gain new ones; core-api and web run as a non-root user on a read-only file system.

To use a different port, change `PENFII_HTTP_PORT` and the port in `PENFII_PUBLIC_URL` together.

## The MCP gateway (optional)

The MCP gateway lets AI clients work in Penfii as a signed-in person, after that person approves the client in the browser. It is off by default. To turn it on, set `PENFII_MCP_ENABLED=1` in `.env` and run `docker compose up -d` again. MCP clients then connect to `PENFII_PUBLIC_URL/mcp`. `PENFII_PUBLIC_URL` must be `https://` (or `http://localhost` / `http://127.0.0.1`), otherwise core-api refuses to start. Set `PENFII_MCP_DCR_ENABLED=1` as well for clients that register themselves. The other MCP settings are in [Configuration](configuration.md#mcp-gateway).

What the tools do, the scopes and what stays in the web app are in [MCP tools](mcp-tools.md). A connection that was approved before a new scope existed keeps working with what it had; a client that needs a new scope asks the person again. A Workspace owner or admin decides in the web app which risk classes (write, destructive, administrative) are allowed; the standard policy allows reading and writing only.

## Updating

Back up first ([Backup](#backup)), then check out the release and rebuild:

```sh
git fetch --tags
git checkout v0.4.1
cd docker
docker compose up -d --build
```

`docker compose up` runs the one-shot `db-setup` service again before core-api starts (it is started again when its image is unchanged and re-created when the image changed), and `db-setup` applies pending database migrations with the schema owner's credential. core-api itself never migrates: it checks the history and refuses to start when it is behind. There is no upgrade guarantee between 0.x releases: read the [changelog](../CHANGELOG.md) of every release you move across before you start.

### From v0.4.0 to v0.4.1

v0.4.1 widens the MCP gateway (see the [changelog](../CHANGELOG.md#041)). It has no database migration and no new or changed setting: back up ([Backup](#backup)), check out `v0.4.1` and run `docker compose up -d --build` with the same `docker/.env`. An existing MCP connection keeps its scopes; a client that wants the new tools asks for the new scopes and the person approves them again. Coming from v0.3.1 or older, read the next section too.

### From v0.2.0, v0.2.1, v0.2.2, v0.3.0 or v0.3.1 to v0.4.0

v0.4.0 is a security release and, unlike the 0.3.x releases, it needs a few things from you. Read the [changelog](../CHANGELOG.md#040) entry first, back up ([Backup](#backup)), then check out `v0.4.0` and run `docker compose up -d --build` as above. In short:

- **Database:** one migration, `0003_after_v0_3_1.sql`, is applied by `db-setup` on the first start, in one transaction. It adds the tables for e-mail change and two-step verification and removes access that people removed from a workspace still held (Team, Project, Group and Doc roles, private channels). Your data is kept. There is no down migration: going back to v0.3.1 means restoring the backup. A development database that already ran the earlier split 0003 to 0007 is recognised and rewritten to 1, 2, 3 without running them again.
- **Compose:** the file changed (networks, non-root and read-only containers, `minio-setup`, web on port 8080 inside). Follow [Upgrading to the hardened Compose file](#upgrading-to-the-hardened-compose-file): run `docker compose down` (no `-v`) once before `up -d --build` so the new networks are created.
- **Reverse proxy on another machine:** add it to `PENFII_TRUSTED_PROXIES` (and `PENFII_EDGE_PROXIES`) before you upgrade, or every visitor is counted as one address. See [Network and TLS](#network-and-tls).
- **Settings:** nothing new is required. `PENFII_S3_APP_ACCESS_KEY` and `PENFII_S3_APP_SECRET_KEY` ([Object storage access key](#object-storage-access-key)) and `PENFII_MFA_ENCRYPTION_KEY` ([Configuration](configuration.md)) are optional. If you set the MFA key, keep a copy elsewhere: changing or losing it makes the two-step secrets sealed with it unreadable (`admin-cli user-mfa-reset` recovers an account).
- **Behaviour that people notice:** a new e-mail address is confirmed from that address first (without SMTP it cannot be changed in the profile, use `admin-cli user-set-email`); an existing account accepts an invitation only while signed in to it; a password change ends the other devices and a reset ends every session.

### From v0.2.0, v0.2.1, v0.2.2 or v0.3.0 to v0.3.1

v0.2.1 and v0.2.2 fix defects, v0.3.0 adds features, and v0.3.1 fixes search recovery and improves English matching. None of them has a database migration or a new or changed setting: check out `v0.3.1` and run `docker compose up -d --build` as above, with the same `docker/.env`. `db-setup` finds the schema current and core-api starts on the v0.2.0 database. Coming from v0.1.0, follow the section below first.

v0.3.1 changes how search analyzes text (search mapping version 2). On its first start, core-api builds every search index again from PostgreSQL in the background. Search keeps answering from the old indices until each new one is ready, so nothing has to be done by hand. Until each kind is rebuilt, OpenSearch needs room for the new index next to the current one (and next to a previous generation, if an earlier rebuild left one); after the switch the current index is kept as the previous generation and anything older is removed. `admin-cli search-status` shows when every kind is `READY` again. To check a backup before or after upgrading, see the [restore rehearsal](#restore-rehearsal).

### Upgrading to the hardened Compose file

The Compose file of v0.4.0 and later runs core-api and web as non-root users on read-only file systems, drops Linux capabilities, splits the network in two (`frontend` for web and core-api, an `internal` `data` network for PostgreSQL, OpenSearch and MinIO), pins the `frontend` range, and moves bucket creation and the MinIO account setup to a one-shot `minio-setup` service (see [Object storage access key](#object-storage-access-key)). Your data volumes, `.env`, URL and port do not change. Nothing new is required in `.env`.

1. Back up ([Backup](#backup)) and check out the release.
2. `cd docker && docker compose up -d --build`. Compose creates the two new networks and re-creates every container; the volumes are kept. The old `<project>_default` network stays behind until you remove it (`docker network rm penfii_default`, once nothing is attached).
3. Check `docker compose ps` (everything `healthy`, `minio-setup` and `db-setup` exited with 0), open Penfii, and download a file.

What can need your attention:

- **The web container listens on 8080 inside** (it runs as a non-root user and cannot bind port 80). The published address does not change: `PENFII_HTTP_PORT` still maps to it, and a reverse proxy that talks to the host port is unaffected. Only something that connected to port 80 of the container itself, or an override file that maps to container port 80, must be changed to 8080.
- **The `frontend` network has a fixed range**, `172.29.240.0/24`. If it overlaps a network of yours (a VPN, a LAN route), compose says `Pool overlaps` and does not start: set `PENFII_FRONTEND_SUBNET` in `.env` to a free range. Changing it later needs the network removed (`docker compose down` keeps the volumes), because Docker does not change the range of a network that exists. A second stack on the same host (the [restore rehearsal](#restore-rehearsal) does this) needs a different range.
- **Reverse proxy on another machine: add it to `PENFII_TRUSTED_PROXIES` before you upgrade.** The variable now defaults to the frontend range, not to all private ranges: core-api believes `X-Forwarded-For` from the web container only, and the web container passes on the address it was reached from. A proxy on the same host as Penfii reaches the web container through Docker's gateway on that range and keeps working, and so do people who open the web container directly (`PENFII_HTTP_BIND=0.0.0.0` on a LAN, no proxy). A proxy on **another** machine (`192.168.1.20` on your LAN, say) is not trusted any more: every visitor then appears to come from that proxy's address, so they share one sign-in throttle (one person's failed attempts can lock out everyone) and the audit log records the proxy's address. Add it, keeping the frontend range, for example `PENFII_TRUSTED_PROXIES=172.29.240.0/24,192.168.1.20` (use your `PENFII_FRONTEND_SUBNET` if you changed it). core-api logs a warning naming the address and `PENFII_TRUSTED_PROXIES` the first time it sees such a proxy. If you set the variable yourself, nothing changes; a proxy with a public address goes in both `PENFII_EDGE_PROXIES` and `PENFII_TRUSTED_PROXIES`, as before.
- Anything you attached to the old single Compose network by hand (a monitoring agent, a `psql` container) can no longer reach PostgreSQL, OpenSearch or MinIO from the `frontend` network. For `psql` use `docker compose exec postgres psql -U penfii penfii`; the other services are meant to be reached through core-api only.
- **Rolling back:** run `docker compose down` (no `-v`) while the new release is still checked out, so its networks are removed, then check out the previous release and run `docker compose up -d --build`. The data volumes are untouched, the older Compose file simply does not have the networks and `minio-setup`, and core-api goes back to the root key pair. Extra settings in `.env` are ignored by the older file. The MinIO app account stays in the volume, unused and harmless; to remove it see below.

### Object storage access key

core-api is the only program that uses the file store, and until now it used MinIO's root account to do it. The `minio-setup` service (run on every `docker compose up`, with the root key pair, before core-api starts) now makes the bucket `penfii` and, when you give it `PENFII_S3_APP_ACCESS_KEY` and `PENFII_S3_APP_SECRET_KEY`, an account that may only list the bucket and read, write and delete objects in it. core-api then connects as that account: it cannot create or delete buckets, read other buckets or use MinIO's admin API, and the root key pair stays out of core-api's environment. core-api no longer creates the bucket itself under Compose (`PENFII_S3_ENSURE_BUCKET=0`).

- **New installation:** `.env.example` already has both lines (replace `CHANGE_ME`; 8 or more characters, different from the root pair, letters and digits).
- **Existing installation:** nothing breaks if you do nothing. With no app key pair, core-api keeps using the root key pair, and `minio-setup` prints a warning in `docker compose logs minio-setup` on every start. To move over, add two lines to `.env` and run `docker compose up -d`:

  ```sh
  PENFII_S3_APP_ACCESS_KEY=<8+ letters and digits, not the root key>
  PENFII_S3_APP_SECRET_KEY=<8+ letters and digits>
  ```

  `minio-setup` makes the account, attaches the policy and core-api is re-created with the new key pair; uploaded files stay where they are (they belong to the bucket, not to an account). Check by downloading and uploading a file. Set both or neither: with one of them `minio-setup` fails and core-api does not start, so a typo is not silently ignored. It fails the same way while either value is still `CHANGE_ME`, and when the app key equals the root key but the secrets differ.
- **Roll back:** delete the two lines and run `docker compose up -d` again. core-api uses the root key pair again; the account stays in MinIO, harmless. To remove it too: `docker compose run --rm --no-deps minio-setup mc admin user rm local <the app key>` (`minio-setup` followed by `mc ...` runs that one mc command with the root key pair as the alias `local`; no secret goes on a command line).
- **Change the secret:** edit the value and run `docker compose up -d`; `minio-setup` sets the new secret and core-api restarts with it.
- **Rename the app account:** set the new name in `PENFII_S3_APP_ACCESS_KEY` and run `docker compose up -d`; `minio-setup` makes the new account and core-api switches to it. The old account is not removed by itself and still has the bucket policy, so remove it once core-api works with the new one: `docker compose run --rm --no-deps minio-setup mc admin user rm local <the old app key>`.
- The accounts live in the file store volume, so the file backup carries them and a [restore rehearsal](#restore-rehearsal) checks that they came back. A backup made before the app account existed restores fine: `minio-setup` makes it again from `.env`.
- Changing the **root** key pair of an existing MinIO volume is a separate operation (the volume keeps the root account it was made with); change the password in MinIO first, then in `.env`. That, and doing this on a deployed machine, is yours to schedule; the Compose file does not do it for you.
- The image (`pgsty/silo`) is the one the Compose file already used; it ships the `mc` client the setup uses.

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
