use std::fs;

use gigawatts::accounts::Attribution;
use gigawatts::config::Config;
use gigawatts::contract::{
    AccountLimits, AccountToday, Alert, AlertState, DollarPace, ForecastBasis, Limit,
    LimitForecast, LimitPace, LimitSource, PaceBasis, PaceState, PacedLimit, Problem, ProblemKind,
    SCHEMA_VERSION, SessionSpend, Share, Snapshot,
};
use gigawatts::engine::snapshot;
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::git::Resolver;
use gigawatts::store::{LimitsReading, Store};

const ACCOUNT: &str = "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0";
const IDLE_ACCOUNT: &str =
    "00000000-0000-4000-8000-00000000000b/00000000-0000-4000-8000-0000000000f0";
const UNTRACKED_ACCOUNT: &str =
    "00000000-0000-4000-8000-00000000000c/00000000-0000-4000-8000-0000000000f0";

fn local_noon_in_utc(store_day: &str, days_back: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1 || 'T12:00:00', ?2 || ' days', 'utc')",
            rusqlite::params![store_day, -days_back],
            |row| row.get(0),
        )
        .unwrap()
}

fn days_before(store_day: &str, days: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT date(?1, ?2 || ' days')",
            rusqlite::params![store_day, -days],
            |row| row.get(0),
        )
        .unwrap()
}

fn local_ms(store_day: &str, clock: &str) -> i64 {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT CAST(unixepoch(?1 || 'T' || ?2, 'utc') AS INTEGER) * 1000",
            [store_day, clock],
            |row| row.get(0),
        )
        .unwrap()
}

fn is_weekend(store_day: &str) -> bool {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%w', ?1) IN ('0', '6')",
            [store_day],
            |row| row.get(0),
        )
        .unwrap()
}

fn utc_text(ms: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1 / 1000, 'unixepoch')",
            [ms],
            |row| row.get(0),
        )
        .unwrap()
}

fn request_line(request_id: &str, timestamp: &str) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"s1","requestId":"{request_id}","timestamp":"{timestamp}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1000,"output_tokens":500}}}}}}"#
    ) + "\n"
}

