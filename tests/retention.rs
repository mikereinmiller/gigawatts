use std::collections::{BTreeMap, HashMap, HashSet};

use gigawatts::accounts::Attribution;
use gigawatts::config::{Switch, epoch_ms_from_text};
use gigawatts::contract::{CostStatus, DataRow, GroupBy, Range};
use gigawatts::data::query;
use gigawatts::pricing::{ModelPrices, PriceTable, Prices};
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::git::Resolver;
use gigawatts::sources::transcript::{
    CostState, Line, ModelCost, Request, RequestDetail, SessionTotals, Tokens,
};
use gigawatts::store::{FilePosition, Store};

const MODEL: &str = "claude-opus-5-5";
const TODAY: &str = "2026-03-01";
const KEEP_DAYS: u32 = 30;
const EXPIRED_NOON: &str = "2026-01-10T18:00:00.000Z";
const KEPT_NOON: &str = "2026-02-25T18:00:00.000Z";
const KEPT_DAY: &str = "2026-02-25";

fn account() -> Account {
    Account {
        account_uuid: "00000000-0000-4000-8000-00000000000a".to_string(),
        organization_uuid: "00000000-0000-4000-8000-0000000000f0".to_string(),
    }
}

fn request(session: &str, request_id: &str, timestamp: &str) -> Request {
    Request {
        session_id: session.to_string(),
        request_id: request_id.to_string(),
        model: MODEL.to_string(),
        tokens: Tokens {
            input: 1_000,
            output: 500,
            ..Tokens::default()
        },
        detail: RequestDetail {
            timestamp: Some(timestamp.to_string()),
            cwd: Some("/work/gigawatts".to_string()),
            git_branch: Some("main".to_string()),
            ..RequestDetail::default()
        },
        ..Request::default()
    }
}

fn cost_state(session: &str, requests: &[&Request]) -> CostState {
    let table = PriceTable::built_in();
    let cost: f64 = requests.iter().filter_map(|r| table.price(r)).sum();
    let tokens = Tokens {
        input: requests.iter().map(|r| r.tokens.input).sum(),
        output: requests.iter().map(|r| r.tokens.output).sum(),
        ..Tokens::default()
    };
    CostState {
        session_id: session.to_string(),
        total_cost_usd: cost,
        models: BTreeMap::from([(
            MODEL.to_string(),
            ModelCost {
                tokens,
                has_cache_write_split: true,
                cost_usd: cost,
                thinking_tokens: 0,
                web_fetches: 0,
            },
        )]),
        totals: SessionTotals {
            start_time_ms: requests[0]
                .detail
                .timestamp
                .as_deref()
                .and_then(epoch_ms_from_text),
            ..SessionTotals::default()
        },
    }
}

type SessionRow<'a> = (
    Option<&'a str>,
    &'a str,
    u64,
    Option<f64>,
    Option<CostStatus>,
);

struct History {
    _dir: tempfile::TempDir,
    store: Store,
}

impl History {
    fn new() -> History {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
        History { _dir: dir, store }
    }

    fn read(&mut self, file: &str, lines: Vec<Line>) {
        self.read_priced(file, lines, &PriceTable::built_in());
    }

    fn read_priced(&mut self, file: &str, lines: Vec<Line>, table: &PriceTable) {
        self.read_as(file, lines, table, &Attribution::single(Some(account())));
    }

    fn read_as(
        &mut self,
        file: &str,
        lines: Vec<Line>,
        table: &PriceTable,
        attribution: &Attribution,
    ) {
        let position = FilePosition {
            path: file.to_string(),
            device: 1,
            inode: 1,
            first_line_hash: Some(1),
            offset: 1,
        };
        self.store
            .save_file(&position, true, &lines, attribution, table, &Resolver)
            .unwrap();
    }

    fn report(&self) -> Vec<DataRow> {
        query(&self.store, Range::All, GroupBy::Model, &[], TODAY)
            .unwrap()
            .rows
    }
}

