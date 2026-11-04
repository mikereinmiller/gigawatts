use std::fs::{self, OpenOptions};
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gigawatts::engine::control::{self, Command as EngineCommand};
use gigawatts::engine::lock::EngineLock;
use gigawatts::engine::{Paths, version};

fn gigawatts(home: &Path, args: &[&str]) -> Output {
    gigawatts_with_claude_in(home, &stand_in_bin(), args)
}

fn stand_in_bin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bin")
}

fn gigawatts_with_claude_in(home: &Path, claude_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gigawatts"))
        .args(args)
        .env("HOME", home)
        .env("PATH", path_with_claude_in(claude_dir))
        .output()
        .unwrap()
}

fn path_with_claude_in(claude_dir: &Path) -> String {
    format!(
        "{}:{}",
        claude_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

struct Home {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        gigawatts(&self.path, &["stop"]);
    }
}

fn today_noon_utc() -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', date('now', 'localtime') || 'T12:00:00', 'utc')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn append_request(transcript: &Path, request_id: &str) {
    let line = format!(
        r#"{{"type":"assistant","sessionId":"s1","requestId":"{request_id}","timestamp":"{}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1000,"output_tokens":500}}}}}}"#,
        today_noon_utc()
    ) + "\n";
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(transcript)
        .unwrap();
    file.write_all(line.as_bytes()).unwrap();
}

fn requests_today(snapshot: &Path) -> Option<u64> {
    let text = fs::read_to_string(snapshot).ok()?;
    let snapshot: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(
        snapshot["accounts"]
            .as_array()?
            .iter()
            .filter_map(|account| account["requests"].as_u64())
            .sum(),
    )
}

fn wait_for_requests(snapshot: &Path, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if requests_today(snapshot) == Some(expected) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "the snapshot never showed {expected} requests today: {:?}",
        fs::read_to_string(snapshot)
    );
}

#[test]
fn the_engine_keeps_the_snapshot_current_until_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home {
        path: dir.path().join("h"),
        _dir: dir,
    };
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        home.path.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    let transcript = projects.join("s1.jsonl");
    append_request(&transcript, "req_1");
    let paths = Paths::new(&home.path);

    let started = gigawatts(&home.path, &["engine"]);
    assert_eq!(stdout(&started), "Started the engine.\n", "{started:?}");
    wait_for_requests(&paths.snapshot(), 1);

    append_request(&transcript, "req_2");
    wait_for_requests(&paths.snapshot(), 2);

    append_request(&projects.join("s2.jsonl"), "req_3");
    wait_for_requests(&paths.snapshot(), 3);

    let again = gigawatts(&home.path, &["engine"]);
    assert_eq!(stdout(&again), "The engine is already running.\n");

    let stopped = gigawatts(&home.path, &["stop"]);
    assert_eq!(stdout(&stopped), "Stopped the engine.\n", "{stopped:?}");
    assert!(!EngineLock::is_held(&paths.lock()).unwrap());
    assert!(!paths.socket().exists());
    let stopped_again = gigawatts(&home.path, &["stop"]);
    assert_eq!(stdout(&stopped_again), "The engine isn't running.\n");
}

#[test]
fn a_second_foreground_engine_exits_while_one_runs() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home {
        path: dir.path().join("h"),
        _dir: dir,
    };
    fs::create_dir_all(home.path.join(".claude").join("projects")).unwrap();
    assert_eq!(
        stdout(&gigawatts(&home.path, &["engine"])),
        "Started the engine.\n"
    );

    let second = gigawatts(&home.path, &["engine", "--foreground"]);

    assert_eq!(second.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&second.stderr).contains("already running"));
}

fn home_with_login() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = Home {
        path: dir.path().join("h"),
        _dir: dir,
    };
    fs::create_dir_all(home.path.join(".claude")).unwrap();
    fs::write(
        home.path.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    home
}

struct OldEngine(std::sync::mpsc::Receiver<Vec<String>>);

