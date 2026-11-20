use std::fs;
use std::path::{Path, PathBuf};

use gigawatts::accounts::{self, Attribution};
use gigawatts::config::Config;
use gigawatts::config::Switch;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::git::Resolver;
use gigawatts::store::SeenLogin;
use gigawatts::store::Store;

const PLAIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/projects/plain/00000001-0000-4000-8000-194e1652b207.jsonl"
);

const PERSON: &str = "00000000-0000-4000-8000-00000000000a";
const ENTERPRISE_ORG: &str = "00000000-0000-4000-8000-0000000000e0";
const ULTRA_ORG: &str = "00000000-0000-4000-8000-0000000000f0";
const THIRD_ORG: &str = "00000000-0000-4000-8000-0000000000c0";

struct Home {
    _dir: tempfile::TempDir,
    path: PathBuf,
    database: PathBuf,
    store: Store,
}

fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("home");
    fs::create_dir_all(&path).unwrap();
    let database = dir.path().join("history.sqlite3");
    let store = Store::open(&database).unwrap();
    Home {
        _dir: dir,
        path,
        database,
        store,
    }
}

fn claude_json(organization: &str, email: &str) -> String {
    format!(
        r#"{{"oauthAccount":{{"accountUuid":"{PERSON}","organizationUuid":"{organization}","emailAddress":"{email}"}}}}"#
    )
}

fn add_config_dir(home: &Path, name: &str, claude_json_path: &Path, json: &str, transcript: &str) {
    let projects = home.join(name).join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    fs::copy(transcript, projects.join("s.jsonl")).unwrap();
    fs::write(claude_json_path, json).unwrap();
}

fn labels_and_requests(home: &Home, config: &Config) -> Vec<(String, u64)> {
    let directories = accounts::discover(&home.path, config);
    let mut rows: Vec<_> = accounts::list(&home.store, &directories, config)
        .unwrap()
        .into_iter()
        .map(|row| (row.label, row.requests))
        .collect();
    rows.sort();
    rows
}

#[test]
fn a_switch_time_splits_one_directory_between_two_organizations() {
    let mut home = home();
    add_config_dir(
        &home.path,
        ".claude",
        &home.path.join(".claude.json"),
        &claude_json(ULTRA_ORG, "me@example.com"),
        PLAIN,
    );
    let config = Config::parse(
        &format!(
            r#"
[accounts."{PERSON}/{ENTERPRISE_ORG}"]
label = "Enterprise"

[accounts."{PERSON}/{ULTRA_ORG}"]
label = "Ultra"

[directories."~/.claude"]
switched_at = 2026-01-08T23:51:00Z
earlier_account = "{PERSON}/{ENTERPRISE_ORG}"
"#
        ),
        &home.path,
    )
    .unwrap();

    let directories = accounts::discover(&home.path, &config);
    accounts::ingest(
        &mut home.store,
        &directories,
        &config,
        &PriceTable::built_in(),
        &Resolver,
    )
    .unwrap();

    assert_eq!(
        labels_and_requests(&home, &config),
        vec![("Enterprise".to_string(), 12), ("Ultra".to_string(), 15)]
    );
}

fn switched_home(extra_settings: &str) -> (Home, Config) {
    let home = home();
    add_config_dir(
        &home.path,
        ".claude",
        &home.path.join(".claude.json"),
        &claude_json(ULTRA_ORG, "me@example.com"),
        PLAIN,
    );
    let config = Config::parse(
        &format!(
            r#"
[directories."~/.claude"]
switched_at = 2026-01-08T23:51:00Z
earlier_account = "{PERSON}/{ENTERPRISE_ORG}"
{extra_settings}
"#
        ),
        &home.path,
    )
    .unwrap();
    (home, config)
}

fn ingest_all(home: &mut Home, config: &Config) {
    let directories = accounts::discover(&home.path, config);
    accounts::ingest(
        &mut home.store,
        &directories,
        config,
        &PriceTable::built_in(),
        &Resolver,
    )
    .unwrap();
}

#[test]
fn an_untracked_earlier_account_keeps_only_the_later_requests() {
    let (mut home, config) = switched_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false"
    ));

    ingest_all(&mut home, &config);

    assert_eq!(home.store.request_count().unwrap(), 15);
}

