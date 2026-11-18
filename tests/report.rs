use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use gigawatts::engine::Paths;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

struct Home {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        gigawatts(&self.path, &["stop"]);
    }
}

fn gigawatts(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gigawatts"))
        .args(args)
        .env("HOME", home)
        .env(
            "PATH",
            format!(
                "{}/tests/bin:{}",
                env!("CARGO_MANIFEST_DIR"),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap()
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

fn home_with_spend_today(config: &str) -> Home {
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
    let line = serde_json::json!({
        "type": "assistant",
        "sessionId": "s1",
        "requestId": "r1",
        "timestamp": today_noon_utc(),
        "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1_000, "output_tokens": 500}}
    });
    fs::write(projects.join("s1.jsonl"), format!("{line}\n")).unwrap();
    let config_path = Paths::new(&home.path).config();
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(&config_path, config).unwrap();
    home
}

fn ingested(home: &Home) {
    let paths = Paths::new(&home.path);
    let mut store = Store::open(&paths.database()).unwrap();
    let config = gigawatts::config::Config::load(&paths.config(), &home.path).unwrap();
    let directories = gigawatts::accounts::discover(&home.path, &config);
    gigawatts::accounts::ingest(
        &mut store,
        &directories,
        &config,
        &PriceTable::built_in(),
        &Resolver::default(),
    )
    .unwrap();
}

fn serving(home: &Home) -> u16 {
    ingested(home);
    let listener = gigawatts::report::bind(0).unwrap();
    let port = listener.local_addr().unwrap().port();
    gigawatts::report::serve(listener, Paths::new(&home.path)).unwrap();
    port
}

struct Response {
    status: u16,
    headers: String,
    body: String,
}

fn send(port: u16, request: &[u8]) -> Response {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = stream.write_all(request);
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Response {
        status,
        headers: head.to_lowercase(),
        body: body.to_string(),
    }
}

fn get(port: u16, path: &str, extra_headers: &str) -> Response {
    send(
        port,
        format!("GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\n{extra_headers}\r\n").as_bytes(),
    )
}

#[test]
fn api_data_answers_with_the_same_json_as_the_data_command() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let response = get(port, "/api/data?range=week", "");

    assert_eq!(response.status, 200, "{}", response.body);
    assert!(
        response.headers.contains("content-type: application/json"),
        "{}",
        response.headers
    );
    let served: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let printed = gigawatts(&home.path, &["data", "--range", "week", "--json"]);
    assert!(printed.status.success(), "{printed:?}");
    let printed: serde_json::Value = serde_json::from_slice(&printed.stdout).unwrap();
    assert_eq!(served, printed);
    assert_eq!(served["rows"][0]["requests"], 1);
}

#[test]
fn the_page_is_served_with_a_policy_that_blocks_outside_resources() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let response = get(port, "/", "");

    assert_eq!(response.status, 200);
    assert!(response.headers.contains("content-type: text/html"));
    assert!(
        response
            .headers
            .contains("content-security-policy: default-src 'none'"),
        "{}",
        response.headers
    );
    assert!(response.body.contains("/api/data"), "{}", response.body);
}

#[test]
fn an_unknown_path_is_not_found() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    assert_eq!(get(port, "/etc/passwd", "").status, 404);
}

#[test]
fn a_bad_range_is_a_bad_request() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let response = get(port, "/api/data?range=decade", "");

    assert_eq!(response.status, 400);
    assert!(response.body.contains("decade"), "{}", response.body);
}

#[test]
fn a_host_other_than_this_machine_is_refused() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let rebound = send(
        port,
        format!("GET /api/data HTTP/1.1\r\nHost: attacker.example:{port}\r\n\r\n").as_bytes(),
    );
    let missing = send(port, b"GET /api/data HTTP/1.1\r\n\r\n");

    assert_eq!(rebound.status, 403);
    assert!(!rebound.body.contains("rows"), "{}", rebound.body);
    assert_eq!(missing.status, 403);
    for host in ["127.0.0.1", "gigawatts.localhost", "localhost"] {
        let allowed = send(
            port,
            format!("GET /api/data HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n").as_bytes(),
        );
        assert_eq!(allowed.status, 200, "{host}");
    }
}

#[test]
fn a_cross_site_request_is_refused() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let other_origin = get(port, "/api/data", "Origin: https://attacker.example\r\n");
    let opaque_origin = get(port, "/api/data", "Origin: null\r\n");
    let cross_site = get(port, "/api/data", "Sec-Fetch-Site: cross-site\r\n");
    let same_site = get(port, "/api/data", "Sec-Fetch-Site: same-site\r\n");
    let same_origin = get(
        port,
        "/api/data",
        &format!("Origin: http://localhost:{port}\r\nSec-Fetch-Site: same-origin\r\n"),
    );
    let typed_in = get(port, "/", "Sec-Fetch-Site: none\r\n");

    assert_eq!(other_origin.status, 403);
    assert_eq!(opaque_origin.status, 403);
    assert_eq!(cross_site.status, 403);
    assert_eq!(same_site.status, 403);
    assert_eq!(same_origin.status, 200);
    assert_eq!(typed_in.status, 200);
}