fn requests(requests: &[&Request]) -> Vec<Line> {
    requests
        .iter()
        .map(|request| Line::Request(Box::new((*request).clone())))
        .collect()
}

#[test]
fn reading_an_expired_days_transcript_again_keeps_that_days_totals() {
    let mut history = History::new();
    let first = request("s1", "req_1", EXPIRED_NOON);
    let second = request("s2", "req_2", EXPIRED_NOON);
    history.read("s1.jsonl", requests(&[&first]));
    history.read("s2.jsonl", requests(&[&second]));
    let before = history.report();

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history.read("s1.jsonl", requests(&[&first]));

    assert_eq!(history.report(), before);
    assert_eq!(before[0].requests, 2);
    assert_eq!(history.store.request_count().unwrap(), 0);
}

#[test]
fn a_price_change_after_cleanup_leaves_expired_sessions_as_they_were() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(cost_state("s1", &[&old])));
    history.read("s1.jsonl", lines);
    let before = history.report();

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let doubled = Prices {
        input: 10.0,
        cache_write_5m: 12.5,
        cache_write_1h: 20.0,
        cache_read: 1.0,
        output: 50.0,
    };
    let table = PriceTable::with_overrides(HashMap::from([(
        MODEL.to_string(),
        ModelPrices::standard(doubled),
    )]));
    history.store.reprice_if_changed(&table).unwrap();

    assert_eq!(history.report(), before);
}

#[test]
fn a_session_spanning_the_cutoff_gains_no_not_in_transcript_row_when_its_cost_arrives() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let new = request("s1", "req_2", KEPT_NOON);
    history.read("s1.jsonl", requests(&[&old, &new]));
    let before = history.report();

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let mut lines = requests(&[&old, &new]);
    lines.push(Line::CostState(cost_state("s1", &[&old, &new])));
    history.read("s1.jsonl", lines);

    let after = history.report();
    assert!(after.iter().all(|row| !row.not_in_transcript), "{after:#?}");
    assert_eq!(after, before);
}

#[test]
fn turning_tracking_off_keeps_the_missing_spend_of_expired_sessions() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut recorded = cost_state("s1", &[&old]);
    let model = recorded.models.get_mut(MODEL).unwrap();
    model.tokens.output += 1_000;
    model.cost_usd += 1.0;
    recorded.total_cost_usd += 1.0;
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);
    let before = history.report();
    assert!(before.iter().any(|row| row.not_in_transcript));

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .delete_emptied_sessions(&HashSet::from([account()]))
        .unwrap();

    assert_eq!(history.report(), before);
}

#[test]
fn sessions_in_a_branch_still_add_up_to_the_branch_after_cleanup() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let new = request("s2", "req_2", KEPT_NOON);
    history.read("s1.jsonl", requests(&[&old]));
    history.read("s2.jsonl", requests(&[&new]));

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let sessions = query(
        &history.store,
        Range::All,
        GroupBy::Branch,
        &["gigawatts · main"],
        TODAY,
    )
    .unwrap()
    .rows;

    let rows: Vec<SessionRow> = sessions
        .iter()
        .map(|row| {
            (
                row.key.as_deref(),
                row.label.as_str(),
                row.requests,
                row.cost_usd,
                row.status,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                Some("s2"),
                "s2",
                1,
                Some(0.014),
                Some(CostStatus::Estimated)
            ),
            (
                None,
                "older sessions, detail not kept",
                1,
                Some(0.014),
                None
            ),
        ]
    );
}

#[test]
fn deleting_titles_reaches_sessions_whose_detail_expired() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut lines = requests(&[&old]);
    lines.push(Line::Title {
        session_id: "s1".to_string(),
        title: "Fix the parser".to_string(),
    });
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let deleted = history.store.delete_titles(&account()).unwrap();

    assert_eq!(deleted, 1);
    assert_eq!(history.store.session_title("s1").unwrap(), None);
}

