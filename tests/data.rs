use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

use gigawatts::accounts::Attribution;
use gigawatts::config::{Config, Switch};
use gigawatts::contract::{DataResponse, GroupBy, Range, SCHEMA_VERSION};
use gigawatts::data::{label_accounts, query, render};
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/projects");

fn local_noon_in_utc(day: &str) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1 || 'T12:00:00', 'utc')",
            [day],
            |row| row.get(0),
        )
        .unwrap()
}

fn request_line(request_id: &str, day: &str) -> String {
    let timestamp = local_noon_in_utc(day);
    format!(
        r#"{{"type":"assistant","sessionId":"s1","requestId":"{request_id}","timestamp":"{timestamp}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":10}}}}}}"#
    ) + "\n"
}

fn store_with_requests_on(days: &[&str]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines: String = days
        .iter()
        .enumerate()
        .map(|(n, day)| request_line(&format!("r{n}"), day))
        .collect();
    fs::write(projects.join("s.jsonl"), lines).unwrap();
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

fn requests_in(store: &Store, range: Range, today: &str) -> (Option<String>, u64) {
    let response = query(store, range, GroupBy::Model, &[], today).unwrap();
    let requests = response.rows.iter().map(|row| row.requests).sum();
    (response.since, requests)
}

#[test]
fn ranges_start_today_on_monday_and_on_the_first_of_the_month() {
    let days = [
        "2025-12-31",
        "2026-01-02",
        "2026-01-05",
        "2026-01-07",
        "2026-01-08",
    ];
    let (_dir, store) = store_with_requests_on(&days);
    let thursday = "2026-01-08";

    assert_eq!(
        requests_in(&store, Range::Today, thursday),
        (Some("2026-01-08".into()), 1)
    );
    assert_eq!(
        requests_in(&store, Range::Week, thursday),
        (Some("2026-01-05".into()), 3)
    );
    assert_eq!(
        requests_in(&store, Range::Month, thursday),
        (Some("2026-01-01".into()), 4)
    );
    assert_eq!(requests_in(&store, Range::All, thursday), (None, 5));
}

#[test]
fn a_week_asked_for_on_monday_starts_that_day() {
    let (_dir, store) = store_with_requests_on(&["2026-01-04", "2026-01-05"]);

    assert_eq!(
        requests_in(&store, Range::Week, "2026-01-05"),
        (Some("2026-01-05".into()), 1)
    );
}

#[test]
fn a_response_carries_its_schema_version() {
    let (_dir, store) = store_with_requests_on(&["2026-01-05"]);

    let response: DataResponse =
        query(&store, Range::All, GroupBy::Model, &[], "2026-01-05").unwrap();

    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 2);
}

#[test]
fn renders_cost_by_model_for_the_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        Path::new(FIXTURES),
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();

    let response = query(&store, Range::All, GroupBy::Model, &[], "2026-11-03").unwrap();

    insta::assert_snapshot!(render(&response));
}

fn account(organization: &str) -> Account {
    format!("00000000-0000-4000-8000-00000000000a/{organization}")
        .parse()
        .unwrap()
}

fn ingest_switched(store: &mut Store, projects: &Path) {
    let attribution = Attribution {
        current: Some(account("00000000-0000-4000-8000-0000000000f0")),
        switch: Some(Switch {
            at_ms: 1_767_700_000_000,
            earlier: account("00000000-0000-4000-8000-0000000000e0"),
        }),
        ..Attribution::default()
    };
    ingest_tree(
        store,
        projects,
        &PriceTable::built_in(),
        &attribution,
        &Resolver::default(),
    )
    .unwrap();
}

fn switched_store(lines: &str) -> (tempfile::TempDir, Store, Account, Account) {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    fs::write(projects.join("s.jsonl"), lines).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_switched(&mut store, &projects);
    let enterprise = account("00000000-0000-4000-8000-0000000000e0");
    let ultra = account("00000000-0000-4000-8000-0000000000f0");
    (dir, store, enterprise, ultra)
}

