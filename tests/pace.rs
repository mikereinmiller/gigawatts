use std::fs;
use std::path::Path;

use gigawatts::accounts::Attribution;
use gigawatts::config::{Config, epoch_ms_from_text};
use gigawatts::contract::{DollarPace, Limit, LimitPace, PaceBasis, PaceState, SessionSpend};
use gigawatts::ingest::ingest_tree;
use gigawatts::pace::{active_sessions, dollar_pace, fastest_session, limit_pace};
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
        })
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

    let spend = |session: &str, last_15_minutes_usd: f64, total_usd: f64| SessionSpend {
        session: session.to_string(),
        label: session.to_string(),
        repo: "orchard".to_string(),
        branch: Some("ab/carts".to_string()),
        last_15_minutes_usd,
        total_usd,
    };
    assert_eq!(active, vec![spend("s2", 5.0, 5.0), spend("s1", 4.0, 14.0)]);
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
