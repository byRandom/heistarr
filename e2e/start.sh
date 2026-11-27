#!/usr/bin/env bash
# Starts the fake services and Heistarr on a fresh data folder, for the
# browser tests. Playwright runs this (see playwright.config.mjs).
set -euo pipefail
cd "$(dirname "$0")"
RUN="$PWD/.run"
rm -rf "$RUN" && mkdir -p "$RUN/library" "$RUN/downloads"
python3 -I fakes.py "$RUN/downloads" &
FAKES=$!
trap 'kill $FAKES 2>/dev/null || true' EXIT
export HEISTARR_DATA="$RUN/data"
export HEISTARR_PORT=6499 HEISTARR_BIND=127.0.0.1
export HEISTARR_WEB="${HEISTARR_WEB:-$PWD/../crates/web/dist}"
export HEISTARR_TMDB_API=http://127.0.0.1:7101/3
export HEISTARR_TMDB_IMAGES=http://127.0.0.1:7101/img
export HEISTARR_CHECK_INTERVAL=2
export RUST_LOG=info
"${HEISTARR_BIN:-$PWD/../target/release/heistarr}"