impl OldEngine {
    fn heard(&self) -> Vec<String> {
        self.0
            .recv_timeout(Duration::from_secs(15))
            .expect("the old engine was never told to stop")
    }
}

fn old_engine(paths: &Paths) -> OldEngine {
    fs::create_dir_all(paths.data_dir()).unwrap();
    let lock = EngineLock::acquire(&paths.lock()).unwrap().unwrap();
    let listener = UnixListener::bind(paths.socket()).unwrap();
    let socket = paths.socket();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut lock = Some(lock);
        let mut heard = Vec::new();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            heard.push(line.trim().to_string());
            if line.trim() == "stop" {
                writeln!(stream, "stopping").unwrap();
                fs::remove_file(&socket).unwrap();
                drop(lock.take());
                let _ = sender.send(heard);
                return;
            }
            writeln!(stream, "0.0.1").unwrap();
        }
    });
    OldEngine(receiver)
}

fn running_version(paths: &Paths) -> Option<String> {
    control::send(&paths.socket(), EngineCommand::Version).ok()
}

#[test]
fn starting_the_engine_replaces_an_older_one() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let old = old_engine(&paths);

    let started = gigawatts(&home.path, &["engine"]);

    assert_eq!(
        stdout(&started),
        format!("Stopped engine version 0.0.1 and started {}.\n", version()),
        "{started:?}"
    );
    assert_eq!(old.heard(), vec!["version", "stop"]);
    assert_eq!(running_version(&paths).as_deref(), Some(version().as_str()));
}

#[test]
fn any_command_that_opens_the_history_replaces_an_older_engine_first() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let old = old_engine(&paths);

    let doctor = gigawatts(&home.path, &["doctor"]);

    assert_eq!(doctor.status.code(), Some(0), "{doctor:?}");
    assert_eq!(old.heard(), vec!["version", "stop"]);
    assert_eq!(running_version(&paths).as_deref(), Some(version().as_str()));
}

#[test]
fn ingest_asks_a_running_engine_to_read_instead_of_writing_beside_it() {
    let home = home_with_login();
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    gigawatts(&home.path, &["engine"]);

    let ingested = gigawatts(&home.path, &["ingest"]);

    assert_eq!(
        stdout(&ingested),
        "The engine is running, so it read the new transcript lines.\n",
        "{ingested:?}"
    );
}

#[test]
fn a_projects_folder_made_after_the_engine_started_is_read() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    assert_eq!(
        stdout(&gigawatts(&home.path, &["engine"])),
        "Started the engine.\n"
    );
    wait_for_requests(&paths.snapshot(), 0);

    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    append_request(&projects.join("s1.jsonl"), "req_1");

    wait_for_requests(&paths.snapshot(), 1);
}

#[test]
fn an_unreadable_transcript_shows_as_a_problem_while_the_rest_is_read() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    let locked = projects.join("a.jsonl");
    append_request(&locked, "req_1");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    append_request(&projects.join("b.jsonl"), "req_2");

    gigawatts(&home.path, &["engine"]);

    wait_for_requests(&paths.snapshot(), 1);
    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(paths.snapshot()).unwrap()).unwrap();
    let kinds: Vec<&str> = snapshot["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|problem| problem["kind"].as_str())
        .collect();
    assert_eq!(kinds, vec!["read_failing"]);
}

#[test]
fn a_config_directory_added_while_the_engine_runs_is_read() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 0);
    let work = home._dir.path().join("work");
    let projects = work.join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        work.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000e0"}}"#,
    )
    .unwrap();
    append_request(&projects.join("s1.jsonl"), "req_1");

    let added = gigawatts(&home.path, &["accounts", "add", &work.to_string_lossy()]);

    assert_eq!(added.status.code(), Some(0), "{added:?}");
    wait_for_requests(&paths.snapshot(), 1);
}

