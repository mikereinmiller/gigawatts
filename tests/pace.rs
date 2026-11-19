use std::fs;
use std::path::Path;

use gigawatts::accounts::Attribution;
use gigawatts::config::{Config, epoch_ms_from_text};
use gigawatts::contract::{
    DollarPace, ForecastBasis, Limit, LimitForecast, LimitPace, PaceBasis, PaceState, SessionSpend,
    Share,
};
use gigawatts::ingest::ingest_tree;
use gigawatts::pace::{
    active_sessions, dollar_pace, fastest_session, limit_forecast, limit_pace, sessions_today,
};
use gigawatts::pricing::PriceTable;
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::get_usage;
use gigawatts::sources::git::Resolver;
use gigawatts::store::{LimitPoint, Store};

const WORK: &str = "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000e0";

fn work() -> Account {
    WORK.parse().unwrap()
}

fn utc_ms(timestamp: &str) -> i64 {
    epoch_ms_from_text(timestamp).unwrap()
}

fn enterprise_credits() -> Limit {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/usage/enterprise/get-usage.json");
    let usage = get_usage::parse_usage(&fs::read_to_string(path).unwrap()).unwrap();
    usage
        .limits
        .into_iter()
        .find(|limit| limit.kind == "credits")
        .unwrap()
}

fn local_to_utc(local: &str) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, 'utc')",
            [local],
            |row| row.get(0),
        )
        .unwrap()
}

fn local_ms(local: &str) -> i64 {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT CAST(unixepoch(?1, 'utc') AS INTEGER) * 1000",
            [local],
            |row| row.get(0),
        )
        .unwrap()
}

// Opus 5.5 output is $20 per million tokens, so 50,000 output tokens cost $1.
fn spend_line(n: usize, local: &str, dollars: u64) -> String {
    let timestamp = local_to_utc(local);
    let output = dollars * 50_000;
    format!(
        r#"{{"type":"assistant","sessionId":"s1","requestId":"r{n}","timestamp":"{timestamp}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":0,"output_tokens":{output}}}}}}}"#
    ) + "\n"
}

fn store_with_spend(spends: &[(&str, u64)]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines: String = spends
        .iter()
        .enumerate()
        .map(|(n, (local, dollars))| spend_line(n, local, *dollars))
        .collect();
    fs::write(projects.join("s1.jsonl"), lines).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &projects,
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();
    (dir, store)
}