#[test]
fn a_directory_whose_accounts_are_all_untracked_is_not_read() {
    let (mut home, config) = switched_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false\n[accounts.\"{PERSON}/{ULTRA_ORG}\"]\ntracking = false"
    ));

    let directories = accounts::discover(&home.path, &config);
    let summary = accounts::ingest(
        &mut home.store,
        &directories,
        &config,
        &PriceTable::built_in(),
        &Resolver,
    )
    .unwrap();

    assert_eq!((summary.files, home.store.request_count().unwrap()), (0, 0));
}

const PLAIN_SESSION: &str = "00000001-0000-4000-8000-194e1652b207";

fn plain_home(settings: &str) -> (Home, Config) {
    let home = home();
    add_config_dir(
        &home.path,
        ".claude",
        &home.path.join(".claude.json"),
        &claude_json(ULTRA_ORG, "me@example.com"),
        PLAIN,
    );
    let config = Config::parse(settings, &home.path).unwrap();
    (home, config)
}

fn ultra() -> Account {
    format!("{PERSON}/{ULTRA_ORG}").parse().unwrap()
}

#[test]
fn titles_are_not_stored_for_an_account_that_turned_them_off() {
    let (mut home, config) = plain_home(&format!(
        "[accounts.\"{PERSON}/{ULTRA_ORG}\"]\nstore_titles = false"
    ));

    ingest_all(&mut home, &config);

    assert_eq!(home.store.session_title(PLAIN_SESSION).unwrap(), None);
}

#[test]
fn deleting_an_accounts_titles_leaves_its_costs() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    let before = home.store.session_title(PLAIN_SESSION).unwrap();

    let deleted = home.store.delete_titles(&ultra()).unwrap();

    assert_eq!(before.as_deref(), Some("Title 1"));
    assert_eq!(deleted, 1);
    assert_eq!(home.store.session_title(PLAIN_SESSION).unwrap(), None);
    assert_eq!(home.store.request_count().unwrap(), 27);
}

fn enterprise() -> Account {
    format!("{PERSON}/{ENTERPRISE_ORG}").parse().unwrap()
}

#[test]
fn forgetting_an_account_deletes_only_its_history() {
    let (mut home, config) = switched_home("");
    ingest_all(&mut home, &config);

    let forgotten = home.store.forget(&enterprise()).unwrap();

    let totals = home.store.totals_by_model(None).unwrap();
    assert_eq!(forgotten, 12);
    assert_eq!(home.store.request_count().unwrap(), 15);
    assert_eq!(totals.iter().map(|total| total.requests).sum::<u64>(), 15);
    assert_eq!(
        home.store.session_title(PLAIN_SESSION).unwrap().as_deref(),
        Some("Title 1")
    );
}

#[test]
fn forgetting_every_account_of_a_session_deletes_the_session() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);

    home.store.forget(&ultra()).unwrap();

    assert_eq!(home.store.request_count().unwrap(), 0);
    assert!(home.store.totals_by_model(None).unwrap().is_empty());
    assert_eq!(home.store.session_title(PLAIN_SESSION).unwrap(), None);
    assert_eq!(home.store.session_count().unwrap(), 0);
}

#[test]
fn finds_the_default_directory_config_shaped_folders_and_existing_extra_paths() {
    let home = home();
    add_config_dir(
        &home.path,
        ".claude",
        &home.path.join(".claude.json"),
        &claude_json(ULTRA_ORG, "me@example.com"),
        PLAIN,
    );
    add_config_dir(
        &home.path,
        ".claude-work",
        &home.path.join(".claude-work").join(".claude.json"),
        &claude_json(ENTERPRISE_ORG, "me@work.example"),
        PLAIN,
    );
    fs::create_dir_all(home.path.join("notes").join("projects")).unwrap();
    let elsewhere = home._dir.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join("projects")).unwrap();
    let config = Config::parse(
        &format!(
            "extra_paths = [{:?}, \"~/gone\"]",
            elsewhere.display().to_string()
        ),
        &home.path,
    )
    .unwrap();

    let found: Vec<PathBuf> = accounts::discover(&home.path, &config)
        .into_iter()
        .map(|directory| directory.path)
        .collect();

    assert_eq!(
        found,
        vec![
            home.path.join(".claude"),
            home.path.join(".claude-work"),
            elsewhere
        ]
    );
}