#[test]
fn the_snapshot_carries_each_tracked_accounts_spend_today_and_the_engines_problems() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let today = store.local_today().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines = [
        request_line("r1", &local_noon_in_utc(&today, 0)),
        request_line("r2", &local_noon_in_utc(&today, 0)),
        request_line("r3", &local_noon_in_utc(&today, 7)),
    ]
    .concat();
    fs::write(projects.join("s1.jsonl"), lines).unwrap();
    let table = PriceTable::built_in();
    ingest(&mut store, &projects, &table, ACCOUNT);
    for (folder, account, days_back) in [
        ("idle", IDLE_ACCOUNT, 7),
        ("untracked", UNTRACKED_ACCOUNT, 0),
    ] {
        let other = dir.path().join(folder);
        fs::create_dir_all(&other).unwrap();
        fs::write(
            other.join("s.jsonl"),
            request_line(
                &format!("r-{folder}"),
                &local_noon_in_utc(&today, days_back),
            )
            .replace("\"s1\"", &format!("\"s-{folder}\"")),
        )
        .unwrap();
        ingest(&mut store, &other, &table, account);
    }
    let config = Config::parse(
        &format!(
            "[accounts.\"{ACCOUNT}\"]\nlabel = \"personal\"\n\n[accounts.\"{UNTRACKED_ACCOUNT}\"]\ntracking = false\n"
        ),
        dir.path(),
    )
    .unwrap();
    let week_ago = days_before(&today, 7);
    let weekend = is_weekend(&today);
    let now_ms = local_ms(&today, "12:10:00");
    let two_days_ms = 2 * 24 * 3_600_000;
    let weekly = Limit {
        kind: "weekly_all".to_string(),
        group: Some("weekly".to_string()),
        scope: None,
        percent: 63.0,
        severity: Some("normal".to_string()),
        resets_at: Some(utc_text(now_ms + two_days_ms)),
        active: false,
        credits: None,
    };
    for account in [ACCOUNT, UNTRACKED_ACCOUNT] {
        store
            .record_limits(
                &account.parse().unwrap(),
                &LimitsReading {
                    fetched_at_ms: 1_699_999_400_000,
                    source: LimitSource::GetUsage,
                    limits: vec![weekly.clone()],
                },
            )
            .unwrap();
    }
    let watch_failing = Problem {
        kind: ProblemKind::WatchFailing,
        affects: "a".to_string(),
        since: None,
        cause: "b".to_string(),
        fix: "c".to_string(),
        account: None,
    };

    let snapshot = snapshot::build(
        &store,
        &config,
        now_ms,
        std::slice::from_ref(&watch_failing),
    )
    .unwrap();

    let s1 = SessionSpend {
        session: "s1".to_string(),
        label: "s1".to_string(),
        repo: "unknown folder".to_string(),
        branch: None,
        last_15_minutes_usd: 0.028,
        total_usd: 0.042,
        last_active_ms: local_ms(&today, "12:00:00"),
        today_usd: 0.028,
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
        snapshot,
        Snapshot {
            schema_version: SCHEMA_VERSION,
            engine_version: gigawatts::engine::version(),
            written_at_ms: now_ms,
            alerts: Vec::new(),
            today,
            accounts: vec![
                AccountToday {
                    account: Some(ACCOUNT.to_string()),
                    account_label: "personal".to_string(),
                    configured_label: Some("personal".to_string()),
                    requests: 2,
                    cost_usd: 0.028,
                    pace: DollarPace {
                        today_usd: 0.028,
                        usual_usd: Some(0.014),
                        usual_days: 1,
                        clock: "12:10".to_string(),
                        weekend,
                        earlier_days_usd: vec![0.014],
                        earlier_day_totals_usd: vec![0.014],
                        earlier_days: vec![week_ago.clone()],
                    },
                    fastest_session: Some(s1.clone()),
                    active_sessions: vec![s1.clone()],
                    sessions_today: vec![s1],
                    unpriced_models: Vec::new(),
                    adjusted_models: Vec::new(),
                    cost_estimated: true,
                    limits: Some(AccountLimits {
                        source: LimitSource::GetUsage,
                        fetched_at_ms: 1_699_999_400_000,
                        limits: vec![PacedLimit {
                            limit: weekly,
                            pace: Some(LimitPace {
                                state: PaceState::Under,
                                basis: PaceBasis::Even,
                                expected_percent: 500.0 / 7.0,
                                window_start_ms: now_ms - 5 * 24 * 3_600_000,
                                window_end_ms: now_ms + two_days_ms,
                                provisional: false,
                            }),
                            forecast: None,
                            this_machine_usd: None,
                            elsewhere_usd: None,
                            elsewhere_percent: None,
                        }],
                    }),
                },
                AccountToday {
                    account: Some(IDLE_ACCOUNT.to_string()),
                    account_label: IDLE_ACCOUNT.to_string(),
                    configured_label: None,
                    requests: 0,
                    cost_usd: 0.0,
                    pace: DollarPace {
                        today_usd: 0.0,
                        usual_usd: Some(0.014),
                        usual_days: 1,
                        clock: "12:10".to_string(),
                        weekend,
                        earlier_days_usd: vec![0.014],
                        earlier_day_totals_usd: vec![0.014],
                        earlier_days: vec![week_ago.clone()],
                    },
                    fastest_session: None,
                    active_sessions: Vec::new(),
                    sessions_today: Vec::new(),
                    unpriced_models: Vec::new(),
                    adjusted_models: Vec::new(),
                    cost_estimated: false,
                    limits: None,
                },
            ],
            problems: vec![watch_failing],
        }
    );
}

fn ingest(store: &mut Store, projects: &std::path::Path, table: &PriceTable, account: &str) {
    let account: Account = account.parse().unwrap();
    ingest_tree(
        store,
        projects,
        table,
        &Attribution::single(Some(account)),
        &Resolver,
    )
    .unwrap();
}

