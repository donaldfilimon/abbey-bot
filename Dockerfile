# Build stage. The image and rust-toolchain.toml name the same exact stable
# compiler. Build and runtime also share Debian trixie: mixing a floating
# builder with an older runtime can produce a binary that needs a newer glibc
# and dies at startup with "GLIBC_2.xx not found".
FROM rust:1.98.0-slim-trixie AS build
WORKDIR /src
COPY . .
# --locked: Cargo.lock is committed; a deploy build must not resolve afresh.
RUN cargo build --release --locked

# Runtime stage. No packages at all: the active Linux dependency graph uses
# compiled WebPKI roots and rejects native-tls/OpenSSL packages. The repository
# gate proves that target-specific graph with scripts/check-linux-tls-tree.py,
# so installing ca-certificates here would be a dead layer.
FROM debian:trixie-slim
RUN useradd --system --create-home --home-dir /app abbey
USER abbey
WORKDIR /app
COPY --from=build /src/target/release/abbey-bot /app/abbey-bot
# Configuration is env-only: DISCORD_TOKEN (required), ABBEY_GUILD_ID, RUST_LOG,
# the backend variables, and ABBEY_DATA_DIR. Pass with `docker run --env-file`;
# never bake a token into an image layer. For persistence mount a volume and
# point ABBEY_DATA_DIR at it (e.g. -v abbey-data:/app/data -e ABBEY_DATA_DIR=/app/data).
ENTRYPOINT ["/app/abbey-bot"]
