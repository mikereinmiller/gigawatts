use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use gigawatts::accounts::Attribution;
use gigawatts::contract::DataRow;
use gigawatts::ingest::{Summary, ingest_tree};
use gigawatts::pricing::PriceTable;
use gigawatts::sources::git::Resolver;
use gigawatts::sources::transcript::Corpus;
use gigawatts::store::Store;

const PLAIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/projects/plain/00000001-0000-4000-8000-194e1652b207.jsonl"
);
const PLAIN_UNIQUE_REQUEST_IDS: u64 = 27;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/projects");

fn ingest(store: &mut Store, projects: &Path) -> Summary {
    ingest_tree(
        store,
        projects,
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .expect("ingest")
}

fn request_line(request_id: &str, output_tokens: u64) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"s1","requestId":"{request_id}","timestamp":"2026-01-08T12:00:00.000Z","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":2,"output_tokens":{output_tokens}}}}}}}"#
    ) + "\n"
}

struct Scratch {
    _dir: tempfile::TempDir,
    projects: std::path::PathBuf,
    store: Store,
}

fn scratch() -> Scratch {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(projects.join("p")).unwrap();
    let store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    Scratch {
        _dir: dir,
        projects,
        store,
    }
}

fn opus_output(store: &Store) -> u64 {
    let totals = store.totals_by_model(None).unwrap();
    totals.iter().map(|total| total.output_tokens).sum()
}

#[test]
fn ingesting_again_after_an_append_stores_each_request_id_once() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(projects.join("p")).unwrap();
    let transcript = projects.join("p").join("s.jsonl");
    let bytes = fs::read(PLAIN).unwrap();
    let cut = bytes.len() / 2;
    fs::write(&transcript, &bytes[..cut]).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

    ingest(&mut store, &projects);
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap()
        .write_all(&bytes[cut..])
        .unwrap();
    ingest(&mut store, &projects);

    assert_eq!(store.request_count().unwrap(), PLAIN_UNIQUE_REQUEST_IDS);
}

#[test]
fn an_unchanged_file_is_not_read_again() {
    let mut scratch = scratch();
    fs::write(
        scratch.projects.join("p").join("s.jsonl"),
        request_line("r1", 10),
    )
    .unwrap();

    let first = ingest(&mut scratch.store, &scratch.projects);
    let second = ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(
        (first.files_with_new_lines, second.files_with_new_lines),
        (1, 0)
    );
}

#[test]
fn a_partial_last_line_is_read_once_it_is_complete() {
    let mut scratch = scratch();
    let transcript = scratch.projects.join("p").join("s.jsonl");
    let line = request_line("r1", 10);
    fs::write(&transcript, &line[..line.len() / 2]).unwrap();

    ingest(&mut scratch.store, &scratch.projects);
    let before = scratch.store.request_count().unwrap();
    fs::write(&transcript, &line).unwrap();
    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!((before, scratch.store.request_count().unwrap()), (0, 1));
}

#[test]
fn a_request_streamed_over_several_lines_keeps_its_final_output() {
    let mut scratch = scratch();
    let lines = [
        request_line("r1", 6),
        request_line("r1", 133),
        request_line("r1", 133),
    ]
    .concat();
    fs::write(scratch.projects.join("p").join("s.jsonl"), lines).unwrap();

    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(scratch.store.request_count().unwrap(), 1);
    assert_eq!(opus_output(&scratch.store), 133);
}

#[test]
fn a_repeated_request_in_another_file_counts_once() {
    let mut scratch = scratch();
    fs::write(
        scratch.projects.join("p").join("s.jsonl"),
        request_line("r1", 50),
    )
    .unwrap();
    fs::write(
        scratch.projects.join("p").join("t.jsonl"),
        request_line("r1", 50),
    )
    .unwrap();

    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(opus_output(&scratch.store), 50);
}