#[test]
fn dollar_pace_compares_today_with_the_median_of_the_last_20_workdays_by_this_clock_time() {
    let workdays_newest_first = [
        "2026-11-03",
        "2026-11-02",
        "2026-10-30",
        "2026-10-29",
        "2026-10-28",
        "2026-10-26",
        "2026-10-23",
        "2026-10-22",
        "2026-10-21",
        "2026-10-20",
        "2026-10-19",
        "2026-10-16",
        "2026-10-15",
        "2026-10-14",
        "2026-10-13",
        "2026-10-12",
        "2026-10-09",
        "2026-10-08",
        "2026-10-07",
        "2026-10-06",
    ];
    let mut spends: Vec<(String, u64)> = Vec::new();
    for (n, day) in workdays_newest_first.iter().enumerate() {
        spends.push((format!("{day} 13:00:00"), n as u64 + 1));
        spends.push((format!("{day} 15:00:00"), 100));
    }
    spends.push(("2026-10-05 13:00:00".to_string(), 50));
    spends.push(("2026-10-31 13:00:00".to_string(), 40));
    spends.push(("2026-11-01 13:00:00".to_string(), 40));
    spends.push(("2026-11-04 09:00:00".to_string(), 21));
    let spends: Vec<(&str, u64)> = spends
        .iter()
        .map(|(local, dollars)| (local.as_str(), *dollars))
        .collect();
    let (_dir, store) = store_with_spend(&spends);

    let pace = dollar_pace(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(
        pace,
        DollarPace {
            today_usd: 21.0,
            usual_usd: Some(10.5),
            usual_days: 20,
            clock: "14:00".to_string(),
            weekend: false,
            earlier_days_usd: vec![
                20.0, 19.0, 18.0, 17.0, 16.0, 15.0, 14.0, 13.0, 12.0, 11.0, 10.0, 9.0, 8.0, 7.0,
                6.0, 5.0, 4.0, 3.0, 2.0, 1.0,
            ],
            earlier_day_totals_usd: vec![
                120.0, 119.0, 118.0, 117.0, 116.0, 115.0, 114.0, 113.0, 112.0, 111.0, 110.0, 109.0,
                108.0, 107.0, 106.0, 105.0, 104.0, 103.0, 102.0, 101.0,
            ],
            earlier_days: workdays_newest_first
                .iter()
                .rev()
                .map(|day| day.to_string())
                .collect(),
        }
    );
    assert_eq!(pace.times_usual(), Some(2.0));
}

#[test]
fn monthly_credits_pace_from_the_credit_reset_setting_and_stay_provisional_until_it_is_set() {
    let credits = enterprise_credits();
    assert_eq!(credits.percent, 20.0);
    let mid_month = utc_ms("2026-11-16T00:00:00Z");
    let unset = Config::parse("", Path::new("/home/me")).unwrap();

    let pace = limit_pace(
        &credits,
        mid_month,
        mid_month,
        &[],
        unset.account(&work()).credit_reset.as_ref(),
    );

    assert_eq!(
        pace,
        Some(LimitPace {
            state: PaceState::Under,
            basis: PaceBasis::Even,
            expected_percent: 50.0,
            window_start_ms: utc_ms("2026-11-01T00:00:00Z"),
            window_end_ms: utc_ms("2026-12-01T00:00:00Z"),
            provisional: true,
        })
    );

    let set = Config::parse(
        &format!("[accounts.\"{WORK}\"]\ncredit_reset = 2026-10-10T00:00:00Z\n"),
        Path::new("/home/me"),
    )
    .unwrap();

    let pace = limit_pace(
        &credits,
        mid_month,
        mid_month,
        &[],
        set.account(&work()).credit_reset.as_ref(),
    );

    assert_eq!(
        pace,
        Some(LimitPace {
            state: PaceState::Even,
            basis: PaceBasis::Even,
            expected_percent: 20.0,
            window_start_ms: utc_ms("2026-11-10T00:00:00Z"),
            window_end_ms: utc_ms("2026-12-10T00:00:00Z"),
            provisional: false,
        })
    );
}

fn timed_limit(kind: &str, percent: f64, resets_at: &str) -> Limit {
    Limit {
        kind: kind.to_string(),
        group: None,
        scope: None,
        percent,
        severity: None,
        resets_at: Some(resets_at.to_string()),
        active: true,
        credits: None,
    }
}

fn weekly_point(fetched_at: &str, percent: f64) -> LimitPoint {
    LimitPoint {
        kind: "weekly_all".to_string(),
        scope: None,
        fetched_at_ms: utc_ms(fetched_at),
        percent,
        resets_at: None,
    }
}

#[test]
fn the_5_hour_limit_compares_with_even_pace_through_its_window() {
    let limit = timed_limit("session", 5.0, "2026-11-04T23:59:59.945577+00:00");

    let pace = limit_pace(
        &limit,
        utc_ms("2026-11-04T20:59:59.945Z"),
        utc_ms("2026-11-04T20:59:59.945Z"),
        &[],
        None,
    )
    .unwrap();

    assert_eq!(pace.basis, PaceBasis::Even);
    assert_eq!(pace.state, PaceState::Under);
    assert_eq!(pace.window_start_ms, utc_ms("2026-11-04T18:59:59.945Z"));
    assert!((pace.expected_percent - 40.0).abs() < 1e-9);
}

fn forecast_at(limit: &Limit, fetched_at: &str) -> Option<LimitForecast> {
    let fetched_at_ms = utc_ms(fetched_at);
    let pace = limit_pace(limit, fetched_at_ms, fetched_at_ms, &[], None).unwrap();
    limit_forecast(limit, &pace, fetched_at_ms, &[])
}

fn session_point(fetched_at: &str, percent: f64) -> LimitPoint {
    LimitPoint {
        kind: "session".to_string(),
        scope: None,
        fetched_at_ms: utc_ms(fetched_at),
        percent,
        resets_at: None,
    }
}

fn forecast_with(limit: &Limit, fetched_at: &str, recent: &[LimitPoint]) -> Option<LimitForecast> {
    let fetched_at_ms = utc_ms(fetched_at);
    let pace = limit_pace(limit, fetched_at_ms, fetched_at_ms, &[], None).unwrap();
    limit_forecast(limit, &pace, fetched_at_ms, recent)
}

fn window_forecast(full_at: &str) -> Option<LimitForecast> {
    Some(LimitForecast {
        full_at_ms: utc_ms(full_at),
        basis: ForecastBasis::Window,
    })
}

#[test]
fn a_burst_in_the_last_30_minutes_forecasts_100_percent_when_the_window_rate_resets_first() {
    let limit = timed_limit("session", 40.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T12:00:00Z",
        &[session_point("2026-11-11T11:20:00Z", 25.0)],
    );

    assert_eq!(
        forecast,
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-11T14:00:00Z"),
            basis: ForecastBasis::Recent,
        })
    );
}

