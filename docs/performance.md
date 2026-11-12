# Performance

Measured results, so a slow drift shows up when someone looks. Each run of `tests/heavy_year.rs` prints its numbers with `cargo test --test heavy_year -- --nocapture`.

## Heavy year

`tests/heavy_year.rs` builds a year of 5,000 requests a day with 90-day request retention. The size limit is strict. The time limits are set at about twice the typical time, so they catch a regression of 2× or more, such as a lost index. They don't prove an exact speed: the same query varies by half again from run to run on a busy machine.

Measured 2026-11-11 on the Mac (Apple Silicon), across six runs: four debug, two release.

| What | Limit | Measured |
|---|---|---|
| Database size | 110 MB | 106.1 MB, 247 bytes per stored request |
| Today's query | 5 ms | 0.34–0.39 ms |
| Dollar pace, active sessions, and today's cost flags | 40 ms | 18.7–19.4 ms |
| Yearly report by model | 250 ms | 61–87 ms |
| Yearly report by repo | 250 ms | 77–113 ms |
| Yearly report by branch | 250 ms | 84–138 ms |
| Daily series by repo | 250 ms | 76–112 ms |
| Drill to sessions | 250 ms | 58–78 ms |

Release builds measured the same as debug builds, because nearly all the time is spent inside SQLite.
