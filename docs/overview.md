# Gigawatts project overview

Gigawatts is a local usage monitor for Claude Code. It shows how much of your usage limits you've used, what your work cost, and where it went, from a terminal window or the menu bar. It spends no Claude tokens, needs no changes to Claude Code's settings, and never handles your login.

Status: overview phase. No code yet.

## Goals

- **Glance.** Remaining limits, today's cost and pace, and which session is spending fastest, readable in a second.
- **History.** A permanent local record that survives Claude Code deleting old transcripts (after 30 days by default, `cleanupPeriodDays`).
- **Reports.** Cost by repo, branch, ticket, and model, over days, weeks, and months.
- **Light.** No wakeups while Claude Code is idle, and a memory footprint in the tens of MB.
- **Trustworthy for anyone.** Coworkers and strangers can install it without changing their setup, handing over credentials, or sending data anywhere.

## Non-goals

- **Monitoring other people.** Gigawatts shows one person their own usage. Team dashboards are what Anthropic's admin console is for.
- **Exact billing.** It reports the figures Claude Code records and labels anything it estimates. Anthropic's bill is the authority.
- **A server.** Nothing runs outside the user's own machines.

## Principles

1. **Read-only toward Claude Code.** Gigawatts reads Claude Code's files and never writes to them or to its settings. Optional integrations are installed by an explicit command, add to the user's setup without replacing anything, and come out with one command.
2. **Never touch credentials.** Anthropic's [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) says "developers may not collect, store, or intermediate Claude.ai credentials or session tokens." Gigawatts never reads the OAuth token. When it needs fresh limits, it asks the unmodified `claude` binary, which uses its own login.
3. **Every number shows its source and age.** For example: "32% weekly, from Claude Code, 4 min ago" or "$14.20 today, estimated."
4. **Event-driven.** It reacts to file changes and sets no polling timers while Claude Code is idle.
5. **Degrade instead of breaking.** Every source is undocumented or can change. When one changes, Gigawatts says which number is unavailable and keeps showing the rest.
6. **Numbers stay local.** The database holds counts, costs, models, repos, branches, times, and session titles. It never stores prompt or code text.

## How the numbers work

Gigawatts answers two different questions, from two different places.

- **"What did I use?"** Claude Code writes a line to the session's transcript for every request it makes: the time, working directory, git branch, model, and token counts. It also records the dollar cost it calculated. Gigawatts' Rust code adds these up into every total and breakdown: today's cost and pace, active sessions, and history by repo, branch, ticket, skill, and model. This runs continuously and is cheap. It wakes on file changes and reads only new lines.
- **"How much of my allowance is that?"** The limit percentages and reset times, like "25% of your 5-hour window". Only Anthropic knows your allowance, and it counts use from everywhere: claude.ai in a browser, the desktop app, and other machines on the same account. Local files never see that use, so Gigawatts can't calculate limits itself. It reads Anthropic's answer from Claude Code's cached copy, and refreshes it occasionally through Claude Code.

Limits only change while you use Claude, so while you're idle Gigawatts holds the last percentage until its reset time and asks nothing.

```
5-hour limit   25%   resets 12:09 AM      ◀ Anthropic, through Claude Code
Weekly         32%   resets Sat 9:59 PM   ◀ Anthropic, through Claude Code
────────────────────────────────────────
Today          $14.20   normal pace       ◀ transcripts
Active         dotfiles · main · $3.10    ◀ transcripts
This week      by repo, model, skill      ◀ transcripts, then the history database
```

## Data sources

