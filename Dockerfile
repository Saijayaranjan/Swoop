# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------------------------
# Stage 1: build the embedded remote web UI (crates/swoop-server embeds web/remote/dist via
# rust-embed at compile time, so it must exist before the Rust build starts).
# ---------------------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS web-builder
WORKDIR /web
COPY web/remote/package.json web/remote/package-lock.json* ./
RUN npm install
COPY web/remote/ ./
RUN npm run build

# ---------------------------------------------------------------------------------------------
# Stage 2: build the `swoop` binary in release mode.
# ---------------------------------------------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /src

# Cache dependency compilation across rebuilds triggered only by source changes.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/

# Drop in the pre-built web UI where rust-embed expects it.
COPY --from=web-builder /web/dist/ web/remote/dist/

RUN cargo build --release --locked -p swoop-cli

# ---------------------------------------------------------------------------------------------
# Stage 3: minimal runtime image.
# ---------------------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin swoop

COPY --from=builder /src/target/release/swoop /usr/local/bin/swoop

ENV SWOOP_DATA_DIR=/data
RUN mkdir -p /data /downloads \
    && chown -R swoop:swoop /data /downloads

VOLUME ["/data", "/downloads"]
EXPOSE 41780

USER swoop
WORKDIR /home/swoop

ENTRYPOINT ["swoop", "server", "--remote", "0.0.0.0:41780", "--download-dir", "/downloads"]
