use gigawatts::contract::{Credits, Limit, LimitSource};
use gigawatts::limits::{LimitsRow, ShownLimit, render};

const MINUTE: i64 = 60_000;
const NOW: i64 = 1_793_800_000_000;

fn window(kind: &str, scope: Option<&str>, percent: f64, severity: &str) -> Limit {
    Limit {
        kind: kind.to_string(),
        group: None,
        scope: scope.map(str::to_string),
        percent,
        severity: Some(severity.to_string()),
        resets_at: Some("2026-11-08T02:59:59.945600+00:00".to_string()),
        active: false,
        credits: None,
    }
}

#[test]
fn each_account_shows_its_limits_with_source_age_and_reset() {
    let rows = vec![
        LimitsRow {
            label: "Ultra".to_string(),
            source: LimitSource::GetUsage,
            fetched_at_ms: NOW - 3 * MINUTE,
            fetched_local: "2026-11-04 13:30".to_string(),
            limits: vec![
                ShownLimit {
                    limit: window("session", None, 2.0, "normal"),
                    resets_local: Some("2026-11-04 17:59".to_string()),
                },
                ShownLimit {
                    limit: window("weekly_all", None, 63.0, "normal"),
                    resets_local: Some("2026-11-07 21:59".to_string()),
                },
                ShownLimit {
                    limit: window("weekly_scoped", Some("Fable"), 90.0, "critical"),
                    resets_local: Some("2026-11-07 21:59".to_string()),
                },
            ],
        },
        LimitsRow {
            label: "Enterprise".to_string(),
            source: LimitSource::CachedUsageUtilization,
            fetched_at_ms: NOW - 50 * 60 * MINUTE,
            fetched_local: "2026-11-02 11:40".to_string(),
            limits: vec![ShownLimit {
                limit: Limit {
                    kind: "credits".to_string(),
                    group: None,
                    scope: None,
                    percent: 20.0,
                    severity: Some("normal".to_string()),
                    resets_at: None,
                    active: false,
                    credits: Some(Credits {
                        used: 437.6,
                        limit: Some(2200.0),
                        currency: "USD".to_string(),
                    }),
                },
                resets_local: None,
            }],
        },
    ];

    assert_eq!(
        render(&rows, NOW),
        "\
Ultra, from get_usage at 2026-11-04 13:30 (3 minutes ago)
  Session                2%  resets 2026-11-04 17:59
  Weekly, all models    63%  resets 2026-11-07 21:59
  Weekly, Fable         90%  resets 2026-11-07 21:59, critical

Enterprise, from cachedUsageUtilization at 2026-11-02 11:40 (2 days ago)
  Credits               20%  437.60 of 2200.00 USD
"
    );
}

#[test]
fn no_readings_says_when_they_arrive() {
    assert_eq!(
        render(&[], NOW),
        "No limit readings yet. The engine reads them while Claude Code is active.\n"
    );
}
