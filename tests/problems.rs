use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use gigawatts::accounts::Attribution;
use gigawatts::contract::{Problem, ProblemKind};
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::PriceTable;
use gigawatts::problems;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

const TIMESTAMP: &str = "2026-01-08T12:00:00.000Z";

fn request(request: &str, model: &str, input: u64, output: u64) -> String {
    request_line("s1", request, model, input, output)
}

fn request_line(session: &str, request: &str, model: &str, input: u64, output: u64) -> String {
    serde_json::json!({
        "type": "assistant",
        "sessionId": session,
        "requestId": request,
        "timestamp": TIMESTAMP,
        "cwd": "/Users/nobody/gone",
        "gitBranch": "main",
        "message": {
            "model": model,
            "usage": {"input_tokens": input, "output_tokens": output}
        }
    })
    .to_string()
        + "\n"
}

fn cost_state(session: &str, input: u64, output: u64, cost_usd: f64) -> String {
    serde_json::json!({
        "type": "cost-state",
        "sessionId": session,
        "totalCostUSD": cost_usd,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": input,
                "outputTokens": output,
                "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0,
                "webSearchRequests": 0,
                "costUSD": cost_usd
            }
        }
    })
    .to_string()
        + "\n"
}

const UNRECOGNIZED: &str = r#"{"type":"assistant-v2","timestamp":"2026-01-08T12:00:00.000Z","message":{"usage":{"input_tokens":1}}}
"#;

struct Scratch {
    dir: tempfile::TempDir,
    projects: PathBuf,
    store: Store,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        fs::create_dir_all(projects.join("p")).unwrap();
        let store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
        Scratch {
            dir,
            projects,
            store,
        }
    }

    fn append(&self, file: &str, text: &str) {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.projects.join("p").join(file))
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn ingest(&mut self) {
        ingest_tree(
            &mut self.store,
            &self.projects,
            &PriceTable::built_in(),
            &Attribution::default(),
            &Resolver::default(),
        )
        .unwrap();
    }

    fn problems(&self) -> Vec<Problem> {
        problems::collect(&self.store, &PriceTable::built_in()).unwrap()
    }

    fn database(&self) -> PathBuf {
        self.dir.path().join("history.sqlite3")
    }

    fn execute(&self, sql: &str) {
        rusqlite::Connection::open(self.database())
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }

    fn reopen(&mut self) {
        self.store = Store::open(&self.database()).unwrap();
    }

    fn offset(&self, file: &str) -> u64 {
        let path = self.projects.join("p").join(file);
        self.store
            .position(&path.to_string_lossy())
            .unwrap()
            .unwrap()
            .offset
    }
}

fn kinds(problems: &[Problem]) -> Vec<ProblemKind> {
    problems.iter().map(|problem| problem.kind).collect()
}

#[test]
fn a_model_with_no_price_is_named_in_one_problem() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-opus-5-5", 1_000, 100)
            + &request("r2", "claude-invented-9", 1_000, 100)
            + &request("r3", "claude-invented-9", 2_000, 200)),
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(kinds(&problems), vec![ProblemKind::NoPrice], "{problems:?}");
    assert_eq!(problems[0].cause, "no price for claude-invented-9");
    assert_eq!(problems[0].affects, "cost: claude-invented-9 in 2 requests");
    assert!(problems[0].fix.contains("[prices.claude-invented-9]"));
    assert!(
        problems[0].fix.contains("then run `gigawatts ingest`"),
        "{}",
        problems[0].fix
    );
    assert_eq!(problems[0].since, Some(local_time(TIMESTAMP)));
}

#[test]
fn a_model_with_no_price_seen_only_in_a_cost_state_counts_its_sessions() {
    let mut scratch = Scratch::new();
    let cost_state = serde_json::json!({
        "type": "cost-state",
        "sessionId": "s1",
        "totalCostUSD": 0.25,
        "startTime": 1767873600000_i64,
        "modelUsage": {
            "claude-invented-9": {
                "inputTokens": 2_000, "outputTokens": 300, "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.25
            }
        }
    });
    scratch.append("s1.jsonl", &(cost_state.to_string() + "\n"));

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(kinds(&problems), vec![ProblemKind::NoPrice], "{problems:?}");
    assert_eq!(
        problems[0].affects,
        "cost: what Claude Code recorded for claude-invented-9 in 1 session"
    );
}

