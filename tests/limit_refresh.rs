use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use gigawatts::accounts::{self, Directory};
use gigawatts::config::Config;
use gigawatts::contract::LimitSource;
use gigawatts::limits::{self, Refresher};
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::get_usage::{self, Runner};
use gigawatts::store::Store;

const MINUTE: i64 = 60_000;
const START: i64 = 1_793_800_000_000;

fn account() -> Account {
    Account {
        account_uuid: "00000000-0000-4000-8000-00000000000a".to_string(),
        organization_uuid: "00000000-0000-4000-8000-0000000000f0".to_string(),
    }
}

#[test]
fn an_account_without_a_reading_is_due_at_its_first_activity_then_every_ten_minutes() {
    let mut refresher = Refresher::default();

    refresher.activity(&account(), START, None);
    assert_eq!(refresher.due(START), vec![account()]);
    refresher.succeeded(&account(), START);

    assert_eq!(refresher.due(START + 9 * MINUTE), vec![]);
    assert_eq!(refresher.next_due_ms(START), None);
    refresher.activity(&account(), START + 5 * MINUTE, None);
    assert_eq!(refresher.next_due_ms(START), Some(START + 10 * MINUTE));
    assert_eq!(refresher.due(START + 10 * MINUTE), vec![account()]);
}

#[test]
fn nothing_is_due_once_the_account_goes_ten_minutes_without_activity() {
    let mut refresher = Refresher::default();
    refresher.activity(&account(), START, None);
    refresher.succeeded(&account(), START + MINUTE);

    assert_eq!(refresher.next_due_ms(START), None);
    assert_eq!(refresher.due(START + 11 * MINUTE), vec![]);
    assert_eq!(refresher.due(START + 60 * MINUTE), vec![]);
}

#[test]
fn a_fresh_saved_reading_waits_out_the_rest_of_its_interval() {
    let mut refresher = Refresher::default();

    refresher.activity(&account(), START, Some(START - 3 * MINUTE));

    assert_eq!(refresher.due(START), vec![]);
    assert_eq!(refresher.next_due_ms(START), Some(START + 7 * MINUTE));
}

#[test]
fn the_backoff_wait_doubles_with_each_failure() {
    let mut refresher = Refresher::default();
    let mut waits = Vec::new();
    let mut now = START;
    for _ in 0..5 {
        refresher.failed(&account(), now, "Not logged in");
        let wait = refresher.retry_after_ms(&account()).unwrap();
        waits.push(wait / MINUTE);
        now += wait;
    }

    assert_eq!(waits, vec![10, 20, 40, 60, 60]);
    let failing = refresher.failing();
    assert_eq!(failing.len(), 1);
    assert_eq!(failing[0].since_ms, START);
    assert_eq!(failing[0].error, "Not logged in");
    refresher.succeeded(&account(), now);
    assert!(refresher.failing().is_empty());
}

struct FakeClaude {
    response: String,
    requests: RefCell<Vec<(Option<PathBuf>, String)>>,
}

impl Runner for FakeClaude {
    fn run(&self, config_dir: Option<&Path>, request: &str) -> Result<String, get_usage::Error> {
        self.requests
            .borrow_mut()
            .push((config_dir.map(Path::to_path_buf), request.to_string()));
        let request: serde_json::Value = serde_json::from_str(request).unwrap();
        let mut response: serde_json::Value = serde_json::from_str(&self.response).unwrap();
        if request["request"]["skip_behaviors"] == true
            && let Some(usage) = response["response"]["response"].as_object_mut()
        {
            usage.remove("behaviors");
        }
        Ok(response.to_string())
    }
}

fn fake_claude() -> FakeClaude {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage/ultra/get-usage.json");
    let response: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture).unwrap()).unwrap();
    FakeClaude {
        response: serde_json::json!({"type":"control_response","response":{"subtype":"success","request_id":get_usage::REQUEST_ID,"response":response}}).to_string(),
        requests: RefCell::new(Vec::new()),
    }
}

fn logged_in_directories(home: &Path) -> Vec<Directory> {
    let login = format!(
        r#"{{"oauthAccount":{{"accountUuid":"{}","organizationUuid":"{}"}}}}"#,
        account().account_uuid,
        account().organization_uuid
    );
    fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
    fs::write(home.join(".claude.json"), &login).unwrap();
    let work = home.join(".claude-work");
    fs::create_dir_all(work.join("projects")).unwrap();
    fs::write(work.join(".claude.json"), &login).unwrap();
    accounts::discover(home, &Config::default())
}

#[test]
fn a_refresh_saves_the_limits_and_skips_advice_fetched_within_the_hour() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let directories = logged_in_directories(dir.path());
    let claude = fake_claude();

    limits::refresh(
        &mut store,
        &claude,
        &directories[0],
        &|| START,
        "2026-11-04",
    )
    .unwrap();
    limits::refresh(
        &mut store,
        &claude,
        &directories[0],
        &|| START + 10 * MINUTE,
        "2026-11-04",
    )
    .unwrap();
    limits::refresh(
        &mut store,
        &claude,
        &directories[1],
        &|| START + 20 * MINUTE,
        "2026-11-05",
    )
    .unwrap();

    let requests = claude.requests.borrow();
    let skip_behaviors: Vec<bool> = requests
        .iter()
        .map(|(_, request)| {
            let request: serde_json::Value = serde_json::from_str(request).unwrap();
            request["request"]["skip_behaviors"].as_bool().unwrap()
        })
        .collect();
    assert_eq!(skip_behaviors, vec![false, true, false]);
    let config_dirs: Vec<Option<PathBuf>> = requests.iter().map(|(dir, _)| dir.clone()).collect();
    assert_eq!(
        config_dirs,
        vec![None, None, Some(dir.path().join(".claude-work"))]
    );
    let latest = store.latest_limits().unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].0, account());
    assert_eq!(latest[0].1.fetched_at_ms, START + 20 * MINUTE);
    assert_eq!(latest[0].1.source, LimitSource::GetUsage);
    assert_eq!(latest[0].1.limits.len(), 3);
}

