use gigawatts::contract::{
    AccountLimits, AccountToday, Credits, DollarPace, Limit, LimitPace, LimitSource, PaceBasis,
    PaceState, PacedLimit,
};
use gigawatts::status::render_line;

const NOW_MS: i64 = 1_793_815_800_000;

fn utc_minute(ms: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%d %H:%M', ?1 / 1000, 'unixepoch')",
            [ms],
            |row| row.get(0),
        )
        .unwrap()
}

fn timed(kind: &str, percent: f64, resets_at: &str, pace: Option<LimitPace>) -> PacedLimit {
    PacedLimit {
        limit: Limit {
            kind: kind.to_string(),
            group: None,
            scope: None,
            percent,
            severity: None,
            resets_at: Some(resets_at.to_string()),
            active: true,
            credits: None,
        },
        pace,
    }
}

fn pace(state: PaceState, basis: PaceBasis, provisional: bool) -> Option<LimitPace> {
    Some(LimitPace {
        state,
        basis,
        expected_percent: 50.0,
        window_start_ms: 0,
        window_end_ms: 1_794_441_600_000,
        provisional,
    })
}

fn account(limits: Option<AccountLimits>, cost_usd: f64, today: DollarPace) -> AccountToday {
    AccountToday {
        account: None,
        account_label: "personal".to_string(),
        requests: 3,
        cost_usd,
        pace: today,
        fastest_session: None,
        active_sessions: Vec::new(),
        unpriced_models: Vec::new(),
        adjusted_models: Vec::new(),
        cost_estimated: false,
        limits,
    }
}

fn usual(today_usd: f64, usual_usd: Option<f64>, usual_days: u32) -> DollarPace {
    DollarPace {
        today_usd,
        usual_usd,
        usual_days,
        clock: "12:10".to_string(),
        weekend: false,
        earlier_days_usd: Vec::new(),
    }
}

#[test]
fn status_shows_the_closest_limit_with_its_pace_and_today_against_usual() {
    let limits = AccountLimits {
        source: LimitSource::GetUsage,
        fetched_at_ms: NOW_MS - 3 * 60_000,
        limits: vec![
            timed(
                "session",
                5.0,
                "2026-11-04T23:59:59.945577+00:00",
                pace(PaceState::Even, PaceBasis::Even, false),
            ),
            timed(
                "weekly_all",
                63.0,
                "2026-11-08T02:59:59.945600+00:00",
                pace(PaceState::Over, PaceBasis::Usual, false),
            ),
        ],
    };

    let line = render_line(
        &account(Some(limits), 14.2, usual(14.0, Some(7.0), 20)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: Weekly, all models 63% over your usual pace, resets 2026-11-08 02:59, read 3 minutes ago · today $14.20, 2.0× your usual by 12:10"
    );
}

#[test]
fn monthly_credits_show_their_pace_window_end_and_the_provisional_label() {
    let limits = AccountLimits {
        source: LimitSource::CachedUsageUtilization,
        fetched_at_ms: NOW_MS - 2 * 3_600_000,
        limits: vec![PacedLimit {
            limit: Limit {
                kind: "credits".to_string(),
                group: None,
                scope: None,
                percent: 20.0,
                severity: None,
                resets_at: None,
                active: false,
                credits: Some(Credits {
                    used: 437.6,
                    limit: Some(2200.0),
                    currency: "USD".to_string(),
                }),
            },
            pace: pace(PaceState::Under, PaceBasis::Even, true),
        }],
    };

    let line = render_line(
        &account(Some(limits), 0.0, usual(0.0, None, 0)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: Credits 20% under even pace (provisional until credit_reset is set), resets 2026-11-12 00:00, read 2 hours ago, may not include use outside Claude Code · today $0.00, no usual yet"
    );
}

#[test]
fn an_account_without_readings_still_shows_today_against_a_young_usual() {
    let line = render_line(
        &account(None, 3.0, usual(3.0, Some(2.0), 4)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: no limit readings yet · today $3.00, 1.5× your usual by 12:10 (provisional, 4 of 20 days)"
    );
}

#[test]
fn a_limit_that_reset_since_the_reading_is_not_the_closest() {
    let limits = AccountLimits {
        source: LimitSource::GetUsage,
        fetched_at_ms: NOW_MS - 10 * 3_600_000,
        limits: vec![
            timed("session", 90.0, "2026-11-04T09:00:00Z", None),
            timed(
                "weekly_all",
                60.0,
                "2026-11-08T02:59:59.945600+00:00",
                pace(PaceState::Over, PaceBasis::Even, false),
            ),
        ],
    };

    let line = render_line(
        &account(Some(limits), 0.0, usual(0.0, None, 0)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: Weekly, all models 60% over even pace, resets 2026-11-08 02:59, read 10 hours ago, may not include use outside Claude Code · today $0.00, no usual yet"
    );
}

#[test]
fn when_every_limit_reset_since_the_reading_the_line_says_so() {
    let limits = AccountLimits {
        source: LimitSource::GetUsage,
        fetched_at_ms: NOW_MS - 10 * 3_600_000,
        limits: vec![
            timed("session", 90.0, "2026-11-04T09:00:00Z", None),
            PacedLimit {
                limit: Limit {
                    kind: "credits".to_string(),
                    group: None,
                    scope: None,
                    percent: 95.0,
                    severity: None,
                    resets_at: None,
                    active: false,
                    credits: None,
                },
                pace: None,
            },
        ],
    };

    let line = render_line(
        &account(Some(limits), 0.0, usual(0.0, None, 0)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: limits reset since the last reading, read 10 hours ago · today $0.00, no usual yet"
    );
}

#[test]
fn spend_from_sessions_claude_code_has_not_checked_yet_is_labeled_estimated() {
    let mut open = account(None, 3.0, usual(3.0, Some(2.0), 20));
    open.cost_estimated = true;

    let line = render_line(&open, NOW_MS, &utc_minute);

    assert_eq!(
        line,
        "personal: no limit readings yet · today $3.00, estimated, 1.5× your usual by 12:10"
    );
}
