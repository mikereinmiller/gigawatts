# Gigawatts project overview

Gigawatts is a local usage monitor for Claude Code. It shows how much of your usage limits you've used, what your work cost, and where it went, from a terminal window or the menu bar. It spends no Claude tokens, needs no changes to Claude Code's settings, and never handles your login.

Status: version 1 shaped. No code yet.

## Goals

- **Glance.** Remaining limits, today's cost and pace, and which session is spending fastest, readable in a second.
- **History.** A permanent local record that survives Claude Code deleting old transcripts (after 30 days by default, `cleanupPeriodDays`).
- **Reports.** Cost by repo, branch, skill, agent, and model, over days, weeks, and months.
- **Light.** No wakeups while Claude Code is idle, and a memory footprint in the tens of MB.
- **Trustworthy for anyone.** Coworkers and strangers can install it without changing their setup, handing over credentials, or sending data anywhere.

## Non-goals

- **Monitoring other people.** Gigawatts shows one person their own usage. Team dashboards are what Anthropic's admin console is for.
- **Exact billing.** It reports the figures Claude Code records and labels anything it estimates. Anthropic's bill is the authority.
- **A server.** Nothing runs outside the user's own machines.
- **Calling a model.** Gigawatts never sends anything to Claude. Judgment about the numbers comes from the insights skill, which runs only when you ask Claude for it.

## Principles

1. **Read-only toward Claude Code.** Gigawatts reads Claude Code's files and never writes to them or to its settings. Optional integrations are installed by an explicit command, add to the user's setup without replacing anything, and come out with one command.
2. **Never touch credentials.** Anthropic's [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) says "developers may not collect, store, or intermediate Claude.ai credentials or session tokens." Gigawatts never reads the OAuth token. When it needs fresh limits, it asks the unmodified `claude` binary, which uses its own login.
3. **Every number shows its source and age.** For example: "32% weekly, from Claude Code, 4 min ago" or "$14.20 today, estimated."
4. **Event-driven.** It reacts to file changes and sets no polling timers while Claude Code is idle.
5. **Degrade instead of breaking.** Every source is undocumented or can change. When one changes, Gigawatts marks the numbers it affects and keeps showing the rest.
6. **Numbers stay local.** The database holds counts, costs, models, repos, branches, times, and session titles. It never stores prompts, responses, tool inputs, file contents, or commands. Gigawatts never sends your data anywhere. Its one network request downloads Anthropic's public pricing page, and it can be turned off.

## Stages

- **Version 1.** The engine and the terminal view, working correctly on macOS and Omarchy Linux, for one user (the author) on two machines.
- **Version 1.1.** Forecasts, the HTML report page, and the insights skill.
- **Version 2.** The macOS menu bar, notifications, and themes.
- **Later.** Packaging for other people, the Omarchy bar widget, syncing across machines, Windows, and the status line integration if a use case appears.

## How the numbers work

Gigawatts answers two different questions, from two different places.

- **"What did I use?"** Claude Code writes a line to the session's transcript for every request it makes: the time, working directory, git branch, model, skill, agent, and token counts. When the session ends, it also records the dollar cost it calculated. Gigawatts' Rust code adds these up into every total and breakdown: today's cost and pace, active sessions, and history by repo, branch, skill, agent, and model. This runs continuously and is cheap. It wakes on file changes and reads only new lines.
- **"How much of my allowance is that?"** The limit percentages and reset times, like "25% of your 5-hour window". Only Anthropic knows your allowance, and it counts use from everywhere: claude.ai in a browser, the desktop app, cloud sessions, and other machines on the same account. Local files never see that use, so Gigawatts can't calculate limits itself. It asks Anthropic through Claude Code while you work, and again whenever you look.

```
5-hour limit       5%   even pace    resets 12:09 AM   ◀ Anthropic, through Claude Code
Weekly            33%   under pace   resets Sat 9:59 PM
Weekly, Fable     45%   under pace   resets Sat 9:59 PM
────────────────────────────────────────
Today          $14.20   1.8× your usual by 2 PM        ◀ transcripts
Active         orchard-shop-api · ab/rel42-carts · $2.10 in 15 min
This week      by repo, branch, skill, agent, model    ◀ transcripts, then the history database
```