#[test]
fn a_price_override_clears_the_no_price_problem() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", &request("r1", "claude-invented-9", 1_000, 100));
    scratch.ingest();
    let config = gigawatts::config::Config::parse(
        "[prices.claude-invented-9]\ninput = 1.0\ncache_write_5m = 1.25\ncache_write_1h = 2.0\ncache_read = 0.1\noutput = 5.0\n",
        scratch.dir.path(),
    )
    .unwrap();

    let problems = problems::collect(&scratch.store, &config.price_table()).unwrap();

    assert_eq!(problems, vec![]);
}

#[test]
fn an_adjusted_session_is_a_price_gap_problem_naming_its_model() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-opus-5-5", 100_000, 10_000)
            + &request("r2", "claude-opus-5-5", 0, 30_000)
            + &cost_state("s1", 100_000, 40_000, 1.8)),
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::PriceGap],
        "{problems:?}"
    );
    assert_eq!(
        problems[0].cause,
        "claude-opus-5-5's prices differ from Claude Code's record by at least $0.60"
    );
    assert_eq!(problems[0].affects, "cost: claude-opus-5-5 in 1 session");
    assert!(
        problems[0].fix.contains("run `gigawatts ingest`"),
        "{}",
        problems[0].fix
    );
}

#[test]
fn a_price_above_claude_codes_record_despite_missing_tokens_is_a_price_gap_problem() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-opus-5-5", 100_000, 10_000)
            + &cost_state("s1", 150_000, 12_000, 0.1)),
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::PriceGap],
        "{problems:?}"
    );
    assert_eq!(
        problems[0].cause,
        "claude-opus-5-5's prices differ from Claude Code's record by at least $0.50"
    );
}

#[test]
fn a_cost_state_recording_no_cost_is_not_a_problem() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-opus-5-5", 100_000, 10_000)
            + &cost_state("s1", 100_000, 10_000, 0.0)),
    );

    scratch.ingest();

    assert_eq!(scratch.problems(), vec![]);
}

#[test]
fn price_gaps_in_opposite_directions_add_up_instead_of_cancelling() {
    let mut scratch = Scratch::new();
    let session = |id: &str, recorded: f64| {
        let line = |request: &str, input: u64, output: u64| {
            request_line(id, request, "claude-opus-5-5", input, output)
        };
        line(&format!("{id}-r1"), 100_000, 10_000)
            + &line(&format!("{id}-r2"), 0, 30_000)
            + &cost_state(id, 100_000, 40_000, recorded)
    };
    scratch.append("s1.jsonl", &session("s1", 1.8));
    scratch.append("s2.jsonl", &session("s2", 0.6));

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        problems[0].cause,
        "claude-opus-5-5's prices differ from Claude Code's record by at least $1.20"
    );
    assert_eq!(problems[0].affects, "cost: claude-opus-5-5 in 2 sessions");
}

#[test]
fn spend_missing_from_the_transcript_is_not_a_problem() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-opus-5-5", 100_000, 10_000)
            + &cost_state("s1", 150_000, 12_000, 1.0)),
    );

    scratch.ingest();

    assert_eq!(scratch.problems(), vec![]);
}

#[test]
fn unrecognized_lines_stay_a_problem_after_later_reads() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", &(UNRECOGNIZED.to_string() + UNRECOGNIZED));
    scratch.append("s2.jsonl", UNRECOGNIZED);
    scratch.ingest();
    scratch.append("s1.jsonl", &request("r1", "claude-opus-5-5", 1_000, 100));

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::UnrecognizedLines],
        "{problems:?}"
    );
    assert_eq!(
        problems[0].cause,
        "Lines of type \"assistant-v2\" that carry usage (3 in 2 files)"
    );
    assert_eq!(problems[0].since, Some(local_time(TIMESTAMP)));
}

#[test]
fn an_appended_unrecognized_line_adds_to_the_count_and_keeps_the_first_since() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", UNRECOGNIZED);
    scratch.ingest();
    scratch.append(
        "s1.jsonl",
        &UNRECOGNIZED.replace("2026-01-08T12:00:00.000Z", "2026-01-09T12:00:00.000Z"),
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        problems[0].cause,
        "Lines of type \"assistant-v2\" that carry usage (2 in 1 file)"
    );
    assert_eq!(problems[0].since, Some(local_time(TIMESTAMP)));
}

#[test]
fn a_line_with_no_type_is_described_as_having_none() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        "{\"timestamp\":\"2026-01-08T12:00:00.000Z\",\"message\":{\"usage\":{}}}\n",
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        problems[0].cause,
        "Lines with no type that carry usage (1 in 1 file)"
    );
}

#[test]
fn unrecognized_lines_in_a_deleted_transcript_stop_being_a_problem() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", UNRECOGNIZED);
    scratch.ingest();
    fs::remove_file(scratch.projects.join("p").join("s1.jsonl")).unwrap();

    scratch.ingest();

    assert_eq!(scratch.problems(), vec![]);
}

