# Reading Gigawatts insights

Follow these steps to tell the user where their Claude Code spend goes, why their costliest runs cost what they do, and which edits to their skill and agent files would make them cheaper. Gigawatts measures cost; the user judges quality. Every proposal names its cost effect from the numbers and leaves the quality call to the user.

## Rules for every claim

- What Gigawatts recorded beats what a file says. A config file or frontmatter can be overridden at launch, so never say a model or effort is in effect unless the numbers show requests on it. When a file and the numbers disagree, report both and say the file isn't what ran.
- Name the command output, session, or `path:line` behind every number and every claim about a file. A claim you couldn't check is labeled unverified, with the check that would settle it.
- A skill's cost includes the subagents it launched. Say which part a finding is about: the skill's own conversation, or its subagents.
- A saving is **measured** only when runs of the same skill already ran both ways (before and after a shift, or sessions with and without the pattern). Otherwise it is **estimated**: show the arithmetic, such as "2 of 6 reviewer launches per session × $0.94".
- Volume is not a finding. "You run `/review` a lot" tells the user nothing. A finding is a pattern in how a skill or session spends, tied to a line that sets it.
- When a check or section has too little data to judge, write "not enough data" with what's missing. Never leave it out silently.

## 1. Read the numbers

Run `gigawatts insights --json`. `gigawatts schema` (the `insights` root) defines every field.

Facts the fields rest on:

- A skill run is one stretch of the skill in a session's main conversation. A skill's `cost_usd` includes the subagents it launched, and `subagent_cost_usd` is that part. A subagent's cost also appears in the skill that ran it, so compare rows within a list and never add the lists together.
- Each skill row's `facts` is computed from the whole history in range, so take these numbers from it and never recount them from transcripts: `runs_per_session`, `repeat_run_cost_usd` (what runs after the first in a session cost, subagents included), `top_tenth_session_share` (the share of the row's cost in its costliest tenth of sessions), and `launches`, per subagent type: `launches`, `per_run`, `cost_usd`, and `first_request_cost_usd`, mostly each launch's starting context written to the cache. A subagent row has its own `first_request_cost_usd` across all its launches.
- `weeks` covers the last 8 Monday weeks. A `shift` is a move of 30% or more in cost per run, with its `causes`: the model, effort, or Claude Code version whose largest share of spend changed across it. A cause with `in_subagents: true` changed in the subagents the skill launched, not in the skill's own conversation. An empty `causes` means none of those changed.

## 2. Pick the targets

Take the three costliest skills, the three costliest subagents, and every row with a shift. A row with fewer than 10 `runs` is too thin to judge: list its numbers and propose nothing for it. A shift whose latest weeks are already back near the cost before it is already fixed: report it as fixed and propose nothing.

## 3. Look inside the costliest sessions

The weekly numbers say what a skill costs on average. The sessions say why. For each target with 10 runs or more:

1. Run `gigawatts data --since <data_starts> --by skill --drill <name> --json` (`--by agent` for a subagent). Each row is one session's spend on that skill or subagent, with its `key` (the session ID), `account_label`, `requests`, `cost_usd`, and `paid_for`, what the cost paid for in dollars. A session can hold several runs of a skill, so a session's cost is not a run's cost.
2. Find the shape of the spend from `facts.top_tenth_session_share`, and the median session's cost from the drill rows. Judge spread only when the drill shows 10 sessions or more, since the costliest tenth is always at least one session; with fewer, write "not enough data" in the Spread column. Under about 0.3 means the cost is spread evenly and the skill's design sets the price. Much more means a few sessions went wrong or were unusually large; study those.
3. Read `paid_for` of the costliest sessions against the median one, always in dollars. Never reason from token counts: cache reads are by far the largest count in almost every session while costing the least per token, so counts point at the wrong thing.
   - `cache_read_usd` the largest part: a long context re-read on every turn. More `requests` means more turns paying for it.
   - `cache_write_usd` the largest part: context sent fresh, from large file or diff reads, a resumed session whose cache expired, or many new subagents each starting cold.
   - `output_usd` the largest part: long generated text, such as full reports or rewritten files.