#[test]
fn a_limit_on_course_to_fill_carries_a_forecast_from_the_time_it_was_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let today = store.local_today().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        request_line("r1", &local_noon_in_utc(&today, 0)),
    )
    .unwrap();
    ingest(&mut store, &projects, &PriceTable::built_in(), ACCOUNT);
    let fetched_at_ms = local_ms(&today, "12:00:00");
    let hour_ms = 3_600_000;
    store
        .record_limits(
            &ACCOUNT.parse().unwrap(),
            &LimitsReading {
                fetched_at_ms,
                source: LimitSource::GetUsage,
                limits: vec![Limit {
                    kind: "session".to_string(),
                    group: None,
                    scope: None,
                    percent: 60.0,
                    severity: None,
                    resets_at: Some(utc_text(fetched_at_ms + 3 * hour_ms)),
                    active: true,
                    credits: None,
                }],
            },
        )
        .unwrap();
    let config = Config::parse("", dir.path()).unwrap();

    let built = snapshot::build(&store, &config, fetched_at_ms + 30 * 60_000, &[]).unwrap();

    let limits = built.accounts[0].limits.as_ref().unwrap();
    assert_eq!(
        limits.limits[0].forecast,
        Some(LimitForecast {
            full_at_ms: fetched_at_ms + 80 * 60_000,
            basis: ForecastBasis::Window,
        })
    );
}

#[test]
fn a_burst_in_the_last_30_minutes_gives_a_forecast_at_the_recent_pace() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let today = store.local_today().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s1.jsonl"),
        request_line("r1", &local_noon_in_utc(&today, 0)),
    )
    .unwrap();
    ingest(&mut store, &projects, &PriceTable::built_in(), ACCOUNT);
    let fetched_at_ms = local_ms(&today, "12:00:00");
    let minute_ms = 60_000;
    let resets_at = utc_text(fetched_at_ms + 180 * minute_ms);
    let session = |percent: f64| Limit {
        kind: "session".to_string(),
        group: None,
        scope: None,
        percent,
        severity: None,
        resets_at: Some(resets_at.clone()),
        active: true,
        credits: None,
    };
    for (at_ms, percent) in [
        (fetched_at_ms - 40 * minute_ms, 30.0),
        (fetched_at_ms, 60.0),
    ] {
        store
            .record_limits(
                &ACCOUNT.parse().unwrap(),
                &LimitsReading {
                    fetched_at_ms: at_ms,
                    source: LimitSource::GetUsage,
                    limits: vec![session(percent)],
                },
            )
            .unwrap();
    }
    let config = Config::parse("", dir.path()).unwrap();

    let built = snapshot::build(&store, &config, fetched_at_ms + 5 * minute_ms, &[]).unwrap();

    let limits = built.accounts[0].limits.as_ref().unwrap();
    assert_eq!(
        limits.limits[0].forecast,
        Some(LimitForecast {
            full_at_ms: fetched_at_ms + 40 * minute_ms,
            basis: ForecastBasis::Recent,
        })
    );
    assert_eq!(
        built.alerts,
        vec![Alert {
            account: Some(ACCOUNT.to_string()),
            account_label: built.accounts[0].account_label.clone(),
            limit: "Session".to_string(),
            state: AlertState::Filling,
            at_ms: fetched_at_ms + 40 * minute_ms,
        }]
    );
}