#[test]
fn cleanup_deletes_the_tool_rows_of_expired_requests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite3");
    let mut history = History {
        store: Store::open(&path).unwrap(),
        _dir: dir,
    };
    let mut old = request("s1", "req_1", EXPIRED_NOON);
    old.detail.tools = vec!["Bash".to_string(), "Read".to_string()];
    let mut new = request("s2", "req_2", KEPT_NOON);
    new.detail.tools = vec!["Read".to_string()];
    history.read("s1.jsonl", requests(&[&old]));
    history.read("s2.jsonl", requests(&[&new]));

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();

    let tool_rows: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM request_tools", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tool_rows, 1);
    assert_eq!(history.store.request_tools("req_2").unwrap(), ["Read"]);
}

#[test]
fn a_session_running_at_cleanup_still_records_spend_missing_from_its_transcript() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let new = request("s1", "req_2", KEPT_NOON);
    history.read("s1.jsonl", requests(&[&old, &new]));

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let mut recorded = cost_state("s1", &[&old, &new]);
    let model = recorded.models.get_mut(MODEL).unwrap();
    model.tokens.output += 1_000;
    model.cost_usd += 1.0;
    recorded.total_cost_usd += 1.0;
    history.read("s1.jsonl", vec![Line::CostState(recorded)]);

    let missing: Vec<(u64, f64)> = history
        .report()
        .iter()
        .filter(|row| row.not_in_transcript)
        .map(|row| (row.output_tokens, row.cost_usd.unwrap()))
        .collect();
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0].0, 1_000);
    assert!((missing[0].1 - 1.0).abs() < 1e-9, "{missing:?}");
}

#[test]
fn a_session_resumed_after_its_early_requests_expired_is_estimated_again() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(cost_state("s1", &[&old])));
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let resumed = request("s1", "req_2", KEPT_NOON);
    history.read("s1.jsonl", requests(&[&resumed]));

    let sessions = query(&history.store, Range::All, GroupBy::Model, &[MODEL], TODAY)
        .unwrap()
        .rows;
    let s1 = sessions
        .iter()
        .find(|row| row.key.as_deref() == Some("s1"))
        .unwrap();
    assert_eq!(s1.status, Some(CostStatus::Estimated));
}

fn total_cost(rows: &[DataRow]) -> f64 {
    rows.iter().filter_map(|row| row.cost_usd).sum()
}

fn priced_override(model: &str, input: f64, output: f64) -> PriceTable {
    let prices = Prices {
        input,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read: input / 10.0,
        output,
    };
    PriceTable::with_overrides(HashMap::from([(
        model.to_string(),
        ModelPrices::standard(prices),
    )]))
}

#[test]
fn a_session_first_read_after_cleanup_adds_no_spend_on_its_kept_days() {
    let mut history = History::new();
    let other = request("s0", "req_0", EXPIRED_NOON);
    history.read("s0.jsonl", requests(&[&other]));
    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let new = request("s1", "req_2", KEPT_NOON);

    let mut lines = requests(&[&old, &new]);
    lines.push(Line::CostState(cost_state("s1", &[&old, &new])));
    history.read("s1.jsonl", lines);

    let since_cutoff = query(&history.store, Range::Month, GroupBy::Model, &[], KEPT_DAY)
        .unwrap()
        .rows;
    assert!(
        since_cutoff.iter().all(|row| !row.not_in_transcript),
        "{since_cutoff:#?}"
    );
    assert!((total_cost(&since_cutoff) - 0.014).abs() < 1e-9);
}

#[test]
fn a_price_change_keeps_a_session_spanning_the_cutoff_at_its_recorded_cost() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let new = request("s1", "req_2", KEPT_NOON);
    let mut lines = requests(&[&old, &new]);
    lines.push(Line::CostState(cost_state("s1", &[&old, &new])));
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .reprice_if_changed(&priced_override(MODEL, 10.0, 50.0))
        .unwrap();

    assert!((total_cost(&history.report()) - 0.028).abs() < 1e-9);
}