#[test]
fn only_get_is_answered() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let response = send(
        port,
        format!("POST /api/data HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: 0\r\n\r\n")
            .as_bytes(),
    );

    assert_eq!(response.status, 405);
}

#[test]
fn an_oversized_request_is_refused_and_the_server_keeps_answering() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);
    let padding = "x".repeat(20_000);

    let oversized = send(
        port,
        format!("GET /api/data HTTP/1.1\r\nHost: localhost:{port}\r\nX-Padding: {padding}\r\n\r\n")
            .as_bytes(),
    );

    assert_eq!(oversized.status, 431);
    assert_eq!(get(port, "/api/data", "").status, 200);
}

#[test]
fn a_client_that_never_finishes_its_request_doesnt_stop_the_server_for_long() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);
    let mut stalled = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stalled.write_all(b"GET / HTTP/1.1\r\n").unwrap();
    let started = Instant::now();

    let response = get(port, "/api/data", "");

    assert_eq!(response.status, 200);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
}

fn wait_for_problem(home: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let doctor = gigawatts(home, &["doctor"]);
        let listed = String::from_utf8_lossy(&doctor.stdout).into_owned();
        if listed.contains("report page") || Instant::now() > deadline {
            return listed;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn a_taken_port_leaves_the_engine_running_with_a_problem() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let home = home_with_spend_today(&format!("report_port = {port}\n"));

    let started = gigawatts(&home.path, &["engine"]);

    assert_eq!(
        String::from_utf8_lossy(&started.stdout),
        "Started the engine. The flux capacitor is warming up.\n",
        "{started:?}"
    );
    let listed = wait_for_problem(&home.path);
    assert!(listed.contains(&format!("port {port}")), "{listed}");
    assert!(listed.contains("report_port"), "{listed}");
    let data = gigawatts(&home.path, &["data", "--range", "today"]);
    assert!(data.status.success(), "{data:?}");
    let stopped = gigawatts(&home.path, &["stop"]);
    assert_eq!(
        String::from_utf8_lossy(&stopped.stdout),
        "Stopped the engine. The flux capacitor is cooling down.\n"
    );
}

#[test]
fn the_engine_serves_the_report_on_its_configured_port() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let home = home_with_spend_today(&format!("report_port = {port}\n"));

    gigawatts(&home.path, &["engine"]);

    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", port)).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut requests = serde_json::Value::Null;
    while requests.is_null() && Instant::now() < deadline {
        let response = get(port, "/api/data?range=today", "");
        assert_eq!(response.status, 200, "{}", response.body);
        requests = serde_json::from_str::<serde_json::Value>(&response.body).unwrap()["rows"][0]
            ["requests"]
            .clone();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(requests, 1);
}

#[test]
fn turning_the_page_off_while_the_engine_runs_says_to_restart_it() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let home = home_with_spend_today(&format!("report_port = {port}\n"));
    gigawatts(&home.path, &["engine"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", port)).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }

    fs::write(Paths::new(&home.path).config(), "report_port = 0\n").unwrap();
    let line = serde_json::json!({
        "type": "assistant",
        "sessionId": "s2",
        "requestId": "r2",
        "timestamp": today_noon_utc(),
        "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1_000, "output_tokens": 500}}
    });
    fs::write(
        home.path.join(".claude/projects/p/s2.jsonl"),
        format!("{line}\n"),
    )
    .unwrap();

    let listed = wait_for_problem(&home.path);
    assert!(
        listed.contains(&format!("still serving the report page on port {port}")),
        "{listed}"
    );
    assert!(listed.contains("`gigawatts stop`"), "{listed}");
}

#[test]
fn turning_the_page_on_while_the_engine_runs_says_to_restart_it() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let home = home_with_spend_today("report_port = 0\n");
    gigawatts(&home.path, &["engine"]);

    fs::write(
        Paths::new(&home.path).config(),
        format!("report_port = {port}\n"),
    )
    .unwrap();
    let line = serde_json::json!({
        "type": "assistant",
        "sessionId": "s2",
        "requestId": "r2",
        "timestamp": today_noon_utc(),
        "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1_000, "output_tokens": 500}}
    });
    fs::write(
        home.path.join(".claude/projects/p/s2.jsonl"),
        format!("{line}\n"),
    )
    .unwrap();

    let listed = wait_for_problem(&home.path);
    assert!(
        listed.contains("the engine started with the report page off"),
        "{listed}"
    );
    assert!(listed.contains(&format!("now port {port}")), "{listed}");
    assert!(listed.contains("`gigawatts stop`"), "{listed}");
}

#[test]
fn report_says_the_server_is_off_when_the_port_is_zero() {
    let home = home_with_spend_today("report_port = 0\n");

    let output = gigawatts(&home.path, &["report"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "gigawatts: the report page is off, because `report_port = 0` in ~/.config/gigawatts/config.toml. Remove that line or set a port to turn it on.\n"
    );
}

#[test]
fn report_doesnt_open_another_programs_page_on_its_port() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let home = home_with_spend_today(&format!("report_port = {port}\n"));

    let output = gigawatts(&home.path, &["report"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!(
            "gigawatts: the report page isn't answering on port {port}. The engine reads `report_port` only when it starts, so after changing it run `gigawatts stop` and `gigawatts report`. Otherwise `gigawatts doctor` lists why.\n"
        )
    );
}

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

fn home_with_spend_on(requests: &[(&str, &str, &str, u64)]) -> Home {
    let home = home_with_spend_today("report_port = 0\n");
    let lines: String = requests
        .iter()
        .enumerate()
        .map(|(n, (day, cwd, branch, output))| {
            serde_json::json!({
                "type": "assistant",
                "sessionId": format!("s-{cwd}"),
                "requestId": format!("r{n}"),
                "timestamp": local_noon_in_utc(day),
                "cwd": cwd,
                "gitBranch": branch,
                "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 0, "output_tokens": output}}
            })
            .to_string()
                + "\n"
        })
        .collect();
    fs::write(home.path.join(".claude/projects/p/s1.jsonl"), lines).unwrap();
    home
}

fn json(response: &Response) -> serde_json::Value {
    assert_eq!(response.status, 200, "{}", response.body);
    serde_json::from_str(&response.body).unwrap()
}

fn cents(value: f64) -> i64 {
    (value * 100.0).round() as i64
}

fn data_total(data: &serde_json::Value) -> i64 {
    cents(
        data["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["cost_usd"].as_f64())
            .sum(),
    )
}

fn series_total(series: &serde_json::Value) -> i64 {
    cents(
        series["days"]
            .as_array()
            .unwrap()
            .iter()
            .map(|day| day["cost_usd"].as_f64().unwrap())
            .sum(),
    )
}

#[test]
fn the_daily_series_adds_up_to_the_data_totals_for_the_same_range() {
    let home = home_with_spend_on(&[
        ("2026-01-05", "/work/alpha", "main", 100_000),
        ("2026-01-06", "/work/beta", "main", 200_000),
        ("2026-01-07", "/work/alpha", "fix", 300_000),
        ("2026-01-07", "/work/gamma", "main", 50_000),
    ]);
    let port = serving(&home);

    for range in [
        "since=2026-01-05&until=2026-01-06",
        "range=all",
        "since=2026-01-06",
    ] {
        let data = json(&get(port, &format!("/api/data?by=repo&{range}"), ""));
        let series = json(&get(
            port,
            &format!("/api/series?by=repo&top=1&{range}"),
            "",
        ));

        assert_eq!(
            series_total(&series),
            data_total(&data),
            "{range}: {series}"
        );
        assert!(data_total(&data) > 0, "{range}");
        assert_eq!(series["since"], data["since"], "{range}");
        assert_eq!(series["until"], data["until"], "{range}");
    }
    let custom = json(&get(
        port,
        "/api/series?by=repo&top=1&since=2026-01-05&until=2026-01-06",
        "",
    ));
    let days: Vec<(&str, &str, bool)> = custom["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|day| {
            (
                day["day"].as_str().unwrap(),
                day["label"].as_str().unwrap(),
                day["other"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        days,
        [("2026-01-05", "other", true), ("2026-01-06", "beta", false),]
    );
}

#[test]
fn drilling_into_a_repo_lists_its_branches_then_a_branch_lists_its_sessions() {
    let home = home_with_spend_on(&[
        ("2026-01-05", "/work/alpha", "main", 100_000),
        ("2026-01-06", "/work/alpha", "fix it", 200_000),
        ("2026-01-06", "/work/beta", "main", 200_000),
    ]);
    let port = serving(&home);

    let branches = json(&get(
        port,
        "/api/data?range=all&by=repo&drill=%2Fwork%2Falpha",
        "",
    ));
    let sessions = json(&get(
        port,
        "/api/data?range=all&by=repo&drill=%2Fwork%2Falpha&drill=fix%20it",
        "",
    ));

    assert_eq!(branches["level"], "branch");
    let labels: Vec<&str> = branches["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["fix it", "main"]);
    assert_eq!(sessions["level"], "session");
    assert_eq!(sessions["drill"][1]["label"], "fix it");
    assert_eq!(sessions["rows"].as_array().unwrap().len(), 1);
}

#[test]
fn drilling_into_a_name_that_isnt_there_is_a_bad_request() {
    let home = home_with_spend_on(&[("2026-01-05", "/work/alpha", "main", 100_000)]);
    let port = serving(&home);

    let response = get(port, "/api/data?range=all&by=repo&drill=%2Fwork%2Fnope", "");

    assert_eq!(response.status, 400);
    assert!(response.body.contains("/work/nope"), "{}", response.body);
}

#[test]
fn spend_by_subagent_and_model_adds_up_to_the_data_totals() {
    let home = home_with_spend_on(&[
        ("2026-01-05", "/work/alpha", "main", 100_000),
        ("2026-01-06", "/work/beta", "main", 200_000),
    ]);
    let port = serving(&home);

    for range in ["range=all", "since=2026-01-06&until=2026-01-06"] {
        let data = json(&get(port, &format!("/api/data?by=agent&{range}"), ""));
        let agents = json(&get(port, &format!("/api/agents?{range}"), ""));

        let rows = agents["rows"].as_array().unwrap();
        let summed = cents(
            rows.iter()
                .map(|row| row["cost_usd"].as_f64().unwrap())
                .sum(),
        );
        assert_eq!(summed, data_total(&data), "{range}: {agents}");
        assert_eq!(agents["since"], data["since"], "{range}");
        assert_eq!(rows[0]["agent"], "you (no subagent)", "{agents}");
        assert_eq!(rows[0]["model"], "claude-opus-5-5", "{agents}");
    }
}

#[test]
fn now_answers_with_each_accounts_numbers_for_today() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);

    let now = json(&get(port, "/api/now", ""));

    assert_eq!(now["schema_version"], 2);
    let account = &now["accounts"][0];
    assert_eq!(
        account["account"],
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
    );
    assert_eq!(account["requests"], 1);
    assert!(account["cost_usd"].as_f64().unwrap() > 0.0, "{now}");
}

#[test]
fn now_matches_the_snapshot_the_engine_writes() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let home = home_with_spend_today(&format!("report_port = {port}\n"));
    gigawatts(&home.path, &["engine"]);
    let snapshot_path = Paths::new(&home.path).snapshot();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut written = serde_json::Value::Null;
    while Instant::now() < deadline {
        if let Some(read) = fs::read_to_string(&snapshot_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .filter(|read| read["accounts"][0]["requests"] == 1)
        {
            written = read;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(!written.is_null(), "the engine never wrote a snapshot");

    let mut now = json(&get(port, "/api/now", ""));
    let mut written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap()).unwrap();

    for read in [&mut now, &mut written] {
        read["written_at_ms"] = serde_json::Value::Null;
        read["accounts"][0]["pace"]["clock"] = serde_json::Value::Null;
    }
    assert_eq!(now, written);
}

fn home_with_lines(lines: &[serde_json::Value]) -> Home {
    let home = home_with_spend_today("report_port = 0\n");
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    fs::write(home.path.join(".claude/projects/p/s1.jsonl"), text).unwrap();
    home
}

fn request(
    session: &str,
    id: &str,
    time: &str,
    skill: Option<&str>,
    usage: serde_json::Value,
) -> serde_json::Value {
    let mut line = serde_json::json!({
        "type": "assistant",
        "sessionId": session,
        "requestId": id,
        "timestamp": time,
        "cwd": "/work/alpha",
        "gitBranch": "main",
        "message": {"model": "claude-opus-5-5", "usage": usage}
    });
    if let Some(skill) = skill {
        line["attributionSkill"] = skill.into();
    }
    line
}

#[test]
fn sessions_list_each_sessions_place_main_skill_time_and_context() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            Some("review"),
            serde_json::json!({"input_tokens": 1_000, "cache_read_input_tokens": 50_000, "output_tokens": 100_000}),
        ),
        request(
            "s-a",
            "r2",
            "2026-01-05T15:04:00Z",
            Some("review"),
            serde_json::json!({"input_tokens": 1_000, "cache_read_input_tokens": 90_000, "output_tokens": 100_000}),
        ),
        request(
            "s-b",
            "r3",
            "2026-01-05T16:00:00Z",
            None,
            serde_json::json!({"input_tokens": 1_000, "output_tokens": 10_000}),
        ),
    ]);
    let port = serving(&home);

    let listed = json(&get(
        port,
        "/api/sessions?range=all&by=repo&drill=%2Fwork%2Falpha&drill=main",
        "",
    ));
    let data = json(&get(
        port,
        "/api/data?range=all&by=repo&drill=%2Fwork%2Falpha&drill=main",
        "",
    ));

    let sessions = listed["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2, "{listed}");
    let first = &sessions[0];
    assert_eq!(first["session"], "s-a");
    assert_eq!(first["repo"], "alpha");
    assert_eq!(first["branch"], "main");
    assert_eq!(first["requests"], 2);
    assert_eq!(first["active_minutes"], 4);
    assert_eq!(first["peak_context"], 91_000);
    assert_eq!(first["skills"][0]["label"], "review");
    assert_eq!(first["skills"][0]["share"], 1.0);
    assert_eq!(sessions[1]["skills"][0]["label"], "no skill");
    let listed_total = cents(
        sessions
            .iter()
            .map(|session| session["cost_usd"].as_f64().unwrap())
            .sum(),
    );
    assert_eq!(listed_total, data_total(&data));
}

#[test]
fn sessions_are_limited_to_the_most_expensive() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 100_000}),
        ),
        request(
            "s-b",
            "r2",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 300_000}),
        ),
        request(
            "s-c",
            "r3",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 200_000}),
        ),
    ]);
    let port = serving(&home);

    let listed = json(&get(port, "/api/sessions?range=all&limit=2", ""));

    let order: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["session"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["s-b", "s-c"]);
    assert_eq!(listed["total"], 3);
}

