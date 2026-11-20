# Report page design

What the report page shows, in what order, and why. Researched 2026-11-11 from dashboard and cost-reporting guidance, the old `/spend` and `/today` skills, and the dataviz rules this repo's charts follow.

## What the research says

- **A number needs a comparison to mean anything.** A bare total doesn't say whether to act. Pair each headline number with a previous period, a pace, or a limit, so it reads as a verdict ([Tabular Editor on KPI cards](https://tabulareditor.com/blog/kpi-card-best-practices-dashboard-design), [Esri on effective dashboards](https://enterprise.arcgis.com/en/portal/10.6/use/author-effective-dashboards.htm)).
- **Few headline numbers, the most important biggest and first.** Eight to twelve cards compete for attention. Keep the headline row to four tiles or fewer, with one hero figure (Stephen Few, via the same sources).
- **Overview first, then zoom and filter, then details on demand.** Shneiderman's mantra: the page opens on the overview, the date range and grouping narrow it, and a click or hover gives the detail ([Shneiderman 1996](https://webspace.science.uu.nl/~telea001/uploads/VACourse/Shneiderman96.pdf)). One filter row at the top scopes everything below it.
- **Cost reports answer four questions:** what was used, what it cost, why it changed, and what to do next. They also show how fresh the data is ([OneUptime on showback dashboards](https://oneuptime.com/blog/post/2026-07-21-cloud-showback-dashboard-metrics/markdown)).
- **Budget views project the current rate to the end of the period** and compare that with the limit. The projection is the number that changes behavior ([budget dashboard guide](https://www-infograph-ai.pages.dev/blog/budget-dashboard), [Datadog on cloud cost management](https://www.datadoghq.com/blog/cloud-cost-management-finops)).
- **Stacked bars show the total and the leading part well, and middle parts poorly.** Cap the stack at the top series plus "other", and give exact comparisons to ranked bars or a table ([PolicyViz](https://policyviz.com/2025/06/24/can-we-improve-on-the-stacked-bar-chart/), [SAS on "Others"](https://blogs.sas.com/content/iml/2013/01/14/create-a-bar-chart-with-an-others-category.html)).
- **Tables carry the detail; bars inside rows show magnitude at a glance.** Right-aligned tabular figures and muted rules, with charts as summaries above the table, as Stripe's dashboard does ([Datawrapper on bars in tables](https://www.datawrapper.de/academy/how-to-add-bar-charts-line-charts-to-tables), [Setproduct on data tables](https://www.setproduct.com/blog/data-table-ui-design)).
- **AI spend pages break cost down by model with the token mix beside it**, and separate rising volume from rising unit cost ([OpenRouter's activity page](https://openrouter.ai/blog/announcements/activity-dashboard/), [Metabase's LLM spend overview](https://www.metabase.com/dashboards/llm-spend-overview)).

From the old skills, the ideas that held up are credits left and how long they last, credits per remaining weekday, and who did the work, meaning which subagent ran on which model.

## Layout

One page, read top to bottom, one account at a time.

When `gigawatts doctor` lists a problem, a box above the headline names what each one makes wrong and since when, with its cause, so a dip in a chart isn't taken at face value. The fix steps stay in `doctor`.

1. **Filter row.** One toolbar: an account picker (each tracked account with its spend today, plus All accounts), a range picker, any scope chips, a theme picker (Auto, Light, Dark, remembered in the browser), and Export. The range picker's button names the preset and its dates ("This month | Nov 1–11"), and its menu holds the presets with a custom from-and-to form below them. Account is context you switch rarely and range is a filter you change often, so both are menus behind one button each rather than tab bars, which keeps the row to one line on a phone. Everything below follows both. The view is kept in the address, so a reload or a bookmark returns to it.
2. **Headline row, at most four tiles.**
   - *Spend* (the hero): the range's total, with the change against the previous period. This month compares with the same days of last month (Nov 1–11 with Oct 1–11), and last month with the whole month before, as monthly reporting does. Other ranges compare with the same number of days just before them. Every comparison names its dates. History counts as starting the day after the earliest record, because Claude Code's transcript cleanup usually cuts that first day part-way; a comparison that reaches further back says "No comparison" instead of showing a ratio.
   - *This month*: month-to-date spend and the pace to the end of the month.
   - *Credits* for an account with a credit limit: used against the limit as a meter, what's left, and when it runs out if the engine forecasts that before the reset, otherwise whether it lasts past the reset at this month's average day. For an account with session and weekly limits: the closest limit as a meter, with its reset and the 100% forecast when there is one.
   - *Per weekday left* for a credit account: what's left divided by the weekdays until the reset, against this month's average weekday. Otherwise, *today* against the usual day.
3. **Spend per day.** Stacked columns of the top 8 repos or models, plus "other", with a hover readout of every series for that day. A legend is always shown, and the table below carries every value.
4. **Limits over time.** One chart per limit (session, weekly, any scoped weekly limit such as Fable, and credits for a credit account) showing its percent across the range, starting from the reading in effect on the first day. Dashed lines mark resets, and the line drops to 0% at each one until the next reading. Credits readings carry no reset time, so their monthly reset shows only as the drop. Limits belong to the account, so in a scope the chart stays and says it covers the whole account.
5. **Where it went.** Repo, skill, subagent, or model, as a table: share, cost, sessions, cost per session, and what the cost paid for (input, output, cache writes, cache reads, at list price). Once there are more than 8 rows, rows under 1% fold into one "Others (N)" row that carries their sum, so the rows still add up to the total. Its "Show" link lists them at the same level, with a "Show fewer" row under them. A click on any other row opens its side panel, and a branch is a breakdown inside its repo's scope. Raw token counts are left out, because a cache-read token costs a twentieth of an input token and a hundredth of an output token, so counts mislead.
6. **Who did the work.** Subagents (and your own conversation) by model, as a table. It shows at a glance when a subagent runs on the expensive model. Its rows fold the same way, and the printed page shows the "Others (N)" row without the link.
7. **Biggest sessions.** The ten most expensive sessions in the range. A session row shows its title with its date, main skill, active time, and peak context, and opens a side panel with its split by skill, model, and subagent.

The page then turns from what happened in the range to what to change. The cards below don't follow the range picker, so they sit last, and each heading names its own time window.

8. **Anthropic's advice.** Unscoped page only, and only when the range includes today.
9. **Insights.** Unscoped page only.

A past range (last month, or a custom range that ends before today) leads with that period instead of current state: its average active day, busiest day, and biggest driver.

## Details and scope

The page has two ways to look closer, for two different questions.

- **The side panel answers "what is this one thing?"** and leaves the page as it is, so you can open a row, close it, and open the next while comparing. A click on any row or on a day in the chart opens it. The panel shows the same parts for everything: the total against the previous period, a small trend over the range, what the cost paid for (input, output, cache writes, cache reads) with one sentence on what the largest part usually means, its split along the other groupings (a repo's skills and models, a skill's repos), and its top sessions. A split list shows up to 7 entries, largest first; with more, it shows 6 and the rest as one "Others (N)" line with their sum, or "Other" in a day's panel when the rest include the chart's own "Other". A day's panel compares it with the usual day. A session's panel stays as it is.
- **A scope answers "show me everything within this"** and redraws the whole page for it. The panel's *Focus on <name>* button applies it, and on a day it sets the range to that day. The scope shows as a chip in the filter row (`Repo: gigawatts ✕`). Scopes combine, such as a repo and a skill, and the address keeps them, so Back and bookmarks work.

The "Not in transcripts" row (spend Claude Code billed for requests it never wrote to its transcripts) opens a panel that explains it and shows its cost, its change against the period before, its days, and its split by model. It has no Focus button, since nothing can be filtered to it.

The "Anthropic's advice" card lists Anthropic's recommendations from Claude Code's `/usage`, which the engine saves hourly, with each one's share of the week's usage, in the order and number Claude Code gives them. Its hint says "this week's usage" or "today's usage" to match that share, since it doesn't follow the range picker. It stays out of exports, because the advice names files and paths.

The "Insights" card closes the unscoped page. It is one card, built like the others, with two tabs in its header: "Suggestions" (what to fix next, one row per skill or subagent) and "Changes" (did past changes make cost per run better or worse). Both come from `gigawatts insights`, so one line under the header says they cover the last 8 weeks across every account and ignore both the range and the account picker. Suggestions is the default tab, because what to do next matters more than the history; Changes is the check on whether earlier edits worked. The tab isn't called "Savings" because Changes leads with money already saved, and not "History" because it lists only changes of 30% or more, not every week. In Suggestions, skills and subagents share one list under "What ran", with "Subagent" under a subagent's name, so the biggest saving leads whichever kind it is. A skill's cost already includes the subagents it launched, so a note under the list says not to add rows together.

Changes is its own tab, not rows in Suggestions, because ranking by what's left to save sinks a change that already worked: the biggest saving in the history would sit low with nothing left to save. Its first line gives the net verdict ("Saving $1,689 a month, costing $360 a month"). Below it is one line per change event, newest first: the date, the cause, the rows it moved, and "saving" or "costing" dollars a month. An event that moved several rows, such as a skill and the subagent it launches, is one line, sized by the row it moved most, so nothing is counted twice. Rows join one event when they move the same way in the same week with the same model or effort change, or, with no such change, when one is a skill and the other a subagent it launches. A rise that a later change set back is folded into that later line ("undoing Oct 12: moved to Fable 5.1, high effort") instead of being listed on its own; with no such line it shows muted as "reversed <date>". A rise counts as reversed when the same row, or any row it also moved, later switches the same model or effort back, or when a later drop on the same row brings cost per run back to where the rise started or below.

A click on a change opens its own panel: what it saves or costs, the next step ("Keep it", "setting it back would save about $X a month", or, with no model or effort cause, which skill's edits to check around that date), then a weekly cost-per-run chart for each row it moved, with the weeks before the change faded, a dot on the change week and on the week of any rise it undid, and the two-week averages on each side beside the row's name, and a link to each row's panel. The charts are the point of the panel: a change is something that happened over time, and two averages alone can't show whether it held.

In Suggestions, each row is one skill or subagent. A row shows its biggest takeaway with "+N more" when it has others, its runs over the 8 weeks (so a $6-a-run skill with 6 runs reads differently from one with 800), what it has at stake a month, and its cost per run over the last two weeks with the change against the two before. There is no trend chart in the row: at row height it has no scale, low-run weeks make it look busy, and it repeats the last-two-weeks figure, so the weekly view lives in the panel. Rows are ranked by what they have at stake, the way cost tools rank recommendations by dollar impact (Datadog's recommendations, Google's Active Assist, AWS anomaly root causes), and rows with nothing at stake follow in cost order. Eight show at first, with "Show all" for the rest. A finding about a subagent a skill launches ("1.8 code-reviewer launches a run") stays on the skill, which decides how many to launch.

A click opens the row's side panel: its numbers; its cost per run as one bar a week for 8 weeks, with the top of the scale in dollars, each week's run count under its bar, a dot on each week a change started, and a hover readout of the week's cost per run, runs, and total; its takeaways, each a title, its dollars a month, and what to check; its changes, the only place a row's own changes are listed, each the date and cause, what it saves or costs a month or "reversed <date>", the two-week averages on each side, and the other rows the same change moved; the subagents it launches; and a prompt to copy into Claude for the edits. Takeaways and changes come from fixed rules over the insights numbers, computed by `insights::takeaways` and served with the insights numbers by the private `/api/insights` route. The takeaways are: repeat runs at 25% or more of the cost, 1.5 or more launches of one subagent a run, first requests at 25% or more of a subagent's cost, a costliest tenth of sessions at half the cost or more over at least 10 sessions, and a rise in cost per run that no later change reversed. Anything under $20 a month is left out, and a row with fewer than 10 runs says it's too thin to judge. A Claude Code version change is never named as a cause, since Claude Code updates every few days. The page says where to look; the edit itself needs your skill files, which only Claude reads.

It always covers the default 8 weeks, whatever the range picker says, because a shift needs four weeks to compare. It is left out of a scoped page, since its runs can't be split by repo, and out of exports, because it always covers the default 8 weeks across every account and would disagree with the range and scope the export states.

A click previews and the button commits, because redrawing the page on every click gets in the way of scanning. A session is never a scope: it is the last level, and its panel already shows everything about it.

Within a scope the page changes to fit it:

| Scope | Headline tiles | Chart stacks by | Breakdown tabs (first is default) |
|---|---|---|---|
| None | As in Layout | Repo or model | Repo, skill, subagent, model |
| Repo | Spend against the previous period, share of the account's spend, sessions, cost per session | Branch | Branch, skill, subagent, model |
| Skill | Same | Repo | Repo, subagent, model |
| Model | Same | Repo | Repo, skill, subagent; "Who did the work" leads, since it shows who chose the model |
| Subagent | Same | Model | Model, repo, skill |

Credits, limits, and month pace belong to the account, not to a scope, so in a scope they leave the headline and become one line under the filter row ("Account: $412 this month, 62% of credits"). The limits chart stays and says it covers the whole account. A scoped "Biggest sessions" lists that scope's sessions.

Scope needs the API to filter by any grouping, not only down a grouping's own levels: `repo`, `branch`, `skill`, `model`, and `agent` parameters on `/api/data`, `/api/series`, `/api/mix`, `/api/sessions`, and `/api/agents`.

## Export

Export shares a summary and leaves out repo, branch, and session names; skill, subagent, and model names stay, since you name those. A scoped view exports with its scope named in the title, so a repo's name appears only when you scoped to it. Two outputs:

- *Markdown*: the summary as text and tables. It pastes cleanly into Slack, GitHub, and Linear, and it is the best input for an agent, since models read exact values from a table more reliably than from a chart image ([Format Matters, 2025](https://arxiv.org/html/2511.10075v1)).
- *PDF* through the browser's print dialog and a print stylesheet: the shareable picture, charts included, since they are SVG.

Both carry the same summary, written for someone judging the period rather than watching today: spend against the period before, active days with the average active day and the busiest one, sessions with cost per session, and credits used on a credit plan or otherwise how much input came from the prompt cache. Then spend per day by model, tables by model and by skill with each row's change against the period before, and who did the work. Today, limit forecasts, and the busiest session are left out, because they describe this moment rather than the period. A footnote explains spend that is not in transcripts.

A PNG is left out. Drawing the page to an image takes either an SVG `foreignObject`, which Safari renders unreliably, or a library, which the page otherwise avoids. Claude Enterprise admins already get per-user, per-model spend from Anthropic's analytics and spend report; this adds why the money went where it did.

## Rules

- Colors follow the dataviz reference palette, validated in light and dark (`validate_palette.js`: all checks pass, with three light colors under 3:1 relieved by the table and legend). Status colors (good, warning, critical) appear only on meters, always beside a number.
- A series keeps its color everywhere on the page: the chart, its legend, and the breakdown bars.
- One axis per chart. Gridlines are solid hairlines. Bars are at most 24 px wide, with a 2 px gap between stacked parts.
- Every estimate is labeled: the month pace, how long credits last, and the per-weekday figure all say what they're based on.
- The page reads only the engine's own API on 127.0.0.1, and inserts all data as text.
