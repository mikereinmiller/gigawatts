use std::fs::{self, OpenOptions};
use std::io::Write as _;

use gigawatts::accounts;
use gigawatts::config::Config;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

const PLAIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/projects/plain/00000001-0000-4000-8000-194e1652b207.jsonl"
);

fn request_line(request_id: &str) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"s9","requestId":"{request_id}","timestamp":"2026-01-09T12:00:00.000Z","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":10}}}}}}"#
    ) + "\n"
}

fn append(path: &std::path::Path, text: &str) {
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

#[test]
fn a_file_event_reads_only_that_file_under_the_path_the_full_scan_used() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let projects = home.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    let changed = projects.join("s.jsonl");
    fs::copy(PLAIN, &changed).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let config = Config::parse("", &home).unwrap();
    let table = PriceTable::built_in();
    let directories = accounts::discover(&home, &config);
    accounts::ingest(&mut store, &directories, &config, &table, &Resolver).unwrap();
    let before = store.request_count().unwrap();
    append(&changed, &request_line("req_new_1"));
    append(
        &projects.join("untouched.jsonl"),
        &request_line("req_new_2"),
    );
    let event_path = changed.canonicalize().unwrap();

    accounts::ingest_files(
        &mut store,
        &directories,
        &config,
        &table,
        &Resolver,
        std::slice::from_ref(&event_path),
    )
    .unwrap();

    assert_eq!(store.request_count().unwrap(), before + 1);
    let position = store.position(&changed.to_string_lossy()).unwrap().unwrap();
    assert_eq!(position.offset, fs::metadata(&changed).unwrap().len());
    if event_path != changed {
        assert_eq!(store.position(&event_path.to_string_lossy()).unwrap(), None);
    }
}

#[test]
fn a_file_gone_before_its_event_is_read_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let config = Config::parse("", &home).unwrap();
    let directories = accounts::discover(&home, &config);
    let gone = home.join(".claude").join("projects").join("gone.jsonl");

    let summary = accounts::ingest_files(
        &mut store,
        &directories,
        &config,
        &PriceTable::built_in(),
        &Resolver,
        &[gone],
    )
    .unwrap();

    assert_eq!(summary.files, 0);
}

#[test]
fn an_unreadable_file_in_a_batch_doesnt_stop_the_files_after_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let projects = home.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    let locked = projects.join("a.jsonl");
    append(&locked, &request_line("req_locked"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = projects.join("b.jsonl");
    append(&readable, &request_line("req_readable"));
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let config = Config::parse("", &home).unwrap();
    let directories = accounts::discover(&home, &config);

    let read = accounts::ingest_files(
        &mut store,
        &directories,
        &config,
        &PriceTable::built_in(),
        &Resolver,
        &[locked, readable],
    );

    assert!(read.is_err());
    assert_eq!(store.request_count().unwrap(), 1);
}
