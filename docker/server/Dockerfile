FROM rust:1.99.0-bookworm AS build
WORKDIR /src/server
COPY server/Cargo.toml server/Cargo.lock server/rust-toolchain.toml ./
COPY server/apps ./apps
COPY server/crates ./crates
COPY server/vendor ./vendor
COPY server/.sqlx ./.sqlx
COPY database/migrations /src/database/migrations
ENV SQLX_OFFLINE=true
RUN cargo build --release --locked -p core-api -p admin-cli

FROM debian:trixie-slim
# Security updates published after the base image was built (glibc, zlib, ...) are taken at build time.
# CI passes a new value for every run attempt so cached package upgrades cannot go stale.
ARG PENFII_SECURITY_REFRESH
RUN apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/server/target/release/core-api /usr/local/bin/core-api
COPY --from=build /src/server/target/release/admin-cli /usr/local/bin/admin-cli
# Not root. The server writes nothing to its file system (the Compose file makes it read-only).
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin penfii
USER 10001:10001
EXPOSE 8080
CMD ["core-api"]