#[test]
fn an_account_without_a_label_is_listed_by_its_email() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);

    assert_eq!(
        labels_and_requests(&home, &config),
        vec![("me@example.com".to_string(), 27)]
    );
}

fn request_at(request_id: &str, timestamp: &str) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"s-{request_id}","requestId":"{request_id}","timestamp":"{timestamp}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":2,"output_tokens":5}}}}}}"#
    ) + "\n"
}

#[test]
fn a_changed_login_splits_unread_requests_at_the_time_it_was_seen() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    fs::write(
        home.path
            .join(".claude")
            .join("projects")
            .join("p")
            .join("t.jsonl"),
        request_at("r-before", "2026-02-01T12:00:00.000Z")
            + &request_at("r-after", "2099-01-01T12:00:00.000Z"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    let changes = home.store.login_changes().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (&changes[0].path, &changes[0].previous, &changes[0].current),
        (
            &home.path.join(".claude").display().to_string(),
            &ultra(),
            &enterprise()
        )
    );
    assert_eq!(
        labels_and_requests(&home, &config),
        vec![
            ("me@example.com".to_string(), 28),
            ("me@work.example".to_string(), 1)
        ]
    );
}

#[test]
fn a_switch_set_after_an_ingest_moves_the_stored_requests() {
    let (mut home, before) = plain_home("");
    ingest_all(&mut home, &before);
    let after = Config::parse(
        &format!(
            "[directories.\"~/.claude\"]\nswitched_at = 2026-01-08T23:51:00Z\nearlier_account = \"{PERSON}/{ENTERPRISE_ORG}\""
        ),
        &home.path,
    )
    .unwrap();

    ingest_all(&mut home, &after);

    assert_eq!(
        labels_and_requests(&home, &after),
        vec![
            (format!("{PERSON}/{ENTERPRISE_ORG}"), 12),
            ("me@example.com".to_string(), 15)
        ]
    );
    let by_account: Vec<(Option<String>, u64)> = home
        .store
        .totals_by_model(None)
        .unwrap()
        .into_iter()
        .map(|row| (row.account, row.requests))
        .collect();
    let enterprise_requests: u64 = by_account
        .iter()
        .filter(|(account, _)| account.as_deref() == Some(&enterprise().to_string()))
        .map(|(_, requests)| requests)
        .sum();
    assert_eq!(enterprise_requests, 12);
}

#[test]
fn a_switch_removed_after_an_ingest_moves_the_requests_back() {
    let (mut home, switched) = switched_home("");
    ingest_all(&mut home, &switched);

    ingest_all(&mut home, &Config::default());

    assert_eq!(
        labels_and_requests(&home, &Config::default()),
        vec![
            (format!("{PERSON}/{ENTERPRISE_ORG}"), 0),
            ("me@example.com".to_string(), 27)
        ]
    );
}

#[test]
fn an_earlier_account_turned_off_after_an_ingest_keeps_its_requests_on_the_next_read() {
    let (mut home, tracked) = switched_home("");
    ingest_all(&mut home, &tracked);
    let untracked = Config::parse(
        &format!(
            "[directories.\"~/.claude\"]\nswitched_at = 2026-01-08T23:51:00Z\nearlier_account = \"{PERSON}/{ENTERPRISE_ORG}\"\n[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false"
        ),
        &home.path,
    )
    .unwrap();
    let directories = accounts::discover(&home.path, &untracked);
    accounts::read_again_for(&mut home.store, &ultra(), &directories, &untracked).unwrap();

    ingest_all(&mut home, &untracked);

    assert_eq!(home.store.request_count().unwrap(), 27);
}

#[test]
fn forgetting_the_account_before_a_seen_change_keeps_it_out_on_later_reads() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    ingest_all(&mut home, &config);
    home.store.forget(&ultra()).unwrap();
    let forgotten = Config::parse(
        &format!("[accounts.\"{PERSON}/{ULTRA_ORG}\"]\ntracking = false"),
        &home.path,
    )
    .unwrap();

    ingest_all(&mut home, &forgotten);

    assert_eq!(home.store.request_count().unwrap(), 0);
}