#[test]
fn skips_transcripts_claude_code_set_aside() {
    let mut scratch = scratch();
    let folder = scratch.projects.join("p");
    fs::write(folder.join("s.jsonl"), request_line("r1", 10)).unwrap();
    fs::write(
        folder.join("s.superseded-1767916011592.jsonl"),
        request_line("r2", 10),
    )
    .unwrap();
    fs::write(
        folder.join("s.jsonl.orphaned-1767916011592.jsonl"),
        request_line("r3", 10),
    )
    .unwrap();

    let summary = ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(summary.files, 1);
    assert_eq!(scratch.store.request_count().unwrap(), 1);
}

#[test]
fn a_rewritten_file_is_read_from_the_start() {
    let mut scratch = scratch();
    let transcript = scratch.projects.join("p").join("s.jsonl");
    fs::write(
        &transcript,
        [request_line("r1", 10), request_line("r2", 10)].concat(),
    )
    .unwrap();
    ingest(&mut scratch.store, &scratch.projects);

    fs::write(
        &transcript,
        [
            request_line("r3", 10),
            request_line("r4", 10),
            request_line("r5", 10),
        ]
        .concat(),
    )
    .unwrap();
    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(scratch.store.request_count().unwrap(), 5);
}

fn total(
    model: &str,
    requests: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost_usd: f64,
) -> DataRow {
    DataRow {
        account: None,
        account_label: "No account".to_string(),
        key: Some(model.to_string()),
        label: model.to_string(),
        not_in_transcript: false,
        status: None,
        requests,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        web_searches: 0,
        cost_usd: Some(cost_usd),
        paid_for: None,
    }
}

#[test]
fn totals_by_model_match_the_fixture_transcripts() {
    let mut scratch = scratch();
    let corpus = Corpus::read(Path::new(FIXTURES)).unwrap();
    let table = PriceTable::built_in();
    let priced = |model: &str| -> f64 {
        corpus
            .requests
            .values()
            .filter(|request| request.model == model)
            .map(|request| table.price(request).unwrap())
            .sum()
    };

    ingest(&mut scratch.store, Path::new(FIXTURES));
    let mut totals = scratch.store.totals_by_model(None).unwrap();
    totals.retain(|total| !total.not_in_transcript);
    for total in &mut totals {
        let expected = priced(&total.label);
        let stored = total.cost_usd.unwrap();
        assert!(
            (stored - expected).abs() < 1e-9,
            "{}: {stored} vs {expected}",
            total.label
        );
        total.cost_usd = Some(expected);
    }

    assert_eq!(
        totals,
        vec![
            total(
                "claude-opus-5-5",
                322,
                662,
                252107,
                100950580,
                1262266,
                priced("claude-opus-5-5")
            ),
            total(
                "claude-fable-5-1",
                45,
                1232,
                113351,
                1843991,
                518733,
                priced("claude-fable-5-1")
            ),
            total(
                "claude-haiku-5-5",
                6,
                12,
                11548,
                334050,
                110697,
                priced("claude-haiku-5-5")
            ),
        ]
    );
}

fn tool_line(request_id: &str, output_tokens: u64, tool: &str) -> String {
    request_line(request_id, output_tokens).replace(
        r#""usage":"#,
        &format!(
            r#""content":[{{"type":"tool_use","id":"t","name":"{tool}","input":{{}}}}],"usage":"#
        ),
    )
}

#[test]
fn a_request_keeps_the_tools_from_each_of_its_lines() {
    let mut scratch = scratch();
    let lines = [
        tool_line("r1", 10, "Read"),
        tool_line("r1", 10, "Bash"),
        tool_line("r1", 10, "Read"),
    ]
    .concat();
    fs::write(scratch.projects.join("p").join("s.jsonl"), lines).unwrap();

    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(
        scratch.store.request_tools("r1").unwrap(),
        vec!["Bash", "Read"]
    );
}