fn engine_with_an_unreadable_transcript() -> (Home, Paths) {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    let locked = projects.join("a.jsonl");
    append_request(&locked, "req_1");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 0);
    (home, paths)
}

#[test]
fn ingest_fails_when_the_running_engine_cant_read() {
    let (home, _paths) = engine_with_an_unreadable_transcript();

    let ingested = gigawatts(&home.path, &["ingest"]);

    assert_eq!(ingested.status.code(), Some(2), "{ingested:?}");
    assert!(
        String::from_utf8_lossy(&ingested.stderr).contains("a.jsonl"),
        "{ingested:?}"
    );
}

#[test]
fn doctor_lists_the_running_engines_problems() {
    let (home, _paths) = engine_with_an_unreadable_transcript();

    let doctor = gigawatts(&home.path, &["doctor"]);

    assert_eq!(doctor.status.code(), Some(1), "{doctor:?}");
    assert!(
        stdout(&doctor).contains("Gigawatts can't read a transcript"),
        "{doctor:?}"
    );
}

const LOGIN_ACCOUNT: &str =
    "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0";

fn engine_with_one_request_today() -> (Home, Paths) {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    append_request(&projects.join("s1.jsonl"), "req_1");
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 1);
    (home, paths)
}

fn gigawatts_answering(home: &Path, args: &[&str], answer: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gigawatts"))
        .args(args)
        .env("HOME", home)
        .env("PATH", path_with_claude_in(&stand_in_bin()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{answer}").unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn turning_tracking_off_while_the_engine_runs_takes_the_account_out_of_the_snapshot() {
    let (home, paths) = engine_with_one_request_today();

    let off = gigawatts(&home.path, &["accounts", "track", LOGIN_ACCOUNT, "off"]);

    assert_eq!(off.status.code(), Some(0), "{off:?}");
    assert_eq!(requests_today(&paths.snapshot()), Some(0));
}

#[test]
fn forgetting_an_account_while_the_engine_runs_takes_it_out_of_the_snapshot() {
    let (home, paths) = engine_with_one_request_today();

    let forgot = gigawatts_answering(&home.path, &["accounts", "forget", LOGIN_ACCOUNT], "y");

    assert_eq!(forgot.status.code(), Some(0), "{forgot:?}");
    assert_eq!(requests_today(&paths.snapshot()), Some(0));
    assert_eq!(running_version(&paths).as_deref(), Some(version().as_str()));
}

#[test]
fn ingest_fails_on_a_config_it_cant_parse_while_the_engine_runs() {
    let (home, _paths) = engine_with_one_request_today();
    let config = Paths::new(&home.path).config();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "[prices.\"claude-opus-5-5\"\ninput = 5\n").unwrap();

    let ingested = gigawatts(&home.path, &["ingest"]);

    assert_eq!(ingested.status.code(), Some(2), "{ingested:?}");
    assert!(
        String::from_utf8_lossy(&ingested.stderr).contains("config.toml"),
        "{ingested:?}"
    );
}

#[test]
fn a_projects_folder_deleted_and_made_again_while_the_engine_runs_is_read() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let projects = home.path.join(".claude").join("projects");
    fs::create_dir_all(projects.join("p")).unwrap();
    append_request(&projects.join("p").join("s1.jsonl"), "req_1");
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 1);

    fs::remove_dir_all(&projects).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    fs::create_dir_all(projects.join("p")).unwrap();
    append_request(&projects.join("p").join("s2.jsonl"), "req_2");

    wait_for_requests(&paths.snapshot(), 2);
}

#[test]
fn a_claude_folder_made_after_the_engine_started_is_read() {
    let home = home_with_login();
    fs::remove_dir(home.path.join(".claude")).unwrap();
    let paths = Paths::new(&home.path);
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 0);

    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    append_request(&projects.join("s1.jsonl"), "req_1");

    wait_for_requests(&paths.snapshot(), 1);
}