#[test]
fn writing_replaces_the_whole_file_and_leaves_no_staging_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.json");
    fs::write(&path, "old").unwrap();
    let snapshot = Snapshot {
        schema_version: SCHEMA_VERSION,
        engine_version: "9.9.9".to_string(),
        written_at_ms: 1,
        today: "2026-11-03".to_string(),
        accounts: Vec::new(),
        problems: Vec::new(),
        alerts: Vec::new(),
    };

    snapshot::write(&path, &snapshot).unwrap();

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["engine_version"], "9.9.9");
    assert_eq!(written["schema_version"], SCHEMA_VERSION);
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec!["snapshot.json"]);
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn only_an_account_with_unpriced_requests_in_open_sessions_or_adjusted_requests_today_names_those_models()
 {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let today = store.local_today().unwrap();
    let noon = |days_back| local_noon_in_utc(&today, days_back);
    let session = |line: String, session: &str| line.replace("\"s1\"", &format!("\"{session}\""));
    let gap_cost_state = r#"{"type":"cost-state","sessionId":"adjusted","totalCostUSD":1.0,"modelUsage":{"claude-opus-5-5":{"inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":1.0}}}"#;
    let used = dir.path().join("used");
    fs::create_dir_all(&used).unwrap();
    fs::write(
        used.join("adjusted.jsonl"),
        session(request_line("a1", &noon(0)), "adjusted") + gap_cost_state + "\n",
    )
    .unwrap();
    fs::write(
        used.join("unpriced.jsonl"),
        session(request_line("u1", &noon(0)), "unpriced")
            .replace("claude-opus-5-5", "claude-opus-9-9"),
    )
    .unwrap();
    let closed_cost_state = r#"{"type":"cost-state","sessionId":"closed","totalCostUSD":0.2,"modelUsage":{"claude-opus-9-7":{"inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.2}}}"#;
    fs::write(
        used.join("closed.jsonl"),
        session(request_line("c1", &noon(0)), "closed")
            .replace("claude-opus-5-5", "claude-opus-9-7")
            + closed_cost_state
            + "\n",
    )
    .unwrap();
    let shortfall_cost_state = r#"{"type":"cost-state","sessionId":"shortfall","totalCostUSD":0.000001,"modelUsage":{"claude-sonnet-5-5":{"inputTokens":2000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.000001}}}"#;
    fs::write(
        used.join("shortfall.jsonl"),
        session(request_line("s1", &noon(0)), "shortfall")
            .replace("claude-opus-5-5", "claude-sonnet-5-5")
            + shortfall_cost_state
            + "\n",
    )
    .unwrap();
    fs::write(
        used.join("old.jsonl"),
        session(request_line("o1", &noon(7)), "old").replace("claude-opus-5-5", "claude-opus-9-8"),
    )
    .unwrap();
    let table = PriceTable::built_in();
    ingest(&mut store, &used, &table, ACCOUNT);
    let other = dir.path().join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("plain.jsonl"),
        session(request_line("p1", &noon(0)), "plain"),
    )
    .unwrap();
    ingest(&mut store, &other, &table, IDLE_ACCOUNT);

    let snapshot = snapshot::build(
        &store,
        &Config::default(),
        local_ms(&today, "13:00:00"),
        &[],
    )
    .unwrap();

    let flags: Vec<_> = snapshot
        .accounts
        .iter()
        .map(|account| {
            (
                account.account.clone().unwrap(),
                account.unpriced_models.clone(),
                account.adjusted_models.clone(),
            )
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            (
                ACCOUNT.to_string(),
                vec!["claude-opus-9-9".to_string()],
                vec!["claude-opus-5-5".to_string()]
            ),
            (IDLE_ACCOUNT.to_string(), Vec::new(), Vec::new()),
        ]
    );
}

#[test]
fn an_unpriced_model_seen_with_and_without_a_date_is_named_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let today = store.local_today().unwrap();
    let noon = local_noon_in_utc(&today, 0);
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines: String = [
        ("r1", "claude-opus-9"),
        ("r2", "claude-opus-9-1-20261201"),
        ("r3", "claude-opus-9-20261101"),
    ]
    .iter()
    .map(|(request, model)| request_line(request, &noon).replace("claude-opus-5-5", model))
    .collect();
    fs::write(projects.join("s1.jsonl"), lines).unwrap();
    ingest(&mut store, &projects, &PriceTable::built_in(), ACCOUNT);

    let snapshot = snapshot::build(
        &store,
        &Config::default(),
        local_ms(&today, "13:00:00"),
        &[],
    )
    .unwrap();

    assert_eq!(
        snapshot.accounts[0].unpriced_models,
        vec!["claude-opus-9".to_string(), "claude-opus-9-1".to_string()]
    );
}

