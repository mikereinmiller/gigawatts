use std::fs;
use std::path::Path;

use gigawatts::accounts::Attribution;
use gigawatts::config::{Config, Switch};
use gigawatts::contract::{DataResponse, GroupBy, Range, SCHEMA_VERSION};
use gigawatts::data::{label_accounts, query, render};
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::claude_json::Account;
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
    )
    .unwrap();
    (dir, store)
}

fn requests_in(store: &Store, range: Range, today: &str) -> (Option<String>, u64) {
    let response = query(store, range, GroupBy::Model, today).unwrap();
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

    let response: DataResponse = query(&store, Range::All, GroupBy::Model, "2026-01-05").unwrap();

    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 1);
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
    )
    .unwrap();

    let response = query(&store, Range::All, GroupBy::Model, "2026-11-03").unwrap();

    insta::assert_snapshot!(render(&response));
}

fn account(organization: &str) -> Account {
    format!("00000000-0000-4000-8000-00000000000a/{organization}")
        .parse()
        .unwrap()
}

#[test]
fn each_account_gets_its_own_rows_and_table() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let lines = request_line("r-enterprise", "2026-01-05") + &request_line("r-ultra", "2026-01-07");
    fs::write(projects.join("s.jsonl"), lines).unwrap();
    let enterprise = account("00000000-0000-4000-8000-0000000000e0");
    let ultra = account("00000000-0000-4000-8000-0000000000f0");
    let attribution = Attribution {
        current: Some(ultra.clone()),
        switch: Some(Switch {
            at_ms: 1_767_700_000_000,
            earlier: enterprise.clone(),
        }),
        ..Attribution::default()
    };
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(&mut store, &projects, &PriceTable::built_in(), &attribution).unwrap();
    let config = Config::parse(
        &format!("[accounts.\"{ultra}\"]\nlabel = \"Ultra\""),
        Path::new("/home"),
    )
    .unwrap();

    let mut response = query(&store, Range::All, GroupBy::Model, "2026-01-08").unwrap();
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
