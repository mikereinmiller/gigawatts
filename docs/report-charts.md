# Report charts

The report page needs two charts. One shows spend per day, stacked by the top repos or models, for up to a year: 365 days by about 10 series. The other shows each limit's percent over the range, with its resets marked. Both need hover labels, have to read in light and dark mode, and must work offline. Whatever draws them is built into the binary (6.8 MB today) and copied into every exported file.

## Candidates

Checked 2026-11-11. Versions and dates come from each project's GitHub releases. Activity counts come from GitHub's participation and contributor stats for the last 52 weeks.

| | uPlot 1.6.32 | Chart.js 4.6.1 | ECharts 6.1.0 | Hand-drawn SVG |
|---|---|---|---|---|
| Minified size | 52 KB (JS and CSS) | 205 KB | 1,096 KB | 0 KB library, about 300 lines of our own JS (estimate) |
| License | MIT | MIT | Apache-2.0 | ours |
| Latest release | 2025-03-14 | 2026-10-22 | 2026-05-19 | n/a |
| Commits, last 52 weeks | 186 (181 by its one maintainer) | 20 | 238 | n/a |
| Active contributors, last 52 weeks | 7 | 5 | 18 | n/a |
| Open issues | 132 | 648 | 1,479 | n/a |
| Stacked bars | Not in the release. Unreleased `master` adds `opts.stack.groups` | Built in | Built in | We draw them |
| Hover labels | A live legend in the release. A tooltip exists only on unreleased `master` (commits 2026-10-30 and 2026-11-10) | Built in, all series for the hovered day | Built in, all series for the hovered day | We write them (the bench has one in about 10 lines) |
| Light and dark | Canvas, so colors are set in JS and a theme change needs a redraw | Same as uPlot | Ships a dark theme, but still needs a redraw | Follows CSS variables, so it switches with no code |
| Export | Adds 52 KB of library to every file | Adds 205 KB | Adds 1.1 MB | The exported file holds the SVG itself, with no library |
| Draw time, Safari 27.2 | 1 ms | 11.5 ms | 14 ms | 9 ms |
| Draw time, Firefox 160 | 1 ms | 11 ms | 26 ms | 11 ms |

The draw times come from the bench page, which draws the same generated year with each candidate and reports the median of 10 draws, including the browser's layout of the result. Firefox rounds its timer to whole milliseconds.

## Pros and cons

**uPlot.** It is the smallest and fastest library. Its README measures 3,600 points at 60 fps using 10% CPU, against 40% for Chart.js and 70% for ECharts. Its one maintainer is very active. The problem is timing: the released version, now 20 months old, has no stacking and no tooltip. Both exist only on `master`, which is 217 commits ahead of the release and changed this week. Today we would either write stacking and the tooltip ourselves, or build from unreleased code that is still moving.

**Chart.js.** Everything the page needs is built in and documented: stacked bars, hover that lists every series for a day, and a legend that hides a series on click. It shipped 4.6.0 and 4.6.1 last month. Work on it has slowed to 20 commits a year from 5 people, with 648 open issues. That matters less for a library this mature that only draws our own local data. It is a canvas library, so dark mode means redrawing with new colors.

**ECharts.** It has the most features and the most active project, under the Apache foundation. It is also 21 times the size of uPlot and 5 times Chart.js, which adds about 1.1 MB to the binary and to every export, for features the page will never use. Its minified file also contains a `new Function` fallback in its map loader, which a strict content security policy would have to allow.

**Hand-drawn SVG.** No third-party code: nothing to vendor, update, or audit. That is the same reason Gigawatts uses no HTTP crate. Colors come from the page's CSS variables, so light and dark need no code. Labels are real text, an export holds the finished SVG, and the page can test exactly what it draws. The cost is that we write and maintain the axes, date ticks, stacking, and hover ourselves, an estimated 300 lines. Thousands of SVG elements also get slow to hover in a way canvas doesn't. The bench's 3,650 bars is the case to time.

## Recommendation

Hand-drawn SVG, if its Safari and Firefox times are close to the canvas libraries. The page draws two simple chart types from our own data, and SVG gives light and dark, export, and offline use without carrying a library. Its main risk, hover over 3,650 bars, is what the bench measures.

If SVG turns out slow or awkward, Chart.js is the fallback. Everything the page needs is built in, at 205 KB. uPlot becomes the better library once a release includes stacking and its tooltip, but today that means unreleased code. ECharts costs too much size for what the page uses.

## Pick

Hand-drawn SVG (Mike, 2026-11-11). Hover worked in every candidate in Safari and Firefox, so draw time, size, and theming decided it. Each chart is one function in the page that takes the API's JSON and returns SVG, so a library can replace just those functions if the page later needs zoom, panning, or hover over dense data.