#[test]
fn spend_today_counts_as_estimated_until_every_session_today_is_checked_against_claude_code() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let today = store.local_today().unwrap();
    let noon = local_noon_in_utc(&today, 0);
    let session = |line: String, session: &str| line.replace("\"s1\"", &format!("\"{session}\""));
    let gap_cost_state = r#"{"type":"cost-state","sessionId":"checked","totalCostUSD":1.0,"modelUsage":{"claude-opus-5-5":{"inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":1.0}}}"#;
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("checked.jsonl"),
        session(request_line("a1", &noon), "checked") + gap_cost_state + "\n",
    )
    .unwrap();
    let table = PriceTable::built_in();
    ingest(&mut store, &projects, &table, ACCOUNT);
    let at = local_ms(&today, "13:00:00");
    let checked = snapshot::build(&store, &Config::default(), at, &[]).unwrap();
    fs::write(
        projects.join("open.jsonl"),
        session(request_line("o1", &noon), "open"),
    )
    .unwrap();
    ingest(&mut store, &projects, &table, ACCOUNT);

    let with_open = snapshot::build(&store, &Config::default(), at, &[]).unwrap();

    assert!(!checked.accounts[0].cost_estimated);
    assert!(with_open.accounts[0].cost_estimated);
}

const CREDIT_PERIOD_START_MS: i64 = 1_793_491_200_000;
const CREDITS_FETCHED_AT_MS: i64 = CREDIT_PERIOD_START_MS + 3 * 24 * 3_600_000;

fn credits_limit(used: f64) -> Limit {
    Limit {
        kind: "credits".to_string(),
        group: None,
        scope: None,
        percent: used / 2200.0 * 100.0,
        severity: None,
        resets_at: None,
        active: false,
        credits: Some(gigawatts::contract::Credits {
            used,
            limit: Some(2200.0),
            currency: "USD".to_string(),
        }),
    }
}

fn credits_snapshot(lines: &[String], limits: Vec<Limit>, keep_from: Option<&str>) -> PacedLimit {
    credits_snapshot_of_sessions(&[("s1", lines.concat())], limits, keep_from)
}

fn credits_snapshot_of_sessions(
    sessions: &[(&str, String)],
    limits: Vec<Limit>,
    keep_from: Option<&str>,
) -> PacedLimit {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    for (session, lines) in sessions {
        fs::write(projects.join(format!("{session}.jsonl")), lines).unwrap();
    }
    ingest(&mut store, &projects, &PriceTable::built_in(), ACCOUNT);
    if let Some(day) = keep_from {
        store.expire_requests(1, day).unwrap();
    }
    store
        .record_limits(
            &ACCOUNT.parse().unwrap(),
            &LimitsReading {
                fetched_at_ms: CREDITS_FETCHED_AT_MS,
                source: LimitSource::GetUsage,
                limits,
            },
        )
        .unwrap();
    let config = Config::parse("", dir.path()).unwrap();
    let built = snapshot::build(&store, &config, CREDITS_FETCHED_AT_MS + 3_600_000, &[]).unwrap();
    built.accounts[0].limits.as_ref().unwrap().limits[0].clone()
}

fn requests_around_the_credit_period() -> Vec<String> {
    [
        ("before-period", CREDIT_PERIOD_START_MS - 3_600_000),
        ("early", CREDIT_PERIOD_START_MS + 3_600_000),
        ("late", CREDITS_FETCHED_AT_MS - 60_000),
        ("after-reading", CREDITS_FETCHED_AT_MS + 60_000),
    ]
    .into_iter()
    .map(|(id, ms)| request_line(id, &utc_text(ms)))
    .collect()
}

