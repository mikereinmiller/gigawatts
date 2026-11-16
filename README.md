# Gigawatts

A local usage monitor for Claude Code. It shows how much of your usage limits you've used, what your work cost, and where it went: by repo, branch, skill, subagent, and model.

It spends no Claude tokens, changes nothing in Claude Code's settings, and never reads your login token. Your numbers stay on your machine.

Gigawatts is in early beta. Expect rough edges, and please report what breaks. It gets fresh limits through a Claude Code request that Anthropic hasn't documented, so a Claude Code release could change it; when that happens, Gigawatts marks the numbers it affects and keeps showing the rest.

## Requirements

- macOS or Linux.
- [Claude Code](https://code.claude.com), installed and logged in, with `claude` on your `PATH`. Gigawatts asks it for fresh limits.
- Rust, to build from source. The repo installs it with [mise](https://mise.jdx.dev), or use [rustup](https://rustup.rs).

## Install

```sh
git clone https://github.com/mikereinmiller/gigawatts.git
cd gigawatts
mise install        # installs Rust stable
mise run install    # builds, installs, and starts the engine
```

That puts `gigawatts` in `~/.cargo/bin`. Make sure that folder is on your `PATH`. Without mise, run `cargo install --path . --locked`, then `gigawatts stop` and `gigawatts engine`.

Then start the background engine at login, so history keeps recording while no window is open:

```sh
gigawatts service install
```

On macOS this adds a launchd agent; on Linux, a systemd user unit. Both start the engine right away.

## Use it

```sh
gigawatts
```

opens the terminal view: your limits with pace and reset countdowns, today's spend against your usual day, the sessions running now, and a spending table you can switch between repo, skill, subagent, and model. Press `?` for every key.

A red light at the right end of the title row means a limit is on course to fill before it resets, or is already full.

| Command | Does |
|---|---|
| `gigawatts` | Opens the terminal view, starting the engine if needed |
| `gigawatts status` | Prints one line: the closest limit and today's spend. `--all` prints one line per account, `--json` everything as JSON |
| `gigawatts report` | Opens the report page in your browser, at `http://127.0.0.1:7878` |
| `gigawatts data` | Prints cost history. `--range week`, `--by repo`, `--json` for scripts and agents |
| `gigawatts schema` | Prints the JSON Schema, with a description of every field, for `status --json`, `data --json`, and `data --limits --json` |
| `gigawatts accounts` | Lists the Claude Code accounts Gigawatts found and whether each is tracked |
| `gigawatts doctor` | Lists anything wrong, with its cause and fix |
| `gigawatts stop` | Stops the engine |

Every command takes `--help`.

## Update

```sh
cd gigawatts
git pull
mise run install
```

`mise run install` stops the running engine and starts the new one. Check what you're running with `gigawatts --version`.

## Accounts

Gigawatts tracks every Claude Code config directory it finds: `~/.claude`, plus any folder in your home directory that looks like one (for example one you use with `CLAUDE_CONFIG_DIR`). Each account's history is kept separately.

- `gigawatts accounts add <path>` tracks a folder somewhere else.
- `gigawatts accounts track <account> off` stops tracking an account and keeps its history.
- `gigawatts accounts forget <account>` deletes an account's history.

## Settings

Settings live in `~/.config/gigawatts/config.toml`. You don't need one to start. The keys an account section takes come from `gigawatts accounts`, which prints each account's ID.

```toml
# Keep per-request detail for 90 days; daily totals are kept forever either way.
request_detail_days = 90

# Serve the report page on another port, or 0 to turn it off.
report_port = 7878

[accounts."<account id>"]
label = "work"
tracking = true
store_titles = true                        # session titles in history
credit_reset = 2026-11-01T00:00:00Z        # when monthly credits reset, if not midnight UTC on the 1st
```

## What it stores, and where

History lives in `~/.local/share/gigawatts/history.sqlite3`, readable only by you. It holds counts, costs, models, repos, branches, times, session titles, and each account's email and IDs. It never stores prompts, responses, tool inputs, file contents, or commands, and Gigawatts sends nothing anywhere.

The report page listens on `127.0.0.1` only, but any user account on the same computer can reach it. On a shared machine, set `report_port = 0`.

## Uninstall

```sh
gigawatts service uninstall
gigawatts stop
cargo uninstall gigawatts
rm -rf ~/.local/share/gigawatts ~/.config/gigawatts   # deletes your history and settings
```

## Ask an agent about your spend

Gigawatts does the math; an agent can do the interpreting. Any agent that can run commands can answer questions from `gigawatts data --json`, and `gigawatts schema` tells it what each field means. Try asking:

- "Run `gigawatts data --range week --by skill --json` and tell me which skill cost the most this week and why."
- "Compare this month's spend by repo with last month's using `gigawatts data`, and point out anything that jumped."
- "Use `gigawatts data --range month --by repo --drill <repo> --json` to tell me which branches in this repo were the most expensive."

## License

MIT. See [`LICENSE`](LICENSE).

## How it works

[`docs/overview.md`](docs/overview.md) is the full design: where each number comes from, how costs are priced, and what's planned.