#[test]
fn a_new_price_override_reprices_requests_already_stored() {
    let mut scratch = scratch();
    let line = request_line("r1", 10).replace("claude-opus-5-5", "claude-opus-5-6");
    fs::write(scratch.projects.join("p").join("s.jsonl"), line).unwrap();
    ingest(&mut scratch.store, &scratch.projects);
    let before = scratch.store.totals_by_model(None).unwrap()[0].cost_usd;
    let config = gigawatts::config::Config::parse(
        "[prices.claude-opus-5-6]\ninput = 6.0\ncache_write_5m = 7.5\ncache_write_1h = 12.0\ncache_read = 0.6\noutput = 30.0",
        Path::new("/home/me"),
    )
    .unwrap();

    let repriced = scratch
        .store
        .reprice_if_changed(&config.price_table())
        .unwrap();
    let again = scratch
        .store
        .reprice_if_changed(&config.price_table())
        .unwrap();

    let after = scratch.store.totals_by_model(None).unwrap()[0]
        .cost_usd
        .unwrap();
    assert_eq!(before, None);
    assert_eq!((repriced, again), (true, false));
    assert!((after - 0.000312).abs() < 1e-12, "{after}");
}

#[test]
fn the_database_and_its_folder_are_readable_only_by_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("share").join("gigawatts");
    let path = folder.join("history.sqlite3");

    let store = Store::open(&path).unwrap();
    store.request_count().unwrap();

    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!((mode(&folder), mode(&path)), (0o700, 0o600));
}

#[test]
fn a_request_without_a_usable_timestamp_is_counted_as_unrecognized() {
    let mut scratch = scratch();
    let lines = [
        r#"{"type":"assistant","sessionId":"s1","requestId":"r-none","message":{"model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":5}}}"#,
        r#"{"type":"assistant","sessionId":"s1","requestId":"r-bad","timestamp":"yesterday","message":{"model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":5}}}"#,
    ];
    fs::write(
        scratch.projects.join("p").join("s.jsonl"),
        lines.join("\n") + "\n",
    )
    .unwrap();

    let summary = ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(scratch.store.request_count().unwrap(), 0);
    assert_eq!(
        summary.unrecognized.get("assistant without a timestamp"),
        Some(&2)
    );
}

#[test]
fn upgrading_keeps_a_directorys_recorded_login_change() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("history.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    for migration in ["001_history.sql", "002_accounts.sql", "003_repricing.sql"] {
        let sql = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/store/migrations")
                .join(migration),
        )
        .unwrap();
        connection.execute_batch(&sql).unwrap();
    }
    connection
        .execute_batch(
            "PRAGMA user_version = 3;
             INSERT INTO accounts (id, account_uuid, organization_uuid) VALUES (1, 'a', 'one'), (2, 'a', 'two');
             INSERT INTO directory_logins (path, account_id, previous_account_id, changed_at_ms)
             VALUES ('/home/me/.claude', 2, 1, 1000);",
        )
        .unwrap();
    drop(connection);

    let store = Store::open(&database).unwrap();

    let seen: Vec<(i64, String)> = store
        .seen_logins("/home/me/.claude")
        .unwrap()
        .into_iter()
        .map(|login| (login.from_ms, login.account.organization_uuid))
        .collect();
    assert_eq!(
        seen,
        vec![(0, "one".to_string()), (1000, "two".to_string())]
    );
}