fn assert_near(actual: Option<f64>, expected: f64) {
    let actual = actual.unwrap_or_else(|| panic!("expected {expected}, got null"));
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn credits_used_beyond_this_machines_spend_in_the_period_count_as_used_elsewhere() {
    let shown = credits_snapshot(
        &requests_around_the_credit_period(),
        vec![credits_limit(84.028)],
        None,
    );

    assert_near(shown.this_machine_usd, 0.028);
    assert_near(shown.elsewhere_usd, 84.0);
}

#[test]
fn credits_used_elsewhere_under_five_dollars_are_left_out() {
    let shown = credits_snapshot(
        &requests_around_the_credit_period(),
        vec![credits_limit(4.928)],
        None,
    );

    assert_near(shown.this_machine_usd, 0.028);
    assert_eq!(shown.elsewhere_usd, None);
}

#[test]
fn credits_used_elsewhere_under_one_percent_of_the_credits_used_are_left_out() {
    let big = request_line("big", &utc_text(CREDIT_PERIOD_START_MS + 3_600_000))
        .replace("\"output_tokens\":500", "\"output_tokens\":30000000");
    let shown = credits_snapshot(&[big], vec![credits_limit(605.004)], None);

    assert_near(shown.this_machine_usd, 600.004);
    assert_eq!(shown.elsewhere_usd, None);
}

#[test]
fn an_account_with_session_limits_gets_no_elsewhere_split_on_its_credits() {
    let session = Limit {
        kind: "session".to_string(),
        group: None,
        scope: None,
        percent: 10.0,
        severity: None,
        resets_at: Some(utc_text(CREDITS_FETCHED_AT_MS + 3_600_000)),
        active: true,
        credits: None,
    };
    let shown = credits_snapshot(
        &requests_around_the_credit_period(),
        vec![credits_limit(84.028), session],
        None,
    );

    assert_eq!((shown.this_machine_usd, shown.elsewhere_usd), (None, None));
}

#[test]
fn credits_get_no_elsewhere_split_once_request_detail_no_longer_reaches_the_periods_start() {
    let shown = credits_snapshot(
        &requests_around_the_credit_period(),
        vec![credits_limit(84.028)],
        Some("2026-11-03"),
    );

    assert_eq!((shown.this_machine_usd, shown.elsewhere_usd), (None, None));
}

fn session_with_unlogged_spend(
    session: &str,
    requests_at_ms: [i64; 2],
    extra_input: u64,
) -> String {
    let requests: String = requests_at_ms
        .iter()
        .enumerate()
        .map(|(n, ms)| {
            request_line(&format!("{session}-{n}"), &utc_text(*ms))
                .replace("\"s1\"", &format!("\"{session}\""))
        })
        .collect();
    let cost = 0.028 + extra_input as f64 * 4.0 / 1e6;
    let cost_state = serde_json::json!({
        "type": "cost-state",
        "sessionId": session,
        "totalCostUSD": cost,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": 2_000 + extra_input,
                "outputTokens": 1_000,
                "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0,
                "webSearchRequests": 0,
                "costUSD": cost
            }
        }
    });
    requests + &cost_state.to_string() + "\n"
}

fn cost_state_only(session: &str, start_ms: i64) -> String {
    serde_json::json!({
        "type": "cost-state",
        "sessionId": session,
        "totalCostUSD": 0.4,
        "startTime": start_ms,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": 100_000,
                "outputTokens": 0,
                "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0,
                "webSearchRequests": 0,
                "costUSD": 0.4
            }
        }
    })
    .to_string()
        + "\n"
}