#[test]
fn each_account_gets_its_own_rows_and_table() {
    let lines = request_line("r-enterprise", "2026-01-05") + &request_line("r-ultra", "2026-01-07");
    let (_dir, store, enterprise, ultra) = switched_store(&lines);
    let config = Config::parse(
        &format!("[accounts.\"{ultra}\"]\nlabel = \"Ultra\""),
        Path::new("/home"),
    )
    .unwrap();

    let mut response = query(&store, Range::All, GroupBy::Model, &[], "2026-01-08").unwrap();
    label_accounts(&mut response, &config);

    let accounts: Vec<_> = response
        .rows
        .iter()
        .map(|row| (row.account.clone(), row.account_label.clone(), row.requests))
        .collect();
    assert_eq!(
        accounts,
        vec![
            (Some(enterprise.to_string()), enterprise.to_string(), 1),
            (Some(ultra.to_string()), "Ultra".to_string(), 1),
        ]
    );
    let rendered = render(&response);
    assert_eq!(rendered.matches("claude-opus-5-5").count(), 2, "{rendered}");
    assert!(rendered.contains("\nUltra\n"), "{rendered}");
}

fn cost_state_line(session: &str, input: u64, output: u64, cost_usd: f64) -> String {
    serde_json::json!({
        "type": "cost-state",
        "sessionId": session,
        "totalCostUSD": cost_usd,
        "startTime": 1_767_787_200_000_i64,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": input, "outputTokens": output, "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": cost_usd
            }
        }
    })
    .to_string()
        + "\n"
}

fn not_in_transcript_by_account(store: &Store) -> Vec<(Option<String>, u64)> {
    let mut rows: Vec<_> = query(store, Range::All, GroupBy::Repo, &[], "2026-01-08")
        .unwrap()
        .rows
        .into_iter()
        .filter(|row| row.not_in_transcript)
        .map(|row| (row.account, row.input_tokens))
        .collect();
    rows.sort();
    rows
}

#[test]
fn not_in_transcript_rows_belong_to_their_sessions_account() {
    let lines = request_line("r-enterprise", "2026-01-05")
        + &request_line("r-ultra", "2026-01-07")
        + &cost_state_line("s1", 1_000, 1_000, 1.0)
        + &cost_state_line("s2", 500, 50, 0.5);

    let (_dir, store, enterprise, ultra) = switched_store(&lines);

    assert_eq!(
        not_in_transcript_by_account(&store),
        vec![
            (Some(enterprise.to_string()), 998),
            (Some(ultra.to_string()), 500),
        ]
    );
}

#[test]
fn forgetting_an_account_removes_its_not_in_transcript_rows_from_a_shared_session() {
    let lines = request_line("r-enterprise", "2026-01-05")
        + &request_line("r-ultra", "2026-01-07")
        + &cost_state_line("s1", 1_000, 1_000, 1.0)
        + &cost_state_line("s2", 500, 50, 0.5);
    let (dir, mut store, enterprise, ultra) = switched_store(&lines);

    store.forget(&enterprise).unwrap();
    let projects = dir.path().join("projects");
    OpenOptions::new()
        .append(true)
        .open(projects.join("s.jsonl"))
        .unwrap()
        .write_all(cost_state_line("s1", 1_200, 1_100, 1.2).as_bytes())
        .unwrap();
    ingest_switched(&mut store, &projects);
    store
        .reprice_if_changed(&PriceTable::with_overrides(Default::default()))
        .unwrap();

    assert_eq!(
        not_in_transcript_by_account(&store),
        vec![(Some(ultra.to_string()), 500)]
    );
}

fn ingest_with_ultra_untracked(store: &mut Store, projects: &Path) {
    let attribution = Attribution {
        current: Some(account("00000000-0000-4000-8000-0000000000f0")),
        switch: Some(Switch {
            at_ms: 1_767_700_000_000,
            earlier: account("00000000-0000-4000-8000-0000000000e0"),
        }),
        untracked: [account("00000000-0000-4000-8000-0000000000f0")].into(),
        ..Attribution::default()
    };
    ingest_tree(
        store,
        projects,
        &PriceTable::built_in(),
        &attribution,
        &Resolver::default(),
    )
    .unwrap();
}