#[test]
fn the_window_rate_wins_when_it_reaches_100_percent_first() {
    let limit = timed_limit("session", 60.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T12:00:00Z",
        &[session_point("2026-11-11T11:20:00Z", 50.0)],
    );

    assert_eq!(forecast, window_forecast("2026-11-11T13:20:00Z"));
}

#[test]
fn no_change_in_the_last_30_minutes_falls_back_to_the_window_rate() {
    let limit = timed_limit("session", 60.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T12:00:00Z",
        &[session_point("2026-11-11T11:00:00Z", 60.0)],
    );

    assert_eq!(forecast, window_forecast("2026-11-11T13:20:00Z"));
}

#[test]
fn there_is_no_recent_rate_until_30_minutes_of_the_window_have_passed() {
    let limit = timed_limit("session", 20.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T10:20:00Z",
        &[session_point("2026-11-11T09:40:00Z", 0.0)],
    );

    assert_eq!(forecast, None);
}

#[test]
fn a_reading_from_the_previous_window_counts_as_0_percent() {
    let limit = timed_limit("session", 30.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T10:40:00Z",
        &[session_point("2026-11-11T09:55:00Z", 70.0)],
    );

    assert_eq!(
        forecast,
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-11T11:50:00Z"),
            basis: ForecastBasis::Recent,
        })
    );
}

#[test]
fn a_changed_reading_followed_by_a_fetch_gap_gives_no_recent_rate() {
    let limit = timed_limit("session", 40.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T12:10:00Z",
        &[session_point("2026-11-11T11:00:00Z", 10.0)],
    );

    assert_eq!(forecast, None);
}

#[test]
fn a_reading_too_old_to_be_in_effect_30_minutes_ago_gives_no_recent_rate() {
    let limit = timed_limit("session", 40.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T12:00:00Z",
        &[session_point("2026-11-11T10:00:00Z", 10.0)],
    );

    assert_eq!(forecast, None);
}

#[test]
fn a_one_point_tick_on_a_weekly_limit_forecasts_nothing() {
    let limit = timed_limit("weekly_all", 20.0, "2026-11-08T04:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-03T04:00:00Z",
        &[weekly_point("2026-11-03T03:25:00Z", 19.0)],
    );

    assert_eq!(forecast, None);
}

#[test]
fn a_one_point_rise_in_30_minutes_forecasts_nothing_when_both_rates_reset_first() {
    let limit = timed_limit("session", 10.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_with(
        &limit,
        "2026-11-11T12:00:00Z",
        &[session_point("2026-11-11T11:20:00Z", 9.0)],
    );

    assert_eq!(forecast, None);
}

#[test]
fn a_forecast_without_a_basis_reads_as_the_window_rate() {
    let forecast: LimitForecast = serde_json::from_str(r#"{"full_at_ms": 1}"#).unwrap();

    assert_eq!(forecast.basis, ForecastBasis::Window);
}

#[test]
fn a_session_limit_at_60_percent_two_hours_in_reaches_100_percent_80_minutes_later() {
    let limit = timed_limit("session", 60.0, "2026-11-11T15:00:00Z");

    let forecast = forecast_at(&limit, "2026-11-11T12:00:00Z");

    assert_eq!(
        forecast,
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-11T13:20:00Z"),
            basis: ForecastBasis::Window,
        })
    );
}