### What Gigawatts can and can't itemize

| Where you use Claude | In the limit percentages | In Gigawatts' cost history |
|---|---|---|
| Claude Code in a terminal | Yes | Yes |
| Claude Code in the desktop app, local session | Yes | Yes. Claude Code writes these transcripts to the same `projects/` folder ([docs](https://code.claude.com/docs/en/claude-directory)) |
| Cowork | Yes | Yes, same transcript store |
| Cloud sessions (claude.ai/code, or "cloud" in the desktop app) | Yes | No, they run on Anthropic's servers |
| claude.ai and desktop app chat | Yes | No, stored on Anthropic's servers |

Claude Code keeps desktop and Cowork transcripts indefinitely unless `desktopSessionCleanupPeriodDays` is set, so only terminal sessions are on the 30-day clock.

## Data sources

| Source | What it gives | Documented | Role |
|---|---|---|---|
| Transcripts, `~/.claude/projects/**/*.jsonl` | Per request: tokens by type (input, output, thinking, cache reads, 5-minute and 1-hour cache writes, web searches), model and requested model, effort, speed, `inference_geo`, `attributionSkill`, `attributionAgent`, working directory, git branch, time, and `requestId`. Per session: `cost-state` records with the cost per model (written when the session ends), durations, lines changed, the session title (`ai-title`), and tools used | Partly. [Location and retention](https://code.claude.com/docs/en/claude-directory) are documented, the line format isn't | Primary source for cost, tokens, and history |
| `.claude.json`, `cachedUsageUtilization` | A generic `limits` list (kind, scope, percent, severity, reset time), credits in `spend` (older `extra_usage` as a fallback), a weekly breakdown by surface, the account ID, and when it was fetched (`fetchedAtMs`). Claude Code doesn't refresh it during ordinary use | No | Read after every refresh. On its own it can be days old |
| `get_usage` control request through an unmodified `claude -p` | Fresh limits and credits, refreshing `cachedUsageUtilization`. Without `skip_behaviors`, also the `behaviors` section that `/usage` shows as "What's contributed to your usage": behavior shares, top agents and skills, and recommendations. Costs no tokens | No (the SDK names it `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`) | Limits refresh every 10 minutes per active account, and when you look (see [Stale data and errors](#stale-data-and-errors)). `behaviors` fetched once a day per active account. Measured on Claude Code 2.1.314: 1.1–1.9 s and 258–305 MB peak per run, released when the process exits |
| Anthropic's pricing page as Markdown, `https://platform.claude.com/docs/en/about-claude/pricing.md` | Per-token prices by model and token type, plus pricing rules (long prompts on Haiku 5.5, US-only inference at 1.1×, fast mode) | Yes, it's Anthropic's public docs | Daily price update. See [Cost](#cost) |
| Status line JSON | Live `rate_limits` on every Claude Code event | [Yes](https://code.claude.com/docs/en/statusline) | Possible later enhancement, not scheduled. It would be installed and removed with one command, wrap the user's existing status line script, and record what it receives, so limits stay current to the second while a session runs, at no cost |

Sources considered and rejected:

- **Calling `api.anthropic.com/api/oauth/usage` directly.** It requires reading the user's OAuth token, which the credential policy above rules out. Every tool that does it also fights repeated Keychain prompts and 429 rate-limit errors.
- **OpenTelemetry export.** It needs environment variables in Claude Code's settings, and company managed settings can override them.
- **Admin and Analytics APIs.** They need an organization admin key and don't serve individual subscribers.
- **The credit formula in `cachedGrowthBookFeatures`.** It's a server-pushed feature flag. A local test landed near the real figure (27% estimated against 25% reported) but proves nothing from one sample.
- **LiteLLM's price list.** A third party's copy of Anthropic's pricing page, when the page itself is available as Markdown.
- **Working prices out from Claude Code's recorded costs.** A best fit over 216 per-model session totals missed most sessions by more than 0.5% and produced negative prices, because the pricing rules above vary by request.

## Accounts

An account is one Claude Code config directory (`~/.claude`, or wherever `CLAUDE_CONFIG_DIR` points), identified by the account ID inside its `.claude.json`, not by the folder's name. Everything is stored per account from day one, and accounts are never added together.

- **Finding accounts.** Gigawatts always checks `~/.claude`. At engine start it scans one level into the home folder for any directory shaped like a config directory (a `projects/` folder and a `.claude.json`), whatever it's named. Extra paths go in the config. `gigawatts accounts` shows what was found and whether each is tracked.
- **Tracking versus displaying.** Tracking means the engine reads an account's transcripts and limits into history. It runs whether or not anything is on screen, because a gap can't be filled after Claude Code deletes the transcripts. Tracking is on by default for every account found, a newly found account is announced in the view, and it can be turned off per account. Off means Gigawatts never reads that directory. Turning it off keeps existing history, and `gigawatts accounts forget` deletes it.
- **Limits belong to the account.** If two directories are ever logged into the same account, they share one set of limits and keep separate cost history.
- **A changed login.** If the account ID inside a directory changes, Gigawatts says so instead of mixing the two accounts' history.

## Architecture

```
Claude Code's files ──▶ engine ──▶ SQLite history ──▶ snapshot + gigawatts data ──▶ views
  transcripts            readers     (private)          (view contract)               terminal
  .claude.json           cost model                                                   report page (1.1)
  claude -p get_usage    rollups                                                      insights skill (1.1)
  pricing.md                                                                          macOS menu bar (2)
```

- **Engine (Rust).** Always its own background process, and the only writer to the database. A lock file stops a second engine from starting. Opening the terminal view starts the engine if it isn't running, and `gigawatts service install` adds a systemd user unit on Linux or a launchd agent on macOS so it also starts at login (`gigawatts service uninstall` removes it). Once started, it runs until the machine restarts or `gigawatts stop`. Closing the view never stops it. While Claude Code is idle it sleeps with zero wakeups.
- **Upgrades.** The engine writes its version into the snapshot. A newer view that finds an older engine stops it and starts its own, which runs any database migrations. If the engine is newer than the view, the view says to reopen it. Restarting is always safe, because read positions are saved in the same transaction as the data.
- **View contract.** The database is private to the engine. Every consumer reads only two things: the snapshot, a JSON file the engine rewrites after each batch of changes with every "now" number per tracked account, and `gigawatts data --json`, which answers history queries by range and grouping. The terminal view calls the same Rust code in-process, and the skill and menu bar run the command. So every number has one implementation, and the database layout can change freely between releases.
- **Schema versioning.** Every snapshot and data response carries an integer `schema_version`. Adding a field doesn't change it, and consumers ignore fields they don't know. Removing or renaming a field, or changing what one means, raises it. `gigawatts schema` prints a JSON Schema generated from the Rust types with `schemars`.
- **Views.** Each platform gets the view that fits it. Views hold no logic.

### Engine internals

| Area | Choice | Alternative considered |
|---|---|---|
| File watching | [`notify`](https://docs.rs/notify) 8.2 with `notify-debouncer-full` 0.7. Events only trigger a re-check of that file. A full scan runs at startup, when the watcher reports dropped events, and every few minutes as a safety net while Claude Code is active. The safety-net scans stop 10 minutes after the last file event, so nothing wakes while Claude Code is idle | A stat loop over known files, which wakes the machine constantly |
| Reading transcripts | Per file: byte offset, file identity, and a hash of the first line, so only complete new lines are read. The offset is saved in the same transaction as the parsed data. Requests are stored by `requestId` and a repeated one is ignored, because Claude Code sometimes sets a transcript aside and writes a new one for the same session. Files named `*.superseded-*` or `*.orphaned-*` are skipped | `linemux`, last released 2022 |
| Parsing | `serde_json` into narrow borrowed structs, skipping lines without `"usage"` before parsing | `sonic-rs` or `simd-json`, only if the first full read proves slow |
| Storage | `rusqlite` 0.41 (bundled SQLite), WAL mode, migrations with `rusqlite_migration`, retention below | `sqlx` needs an async runtime. `redb` has no SQL for rollups |
| Concurrency | Plain threads and channels, all blocking while idle | `tokio`, unnecessary without async networking |
| Tests | `insta` snapshot tests over trimmed real transcripts, and `proptest` checks that the same bytes split into arbitrary appends produce the same totals. Unrecognized record types are counted, so format drift shows up instead of reading as zero | Hand-written unit tests only |

### What gets recorded

Every numeric and categorical field Claude Code provides per request and per session is recorded: all token splits, cost, model and requested model, effort, speed, `inference_geo`, skill, agent, repo, branch, durations, lines changed, and tool names. Session titles are the one free text stored, and can be turned off per account. Nothing you or the model wrote is stored. A field not recorded within 30 days can never be added to older history, which is why the rule is wide.

Anthropic's `behaviors` section from `get_usage` is saved once a day per active account, since it covers only today and this week and isn't kept anywhere else.

### Storage and retention

Rows are written only when something happens, never on a timer.

| Table | One row per | Kept |
|---|---|---|
| Requests | API request, keyed by `requestId` | Forever by default. A setting limits it to a number of days |
| Daily totals | Day × account × repo × skill × agent × model, updated in place | Forever |
| Sessions | Session, updated in place | Forever |
| Limit readings | Change in a limit percentage, per account | Forever |
| Usage advice | Day × account, from `behaviors` | Forever |
| File positions | Transcript file, updated in place | While the file exists |

Request costs are provisional until Claude Code writes the session's `cost-state` when the session ends. Then the session's requests are checked and their daily totals recomputed. If request detail is limited by the setting, a daily cleanup deletes expired rows and runs SQLite's incremental vacuum so the file actually shrinks, and reports fall back to the daily totals and session rows.

Measured in an earlier design session (not rerun since):

| Measure | Result |
|---|---|
| Your usage | About 350 requests per active day (peak 868), about 6 MB a year at the original row size |
| Heavy user (5,000 requests a day) | About 30 MB once the 90-day window fills, then about 5 MB a year |
| Row size | About 67 bytes per request row, before the wider field set |
| Today's total, 1 million rows | 0.4 ms |
| A month grouped by skill and model | 18 ms |

Keeping every request with the wider field set is estimated at 10 to 15 MB a year at your usage, not yet measured. A test in the repo generates a synthetic heavy year and fails the build if the database exceeds 50 MB with the retention setting at 90 days, today's query takes over 5 ms, or a yearly report takes over 100 ms.

## Cost

Claude Code records a dollar cost per session and model, and only when the session ends. It never records a cost per request, and while a session runs it has recorded no cost at all. So Gigawatts prices each request itself:

1. Each request is priced from its tokens by type and the price table, applying the pricing rules its line records (long prompts on Haiku 5.5, `inference_geo` of `us`, fast mode). Until the session ends, that cost is labeled "estimated."
2. When the `cost-state` arrives, the requests of each model are added up and compared with Claude Code's figure. If they match, the costs are exact and carry no label.
3. If they don't match, the requests are scaled to Claude Code's total so every breakdown still adds up to it, the session is labeled "adjusted," and the gap is recorded. A gap means a price or a pricing rule is out of date, and `gigawatts doctor` names the model.

On a subscription, these dollars are what the usage would cost at API prices, not what you're charged.

The price table comes from three layers. A built-in table is copied by hand from Anthropic's pricing page, with the date and link, for the first run and offline use. Once a day while Claude Code is in use, Gigawatts downloads `pricing.md`, reads its model pricing table, and maps names like "Claude Opus 5.5" to IDs like `claude-opus-5-5`. If that table can't be read or a price looks implausible, it keeps the last good copy and says so. Overrides in `config.toml` win over both. The pricing rules are prose on the page and change rarely, so they live in code.

## Pace

- **Limit pace.** Each limit compares how much is used with how much of its window has passed. Used less than has passed means under pace: at your average rate so far, you won't run out before the reset. The 5-hour limit always compares against even pace. Weekly limits start with even pace and switch to your own usual progress at this point in the week once four full weeks of readings exist, and the view labels which comparison it shows.
- **Dollar pace.** Today's spend so far is compared with your usual spend by this clock time, meaning the median of your last 20 days of the same kind (workdays or weekend days) that had any usage. It flags an unusual day and says nothing about limits.
- **Fastest-spending session.** Among sessions that wrote to their transcript in the last 15 minutes, the one that spent the most in those 15 minutes, shown with that rate and its total. Subagent requests count toward the session that launched them.

## Attribution

- **Repo.** When Gigawatts first sees a working directory, it asks git which repository the directory belongs to, resolving a worktree to its main checkout, and stores the repo's name, path, and remote URL with the request. It has to happen then, because a deleted worktree can't be resolved later. Directories outside git count as themselves, and directories under `/tmp` are grouped as "temporary."
- **Branch.** From the line's `gitBranch`. When that is `HEAD`, as during a rebase, Gigawatts asks git for the branch being worked on, and shows "detached" if git can't say.
- **Skill and agent.** Straight from `attributionSkill` and `attributionAgent`. Lines with neither belong to the main conversation with no skill.
- **No tickets.** Cost breaks down by repo and branch. Grouping branches into a larger piece of work is left to asking Claude, which needs no tracker or naming scheme. Branches are stored forever, so a ticket grouping could still be added later over all history.

## Terminal view and CLI

The full view is one screen. The top always shows the current account's limits with pace and age, today's spend against usual, and active sessions with the fastest highlighted. Below it, a history table where one key cycles the grouping (repo, branch, skill, agent, model), another cycles the range (today, week, month, all), and `Enter` drills into a row, for example a repo to its branches to their sessions. In a small window, such as Omarchy's floating terminal, the history collapses behind a key. Anthropic's usage advice sits behind its own key.

It shows one account at a time, starting with the one used most recently, with its name in the header. `Tab` switches accounts, and `a` toggles a stacked view with every account's limits and today's spend.

| Command | Does |
|---|---|
| `gigawatts` | Opens the full view, starting the engine if needed |
| `gigawatts status` | Prints one line: the closest limit with its pace, and today's spend against usual. `--account` picks an account and `--all` prints one line per account. Provisional until used for a while |
| `gigawatts data` | Answers a history query by range and grouping. A readable table by default, versioned JSON with `--json` |
| `gigawatts schema` | Prints the JSON Schema for the snapshot and `data --json` |
| `gigawatts accounts` | Lists accounts and their tracking state. `add <path>`, tracking on or off, and `forget <account>` to delete history |
| `gigawatts service install` / `uninstall` | Adds or removes the login service |
| `gigawatts stop` | Stops the engine |
| `gigawatts doctor` | Lists every current problem with its cause and fix |
| `gigawatts report` (1.1) | Writes a self-contained HTML report page with charts and opens it in the browser |

## Stale data and errors

- **Stale limits.** Every limit shows its age. Opening the view refreshes any account whose reading is over 10 minutes old. `gigawatts status` prints the saved reading and asks the engine to refresh in the background, at most once per 10 minutes. A reading over an hour old is greyed out with "may not include use outside Claude Code," since chat and cloud sessions only show up through limits.
- **Problems.** Each affected number carries a short marker, like "limits: refresh failing since 2 PM" or "cost estimated: no price for claude-opus-5-6." The footer shows when anything is wrong, and `gigawatts doctor` lists each problem with its cause and what to do. Watched problems include unrecognized transcript lines, unknown models, "adjusted" sessions, failing refreshes, and an unreadable `pricing.md`.

## Settings

Settings live in `~/.config/gigawatts/config.toml` on both platforms, and data (database and snapshot) in `~/.local/share/gigawatts/`, readable only by you. Commands like `gigawatts accounts add` edit the TOML file with `toml_edit`, keeping comments and layout. Per-account settings are keyed by account ID, so one file kept in dotfiles works on every machine, and extra paths that don't exist on a machine are skipped.

| Setting | Default |
|---|---|
| Limit refresh interval while active | 10 minutes |
| Days of request detail to keep | Unset, meaning forever |
| Extra account paths | None |
| Tracking, per account | On |
| Label, per account | The account's email |
| Store session titles, per account | On. Turning it off offers to delete stored titles |
| Price overrides, per model | None |
| Daily `pricing.md` download | On |

## Insights skill (version 1.1)

An [Agent Skill](https://agentskills.io/specification) in the Gigawatts repo, so it works in Claude Code and other agents that load the format. It reads `gigawatts data --json` and turns the numbers into answers and suggestions: where the spend went, what changed over time, and whether a change helped, each with the numbers behind it and the specific setting or file to change. Gigawatts calculates, and the skill interprets. It builds on Anthropic's own advice from `behaviors` rather than repeating it, and can add its findings to the 1.1 report page. What it does when invoked gets designed in 1.1, around the gaps version 1 leaves, with history to compare. `/spend` keeps working until then. An MCP server isn't planned.

## Views

| View | Platform | Technology | When |
|---|---|---|---|
| Terminal: full view, one-line summary, and `gigawatts data` | All | Rust, [`ratatui`](https://ratatui.rs) 0.30, redrawn only when data changes | Version 1 |
| Floating terminal window on a key binding | Omarchy | The terminal view | Version 1, no extra code |
| HTML report page | All | A self-contained page with inline charts, written by `gigawatts report` | Version 1.1 |
| Menu bar with a popover and charts | macOS | SwiftUI, reading the view contract. No Rust bindings | Version 2 |
| Bar widget with a popup | Omarchy | A QML plugin in `~/.config/omarchy/plugins/<id>/`, the same plugin format Omarchy's own widgets use | Later |
| Tray icon | Windows | To be decided. Rust `tray-icon` is the likely candidate | Later |

Why SwiftUI for the macOS view, and the alternatives considered:

- **SwiftUI** gives a native popover, Swift Charts, accessibility, and template icons with no extra process. The Swift app reads the contract, so it stays small and holds no logic. Rust engines with Swift views run in production in Element X and Firefox.
- **Tauri** was rejected. Its main advantage, one panel for every OS, fails on Omarchy: Wayland doesn't let an app position its own window under a tray icon, and the panel wouldn't be part of Omarchy's bar. It also adds WebKit processes (about 30 MB in [one developer's prototype](https://github.com/Jbmanllr/accroster/issues/9)).
- **Rust `tray-icon`** shows only a text menu, which can't hold the charts.
- **`objc2`** (AppKit from Rust) means hand-drawn charts in unsafe code, and the library is mostly one maintainer's work.

## Menu bar (version 2)

The menu bar icon and panel, notifications, and themes are version 2.

### Menu bar icon and panel

- **The icon is a ring gauge** that fills toward whichever limit is closest. It's monochrome while normal (a template icon on macOS, tinted by the system), turns amber at 70% and red at 90%, and shows a dashed outline when the data is stale. It changes in 10% steps with a small buffer before changing back, so it never flickers, and it's never animated.
- **One number sits beside the icon** where the platform allows text. The headline is a setting. The default shows the closest limit's percentage, falling back to today's dollars when every limit is under 50%.
- **The panel**, top to bottom: each limit as a bar with its percentage and reset countdown; today's dollars and pace; active sessions with repo, branch, model, and session dollars, each expandable into its split by skill and model; the month's total and pace, with credits where the plan has them; and a footer with the data's age, a refresh button, and a link to the terminal view.
- **Wording names the limit, the number, and the reset time,** for example "5-hour limit at 91%, resets in 1h 20m".
- **Accounts** use the same rules as the terminal view: follow the most recent account by default, pin one, or show all side by side, never added together.

### Notifications

- At 80% and 95% of a limit, once per window per account. Thresholds are settings, and notifications can be turned off.
- A reset notice, sent only if you actually hit that limit.
- Every notification names its account, for example "Personal: 5-hour limit at 91%".

### Themes

- **The default theme is generic:** the ring gauge and plain wording above. Most people never open settings, so the default has to look clean and say things plainly.
- **Movie themes are opt-in,** split into a voice (wording) and a look (icons). Voices are short text files: Ghostbusters, Back to the Future, Spinal Tap, Hitchhiker's Guide, Spaceballs, Star Trek, and Jaws. Looks need an icon set per state, drawn to read at 16 pixels in light and dark, so they come one at a time.
- **A joke never hides a number.** "Don't cross the streams!" still comes with "5-hour limit at 91%, resets in 1h 20m".

## Features

Version 1, the engine and terminal view, working correctly on macOS and Omarchy Linux:

- Every limit Anthropic reports, with pace, source, and age.
- Today's cost and pace against your own usual.
- Active sessions, with the fastest-spending one highlighted.
- Durable history that outlives transcript cleanup.
- Breakdowns by repo, branch, skill, agent, and model.
- Multiple accounts on one machine, tracked separately and shown one at a time or stacked.
- Credit-based plans, using `spend`.
- Anthropic's usage advice, saved daily.
- `gigawatts data` for scripts and agents, with a versioned JSON contract.

Version 1.1:

- A forecast of when you'll hit a limit compared with when it resets, built on the pace and limit readings version 1 records.
- The HTML report page.
- The insights skill.

Version 2:

- The macOS menu bar view.
- Notifications at limit thresholds, when a window resets, and when a session spends far above your normal rate.
- Themes: the default theme and the opt-in movie voices.

Later:

- Packaging for other people: a Homebrew tap, prebuilt binaries, and install docs.
- The Omarchy bar widget.
- Combined totals across your machines, by syncing daily rollups through a shared folder.
- The Windows view.
- Movie theme looks, one at a time.
- A one-time import of earlier `/spend` history (`~/.claude/spend/`), so months whose transcripts Claude Code already deleted still appear.
- The status line integration, for limits that are current to the second, if a use case appears.

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

- **Version 1.** Clone the repo and run `cargo install --path .`, with Rust installed through mise. Updating is `git pull` and the same command, and the view restarts an older engine on its own. `gigawatts service install` sets up the login service.
- **Later, for other people.** A Homebrew tap built with [`cargo-dist`](https://github.com/axodotdev/cargo-dist), keeping the same service commands.
- **macOS view.** A signed and notarized cask. That needs an Apple Developer ID ($99 a year), and Homebrew 5 disables casks that fail Gatekeeper. `cargo-dist` doesn't notarize, so CI runs `notarytool` itself.
- **Command.** The binary is `gigawatts`, so it can't collide with other tools. Users can add their own short alias such as `gw`.
- **Name.** Gigawatts uses no Anthropic or Claude Code names or logos in its product name, per the legal page. "A usage monitor for Claude Code" in a description is allowed.

## Risks

- **Undocumented formats.** Transcripts, `cachedUsageUtilization`, and `get_usage` can change in any Claude Code release. Each source sits behind its own reader with fixture tests, and `gigawatts doctor` reports what stopped making sense.
- **`get_usage` rate limits.** It likely hits the same rate-limited endpoint other tools fight. Refreshes stay infrequent, back off on failure, and fall back to the cached copy.
- **The pricing page changes shape.** If `pricing.md` stops parsing, Gigawatts keeps the last good table and says so, and the check against Claude Code's totals catches wrong prices.
- **Swift adds a second language.** The view is kept thin so the Swift code stays small.

## Open questions

1. **Whether per-request costs match Claude Code's totals.** An early build step compares them with the long-prompt, US-only, and fast-mode rules applied. If mismatches stay common, the "adjusted" label needs rethinking.
2. **Whether Anthropic's docs terms allow automated downloads of `pricing.md`.** To check before building the updater. If not, the built-in table and overrides are the only sources.
3. **The work Mac's headline numbers.** Settled 2026-11-02: the enterprise account reports monthly credits only (`spend` and `extra_usage`), with no 5-hour or weekly windows and no reset time.
4. **Repo hosting and license.** Personal account or employer org. An employer IP policy may decide this. Blocks only a release to other people.
5. **Whether Anthropic is comfortable with scheduled `get_usage` calls.** It runs Claude Code unmodified with the user's own login, which the policy page allows, but nothing public addresses running it on a schedule. Blocks only a release to other people.
6. **The Windows view technology.** Decided when Windows work starts.