#[test]
fn each_special_kind_of_unreadable_line_says_what_went_wrong() {
    let mut scratch = Scratch::new();
    let no_timestamp = r#"{"type":"assistant","sessionId":"s1","requestId":"r1","message":{"model":"claude-opus-5-5","usage":{"input_tokens":1,"output_tokens":1}}}"#;
    let broken_cost_state = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":1.0}"#;
    scratch.append(
        "s1.jsonl",
        &format!("{{\"type\":\"usage\" \n{no_timestamp}\n{broken_cost_state}\n"),
    );

    scratch.ingest();

    let causes: Vec<(String, String)> = scratch
        .problems()
        .into_iter()
        .map(|problem| (problem.cause, problem.affects))
        .collect();
    assert_eq!(
        causes,
        vec![
            (
                "Assistant lines with usage but no timestamp (1 in 1 file)".to_string(),
                "history: those requests are missing from every total, because they can't be placed on a day".to_string()
            ),
            (
                "Cost-state lines that don't parse (1 in 1 file)".to_string(),
                "cost: those sessions stay estimated instead of matching Claude Code's record".to_string()
            ),
            (
                "Transcript lines mentioning usage that don't parse (1 in 1 file)".to_string(),
                "history: any requests on those lines are missing from every total".to_string()
            ),
        ]
    );
}

#[test]
fn an_unreadable_rename_says_the_session_keeps_its_generated_title() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        "{\"type\":\"custom-title\",\"sessionId\":\"s1\"}\n",
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        (problems[0].cause.as_str(), problems[0].affects.as_str()),
        (
            "Rename lines that don't parse (1 in 1 file)",
            "session titles: renamed sessions show the generated title instead"
        )
    );
}

#[test]
fn reading_a_file_again_from_the_start_counts_its_unrecognized_lines_once() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", UNRECOGNIZED);
    scratch.ingest();
    let projects = scratch.projects.clone();
    scratch.store.forget_positions_under(&projects).unwrap();

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        problems[0].cause,
        "Lines of type \"assistant-v2\" that carry usage (1 in 1 file)"
    );
}

fn local_time(timestamp: &str) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row("SELECT datetime(?1, 'localtime')", [timestamp], |row| {
            row.get(0)
        })
        .unwrap()
}

fn gigawatts(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gigawatts"))
        .args(args)
        .env("HOME", home)
        .env("PATH", path_with_stand_in_claude())
        .output()
        .unwrap()
}

fn home_with(transcript: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let projects = home.path().join(".claude").join("projects").join("p");
    fs::create_dir_all(&projects).unwrap();
    fs::write(projects.join("s1.jsonl"), transcript).unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000e0"}}"#,
    )
    .unwrap();
    home
}

#[test]
fn doctor_lists_each_problem_and_exits_nonzero() {
    let home = home_with(&request("r1", "claude-invented-9", 1_000, 100));
    assert_eq!(gigawatts(home.path(), &["ingest"]).status.code(), Some(0));

    let output = gigawatts(home.path(), &["doctor"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stdout.contains("no price for claude-invented-9"),
        "{stdout}"
    );
    assert!(stdout.contains("[prices.claude-invented-9]"), "{stdout}");
}

#[test]
fn doctor_exits_zero_when_nothing_is_wrong() {
    let home = home_with(&request("r1", "claude-opus-5-5", 1_000, 100));
    assert_eq!(gigawatts(home.path(), &["ingest"]).status.code(), Some(0));

    let output = gigawatts(home.path(), &["doctor"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "No problems.\n\nPrices: the built-in table, copied 2026-11-11 from https://platform.claude.com/docs/en/about-claude/pricing.md.\n"
    );
}

#[test]
fn doctor_names_the_models_priced_from_the_downloaded_page_and_when() {
    let home = home_with(&request("r1", "claude-opus-5-5", 1_000, 100));
    assert_eq!(gigawatts(home.path(), &["ingest"]).status.code(), Some(0));
    let mut store =
        Store::open(&home.path().join(".local/share/gigawatts/history.sqlite3")).unwrap();
    let prices = *PriceTable::built_in().model("claude-opus-5-5").unwrap();
    let fetched_at_ms = 1_767_873_600_000;
    store
        .save_downloaded_prices(&gigawatts::store::DownloadedPrices {
            fetched_at_ms,
            models: [
                ("claude-opus-5-6".to_string(), prices),
                ("claude-opus-5-5".to_string(), prices),
            ]
            .into(),
        })
        .unwrap();
    let fetched = store.local_datetime(fetched_at_ms).unwrap().unwrap();

    let output = gigawatts(home.path(), &["doctor"]);

    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "No problems.\n\nPrices: the built-in table, copied 2026-11-11, with claude-opus-5-5 and claude-opus-5-6 from https://platform.claude.com/docs/en/about-claude/pricing.md, downloaded {fetched}.\n"
        )
    );
}

