# Penfii @VERSION@ — Docker install

This ZIP installs Penfii with Docker Compose from the official images on the GitHub Container Registry
(`ghcr.io/penfii/core-api`, `ghcr.io/penfii/web`, `ghcr.io/penfii/opensearch`, tag `@VERSION@`). It needs no source
code, Rust or Node. The images are `linux/amd64` only; other platforms are not tested and not supported.

Penfii is an early (0.x) release: the API, the database schema and the settings can change between versions. Read
[Known limitations](https://github.com/penfii/penfii/blob/v@VERSION@/docs/known-limitations.md) first.

## What you need

- Docker with Compose v2 on a `linux/amd64` host, with access to `ghcr.io`.
- 4 GiB of memory or more for the whole stack (OpenSearch alone uses about 1.4 GiB).
- On Linux hosts, OpenSearch asks for `vm.max_map_count=262144` for production use.

## Check the download

```sh
sha256sum -c SHA256SUMS --ignore-missing    # SHA256SUMS is a file of the same release
unzip penfii-v@VERSION@-docker.zip
cd penfii
```

## Install

```sh
cp .env.example .env
```

Edit `.env`: replace every `CHANGE_ME` (the PostgreSQL password and the two database role passwords: letters and
digits only; the object storage key pairs: 8 or more characters, the app pair different from the root pair), and set
`PENFII_PUBLIC_URL` to the address people will open (default `http://localhost:8080`). Then:

```sh
docker compose pull
docker compose up -d
docker compose ps --all      # services healthy; db-setup and minio-setup exited with code 0
```

Create the first account and workspace (there is no sign-up page). The command asks for the password:

```sh
docker compose run --rm admin-cli bootstrap \
  --email owner@example.com --display-name Owner --workspace-name Penfii --time-zone Asia/Seoul
```

Open the address from `PENFII_PUBLIC_URL` and sign in. Before you let others in, read
[Network and TLS](https://github.com/penfii/penfii/blob/v@VERSION@/docs/self-hosting.md#network-and-tls): the
published port is bound to this machine only (`PENFII_HTTP_BIND=127.0.0.1`), and OpenSearch has no authentication,
so it stays on the internal network.

## Upgrade

1. Back up PostgreSQL and the file store first
   ([Backup](https://github.com/penfii/penfii/blob/v@VERSION@/docs/self-hosting.md#backup)). There is no down
   migration: going back means restoring the backup.
2. Read the [changelog](https://github.com/penfii/penfii/blob/v@VERSION@/CHANGELOG.md) of every release you move across.
3. Unpack the new ZIP into a new directory, copy your `.env` into it and compare it with the new `.env.example`.
4. In the old directory run `docker compose down` (**without** `-v`: the volumes hold your data), then in the new one:

   ```sh
   docker compose pull
   docker compose up -d
   ```

   `db-setup` applies pending migrations before core-api starts. Both directories use the Compose project name
   `penfii`, so the data volumes (`penfii-pg`, `penfii-os`, `penfii-files`) carry over.

Upgrading an installation that was built from source with the repository's `docker/compose.yaml` works the same way:
keep its `docker/.env`, run `docker compose down` there, and start this directory instead.

## Operate

```sh
docker compose run --rm admin-cli <command>     # workspace-list, create-user, user-set-password, search-status, ...
docker compose logs -f core-api
docker compose down                             # stop; data volumes are kept
```

Documentation for this release: <https://github.com/penfii/penfii/tree/v@VERSION@/docs>. Images are tied to the
commit they were built from: `docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}' ghcr.io/penfii/core-api:@VERSION@`.

Penfii is licensed under GPL-3.0-only (`LICENSE`). Report security problems as described in
[SECURITY.md](https://github.com/penfii/penfii/blob/v@VERSION@/SECURITY.md).
