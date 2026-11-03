use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use gigawatts::accounts::Attribution;
use gigawatts::contract::{CostStatus, DataRow, GroupBy, Level, Range};
use gigawatts::data::{self, query};
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::{ModelPrices, PriceTable, Prices};
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

const DAY: &str = "2026-01-08";
const TIMESTAMP: &str = "2026-01-08T12:00:00.000Z";

struct Line<'a> {
    session: &'a str,
    request: &'a str,
    cwd: &'a str,
    branch: &'a str,
    skill: Option<&'a str>,
    agent: Option<&'a str>,
    input: u64,
    output: u64,
}

impl Default for Line<'_> {
    fn default() -> Self {
        Line {
            session: "s1",
            request: "r1",
            cwd: "/Users/nobody/gone",
            branch: "main",
            skill: None,
            agent: None,
            input: 100_000,
            output: 10_000,
        }
    }
}

impl Line<'_> {
    fn json(&self) -> String {
        let mut line = serde_json::json!({
            "type": "assistant",
            "sessionId": self.session,
            "requestId": self.request,
            "timestamp": TIMESTAMP,
            "cwd": self.cwd,
            "gitBranch": self.branch,
            "message": {
                "model": "claude-opus-5-5",
                "usage": {"input_tokens": self.input, "output_tokens": self.output}
            }
        });
        if let Some(skill) = self.skill {
            line["attributionSkill"] = skill.into();
        }
        if let Some(agent) = self.agent {
            line["attributionAgent"] = agent.into();
        }
        line.to_string() + "\n"
    }
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

struct Scratch {
    dir: tempfile::TempDir,
    projects: PathBuf,
    store: Store,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects").join("p");
        fs::create_dir_all(&projects).unwrap();
        let store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
        Scratch {
            dir,
            projects,
            store,
        }
    }

    fn append(&self, file: &str, text: &str) {
        let path = self.projects.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn ingest_with(&mut self, table: &PriceTable) {
        ingest_tree(
            &mut self.store,
            &self.dir.path().join("projects"),
            table,
            &Attribution::default(),
            &Resolver::default(),
        )
        .unwrap();
    }

    fn ingest(&mut self) {
        self.ingest_with(&PriceTable::built_in());
    }

    fn rows(&self, by: GroupBy, drill: &[&str]) -> Vec<DataRow> {
        query(&self.store, Range::All, by, drill, DAY).unwrap().rows
    }

    fn sql<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        rusqlite::Connection::open(self.dir.path().join("history.sqlite3"))
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }
}

fn labels(rows: &[DataRow]) -> Vec<(String, u64)> {
    rows.iter()
        .map(|row| (row.label.clone(), row.requests))
        .collect()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn worktree_of(root: &Path, repo: &str) -> PathBuf {
    let main = root.join(repo);
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let worktree = root.join(format!("{repo}-rel42"));
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "ab/rel42",
            worktree.to_str().unwrap(),
        ],
    );
    worktree
}

fn assert_cost(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("a cost");
    assert!((actual - expected).abs() < 1e-9, "{actual} vs {expected}");
}

#[test]
fn grouping_by_repo_names_a_worktrees_main_checkout() {
    let mut scratch = Scratch::new();
    let worktree = worktree_of(scratch.dir.path(), "orchard-api");
    let worktree = worktree.to_str().unwrap();
    scratch.append(
        "s1.jsonl",
        &(Line {
            request: "r1",
            cwd: worktree,
            ..Line::default()
        }
        .json()
            + &Line {
                request: "r2",
                cwd: worktree,
                ..Line::default()
            }
            .json()
            + &Line {
                request: "r3",
                cwd: "/tmp/scratch",
                ..Line::default()
            }
            .json()),
    );

    scratch.ingest();

    assert_eq!(
        labels(&scratch.rows(GroupBy::Repo, &[])),
        vec![("orchard-api".to_string(), 2), ("temporary".to_string(), 1)]
    );
}