4. Open transcripts only to explain a fact the numbers raise, such as why repeat runs cost what they do or what a subagent receives first. Take the costliest two sessions and one median session. Each is `<key>.jsonl` under `projects/` in the config folder of the session's account (find it with `fd <key>.jsonl <config folder>/projects`). Transcripts are large, so never read one whole. Use `jq` to find the largest tool results, subagent prompt sizes, and the user's messages. Comparing the costly sessions with the median one shows what the costly ones did differently. Counts of runs and launches come from `facts`, not from these transcripts.

## 4. Find each target's file

The numbers combine every config folder Gigawatts has read, tracked or not, so search all of them. Run `gigawatts accounts`; it lists each account's config folders, such as `~/.claude`, and each folder that isn't logged in. Then look in this order and read every file that matches:

- Skills: `.claude/skills/<name>/SKILL.md` in the current project, then `skills/<name>/SKILL.md` and `plugins/**/skills/<name>/SKILL.md` in each config folder `gigawatts accounts` lists.
- Subagents: `.claude/agents/<name>.md` in the current project, then `agents/<name>.md` and `plugins/**/agents/<name>.md` in each config folder `gigawatts accounts` lists.
- Claude Code's built-in subagents (`general-purpose`, `Explore`, `Plan`) have no file. Search the skill files for the name instead, and treat the skills that launch it as the targets.

Also read every file a skill tells Claude to read or follow, since its text loads too. A skill or subagent can also live in another project's `.claude/` folder, which can't be seen from here. Report a target with no file found as "not found here, possibly in another project", with the paths checked.

## 5. Check each skill for cost patterns

Go through every check below for each target. Each names what to look for in the files, how to confirm it in the sessions, and the usual edit. Report a check only when the sessions confirm it; a pattern in the text that never shows up in the sessions costs nothing.

1. **Repeated work.** The skill redoes a full pass when it runs again on the same input (a second review of an unchanged diff, a re-read of the same files). Confirm: `facts.repeat_run_cost_usd` is the dollar size; then the transcripts show whether the repeat runs covered unchanged input. Edit: reuse earlier results and redo only what changed.
2. **Fan-out.** The skill launches several subagents with overlapping work. Confirm: `per_run` in `facts.launches`, and from the transcripts whether each launch's findings were distinct. Edit: fewer subagents, or one with combined focus below a size threshold.
3. **What each subagent receives.** Every subagent starts with an empty cache, so whatever it receives or reads first is paid for as cache writes on every launch. This is a lever the skill controls, not a fixed cost of using subagents. Look for subagents that get the whole input when they need part of it, or file contents pasted into the prompt when a path would do. Confirm: `first_request_cost_usd` in `facts.launches` against that entry's `cost_usd`, and the prompt sizes in the transcripts. Edit: pass a slice or a path, or launch fewer.
4. **Unpinned subagents.** A launch names no model or effort, so it inherits the session's. Confirm: the models those subagents' requests ran on. Edit: name the model in the launch line or the agent's frontmatter.
5. **Text loaded every run.** The skill, its reference files, or files it reads in full are large. Confirm: file sizes (`wc -c`) against the session's cache writes. Edit: move rarely needed parts into files read only on the branch that needs them.
6. **Size limits.** Nothing caps how big an input the skill takes on in one go (a 6,000-line diff, a whole repo). Confirm: the inputs of the costliest sessions against the median. Edit: split above a threshold, such as per commit or per directory.
7. **Long sessions.** The skill runs late in long sessions, so every turn re-reads a large context. Confirm: high cache reads with many requests, and where in the session the skill's turns fall. Edit: a note to start a fresh session or compact before the skill, or have the skill hand its heavy part to a subagent.
8. **Cold caches.** Cache writes spike after idle gaps, such as a session resumed the next day. Confirm: timestamps with long gaps before cache-write spikes. Edit: usually none in the skill; tell the user what it costs.
9. **Heavy tool output.** Screenshots, full page dumps, or long command output fill the context. Confirm: the largest tool results by tool name. Edit: text-only tools first (`get_page_text` over screenshots), `head` or filters on long commands.
10. **Loops and retries.** The same tool call repeats with small changes, or a step fails and reruns. Confirm: repeated calls in the transcript. Edit: fix the instruction that causes the retry.
11. **Effort and model.** High effort or a larger model on mechanical steps. Confirm: requests on that model or effort in the numbers, and what those steps do. Edit: a lower setting for those steps.

