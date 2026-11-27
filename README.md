# Heistarr

A self-hosted media manager written in Rust. One app, one login, one port:
find movies, pick the best release with your rules, send it to your download
client, file it in your library with a clean name and tell your media server.

Status: **v0.1 — movies**. Series, requests, subtitles and music come later:
see the [roadmap](ROADMAP.md) and the [changelog](CHANGELOG.md).

## What works

- First-run setup: the first visitor creates the owner account.
- Movie catalog from TMDB (your own key), titles and overviews in your language.
- Library: add, remove, status (wanted / downloading / available), and
  **Scan library** to bring in the movies you already have.
- Torznab indexers (Jackett, Prowlarr) with a Test button.
- Release rules: resolution range, size, seeders, preferred/blocked words,
  x265, **audio language** (CASTELLANO, LATINO, MULTi, DUAL…), upgrades until a
  quality cutoff.
- Interactive search (every release, its score and why it is rejected) and
  automatic search every 15 minutes for wanted movies.
- Download clients: **qBittorrent** and **Synology Download Station**
  (DSM 6 and 7).
- Failed or stalled (6 h) downloads are blocklisted and the next release is tried.
- Import: the largest video file is hardlinked (or copied across filesystems)
  to `Movies/Title (Year)/Title (Year).ext`, with path mapping for Docker.
- Jellyfin, Emby or Plex rescan after each import.
- Secrets (API keys, passwords) encrypted at rest and never sent back whole.
- Six themes; **Miami Neon** by default.

## Run it with Docker (Synology)

Images for amd64 and arm64 are published on GitHub Container Registry. See
[docker-compose.example.yml](docker-compose.example.yml), or:

```bash
docker run -d --name heistarr -p 6464:6464 \
  -e PUID=1026 -e PGID=100 -e UMASK=002 \
  -v /volume1/docker/heistarr:/config -v /volume1/data:/data \
  ghcr.io/byrandom/heistarr:edge
```

Use a version tag (`:0.1.0`, `:0.1`) to stay on one version. To build the
image yourself: `docker build -t heistarr .`. Linux binaries are attached to
each [release](https://github.com/byRandom/heistarr/releases).

Open `http://<nas>:6464`, create the owner, then in **Settings**:

1. TMDB key (from themoviedb.org → Settings → API).
2. Movies folder, e.g. `/data/media/movies`.
3. Download client → **Synology Download Station**: DSM address
   (`http://<nas-ip>:5000`), a DSM user with Download Station access and
   **without two-step verification**, destination shared folder
   `data/torrents`.
4. Path mapping: Download Station reports files as `/<shared folder>/…`
   (e.g. `/data/torrents/…`). Map that to where the same folder is inside
   the container. With the share mounted at `/data`, both are `/data/torrents`.
5. An indexer (Jackett/Prowlarr Torznab URL and API key).

Keep downloads and the library in the **same** shared folder so imports are
hardlinks (no extra space). Note: removing a finished task from Download
Station keeps its files; clean the download folder from DSM if needed.

## Build from source

Needs Rust (stable), the wasm target and `wasm-bindgen-cli` 0.2.100:

```bash
rustup target add wasm32-unknown-unknown
```

```bash
cargo install wasm-bindgen-cli --version 0.2.100 --locked
```

```bash
./scripts/build-web.sh && cargo run --release -p heistarr-server
```

Environment variables: `HEISTARR_DATA` (default `./data`), `HEISTARR_PORT`
(6464), `HEISTARR_BIND` (`0.0.0.0`), `HEISTARR_WEB` (web UI folder),
`HEISTARR_MASTER_KEY` (base64, 32 bytes; otherwise `master.key` in the data
folder — back it up with the database), `HEISTARR_SECURE_COOKIE=1` behind an
HTTPS proxy, `RUST_LOG`.

Logs are written to `<data>/logs/` (daily, 14 days kept). `GET /healthz`
answers `ok`.

## Layout

```
crates/core    shared API types, release-name parser, pick rules (no I/O)
crates/server  `heistarr` binary: axum API, SQLite, clients, importer, scheduler
crates/web     Leptos web UI compiled to WebAssembly
style/         CSS with one block of variables per theme
```

## Tests

```bash
cargo test --workspace
```

The server tests run the real app against fake TMDB, Torznab, qBittorrent,
Synology and Jellyfin servers started inside the test; nothing goes to the
Internet.

## Security notes

- Do not expose port 6464 to the Internet; use Tailscale or an HTTPS reverse
  proxy (then set `HEISTARR_SECURE_COOKIE=1`).
- Sign-in is rate-limited (5 failures per 15 min per address and per user).
- State-changing API calls need the `X-Heistarr` header (CSRF protection);
  the session cookie is `HttpOnly; SameSite=Strict`.

## License

Heistarr is free software under the [GNU Affero General Public License v3.0
or later](LICENSE). If you run a modified version for other people, you must
offer them its source code.

This product uses the TMDB API but is not endorsed or certified by TMDB.
