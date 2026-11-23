FROM node:22.23.3-bookworm-slim AS build
WORKDIR /src
COPY client/package.json client/package-lock.json client/tsconfig.base.json ./
COPY client/apps/web/package.json ./apps/web/
COPY client/packages/api-client/package.json ./packages/api-client/
COPY client/packages/client/package.json ./packages/client/
COPY client/packages/contracts/package.json ./packages/contracts/
RUN npm ci
COPY client/apps ./apps
COPY client/packages ./packages
RUN npm run build

FROM caddy:2.11.9-alpine
# The image gives the caddy binary the file capability to bind ports below 1024. It cannot start without that
# capability in the container's set, and this one listens on 8080 as an unprivileged user, so remove it.
# The base's own zlib and OpenSSL libraries lag the Alpine repository (a Critical zlib finding between base
# releases), so those three are upgraded in place; the Caddy tag stays the explicit pin.
RUN apk add --no-cache libcap-setcap && apk upgrade --no-cache zlib libcrypto3 libssl3 \
    && setcap -r /usr/bin/caddy && apk del libcap-setcap
COPY docker/web/Caddyfile /etc/caddy/Caddyfile
COPY --from=build /src/apps/web/dist /srv
# Not root: an unprivileged user, so the port is 8080, and Caddy's state directories point at /tmp (a tmpfs
# in the Compose file; the root file system is read-only).
ENV XDG_DATA_HOME=/tmp/caddy-data XDG_CONFIG_HOME=/tmp/caddy-config
USER 10001:10001
EXPOSE 8080