#[test]
fn a_limit_has_no_forecast_before_a_tenth_of_its_window_has_passed() {
    let limit = timed_limit("session", 50.0, "2026-11-11T15:00:00Z");

    assert_eq!(forecast_at(&limit, "2026-11-11T10:29:59Z"), None);
    assert_eq!(
        forecast_at(&limit, "2026-11-11T10:30:00Z"),
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-11T11:00:00Z"),
            basis: ForecastBasis::Window,
        })
    );
}

#[test]
fn a_limit_at_0_percent_has_no_forecast() {
    let limit = timed_limit("session", 0.0, "2026-11-11T15:00:00Z");

    assert_eq!(forecast_at(&limit, "2026-11-11T12:00:00Z"), None);
}

#[test]
fn a_limit_that_reaches_100_percent_at_or_after_its_reset_has_no_forecast() {
    let after = timed_limit("session", 30.0, "2026-11-11T15:00:00Z");
    let at = timed_limit("session", 40.0, "2026-11-11T15:00:00Z");

    assert_eq!(forecast_at(&after, "2026-11-11T12:00:00Z"), None);
    assert_eq!(forecast_at(&at, "2026-11-11T12:00:00Z"), None);
}

#[test]
fn a_limit_already_at_100_percent_has_no_forecast() {
    let limit = timed_limit("session", 100.0, "2026-11-11T15:00:00Z");

    assert_eq!(forecast_at(&limit, "2026-11-11T12:00:00Z"), None);
}

#[test]
fn a_weekly_limit_at_40_percent_two_days_in_reaches_100_percent_on_day_five() {
    let limit = timed_limit("weekly_all", 40.0, "2026-11-08T04:00:00Z");

    assert_eq!(
        forecast_at(&limit, "2026-11-03T04:00:00Z"),
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-06T04:00:00Z"),
            basis: ForecastBasis::Window,
        })
    );
}

#[test]
fn monthly_credits_forecast_from_their_window_with_or_without_credit_reset() {
    let credits = enterprise_credits();
    let unset_fetch = utc_ms("2026-11-04T00:00:00Z");
    let unset = limit_pace(&credits, unset_fetch, unset_fetch, &[], None).unwrap();

    assert!(unset.provisional);
    assert_eq!(
        limit_forecast(&credits, &unset, unset_fetch, &[]),
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-16T00:00:00Z"),
            basis: ForecastBasis::Window,
        })
    );

    let set = Config::parse(
        &format!("[accounts.\"{WORK}\"]\ncredit_reset = 2026-10-10T00:00:00Z\n"),
        Path::new("/home/me"),
    )
    .unwrap();
    let set_fetch = utc_ms("2026-11-13T00:00:00Z");
    let set_pace = limit_pace(
        &credits,
        set_fetch,
        set_fetch,
        &[],
        set.account(&work()).credit_reset.as_ref(),
    )
    .unwrap();

    assert_eq!(
        limit_forecast(&credits, &set_pace, set_fetch, &[]),
        Some(LimitForecast {
            full_at_ms: utc_ms("2026-11-25T00:00:00Z"),
            basis: ForecastBasis::Window,
        })
    );
}

#[test]
fn a_weekly_limit_uses_even_pace_until_four_full_weeks_of_readings_exist() {
    let limit = timed_limit("weekly_all", 33.0, "2026-11-08T03:59:59.664575+00:00");
    let wednesday = utc_ms("2026-11-04T03:59:59Z");
    let three_weeks = [weekly_point("2026-10-12T00:00:00Z", 10.0)];

    let pace = limit_pace(&limit, wednesday, wednesday, &three_weeks, None).unwrap();

    assert_eq!(pace.basis, PaceBasis::Even);
    assert_eq!(pace.state, PaceState::Under);

    let one_old_reading = [weekly_point("2026-06-01T00:00:00Z", 10.0)];

    let pace = limit_pace(&limit, wednesday, wednesday, &one_old_reading, None).unwrap();

    assert_eq!(pace.basis, PaceBasis::Even);
}

