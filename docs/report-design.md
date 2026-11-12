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

1. **Filter row.** One toolbar: an account picker (each tracked account with its spend today, plus All accounts), a range picker, any scope chips, and Export. The range picker's button names the preset and its dates ("This month | Nov 1–11"), and its menu holds the presets with a custom from-and-to form below them. Account is context you switch rarely and range is a filter you change often, so both are menus behind one button each rather than tab bars, which keeps the row to one line on a phone. Everything below follows both. The view is kept in the address, so a reload or a bookmark returns to it.
2. **Headline row, at most four tiles.**
   - *Spend* (the hero): the range's total, with the change against the previous period. This month compares with the same days of last month (Nov 1–11 with Oct 1–11), and last month with the whole month before, as monthly reporting does. Other ranges compare with the same number of days just before them. Every comparison names its dates. History counts as starting the day after the earliest record, because Claude Code's transcript cleanup usually cuts that first day part-way; a comparison that reaches further back says "No comparison" instead of showing a ratio.
   - *This month*: month-to-date spend and the pace to the end of the month.
   - *Credits* for an account with a credit limit: used against the limit as a meter, what's left, and how long it lasts at this month's average day. For an account with session and weekly limits: the closest limit as a meter, with its reset and the 100% forecast when there is one.
   - *Per weekday left* for a credit account: what's left divided by the weekdays until the reset, against this month's average weekday. Otherwise, *today* against the usual day.
3. **Spend per day.** Stacked columns of the top 8 repos or models, plus "other", with a hover readout of every series for that day. A legend is always shown, and the table below carries every value.
4. **Limits over time.** One chart per limit (session, weekly, any scoped weekly limit such as Fable, and credits for a credit account) showing its percent across the range, starting from the reading in effect on the first day. Dashed lines mark resets, and the line drops to 0% at each one until the next reading. Credits readings carry no reset time, so their monthly reset shows only as the drop. Limits belong to the account, so in a scope the chart stays and says it covers the whole account.
5. **Where it went.** Repo, skill, subagent, or model, as a table: share, cost, sessions, cost per session, and what the cost paid for (input, output, cache writes, cache reads, at list price). Rows under 1% fold into one row once there are more than 8. Nothing expands in place: a click opens the row's side panel, and a branch is a breakdown inside its repo's scope. Raw token counts are left out, because a cache-read token costs a twentieth of an input token and a hundredth of an output token, so counts mislead.
6. **Who did the work.** Subagents (and your own conversation) by model, as a table. It shows at a glance when a subagent runs on the expensive model.
7. **Biggest sessions.** The ten most expensive sessions in the range. A session row shows its title with its date, main skill, active time, and peak context, and opens a side panel with its split by skill, model, and subagent.

A past range (last month, or a custom range that ends before today) leads with that period instead of current state: its average active day, busiest day, and biggest driver.

## Details and scope

The page has two ways to look closer, for two different questions.

- **The side panel answers "what is this one thing?"** and leaves the page as it is, so you can open a row, close it, and open the next while comparing. A click on any row or on a day in the chart opens it. The panel shows the same parts for everything: the total against the previous period, a small trend over the range, what the cost paid for (input, output, cache writes, cache reads) with one sentence on what the largest part usually means, its split along the other groupings (a repo's skills and models, a skill's repos), and its top sessions. A day's panel compares it with the usual day. A session's panel stays as it is.
- **A scope answers "show me everything within this"** and redraws the whole page for it. The panel's *Focus on <name>* button applies it, and on a day it sets the range to that day. The scope shows as a chip in the filter row (`Repo: gigawatts ✕`). Scopes combine, such as a repo and a skill, and the address keeps them, so Back and bookmarks work.

The "Not in transcripts" row (spend Claude Code billed for requests it never wrote to its transcripts) opens a panel that explains it and shows its cost, its change against the period before, its days, and its split by model. It has no Focus button, since nothing can be filtered to it.

A "Ways to spend less" card on the unscoped page lists Anthropic's recommendations from Claude Code's `/usage`, which the engine saves hourly, with each one's share of the week's usage. It stays out of exports, because the advice names files and paths.

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
