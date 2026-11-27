# Roadmap

Where Heistarr is going, version by version. Each version is usable on its
own; if something takes longer, the next version shrinks, not the quality of
the current one. Dates are not promised. The detailed reasoning lives in
[PLAN.md](PLAN.md) (Spanish).

Legend: ✅ done · 🚧 in progress · ⬜ planned

## 0.1 — Movies, end to end ✅

The Radarr core, with the download client that matters most here.

- ✅ Owner account, sign-in, encrypted settings, six themes (Miami Neon default)
- ✅ TMDB catalog, metadata language, poster proxy with cache
- ✅ Torznab indexers (Jackett, Prowlarr)
- ✅ Release rules with audio language (CASTELLANO, LATINO, MULTi, DUAL) and upgrade cutoff
- ✅ Interactive and automatic search, blocklist, retry with the next release
- ✅ qBittorrent and **Synology Download Station**
- ✅ Hardlink import with path mapping, scan of the existing library
- ✅ Jellyfin / Emby / Plex rescan
- ✅ Docker image (amd64, arm64), release workflow

## 0.2 — Solid on the NAS

Make 0.1 something you can leave running and forget.

- ⬜ Image tested on a real Synology (DSM 7) with Download Station
- ⬜ Release-name test corpus: hundreds of real names, grown with every bug report
- ⬜ Browser tests (Playwright) in CI, with screenshots
- ⬜ Calendar of upcoming digital/physical releases for wanted movies
- ⬜ Minimum availability (announced, in cinemas, released) before searching
- ⬜ Rename existing files to the naming scheme; configurable naming
- ⬜ Clean up the download folder after import for Download Station
- ⬜ Logs page in the UI
- ⬜ Spanish translation of the UI

## 0.3 — Series (Sonarr)

- ⬜ Series, seasons and episodes from TMDB; choose what to monitor
- ⬜ Parser for `S01E02`, season packs, double episodes, specials, anime numbering
- ⬜ RSS sync of the indexers to catch new episodes as they appear
- ⬜ Episode calendar
- ⬜ Import to `Series/Name (Year)/Season 01/Name - S01E02 - Title.ext`

## 0.4 — Discover and requests (Overseerr)

- ⬜ Home page: trending, popular, upcoming, by genre
- ⬜ Title page: cast, ratings, trailer (YouTube embed or HTML5 video)
- ⬜ Several users with roles: owner, trusted (auto-approved), regular (queued)
- ⬜ Approval queue; request a whole series or one season
- ⬜ Plex / Jellyfin watchlists turned into requests

## 0.5 — More clients and indexers (Prowlarr)

- ⬜ Newznab indexers (usenet); indexer health and rate limits
- ⬜ Transmission, Deluge, rTorrent, aria2, SABnzbd, NZBGet
- ⬜ Emby/Jellyfin/Plex: per-library refresh instead of a full rescan

## 0.6 — Subtitles (Bazarr)

- ⬜ OpenSubtitles and other providers, chosen languages
- ⬜ Saved next to the video; automatic search after import, retries

## 0.7 — Music (Lidarr)

- ⬜ MusicBrainz and Cover Art Archive: artists, albums, tracks
- ⬜ Audio release parser (FLAC/MP3, bitrate) and audio quality rules
- ⬜ Import and basic tagging

## 0.8 — Notifications and migration

- ⬜ Discord, email, Telegram, ntfy, Gotify, Pushover, webhook
- ⬜ Import settings and library from Sonarr, Radarr, Lidarr, Prowlarr, Bazarr,
  Overseerr, qBittorrent and SABnzbd (read-only on the old apps)

## 0.9 — Operations and security

- ⬜ Backup and restore from the web UI
- ⬜ History clean-up settings, safe schema upgrades with automatic backup
- ⬜ Full security review before anyone exposes it outside home
  (recommended setup: Tailscale or an HTTPS reverse proxy)

## 1.0 — Stable

- ⬜ Public, versioned REST API (`/api/v1`) with API keys
- ⬜ Upgrade path guaranteed from every 0.x; documentation site
- ⬜ No breaking configuration changes within 1.x

## 1.1 — Desktop app

- ⬜ Tauri app (Windows, Linux) that wraps the web UI and connects to your server
- ⬜ Installers built by GitHub Actions

## Releases

- Versions follow [Semantic Versioning](https://semver.org/). Every 0.x
  release is published as a pre-release.
- Each release has notes in [CHANGELOG.md](CHANGELOG.md), Linux binaries for
  amd64 and arm64, and a Docker image:
  `ghcr.io/byrandom/heistarr:<version>`, `:<major.minor>`, `:edge` (newest),
  and `:latest` from 1.0 on.
- To cut a release: set `version` in `Cargo.toml`, move the changelog
  entries from *Unreleased* to the new version, merge to `main`, then run the
  **Release** workflow from the Actions tab (or push a `vX.Y.Z` tag).