#[test]
fn forgetting_the_account_before_a_seen_change_keeps_it_out_when_the_directory_is_read_again() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    ingest_all(&mut home, &config);
    home.store.forget(&ultra()).unwrap();
    let forgotten = Config::parse(
        &format!("[accounts.\"{PERSON}/{ULTRA_ORG}\"]\ntracking = false"),
        &home.path,
    )
    .unwrap();
    home.store
        .forget_positions_under(&home.path.join(".claude").join("projects"))
        .unwrap();

    ingest_all(&mut home, &forgotten);

    assert_eq!(home.store.request_count().unwrap(), 0);
}

#[test]
fn requests_read_while_logged_out_move_to_the_account_that_logs_in() {
    let (mut home, config) = plain_home("");
    fs::write(home.path.join(".claude.json"), "{}").unwrap();
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ULTRA_ORG, "me@example.com"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    let connection = rusqlite::Connection::open(&home.database).unwrap();
    let without_account: i64 = connection
        .query_row(
            "SELECT count(*) FROM requests WHERE account_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (
            without_account,
            labels_and_requests(&home, &Config::default())
        ),
        (0, vec![("me@example.com".to_string(), 27)])
    );
}

#[test]
fn forgetting_a_directorys_current_login_keeps_the_earlier_accounts_history() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    ingest_all(&mut home, &config);
    home.store.forget(&enterprise()).unwrap();
    let forgotten = Config::parse(
        &format!("[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false"),
        &home.path,
    )
    .unwrap();
    fs::write(
        home.path.join(".claude.json"),
        claude_json(THIRD_ORG, "me@third.example"),
    )
    .unwrap();

    ingest_all(&mut home, &forgotten);

    assert!(labels_and_requests(&home, &forgotten).contains(&("me@example.com".to_string(), 27)));
}

#[test]
fn logging_in_after_a_logged_out_read_keeps_a_switched_directorys_history() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    ingest_all(&mut home, &config);
    fs::write(home.path.join(".claude.json"), "{}").unwrap();
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(THIRD_ORG, "me@third.example"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    assert!(
        labels_and_requests(&home, &Config::default())
            .contains(&("me@example.com".to_string(), 27))
    );
}

#[test]
fn requests_read_while_logged_out_move_to_the_login_for_an_extra_path_with_a_trailing_slash() {
    let mut home = home();
    let outside_home = home.path.parent().unwrap().to_path_buf();
    add_config_dir(
        &outside_home,
        "work",
        &outside_home.join("work").join(".claude.json"),
        "{}",
        PLAIN,
    );
    let config = Config::parse(
        &format!(
            "extra_paths = [{:?}]",
            format!("{}/", outside_home.join("work").display())
        ),
        &home.path,
    )
    .unwrap();
    ingest_all(&mut home, &config);
    fs::write(
        outside_home.join("work").join(".claude.json"),
        claude_json(ULTRA_ORG, "me@example.com"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    let connection = rusqlite::Connection::open(&home.database).unwrap();
    let without_account: i64 = connection
        .query_row(
            "SELECT count(*) FROM requests WHERE account_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(without_account, 0);
}

#[test]
fn forgetting_an_account_then_logging_in_as_another_keeps_the_history_deleted() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    home.store.forget(&ultra()).unwrap();
    let forgotten = Config::parse(
        &format!("[accounts.\"{PERSON}/{ULTRA_ORG}\"]\ntracking = false"),
        &home.path,
    )
    .unwrap();
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();

    ingest_all(&mut home, &forgotten);

    assert_eq!(home.store.request_count().unwrap(), 0);
}

#[test]
fn requests_read_while_logged_out_move_to_the_same_account_logging_back_in() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(home.path.join(".claude.json"), "{}").unwrap();
    let mut transcript = fs::OpenOptions::new()
        .append(true)
        .open(home.path.join(".claude/projects/p/s.jsonl"))
        .unwrap();
    std::io::Write::write_all(
        &mut transcript,
        request_at("r-logged-out", "2026-02-01T12:00:00.000Z").as_bytes(),
    )
    .unwrap();
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ULTRA_ORG, "me@example.com"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    assert_eq!(
        labels_and_requests(&home, &Config::default()),
        vec![("me@example.com".to_string(), 28)]
    );
}

#[test]
fn renders_accounts_with_tracking_directories_and_login_changes() {
    let (mut home, config) = switched_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\nlabel = \"Enterprise\"\ntracking = false"
    ));
    ingest_all(&mut home, &config);
    fs::create_dir_all(home.path.join(".claude-spare").join("projects")).unwrap();
    fs::write(home.path.join(".claude-spare").join(".claude.json"), "{}").unwrap();
    let directories = accounts::discover(&home.path, &config);
    let rows = accounts::list(&home.store, &directories, &config).unwrap();
    let change = gigawatts::store::LoginChange {
        path: home.path.join(".claude").display().to_string(),
        previous: enterprise(),
        current: ultra(),
        changed_at_ms: 0,
    };

    insta::assert_snapshot!(accounts::render(&rows, &[change], &directories, &home.path));
}

