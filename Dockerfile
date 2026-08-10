# Build stage. The rust image's rustup honours rust-toolchain.toml, so the
# toolchain channel it names (a floating nightly) is installed at build time
# rather than hardcoded here. The tag pins the Debian release: `rust:slim`
# floats to the newest Debian (currently trixie, glibc 2.41), and a binary
# linked there fails at startup on the bookworm runtime below with
# "GLIBC_2.xx not found" — build and runtime must share a release.
FROM rust:slim-bookworm AS build
WORKDIR /src
COPY . .
# --locked: Cargo.lock is committed; a deploy build must not resolve afresh.
RUN cargo build --release --locked

# Runtime stage. No packages at all: TLS roots are compiled into the binary
# (webpki-roots via hyper-rustls and tokio-tungstenite — verifiable in
# Cargo.lock; nothing native-tls or openssl is present), so the system cert
# store is never read and installing ca-certificates would be a dead layer.
FROM debian:bookworm-slim
RUN useradd --system --create-home --home-dir /app abbey
USER abbey
WORKDIR /app
COPY --from=build /src/target/release/abbey-bot /app/abbey-bot
# Configuration is env-only: DISCORD_TOKEN (required), ABBEY_GUILD_ID, RUST_LOG.
# Pass with `docker run --env-file`; never bake a token into an image layer.
ENTRYPOINT ["/app/abbey-bot"]