#[test]
fn data_ends_with_a_pointer_to_doctor_when_anything_is_wrong() {
    let home = home_with(&request("r1", "claude-invented-9", 1_000, 100));
    assert_eq!(gigawatts(home.path(), &["ingest"]).status.code(), Some(0));

    let output = gigawatts(home.path(), &["data", "--range", "all"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.ends_with("\n1 problem. `gigawatts doctor` lists it.\n"),
        "{stdout}"
    );
}

fn ingest_home(store: &mut Store, home: &Path) {
    let config = gigawatts::config::Config::default();
    let directories = gigawatts::accounts::discover(home, &config);
    gigawatts::accounts::ingest(
        store,
        &directories,
        &config,
        &PriceTable::built_in(),
        &Resolver::default(),
    )
    .unwrap();
}

#[test]
fn unrecognized_lines_in_a_projects_folder_that_is_gone_stop_being_a_problem() {
    let home = home_with(UNRECOGNIZED);
    let mut store = Store::open(&home.path().join("history.sqlite3")).unwrap();
    ingest_home(&mut store, home.path());
    fs::remove_dir_all(home.path().join(".claude").join("projects")).unwrap();

    ingest_home(&mut store, home.path());

    assert_eq!(
        problems::collect(&store, &PriceTable::built_in()).unwrap(),
        vec![]
    );
}

#[test]
fn unrecognized_lines_stay_while_their_directorys_login_cant_be_read() {
    let home = home_with(UNRECOGNIZED);
    let mut store = Store::open(&home.path().join("history.sqlite3")).unwrap();
    ingest_home(&mut store, home.path());
    fs::write(home.path().join(".claude.json"), "{").unwrap();

    ingest_home(&mut store, home.path());

    let problems = problems::collect(&store, &PriceTable::built_in()).unwrap();
    assert_eq!(kinds(&problems), vec![ProblemKind::UnrecognizedLines]);
}

#[test]
fn an_unrecognized_line_stamped_past_year_9999_still_lists_without_a_since() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        "{\"type\":\"assistant-v2\",\"timestamp\":\"9999-12-31T23:59:59-23:59\",\"message\":{\"usage\":{}}}\n",
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(kinds(&problems), vec![ProblemKind::UnrecognizedLines]);
    assert_eq!(problems[0].since, None);
}

#[test]
fn dated_versions_of_one_unpriced_model_are_one_problem() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-invented-9-20260101", 1_000, 100)
            + &request("r2", "claude-invented-9-20260301", 1_000, 100)),
    );

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(kinds(&problems), vec![ProblemKind::NoPrice], "{problems:?}");
    assert_eq!(problems[0].cause, "no price for claude-invented-9");
    assert_eq!(problems[0].affects, "cost: claude-invented-9 in 2 requests");
}

#[test]
fn doctor_prints_what_each_problem_affects_without_a_label() {
    let home = home_with(&request("r1", "claude-invented-9", 1_000, 100));
    assert_eq!(gigawatts(home.path(), &["ingest"]).status.code(), Some(0));

    let output = gigawatts(home.path(), &["doctor"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\n  cost: claude-invented-9 in 1 request\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("Affects"), "{stdout}");
}

#[test]
fn unrecognized_lines_count_again_when_a_missing_projects_folder_returns() {
    let home = home_with(UNRECOGNIZED);
    let mut store = Store::open(&home.path().join("history.sqlite3")).unwrap();
    ingest_home(&mut store, home.path());
    let projects = home.path().join(".claude").join("projects");
    let away = home.path().join("projects-away");
    fs::rename(&projects, &away).unwrap();
    ingest_home(&mut store, home.path());
    fs::rename(&away, &projects).unwrap();

    ingest_home(&mut store, home.path());

    let problems = problems::collect(&store, &PriceTable::built_in()).unwrap();
    assert_eq!(kinds(&problems), vec![ProblemKind::UnrecognizedLines]);
}

#[test]
fn unrecognized_lines_count_again_when_a_missing_transcript_returns() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", UNRECOGNIZED);
    scratch.ingest();
    let transcript = scratch.projects.join("p").join("s1.jsonl");
    let away = scratch.dir.path().join("s1.jsonl");
    fs::rename(&transcript, &away).unwrap();
    scratch.ingest();
    fs::rename(&away, &transcript).unwrap();

    scratch.ingest();

    assert_eq!(
        kinds(&scratch.problems()),
        vec![ProblemKind::UnrecognizedLines]
    );
}