#[test]
fn a_weekly_limit_compares_with_the_median_of_the_last_four_weeks_at_this_point() {
    let limit = timed_limit("weekly_all", 45.0, "2026-11-08T03:59:59.664575+00:00");
    let wednesday = utc_ms("2026-11-04T03:59:59Z");
    let history = [
        weekly_point("2026-10-03T12:00:00Z", 90.0),
        weekly_point("2026-10-05T00:00:00Z", 20.0),
        weekly_point("2026-10-08T00:00:00Z", 70.0),
        weekly_point("2026-10-12T00:00:00Z", 60.0),
        weekly_point("2026-10-22T00:00:00Z", 30.0),
        weekly_point("2026-10-27T00:00:00Z", 50.0),
        weekly_point("2026-11-03T00:00:00Z", 10.0),
    ];

    let pace = limit_pace(&limit, wednesday, wednesday, &history, None).unwrap();

    assert_eq!(pace.basis, PaceBasis::Usual);
    assert_eq!(pace.expected_percent, 35.0);
    assert_eq!(pace.state, PaceState::Over);
}

fn session_spend_line(session: &str, request: &str, local: &str, dollars: u64) -> String {
    let timestamp = local_to_utc(local);
    let output = dollars * 50_000;
    format!(
        r#"{{"type":"assistant","sessionId":"{session}","requestId":"{request}","timestamp":"{timestamp}","cwd":"/work/orchard","gitBranch":"ab/carts","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":0,"output_tokens":{output}}}}}}}"#
    ) + "\n"
}