#[test]
fn a_model_priced_after_cleanup_is_not_counted_twice() {
    const UNPRICED: &str = "claude-invented-9";
    let mut history = History::new();
    let mut old = request("s1", "req_1", EXPIRED_NOON);
    let mut new = request("s1", "req_2", KEPT_NOON);
    old.model = UNPRICED.to_string();
    new.model = UNPRICED.to_string();
    let mut recorded = cost_state("s1", &[&old, &new]);
    recorded.models.clear();
    recorded.models.insert(
        UNPRICED.to_string(),
        ModelCost {
            tokens: Tokens {
                input: 2_000,
                output: 1_000,
                ..Tokens::default()
            },
            has_cache_write_split: true,
            cost_usd: 1.0,
            thinking_tokens: 0,
            web_fetches: 0,
        },
    );
    recorded.total_cost_usd = 1.0;
    let mut lines = requests(&[&old, &new]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .reprice_if_changed(&priced_override(UNPRICED, 5.0, 25.0))
        .unwrap();

    assert!(
        (total_cost(&history.report()) - 1.0).abs() < 1e-9,
        "{:#?}",
        history.report()
    );
}

#[test]
fn a_fully_expired_session_keeps_its_recorded_spend_when_its_model_gets_a_price() {
    const UNPRICED: &str = "claude-invented-9";
    let mut history = History::new();
    let mut old = request("s1", "req_1", EXPIRED_NOON);
    old.model = UNPRICED.to_string();
    let mut recorded = cost_state("s1", &[&old]);
    let mut model = recorded.models.remove(MODEL).unwrap();
    model.tokens = old.tokens.clone();
    model.cost_usd = 1.0;
    recorded.models.insert(UNPRICED.to_string(), model);
    recorded.total_cost_usd = 1.0;
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .reprice_if_changed(&priced_override(UNPRICED, 5.0, 25.0))
        .unwrap();

    assert!(
        (total_cost(&history.report()) - 1.0).abs() < 1e-9,
        "{:#?}",
        history.report()
    );
}

#[test]
fn a_price_change_keeps_the_missing_spend_of_a_fully_expired_session_with_no_start_time() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut recorded = cost_state("s1", &[&old]);
    recorded.totals.start_time_ms = None;
    let model = recorded.models.get_mut(MODEL).unwrap();
    model.tokens.output += 1_000;
    model.cost_usd += 1.0;
    recorded.total_cost_usd += 1.0;
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);
    let before = history.report();

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .reprice_if_changed(&priced_override(MODEL, 10.0, 50.0))
        .unwrap();

    assert_eq!(history.report(), before);
}

#[test]
fn a_session_first_read_after_cleanup_adds_no_missing_spend_after_a_later_cleanup() {
    let mut history = History::new();
    let other = request("s0", "req_0", EXPIRED_NOON);
    history.read("s0.jsonl", requests(&[&other]));
    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let middle = request("s1", "req_2", "2026-02-05T18:00:00.000Z");
    let new = request("s1", "req_3", KEPT_NOON);
    let mut lines = requests(&[&old, &middle, &new]);
    lines.push(Line::CostState(cost_state("s1", &[&old, &middle, &new])));
    history.read("s1.jsonl", lines);

    history
        .store
        .expire_requests(KEEP_DAYS, "2026-03-10")
        .unwrap();
    history
        .store
        .reprice_if_changed(&priced_override(MODEL, 10.0, 50.0))
        .unwrap();

    let report = history.report();
    assert!(
        report.iter().all(|row| !row.not_in_transcript),
        "{report:#?}"
    );
}

