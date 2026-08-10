# Build stage. The rust image's rustup honours rust-toolchain.toml, so the
# pinned nightly is installed at build time rather than hardcoded here.
FROM rust:slim AS build
WORKDIR /src
COPY . .
# --locked: Cargo.lock is committed; a deploy build must not resolve afresh.
RUN cargo build --release --locked

# Runtime stage. The binary needs CA certificates for TLS to Discord and
# nothing else from userland.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --create-home --home-dir /app abbey
USER abbey
WORKDIR /app
COPY --from=build /src/target/release/abbey-bot /app/abbey-bot
# Configuration is env-only: DISCORD_TOKEN (required), ABBEY_GUILD_ID, RUST_LOG.
# Pass with `docker run --env-file`; never bake a token into an image layer.
ENTRYPOINT ["/app/abbey-bot"]
