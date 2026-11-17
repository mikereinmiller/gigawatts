use gigawatts::alerts::alerts;
use gigawatts::contract::{
    AccountLimits, AccountToday, Alert, AlertState, DollarPace, ForecastBasis, Limit,
    LimitForecast, LimitSource, PacedLimit,
};

const NOW_MS: i64 = 1_793_815_800_000;
const IN_2H_40M: &str = "2026-11-04T20:50:00Z";
const IN_2H_40M_MS: i64 = 1_793_825_400_000;

fn limit(kind: &str, scope: Option<&str>, percent: f64) -> PacedLimit {
    PacedLimit {
        limit: Limit {
            kind: kind.to_string(),
            group: None,
            scope: scope.map(str::to_string),
            percent,
            severity: None,
            resets_at: Some(IN_2H_40M.to_string()),
            active: true,
            credits: None,
        },
        pace: None,
        forecast: None,
        this_machine_usd: None,
        elsewhere_usd: None,
        elsewhere_percent: None,
    }
}

fn filling(kind: &str, minutes: i64) -> PacedLimit {
    let mut shown = limit(kind, None, 60.0);
    shown.forecast = Some(LimitForecast {
        full_at_ms: NOW_MS + minutes * 60_000,
        basis: ForecastBasis::Window,
    });
    shown
}

fn account(id: &str, label: &str, fetched_ago_ms: i64, limits: Vec<PacedLimit>) -> AccountToday {
    AccountToday {
        account: Some(id.to_string()),
        account_label: label.to_string(),
        configured_label: None,
        requests: 0,
        cost_usd: 0.0,
        pace: DollarPace {
            today_usd: 0.0,
            usual_usd: None,
            usual_days: 0,
            clock: "18:10".to_string(),
            weekend: false,
            earlier_days_usd: Vec::new(),
            earlier_day_totals_usd: Vec::new(),
            earlier_days: Vec::new(),
        },
        fastest_session: None,
        active_sessions: Vec::new(),
        unpriced_models: Vec::new(),
        adjusted_models: Vec::new(),
        cost_estimated: false,
        limits: Some(AccountLimits {
            source: LimitSource::GetUsage,
            fetched_at_ms: NOW_MS - fetched_ago_ms,
            limits,
        }),
    }
}

#[test]
fn a_limit_at_100_percent_is_a_full_alert_with_when_it_comes_back() {
    let accounts = vec![account(
        "a/o",
        "personal",
        3 * 60_000,
        vec![limit("session", None, 100.0)],
    )];

    assert_eq!(
        alerts(&accounts, NOW_MS),
        vec![Alert {
            account: Some("a/o".to_string()),
            account_label: "personal".to_string(),
            limit: "Session".to_string(),
            state: AlertState::Full,
            at_ms: IN_2H_40M_MS,
        }]
    );
}

#[test]
fn a_limit_on_course_to_fill_is_a_filling_alert_with_when_it_fills() {
    let accounts = vec![account(
        "a/o",
        "personal",
        3 * 60_000,
        vec![filling("session", 80)],
    )];

    let found = alerts(&accounts, NOW_MS);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].state, AlertState::Filling);
    assert_eq!(found[0].at_ms, NOW_MS + 80 * 60_000);
}

#[test]
fn full_alerts_come_first_the_one_back_last_leading_then_the_soonest_to_fill() {
    let accounts = vec![
        account(
            "a/o",
            "personal",
            3 * 60_000,
            vec![filling("weekly_all", 30)],
        ),
        account(
            "b/o",
            "work",
            3 * 60_000,
            vec![
                filling("session", 20),
                limit("weekly_scoped", Some("Fable"), 100.0),
            ],
        ),
    ];
    let mut later = account(
        "c/o",
        "spare",
        3 * 60_000,
        vec![limit("session", None, 100.0)],
    );
    later.limits.as_mut().unwrap().limits[0].limit.resets_at =
        Some("2026-11-04T19:10:00Z".to_string());
    let mut accounts = accounts;
    accounts.push(later);

    let order: Vec<(String, AlertState)> = alerts(&accounts, NOW_MS)
        .into_iter()
        .map(|alert| (alert.limit, alert.state))
        .collect();

    assert_eq!(
        order,
        vec![
            ("Fable".to_string(), AlertState::Full),
            ("Session".to_string(), AlertState::Full),
            ("Session".to_string(), AlertState::Filling),
            ("Weekly".to_string(), AlertState::Filling),
        ]
    );
}

#[test]
fn a_reading_over_an_hour_old_raises_no_filling_alert_but_a_full_one_stays() {
    let accounts = vec![account(
        "a/o",
        "personal",
        2 * 3_600_000,
        vec![filling("session", 80), limit("weekly_all", None, 100.0)],
    )];

    let found = alerts(&accounts, NOW_MS);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].state, AlertState::Full);
}

#[test]
fn a_limit_whose_reset_has_passed_raises_nothing() {
    let accounts = vec![account(
        "a/o",
        "personal",
        3 * 60_000,
        vec![limit("session", None, 100.0)],
    )];

    assert!(alerts(&accounts, IN_2H_40M_MS + 1).is_empty());
}

#[test]
fn a_forecast_that_fell_due_since_the_reading_counts_as_filling_now() {
    let mut overdue = filling("session", 0);
    overdue.forecast.as_mut().unwrap().full_at_ms = NOW_MS - 20 * 60_000;
    let accounts = vec![account("a/o", "personal", 40 * 60_000, vec![overdue])];

    let found = alerts(&accounts, NOW_MS);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].state, AlertState::Filling);
    assert_eq!(found[0].at_ms, NOW_MS);
}

#[test]
fn a_limit_forecast_to_fill_more_than_a_day_out_raises_no_alert() {
    let accounts = vec![account(
        "a/o",
        "personal",
        3 * 60_000,
        vec![
            filling("weekly_all", 24 * 60 + 10),
            filling("session", 24 * 60),
        ],
    )];

    let found = alerts(&accounts, NOW_MS);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].limit, "Session");
}
