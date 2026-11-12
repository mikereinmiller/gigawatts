use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn gigawatts(home: Option<&Path>, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gigawatts"));
    command
        .args(args)
        .current_dir(cwd)
        .env_remove("HOME")
        .env("PATH", path_with_stand_in_claude());
    if let Some(home) = home {
        command.env("HOME", home);
    }
    command.output().unwrap()
}

#[test]
fn without_home_nothing_is_written_and_the_command_fails() {
    let dir = tempfile::tempdir().unwrap();

    let output = gigawatts(None, dir.path(), &["data"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("HOME isn't set"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn editing_a_symlinked_config_writes_through_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let dotfiles = dir.path().join("dotfiles");
    let extra = dir.path().join("work-claude");
    fs::create_dir_all(home.join(".config").join("gigawatts")).unwrap();
    fs::create_dir_all(&dotfiles).unwrap();
    fs::create_dir_all(extra.join("projects")).unwrap();
    let target = dotfiles.join("config.toml");
    fs::write(&target, "# kept in dotfiles\n").unwrap();
    let link = home.join(".config").join("gigawatts").join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let output = gigawatts(
        Some(&home),
        dir.path(),
        &["accounts", "add", &extra.to_string_lossy()],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let written = fs::read_to_string(&target).unwrap();
    assert!(written.contains("# kept in dotfiles\n"), "{written}");
    assert!(written.contains("work-claude"), "{written}");
}

#[test]
fn schema_prints_the_snapshot_data_and_limits_schemas() {
    let dir = tempfile::tempdir().unwrap();

    let output = gigawatts(Some(dir.path()), dir.path(), &["schema"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let schemas: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schemas["snapshot"]["title"], "Snapshot");
    assert_eq!(schemas["data"]["title"], "DataResponse");
    assert_eq!(schemas["limits"]["title"], "LimitsResponse");
    for name in ["snapshot", "data", "limits"] {
        let required = schemas[name]["required"].as_array().unwrap();
        assert!(required.contains(&"schema_version".into()), "{name}");
    }
    assert_eq!(
        schemas["snapshot"]["properties"]["engine_version"]["type"],
        "string"
    );
    let account_fields = schemas["snapshot"]["$defs"]["AccountToday"]["required"]
        .as_array()
        .unwrap();
    assert!(
        account_fields.contains(&"account".into()),
        "{account_fields:?}"
    );
}

#[test]
fn ingest_says_it_read_the_rest_when_one_transcript_is_unreadable() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = home.join(".claude").join("projects").join("p");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    let locked = project.join("a.jsonl");
    fs::write(&locked, "").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    fs::write(project.join("b.jsonl"), "").unwrap();

    let output = gigawatts(Some(&home), dir.path(), &["ingest"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("gigawatts: read every other transcript, but can't read ")
            && stderr.contains("a.jsonl"),
        "{stderr}"
    );
}

fn path_with_stand_in_claude() -> String {
    format!(
        "{}/tests/bin:{}",
        env!("CARGO_MANIFEST_DIR"),
        std::env::var("PATH").unwrap_or_default()
    )
}

const ACCOUNT: &str = "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0";

fn logged_in_home(transcript: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let projects = home.path().join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    fs::write(projects.join("s1.jsonl"), transcript).unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    home
}

fn opus_request(request: &str, timestamp: &str, input: u64, output: u64) -> String {
    serde_json::json!({
        "type": "assistant",
        "sessionId": "s1",
        "requestId": request,
        "timestamp": timestamp,
        "message": {
            "model": "claude-opus-5-5",
            "usage": {"input_tokens": input, "output_tokens": output}
        }
    })
    .to_string()
        + "\n"
}

#[test]
fn data_json_prints_the_table_rows_as_a_data_response() {
    let transcript = opus_request("r1", "2026-01-05T12:00:00Z", 1_000_000, 0)
        + &opus_request("r2", "2026-01-05T13:00:00Z", 0, 100_000);
    let home = logged_in_home(&transcript);
    let ingested = gigawatts(Some(home.path()), home.path(), &["ingest"]);
    assert_eq!(ingested.status.code(), Some(0), "{ingested:?}");

    let output = gigawatts(
        Some(home.path()),
        home.path(),
        &["data", "--range", "all", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let response: gigawatts::contract::DataResponse =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response.schema_version, 2);
    assert_eq!(response.range, gigawatts::contract::Range::All);
    assert_eq!(response.level, gigawatts::contract::Level::Model);
    assert_eq!(response.since, None);
    let [row] = response.rows.as_slice() else {
        panic!("{:?}", response.rows);
    };
    assert_eq!(row.account.as_deref(), Some(ACCOUNT));
    assert_eq!(row.label, "claude-opus-5-5");
    assert_eq!(row.requests, 2);
    assert_eq!(row.input_tokens, 1_000_000);
    assert_eq!(row.output_tokens, 100_000);
    let table = gigawatts(Some(home.path()), home.path(), &["data", "--range", "all"]);
    let cost = format!("${:.2}", row.cost_usd.unwrap());
    assert!(
        String::from_utf8_lossy(&table.stdout).contains(&cost),
        "{cost} not in {table:?}"
    );
}

#[test]
fn data_limits_json_prints_each_accounts_latest_limits() {
    let home = logged_in_home("");
    let database = home.path().join(".local/share/gigawatts/history.sqlite3");
    fs::create_dir_all(database.parent().unwrap()).unwrap();
    let mut store = gigawatts::store::Store::open(&database).unwrap();
    let response = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage/ultra/get-usage.json"),
    )
    .unwrap();
    store
        .record_limits(
            &ACCOUNT.parse().unwrap(),
            &gigawatts::store::LimitsReading {
                fetched_at_ms: 1_793_820_000_000,
                source: gigawatts::contract::LimitSource::GetUsage,
                limits: gigawatts::sources::get_usage::parse_usage(&response)
                    .unwrap()
                    .limits,
            },
        )
        .unwrap();
    drop(store);

    let output = gigawatts(
        Some(home.path()),
        home.path(),
        &["data", "--limits", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed["schema_version"], 2);
    let account = &printed["accounts"][0];
    assert_eq!(account["account"], ACCOUNT);
    assert_eq!(account["account_label"], ACCOUNT);
    assert_eq!(account["source"], "get_usage");
    assert_eq!(account["fetched_at_ms"], 1_793_820_000_000_i64);
    let kinds: Vec<(&str, f64)> = account["limits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|limit| {
            (
                limit["kind"].as_str().unwrap(),
                limit["percent"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("session", 2.0),
            ("weekly_all", 63.0),
            ("weekly_scoped", 90.0)
        ]
    );
}

#[test]
fn data_since_and_until_print_that_range() {
    let transcript = opus_request("r1", "2026-01-04T12:00:00Z", 1_000, 0)
        + &opus_request("r2", "2026-01-05T12:00:00Z", 1_000, 0)
        + &opus_request("r3", "2026-01-06T12:00:00Z", 1_000, 0);
    let home = logged_in_home(&transcript);
    let ingested = gigawatts(Some(home.path()), home.path(), &["ingest"]);
    assert_eq!(ingested.status.code(), Some(0), "{ingested:?}");

    let output = gigawatts(
        Some(home.path()),
        home.path(),
        &[
            "data",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let response: gigawatts::contract::DataResponse =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response.range, gigawatts::contract::Range::Custom);
    assert_eq!(response.since.as_deref(), Some("2026-01-05"));
    assert_eq!(response.until.as_deref(), Some("2026-01-05"));
    assert_eq!(response.rows.iter().map(|row| row.requests).sum::<u64>(), 1);
}

#[test]
fn data_rejects_a_date_that_isnt_on_the_calendar() {
    let home = logged_in_home("");

    let output = gigawatts(
        Some(home.path()),
        home.path(),
        &["data", "--since", "2026-02-30"],
    );

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("2026-02-30 isn't a date"), "{stderr}");
}

#[test]
fn data_rejects_a_range_that_ends_before_it_starts() {
    let home = logged_in_home("");

    let output = gigawatts(
        Some(home.path()),
        home.path(),
        &["data", "--since", "2026-01-06", "--until", "2026-01-05"],
    );

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "gigawatts: --until 2026-01-05 is before --since 2026-01-06.\n"
    );
}

#[test]
fn data_since_conflicts_with_range() {
    let home = logged_in_home("");

    let output = gigawatts(
        Some(home.path()),
        home.path(),
        &["data", "--range", "week", "--since", "2026-01-05"],
    );

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot be used with"),
        "{output:?}"
    );
}