#[test]
fn the_session_limit_applies_to_each_account_so_a_smaller_account_still_lists_its_own() {
    let home = home_with_lines(&[
        request(
            "big-1",
            "r1",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 300_000}),
        ),
        request(
            "big-2",
            "r2",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 200_000}),
        ),
    ]);
    let work = home.path.join(".claude-work");
    fs::create_dir_all(work.join("projects/p")).unwrap();
    fs::write(
        work.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000b","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    let small = request(
        "small-1",
        "r3",
        "2026-01-05T15:00:00Z",
        None,
        serde_json::json!({"output_tokens": 1_000}),
    );
    fs::write(work.join("projects/p/w.jsonl"), format!("{small}\n")).unwrap();
    let port = serving(&home);

    let listed = json(&get(port, "/api/sessions?range=all&limit=1", ""));

    let order: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["session"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["big-1", "small-1"], "{listed}");
    assert_eq!(listed["total"], 3);
}

#[test]
fn the_cost_mix_answers_at_the_session_level() {
    let home = home_with_lines(&[request(
        "s-a",
        "r1",
        "2026-01-05T15:00:00Z",
        Some("review"),
        serde_json::json!({"input_tokens": 1_000_000}),
    )]);
    let port = serving(&home);

    let response = get(port, "/api/mix?range=all&by=skill&drill=review", "");

    assert_eq!(response.status, 200, "{}", response.body);
    let mix = json(&response);
    assert_eq!(mix["level"], "session");
    assert_eq!(mix["rows"][0]["key"], "s-a");
    assert_eq!(mix["rows"][0]["input_usd"], 4.0);
}