#[test]
fn renders_an_unreadable_directory_without_claiming_its_requests_are_stored() {
    let (home, config) = plain_home("");
    fs::write(home.path.join(".claude.json"), "{\"oauthAccount\":").unwrap();
    let directories = accounts::discover(&home.path, &config);
    let rows = accounts::list(&home.store, &directories, &config).unwrap();

    let rendered = accounts::render(&rows, &[], &directories, &home.path);

    assert!(
        !rendered.contains("stored without an account"),
        "{rendered}"
    );
}

#[test]
fn a_title_written_before_the_first_request_follows_the_sessions_account() {
    let (mut home, config) = switched_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\nstore_titles = false"
    ));

    ingest_all(&mut home, &config);

    assert_eq!(home.store.session_title(PLAIN_SESSION).unwrap(), None);
}

#[test]
fn a_title_read_after_its_sessions_requests_follows_the_stored_account() {
    let (mut home, config) = plain_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\nstore_titles = false"
    ));
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    let transcript = home
        .path
        .join(".claude")
        .join("projects")
        .join("p")
        .join("s.jsonl");
    let requests_only: String = fs::read_to_string(PLAIN)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("\"ai-title\""))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&transcript, requests_only).unwrap();
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ULTRA_ORG, "me@example.com"),
    )
    .unwrap();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(
        &mut file,
        format!(
            "{{\"aiTitle\":\"Later\",\"sessionId\":\"{PLAIN_SESSION}\",\"type\":\"ai-title\"}}\n"
        )
        .as_bytes(),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    assert_eq!(home.store.session_title(PLAIN_SESSION).unwrap(), None);
}

#[test]
fn forgetting_an_account_leaves_none_of_its_email_or_branch_and_folder_labels() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);

    home.store.forget(&ultra()).unwrap();

    let connection = rusqlite::Connection::open(&home.database).unwrap();
    let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(count("SELECT count(*) FROM labels"), 0);
    assert_eq!(
        count("SELECT count(*) FROM accounts WHERE email IS NOT NULL"),
        0
    );
    assert_eq!(count("SELECT count(*) FROM request_tools"), 0);
    assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
}

#[test]
fn forgetting_one_of_two_accounts_leaves_no_broken_references() {
    let (mut home, config) = switched_home("");
    ingest_all(&mut home, &config);

    home.store.forget(&enterprise()).unwrap();

    let connection = rusqlite::Connection::open(&home.database).unwrap();
    let broken: i64 = connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(broken, 0);
}

#[test]
fn a_directory_with_an_unreadable_login_waits_until_it_can_be_read() {
    let (mut home, config) = plain_home("");
    fs::write(home.path.join(".claude.json"), "{\"oauthAccount\":").unwrap();
    ingest_all(&mut home, &config);
    let while_unreadable = home.store.request_count().unwrap();
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ULTRA_ORG, "me@example.com"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    assert_eq!(while_unreadable, 0);
    assert_eq!(
        labels_and_requests(&home, &config),
        vec![("me@example.com".to_string(), 27)]
    );
}