#[test]
fn an_expired_unpriced_model_keeps_its_spend_beside_a_kept_model_when_it_gets_a_price() {
    const UNPRICED: &str = "claude-invented-9";
    let mut history = History::new();
    let mut old = request("s1", "req_1", EXPIRED_NOON);
    old.model = UNPRICED.to_string();
    let new = request("s1", "req_2", KEPT_NOON);
    let mut recorded = cost_state("s1", &[&new]);
    recorded.models.insert(
        UNPRICED.to_string(),
        ModelCost {
            tokens: old.tokens.clone(),
            has_cache_write_split: true,
            cost_usd: 1.0,
            thinking_tokens: 0,
            web_fetches: 0,
        },
    );
    recorded.total_cost_usd += 1.0;
    let mut lines = requests(&[&old, &new]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .reprice_if_changed(&priced_override(UNPRICED, 5.0, 25.0))
        .unwrap();

    assert!(
        (total_cost(&history.report()) - 1.014).abs() < 1e-9,
        "{:#?}",
        history.report()
    );
}

#[test]
fn cleanup_drops_the_price_gaps_of_requests_it_expired() {
    let mut history = History::new();
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut recorded = cost_state("s1", &[&old]);
    recorded.total_cost_usd *= 2.0;
    recorded.models.get_mut(MODEL).unwrap().cost_usd *= 2.0;
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);
    assert_eq!(history.store.price_gaps().unwrap().len(), 1);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();

    assert!(history.store.price_gaps().unwrap().is_empty());
}

#[test]
fn an_accounts_request_count_includes_expired_days() {
    let mut history = History::new();
    let first = request("s1", "req_1", EXPIRED_NOON);
    let second = request("s1", "req_2", KEPT_NOON);
    history.read("s1.jsonl", requests(&[&first, &second]));

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();

    assert_eq!(history.store.accounts().unwrap()[0].requests, 2);
}

#[test]
fn an_expired_unpriced_model_keeps_a_session_estimated_after_a_price_change() {
    const UNPRICED: &str = "claude-invented-9";
    let mut history = History::new();
    let mut old = request("s1", "req_1", EXPIRED_NOON);
    old.model = UNPRICED.to_string();
    let new = request("s1", "req_2", KEPT_NOON);
    let mut recorded = cost_state("s1", &[&new]);
    recorded.models.insert(
        UNPRICED.to_string(),
        ModelCost {
            tokens: old.tokens.clone(),
            has_cache_write_split: true,
            cost_usd: 1.0,
            thinking_tokens: 0,
            web_fetches: 0,
        },
    );
    recorded.total_cost_usd += 1.0;
    let mut lines = requests(&[&old, &new]);
    lines.push(Line::CostState(recorded));
    history.read("s1.jsonl", lines);

    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    history
        .store
        .reprice_if_changed(&priced_override("claude-other-9", 1.0, 5.0))
        .unwrap();

    let sessions = query(&history.store, Range::All, GroupBy::Model, &[MODEL], TODAY)
        .unwrap()
        .rows;
    let s1 = sessions
        .iter()
        .find(|row| row.key.as_deref() == Some("s1"))
        .unwrap();
    assert_eq!(s1.status, Some(CostStatus::Estimated));
}

#[test]
fn forgetting_an_account_counts_its_expired_requests() {
    let mut history = History::new();
    let first = request("s1", "req_1", EXPIRED_NOON);
    let second = request("s1", "req_2", KEPT_NOON);
    history.read("s1.jsonl", requests(&[&first, &second]));
    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();

    let deleted = history.store.forget(&account()).unwrap();

    assert_eq!(deleted, 2);
}

#[test]
fn a_session_of_an_untracked_earlier_account_read_after_cleanup_keeps_nothing() {
    let mut history = History::new();
    let other = request("s0", "req_0", EXPIRED_NOON);
    history.read("s0.jsonl", requests(&[&other]));
    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let later: Account = Account {
        account_uuid: "00000000-0000-4000-8000-00000000000b".to_string(),
        organization_uuid: "00000000-0000-4000-8000-0000000000f0".to_string(),
    };
    let attribution = Attribution {
        current: Some(later),
        switch: Some(Switch {
            at_ms: epoch_ms_from_text(KEPT_NOON).unwrap(),
            earlier: account(),
        }),
        untracked: HashSet::from([account()]),
        ..Attribution::default()
    };
    let old = request("s1", "req_1", EXPIRED_NOON);
    let mut lines = requests(&[&old]);
    lines.push(Line::CostState(cost_state("s1", &[&old])));
    lines.push(Line::Title {
        session_id: "s1".to_string(),
        title: "Secret".to_string(),
    });

    history.read_as("s1.jsonl", lines, &PriceTable::built_in(), &attribution);

    assert_eq!(history.store.session_title("s1").unwrap(), None);
}

