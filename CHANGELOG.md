# Changelog

All notable changes to Heistarr. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Until 1.0, minor versions may
change the configuration or the database; upgrades are migrated
automatically, but there is no downgrade.

## [Unreleased]

## [0.1.1] - 2026-11-27

### Added
- Browser tests (Playwright) of the main journey with Download Station,
  against fake services; run in CI and before every release.
- Corpus of real release names checked by the parser tests.
- `TESTING.md`: how the tests are organised and the rules for new work.

### Fixed
- `cargo run` showed a blank page when the web UI was not built: the UI is now
  found from any working directory, and when it is missing the browser
  explains how to build it.
- A leading `[Group]` tag is no longer read as part of the title; `Ingles`
  is read as English audio.

## [0.1.0] - 2026-11-27

First version: movies from search to library.

### Added
- First-run setup of the owner account; sign-in with rate limiting, HttpOnly
  session cookie and CSRF protection.
- Settings stored in SQLite; API keys and passwords encrypted at rest and
  masked in the UI.
- Movie catalog from TMDB with your own key and metadata language.
- Torznab indexers (Jackett, Prowlarr) with a Test button.
- Release rules: resolution range, size, seeders, preferred and blocked
  words, x265, audio language (CASTELLANO, LATINO, MULTi, DUAL…), upgrades
  until a quality cutoff.
- Interactive search showing every release, its score and why it is rejected;
  automatic search every 15 minutes.
- Download clients: qBittorrent and Synology Download Station (DSM 6 and 7).
- Failed or stalled downloads are blocklisted and the next release is tried.
- Import by hardlink (or copy) to `Movies/Title (Year)/Title (Year).ext`, with
  path mapping; scan of the movies already on disk.
- Jellyfin, Emby and Plex rescan after import.
- Web UI with six themes, Miami Neon by default.
- Docker image with `PUID`/`PGID`/`UMASK` and a health check, published on
  `ghcr.io/byrandom/heistarr`; Linux binaries for amd64 and arm64.
- Licensed under the GNU AGPL v3 or later.

[Unreleased]: https://github.com/byRandom/heistarr/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/byRandom/heistarr/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/byRandom/heistarr/commits/v0.1.0