#[test]
fn turning_an_earlier_account_back_on_reads_its_requests_again() {
    let (mut home, untracked) = switched_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false"
    ));
    ingest_all(&mut home, &untracked);
    let tracked = Config::parse(
        &format!(
            "[directories.\"~/.claude\"]\nswitched_at = 2026-01-08T23:51:00Z\nearlier_account = \"{PERSON}/{ENTERPRISE_ORG}\""
        ),
        &home.path,
    )
    .unwrap();
    let directories = accounts::discover(&home.path, &tracked);

    accounts::read_again_for(&mut home.store, &enterprise(), &directories, &tracked).unwrap();
    ingest_all(&mut home, &tracked);

    assert_eq!(home.store.request_count().unwrap(), 27);
}

#[test]
fn a_title_read_later_for_an_untracked_earlier_accounts_session_is_not_stored() {
    let (mut home, config) = switched_home(&format!(
        "[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false"
    ));
    let transcript = home
        .path
        .join(".claude")
        .join("projects")
        .join("p")
        .join("s.jsonl");
    let requests_only: String = fs::read_to_string(PLAIN)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("\"ai-title\""))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&transcript, requests_only).unwrap();
    ingest_all(&mut home, &config);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(
        &mut file,
        format!(
            "{{\"aiTitle\":\"Later\",\"sessionId\":\"{PLAIN_SESSION}\",\"type\":\"ai-title\"}}\n"
        )
        .as_bytes(),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    assert_eq!(home.store.session_title(PLAIN_SESSION).unwrap(), None);
}

#[test]
fn a_login_change_from_an_untracked_account_is_reported_without_its_email() {
    let (mut home, config) = plain_home(&format!(
        "[accounts.\"{PERSON}/{ULTRA_ORG}\"]\ntracking = false"
    ));
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();

    ingest_all(&mut home, &config);

    let changes = home.store.login_changes().unwrap();
    assert_eq!(
        changes
            .iter()
            .map(|change| (&change.previous, &change.current))
            .collect::<Vec<_>>(),
        vec![(&ultra(), &enterprise())]
    );
    let untracked_email = home
        .store
        .accounts()
        .unwrap()
        .into_iter()
        .find(|stored| stored.account == ultra())
        .and_then(|stored| stored.email);
    assert_eq!(untracked_email, None);
}

#[test]
fn a_session_moved_by_a_later_switch_takes_its_first_requests_account() {
    let (mut home, before) = plain_home("");
    ingest_all(&mut home, &before);
    let after = Config::parse(
        &format!(
            "[directories.\"~/.claude\"]\nswitched_at = 2026-01-08T23:51:00Z\nearlier_account = \"{PERSON}/{ENTERPRISE_ORG}\"\n[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\nstore_titles = false"
        ),
        &home.path,
    )
    .unwrap();
    ingest_all(&mut home, &after);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(
            home.path
                .join(".claude")
                .join("projects")
                .join("p")
                .join("s.jsonl"),
        )
        .unwrap();
    std::io::Write::write_all(
        &mut file,
        format!(
            "{{\"aiTitle\":\"Later\",\"sessionId\":\"{PLAIN_SESSION}\",\"type\":\"ai-title\"}}\n"
        )
        .as_bytes(),
    )
    .unwrap();

    ingest_all(&mut home, &after);

    assert_eq!(
        home.store.session_title(PLAIN_SESSION).unwrap().as_deref(),
        Some("Title 1")
    );
}

#[test]
fn a_second_login_change_leaves_the_first_accounts_requests_alone() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    for organization in [ENTERPRISE_ORG, "00000000-0000-4000-8000-0000000000c0"] {
        fs::write(
            home.path.join(".claude.json"),
            claude_json(organization, "me@example.com"),
        )
        .unwrap();
        ingest_all(&mut home, &config);
    }

    let requests: Vec<(Account, u64)> = home
        .store
        .accounts()
        .unwrap()
        .into_iter()
        .map(|stored| (stored.account, stored.requests))
        .filter(|(_, requests)| *requests > 0)
        .collect();
    assert_eq!(requests, vec![(ultra(), 27)]);
}