#[test]
fn a_title_read_later_for_an_expired_session_follows_its_accounts_setting() {
    let mut history = History::new();
    let other = request("s0", "req_0", EXPIRED_NOON);
    history.read("s0.jsonl", requests(&[&other]));
    history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();
    let later: Account = Account {
        account_uuid: "00000000-0000-4000-8000-00000000000b".to_string(),
        organization_uuid: "00000000-0000-4000-8000-0000000000f0".to_string(),
    };
    let attribution = Attribution {
        current: Some(later),
        switch: Some(Switch {
            at_ms: epoch_ms_from_text(KEPT_NOON).unwrap(),
            earlier: account(),
        }),
        without_titles: HashSet::from([account()]),
        ..Attribution::default()
    };
    let old = request("s1", "req_1", EXPIRED_NOON);
    let table = PriceTable::built_in();
    history.read_as("s1.jsonl", requests(&[&old]), &table, &attribution);

    let title = vec![Line::Title {
        session_id: "s1".to_string(),
        title: "Secret".to_string(),
    }];
    history.read_as("s1.jsonl", title, &table, &attribution);

    assert_eq!(history.store.session_title("s1").unwrap(), None);
}

#[test]
fn cleanup_frees_pages_a_previous_cleanup_left_behind_even_when_nothing_new_expired() {
    let mut history = History::new();
    let kept: Vec<Request> = (0..2_000)
        .map(|n| request("s1", &format!("req_{n}"), KEPT_NOON))
        .collect();
    history.read("s1.jsonl", requests(&kept.iter().collect::<Vec<_>>()));
    let path = history._dir.path().join("history.sqlite3");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute("DELETE FROM requests", []).unwrap();
    let free_pages = || -> i64 {
        raw.query_row("PRAGMA freelist_count", [], |row| row.get(0))
            .unwrap()
    };
    assert!(free_pages() > 0);

    let expired = history.store.expire_requests(KEEP_DAYS, TODAY).unwrap();

    assert_eq!(expired, 0);
    assert_eq!(free_pages(), 0);
}

#[test]
fn a_renamed_sessions_title_is_not_stored_for_an_account_with_titles_off() {
    let mut history = History::new();
    let attribution = Attribution {
        current: Some(account()),
        without_titles: HashSet::from([account()]),
        ..Attribution::default()
    };
    let table = PriceTable::built_in();
    let kept = request("s1", "req_1", KEPT_NOON);
    let mut lines = requests(&[&kept]);
    lines.push(Line::CustomTitle {
        session_id: "s1".to_string(),
        title: "Secret".to_string(),
    });

    history.read_as("s1.jsonl", lines, &table, &attribution);

    assert_eq!(history.store.session_title("s1").unwrap(), None);
}

#[test]
fn after_deleting_a_renamed_sessions_title_a_generated_title_is_stored_again() {
    let mut history = History::new();
    let kept = request("s1", "req_1", KEPT_NOON);
    let mut lines = requests(&[&kept]);
    lines.push(Line::CustomTitle {
        session_id: "s1".to_string(),
        title: "Login page".to_string(),
    });
    history.read("s1.jsonl", lines);
    history.store.delete_titles(&account()).unwrap();

    history.read(
        "s1.jsonl",
        vec![Line::Title {
            session_id: "s1".to_string(),
            title: "Fix the form".to_string(),
        }],
    );

    assert_eq!(
        history.store.session_title("s1").unwrap().as_deref(),
        Some("Fix the form")
    );
}