#[test]
fn sessions_under_a_skill_are_the_ones_that_ran_it() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            Some("review"),
            serde_json::json!({"output_tokens": 100_000}),
        ),
        request(
            "s-b",
            "r2",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 300_000}),
        ),
    ]);
    let port = serving(&home);

    let listed = json(&get(
        port,
        "/api/sessions?range=all&by=skill&drill=review",
        "",
    ));

    let sessions = listed["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{listed}");
    assert_eq!(sessions[0]["session"], "s-a");
}

#[test]
fn a_scoped_session_splits_only_the_spend_in_the_scope() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            Some("review"),
            serde_json::json!({"output_tokens": 50_000}),
        ),
        request(
            "s-a",
            "r2",
            "2026-01-05T15:30:00Z",
            None,
            serde_json::json!({"output_tokens": 100_000}),
        ),
    ]);
    let port = serving(&home);

    let listed = json(&get(port, "/api/sessions?range=all&skill=review", ""));

    let session = &listed["sessions"][0];
    assert_eq!(session["cost_usd"], 1.0, "{listed}");
    assert_eq!(session["requests"], 1);
    assert_eq!(
        session["skills"],
        serde_json::json!([{"label": "review", "share": 1.0}])
    );
    assert_eq!(session["first_ms"], session["last_ms"]);
}