#[test]
fn an_untracked_accounts_spend_in_a_shared_session_stays_out_of_the_tracked_account() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines = request_line("r-enterprise", "2026-01-05")
        + &request_line("r-ultra", "2026-01-07")
        + &cost_state_line("s1", 1_000, 1_000, 1.0);
    fs::write(projects.join("s.jsonl"), lines).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

    ingest_with_ultra_untracked(&mut store, &projects);

    assert_eq!(not_in_transcript_by_account(&store), vec![]);
    let rows = query(&store, Range::All, GroupBy::Model, &[], "2026-01-08")
        .unwrap()
        .rows;
    let accounts: Vec<_> = rows
        .iter()
        .map(|row| (row.account.clone(), row.requests))
        .collect();
    let enterprise = account("00000000-0000-4000-8000-0000000000e0");
    assert_eq!(accounts, vec![(Some(enterprise.to_string()), 1)]);
}

#[test]
fn an_untracked_request_read_later_returns_the_sessions_scaled_requests_to_table_prices() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let transcript = projects.join("s.jsonl");
    fs::write(
        &transcript,
        request_line("r-enterprise", "2026-01-05") + &cost_state_line("s1", 1, 10, 1.0),
    )
    .unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_with_ultra_untracked(&mut store, &projects);
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap()
        .write_all(request_line("r-ultra", "2026-01-07").as_bytes())
        .unwrap();

    ingest_with_ultra_untracked(&mut store, &projects);

    let rows = query(&store, Range::All, GroupBy::Model, &[], "2026-01-08")
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let cost = rows[0].cost_usd.unwrap();
    assert!((cost - 0.000_204).abs() < 1e-12, "{cost}");
}

#[test]
fn forgetting_an_account_returns_a_shared_sessions_scaled_requests_to_table_prices() {
    let lines = request_line("r-enterprise", "2026-01-05")
        + &request_line("r-ultra", "2026-01-07")
        + &cost_state_line("s1", 2, 20, 1.0);
    let (_dir, mut store, enterprise, ultra) = switched_store(&lines);
    store.reprice_if_changed(&PriceTable::built_in()).unwrap();

    store.forget(&enterprise).unwrap();
    store.reprice_if_changed(&PriceTable::built_in()).unwrap();

    let rows = query(&store, Range::All, GroupBy::Model, &[], "2026-01-08")
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].account, Some(ultra.to_string()));
    let cost = rows[0].cost_usd.unwrap();
    assert!((cost - 0.000_204).abs() < 1e-12, "{cost}");
}

#[test]
fn reading_again_after_tracking_an_account_takes_the_sessions_cost_state_back() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines = request_line("r-enterprise", "2026-01-05")
        + &request_line("r-ultra", "2026-01-07")
        + &cost_state_line("s1", 1_000, 1_000, 1.0);
    fs::write(projects.join("s1.jsonl"), lines).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_with_ultra_untracked(&mut store, &projects);

    store.forget_positions_under(&projects).unwrap();
    ingest_switched(&mut store, &projects);

    let enterprise = account("00000000-0000-4000-8000-0000000000e0");
    assert_eq!(
        not_in_transcript_by_account(&store),
        vec![(Some(enterprise.to_string()), 998)]
    );
}

#[test]
fn tracking_a_forgotten_account_again_takes_a_shared_sessions_cost_state_back() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines = request_line("r-enterprise", "2026-01-05")
        + &request_line("r-ultra", "2026-01-07")
        + &cost_state_line("s1", 1_000, 1_000, 1.0);
    fs::write(projects.join("s1.jsonl"), lines).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_switched(&mut store, &projects);
    let enterprise = account("00000000-0000-4000-8000-0000000000e0");

    store.forget(&enterprise).unwrap();
    store.forget_positions_under(&projects).unwrap();
    ingest_switched(&mut store, &projects);

    assert_eq!(
        not_in_transcript_by_account(&store),
        vec![(Some(enterprise.to_string()), 998)]
    );
}