#[test]
fn the_fastest_session_spent_most_in_the_last_15_minutes_counting_its_subagents() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(projects.join("s2").join("subagents")).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        session_spend_line("s1", "a1", "2026-11-04 13:00:00", 10)
            + &session_spend_line("s1", "a2", "2026-11-04 13:55:00", 4),
    )
    .unwrap();
    fs::write(
        projects.join("s2.jsonl"),
        session_spend_line("s2", "b1", "2026-11-04 13:50:00", 3),
    )
    .unwrap();
    fs::write(
        projects.join("s2").join("subagents").join("agent-x.jsonl"),
        session_spend_line("s2", "b2", "2026-11-04 13:58:00", 2),
    )
    .unwrap();
    fs::write(
        projects.join("s3.jsonl"),
        session_spend_line("s3", "c1", "2026-11-04 13:40:00", 20),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let fastest = fastest_session(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(
        fastest,
        Some(SessionSpend {
            session: "s2".to_string(),
            label: "s2".to_string(),
            repo: "orchard".to_string(),
            branch: Some("ab/carts".to_string()),
            last_15_minutes_usd: 5.0,
            total_usd: 5.0,
            last_active_ms: local_ms("2026-11-04 13:58:00"),
            today_usd: 5.0,
            top_driver: Some(Share {
                label: "no skill".to_string(),
                share: 1.0,
            }),
            models: vec![Share {
                label: "claude-opus-5-5".to_string(),
                share: 1.0,
            }],
        })
    );
}

#[test]
fn an_active_session_counts_what_it_spent_today_apart_from_its_total() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        session_spend_line("s1", "a1", "2026-11-03 22:00:00", 7)
            + &session_spend_line("s1", "a2", "2026-11-04 09:00:00", 10)
            + &session_spend_line("s1", "a3", "2026-11-04 13:55:00", 4),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let active = active_sessions(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(active[0].total_usd, 21.0);
    assert_eq!(active[0].today_usd, 14.0);
}

#[test]
fn an_active_session_names_the_skill_or_subagent_and_the_models_that_spent_most_today() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(&projects).unwrap();
    let with = |line: String, field: &str, value: &str| {
        line.replace(
            r#""gitBranch":"ab/carts","#,
            &format!(r#""gitBranch":"ab/carts","{field}":"{value}","#),
        )
    };
    fs::write(
        projects.join("s1.jsonl"),
        with(
            session_spend_line("s1", "a0", "2026-11-03 22:00:00", 50),
            "attributionSkill",
            "implement",
        ) + &with(
            session_spend_line("s1", "a1", "2026-11-04 09:00:00", 6),
            "attributionSkill",
            "review",
        ) + &with(
            session_spend_line("s1", "a2", "2026-11-04 10:00:00", 2),
            "attributionAgent",
            "code-reviewer",
        ) + &session_spend_line("s1", "a3", "2026-11-04 13:55:00", 4)
            .replace("claude-opus-5-5", "claude-sonnet-5-5"),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let active = active_sessions(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(active[0].today_usd, 10.0);
    assert_eq!(
        active[0].top_driver,
        Some(Share {
            label: "review".to_string(),
            share: 0.6
        })
    );
    assert_eq!(
        active[0].models,
        vec![
            Share {
                label: "claude-opus-5-5".to_string(),
                share: 0.8
            },
            Share {
                label: "claude-sonnet-5-5".to_string(),
                share: 0.2
            }
        ]
    );
}

#[test]
fn active_sessions_are_every_session_written_in_the_last_15_minutes_fastest_first() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        session_spend_line("s1", "a1", "2026-11-04 13:00:00", 10)
            + &session_spend_line("s1", "a2", "2026-11-04 13:55:00", 4),
    )
    .unwrap();
    fs::write(
        projects.join("s2.jsonl"),
        session_spend_line("s2", "b1", "2026-11-04 13:50:00", 5),
    )
    .unwrap();
    fs::write(
        projects.join("s3.jsonl"),
        session_spend_line("s3", "c1", "2026-11-04 13:40:00", 20),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let active = active_sessions(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    let spend =
        |session: &str, last_15_minutes_usd: f64, total_usd: f64, last: &str| SessionSpend {
            session: session.to_string(),
            label: session.to_string(),
            repo: "orchard".to_string(),
            branch: Some("ab/carts".to_string()),
            last_15_minutes_usd,
            total_usd,
            last_active_ms: local_ms(last),
            today_usd: total_usd,
            top_driver: Some(Share {
                label: "no skill".to_string(),
                share: 1.0,
            }),
            models: vec![Share {
                label: "claude-opus-5-5".to_string(),
                share: 1.0,
            }],
        };
    assert_eq!(
        active,
        vec![
            spend("s2", 5.0, 5.0, "2026-11-04 13:50:00"),
            spend("s1", 4.0, 14.0, "2026-11-04 13:55:00")
        ]
    );
}

#[test]
fn a_session_with_only_unpriced_requests_in_the_last_15_minutes_counts_as_spending_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        session_spend_line("s1", "a1", "2026-11-04 13:50:00", 1)
            .replace("claude-opus-5-5", "claude-opus-9-9"),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let fastest = fastest_session(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(
        fastest,
        Some(SessionSpend {
            session: "s1".to_string(),
            label: "s1".to_string(),
            repo: "orchard".to_string(),
            branch: Some("ab/carts".to_string()),
            last_15_minutes_usd: 0.0,
            total_usd: 0.0,
            last_active_ms: local_ms("2026-11-04 13:50:00"),
            today_usd: 0.0,
            top_driver: Some(Share {
                label: "no skill".to_string(),
                share: 0.0,
            }),
            models: vec![Share {
                label: "claude-opus-9-9".to_string(),
                share: 0.0,
            }],
        })
    );
}

#[test]
fn a_credit_reset_on_the_31st_with_a_time_zone_moves_to_the_last_day_of_shorter_months() {
    let credits = enterprise_credits();
    let set = Config::parse(
        &format!("[accounts.\"{WORK}\"]\ncredit_reset = 2026-01-31T00:00:00-05:00\n"),
        Path::new("/home/me"),
    )
    .unwrap();
    let reset = set.account(&work()).credit_reset;

    let february = limit_pace(
        &credits,
        utc_ms("2027-02-15T00:00:00Z"),
        utc_ms("2027-02-15T00:00:00Z"),
        &[],
        reset.as_ref(),
    )
    .unwrap();

    assert_eq!(february.window_start_ms, utc_ms("2027-01-31T05:00:00Z"));
    assert_eq!(february.window_end_ms, utc_ms("2027-02-28T05:00:00Z"));

    let new_year = limit_pace(
        &credits,
        utc_ms("2027-01-01T03:00:00Z"),
        utc_ms("2027-01-01T03:00:00Z"),
        &[],
        reset.as_ref(),
    )
    .unwrap();

    assert_eq!(new_year.window_start_ms, utc_ms("2026-12-31T05:00:00Z"));
    assert_eq!(new_year.window_end_ms, utc_ms("2027-01-31T05:00:00Z"));

    let before_the_reset = limit_pace(
        &credits,
        utc_ms("2026-12-31T04:59:00Z"),
        utc_ms("2026-12-31T04:59:00Z"),
        &[],
        reset.as_ref(),
    )
    .unwrap();

    assert_eq!(
        before_the_reset.window_start_ms,
        utc_ms("2026-11-30T05:00:00Z")
    );
    assert_eq!(
        before_the_reset.window_end_ms,
        utc_ms("2026-12-31T05:00:00Z")
    );
}

#[test]
fn a_credit_reading_from_before_this_month_has_no_pace() {
    let credits = enterprise_credits();

    let pace = limit_pace(
        &credits,
        utc_ms("2026-10-25T00:00:00Z"),
        utc_ms("2026-11-16T00:00:00Z"),
        &[],
        None,
    );

    assert_eq!(pace, None);
}

#[test]
fn an_active_session_counts_its_last_15_minutes_its_total_and_when_it_was_last_active() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        session_spend_line("s1", "a0", "2026-11-04 12:58:00", 9)
            + &session_spend_line("s1", "a1", "2026-11-04 13:02:00", 1)
            + &session_spend_line("s1", "a2", "2026-11-04 13:07:00", 2)
            + &session_spend_line("s1", "a3", "2026-11-04 13:30:00", 3)
            + &session_spend_line("s1", "a4", "2026-11-04 13:59:00", 4),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let active = active_sessions(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(active[0].last_15_minutes_usd, 4.0);
    assert_eq!(active[0].total_usd, 19.0);
    assert_eq!(active[0].last_active_ms, local_ms("2026-11-04 13:59:00"));
}

#[test]
fn a_weekly_limit_level_with_the_clock_forecasts_nothing() {
    let limit = timed_limit("weekly_all", 12.0, "2026-11-08T04:00:00Z");

    assert_eq!(forecast_at(&limit, "2026-11-01T23:59:00Z"), None);
}

#[test]
fn a_limit_forecasts_only_once_it_runs_more_than_5_points_ahead_of_the_clock() {
    let level = timed_limit("session", 45.0, "2026-11-11T15:00:00Z");
    let ahead = timed_limit("session", 46.0, "2026-11-11T15:00:00Z");

    assert_eq!(forecast_at(&level, "2026-11-11T12:00:00Z"), None);
    assert_eq!(
        forecast_at(&ahead, "2026-11-11T12:00:00Z"),
        window_forecast("2026-11-11T14:20:52.174Z")
    );
}

#[test]
fn a_weekly_limit_within_5_points_of_full_forecasts_even_when_level_with_the_clock() {
    let limit = timed_limit("weekly_all", 99.0, "2026-11-08T04:00:00Z");

    assert_eq!(
        forecast_at(&limit, "2026-11-07T19:36:00Z"),
        window_forecast("2026-11-07T21:12:43.636Z")
    );
}

fn store_with_sessions(files: &[(&str, String)]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects").join("-work-orchard");
    fs::create_dir_all(&projects).unwrap();
    for (name, lines) in files {
        fs::write(projects.join(format!("{name}.jsonl")), lines).unwrap();
    }
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &dir.path().join("projects"),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();
    (dir, store)
}

#[test]
fn a_session_quiet_for_two_hours_is_in_sessions_today_but_not_active() {
    let (_dir, store) = store_with_sessions(&[
        (
            "s1",
            session_spend_line("s1", "a1", "2026-11-04 11:30:00", 6)
                + &session_spend_line("s1", "a2", "2026-11-04 12:00:00", 2),
        ),
        (
            "s2",
            session_spend_line("s2", "b1", "2026-11-04 13:55:00", 3),
        ),
    ]);
    let now = local_ms("2026-11-04 14:00:00");

    let today = sessions_today(&store, None, now).unwrap();
    let active = active_sessions(&store, None, now).unwrap();

    let spend =
        |session: &str, last_15_minutes_usd: f64, today_usd: f64, last: &str| SessionSpend {
            session: session.to_string(),
            label: session.to_string(),
            repo: "orchard".to_string(),
            branch: Some("ab/carts".to_string()),
            last_15_minutes_usd,
            total_usd: today_usd,
            last_active_ms: local_ms(last),
            today_usd,
            top_driver: Some(Share {
                label: "no skill".to_string(),
                share: 1.0,
            }),
            models: vec![Share {
                label: "claude-opus-5-5".to_string(),
                share: 1.0,
            }],
        };
    assert_eq!(
        today,
        vec![
            spend("s1", 0.0, 8.0, "2026-11-04 12:00:00"),
            spend("s2", 3.0, 3.0, "2026-11-04 13:55:00"),
        ]
    );
    assert_eq!(
        active
            .iter()
            .map(|s| s.session.as_str())
            .collect::<Vec<_>>(),
        vec!["s2"]
    );
}

#[test]
fn a_session_running_past_midnight_shows_only_todays_spend_in_sessions_today() {
    let (_dir, store) = store_with_sessions(&[(
        "s1",
        session_spend_line("s1", "a1", "2026-11-03 23:50:00", 7)
            + &session_spend_line("s1", "a2", "2026-11-04 00:10:00", 3),
    )]);

    let today = sessions_today(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(today.len(), 1);
    assert_eq!(today[0].today_usd, 3.0);
    assert_eq!(today[0].total_usd, 10.0);
    assert_eq!(today[0].last_15_minutes_usd, 0.0);
}

#[test]
fn a_session_active_only_yesterday_is_not_in_sessions_today() {
    let (_dir, store) = store_with_sessions(&[(
        "s1",
        session_spend_line("s1", "a1", "2026-11-03 23:58:00", 7),
    )]);

    let today = sessions_today(&store, None, local_ms("2026-11-04 00:05:00")).unwrap();

    assert_eq!(today, Vec::new());
}

#[test]
fn a_finished_session_names_the_skill_and_models_that_spent_most_today() {
    let with_skill = |line: String, skill: &str| {
        line.replace(
            r#""gitBranch":"ab/carts","#,
            &format!(r#""gitBranch":"ab/carts","attributionSkill":"{skill}","#),
        )
    };
    let (_dir, store) = store_with_sessions(&[(
        "s1",
        with_skill(
            session_spend_line("s1", "a1", "2026-11-04 09:00:00", 6),
            "review",
        ) + &session_spend_line("s1", "a2", "2026-11-04 10:00:00", 4)
            .replace("claude-opus-5-5", "claude-sonnet-5-5"),
    )]);

    let today = sessions_today(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(
        today[0].top_driver,
        Some(Share {
            label: "review".to_string(),
            share: 0.75
        })
    );
    assert_eq!(
        today[0].models,
        vec![
            Share {
                label: "claude-opus-5-5".to_string(),
                share: 0.75
            },
            Share {
                label: "claude-sonnet-5-5".to_string(),
                share: 0.25
            }
        ]
    );
}

#[test]
fn just_after_midnight_a_session_counts_its_last_15_minutes_from_before_midnight() {
    let (_dir, store) = store_with_sessions(&[(
        "s1",
        session_spend_line("s1", "a1", "2026-11-03 23:55:00", 10)
            + &session_spend_line("s1", "a2", "2026-11-04 00:02:00", 1),
    )]);

    let today = sessions_today(&store, None, local_ms("2026-11-04 00:05:00")).unwrap();

    assert_eq!(today.len(), 1);
    assert_eq!(today[0].last_15_minutes_usd, 11.0);
    assert_eq!(today[0].today_usd, 1.0);
}

#[test]
fn a_session_with_only_unpriced_requests_today_is_in_sessions_today_at_0() {
    let (_dir, store) = store_with_sessions(&[(
        "s1",
        session_spend_line("s1", "a1", "2026-11-04 09:00:00", 1)
            .replace("claude-opus-5-5", "claude-opus-9-9"),
    )]);

    let today = sessions_today(&store, None, local_ms("2026-11-04 14:00:00")).unwrap();

    assert_eq!(today.len(), 1);
    assert_eq!(today[0].session, "s1");
    assert_eq!(today[0].today_usd, 0.0);
}