For a shift, start from its `causes`. A `version` cause alone is weak evidence, since Claude Code updates every few days. When `causes` has no `model` or `effort` entry, run `git log --since <week minus 14 days> --until <week plus 14 days> -- <file>` when the file sits in a git repo, and name the edit that lines up with the shift.

## 6. Check sessions and caching across all usage

Skills aside, how sessions are run changes cost too. Run `gigawatts data --since <data_starts> --by repo --json` and add up `paid_for` across the rows.

- Report each part's share of the total: cache reads, cache writes, output, input.
- When cache reads are more than half: find the costliest sessions by `cache_read_usd` (`--by repo --drill <repo>` lists a repo's branches, then sessions). If they run hundreds of requests, long sessions are re-reading their context on every turn. The habit to suggest is starting a new session or running `/compact` between unrelated tasks. Name the sessions it would have helped and roughly what they spent on cache reads.
- When cache writes are more than a quarter, first split them with `gigawatts data --since <data_starts> --by agent --json`: subagents' cache writes are check 3 in step 5, and only the main conversation's ("you (no subagent)") belong here. For each of the five costliest sessions, count gaps of over an hour between turns with `jq -rs '[.[] | select(.type=="assistant" or .type=="user") | .timestamp[:19] + "Z" | fromdateiso8601] | [range(1; length) as $i | .[$i] - .[$i-1]] | map(select(. > 3600)) | length' <transcript>`. A turn after such a gap writes the whole context again. Report how many of those sessions had gaps, and suggest the habit only if most did: finish a task in one sitting, or start a fresh session instead of resuming a long one the next day.
- Propose a habit only when the part it reduces is at least 10% of total spend, and say how much of that the habit would plausibly save. Below that, report the shares and say no habit change is worth it.

## 7. Report

Use exactly this layout. Tables carry the numbers; sentences explain them. Keep every finding in the same order of parts so the user can compare them.

**Where the money goes.** Start with one line on what the money paid for, from step 6: "62% cache reads, 21% cache writes, 15% output, 2% input". Then one table, costliest first, with the targets from step 2:

| Skill or subagent | Runs | Total | Per run | This week | Subagents | Spread |
|---|---|---|---|---|---|---|

"This week" is the last week's cost per run; "Per run" covers all 8 weeks, so the two together show the trend. "Subagents" is `subagent_cost_usd` as a share of the total, or "-" for a subagent row. "Spread" is "even" or "outliers" from step 3. Under the table, at most three sentences on what stands out.

**Findings**, ranked by estimated saving, each estimated from the dollar figures in `facts` and `paid_for` so two readings of the same history rank the same way, each as:

> ### 1. A title that names the pattern, not the skill
> **Evidence:** what the numbers and sessions show, with the session IDs or command.
> **Edit:** `path:line`, the current text, and the proposed text.
> **Saving:** dollars a month, labeled measured or estimated, with the arithmetic.
> **Risk:** what it might cost in quality, in one sentence.

**Sessions and caching**: the shares from step 6 and any habit worth changing, in at most three sentences.

**Already fixed** and **too thin to judge**: one line each.

**Proposals.** A closing table, ranked by saving:

| # | Edit | Saving a month | Measured or estimated | Risk |
|---|---|---|---|---|

Edit nothing until the user picks which proposals to apply. End with: ask again in two weeks, once enough runs exist on each side to show whether the change worked.
