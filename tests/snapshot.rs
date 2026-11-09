use std::fs;

use gigawatts::accounts::Attribution;
use gigawatts::config::Config;
use gigawatts::contract::{
    AccountLimits, AccountToday, DollarPace, FastestSession, Limit, LimitPace, LimitSource,
    PaceBasis, PaceState, PacedLimit, Problem, ProblemKind, SCHEMA_VERSION, Snapshot,
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
    };

    let snapshot = snapshot::build(
        &store,
        &config,
        now_ms,
        std::slice::from_ref(&watch_failing),
    )
    .unwrap();

    assert_eq!(
        snapshot,
        Snapshot {
            schema_version: SCHEMA_VERSION,
            engine_version: gigawatts::engine::version(),
            written_at_ms: now_ms,
            today,
            accounts: vec![
                AccountToday {
                    account: Some(ACCOUNT.to_string()),
                    account_label: "personal".to_string(),
                    requests: 2,
                    cost_usd: 0.028,
                    pace: DollarPace {
                        today_usd: 0.028,
                        usual_usd: Some(0.014),
                        usual_days: 1,
                        clock: "12:10".to_string(),
                    },
                    fastest_session: Some(FastestSession {
                        session: "s1".to_string(),
                        label: "s1".to_string(),
                        repo: "unknown folder".to_string(),
                        branch: None,
                        last_15_minutes_usd: 0.028,
                        total_usd: 0.042,
                    }),
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
                        }],
                    }),
                },
                AccountToday {
                    account: Some(IDLE_ACCOUNT.to_string()),
                    account_label: IDLE_ACCOUNT.to_string(),
                    requests: 0,
                    cost_usd: 0.0,
                    pace: DollarPace {
                        today_usd: 0.0,
                        usual_usd: Some(0.014),
                        usual_days: 1,
                        clock: "12:10".to_string(),
                    },
                    fastest_session: None,
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
        &Resolver::default(),
    )
    .unwrap();
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