#[test]
fn ingest_waits_for_an_engine_that_holds_the_lock_but_isnt_listening_yet() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    fs::create_dir_all(paths.data_dir()).unwrap();
    let lock = EngineLock::acquire(&paths.lock()).unwrap().unwrap();
    let socket = paths.socket();
    std::thread::spawn(move || {
        let _lock = lock;
        std::thread::sleep(Duration::from_millis(1500));
        let listener = UnixListener::bind(&socket).unwrap();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let reply = match line.trim() {
                "version" => version(),
                "refresh" => "refreshed".to_string(),
                _ => return,
            };
            writeln!(stream, "{reply}").unwrap();
        }
    });

    let ingested = gigawatts(&home.path, &["ingest"]);

    assert_eq!(
        stdout(&ingested),
        "The engine is running, so it read the new transcript lines.\n",
        "{ingested:?}"
    );
}

fn problem_kinds_and_causes(snapshot: &Path) -> Vec<(String, String)> {
    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(snapshot).unwrap()).unwrap();
    snapshot["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| {
            (
                problem["kind"].as_str().unwrap().to_string(),
                problem["cause"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn an_unreadable_login_shows_as_a_problem_and_ingest_still_warns_about_it() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    fs::create_dir_all(home.path.join(".claude").join("projects")).unwrap();
    fs::write(home.path.join(".claude.json"), "{\"oauthAccount\":").unwrap();
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 0);

    let problems = problem_kinds_and_causes(&paths.snapshot());
    let ingested = gigawatts(&home.path, &["ingest"]);

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].0, "read_failing");
    assert!(problems[0].1.contains(".claude.json"), "{problems:?}");
    assert!(
        String::from_utf8_lossy(&ingested.stderr).contains("can't read the login"),
        "{ingested:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_the_engine_cant_watch_shows_as_a_problem_until_it_can() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let projects = home.path.join(".claude").join("projects");
    let locked = projects.join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 0);
    let watch_problem = |snapshot: &Path| -> Option<serde_json::Value> {
        let snapshot: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(snapshot).unwrap()).unwrap();
        snapshot["problems"]
            .as_array()
            .unwrap()
            .iter()
            .find(|problem| problem["kind"] == "watch_failing")
            .cloned()
    };
    let first = watch_problem(&paths.snapshot()).expect("a watch_failing problem");

    std::thread::sleep(Duration::from_millis(1100));
    gigawatts(&home.path, &["ingest"]);
    let again = watch_problem(&paths.snapshot()).expect("still a watch_failing problem");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    gigawatts(&home.path, &["ingest"]);

    assert_eq!(again["since"], first["since"]);
    assert_eq!(watch_problem(&paths.snapshot()), None);
}

#[test]
fn stop_reaches_an_engine_that_holds_the_lock_but_isnt_listening_yet() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    fs::create_dir_all(paths.data_dir()).unwrap();
    let lock = EngineLock::acquire(&paths.lock()).unwrap().unwrap();
    let socket = paths.socket();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let listener = UnixListener::bind(&socket).unwrap();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line.trim() == "stop" {
                writeln!(stream, "stopping").unwrap();
                fs::remove_file(&socket).unwrap();
                drop(lock);
                return;
            }
            writeln!(stream, "{}", version()).unwrap();
        }
    });

    let stopped = gigawatts(&home.path, &["stop"]);

    assert_eq!(stdout(&stopped), "Stopped the engine.\n", "{stopped:?}");
}

#[test]
fn tracking_on_says_the_engine_read_it_even_when_another_transcript_cant_be_read() {
    let (home, _paths) = engine_with_an_unreadable_transcript();

    let on = gigawatts(&home.path, &["accounts", "track", LOGIN_ACCOUNT, "on"]);

    assert_eq!(on.status.code(), Some(0), "{on:?}");
    assert!(
        stdout(&on).contains("The engine read its transcripts from the start."),
        "{on:?}"
    );
    assert!(
        String::from_utf8_lossy(&on.stderr).contains("`gigawatts doctor`"),
        "{on:?}"
    );
}