#[test]
fn a_forgotten_session_is_still_reset_after_its_transcript_was_missing_for_a_read() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(UNRECOGNIZED.to_string() + &request("r1", "claude-opus-5-5", 1_000, 100)),
    );
    scratch.ingest();
    let database = rusqlite::Connection::open(scratch.dir.path().join("history.sqlite3")).unwrap();
    database
        .execute("UPDATE sessions SET cost_forgotten = 1", [])
        .unwrap();
    let transcript = scratch.projects.join("p").join("s1.jsonl");
    let away = scratch.dir.path().join("s1.jsonl");
    fs::rename(&transcript, &away).unwrap();
    scratch.ingest();
    fs::rename(&away, &transcript).unwrap();

    let projects = scratch.projects.clone();
    scratch.store.forget_positions_under(&projects).unwrap();

    let forgotten: i64 = database
        .query_row(
            "SELECT count(*) FROM sessions WHERE cost_forgotten",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(forgotten, 0);
}

#[test]
fn dated_versions_of_one_model_are_one_price_gap_problem() {
    let mut scratch = Scratch::new();
    let session = |id: &str, model: &str| {
        let recorded = serde_json::json!({
            "type": "cost-state",
            "sessionId": id,
            "totalCostUSD": 1.8,
            "modelUsage": {
                model: {
                    "inputTokens": 100_000, "outputTokens": 40_000, "cacheReadInputTokens": 0,
                    "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 1.8
                }
            }
        });
        request_line(id, &format!("{id}-r1"), model, 100_000, 10_000)
            + &request_line(id, &format!("{id}-r2"), model, 0, 30_000)
            + &recorded.to_string()
            + "\n"
    };
    scratch.append("s1.jsonl", &session("s1", "claude-opus-5-5-20260101"));
    scratch.append("s2.jsonl", &session("s2", "claude-opus-5-5-20260301"));

    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::PriceGap],
        "{problems:?}"
    );
    assert_eq!(problems[0].affects, "cost: claude-opus-5-5 in 2 sessions");
    assert_eq!(
        problems[0].cause,
        "claude-opus-5-5's prices differ from Claude Code's record by at least $1.20"
    );
}

#[test]
fn a_newer_transcript_reader_reads_every_transcript_again() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", UNRECOGNIZED);
    scratch.ingest();
    let read_to = UNRECOGNIZED.len() as u64;

    scratch.reopen();
    assert_eq!(scratch.offset("s1.jsonl"), read_to);

    scratch.execute("UPDATE store_settings SET value = '0' WHERE key = 'reader_version'");
    scratch.reopen();
    assert_eq!(scratch.offset("s1.jsonl"), 0);
}

#[test]
fn upgrading_to_problems_reports_lines_already_read_and_drops_gaps_on_no_cost() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(request("r1", "claude-opus-5-5", 100_000, 10_000)
            + UNRECOGNIZED
            + &cost_state("s1", 100_000, 10_000, 0.0)),
    );
    scratch.ingest();
    scratch.execute(
        "ALTER TABLE sessions DROP COLUMN title_is_custom;
         ALTER TABLE not_in_transcript DROP COLUMN cwd_label;
         ALTER TABLE not_in_transcript DROP COLUMN branch_label;
         DROP TABLE limit_fetches;
         DROP TABLE limit_readings;
         DROP TABLE usage_advice;
         ALTER TABLE sessions DROP COLUMN requests_skipped;
         DROP TABLE expired_requests;
         ALTER TABLE sessions DROP COLUMN requests_expired;
         DROP INDEX requests_without_price;
         DROP INDEX session_models_by_model;
         DROP INDEX session_models_with_price_gap;
         DROP TABLE unrecognized_lines;
         UPDATE session_models SET price_gap_usd = 0.5;
         DELETE FROM store_settings WHERE key = 'reader_version';
         PRAGMA user_version = 5;",
    );

    scratch.reopen();
    scratch
        .store
        .reprice_if_changed(&PriceTable::built_in())
        .unwrap();
    scratch.ingest();

    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::UnrecognizedLines],
        "{problems:?}"
    );
    assert_eq!(
        problems[0].cause,
        "Lines of type \"assistant-v2\" that carry usage (1 in 1 file)"
    );
}

fn path_with_stand_in_claude() -> String {
    format!(
        "{}/tests/bin:{}",
        env!("CARGO_MANIFEST_DIR"),
        std::env::var("PATH").unwrap_or_default()
    )
}
