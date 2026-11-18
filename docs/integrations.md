# Building on Gigawatts

This is for anything that shows Gigawatts' numbers somewhere else: a Herdr pane, a status bar, a script, or an agent asked to build one. Point the agent at this file.

## What to run

| Command | Gives you | Use it for |
|---|---|---|
| `gigawatts status --json` | Everything happening now: each account's limits, today's spend, running sessions, problems, and alerts. Built fresh on every call, and it starts the engine if it isn't running. | Anything that shows the current state. |
| `gigawatts data --json` | Spend history, grouped with `--by repo`, `branch`, `skill`, `agent`, or `model`, over `--range today`, `week`, `month`, or `all`, or `--since` and `--until` dates. `--drill <key>` narrows to one row's breakdown. | Reports and "where did the money go" questions. |
| `gigawatts data --limits --json` | Each account's latest limits, after the engine refreshes any over 10 minutes old when the engine is running; otherwise the saved readings. | Limits alone, without the rest of the snapshot. |
| `gigawatts insights --json` | What each skill and subagent costs per run, week by week over the last 8 Monday weeks (or `--since` and `--until`), with each shift of 30% or more in cost per run and the model, effort, or Claude Code version change that lines up with it. | Finding which skills and subagents to make cheaper, and whether a change worked. |
| `gigawatts insights --help` | After the usual options, Markdown steps for an agent: read `insights --json`, find the skill and agent files behind the costliest rows and the shifts, and propose edits. The text ships in the binary, so it matches the installed version's output. The `insights` table's last line points agents here. | Handing the whole job to an agent instead of writing your own prompt. |
| `gigawatts schema` | A JSON Schema for each of these, with a description of every field. | Learning what each field means. |

Read only these commands. The history database and the report page's `/api/*` routes are private and change between releases. The engine's `~/.local/share/gigawatts/snapshot.json` is the same shape as `status --json`, but it's only rewritten when something changes, so while Claude Code is idle it can be hours old.

## Reading what the cost paid for

Each `data --json` row's `paid_for` splits its tokens' cost into input, output, cache writes, and cache reads, in dollars at list price. Use it instead of token counts: cache reads are the largest count in almost every session but cost the least per token. Cache writes are context sent fresh, as after a cold start or a resume past the cache's lifetime; cache reads are context re-read on every turn. `paid_for` covers every day in the range. It is null when no priced tokens cover a row, as on a model with no price, and its sum can differ from `cost_usd`, which also covers surcharges such as fast mode.

## Reading insights

`skills` and `agents` overlap. A subagent's requests made inside a skill count toward both the skill's row and the subagent's, so never add the two lists together. A skill row's `subagent_cost_usd` is the part of its cost its subagents spent. A skill run is one unbroken stretch of the skill in a session's main conversation; a subagent run is one subagent, counted in the week it started. A week's `cost_usd` includes runs that started the week before, so a week with spend and no runs has a null `cost_per_run_usd`.

Each skill row's `facts` covers the whole range rather than each week: `runs_per_session`, `repeat_run_cost_usd` (what runs after the first in their session cost, subagents included), `top_tenth_session_share` (the share of the row's cost in its costliest tenth of sessions), and `launches`, one entry per subagent type with its launches, `per_run`, `cost_usd`, and `first_request_cost_usd`, what each launch's first request cost added up, which is mostly its starting context written to the cache. A request belongs to the latest run of its skill in its session that started at or before it, so a subagent's requests count in the run that launched it. A subagent row has `facts` null and carries its own `first_request_cost_usd` across all its launches; divide by `runs` for one launch. Subagents launched outside any skill appear only in `agents`.