#[test]
fn upgrading_to_attribution_reads_every_transcript_again_and_reconciles_it() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(projects.join("p")).unwrap();
    let transcript = projects.join("p").join("s1.jsonl");
    fs::write(
        &transcript,
        r#"{"type":"assistant","sessionId":"s1","requestId":"r1","timestamp":"2026-01-08T12:00:00.000Z","cwd":"/Users/nobody/gone","gitBranch":"main","message":{"model":"claude-opus-5-5","usage":{"input_tokens":100000,"output_tokens":10000}}}
{"type":"cost-state","sessionId":"s1","totalCostUSD":0.6,"modelUsage":{"claude-opus-5-5":{"inputTokens":100000,"outputTokens":10000,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.6}}}
"#,
    )
    .unwrap();
    let database = dir.path().join("history.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    for migration in [
        "001_history.sql",
        "002_accounts.sql",
        "003_repricing.sql",
        "004_login_history.sql",
    ] {
        let sql = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/store/migrations")
                .join(migration),
        )
        .unwrap();
        connection.execute_batch(&sql).unwrap();
    }
    connection
        .execute(
            "INSERT INTO file_positions (path, device, inode, first_line_hash, offset)
             VALUES (?1, 1, 1, NULL, 9999)",
            [transcript.to_string_lossy()],
        )
        .unwrap();
    connection
        .execute_batch(
            "PRAGMA user_version = 4;
             INSERT INTO labels (id, text) VALUES (1, 'claude-opus-5-5'), (2, '/Users/nobody/gone'), (3, 'main');
             INSERT INTO sessions (id, uuid, total_cost_usd) VALUES (1, 's1', 0.6);
             INSERT INTO session_models VALUES (1, 1, 100000, 10000, 0, 0, 0, 0, 0, 0, 0, 0.6);
             INSERT INTO requests (request_id, session_id, day, model_label, cwd_label, git_branch_label,
                                   is_sidechain, input_tokens, output_tokens, thinking_tokens,
                                   cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
                                   web_searches, web_fetches, cost_usd)
             VALUES ('r1', 1, '2026-01-08', 1, 2, 3, 0, 100000, 10000, 0, 0, 0, 0, 0, 0, 0.6);
             INSERT INTO daily_totals (day, cwd_label, model_label, requests, input_tokens, output_tokens,
                                       thinking_tokens, cache_read_tokens, cache_write_5m_tokens,
                                       cache_write_1h_tokens, web_searches, web_fetches, cost_usd)
             VALUES ('2026-01-08', 2, 1, 1, 100000, 10000, 0, 0, 0, 0, 0, 0, 0.6);",
        )
        .unwrap();
    drop(connection);

    let mut store = Store::open(&database).unwrap();
    assert_eq!(store.position(&transcript.to_string_lossy()).unwrap(), None);
    ingest(&mut store, &projects);

    assert_eq!(store.request_count().unwrap(), 1);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let upgraded: (String, String, String) = connection
        .query_row(
            "SELECT directories.repo_name, branch.text, sessions.cost_status
             FROM daily_totals
             JOIN directories ON directories.cwd_label = daily_totals.cwd_label
             JOIN labels AS branch ON branch.id = daily_totals.branch_label
             JOIN sessions ON sessions.uuid = 's1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        upgraded,
        ("gone".to_string(), "main".to_string(), "exact".to_string())
    );
}

#[test]
fn upgrading_to_attribution_reconciles_a_session_whose_transcript_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let database = dir.path().join("history.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    for migration in [
        "001_history.sql",
        "002_accounts.sql",
        "003_repricing.sql",
        "004_login_history.sql",
    ] {
        let sql = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/store/migrations")
                .join(migration),
        )
        .unwrap();
        connection.execute_batch(&sql).unwrap();
    }
    connection
        .execute(
            "INSERT INTO store_settings (key, value) VALUES ('price_table', ?1)",
            [PriceTable::built_in().fingerprint()],
        )
        .unwrap();
    connection
        .execute_batch(
            "PRAGMA user_version = 4;
             INSERT INTO labels (id, text) VALUES (1, 'claude-opus-5-5'), (2, '/Users/nobody/gone'), (3, 'main');
             INSERT INTO sessions (id, uuid, total_cost_usd) VALUES (1, 's1', 1.0);
             INSERT INTO session_models VALUES (1, 1, 150000, 12000, 0, 0, 0, 0, 0, 0, 0, 1.0);
             INSERT INTO requests (request_id, session_id, day, model_label, cwd_label, git_branch_label,
                                   is_sidechain, input_tokens, output_tokens, thinking_tokens,
                                   cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
                                   web_searches, web_fetches, cost_usd)
             VALUES ('r1', 1, '2026-01-08', 1, 2, 3, 0, 100000, 10000, 0, 0, 0, 0, 0, 0, 0.6);",
        )
        .unwrap();
    drop(connection);

    let mut store = Store::open(&database).unwrap();
    store.reprice_if_changed(&PriceTable::built_in()).unwrap();
    ingest(&mut store, &projects);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let reconciled: (String, i64) = connection
        .query_row(
            "SELECT sessions.cost_status, not_in_transcript.input_tokens
             FROM sessions JOIN not_in_transcript ON not_in_transcript.session_id = sessions.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(reconciled, ("exact".to_string(), 50_000));
    let repo: String = connection
        .query_row(
            "SELECT repo_name FROM directories
             JOIN labels ON labels.id = directories.cwd_label
             WHERE labels.text = '/Users/nobody/gone'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repo, "gone");
}