#[test]
fn drilling_goes_from_repo_to_branch_to_session() {
    let mut scratch = Scratch::new();
    let worktree = worktree_of(scratch.dir.path(), "orchard-api");
    let worktree = worktree.to_str().unwrap();
    let line = |session, request, branch| Line {
        session,
        request,
        cwd: worktree,
        branch,
        ..Line::default()
    };
    scratch.append(
        "s1.jsonl",
        &(line("s1", "r1", "ab/rel42").json() + &line("s1", "r2", "ab/rel42").json()),
    );
    scratch.append(
        "s2.jsonl",
        &(line("s2", "r3", "ab/rel42").json() + &line("s2", "r4", "main").json()),
    );
    scratch.ingest();

    let branches = query(
        &scratch.store,
        Range::All,
        GroupBy::Repo,
        &["orchard-api"],
        DAY,
    )
    .unwrap();
    assert_eq!(branches.level, Level::Branch);
    assert_eq!(
        labels(&branches.rows),
        vec![("ab/rel42".to_string(), 3), ("main".to_string(), 1)]
    );

    let sessions = query(
        &scratch.store,
        Range::All,
        GroupBy::Repo,
        &["orchard-api", "ab/rel42"],
        DAY,
    )
    .unwrap();
    assert_eq!(sessions.level, Level::Session);
    let sessions: Vec<_> = sessions
        .rows
        .iter()
        .map(|row| (row.key.clone(), row.requests, row.status))
        .collect();
    assert_eq!(
        sessions,
        vec![
            (Some("s1".to_string()), 2, Some(CostStatus::Estimated)),
            (Some("s2".to_string()), 1, Some(CostStatus::Estimated)),
        ]
    );
}

#[test]
fn drilling_into_a_name_two_repos_share_is_refused() {
    let mut scratch = Scratch::new();
    let first = worktree_of(&scratch.dir.path().join("corp"), "api");
    let second = worktree_of(&scratch.dir.path().join("band"), "api");
    scratch.append(
        "s1.jsonl",
        &(Line {
            request: "r1",
            cwd: first.to_str().unwrap(),
            ..Line::default()
        }
        .json()
            + &Line {
                request: "r2",
                cwd: second.to_str().unwrap(),
                ..Line::default()
            }
            .json()),
    );
    scratch.ingest();

    assert_eq!(
        labels(&scratch.rows(GroupBy::Repo, &[])),
        vec![("api".to_string(), 1), ("api".to_string(), 1)]
    );
    assert!(matches!(
        query(&scratch.store, Range::All, GroupBy::Repo, &["api"], DAY),
        Err(data::Error::Ambiguous { .. })
    ));
    let path = scratch.rows(GroupBy::Repo, &[])[0].key.clone().unwrap();
    let by_path = query(&scratch.store, Range::All, GroupBy::Repo, &[&path], DAY).unwrap();
    assert_eq!(labels(&by_path.rows), vec![("main".to_string(), 1)]);
}

#[test]
fn head_in_a_folder_git_cant_read_is_detached() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &Line {
            branch: "HEAD",
            ..Line::default()
        }
        .json(),
    );

    scratch.ingest();

    assert_eq!(
        labels(&scratch.rows(GroupBy::Branch, &[])),
        vec![("detached".to_string(), 1)]
    );
}

#[test]
fn lines_without_a_skill_or_agent_belong_to_the_main_conversation() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line {
            request: "r1",
            skill: Some("review"),
            agent: Some("code-reviewer"),
            ..Line::default()
        }
        .json()
            + &Line {
                request: "r2",
                ..Line::default()
            }
            .json()),
    );

    scratch.ingest();

    assert_eq!(
        labels(&scratch.rows(GroupBy::Skill, &[])),
        vec![("no skill".to_string(), 1), ("review".to_string(), 1)]
    );
    assert_eq!(
        labels(&scratch.rows(GroupBy::Agent, &[])),
        vec![
            ("code-reviewer".to_string(), 1),
            ("main conversation".to_string(), 1)
        ]
    );
}

const PRICED: f64 = 100_000.0 * 4.0 / 1e6 + 10_000.0 * 20.0 / 1e6;

#[test]
fn a_session_is_estimated_until_its_cost_state_and_exact_after() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", &Line::default().json());
    scratch.ingest();
    assert_eq!(
        scratch.rows(GroupBy::Model, &["claude-opus-5-5"])[0].status,
        Some(CostStatus::Estimated)
    );

    scratch.append("s1.jsonl", &cost_state("s1", 100_000, 10_000, PRICED));
    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_cost(rows[0].cost_usd, 0.6);
    assert_eq!(
        scratch.rows(GroupBy::Model, &["claude-opus-5-5"])[0].status,
        Some(CostStatus::Exact)
    );
}

#[test]
fn spend_missing_from_the_transcript_gets_its_own_row() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 150_000, 12_000, 1.0)),
    );

    scratch.ingest();

    let rows = scratch.rows(GroupBy::Repo, &[]);
    let rows: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                row.label.clone(),
                row.not_in_transcript,
                row.requests,
                row.input_tokens,
                row.output_tokens,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("gone".to_string(), false, 1, 100_000, 10_000),
            ("not in transcript".to_string(), true, 0, 50_000, 2_000),
        ]
    );
    let by_model = scratch.rows(GroupBy::Model, &[]);
    assert_cost(by_model[0].cost_usd, 0.6);
    assert_cost(by_model[1].cost_usd, 0.4);
    assert_eq!(
        (by_model[1].key.as_deref(), by_model[1].not_in_transcript),
        (Some("claude-opus-5-5"), true)
    );
    let sessions = scratch.rows(GroupBy::Model, &["claude-opus-5-5"]);
    assert_eq!(sessions[0].status, Some(CostStatus::Exact));
}

