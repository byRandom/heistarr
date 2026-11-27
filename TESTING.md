# Testing

Every push to `main` and every pull request runs the full suite on GitHub
Actions (`.github/workflows/ci.yml`). The **Release** workflow runs the same
suite first and publishes nothing if any of it fails.

| Layer | Where | What it checks | Time |
|---|---|---|---|
| Unit | `#[cfg(test)]` in each module | parser, rules, crypto, importer, auth, clients | seconds |
| Release-name corpus | `crates/core/tests/release_names.tsv` | title, year, resolution, source and audio languages of real release names | seconds |
| Server integration | `crates/server/tests/flow.rs` | the whole API against fake TMDB, Torznab, qBittorrent, Synology and Jellyfin started in the test | seconds |
| Browser (end to end) | `e2e/` | the real binary and web UI in Chromium, against fake services | ~20 s |
| Docker | CI `docker` job | the image builds | minutes |

Nothing in the tests talks to the Internet.

## Running them

```bash
cargo test --workspace
```

Browser tests (need Node 20+ and Python 3):

```bash
cargo build --release -p heistarr-server && ./scripts/build-web.sh
```

```bash
cd e2e && npm ci && npx playwright install chromium && npx playwright test
```

`e2e/start.sh` starts the fake services (`e2e/fakes.py`, ports 7101–7103) and
Heistarr on port 6499 with a fresh data folder in `e2e/.run/`. A failed run
leaves a screenshot in `e2e/test-results/`; in CI it is attached to the run
as the `playwright-report` artifact.

## Rules for new work

- **A bug gets a test first.** Reproduce it in a test that fails, then fix it.
- **A release name parsed wrong** goes into `release_names.tsv` with the
  right answer. The corpus only grows.
- **A new download client, indexer type or media server** gets a fake in
  `crates/server/tests/flow.rs` and an end-to-end test of grab → import.
- **A new screen or user flow** gets a step in `e2e/tests/`.
- **A schema change** is a new entry in `MIGRATIONS` (`crates/server/src/db.rs`),
  never an edit of a shipped one; `db.rs` tests check that reopening works and
  that a newer database is refused.
- Keep `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check` clean: CI fails on either.

## Before a release

The workflow enforces the first three; the rest is by hand.

1. `version` in `Cargo.toml` matches the release.
2. `CHANGELOG.md` has a section for it.
3. CI is green.
4. Try the new image on the NAS (`ghcr.io/byrandom/heistarr:edge` after the
   release, or build it locally) with your real Download Station.
