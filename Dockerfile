# Heistarr image: one binary plus the web UI. Multi-arch (amd64, arm64).
FROM rust:1-bookworm AS build
RUN rustup target add wasm32-unknown-unknown \
 && cargo install wasm-bindgen-cli --version 0.2.100 --locked
WORKDIR /src
COPY . .
RUN ./scripts/build-web.sh release \
 && cargo build --release -p heistarr-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/heistarr /app/heistarr
COPY --from=build /src/crates/web/dist /app/web
COPY docker/entrypoint.sh /entrypoint.sh
ENV HEISTARR_DATA=/config HEISTARR_WEB=/app/web HEISTARR_PORT=6464 PUID=1000 PGID=1000 UMASK=002
EXPOSE 6464
VOLUME ["/config"]
HEALTHCHECK --interval=30s --timeout=10s --start-period=20s CMD ["/app/heistarr", "healthcheck"]
ENTRYPOINT ["/entrypoint.sh"]
