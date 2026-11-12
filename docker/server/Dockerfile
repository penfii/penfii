FROM rust:1.99.1-bookworm AS build
WORKDIR /src/server
COPY server/Cargo.toml server/Cargo.lock server/rust-toolchain.toml ./
COPY server/apps ./apps
COPY server/crates ./crates
COPY database/migrations /src/database/migrations
RUN cargo build --release -p core-api -p admin-cli

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/server/target/release/core-api /usr/local/bin/core-api
COPY --from=build /src/server/target/release/admin-cli /usr/local/bin/admin-cli
EXPOSE 8080
CMD ["core-api"]
