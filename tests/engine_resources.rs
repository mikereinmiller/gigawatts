use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use gigawatts::engine::Paths;

const SESSIONS: usize = 20;
const REQUESTS_PER_SESSION: usize = 500;
const MAX_RSS_KB: u64 = 50 * 1024;
const IDLE_WINDOW: Duration = Duration::from_secs(10);
const MAX_IDLE_CPU: Duration = Duration::from_millis(50);

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

fn home_with_transcripts(config_text: &str) -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = Home {
        path: dir.path().join("h"),
        _dir: dir,
    };
    let config = Paths::new(&home.path).config();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, config_text).unwrap();
    fs::write(
        home.path.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    let projects = home.path.join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    let timestamp = today_noon_utc();
    for session in 0..SESSIONS {
        let lines: String = (0..REQUESTS_PER_SESSION)
            .map(|request| {
                format!(
                    r#"{{"type":"assistant","sessionId":"s{session}","requestId":"r{session}-{request}","timestamp":"{timestamp}","cwd":"/work/repo-{}","attributionSkill":"skill-{}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1000,"cache_read_input_tokens":20000,"output_tokens":500}}}}}}"#,
                    session % 5,
                    request % 4
                ) + "\n"
            })
            .collect();
        fs::write(projects.join(format!("s{session}.jsonl")), lines).unwrap();
    }
    home
}

fn requests_today(snapshot: &Path) -> Option<u64> {
    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(snapshot).ok()?).ok()?;
    Some(
        snapshot["accounts"]
            .as_array()?
            .iter()
            .filter_map(|account| account["requests"].as_u64())
            .sum(),
    )
}

fn wait_for_requests(snapshot: &Path, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if requests_today(snapshot) == Some(expected) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the snapshot never showed {expected} requests today");
}

fn engine_pid(paths: &Paths) -> String {
    fs::read_to_string(paths.lock()).unwrap().trim().to_string()
}

fn ps(pid: &str, field: &str) -> String {
    let output = Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", pid])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn cpu_time(pid: &str) -> Duration {
    if cfg!(target_os = "linux") {
        return linux_cpu_time(pid);
    }
    let seconds = ps(pid, "time").split(':').fold(0.0, |total: f64, part| {
        total * 60.0 + part.parse::<f64>().unwrap()
    });
    Duration::from_secs_f64(seconds)
}

// procps `ps -o time` prints whole seconds, too coarse for MAX_IDLE_CPU.
fn linux_cpu_time(pid: &str) -> Duration {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let (_, after_name) = stat.rsplit_once(')').unwrap();
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    let ticks: f64 = fields[11].parse::<f64>().unwrap() + fields[12].parse::<f64>().unwrap();
    let output = Command::new("getconf").arg("CLK_TCK").output().unwrap();
    let ticks_per_second: f64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap();
    Duration::from_secs_f64(ticks / ticks_per_second)
}

fn assert_small_and_idle(config_text: &str) {
    let home = home_with_transcripts(config_text);
    let paths = Paths::new(&home.path);
    gigawatts(&home.path, &["engine"]);
    wait_for_requests(&paths.snapshot(), (SESSIONS * REQUESTS_PER_SESSION) as u64);
    let pid = engine_pid(&paths);

    let rss_kb: u64 = ps(&pid, "rss").parse().unwrap();
    let before = cpu_time(&pid);
    std::thread::sleep(IDLE_WINDOW);
    let idle_cpu = cpu_time(&pid).saturating_sub(before);

    println!(
        "{config_text:?}: rss {rss_kb} KB, cpu reading the history {before:?}, cpu while idle {idle_cpu:?} over {IDLE_WINDOW:?}"
    );
    assert!(rss_kb <= MAX_RSS_KB, "the engine holds {rss_kb} KB");
    assert!(
        idle_cpu <= MAX_IDLE_CPU,
        "the engine used {idle_cpu:?} of CPU while idle for {IDLE_WINDOW:?}"
    );
}

#[test]
fn the_engine_stays_small_after_reading_history_and_uses_no_cpu_while_idle() {
    assert_small_and_idle("report_port = 0\n");
}

#[test]
fn serving_the_report_page_keeps_the_engine_small_and_idle() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    assert_small_and_idle(&format!("report_port = {port}\n"));
}