#[test]
fn upgrading_to_attribution_labels_head_from_a_gone_transcript_detached() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let database = dir.path().join("history.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    for migration in [
        "001_history.sql",
        "002_accounts.sql",
        "003_repricing.sql",
        "004_login_history.sql",
    ] {
        let sql = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/store/migrations")
                .join(migration),
        )
        .unwrap();
        connection.execute_batch(&sql).unwrap();
    }
    connection
        .execute_batch(
            "PRAGMA user_version = 4;
             INSERT INTO labels (id, text) VALUES (1, 'claude-opus-5-5'), (2, '/Users/nobody/gone'), (3, 'HEAD');
             INSERT INTO sessions (id, uuid) VALUES (1, 's1');
             INSERT INTO requests (request_id, session_id, day, model_label, cwd_label, git_branch_label,
                                   is_sidechain, input_tokens, output_tokens, thinking_tokens,
                                   cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
                                   web_searches, web_fetches, cost_usd)
             VALUES ('r1', 1, '2026-01-08', 1, 2, 3, 0, 100000, 10000, 0, 0, 0, 0, 0, 0, 0.6);",
        )
        .unwrap();
    drop(connection);

    let mut store = Store::open(&database).unwrap();
    ingest(&mut store, &projects);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let branches: Vec<String> = connection
        .prepare(
            "SELECT branch.text FROM daily_totals
             JOIN labels AS branch ON branch.id = daily_totals.branch_label",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(branches, vec!["detached".to_string()]);
}

fn title_line(kind: &str, field: &str, title: &str) -> String {
    format!(r#"{{"type":"{kind}","sessionId":"s1","{field}":"{title}"}}"#) + "\n"
}

fn ai_title(title: &str) -> String {
    title_line("ai-title", "aiTitle", title)
}

fn custom_title(title: &str) -> String {
    title_line("custom-title", "customTitle", title)
}

fn append(transcript: &Path, text: &str) {
    OpenOptions::new()
        .append(true)
        .open(transcript)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

#[test]
fn a_session_renamed_by_its_user_is_titled_with_that_name() {
    let mut scratch = scratch();
    fs::write(
        scratch.projects.join("p").join("s1.jsonl"),
        request_line("r1", 10) + &custom_title("Login page"),
    )
    .unwrap();

    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(
        scratch.store.session_title("s1").unwrap().as_deref(),
        Some("Login page")
    );
}

#[test]
fn the_latest_rename_is_the_title() {
    let mut scratch = scratch();
    fs::write(
        scratch.projects.join("p").join("s1.jsonl"),
        request_line("r1", 10) + &custom_title("Login page") + &custom_title("Signup page"),
    )
    .unwrap();

    ingest(&mut scratch.store, &scratch.projects);

    assert_eq!(
        scratch.store.session_title("s1").unwrap().as_deref(),
        Some("Signup page")
    );
}

#[test]
fn a_rename_wins_over_the_generated_title_whichever_comes_first() {
    for (first, later) in [
        (custom_title("Login page"), ai_title("Fix the form")),
        (ai_title("Fix the form"), custom_title("Login page")),
    ] {
        let mut scratch = scratch();
        let transcript = scratch.projects.join("p").join("s1.jsonl");
        fs::write(&transcript, request_line("r1", 10) + &first).unwrap();
        ingest(&mut scratch.store, &scratch.projects);
        append(&transcript, &later);

        ingest(&mut scratch.store, &scratch.projects);

        assert_eq!(
            scratch.store.session_title("s1").unwrap().as_deref(),
            Some("Login page"),
            "{first}{later}"
        );
    }
}