#[test]
fn cost_per_session_uses_only_the_days_whose_sessions_are_counted() {
    let home = home_with_lines(&[
        request(
            "old",
            "r1",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"output_tokens": 50_000}),
        ),
        request(
            "new",
            "r2",
            &today_noon_utc(),
            None,
            serde_json::json!({"output_tokens": 100_000}),
        ),
    ]);
    let port = serving(&home);
    let mut store = Store::open(&Paths::new(&home.path).database()).unwrap();
    let today = store.local_today().unwrap();
    store.expire_requests(30, &today).unwrap();

    let mix = json(&get(port, "/api/mix?range=all&by=repo", ""));

    let row = &mix["rows"][0];
    assert_eq!(row["sessions"], 1, "{mix}");
    assert_eq!(row["session_cost_usd"], 2.0, "{mix}");
}

#[test]
fn the_cost_mix_prices_each_kind_of_token_and_counts_sessions() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            None,
            serde_json::json!({"input_tokens": 1_000_000, "cache_read_input_tokens": 1_000_000, "cache_creation_input_tokens": 1_000_000, "output_tokens": 100_000}),
        ),
        request(
            "s-b",
            "r2",
            "2026-01-05T16:00:00Z",
            None,
            serde_json::json!({"output_tokens": 100_000}),
        ),
    ]);
    let port = serving(&home);

    let mix = json(&get(port, "/api/mix?range=all&by=repo", ""));

    let row = &mix["rows"][0];
    assert_eq!(row["key"], "/work/alpha", "{mix}");
    assert_eq!(row["sessions"], 2);
    assert_eq!(cents(row["input_usd"].as_f64().unwrap()), 400);
    assert_eq!(cents(row["output_usd"].as_f64().unwrap()), 400);
    assert_eq!(cents(row["cache_read_usd"].as_f64().unwrap()), 20);
    assert_eq!(cents(row["cache_write_usd"].as_f64().unwrap()), 500);
}

