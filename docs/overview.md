# Gigawatts project overview

Gigawatts is a local usage monitor for Claude Code. It shows how much of your usage limits you've used, what your work cost, and where it went, from a terminal window or a page in your browser. It spends no Claude tokens, needs no changes to Claude Code's settings, and never handles your login.

This is the spec: how each number is computed and how the engine behaves. The README covers installing and using it.

## Goals

- **Glance.** Remaining limits, today's cost and pace, which sessions are running, and whether any is spending fast ($20/hr or more unless set otherwise), readable in a second.
- **History.** A permanent local record that survives Claude Code deleting old transcripts (after 30 days by default, `cleanupPeriodDays`).
- **Reports.** Cost by repo, branch, skill, agent, and model, over days, weeks, and months.
- **Light.** No wakeups while Claude Code is idle, and a memory footprint in the tens of MB.
- **Trustworthy for anyone.** Anyone can install it without changing their setup, handing over credentials, or sending data anywhere.

## Non-goals

- **Monitoring other people.** Gigawatts shows one person their own usage. Team dashboards are what Anthropic's admin console is for.
- **Exact billing.** It reports the figures Claude Code records and labels anything it estimates except today's running total (see principle 3). Anthropic's bill is the authority.
- **A server.** Nothing runs outside the user's own machines.
- **Calling a model.** Gigawatts never sends anything to Claude. Judgment about the numbers comes from asking an agent to read `gigawatts data --json`, which happens only when you ask.

## Principles