#[test]
fn starting_an_engine_that_exits_at_once_fails_without_waiting_out_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let started = Instant::now();

    let result = gigawatts::engine::ensure_running(&paths, Path::new("/usr/bin/false"));

    assert!(
        result.as_ref().is_err_and(|error| error
            .to_string()
            .starts_with("the engine exited as it started")),
        "{:?}",
        result.err().map(|error| error.to_string())
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_command_that_pauses_the_engine_stops_an_older_one_and_starts_this_version_after() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    let old = old_engine(&paths);

    let on = gigawatts(&home.path, &["accounts", "track", LOGIN_ACCOUNT, "on"]);

    assert_eq!(on.status.code(), Some(0), "{on:?}");
    assert_eq!(old.heard(), vec!["version", "stop"]);
    assert_eq!(running_version(&paths).as_deref(), Some(version().as_str()));
}

#[test]
fn the_engine_deletes_request_detail_older_than_the_setting_and_keeps_the_totals() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    fs::create_dir_all(paths.config().parent().unwrap()).unwrap();
    fs::write(paths.config(), "request_detail_days = 30\n").unwrap();
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    let old = r#"{"type":"assistant","sessionId":"s0","requestId":"req_old","timestamp":"2020-01-01T12:00:00.000Z","message":{"model":"claude-opus-5-5","usage":{"input_tokens":1000,"output_tokens":500}}}"#;
    fs::write(projects.join("s0.jsonl"), format!("{old}\n")).unwrap();
    append_request(&projects.join("s1.jsonl"), "req_1");

    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 1);

    let stored = gigawatts::store::Store::open(&paths.database())
        .unwrap()
        .request_count()
        .unwrap();
    assert_eq!(stored, 1);
    let all = stdout(&gigawatts(&home.path, &["data", "--range", "all"]));
    assert!(all.contains("$0.03"), "{all}");
}

#[test]
fn a_cleanup_that_fails_shows_as_a_problem() {
    let home = home_with_login();
    let paths = Paths::new(&home.path);
    fs::create_dir_all(paths.config().parent().unwrap()).unwrap();
    fs::write(paths.config(), "request_detail_days = 4294967295\n").unwrap();
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    append_request(&projects.join("s1.jsonl"), "req_1");

    gigawatts(&home.path, &["engine"]);

    wait_for_requests(&paths.snapshot(), 1);
    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(paths.snapshot()).unwrap()).unwrap();
    let kinds: Vec<&str> = snapshot["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|problem| problem["kind"].as_str())
        .collect();
    assert_eq!(kinds, vec!["cleanup_failing"]);
}

fn logged_in_home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = Home {
        path: dir.path().join("h"),
        _dir: dir,
    };
    fs::create_dir_all(home.path.join(".claude").join("projects").join("p")).unwrap();
    fs::write(
        home.path.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    home
}