| Source | What it gives | Documented | Role |
|---|---|---|---|
| Transcripts, `~/.claude/projects/**/*.jsonl` | Tokens per request, model, timestamps, working directory, git branch, and the cost Claude Code recorded (`cost-state` records with `costUSD`) | No | Primary source for cost, tokens, and history |
| `~/.claude.json`, `cachedUsageUtilization` | Limit percentages and reset times per window, credit usage (`extra_usage`), the account ID, and when Claude Code fetched it (`fetchedAtMs`) | No | Primary source for limits. Free to read, but only as fresh as Claude Code's last fetch |
| `get_usage` control request through an unmodified `claude -p` | Fresh limits and credit usage. Claude Code makes the request with its own login, costs no tokens, and refreshes `cachedUsageUtilization` | No (the SDK names it `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`) | Refresh. Runs only while Claude Code is in use and the cached copy is over about 10 minutes old, never while idle, with backoff on failure. Routine refreshes pass `skip_behaviors: true`; the skill and agent breakdown is fetched only when a view shows it. Measured on Claude Code 2.1.314: 1.1–1.9 s and 258–305 MB peak per run, released when the process exits. The cost is Claude Code starting up, so Gigawatts controls it by running it rarely |
| Status line JSON | Live `rate_limits` on every Claude Code event | [Yes](https://code.claude.com/docs/en/statusline) | Optional integration in version 1, off by default. Installed and removed with one command. Wraps the user's existing status line script and records what it receives, so limits stay current to the second while a session runs, at no cost |

Sources considered and rejected:

- **Calling `api.anthropic.com/api/oauth/usage` directly.** It requires reading the user's OAuth token, which the credential policy above rules out. Every tool that does it also fights repeated Keychain prompts and 429 rate-limit errors.
- **OpenTelemetry export.** It needs environment variables in Claude Code's settings, and company managed settings can override them.
- **Admin and Analytics APIs.** They need an organization admin key and don't serve individual subscribers.
- **The credit formula in `cachedGrowthBookFeatures`.** It's a server-pushed feature flag. A local test landed near the real figure (27% estimated against 25% reported) but proves nothing from one sample.

Cost uses the figure Claude Code recorded when present, and a pinned, overridable price table otherwise. Anything priced from the table is labeled "estimated." ccusage's `auto` cost mode uses the same order.

## Architecture

```
Claude Code's files ──▶ engine ──▶ SQLite history ──▶ views
  transcripts            readers     + snapshot          terminal
  ~/.claude.json         cost model  (view contract)     macOS menu bar
  claude -p get_usage    rollups                         Omarchy bar
```

- **Engine (Rust).** One process at a time holds a lock file and is the only writer to the database. It usually runs as a background login item. If none is running, the terminal view takes the lock and does the reading itself, so the terminal view works on its own.
- **View contract.** A schema-versioned snapshot (a JSON file written by temp file and rename, also printed by `gigawatts --json`) holds the current numbers. The SQLite database, opened read-only, holds history. Every view reads only these two. This contract is the main design surface: views stay thin, and a new view never needs engine changes.
- **Views.** Each platform gets the view that fits it. Views hold no logic.

### Engine internals

| Area | Choice | Alternative considered |
|---|---|---|
| File watching | [`notify`](https://docs.rs/notify) 8.2 with `notify-debouncer-full` 0.7. Events only trigger a re-check of that file. A full scan runs at startup, when the watcher reports dropped events, and every few minutes as a safety net | A stat loop over known files, which wakes the machine constantly |
| Reading transcripts | Per file: byte offset, file identity, and a hash of the first line. Only complete new lines are read. The offset is saved in the same transaction as the parsed data, so a crash can't double-count. This is the pattern [Vector](https://vector.dev/docs/reference/configuration/sources/file/) and [fluent-bit](https://docs.fluentbit.io/manual/pipeline/inputs/tail) use | `linemux`, last released 2022 |
| Parsing | `serde_json` into narrow borrowed structs, skipping lines without `"usage"` before parsing | `sonic-rs` or `simd-json`, only if the first full read proves slow |
| Storage | `rusqlite` 0.41 (bundled SQLite), WAL mode, migrations with `rusqlite_migration`, tiered retention (below) | `sqlx` needs an async runtime. `redb` has no SQL for rollups |
| Concurrency | Plain threads and channels, all blocking while idle | `tokio`, unnecessary without async networking |
| Tests | `insta` snapshot tests over trimmed real transcripts, and `proptest` checks that the same bytes split into arbitrary appends produce the same totals. Unrecognized record types are counted, so format drift shows up instead of reading as zero | Hand-written unit tests only |

### Storage and retention

Rows are written only when something happens in a transcript, never on a timer. Only the request table grows with activity, so only it has a time limit.

| Table | One row per | Kept |
|---|---|---|
| Requests | API request | 90 days (a setting) |
| Daily totals | Day × account × repo × skill × agent × model, updated in place | Forever |
| Sessions | Session, updated in place | Forever |
| Limit readings | Change in a limit percentage | 90 days raw, then the daily peak per limit forever |
| File positions | Transcript file, updated in place | While the file exists |

Request rows exist because a request's cost isn't final when it first appears. Claude Code records the actual cost later, covering several requests at once, so recent rows get corrected and their daily totals recomputed from them. Reports read only the daily totals and session rows, so they stay fast whether or not the request detail still exists. A daily cleanup deletes expired rows and runs SQLite's incremental vacuum so the file actually shrinks.

Measured in an earlier design session (not rerun since):

| Measure | Result |
|---|---|
| Your usage | About 350 requests per active day (peak 868), about 6 MB a year |
| Heavy user (5,000 requests a day) | About 30 MB once the 90-day window fills, then about 5 MB a year |
| Row size | About 67 bytes per request row |
| Today's total, 1 million rows | 0.4 ms |
| A month grouped by skill and model | 18 ms |

A test in the repo generates a synthetic heavy year, runs the cleanup, and fails the build if the database exceeds 50 MB, today's query takes over 5 ms, or a yearly report takes over 100 ms.

## Views

| View | Platform | Technology | When |
|---|---|---|---|
| Terminal: full view, one-line summary, and JSON | All | Rust, [`ratatui`](https://ratatui.rs) 0.30, redrawn only when data changes | Version 1 |
| Menu bar with a popover and charts | macOS | SwiftUI, reading the view contract. No Rust bindings | Next |
| Floating terminal window on a key binding | Omarchy | The terminal view | Version 1, no extra code |
| Bar widget with a popup | Omarchy | A QML plugin in `~/.config/omarchy/plugins/<id>/`, the same plugin format Omarchy's own widgets use | Later |
| Tray icon | Windows | To be decided. Rust `tray-icon` is the likely candidate | Later |

Why SwiftUI for the macOS view, and the alternatives considered:

- **SwiftUI** gives a native popover, Swift Charts, accessibility, and template icons with no extra process. The Swift app reads the contract, so it stays small and holds no logic. Rust engines with Swift views run in production in Element X and Firefox.
- **Tauri** was rejected. Its main advantage, one panel for every OS, fails on Omarchy: Wayland doesn't let an app position its own window under a tray icon, and the panel wouldn't be part of Omarchy's bar. It also adds WebKit processes (about 30 MB in [one developer's prototype](https://github.com/Jbmanllr/accroster/issues/9)).
- **Rust `tray-icon`** shows only a text menu, which can't hold the charts.
- **`objc2`** (AppKit from Rust) means hand-drawn charts in unsafe code, and the library is mostly one maintainer's work.

## Product

### Menu bar icon and panel

- **The icon is a ring gauge** that fills toward whichever limit is closest. It's monochrome while normal (a template icon on macOS, tinted by the system), turns amber at 70% and red at 90%, and shows a dashed outline when the data is stale. It changes in 10% steps with a small buffer before changing back, so it never flickers, and it's never animated.
- **One number sits beside the icon** where the platform allows text. The headline is a setting. The default shows the closest limit's percentage, falling back to today's dollars when every limit is under 50%.
- **The panel**, top to bottom: each limit as a bar with its percentage and reset countdown; today's dollars and pace; active sessions with repo, branch, model, and session dollars, each expandable into its split by skill and model; the month's total and pace, with credits where the plan has them; and a footer with the data's age, a refresh button, and a link to the terminal view.
- **Wording names the limit, the number, and the reset time,** for example "5-hour limit at 91%, resets in 1h 20m".

### Accounts

Each Claude Code account (each `CLAUDE_CONFIG_DIR`) is tracked separately, and its numbers are never combined with another's. The views show one account at a time:

- **Follow active (default):** switch to whichever account you used last.
- **Pinned:** stay on the account you pick.
- **Show all:** one section per account, side by side, still never added together.

### Notifications

- At 80% and 95% of a limit, once per window per account. Thresholds are settings, and notifications can be turned off.
- A reset notice, sent only if you actually hit that limit.
- Every notification names its account, for example "Personal: 5-hour limit at 91%".

### Themes

- **The default theme is generic:** the ring gauge and plain wording above. Most people never open settings, so the default has to look clean and say things plainly.
- **Movie themes are opt-in,** split into a voice (wording) and a look (icons). Voices are short text files: Ghostbusters, Back to the Future, Spinal Tap, Hitchhiker's Guide, Spaceballs, Star Trek, and Jaws. Looks need an icon set per state, drawn to read at 16 pixels in light and dark, so they come one at a time.
- **A joke never hides a number.** "Don't cross the streams!" still comes with "5-hour limit at 91%, resets in 1h 20m".

## Features

Version 1, the engine and terminal view:

- Remaining limits for each window, with source and age.
- Today's cost and pace against your own normal pace.
- Active sessions, with the fastest-spending one highlighted.
- Durable history that outlives transcript cleanup.
- Breakdowns by repo, branch, ticket (from the ID in the branch name), and model.
- Multiple accounts on one machine kept apart, using `CLAUDE_CONFIG_DIR` and the account ID.
- Credit-based plans, using `extra_usage`.
- The optional status line integration, for limits that are current to the second.

Next:

- The macOS menu bar view.
- Notifications at limit thresholds, when a window resets, and when a session spends far above your normal rate.
- A forecast of when you'll hit a limit compared with when it resets.

Later:

- The Omarchy bar widget.
- Combined totals across your machines, by syncing daily rollups through a shared folder.
- The Windows view.
- Movie theme looks, one at a time.
- A one-time import of earlier `/spend` history (`~/.claude/spend/`), so months whose transcripts Claude Code already deleted still appear.

## Existing tools and the gaps Gigawatts fills

| Tool | Approach | Recurring problems in its issue tracker |
|---|---|---|
| [CodexBar](https://github.com/steipete/CodexBar), Swift | Reads the OAuth token and calls the usage endpoint | Keychain prompts, battery drain, memory growth to 7 GB, a 100% CPU loop, 429 errors |
| [ccusage](https://github.com/ccusage/ccusage), Rust (via npm) | Reads logs on every run, no background process | Format changes broke cost totals. A request to keep history past log deletion was declined |
| [openusage](https://github.com/robinebers/openusage), Swift | Its own login flow, refreshes every 5 minutes | 429 errors. It's removing cost figures because they were wrong |
| [Claude-Code-Usage-Monitor](https://github.com/Maciek-roboblog/Claude-Code-Usage-Monitor), Python | Logs, plus an optional status line hook. Labels each number by its source | Wrong token counts and reset times |
| [tokscale](https://github.com/junhoyeo/tokscale), Rust | Logs from about 50 agents, priced with LiteLLM | Running out of memory, inflated counts |

Gigawatts targets what none of them do well: history that outlives log deletion, light and steady background running, honest labeling of every number, separate accounts on one machine, credit-based plans, and no credential handling.

## Performance targets

- **Idle.** Zero wakeups while Claude Code is idle. Any timer gets tolerance so the OS can coalesce wakeups, per Apple's [energy guidance](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/Timers.html).
- **Memory.** The engine stays in the low tens of MB.
- **First run.** One full read of existing transcripts, then incremental reads of new lines only.
- **Checked every release.** `powermetrics` and `timerfires` on macOS, `perf` on Linux, against these targets.

## Distribution

- **CLI and engine.** A Homebrew tap built with [`cargo-dist`](https://github.com/axodotdev/cargo-dist), with the engine run through `brew services` on macOS and a systemd user service on Linux.
- **macOS view.** A signed and notarized cask. That needs an Apple Developer ID ($99 a year), and Homebrew 5 disables casks that fail Gatekeeper. `cargo-dist` doesn't notarize, so CI runs `notarytool` itself.
- **Command.** The binary is `gigawatts`, so it can't collide with other tools. Users can add their own short alias such as `gw`.
- **Name.** Gigawatts uses no Anthropic or Claude Code names or logos in its product name, per the legal page. "A usage monitor for Claude Code" in a description is allowed.

## Risks

- **Undocumented formats.** Transcripts, `cachedUsageUtilization`, and `get_usage` can change in any Claude Code release. Each source sits behind its own reader with fixture tests, and the app keeps working with whatever remains.
- **`get_usage` rate limits.** It likely hits the same rate-limited endpoint other tools fight. Refreshes stay infrequent, back off on failure, and fall back to the cached copy.
- **Swift adds a second language.** The view is kept thin so the Swift code stays small.

## Open questions

1. **Repo hosting and license.** Personal account or employer org. An employer IP policy may decide this, and moving later is awkward.
2. **The work Mac's headline numbers.** Whether the work account reports 5-hour and weekly windows or monthly credits. Running `get_usage` on that machine settles it.
3. **Whether Anthropic is comfortable with scheduled `get_usage` calls.** It runs Claude Code unmodified with the user's own login, which the policy page allows, but nothing public addresses running it on a schedule. Worth asking Anthropic before publishing widely.
4. **The Windows view technology.** Decided when Windows work starts.