1. **Read-only toward Claude Code.** Gigawatts reads Claude Code's files and never writes to them or to its settings. Optional integrations are installed by an explicit command, add to the user's setup without replacing anything, and come out with one command.
2. **Never touch credentials.** Anthropic's [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) says "developers may not collect, store, or intermediate Claude.ai credentials or session tokens." Gigawatts never reads the OAuth token. When it needs fresh limits, it asks the unmodified `claude` binary, which uses its own login.
3. **Every number shows its source and age.** For example: "32% weekly, from Claude Code, 4 min ago." Today's dollars are the exception: Gigawatts prices them itself until each session closes and Claude Code records its own cost, and they rise when Claude Code's unlogged usage arrives. That's true nearly all day, so the views don't label it.
4. **Event-driven.** It reacts to file changes and sets no polling timers while Claude Code is idle.
5. **Degrade instead of breaking.** Every source is undocumented or can change. When one changes, Gigawatts marks the numbers it affects and keeps showing the rest.
6. **Numbers stay local.** The database holds counts, costs, models, repos, branches, times, and session titles. It never stores prompts, responses, tool inputs, file contents, or commands. Gigawatts never sends your data anywhere. The one network request, a download of Anthropic's public pricing page when a price looks out of date, can be turned off with `price_updates = false`.

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
| Transcripts, `~/.claude/projects/**/*.jsonl` | Per request: tokens by type (input, output, thinking, cache reads, 5-minute and 1-hour cache writes, web searches), model and requested model, effort, speed, `inference_geo`, `attributionSkill`, `attributionAgent`, working directory, git branch, time, and `requestId`. Per session: `cost-state` records with the cost per model (written when the session ends), durations, lines changed, the session title (the user's rename in `custom-title`, which wins, or else the generated `ai-title`), and tools used | Partly. [Location and retention](https://code.claude.com/docs/en/claude-directory) are documented, the line format isn't | Primary source for cost, tokens, and history |
| `.claude.json`, `cachedUsageUtilization` | A generic `limits` list (kind, scope, percent, severity, reset time), credits in `spend` (older `extra_usage` as a fallback), a weekly breakdown by surface, the `accountUuid` (but not the `organizationUuid`, so readings are assigned to an account through its directory's `oauthAccount`), and when it was fetched (`fetchedAtMs`). Claude Code doesn't refresh it during ordinary use | No | Read at each full scan. A newer `get_usage` reading takes precedence. On its own it can be days old |
| `get_usage` control request through an unmodified `claude -p` | Fresh limits and credits, refreshing `cachedUsageUtilization`. Without `skip_behaviors`, also the `behaviors` section that `/usage` shows as "What's contributed to your usage": behavior shares, top agents and skills, and recommendations. Costs no tokens | No (the SDK names it `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`) | Limits refresh every 10 minutes per active account, a minute after its 5-hour or weekly limit resets, and when you look (see [Stale data and errors](#stale-data-and-errors)). `behaviors` fetched at most once an hour per active account. Measured on Claude Code 2.1.314: 1.1–1.9 s and 258–305 MB peak per run, released when the process exits |
| Anthropic's pricing page as Markdown, `https://platform.claude.com/docs/en/about-claude/pricing.md` | Per-token prices by model and token type, plus pricing rules (long prompts on Haiku 5.5, US-only inference at 1.1×, fast mode) | Yes, it's Anthropic's public docs | Prices copied by hand into the built-in table, and downloaded when a price looks out of date. See [Cost](#cost) |
| Status line JSON | Live `rate_limits` on every Claude Code event | [Yes](https://code.claude.com/docs/en/statusline) | Not used. Limits refresh every 10 minutes while you work, and the forecast's 30-minute rate catches bursts |

Sources considered and rejected:

- **Calling `api.anthropic.com/api/oauth/usage` directly.** It requires reading the user's OAuth token, which the credential policy above rules out. Every tool that does it also fights repeated Keychain prompts and 429 rate-limit errors.
- **OpenTelemetry export.** It needs environment variables in Claude Code's settings, and company managed settings can override them.
- **Admin and Analytics APIs.** They need an organization admin key and don't serve individual subscribers.
- **The credit formula in `cachedGrowthBookFeatures`.** It's a server-pushed feature flag. A local test landed near the real figure (27% estimated against 25% reported) but proves nothing from one sample.
- **LiteLLM's price list.** A third party's copy of Anthropic's pricing page, when the page itself is available as Markdown.
- **Working prices out from Claude Code's recorded costs.** A best fit over 216 per-model session totals missed most sessions by more than 0.5% and produced negative prices, because the pricing rules above vary by request.

## Accounts

An account is one Claude Code config directory (`~/.claude`, or wherever `CLAUDE_CONFIG_DIR` points), identified by the account ID inside its `.claude.json`, not by the folder's name. An account ID is the `accountUuid` and `organizationUuid` pair, because Claude Code keys a login by both and one person can have the same account in two organizations. Everything is stored per account from day one, and history never merges accounts. Today's spend in the all-accounts views (the terminal view's stacked view) is the one place accounts are summed.

- **Finding accounts.** Gigawatts always checks `~/.claude`. At engine start it scans one level into the home folder for any directory shaped like a config directory (a `projects/` folder and a `.claude.json`), whatever it's named. Extra paths go in the config. `gigawatts accounts` shows what was found and whether each is tracked.
- **Tracking versus displaying.** Tracking means the engine reads an account's transcripts and limits into history. It runs whether or not anything is on screen, because a gap can't be filled after Claude Code deletes the transcripts. Tracking is on by default for every account found, a newly found account is announced in the view (except on the view's first launch, which only records the accounts it finds), and it can be turned off per account. Off means Gigawatts never reads that directory. Turning it off keeps existing history, and `gigawatts accounts forget` deletes it. Forgetting keeps the record of how far each transcript file was read, so a later login change can't read the deleted history back in under another account.
- **Limits belong to the account.** If two directories are ever logged into the same account, they share one set of limits and keep separate cost history.
- **A changed login.** If the account ID inside a directory changes, Gigawatts says so instead of mixing the two accounts' history.
- **A switch Gigawatts didn't see.** Transcripts don't record which account made a request, so a directory that changed accounts before Gigawatts first ran (such as moving from a company plan to a personal one in the same `~/.claude`) would otherwise put all its history under the newer account. Each config directory can carry a switch setting: a time and the earlier account ID. Requests before that time, and `cachedUsageUtilization` readings whose `fetchedAtMs` is before it, belong to the earlier account. Days whose request detail has already expired keep the account they were stored under, so set the switch before the retention window passes them.

## Architecture

```
Claude Code's files ──▶ engine ──▶ SQLite history ──▶ snapshot + gigawatts data ──▶ views
  transcripts            readers     (private)          (view contract)               terminal
  .claude.json           cost model                                                   report page
  claude -p get_usage    rollups                                                      integrations
```

- **Engine (Rust).** Always its own background process, and the only writer to the database while it runs. Commands that rewrite history, like `forget`, stop it, write, and start it again, and `gigawatts ingest` writes directly only when no engine runs. A lock file stops a second engine from starting. Opening the terminal view starts the engine if it isn't running, and `gigawatts service install` adds a systemd user unit on Linux or a launchd agent on macOS so it also starts at login (`gigawatts service uninstall` removes it). Once started, it runs until the machine restarts or `gigawatts stop`. Closing the view never stops it. While Claude Code is idle it sleeps with zero wakeups.
- **Upgrades.** The engine writes its version into the snapshot. A newer view that finds an older engine stops it and starts its own, which runs any database migrations. If the engine is newer than the view, the view says to reopen it. Restarting is always safe, because read positions are saved in the same transaction as the data.
- **View contract.** The database is private to the engine. Every consumer reads only two things: the snapshot, with every "now" number per tracked account and the alerts worst first, and `gigawatts data --json`, which answers history queries by range and grouping. `gigawatts status --json` builds the snapshot fresh on every call and starts the engine if it isn't running; the engine also rewrites it to a file after each batch of changes. Every field has a description in `gigawatts schema`, and `docs/integrations.md` explains how to build on both. The terminal view calls the same Rust code in-process, and integrations such as a Herdr pane run the command. So every number has one implementation, and the database layout can change freely between releases. The report page is the one exception: the engine serves it `/api/*` routes that query the database through the same Rust code. Those routes are private to the page, aren't covered by `schema_version` or `gigawatts schema`, and can change in any release, so no other view, herdr mod included, reads them. The engine rewrites the snapshot only after a batch or scan, so its `today` is the local date it was written. A reader on a later date treats every account as having no requests or spend yet today.
- **Schema versioning.** Every snapshot and data response carries an integer `schema_version`. Adding a field doesn't change it, and consumers ignore fields they don't know. Removing or renaming a field, or changing what one means, raises it. `gigawatts schema` prints a JSON Schema generated from the Rust types with `schemars`.
- **Views.** Views hold no logic.

### Engine internals

| Area | Choice | Alternative considered |
|---|---|---|
| File watching | [`notify`](https://docs.rs/notify) 8.2, with the engine's scheduler batching events for a second. Events only trigger a re-check of that file. A full scan runs at startup, when the watcher reports dropped events, and every few minutes as a safety net while Claude Code is active. The safety-net scans stop 10 minutes after the last file event, so nothing wakes while Claude Code is idle | A stat loop over known files, which wakes the machine constantly. `notify-debouncer-full` 0.7, whose thread wakes every quarter of its timeout forever |
| Reading transcripts | Per file: byte offset, file identity, and a hash of the first line, so only complete new lines are read. The offset is saved in the same transaction as the parsed data. Requests are stored by `requestId`, and a repeated one keeps the line with the most output tokens, because Claude Code writes a streamed request over several lines and sometimes sets a transcript aside and writes a new one for the same session. Files named `*.superseded-*` or `*.orphaned-*` are skipped | `linemux`, last released 2022 |
| Parsing | `serde_json` into narrow borrowed structs, skipping lines without `"usage"`, `"cost-state"`, `"ai-title"`, or `"custom-title"` before parsing | `sonic-rs` or `simd-json`, only if the first full read proves slow |
| Storage | `rusqlite` 0.41 (bundled SQLite, the newest `rusqlite_migration` supports), WAL mode, migrations with `rusqlite_migration`, retention below | `sqlx` needs an async runtime. `redb` has no SQL for rollups |
| Concurrency | Plain threads and channels, all blocking while idle | `tokio`, unnecessary without async networking |
| Tests | `insta` snapshot tests over trimmed real transcripts, and `proptest` checks that the same bytes split into arbitrary appends produce the same totals. Lines of an unknown type whose message carries usage are counted, so format drift shows up instead of reading as zero | Hand-written unit tests only |

### What gets recorded

Every numeric and categorical field Claude Code provides per request and per session is recorded: all token splits, cost, model and requested model, effort, speed, `inference_geo`, skill, agent, repo, branch, durations, lines changed, and tool names. Session titles, whether generated or a name you gave the session, are the one free text stored, and can be turned off per account. Nothing else you or the model wrote is stored. A field not recorded within 30 days can never be added to older history, which is why the rule is wide.

Anthropic's `behaviors` section from `get_usage` is fetched at most once an hour per active account, and each day keeps its newest one, since it covers only today and this week and isn't kept anywhere else.

### Storage and retention

Rows are written only when something happens, never on a timer, except the session limit reading, kept on every fetch.

| Table | One row per | Kept |
|---|---|---|
| Requests | API request, keyed by `requestId` | Forever by default. A setting limits it to a number of days |
| Daily totals | Day × account × repo × branch × skill × agent × model, updated in place | Forever |
| Sessions | Session, updated in place | Forever |
| Unlogged usage | Session × model, with the session's main folder and branch, rewritten on each `cost-state` while the model has request detail | Forever |
| Directories | Working directory, resolved once | Forever |
| Limit readings | Every fetch for the session limit; a change in percentage for other limits, per account | Forever |
| Usage advice | Day × account, from `behaviors` | Forever |
| File positions | Transcript file, updated in place | While the file exists |

Request costs are provisional until Claude Code writes the session's `cost-state` when the session ends. Then the session's requests are checked, any spend not in the transcript is stored as its own row, and the daily totals are recomputed. If request detail is limited by the setting, a daily cleanup deletes expired rows and runs SQLite's incremental vacuum so the file actually shrinks, and reports fall back to the daily totals, with a session report showing all expired days as one "older sessions, detail not kept" row. Transcripts first found after cleanup, such as a directory added later, don't add days before the cutoff. A switch setting added later can't move those days to the earlier account, because daily totals don't record the time of each request.

Measured once, early on, and not rerun since:

| Measure | Result |
|---|---|
| One real history | About 350 requests per active day (peak 868), about 6 MB a year at the original row size |
| Heavy user (5,000 requests a day) | About 30 MB once the 90-day window fills, then about 5 MB a year |
| Row size | About 67 bytes per request row, before the wider field set |
| Today's total, 1 million rows | 0.4 ms |
| A month grouped by skill and model | 18 ms |

Keeping every request with the wider field set, plus its tool names, measured 92 MB for a synthetic heavy year with the retention setting at 90 days, about 200 bytes a request including indexes. The index on requests by session and the branch column in daily totals, both needed for cost checks and branch history, brought it to 106.3 MB, 248 bytes per stored request. That works out to about 30 MB a year for that real history. Size is only a disk cost: SQLite's page cache stays at its fixed size, and reports read daily totals rather than requests. A test in the repo generates that heavy year and fails the build if the database exceeds 110 MB, today's query takes over 5 ms, dollar pace, active sessions, sessions today, and today's cost flags take over 40 ms, or a yearly report takes over 250 ms. Those time limits are 4 times looser in CI, whose runners are slower. `docs/performance.md` has the measured times.

## Cost

Claude Code records a dollar cost per session and model, and only when the session ends. It never records a cost per request, and while a session runs it has recorded no cost at all. So Gigawatts prices each request itself:

1. Each request is priced from its tokens by type and the price table, applying the pricing rules its line records (long prompts on Haiku 5.5, `inference_geo` of `us`, fast mode). Until the session ends, that cost is labeled "estimated."
2. When the `cost-state` arrives, the requests of each model are added up and compared with Claude Code's figure. If they match, the costs are exact and carry no label.
3. If Claude Code counted more tokens than the transcript holds, usually because it billed requests it never wrote there, the request costs stay exact, and the difference is stored as "unlogged usage" for that session and model, with the folder and branch most of the session's requests ran in. Repo and branch breakdowns count it under that repo and branch, since a session's place is known. Skill and subagent breakdowns show it as its own row, since guessing which skill or subagent spent it would be wrong. Every breakdown still adds up to Claude Code's total. These requests come from Claude Code itself, not the desktop app or claude.ai, which never write to its transcripts; which requests they are is unknown, and the Haiku share suggests background calls. In the history `gigawatts check-costs` was first run on, it was about 14% of spend.
4. If the tokens match and the cost doesn't, a price or a pricing rule is out of date. The requests are scaled to Claude Code's total, the session is labeled "adjusted," the gap is recorded, and `gigawatts doctor` names the model.

On a subscription, these dollars are what the usage would cost at API prices, not what you're charged.

The price table comes from three layers. A built-in table is copied by hand from Anthropic's pricing page, with the date and link. Prices downloaded from that page win over it, and overrides in `config.toml` win over both. `gigawatts doctor` names any model without a price, and its last line names where prices come from and when they were downloaded. Until a model has a price, the cost Claude Code recorded for it counts as unlogged usage, so totals still match. When any layer changes, stored history is repriced on the next ingest. The pricing rules are prose on the page and change rarely, so they live in code.

The engine downloads `pricing.md` when it sees a model with no price or a price gap, at most once a day while either lasts, and only with `curl`. It reads the model pricing table, keeps the prices that differ from the built-in table, and reprices history. A failed download, a page whose table doesn't parse, or a price more than 5 times or less than a fifth of the current one (or, for a new model, more than 5 times the most expensive) keeps the current prices and raises a problem. `price_updates = false` turns downloads off.

## Pace

- **Limit pace.** Each limit compares how much is used with how much of its window has passed. Used less than has passed means under pace: at your average rate so far, you won't run out before the reset. The 5-hour limit always compares against even pace. Weekly limits start with even pace and switch to your own usual progress at this point in the week once four full weeks of readings exist. A credit limit with no reset time, like an enterprise account's monthly `spend`, is paced over the month that starts at the account's credit reset, which defaults to midnight UTC on the 1st.
- **Dollar pace.** Today's spend so far is compared with your usual spend by this clock time, meaning the median of your last 20 days of the same kind (workdays or weekend days) that had any usage. It flags an unusual day and says nothing about limits. The terminal view's Today panel adds what the day is on pace for: today's spend so far plus what a usual day still spends after this clock time, never less than today's spend so far.
- **Sessions today.** Every session with requests today, counting only what it spent today, so together they add up to the day's total. A session that wrote to its transcript in the last 15 minutes is running; one quiet longer is finished. One that spent at the rate of `fast_usd_per_hour` ($20/hr unless set) or more over those 15 minutes is flagged with that rate and comes first, then the other running sessions, then the finished ones, each group ordered by what it spent today. When more run fast than there are cards, the line adding up the rest counts them with a ▲. Subagent requests count toward the session that launched them.
- **Forecast.** When a limit is on course to fill before it resets, the forecast says when. It takes the earlier of two projections: a straight line through the current window, and, for the 5-hour limit only, the last 30 minutes' rate. The straight-line projection starts once a tenth of the window has passed, and waits until use runs more than 5 points ahead of the share of the window gone, the same band pace calls even, unless the limit is within 5 points of full, since Anthropic reports whole percents and a limit level with the clock would flicker in and out of a forecast.
- **Alert light.** A light red badge at the right end of the terminal view's title row when any account's limit is forecast to fill before it resets and within the next 24 hours, or is already full. A fill more than 24 hours out shows only under its limit.
- **Spend used elsewhere.** For a credits-only account, Anthropic's month-to-date credits minus this machine's spend, as "$494 of $2,200 ($84 elsewhere)" with a second shade in the bar, left out under $5 or under 1% of the credits used. For a subscription, an estimate on the session and weekly limits, as "(~13%)" after the percent in the same dim shade as its part of the bar, which help explains: the limit's percent minus this machine's spend in the window, converted at this machine's dollars per point (its highest daily rate over the last 28 days among days whose rises within a window add up to 5 points or more). Shown only once the limit is at 10% and the estimate at 3 points, since it can be off by 2 to 3 points.

## Attribution

- **Repo.** When Gigawatts first sees a working directory, it asks git which repository the directory belongs to, resolving a worktree to its main checkout, and stores the repo's name, path, and remote URL with the request. It has to happen then, because a deleted worktree can't be resolved later. A deleted folder named `<repo>-<anything>`, or a folder inside one, that sits beside an existing main checkout `<repo>` counts as that repo, since that's how worktrees are named. Other directories outside git count as themselves, and directories under `/tmp` are grouped as "temporary."
- **Branch.** From the line's `gitBranch`. When that is `HEAD`, as during a rebase, Gigawatts asks git for the branch being worked on, and shows "detached" if git can't say.
- **Skill and agent.** Straight from `attributionSkill` and `attributionAgent`. Lines with neither belong to the main conversation with no skill.
- **No tickets.** Cost breaks down by repo and branch. Grouping branches into a larger piece of work is left to asking Claude, which needs no tracker or naming scheme.

## Terminal view and CLI

The full view is one screen. The top always shows the current account's limits with pace and age, today's spend against usual, and every session today under a heading with how many are running and the day's total, each with its cost today, repo and branch, share of today's spend, and the skill or subagent that spent most of it. Sessions spending fast come first, flagged, or counted with a ▲ when the cards are full; then the other running sessions, then the finished ones, each group by cost today. A running session quiet for 5 minutes says `idle 9m`; a finished one is dimmed and says when it was last active, such as `last active 40m ago` or `last active 2h ago`, and gets a card only in a slot left over in the rows the running sessions use, or in one row when nothing is running. With no spend today the panel says `No sessions today`. Yesterday's sessions leave at midnight. Sessions that don't fit add up as "+ N more" in the panel's bottom border, so the count costs no row of cards, and temporary-folder sessions that aren't spending fast add up there as "N background runs"; with no cards to show, that line sits inside the panel. From about 180 columns, the sessions sit beside the spending summary instead of in the top panel. Where the cards sit beside the spending summary, blank rows give way for a second row of cards one at a time, and only when that wins the row: first the row at the top of the sessions, then, in the stacked view's columns, the rows at the top of the limits and today boxes. `s` lists every session today in the same order in the spending table's place, and `↑` and `↓` scroll it. Below it, a spending summary for one grouping at a time: `←` and `→` step through repo, skill, subagent, and model, and `t` through today, week, month, and all, both shown as tabs above the table. `↑` and `↓` move a highlight through the rows, and the table scrolls to keep it on screen. The repo tab lists each repo with its total and a `▸`; `Enter` on a repo shows its top three branches under it, then one line with the rest and the largest of them, `Enter` again hides them, and `e` shows or hides every repo's branches at once, since a branch is the closest thing to one piece of work and the same branch name in two repos is two pieces. Subagent rows are the agents Claude launched, with "you (no subagent)" for your own conversation; models are separate, since a subagent runs on a model. Each row has its cost, its share of the range's total, and the skill (or, on the skill tab, the repo) that spent most in it, such as "53% review" beside a branch. Spend with no skill shows as "no skill". It's for seeing where the money goes at a glance; `gigawatts data` has the full breakdown down to sessions. In a small window, such as Omarchy's floating terminal, the summary collapses behind a key. Anthropic's usage advice sits behind its own key.

It shows one account at a time, starting with the one used most recently, with a row of account tabs under the title and a blank row under the tabs at every size. Under 90 columns today's spend stacks above the limits; from 90 columns it sits to their left in large digits, and where the cards sit beside the spending summary, the spending summary sits under today's spend and the cards under the limits. A short window shows one row of session cards, with the rest counted in its border, and keeps the spending summary behind `Tab`. Below the width where the cards sit beside the spending summary, the layout always keeps room for one row of session cards, whether or not any session is running, so a session starting or ending never moves what sits above the cards. Spare rows go, in order, to the spending summary, a blank row at the top of each pane and between limits, a blank row above the cards, the chart of recent days under today's spend (from 90 columns), and a second row of cards, stopping at the first that doesn't fit, so below that width a taller window never takes away anything a shorter one showed, and the spending summary never drops below its 8 rows. When today's spend runs taller than the limits beside it, the limits get a blank row between each pair if the spare rows cover every gap. Number keys pick an account, `[` and `]` step through the tabs, and the number after the last account opens a stacked view with every account's limits and today's spend, with today's spend across all accounts at the right end of the tab row when it fits. With 2 accounts from 100 columns, or 3 from 150, it puts each account in a column, today's spend above its limits, when the columns fit with the most the boxes below would show: the spending summary, a blank row above the cards, and room for two rows of cards whether or not that many sessions are running; in the columns each limit's details always sit under its bar. Otherwise each account gets one box, titled with where its limits came from. From 90 columns today's spend sits beside the bars, in a side as wide as its longest note about usual spend or pace (34 to 44 columns, the same for every box), in smaller, three-row digits below a blank row with its notes under them, at every height, and the session cards and spending summary give way first when the window is short; below 90 columns it sits above the bars as text. In those boxes spare rows go, in order and stopping at the first that doesn't fit, to the spending summary, a blank row above the cards, and a second row of cards, and each limit's details stay on its name line when they fit. While `s` lists the sessions the boxes take none of these. The boxes sit one under another with no blank row between them. With 9 or more accounts no number is left, so `a` toggles that view instead. In a small window `Tab` switches between the limits and the spending summary. The footer ends with the report page's address when it has room, and help lists it, while the engine is serving the page. `?` opens help, and a key strip in the footer lists every key that works on the current screen. Each limit is a full-width bar colored by pace, with a mark at the expected percentage. The line above it holds the limit's name, its details, such as the countdown to the reset, and its percentage; details that don't fit there go under the bar, and a credit limit's amounts, such as "$437.60 of $2,200 · $82 / day", always do. Once the reset time passes, the bar empties, turns gray, and reads "reset", with when it reset in its details, until the next reading. Pace shows in the bar's color alone, with no words. When a limit is on course to fill before it resets, a countdown to 100% leads its details, in the over-pace color.

| Command | Does |
|---|---|
| `gigawatts` | Opens the full view, starting the engine if needed |
| `gigawatts status` | Prints one line: the closest limit with its pace, "100% by <time>" for the fullest limit and for whichever other limit fills first, when either is on course to fill before it resets, and today's spend against usual. `--account` picks an account and `--all` prints one line per account. `--json` prints the whole snapshot instead, built fresh, for integrations |
| `gigawatts data` | Answers a history query by range (or `--since` and `--until` dates) and grouping. A readable table by default, versioned JSON with `--json` |
| `gigawatts insights` | Prints each skill's and subagent's cost per run over the last 8 Monday weeks (or `--since` and `--until` dates), with each shift and the setting change that lines up with it. A readable table by default, versioned JSON with `--json`. `--help` includes the steps an agent follows to propose edits |
| `gigawatts schema` | Prints the JSON Schema for the snapshot, `data --json`, `data --limits --json`, and `insights --json` |
| `gigawatts accounts` | Lists accounts and their tracking state. `add <path>`, tracking on or off, and `forget <account>` to delete history |
| `gigawatts service install` / `uninstall` | Adds or removes the login service |
| `gigawatts stop` | Stops the engine |
| `gigawatts doctor` | Lists every current problem with its cause and fix |
| `gigawatts report` | Opens the report page the engine serves at `http://localhost:7878` in the default browser, starting the engine if needed. `report_port` in `config.toml` changes the port, and `0` turns the page off. The page assumes a single-user machine: any account on the computer can reach the port, so on a shared one set `report_port = 0` |

## Stale data and errors

- **Stale limits.** Every limit shows its age. Opening the view refreshes any account whose reading is over 10 minutes old. `gigawatts status` prints the saved reading and asks the engine to refresh in the background, at most once per 10 minutes. A reading over an hour old is greyed out.
- **Problems.** Each affected number carries a short marker, like "limits: refresh failing since 2 PM" or "cost leaves out claude-opus-5-6 until its session ends: no price." The footer shows when anything is wrong, and `gigawatts doctor` lists each problem with its cause and what to do. Watched problems include unrecognized transcript lines, unknown models, price gaps ("adjusted" sessions, plus a price that comes out above Claude Code's record even with spend missing from the transcript), failing refreshes, a failing price download, folders the engine can't watch, transcripts or logins it can't read, and a daily cleanup of old request detail that fails.

## Settings

Settings live in `~/.config/gigawatts/config.toml` on both platforms, and data (database and snapshot) in `~/.local/share/gigawatts/`, readable only by you. Commands like `gigawatts accounts add` edit the TOML file with `toml_edit`, keeping comments and layout. Per-account settings are keyed by account ID, so one file kept in dotfiles works on every machine, and extra paths that don't exist on a machine are skipped.

| Setting | Default |
|---|---|
| Limit refresh interval while active | 10 minutes, or a minute after a 5-hour or weekly reset when that comes sooner |
| Days of request detail to keep | Unset, meaning forever |
| Extra account paths | None |
| Tracking, per account | On |
| Label, per account | The account's email |
| Store session titles, per account | On. Turning it off offers to delete stored titles |
| Credit reset, per account | Midnight UTC on the 1st |
| Price overrides, per model | None |
| Price downloads | On. `price_updates = false` turns them off |
| Spending fast | $20/hr over the last 15 minutes. `fast_usd_per_hour` changes it |
| Account switch, per config directory | None. A time and the earlier account ID; requests and `cachedUsageUtilization` readings before it belong to that account |

## Asking an agent

Gigawatts calculates, and any agent interprets: it reads `gigawatts data --json`, with `gigawatts schema` for what each field means, and answers questions like where last week's spend went. What's worth changing depends on each person's repos and habits, which the agent sees and Gigawatts doesn't. The README shows how to ask.

`gigawatts insights` computes what each skill and subagent costs per run, week by week over the last 8 Monday weeks. A skill run is one unbroken stretch of the skill in a session's main conversation, and its cost includes the subagents it ran; a subagent run is one subagent ID, counted in the week it started. A shift is a move of 30% or more in cost per run between the two weeks before a Monday and the two weeks from it, with at least 10 runs on each side; of shifts in the same direction on consecutive Mondays only the largest is kept. Each shift names the model, effort, or Claude Code version with the largest share of spend on each side when they differ. The command prints a table, `--json` prints the published `insights` response, and the unscoped report page shows the same numbers in an "Insights" card, with Suggestions and Changes tabs, left out of exports. `gigawatts insights --help` ends with instructions, compiled into the binary from `docs/insights-guide.md`, that have any agent read the response with the user's skill and agent files and propose edits. Shipping them in the binary keeps them matched to its output and means nothing is copied into Claude Code's files. Gigawatts never watches skill files or judges quality.

## Views

| View | Platform | Technology |
|---|---|---|
| Terminal: full view, one-line summary, and `gigawatts data` | All | Rust, [`ratatui`](https://ratatui.rs) 0.30, redrawn when data changes and once a minute while open |
| Floating terminal window on a key binding | Omarchy | The terminal view, no extra code |
| HTML report page | All | A page the engine serves on 127.0.0.1 only, current on every reload, with an Export button that saves a summary as Markdown or PDF |
| Integrations, such as a Herdr pane | All | Built outside this repo from `docs/integrations.md`, reading `gigawatts status --json` |

## Themes

- **The Back to the Future voice is built in.** There are no other voices and no theme setting. The terminal view's alert light reads "ϟ 1.21 gigawatts! Session full in ~1h 20m" when a limit is on course to hit 100% before it resets and within the next 24 hours, and "ϟ 1.21 gigawatts! Session back in 2h 40m" once it has.
- **A joke never hides a number.** "ϟ 1.21 gigawatts!" always comes with the limit and its countdown, as in "Session full in ~1h 20m".
- **The report page has a theme picker:** Auto, Light, and Dark, remembered in the browser. Auto follows the OS.

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

- **Install.** Clone the repo and run `mise run install`, which builds and installs the binary, stops the running engine, and starts the new one. Updating is `git pull` and the same command. `gigawatts service install` sets up the login service. A rebuilt binary keeps its old engine running unless the version changed, so every behavior change bumps the patch version.
- **Command.** The binary is `gigawatts`, so it can't collide with other tools. Users can add their own short alias such as `gw`.
- **Name.** Gigawatts uses no Anthropic or Claude Code names or logos in its product name, per the legal page. "A usage monitor for Claude Code" in a description is allowed.

## Risks

- **Undocumented formats.** Transcripts, `cachedUsageUtilization`, and `get_usage` can change in any Claude Code release. Each source sits behind its own reader with fixture tests, and `gigawatts doctor` reports what stopped making sense.
- **`get_usage` rate limits.** It likely hits the same rate-limited endpoint other tools fight. Refreshes stay infrequent, back off on failure, and fall back to the cached copy.
- **Scheduled `get_usage` calls.** Gigawatts runs Claude Code unmodified with the user's own login, which the policy page allows, but nothing public says whether Anthropic is comfortable with running it on a schedule.
- **Downloading `pricing.md`.** Anthropic's consumer terms ban scripted access to "Services" including "associated ... websites", while `robots.txt` and `llms.txt` invite readers. Gigawatts downloads the page only when it sees a price problem, at most once a day, and `price_updates = false` turns that off.
- **The built-in price table goes stale.** It is copied by hand, so a new model or price change leaves it behind. The engine downloads the pricing page when it sees either. `gigawatts doctor` names any model without a price, and the check against Claude Code's totals flags a wrong price as an "adjusted" session, or as a price gap when the price comes out above Claude Code's record even with spend missing from the transcript.