fn answering_claude(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage/ultra/get-usage.json");
    let response: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture).unwrap()).unwrap();
    let stream = dir.join("stream.jsonl");
    fs::write(
        &stream,
        serde_json::json!({"type":"control_response","response":{"subtype":"success","request_id":"gigawatts-usage","response":response}}).to_string() + "\n",
    )
    .unwrap();
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(
        &claude,
        format!("#!/bin/sh\ncat > /dev/null\ncat '{}'\n", stream.display()),
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn wait_for_snapshot(
    snapshot: &Path,
    what: &str,
    found: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(value) = fs::read_to_string(snapshot)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .filter(|value| found(value))
        {
            return value;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "the snapshot never showed {what}: {:?}",
        fs::read_to_string(snapshot)
    );
}

#[test]
fn activity_refreshes_limits_through_claude_into_the_snapshot() {
    let home = logged_in_home();
    let bin = answering_claude(&home.path.parent().unwrap().join("claude"));
    let transcript = home.path.join(".claude/projects/p/s1.jsonl");
    append_request(&transcript, "req_1");
    let paths = Paths::new(&home.path);
    let started = gigawatts_with_claude_in(&home.path, &bin, &["engine"]);
    assert_eq!(stdout(&started), "Started the engine.\n", "{started:?}");
    wait_for_requests(&paths.snapshot(), 1);

    append_request(&transcript, "req_2");

    let snapshot = wait_for_snapshot(&paths.snapshot(), "limits", |value| {
        !value["accounts"][0]["limits"].is_null()
    });
    let limits = &snapshot["accounts"][0]["limits"];
    assert_eq!(limits["source"], "get_usage");
    let kinds: Vec<&str> = limits["limits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|limit| limit["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["session", "weekly_all", "weekly_scoped"]);
}

#[test]
fn a_failing_refresh_shows_as_a_problem() {
    let home = logged_in_home();
    let transcript = home.path.join(".claude/projects/p/s1.jsonl");
    append_request(&transcript, "req_1");
    let paths = Paths::new(&home.path);
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 1);

    append_request(&transcript, "req_2");

    let snapshot = wait_for_snapshot(&paths.snapshot(), "a refresh problem", |value| {
        value["problems"].as_array().is_some_and(|problems| {
            problems
                .iter()
                .any(|problem| problem["kind"] == "refresh_failing")
        })
    });
    let problem = snapshot["problems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|problem| problem["kind"] == "refresh_failing")
        .unwrap();
    assert_eq!(
        problem["cause"],
        "Gigawatts can't refresh limits through `claude -p`: `claude -p` exited (exit status: 1) without answering: the test suite stands in for claude"
    );
    assert_eq!(
        problem["affects"],
        "limits for 00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0: no reading yet"
    );
}

#[test]
fn a_refresh_problem_clears_once_the_account_logs_out() {
    let home = logged_in_home();
    let transcript = home.path.join(".claude/projects/p/s1.jsonl");
    append_request(&transcript, "req_1");
    let paths = Paths::new(&home.path);
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), 1);
    append_request(&transcript, "req_2");
    wait_for_snapshot(&paths.snapshot(), "a refresh problem", |value| {
        problem_kinds(value).contains(&"refresh_failing".to_string())
    });

    fs::write(home.path.join(".claude.json"), "{}").unwrap();
    append_request(&transcript, "req_3");

    wait_for_snapshot(&paths.snapshot(), "the refresh problem gone", |value| {
        !problem_kinds(value).contains(&"refresh_failing".to_string())
    });
}

fn problem_kinds(snapshot: &serde_json::Value) -> Vec<String> {
    snapshot["problems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|problem| problem["kind"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn data_limits_prints_the_saved_readings_when_no_engine_runs() {
    let home = logged_in_home();
    let paths = Paths::new(&home.path);
    let mut store = gigawatts::store::Store::open(&paths.database()).unwrap();
    let response = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage/ultra/get-usage.json"),
    )
    .unwrap();
    store
        .record_limits(
            &"00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
                .parse()
                .unwrap(),
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

    let output = Command::new(env!("CARGO_BIN_EXE_gigawatts"))
        .args(["data", "--limits"])
        .env("HOME", &home.path)
        .env("TZ", "UTC")
        .env("PATH", path_with_claude_in(&stand_in_bin()))
        .output()
        .unwrap();

    let printed = stdout(&output);
    let lines: Vec<&str> = printed.lines().collect();
    assert!(
        lines[0].starts_with(
            "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0, from get_usage at 2026-11-04 19:20 ("
        ),
        "{output:?}"
    );
    assert_eq!(
        &lines[1..],
        [
            "  Session                2%  resets 2026-11-04 23:59",
            "  Weekly, all models    63%  resets 2026-11-08 02:59",
            "  Weekly, Fable         90%  resets 2026-11-08 02:59, critical",
        ]
    );
}

#[test]
fn data_limits_asks_a_running_engine_for_fresh_readings() {
    let home = logged_in_home();
    let bin = answering_claude(&home.path.parent().unwrap().join("claude"));
    let started = gigawatts_with_claude_in(&home.path, &bin, &["engine"]);
    assert_eq!(stdout(&started), "Started the engine.\n", "{started:?}");

    let output = gigawatts_with_claude_in(&home.path, &bin, &["data", "--limits"]);

    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).starts_with(
            "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0, from get_usage at "
        ),
        "{output:?}"
    );
}

