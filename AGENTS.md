# Gigawatts

A local usage monitor for Claude Code. A background engine reads Claude Code's transcripts and usage limits into a private SQLite history; a terminal view, a report page, and JSON commands show it.

## Integrating: showing Gigawatts' numbers somewhere else

For a Herdr mod, a status bar, a script, a dashboard, or any other reader: read `docs/integrations.md`, then run `gigawatts schema` for what each field means. Build only on the JSON commands that guide names. The database, the engine's internal files, and the report page's `/api/*` routes change between releases.

## Developing Gigawatts

- `docs/overview.md` is the product spec, and the source of truth for behavior. `.claude/plans/beta-roadmap.md` is the current plan, slice by slice.
- Rust comes from `mise.toml`. Where `cargo` isn't on `PATH`, prefix every command with `mise exec --`.
  - Test: `mise exec -- cargo test`
  - Format: `mise exec -- cargo fmt`
  - Lint: `mise exec -- cargo clippy --all-targets`
  - Cost check on the real corpus: `mise exec -- cargo run --release -- check-costs`
- `mise run install` installs your build and restarts the engine on it. A running engine is replaced only by a different version, so every behavior change bumps the patch version in `Cargo.toml`.
- The JSON contract (`src/contract.rs`) is what integrations build on:
  - Every field and type `gigawatts schema` publishes (`Snapshot`, `DataResponse`, `LimitsResponse`, and what they contain) carries a doc comment, and `tests/cli.rs` fails on one without. The other types in the file belong to the report page's private `/api/*` routes.
  - Adding a field keeps `schema_version`. Removing or renaming one, or changing what it means, raises it.
  - A change readers would notice gets a matching edit to `docs/integrations.md`.
- Gigawatts reads Claude Code's files and never writes to them, and never reads its login token.
