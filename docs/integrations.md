# Building on Gigawatts

This is for anything that shows Gigawatts' numbers somewhere else: a Herdr pane, a status bar, a script, or an agent asked to build one. Point the agent at this file.

## What to run

| Command | Gives you | Use it for |
|---|---|---|
| `gigawatts status --json` | Everything happening now: each account's limits, today's spend, running sessions, problems, and alerts. Built fresh on every call, and it starts the engine if it isn't running. | Anything that shows the current state. |
| `gigawatts data --json` | Spend history, grouped with `--by repo`, `branch`, `skill`, `agent`, or `model`, over `--range today`, `week`, `month`, or `all`, or `--since` and `--until` dates. `--drill <key>` narrows to one row's breakdown. | Reports and "where did the money go" questions. |
| `gigawatts data --limits --json` | Each account's latest limits, after the engine refreshes any over 10 minutes old when the engine is running; otherwise the saved readings. | Limits alone, without the rest of the snapshot. |
| `gigawatts schema` | A JSON Schema for all three, with a description of every field. | Learning what each field means. |

Read only these commands. The history database and the report page's `/api/*` routes are private and change between releases. The engine's `~/.local/share/gigawatts/snapshot.json` is the same shape as `status --json`, but it's only rewritten when something changes, so while Claude Code is idle it can be hours old.

## Checking the version

Every response has `schema_version`, currently `2`. New fields don't change it; a removed or renamed field, or one whose meaning changes, raises it. Check it and ask the user to update your integration when it's a number you don't know. Ignore fields you don't know.

## How often to call

Calling `gigawatts status --json` once a minute is fine. Each call only reads the local database; it never makes Gigawatts ask Anthropic for new limits. The engine refreshes limits itself every 10 minutes while you're using Claude Code, and leaves them alone while you're idle, so a pane left open overnight costs nothing. Nothing changes faster than once a minute except running sessions' spend.

## Showing alerts

`alerts` lists limits that need attention, already ranked worst first, and is empty when nothing does. Show the first one and count the rest:

- `state: "full"`: the limit is at 100%. `at_ms` is when it resets. The terminal view says "Session back in 2h 40m".
- `state: "filling"`: the limit is on course to reach 100% before it resets. `at_ms` is when it's forecast to, or the snapshot's `written_at_ms` when that time has already passed, meaning any minute now. The terminal view says "Session full in ~1h 20m", rounded to 10 minutes because it's a forecast.

Count down from your own clock to `at_ms`, so the countdown stays right between calls. Once `at_ms` passes before your next call, hide a `full` alert, since its limit has reset, and show a `filling` one as any minute now. Name the account (`account_label`) when the alert isn't for the account you're showing.

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
