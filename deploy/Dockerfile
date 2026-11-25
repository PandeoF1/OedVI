# oedvi-server: small image, the game is mounted read-only.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# Built-in templates and builders are embedded at compile time.
COPY examples/server examples/server
# The launcher is a workspace member but is not built here; give cargo a stub
# so the workspace resolves without the WebView dependencies.
RUN mkdir -p launcher/src-tauri/src \
 && printf '[package]\nname = "oedvi-launcher"\nversion = "0.1.0"\nedition = "2024"\n[lib]\npath = "src/lib.rs"\n' > launcher/src-tauri/Cargo.toml \
 && touch launcher/src-tauri/src/lib.rs
RUN cargo build --release -p oedvi-server -p oedvi-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tini curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /var/lib/oedvi oedvi \
 && mkdir -p /var/lib/oedvi && chown oedvi /var/lib/oedvi
COPY --from=build /src/target/release/oedvi-server /src/target/release/oedvi /usr/local/bin/
USER oedvi
VOLUME ["/var/lib/oedvi"]
EXPOSE 7878
ENV OEDVI_CONFIG=/etc/oedvi/oedvi-server.toml
HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://127.0.0.1:7878/api/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "oedvi-server"]