#[test]
fn a_price_gap_with_matching_tokens_scales_requests_and_marks_the_session_adjusted() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line {
            request: "r1",
            ..Line::default()
        }
        .json()
            + &Line {
                request: "r2",
                input: 0,
                output: 30_000,
                ..Line::default()
            }
            .json()
            + &cost_state("s1", 100_000, 40_000, 1.8)),
    );

    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_cost(rows[0].cost_usd, 1.8);
    let sessions = scratch.rows(GroupBy::Model, &["claude-opus-5-5"]);
    assert_eq!(sessions[0].status, Some(CostStatus::Adjusted));
    let gap: f64 = scratch.sql("SELECT price_gap_usd FROM session_models");
    assert!((gap - (1.2 - 1.8)).abs() < 1e-9, "{gap}");
}

#[test]
fn subagent_requests_read_after_the_cost_state_close_the_shortfall() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 200_000, 20_000, 1.2)),
    );
    scratch.ingest();
    assert_eq!(scratch.rows(GroupBy::Model, &[]).len(), 2);

    scratch.append(
        "s1/subagents/agent-a1.jsonl",
        &Line {
            request: "r2",
            ..Line::default()
        }
        .json(),
    );
    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_cost(rows[0].cost_usd, 1.2);
    assert_eq!(rows[0].requests, 2);
}

#[test]
fn repricing_keeps_a_session_scaled_to_its_cost_state() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 100_000, 10_000, 0.9)),
    );
    scratch.ingest();
    let doubled = PriceTable::with_overrides(
        [(
            "claude-opus-5-5".to_string(),
            ModelPrices {
                standard: Prices {
                    input: 8.0,
                    cache_write_5m: 10.0,
                    cache_write_1h: 16.0,
                    cache_read: 0.4,
                    output: 40.0,
                },
                long_prompt: None,
                fast: None,
            },
        )]
        .into(),
    );

    scratch.store.reprice_if_changed(&doubled).unwrap();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_cost(rows[0].cost_usd, 0.9);
    let gap: f64 = scratch.sql("SELECT price_gap_usd FROM session_models");
    assert!((gap - (1.2 - 0.9)).abs() < 1e-9, "{gap}");
}

#[test]
fn a_cost_state_without_a_cache_split_takes_the_transcripts_1h_writes_off_the_shortfall() {
    let mut scratch = Scratch::new();
    let request = serde_json::json!({
        "type": "assistant",
        "sessionId": "s1",
        "requestId": "r1",
        "timestamp": TIMESTAMP,
        "message": {
            "model": "claude-opus-5-5",
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "cache_creation_input_tokens": 100_000,
                "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 100_000}
            }
        }
    });
    let cost_state = serde_json::json!({
        "type": "cost-state",
        "sessionId": "s1",
        "totalCostUSD": 1.3,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": 0,
                "outputTokens": 0,
                "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 160_000,
                "webSearchRequests": 0,
                "costUSD": 1.3
            }
        }
    });
    scratch.append("s1.jsonl", &format!("{request}\n{cost_state}\n"));

    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_eq!(
        (rows[1].not_in_transcript, rows[1].cache_write_tokens),
        (true, 60_000),
        "{rows:?}"
    );
}

#[test]
fn a_shortfall_priced_above_its_cost_state_is_estimated_with_the_gap_stored() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 150_000, 10_000, 0.5)),
    );

    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_cost(rows[0].cost_usd, 0.6);
    assert_eq!(
        (rows[1].not_in_transcript, rows[1].input_tokens),
        (true, 50_000)
    );
    assert_cost(rows[1].cost_usd, 0.0);
    let sessions = scratch.rows(GroupBy::Model, &["claude-opus-5-5"]);
    assert_eq!(sessions[0].status, Some(CostStatus::Estimated));
    let gap: f64 = scratch.sql("SELECT price_gap_usd FROM session_models");
    assert!((gap - (0.6 - 0.5)).abs() < 1e-9, "{gap}");
}

#[test]
fn spend_on_a_model_with_no_price_counts_as_not_in_transcript() {
    let mut scratch = Scratch::new();
    let cost_state = serde_json::json!({
        "type": "cost-state",
        "sessionId": "s1",
        "totalCostUSD": 0.85,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": 100_000, "outputTokens": 10_000, "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.6
            },
            "claude-unpriced": {
                "inputTokens": 2_000, "outputTokens": 300, "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.25
            }
        }
    });
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state.to_string() + "\n"),
    );

    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    let unpriced: Vec<_> = rows
        .iter()
        .filter(|row| row.key.as_deref() == Some("claude-unpriced"))
        .map(|row| (row.not_in_transcript, row.input_tokens, row.output_tokens))
        .collect();
    assert_eq!(unpriced, vec![(true, 2_000, 300)]);
    let total: f64 = rows.iter().filter_map(|row| row.cost_usd).sum();
    assert!((total - 0.85).abs() < 1e-9, "{total}");
    let sessions = scratch.rows(GroupBy::Model, &["claude-opus-5-5"]);
    assert_eq!(sessions[0].status, Some(CostStatus::Estimated));
}