#[test]
fn spend_missing_from_transcripts_counts_only_for_sessions_that_ended_inside_the_period() {
    let hour_ms = 3_600_000;
    let sessions = [
        (
            "inside",
            session_with_unlogged_spend(
                "inside",
                [
                    CREDIT_PERIOD_START_MS + hour_ms,
                    CREDIT_PERIOD_START_MS + 2 * hour_ms,
                ],
                50_000,
            ),
        ),
        (
            "before",
            session_with_unlogged_spend(
                "before",
                [
                    CREDIT_PERIOD_START_MS - 2 * hour_ms,
                    CREDIT_PERIOD_START_MS - 60_000,
                ],
                75_000,
            ),
        ),
        (
            "after",
            session_with_unlogged_spend(
                "after",
                [
                    CREDITS_FETCHED_AT_MS - hour_ms,
                    CREDITS_FETCHED_AT_MS + 60_000,
                ],
                125_000,
            ),
        ),
        (
            "no-requests",
            cost_state_only("no-requests", CREDIT_PERIOD_START_MS + 3 * hour_ms),
        ),
    ];

    let shown = credits_snapshot_of_sessions(&sessions, vec![credits_limit(84.642)], None);

    assert_near(shown.this_machine_usd, 0.642);
    assert_near(shown.elsewhere_usd, 84.0);
}

#[test]
fn credits_get_no_elsewhere_split_while_an_open_session_has_spend_with_no_price() {
    let mut lines = requests_around_the_credit_period();
    lines.push(
        request_line("unpriced", &utc_text(CREDIT_PERIOD_START_MS + 7_200_000))
            .replace("claude-opus-5-5", "claude-opus-9-9"),
    );

    let shown = credits_snapshot(&lines, vec![credits_limit(84.028)], None);

    assert_eq!((shown.this_machine_usd, shown.elsewhere_usd), (None, None));
}

const ESTIMATE_FIRST_DAY: &str = "2026-11-08";

fn request_costing(request_id: &str, at_ms: i64, usd: f64) -> String {
    let output_tokens = ((usd - 0.004) / 0.00002).round() as u64;
    request_line(request_id, &utc_text(at_ms)).replace(
        "\"output_tokens\":500",
        &format!("\"output_tokens\":{output_tokens}"),
    )
}

fn timed_limit(kind: &str, percent: f64, resets_at_ms: i64) -> Limit {
    Limit {
        kind: kind.to_string(),
        group: None,
        scope: None,
        percent,
        severity: None,
        resets_at: Some(utc_text(resets_at_ms)),
        active: true,
        credits: None,
    }
}

fn estimate_snapshot(
    kind: &str,
    readings: &[(i64, f64, i64)],
    spend: &[(i64, f64)],
    now_ms: i64,
) -> PacedLimit {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines: String = spend
        .iter()
        .enumerate()
        .map(|(n, (at_ms, usd))| request_costing(&format!("r{n}"), *at_ms, *usd))
        .collect();
    fs::write(projects.join("s1.jsonl"), lines).unwrap();
    ingest(&mut store, &projects, &PriceTable::built_in(), ACCOUNT);
    for (fetched_at_ms, percent, resets_at_ms) in readings {
        store
            .record_limits(
                &ACCOUNT.parse().unwrap(),
                &LimitsReading {
                    fetched_at_ms: *fetched_at_ms,
                    source: LimitSource::GetUsage,
                    limits: vec![timed_limit(kind, *percent, *resets_at_ms)],
                },
            )
            .unwrap();
    }
    let config = Config::parse("", dir.path()).unwrap();
    let built = snapshot::build(&store, &config, now_ms, &[]).unwrap();
    built.accounts[0].limits.as_ref().unwrap().limits[0].clone()
}

fn weekly_estimate(days: &[(f64, f64)]) -> PacedLimit {
    let resets_ms = local_ms(&days_before(ESTIMATE_FIRST_DAY, -5), "09:00:00");
    let mut readings = vec![(local_ms(ESTIMATE_FIRST_DAY, "09:00:00"), 0.0, resets_ms)];
    let mut spend = Vec::new();
    for (index, (percent, usd)) in days.iter().enumerate() {
        let day = days_before(ESTIMATE_FIRST_DAY, -(index as i64));
        if *usd > 0.0 {
            spend.push((local_ms(&day, "12:00:00"), *usd));
        }
        readings.push((local_ms(&day, "17:00:00"), *percent, resets_ms));
    }
    let last_day = days_before(ESTIMATE_FIRST_DAY, -(days.len() as i64 - 1));
    estimate_snapshot(
        "weekly_all",
        &readings,
        &spend,
        local_ms(&last_day, "18:00:00"),
    )
}