#[test]
fn a_scope_narrows_every_route_to_that_repo_and_splits_it_by_any_grouping() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            Some("review"),
            serde_json::json!({"output_tokens": 100_000}),
        ),
        request(
            "s-a",
            "r2",
            "2026-01-05T15:05:00Z",
            None,
            serde_json::json!({"output_tokens": 200_000}),
        ),
        serde_json::json!({
            "type": "assistant",
            "sessionId": "s-b",
            "requestId": "r3",
            "timestamp": "2026-01-06T15:00:00Z",
            "cwd": "/work/beta",
            "gitBranch": "main",
            "attributionSkill": "review",
            "message": {"model": "claude-opus-5-5", "usage": {"output_tokens": 400_000}}
        }),
    ]);
    let port = serving(&home);
    let scope = "range=all&repo=%2Fwork%2Falpha";

    let whole = json(&get(
        port,
        "/api/data?range=all&by=repo&drill=%2Fwork%2Falpha",
        "",
    ));
    let by_skill = json(&get(port, &format!("/api/data?{scope}&by=skill"), ""));
    let series = json(&get(port, &format!("/api/series?{scope}&by=model"), ""));
    let agents = json(&get(port, &format!("/api/agents?{scope}"), ""));
    let sessions = json(&get(port, &format!("/api/sessions?{scope}"), ""));
    let mix = json(&get(port, &format!("/api/mix?{scope}&by=skill"), ""));

    let skills: Vec<&str> = by_skill["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["label"].as_str().unwrap())
        .collect();
    assert_eq!(skills, ["no skill", "review"], "{by_skill}");
    assert_eq!(data_total(&by_skill), data_total(&whole));
    assert_eq!(series_total(&series), data_total(&whole));
    let agent_total = cents(
        agents["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["cost_usd"].as_f64().unwrap())
            .sum(),
    );
    assert_eq!(agent_total, data_total(&whole));
    assert_eq!(sessions["total"], 1, "{sessions}");
    assert_eq!(sessions["per_account"][0]["sessions"], 1, "{sessions}");
    assert_eq!(
        sessions["per_account"][0]["account"],
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
    );
    assert_eq!(sessions["sessions"][0]["session"], "s-a");
    assert_eq!(mix["rows"].as_array().unwrap().len(), 2, "{mix}");
}

#[test]
fn two_scopes_combine() {
    let home = home_with_lines(&[
        request(
            "s-a",
            "r1",
            "2026-01-05T15:00:00Z",
            Some("review"),
            serde_json::json!({"output_tokens": 100_000}),
        ),
        request(
            "s-a",
            "r2",
            "2026-01-05T15:05:00Z",
            None,
            serde_json::json!({"output_tokens": 200_000}),
        ),
    ]);
    let port = serving(&home);

    let scoped = json(&get(
        port,
        "/api/data?range=all&repo=%2Fwork%2Falpha&skill=review&by=model",
        "",
    ));
    let review = json(&get(port, "/api/data?range=all&by=skill&drill=review", ""));

    assert_eq!(scoped["rows"].as_array().unwrap().len(), 1, "{scoped}");
    assert_eq!(data_total(&scoped), data_total(&review));
}

#[test]
fn advice_answers_with_each_accounts_usage_advice_saved_today() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);
    let mut store = Store::open(&Paths::new(&home.path).database()).unwrap();
    let account: gigawatts::sources::claude_json::Account =
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
            .parse()
            .unwrap();
    let today = store.local_today().unwrap();
    store
        .record_usage_advice(
            &account,
            "2000-01-01",
            1_000,
            &serde_json::json!({"week": {"recommendations": [{"title": "Old advice", "pct": 9}]}}),
        )
        .unwrap();
    store
        .record_usage_advice(
            &account,
            &today,
            2_000,
            &serde_json::json!({"week": {"recommendations": [{"title": "Start fresh", "pct": 62}]}}),
        )
        .unwrap();

    let advice = json(&get(port, "/api/advice", ""));

    let accounts = advice["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1, "{advice}");
    assert_eq!(
        accounts[0]["account"],
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
    );
    assert_eq!(accounts[0]["fetched_at_ms"], 2_000);
    assert_eq!(
        accounts[0]["behaviors"]["week"]["recommendations"][0]["title"],
        "Start fresh"
    );
}

