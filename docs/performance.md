# Performance

Measured results, so a slow drift shows up when someone looks. Each run of `tests/heavy_year.rs` prints its numbers with `cargo test --test heavy_year -- --nocapture`.

## Heavy year

`tests/heavy_year.rs` builds a year of 5,000 requests a day with 90-day request retention. The size limit is strict. The time limits are set at about twice the typical time, so they catch a regression of 2× or more, such as a lost index. They don't prove an exact speed: the same query varies by half again from run to run on a busy machine. When `CI` is set, the time limits are 4 times the ones below, because GitHub's runners measure these queries 2 to 3 times slower than the Mac, so CI catches a regression of about 3× or more.

Measured 2026-11-11 on the Mac (Apple Silicon), across six runs: four debug, two release.

| What | Limit | Measured |
|---|---|---|
| Database size | 110 MB | 106.1 MB, 247 bytes per stored request |
| Today's query | 5 ms | 0.34–0.39 ms |
| Dollar pace, active sessions, sessions today, and today's cost flags (2026-11-19, four release runs) | 40 ms | 21.6–22.4 ms |
| Yearly report by model | 250 ms | 61–87 ms |
| Yearly report by repo | 250 ms | 77–113 ms |
| Yearly report by branch | 250 ms | 84–138 ms |
| Daily series by repo | 250 ms | 76–112 ms |
| Drill to sessions | 250 ms | 58–78 ms |
| 10 biggest sessions (measured 2026-11-12, one debug and one release run) | 250 ms | 102–115 ms |
| Cost mix and session counts by repo (2026-11-12) | 250 ms | 141–173 ms |
| Subagents by model (2026-11-12) | 250 ms | 65–72 ms |
| Session and weekly dollars per point, from 28 days of readings every 10 minutes (2026-11-17, one debug and one release run) | 80 ms | 42–59 ms |

Release builds measured the same as debug builds, because nearly all the time is spent inside SQLite.

## Engine resources

`tests/engine_resources.rs` checks the engine's memory after it reads a generated history and its CPU time while idle. That history is far smaller than a real one, so this catches leaks and an engine that keeps working while idle. It doesn't prove the 50 MB target on real data, and it can't count wakeups. Both of those stay manual measurements before release, with `sudo powermetrics` on the Mac and `perf` on Linux.

Measured 2026-11-11 on the Mac, across four runs, reading 10,000 requests in 20 sessions:

| What | Limit | Measured |
|---|---|---|
| Memory after the first full read | 50 MB | 19.6–20.3 MB |
| CPU time while idle for 10 seconds | 50 ms | 0 ms |
| CPU time to read the history | none | 400 ms |

Measured 2026-11-12 on the Mac with the report server on a free port, one run each:

| What | Server off | Server on |
|---|---|---|
| Memory after the first full read | 19.6 MB | 20.1 MB |
| CPU time while idle for 10 seconds | 0 ms | 0 ms |