#[test]
fn drilling_into_a_model_keeps_each_sessions_not_in_transcript_row() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 150_000, 12_000, 1.0)),
    );
    scratch.ingest();

    let sessions = scratch.rows(GroupBy::Model, &["claude-opus-5-5"]);

    let rows: Vec<_> = sessions
        .iter()
        .map(|row| {
            (
                row.key.clone(),
                row.not_in_transcript,
                row.requests,
                row.input_tokens,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (Some("s1".to_string()), false, 1, 100_000),
            (Some("s1".to_string()), true, 0, 50_000),
        ]
    );
    let total: f64 = sessions.iter().filter_map(|row| row.cost_usd).sum();
    assert!((total - 1.0).abs() < 1e-9, "{total}");
}

#[test]
fn drilling_into_no_skill_leaves_out_not_in_transcript_rows() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 150_000, 12_000, 1.0)),
    );
    scratch.ingest();

    let sessions = scratch.rows(GroupBy::Skill, &["no skill"]);

    assert_eq!(labels(&sessions), vec![("s1".to_string(), 1)]);
    assert!(
        sessions.iter().all(|row| !row.not_in_transcript),
        "{sessions:?}"
    );
}

#[test]
fn drilling_past_sessions_or_into_a_missing_name_is_refused() {
    let mut scratch = Scratch::new();
    scratch.append("s1.jsonl", &Line::default().json());
    scratch.ingest();

    let too_deep = query(
        &scratch.store,
        Range::All,
        GroupBy::Branch,
        &["main", "s1"],
        DAY,
    );
    let missing = query(&scratch.store, Range::All, GroupBy::Repo, &["nope"], DAY);

    assert!(
        matches!(
            too_deep,
            Err(data::Error::TooDeep {
                group_by: GroupBy::Branch
            })
        ),
        "{too_deep:?}"
    );
    assert!(
        matches!(
            &missing,
            Err(data::Error::NotFound { level: Level::Repo, name }) if name == "nope"
        ),
        "{missing:?}"
    );
}

#[test]
fn drilling_into_the_unknown_folder_finds_requests_without_a_cwd() {
    let mut scratch = Scratch::new();
    let line = serde_json::json!({
        "type": "assistant",
        "sessionId": "s1",
        "requestId": "r1",
        "timestamp": TIMESTAMP,
        "gitBranch": "main",
        "message": {
            "model": "claude-opus-5-5",
            "usage": {"input_tokens": 100_000, "output_tokens": 10_000}
        }
    });
    scratch.append("s1.jsonl", &(line.to_string() + "\n"));
    scratch.ingest();

    let branches = scratch.rows(GroupBy::Repo, &["unknown folder"]);

    assert_eq!(labels(&branches), vec![("main".to_string(), 1)]);
}

#[test]
fn drilling_into_a_model_with_only_not_in_transcript_spend_finds_its_session() {
    let mut scratch = Scratch::new();
    let cost_state = serde_json::json!({
        "type": "cost-state",
        "sessionId": "s1",
        "totalCostUSD": 0.85,
        "modelUsage": {
            "claude-opus-5-5": {
                "inputTokens": 100_000, "outputTokens": 10_000, "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.6
            },
            "claude-unpriced": {
                "inputTokens": 2_000, "outputTokens": 300, "cacheReadInputTokens": 0,
                "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.25
            }
        }
    });
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state.to_string() + "\n"),
    );
    scratch.ingest();

    let sessions = scratch.rows(GroupBy::Model, &["claude-unpriced"]);

    let rows: Vec<_> = sessions
        .iter()
        .map(|row| (row.key.clone(), row.not_in_transcript, row.input_tokens))
        .collect();
    assert_eq!(rows, vec![(Some("s1".to_string()), true, 2_000)]);
}

#[test]
fn a_cost_state_recording_no_cost_keeps_table_prices() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &(Line::default().json() + &cost_state("s1", 100_000, 10_000, 0.0)),
    );

    scratch.ingest();

    let rows = scratch.rows(GroupBy::Model, &[]);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_cost(rows[0].cost_usd, 0.6);
    let sessions = scratch.rows(GroupBy::Model, &["claude-opus-5-5"]);
    assert_eq!(sessions[0].status, Some(CostStatus::Estimated));
}