#[test]
fn a_scope_can_name_a_repo_instead_of_giving_its_path() {
    let home = home_with_spend_on(&[
        ("2026-01-05", "/work/alpha", "main", 100_000),
        ("2026-01-06", "/work/beta", "main", 200_000),
    ]);
    let port = serving(&home);

    let by_name = json(&get(port, "/api/data?range=all&by=model&repo=alpha", ""));
    let by_path = json(&get(
        port,
        "/api/data?range=all&by=model&repo=%2Fwork%2Falpha",
        "",
    ));
    let out_of_range = json(&get(
        port,
        "/api/data?since=2026-01-06&until=2026-01-06&by=model&repo=alpha",
        "",
    ));

    assert_eq!(data_total(&by_name), data_total(&by_path));
    assert!(data_total(&by_name) > 0);
    assert!(
        out_of_range["rows"].as_array().unwrap().is_empty(),
        "{out_of_range}"
    );
}

#[test]
fn a_scope_naming_a_repo_two_paths_share_is_a_bad_request_listing_both() {
    let home = home_with_spend_on(&[
        ("2026-01-05", "/work/alpha", "main", 100_000),
        ("2026-01-06", "/other/alpha", "main", 200_000),
    ]);
    let port = serving(&home);

    let response = get(port, "/api/series?range=all&by=model&repo=alpha", "");

    assert_eq!(response.status, 400, "{}", response.body);
    assert!(response.body.contains("/work/alpha"), "{}", response.body);
    assert!(response.body.contains("/other/alpha"), "{}", response.body);
}

fn local_ms(day_and_time: &str) -> i64 {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT CAST(unixepoch(?1, 'utc') AS INTEGER) * 1000",
            [day_and_time],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn limits_over_a_range_start_from_the_reading_in_effect_and_carry_reset_times() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);
    let mut store = Store::open(&Paths::new(&home.path).database()).unwrap();
    let personal: gigawatts::sources::claude_json::Account =
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
            .parse()
            .unwrap();
    let work: gigawatts::sources::claude_json::Account =
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000e0"
            .parse()
            .unwrap();
    for (at, percent, resets_at) in [
        ("2026-01-04 12:00", 40.0, "2026-01-07T09:00:00+00:00"),
        ("2026-01-06 12:00", 80.0, "2026-01-07T09:00:00+00:00"),
        ("2026-01-08 23:00", 15.0, "2026-01-14T09:00:00+00:00"),
        ("2026-01-09 00:30", 16.0, "2026-01-14T09:00:00+00:00"),
    ] {
        store
            .record_limits(
                &personal,
                &limits_reading(
                    at,
                    vec![
                        a_limit("weekly_all", None, percent, resets_at),
                        a_limit("weekly_scoped", Some("Fable"), 10.0, resets_at),
                    ],
                ),
            )
            .unwrap();
    }
    store
        .record_limits(
            &work,
            &limits_reading(
                "2026-01-06 12:00",
                vec![a_limit("session", None, 5.0, "2026-01-06T15:00:00+00:00")],
            ),
        )
        .unwrap();

    let history = json(&get(
        port,
        "/api/limits?since=2026-01-05&until=2026-01-08",
        "",
    ));

    assert_eq!(history["since"], "2026-01-05");
    assert_eq!(history["until"], "2026-01-08");
    assert_eq!(
        history["accounts"],
        serde_json::json!([
            {
                "account": "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000e0",
                "limits": [{
                    "kind": "session",
                    "points": [
                        {"at_ms": local_ms("2026-01-06 12:00"), "percent": 5.0, "resets_at": "2026-01-06T15:00:00+00:00"},
                    ],
                }],
            },
            {
                "account": "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0",
                "limits": [
                    {
                        "kind": "weekly_all",
                        "points": [
                            {"at_ms": local_ms("2026-01-04 12:00"), "percent": 40.0, "resets_at": "2026-01-07T09:00:00+00:00"},
                            {"at_ms": local_ms("2026-01-06 12:00"), "percent": 80.0, "resets_at": "2026-01-07T09:00:00+00:00"},
                            {"at_ms": local_ms("2026-01-08 23:00"), "percent": 15.0, "resets_at": "2026-01-14T09:00:00+00:00"},
                        ],
                    },
                    {
                        "kind": "weekly_scoped",
                        "scope": "Fable",
                        "points": [
                            {"at_ms": local_ms("2026-01-04 12:00"), "percent": 10.0, "resets_at": "2026-01-07T09:00:00+00:00"},
                            {"at_ms": local_ms("2026-01-08 23:00"), "percent": 10.0, "resets_at": "2026-01-14T09:00:00+00:00"},
                        ],
                    },
                ],
            },
        ])
    );
}