A `shift` needs at least 10 runs in each two-week span, so rarely used skills never show one. Its `causes` lists each setting whose largest share of spend changed across it, with `in_subagents` true when the change was in the subagents a skill launched rather than its own conversation; an empty list means no model, effort, or version change lines up with it, and the cause is elsewhere, such as an edit to the skill. Claude Code updates every few days, so a `version` cause on its own is weak evidence; look for an edit then too. Two weeks with any request on a model that has no price are never compared, since that spend counts as $0. Spend Claude Code never wrote to a transcript has no skill or subagent, so it's left out. `data_starts` is later than `since` when this machine no longer keeps request detail back that far. A run or launch that started before `since` is left out. When retention moved `data_starts` past `since`, a run or launch whose earlier requests were deleted counts as starting at its first kept request, so the first weeks can show a few more runs and launches than happened.

## Checking the version

Every response has `schema_version`, currently `2`. New fields don't change it; a removed or renamed field, or one whose meaning changes, raises it. Check it and ask the user to update your integration when it's a number you don't know. Ignore fields you don't know. New values of an enum, such as a problem's `kind`, can appear without a change too, so treat a `kind` you don't know as a generic problem.

## How often to call

Calling `gigawatts status --json` once a minute is fine. Each call only reads the local database; it never makes Gigawatts ask Anthropic for new limits. The engine refreshes limits itself every 10 minutes while you're using Claude Code, and leaves them alone while you're idle, so a pane left open overnight costs nothing. Nothing changes faster than once a minute except running sessions' spend.

## Showing alerts

`alerts` lists limits that need attention, already ranked worst first, and is empty when nothing does. Show the first one and count the rest:

- `state: "full"`: the limit is at 100%. `at_ms` is when it resets. The terminal view says "Session back in 2h 40m".
- `state: "filling"`: the limit is on course to reach 100% before it resets, within the next 24 hours. A fill further out shows only as the limit's `forecast`. `at_ms` is when it's forecast to, or the snapshot's `written_at_ms` when that time has already passed, meaning any minute now. The terminal view says "Session full in ~1h 20m", rounded to 10 minutes because it's a forecast.

Count down from your own clock to `at_ms`, so the countdown stays right between calls. Once `at_ms` passes before your next call, hide a `full` alert, since its limit has reset, and show a `filling` one as any minute now. Name the account (`account_label`) when the alert isn't for the account you're showing.

## Showing what was used elsewhere

On an account whose only limit is credits, that limit's `elsewhere_usd` is what other machines and claude.ai used this credit period: the credits Anthropic counts minus this machine's spend. It's null when the difference is too small to matter, when Gigawatts can't work out this machine's share (then `this_machine_usd` is null too), and on accounts with session or weekly limits. The terminal view puts it after the cap, as "$494.00 of $2,200 ($84 elsewhere)".

On the `session` and `weekly_all` limits, `elsewhere_percent` estimates how many points of `percent` other machines and claude.ai used this window. Anthropic reports these limits only as a percent, so Gigawatts converts this machine's spend to points at a rate it learns from this machine's history, and the estimate can be off by 2 to 3 points. It's null when the limit is under 10%, when the estimate is under 3 points, when there's no history to learn the rate from yet, and while some of this machine's spend in the window has no price. Show it as an estimate, the way the terminal view does with "(~13%)" beside the percent.

## Freshness and gaps

- `today` is the local date the snapshot counts as today. `status --json` always builds it for the current date.
- Each account's `limits.fetched_at_ms` is when Anthropic reported the reading. Over an hour old, it may not include use outside Claude Code, such as claude.ai chats; say how old it is rather than hide it. Filling alerts already leave such readings out.
- `problems` lists anything that makes a number wrong, stale, or missing, each with `affects`, `cause`, and `fix` in plain words. Show a count, and the details on request.
- A cost can be estimated until Claude Code records its own figure when the session ends. `cost_estimated` says when today's total includes such spend.

## When the command fails

`gigawatts` not found means it isn't installed; point to the README. Any failure exits with code 2 and explains on stderr, for example when the running engine is newer than the `gigawatts` on the `PATH`.

## Example

```sh
gigawatts status --json | jq -r '
  .alerts[0] as $a
  | if $a == null then "no alerts"
    else "\($a.account_label) · \($a.limit) \($a.state) at \($a.at_ms / 1000 | todate)" end'
```