#[test]
fn a_failed_refresh_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let directories = logged_in_directories(dir.path());
    let claude = FakeClaude {
        response: serde_json::json!({"type":"control_response","response":{"subtype":"error","request_id":get_usage::REQUEST_ID,"error":"Not logged in"}}).to_string(),
        requests: RefCell::new(Vec::new()),
    };

    let error = limits::refresh(
        &mut store,
        &claude,
        &directories[0],
        &|| START,
        "2026-11-04",
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Claude Code answered with an error: Not logged in"
    );
    assert_eq!(store.latest_limits().unwrap(), vec![]);
}

#[test]
fn an_account_that_went_idle_while_overdue_is_never_due_again() {
    let mut refresher = Refresher::default();
    let other = Account {
        organization_uuid: "00000000-0000-4000-8000-0000000000e0".to_string(),
        ..account()
    };
    refresher.activity(&account(), START, None);
    refresher.activity(&other, START + 5 * MINUTE, None);
    refresher.succeeded(&other, START + 5 * MINUTE);
    refresher.activity(&other, START + 9 * MINUTE, None);

    let now = START + 11 * MINUTE;

    assert_eq!(refresher.due(now), vec![]);
    assert_eq!(refresher.next_due_ms(now), Some(START + 15 * MINUTE));
}

#[test]
fn an_account_no_directory_can_refresh_is_forgotten() {
    let mut refresher = Refresher::default();
    refresher.activity(&account(), START, None);
    refresher.failed(&account(), START, "Not logged in");
    let due_at = START + 10 * MINUTE;
    refresher.activity(&account(), due_at, None);
    assert_eq!(refresher.due(due_at), vec![account()]);

    refresher.keep_only(&std::collections::BTreeSet::new());

    assert_eq!(refresher.due(due_at), vec![]);
    assert_eq!(refresher.next_due_ms(due_at), None);
    assert!(refresher.failing().is_empty());
}

struct SlowClaude {
    inner: FakeClaude,
    clock: std::cell::Cell<i64>,
}

impl Runner for SlowClaude {
    fn run(&self, config_dir: Option<&Path>, request: &str) -> Result<String, get_usage::Error> {
        self.clock.set(self.clock.get() + 2_000);
        self.inner.run(config_dir, request)
    }
}

#[test]
fn a_refresh_is_timed_when_claude_answers() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let directories = logged_in_directories(dir.path());
    let claude = SlowClaude {
        inner: fake_claude(),
        clock: std::cell::Cell::new(START),
    };

    limits::refresh(
        &mut store,
        &claude,
        &directories[0],
        &|| claude.clock.get(),
        "2026-11-04",
    )
    .unwrap();

    assert_eq!(
        store.latest_limits().unwrap()[0].1.fetched_at_ms,
        START + 2_000
    );
}

#[test]
fn advice_is_asked_for_again_once_an_hour_has_passed() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let directories = logged_in_directories(dir.path());
    let claude = fake_claude();

    for minutes in [0, 50, 60, 70] {
        limits::refresh(
            &mut store,
            &claude,
            &directories[0],
            &|| START + minutes * MINUTE,
            "2026-11-04",
        )
        .unwrap();
    }

    let asked: Vec<bool> = claude
        .requests
        .borrow()
        .iter()
        .map(|(_, request)| {
            let request: serde_json::Value = serde_json::from_str(request).unwrap();
            !request["request"]["skip_behaviors"].as_bool().unwrap()
        })
        .collect();
    assert_eq!(asked, vec![true, false, true, false]);
    let advice = store
        .usage_advice(&account(), "2026-11-04")
        .unwrap()
        .unwrap();
    assert_eq!(advice.fetched_at_ms, START + 60 * MINUTE);
}

struct ClaudeThatSwitchesLogin {
    inner: FakeClaude,
    claude_json: PathBuf,
}

impl Runner for ClaudeThatSwitchesLogin {
    fn run(&self, config_dir: Option<&Path>, request: &str) -> Result<String, get_usage::Error> {
        fs::write(
            &self.claude_json,
            r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000b","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
        )
        .unwrap();
        self.inner.run(config_dir, request)
    }
}

#[test]
fn a_refresh_during_which_the_login_changes_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let directories = logged_in_directories(dir.path());
    let claude = ClaudeThatSwitchesLogin {
        inner: fake_claude(),
        claude_json: dir.path().join(".claude.json"),
    };

    limits::refresh(
        &mut store,
        &claude,
        &directories[0],
        &|| START,
        "2026-11-04",
    )
    .unwrap();

    assert_eq!(store.latest_limits().unwrap(), vec![]);
    assert_eq!(store.usage_advice(&account(), "2026-11-04").unwrap(), None);
}
