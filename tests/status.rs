use gigawatts::contract::{
    AccountLimits, AccountToday, Credits, DollarPace, ForecastBasis, Limit, LimitForecast,
    LimitPace, LimitSource, PaceBasis, PaceState, PacedLimit,
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
        forecast: None,
        this_machine_usd: None,
        elsewhere_usd: None,
        elsewhere_percent: None,
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
        configured_label: None,
        requests: 3,
        cost_usd,
        pace: today,
        fastest_session: None,
        active_sessions: Vec::new(),
        sessions_today: Vec::new(),
        unpriced_models: Vec::new(),
        adjusted_models: Vec::new(),
        cost_estimated: false,
        fast_usd_per_hour: 20.0,
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
        earlier_day_totals_usd: Vec::new(),
        earlier_days: Vec::new(),
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
fn a_limit_on_course_to_fill_before_its_reset_says_when_it_reaches_100_percent() {
    let mut session = timed(
        "session",
        60.0,
        "2026-11-04T19:40:00Z",
        pace(PaceState::Over, PaceBasis::Even, false),
    );
    session.forecast = Some(LimitForecast {
        full_at_ms: NOW_MS + 40 * 60_000,
        basis: ForecastBasis::Window,
    });
    let limits = AccountLimits {
        source: LimitSource::GetUsage,
        fetched_at_ms: NOW_MS - 3 * 60_000,
        limits: vec![session],
    };

    let line = render_line(
        &account(Some(limits), 14.2, usual(14.0, Some(7.0), 20)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: Session 60% over even pace, 100% by 2026-11-04 18:50, resets 2026-11-04 19:40, read 3 minutes ago · today $14.20, 2.0× your usual by 12:10"
    );
}

#[test]
fn a_forecast_from_the_last_30_minutes_says_so() {
    let mut session = session_filling_by_18_50(60.0);
    session.forecast = Some(LimitForecast {
        full_at_ms: NOW_MS + 40 * 60_000,
        basis: ForecastBasis::Recent,
    });

    let line = line_for(vec![session]);

    assert!(
        line.contains("100% by 2026-11-04 18:50 at the last 30m's pace, resets"),
        "{line}"
    );
}

fn session_filling_by_18_50(percent: f64) -> PacedLimit {
    let mut session = timed(
        "session",
        percent,
        "2026-11-04T19:40:00Z",
        pace(PaceState::Over, PaceBasis::Even, false),
    );
    session.forecast = Some(LimitForecast {
        full_at_ms: NOW_MS + 40 * 60_000,
        basis: ForecastBasis::Window,
    });
    session
}

fn line_for(limits: Vec<PacedLimit>) -> String {
    let limits = AccountLimits {
        source: LimitSource::GetUsage,
        fetched_at_ms: NOW_MS - 3 * 60_000,
        limits,
    };
    render_line(
        &account(Some(limits), 14.2, usual(14.0, Some(7.0), 20)),
        NOW_MS,
        &utc_minute,
    )
}

#[test]
fn the_fullest_limit_leads_and_another_limit_on_course_to_fill_is_named_after_it() {
    let line = line_for(vec![
        timed(
            "weekly_all",
            63.0,
            "2026-11-08T02:59:59.945600+00:00",
            pace(PaceState::Under, PaceBasis::Usual, false),
        ),
        session_filling_by_18_50(40.0),
    ]);

    assert_eq!(
        line,
        "personal: Weekly, all models 63% under your usual pace, resets 2026-11-08 02:59, Session 100% by 2026-11-04 18:50, read 3 minutes ago · today $14.20, 2.0× your usual by 12:10"
    );
}

#[test]
fn a_full_limit_stays_first_when_another_limit_is_on_course_to_fill() {
    let line = line_for(vec![
        session_filling_by_18_50(50.0),
        timed(
            "weekly_all",
            100.0,
            "2026-11-08T02:59:59.945600+00:00",
            pace(PaceState::Over, PaceBasis::Usual, false),
        ),
    ]);

    assert_eq!(
        line,
        "personal: Weekly, all models 100% over your usual pace, resets 2026-11-08 02:59, Session 100% by 2026-11-04 18:50, read 3 minutes ago · today $14.20, 2.0× your usual by 12:10"
    );
}

fn weekly_filling_at(full_at_ms: i64) -> PacedLimit {
    let mut weekly = timed(
        "weekly_all",
        63.0,
        "2026-11-08T02:59:59.945600+00:00",
        pace(PaceState::Over, PaceBasis::Usual, false),
    );
    weekly.forecast = Some(LimitForecast {
        full_at_ms,
        basis: ForecastBasis::Window,
    });
    weekly
}

#[test]
fn the_fullest_limit_and_an_earlier_filling_limit_both_say_when_they_reach_100_percent() {
    let line = line_for(vec![
        weekly_filling_at(NOW_MS + 3 * 3_600_000),
        session_filling_by_18_50(40.0),
    ]);

    assert_eq!(
        line,
        "personal: Weekly, all models 63% over your usual pace, 100% by 2026-11-04 21:10, resets 2026-11-08 02:59, Session 100% by 2026-11-04 18:50, read 3 minutes ago · today $14.20, 2.0× your usual by 12:10"
    );
}

#[test]
fn the_fullest_limit_filling_first_is_not_named_twice() {
    let line = line_for(vec![
        weekly_filling_at(NOW_MS + 20 * 60_000),
        session_filling_by_18_50(40.0),
    ]);

    assert_eq!(
        line,
        "personal: Weekly, all models 63% over your usual pace, 100% by 2026-11-04 18:30, resets 2026-11-08 02:59, read 3 minutes ago · today $14.20, 2.0× your usual by 12:10"
    );
}

#[test]
fn a_credit_forecast_on_the_default_reset_has_no_caveat() {
    let limits = AccountLimits {
        source: LimitSource::CachedUsageUtilization,
        fetched_at_ms: NOW_MS - 3 * 60_000,
        limits: vec![PacedLimit {
            limit: Limit {
                kind: "credits".to_string(),
                group: None,
                scope: None,
                percent: 20.0,
                severity: None,
                resets_at: None,
                active: false,
                credits: None,
            },
            pace: pace(PaceState::Over, PaceBasis::Even, true),
            forecast: Some(LimitForecast {
                full_at_ms: NOW_MS + 40 * 60_000,
                basis: ForecastBasis::Window,
            }),
            this_machine_usd: None,
            elsewhere_usd: None,
            elsewhere_percent: None,
        }],
    };

    let line = render_line(
        &account(Some(limits), 14.2, usual(14.0, Some(7.0), 20)),
        NOW_MS,
        &utc_minute,
    );

    assert!(
        line.contains(", 100% by 2026-11-04 18:50, resets "),
        "{line}"
    );
}

#[test]
fn monthly_credits_on_the_default_reset_show_their_pace_and_window_end() {
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
            forecast: None,
            this_machine_usd: None,
            elsewhere_usd: None,
            elsewhere_percent: None,
        }],
    };

    let line = render_line(
        &account(Some(limits), 0.0, usual(0.0, None, 0)),
        NOW_MS,
        &utc_minute,
    );

    assert_eq!(
        line,
        "personal: Credits 20% under even pace, resets 2026-11-12 00:00, read 2 hours ago · today $0.00, no usual yet"
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
        "personal: no limit readings yet · today $3.00, 1.5× your usual by 12:10"
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
        "personal: Weekly, all models 60% over even pace, resets 2026-11-08 02:59, read 10 hours ago · today $0.00, no usual yet"
    );
}

#[test]
fn a_limit_that_reset_since_its_reading_is_not_named_as_filling() {
    let mut session = session_filling_by_18_50(40.0);
    session.limit.resets_at = Some("2026-11-04T18:00:00Z".to_string());

    let line = line_for(vec![
        timed(
            "weekly_all",
            30.0,
            "2026-11-08T02:59:59.945600+00:00",
            pace(PaceState::Under, PaceBasis::Usual, false),
        ),
        session,
    ]);

    assert!(!line.contains("100%"), "{line}");
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
                forecast: None,
                this_machine_usd: None,
                elsewhere_usd: None,
                elsewhere_percent: None,
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
fn spend_from_sessions_claude_code_has_not_checked_yet_is_not_labeled_estimated() {
    let mut open = account(None, 3.0, usual(3.0, Some(2.0), 20));
    open.cost_estimated = true;

    let line = render_line(&open, NOW_MS, &utc_minute);

    assert_eq!(
        line,
        "personal: no limit readings yet · today $3.00, 1.5× your usual by 12:10"
    );
}