fn limits_reading(
    at: &str,
    limits: Vec<gigawatts::contract::Limit>,
) -> gigawatts::store::LimitsReading {
    gigawatts::store::LimitsReading {
        fetched_at_ms: local_ms(at),
        source: gigawatts::contract::LimitSource::GetUsage,
        limits,
    }
}

fn a_limit(
    kind: &str,
    scope: Option<&str>,
    percent: f64,
    resets_at: &str,
) -> gigawatts::contract::Limit {
    gigawatts::contract::Limit {
        kind: kind.to_string(),
        group: Some("weekly".to_string()),
        scope: scope.map(str::to_string),
        percent,
        severity: None,
        resets_at: Some(resets_at.to_string()),
        active: true,
        credits: None,
    }
}

#[test]
fn a_preset_range_starts_on_its_first_day_and_has_no_end() {
    let home = home_with_spend_today("report_port = 0\n");
    let port = serving(&home);
    let mut store = Store::open(&Paths::new(&home.path).database()).unwrap();
    let personal: gigawatts::sources::claude_json::Account =
        "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0"
            .parse()
            .unwrap();
    let (monday, today): (String, String) = rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT date('now', 'localtime', '-6 days', 'weekday 1'), date('now', 'localtime')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let last_week = local_ms(&format!("{monday} 00:00")) - 60_000;
    let this_morning = local_ms(&format!("{today} 00:01"));
    for at_ms in [last_week, this_morning] {
        store
            .record_limits(
                &personal,
                &gigawatts::store::LimitsReading {
                    fetched_at_ms: at_ms,
                    source: gigawatts::contract::LimitSource::GetUsage,
                    limits: vec![a_limit(
                        "weekly_all",
                        None,
                        if at_ms == last_week { 90.0 } else { 3.0 },
                        "2099-01-01T00:00:00+00:00",
                    )],
                },
            )
            .unwrap();
    }

    let history = json(&get(port, "/api/limits?range=week", ""));

    assert_eq!(history["since"], monday);
    assert_eq!(history["until"], serde_json::Value::Null);
    let points: Vec<(i64, f64)> = history["accounts"][0]["limits"][0]["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| {
            (
                point["at_ms"].as_i64().unwrap(),
                point["percent"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(points, vec![(last_week, 90.0), (this_morning, 3.0)]);
}

#[test]
fn api_insights_answers_the_last_eight_weeks_cost_per_run() {
    let today: String = rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row("SELECT date('now', 'localtime')", [], |row| row.get(0))
        .unwrap();
    let home = home_with_lines(&[request(
        "s-a",
        "r1",
        &local_noon_in_utc(&today),
        Some("review"),
        serde_json::json!({"input_tokens": 250_000, "output_tokens": 0}),
    )]);
    let port = serving(&home);

    let insights = json(&get(port, "/api/insights", ""));

    let response: gigawatts::contract::InsightsResponse = serde_json::from_value(insights).unwrap();
    assert_eq!(response.weeks.len(), 8);
    let [review] = response.skills.as_slice() else {
        panic!("{:?}", response.skills);
    };
    assert_eq!(review.name, "review");
    assert_eq!(review.runs, 1);
    assert_eq!(review.cost_per_run_usd, Some(1.0));
}

#[test]
fn api_insights_carries_each_rows_takeaways() {
    let today: String = rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row("SELECT date('now', 'localtime')", [], |row| row.get(0))
        .unwrap();
    let home = home_with_lines(&[request(
        "s-a",
        "r1",
        &local_noon_in_utc(&today),
        Some("review"),
        serde_json::json!({"input_tokens": 250_000, "output_tokens": 0}),
    )]);
    let port = serving(&home);

    let answer = &json(&get(port, "/api/insights", ""))["takeaways"];

    assert_eq!(answer["skills"][0]["name"], "review");
    assert_eq!(answer["skills"][0]["to_save_usd"], serde_json::Value::Null);
    assert_eq!(
        answer["skills"][0]["takeaways"][0]["title"],
        "Too few runs to judge"
    );
    assert_eq!(answer["agents"], serde_json::json!([]));
}