#[test]
fn a_weekly_rise_beyond_this_machines_spend_at_its_learned_rate_counts_as_elsewhere() {
    let shown = weekly_estimate(&[(10.0, 50.0), (30.0, 50.0)]);

    let elsewhere = shown.elsewhere_percent.expect("an estimate");
    assert!((elsewhere - 10.0).abs() < 1e-6, "{elsewhere}");
}

#[test]
fn an_estimate_under_three_points_is_left_out() {
    let shown = weekly_estimate(&[(10.0, 50.0), (30.0, 92.0)]);

    assert_eq!(shown.elsewhere_percent, None);
}

#[test]
fn a_limit_under_ten_percent_gets_no_estimate() {
    let shown = weekly_estimate(&[(5.0, 50.0), (9.0, 0.0)]);

    assert_eq!(shown.elsewhere_percent, None);
}

#[test]
fn without_a_day_that_rose_five_points_there_is_no_rate_to_estimate_with() {
    let shown = weekly_estimate(&[(4.0, 50.0), (8.0, 0.0), (12.0, 0.0)]);

    assert_eq!(shown.elsewhere_percent, None);
}

#[test]
fn a_session_limit_gets_an_estimate_from_the_rate_learned_on_earlier_sessions() {
    let second_day = days_before(ESTIMATE_FIRST_DAY, -1);
    let first_resets = local_ms(ESTIMATE_FIRST_DAY, "14:00:00");
    let second_resets = local_ms(&second_day, "19:00:00");

    let shown = estimate_snapshot(
        "session",
        &[
            (local_ms(ESTIMATE_FIRST_DAY, "09:30:00"), 0.0, first_resets),
            (local_ms(ESTIMATE_FIRST_DAY, "13:30:00"), 10.0, first_resets),
            (local_ms(&second_day, "14:30:00"), 0.0, second_resets),
            (local_ms(&second_day, "18:00:00"), 30.0, second_resets),
        ],
        &[
            (local_ms(ESTIMATE_FIRST_DAY, "11:00:00"), 50.0),
            (local_ms(&second_day, "16:00:00"), 50.0),
        ],
        local_ms(&second_day, "18:10:00"),
    );

    let elsewhere = shown.elsewhere_percent.expect("an estimate");
    assert!((elsewhere - 20.0).abs() < 1e-6, "{elsewhere}");
}

#[test]
fn a_rise_across_a_reset_is_not_learned_as_a_rate() {
    let second_day = days_before(ESTIMATE_FIRST_DAY, -1);
    let first_resets = local_ms(ESTIMATE_FIRST_DAY, "11:00:00");
    let next_resets = local_ms(ESTIMATE_FIRST_DAY, "16:00:00");
    let second_resets = local_ms(&second_day, "19:00:00");

    let shown = estimate_snapshot(
        "session",
        &[
            (local_ms(ESTIMATE_FIRST_DAY, "10:00:00"), 4.0, first_resets),
            (local_ms(ESTIMATE_FIRST_DAY, "12:00:00"), 9.0, next_resets),
            (local_ms(&second_day, "14:30:00"), 0.0, second_resets),
            (local_ms(&second_day, "18:00:00"), 30.0, second_resets),
        ],
        &[
            (local_ms(ESTIMATE_FIRST_DAY, "10:30:00"), 100.0),
            (local_ms(&second_day, "16:00:00"), 3.0),
        ],
        local_ms(&second_day, "18:10:00"),
    );

    assert_eq!(shown.elsewhere_percent, None);
}

#[test]
fn credits_get_no_percent_estimate() {
    let shown = credits_snapshot(
        &requests_around_the_credit_period(),
        vec![credits_limit(84.028)],
        None,
    );

    assert_eq!(shown.elsewhere_percent, None);
}
