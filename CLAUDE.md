# Gigawatts

`docs/overview.md` is the product spec. `.claude/plans/gigawatts-v1.md` is the build plan, tracked in git.

## Commands

Rust comes from `mise.toml`. `cargo` is not on PATH in Claude's shell, so prefix every command with `mise exec --`.

- Test: `mise exec -- cargo test`
- Format: `mise exec -- cargo fmt`
- Lint: `mise exec -- cargo clippy --all-targets`
- Cost check on the real corpus: `mise exec -- cargo run --release -- check-costs`