fn requests_by_account(home: &Home) -> Vec<(Account, u64)> {
    home.store
        .accounts()
        .unwrap()
        .into_iter()
        .map(|stored| (stored.account, stored.requests))
        .filter(|(_, requests)| *requests > 0)
        .collect()
}

fn read_everything_again(home: &mut Home, config: &Config) {
    home.store
        .forget_positions_under(&home.path.join(".claude").join("projects"))
        .unwrap();
    ingest_all(home, config);
}

#[test]
fn a_re_read_after_two_login_changes_keeps_the_first_accounts_requests() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    for organization in [ENTERPRISE_ORG, THIRD_ORG] {
        fs::write(
            home.path.join(".claude.json"),
            claude_json(organization, "me@example.com"),
        )
        .unwrap();
        ingest_all(&mut home, &config);
    }

    read_everything_again(&mut home, &config);

    assert_eq!(requests_by_account(&home), vec![(ultra(), 27)]);
}

#[test]
fn a_re_read_after_a_login_change_keeps_the_configured_switch() {
    let (mut home, config) = switched_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(THIRD_ORG, "me@third.example"),
    )
    .unwrap();
    ingest_all(&mut home, &config);

    read_everything_again(&mut home, &config);

    let mut requests = requests_by_account(&home);
    requests.sort();
    let mut expected = vec![(enterprise(), 12), (ultra(), 15)];
    expected.sort();
    assert_eq!(requests, expected);
}

#[test]
fn a_switch_set_after_a_seen_login_change_splits_at_the_switch_time() {
    let (mut home, config) = plain_home("");
    ingest_all(&mut home, &config);
    fs::write(
        home.path.join(".claude.json"),
        claude_json(ENTERPRISE_ORG, "me@work.example"),
    )
    .unwrap();
    ingest_all(&mut home, &config);
    let switched = Config::parse(
        &format!(
            "[directories.\"~/.claude\"]\nswitched_at = 2026-01-08T23:51:00Z\nearlier_account = \"{PERSON}/{ULTRA_ORG}\""
        ),
        &home.path,
    )
    .unwrap();

    ingest_all(&mut home, &switched);

    let mut requests = requests_by_account(&home);
    requests.sort();
    let mut expected = vec![(ultra(), 12), (enterprise(), 15)];
    expected.sort();
    assert_eq!(requests, expected);
}

#[test]
fn after_a_switch_a_login_that_ended_before_it_gets_no_requests() {
    let third: Account = format!("{PERSON}/{THIRD_ORG}").parse().unwrap();
    let seen = |from_ms: i64, account: &Account| SeenLogin {
        from_ms,
        account: account.clone(),
    };
    let attribution = Attribution {
        current: Some(third.clone()),
        switch: Some(Switch {
            at_ms: 300,
            earlier: enterprise(),
        }),
        seen_logins: vec![
            seen(0, &ultra()),
            seen(200, &enterprise()),
            seen(400, &third),
        ],
        ..Attribution::default()
    };

    let accounts: Vec<_> = [100, 350, 500]
        .map(|time_ms| attribution.account_at(Some(time_ms)).cloned())
        .into();

    assert_eq!(
        accounts,
        vec![Some(enterprise()), Some(third.clone()), Some(third)]
    );
}

#[test]
fn a_session_moved_to_an_untracked_account_leaves_nothing_behind() {
    let (mut home, before) = plain_home("");
    ingest_all(&mut home, &before);
    let after = Config::parse(
        &format!(
            "[directories.\"~/.claude\"]\nswitched_at = 2099-01-01T00:00:00Z\nearlier_account = \"{PERSON}/{ENTERPRISE_ORG}\"\n[accounts.\"{PERSON}/{ENTERPRISE_ORG}\"]\ntracking = false"
        ),
        &home.path,
    )
    .unwrap();

    ingest_all(&mut home, &after);

    let connection = rusqlite::Connection::open(&home.database).unwrap();
    let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(count("SELECT count(*) FROM requests"), 0);
    assert_eq!(count("SELECT count(*) FROM sessions"), 0);
    assert_eq!(count("SELECT count(*) FROM session_models"), 0);
    assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
}
