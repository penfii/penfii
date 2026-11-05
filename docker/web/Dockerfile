FROM node:26.12.1-bookworm-slim AS build
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

FROM caddy:2.11.7-alpine
COPY docker/web/Caddyfile /etc/caddy/Caddyfile
COPY --from=build /src/apps/web/dist /srv
EXPOSE 80