#[test]
fn data_limits_warns_about_a_failing_refresh_each_time_it_asks() {
    let home = logged_in_home();
    gigawatts(&home.path, &["engine"]);

    let first = gigawatts(&home.path, &["data", "--limits"]);
    let second = gigawatts(&home.path, &["data", "--limits"]);

    let warning = "gigawatts: the engine couldn't refresh limits: `claude -p` exited (exit status: 1) without answering: the test suite stands in for claude. Showing the saved readings.\n";
    for (call, output) in [("first", first), ("second", second)] {
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            warning,
            "{call}: {output:?}"
        );
    }
}

#[test]
fn a_cached_reading_that_doesnt_parse_shows_as_a_problem() {
    let home = logged_in_home();
    let claude_json = home.path.join(".claude.json");
    fs::write(
        &claude_json,
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"},
"cachedUsageUtilization":{"accountUuid":"00000000-0000-4000-8000-00000000000a","fetchedAtMs":1793820000000,"utilization":{"limits":[{"kind":"session","percent":null}]}}}"#,
    )
    .unwrap();
    let paths = Paths::new(&home.path);

    gigawatts(&home.path, &["engine"]);

    let snapshot = wait_for_snapshot(&paths.snapshot(), "a read problem", |value| {
        problem_kinds(value).contains(&"read_failing".to_string())
    });
    let problem = snapshot["problems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|problem| problem["kind"] == "read_failing")
        .unwrap();
    assert_eq!(
        problem["affects"],
        format!(
            "limits from {}: the saved reading isn't updated",
            claude_json.display()
        )
    );
    assert!(
        problem["cause"].as_str().unwrap().starts_with(&format!(
            "Gigawatts can't read `cachedUsageUtilization` in {}: invalid type: null, expected f64",
            claude_json.display()
        )),
        "{problem}"
    );
}

#[test]
fn data_limits_doesnt_ask_claude_while_the_saved_reading_is_fresh() {
    let home = logged_in_home();
    let claude_dir = home.path.parent().unwrap().join("claude");
    let bin = answering_claude(&claude_dir);
    let runs = claude_dir.join("runs");
    fs::write(
        bin.join("claude"),
        format!(
            "#!/bin/sh\ncat > /dev/null\necho run >> '{}'\ncat '{}'\n",
            runs.display(),
            claude_dir.join("stream.jsonl").display()
        ),
    )
    .unwrap();
    let paths = Paths::new(&home.path);
    let mut store = gigawatts::store::Store::open(&paths.database()).unwrap();
    let five_minutes_ago = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 5 * 60_000;
    store
        .record_limits(
            &"00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
                .parse()
                .unwrap(),
            &gigawatts::store::LimitsReading {
                fetched_at_ms: five_minutes_ago,
                source: gigawatts::contract::LimitSource::CachedUsageUtilization,
                limits: vec![],
            },
        )
        .unwrap();
    drop(store);
    gigawatts_with_claude_in(&home.path, &bin, &["engine"]);

    let output = gigawatts_with_claude_in(&home.path, &bin, &["data", "--limits"]);

    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).contains("from cachedUsageUtilization at"),
        "{output:?}"
    );
    assert!(
        !runs.exists(),
        "claude ran: {:?}",
        fs::read_to_string(&runs)
    );
}
