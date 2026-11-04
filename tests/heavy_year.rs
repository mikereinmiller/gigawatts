use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gigawatts::accounts::Attribution;
use gigawatts::config::epoch_ms_from_text;
use gigawatts::contract::{GroupBy, ProblemKind, Range};
use gigawatts::data::query;
use gigawatts::pricing::PriceTable;
use gigawatts::problems;
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::git::Resolver;
use gigawatts::sources::transcript::{
    CostState, Line, ModelCost, Request, RequestDetail, SessionTotals, Tokens,
};
use gigawatts::store::{FilePosition, Store};

const HEAVY_REQUESTS_PER_DAY: usize = 5_000;
const SESSIONS_PER_DAY: usize = 50;
const PRICE_GAP_EVERY: usize = 10;
const UNPRICED_EVERY: usize = 25;
const UNPRICED_DAYS: usize = 30;
const UNPRICED_REQUEST_MODEL: &str = "claude-invented-8";
const RETENTION_DAYS: usize = 90;
const YEAR_DAYS: usize = 365;
const LAST_DAY: &str = "2026-12-31";
const REPOS: [&str; 5] = ["gigawatts", "dotfiles", "services", "people", "inventory"];
const SKILLS: [Option<&str>; 4] = [None, Some("implement"), Some("review"), Some("debugging")];
const TOOLS: [&str; 5] = [
    "Read",
    "Bash",
    "Grep",
    "Edit",
    "mcp__claude_ai_Linear__get_issue",
];
const MODELS: [&str; 3] = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
const COMBINATIONS: usize = REPOS.len() * SKILLS.len() * MODELS.len();
const GROUPINGS: [GroupBy; 5] = [
    GroupBy::Repo,
    GroupBy::Branch,
    GroupBy::Skill,
    GroupBy::Agent,
    GroupBy::Model,
];

const MAX_BYTES: u64 = 110 * 1024 * 1024;
const MAX_TODAY: Duration = Duration::from_millis(5);
const MAX_YEAR: Duration = Duration::from_millis(100);

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn base62(&mut self, len: usize) -> String {
        const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        (0..len)
            .map(|_| DIGITS[self.below(62) as usize] as char)
            .collect()
    }
}

fn session_uuid(day: usize, session: usize) -> String {
    format!("{day:08x}-0000-4000-8000-{session:012x}")
}

fn request(random: &mut Random, date: &str, day: usize, n: usize, combination: usize) -> Request {
    let repo = REPOS[combination % REPOS.len()];
    let skill = SKILLS[combination / REPOS.len() % SKILLS.len()];
    let model = MODELS[combination / (REPOS.len() * SKILLS.len())];
    let sidechain = random.below(10) < 3;
    let seconds = n * 86_399 / HEAVY_REQUESTS_PER_DAY.max(n + 1);
    let output = random.below(2_000);
    Request {
        session_id: session_uuid(day, n % SESSIONS_PER_DAY),
        request_id: format!("req_011C{}", random.base62(20)),
        model: model.to_string(),
        tokens: Tokens {
            input: random.below(50),
            output,
            cache_read: random.below(200_000),
            cache_write_5m: 0,
            cache_write_1h: random.below(20_000),
            web_searches: 0,
        },
        inference_geo: Some("not_available".to_string()),
        speed: Some("standard".to_string()),
        detail: RequestDetail {
            timestamp: Some(format!(
                "{date}T{:02}:{:02}:{:02}.{:03}Z",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60,
                random.below(1000)
            )),
            cwd: Some(format!("/Users/someone/Workspace/{repo}")),
            git_branch: Some(format!("ab/branch-{}", n % SESSIONS_PER_DAY)),
            requested_model: Some(model.to_string()),
            advisor_model: Some("claude-opus-5-5".to_string()),
            effort: Some("medium".to_string()),
            per_turn_effort: Some("medium".to_string()),
            service_tier: Some("standard".to_string()),
            stop_reason: Some("tool_use".to_string()),
            entrypoint: Some("cli".to_string()),
            version: Some("2.1.316".to_string()),
            user_type: Some("external".to_string()),
            is_sidechain: sidechain,
            agent_id: sidechain.then(|| format!("a{}", random.base62(16))),
            attribution_skill: skill.map(str::to_string),
            attribution_agent: sidechain.then(|| "Explore".to_string()),
            thinking_duration_ms: Some(random.below(5_000)),
            thinking_tokens: output / 4,
            web_fetches: 0,
            tools: (random.below(10) < 7)
                .then(|| TOOLS[random.below(TOOLS.len() as u64) as usize].to_string())
                .into_iter()
                .collect(),
        },
    }
}

fn day_lines(
    random: &mut Random,
    table: &PriceTable,
    date: &str,
    day: usize,
    heavy: bool,
) -> Vec<Line> {
    let count = if heavy {
        HEAVY_REQUESTS_PER_DAY
    } else {
        COMBINATIONS
    };
    let unpriced_days = day >= YEAR_DAYS - UNPRICED_DAYS;
    let requests: Vec<Request> = (0..count)
        .map(|n| {
            let mut request = request(random, date, day, n, n % COMBINATIONS);
            if unpriced_days && n % UNPRICED_EVERY == 0 {
                request.model = UNPRICED_REQUEST_MODEL.to_string();
            }
            request
        })
        .collect();
    let cost_states = cost_states(table, &requests);
    let unrecognized = Line::Unrecognized {
        kind: "assistant-v2".to_string(),
        timestamp: Some(format!("{date}T00:00:00.000Z")),
    };
    requests
        .into_iter()
        .map(|request| Line::Request(Box::new(request)))
        .chain(cost_states.into_iter().map(Line::CostState))
        .chain(std::iter::once(unrecognized))
        .collect()
}

fn cost_states(table: &PriceTable, requests: &[Request]) -> Vec<CostState> {
    let mut sessions: BTreeMap<&str, CostState> = BTreeMap::new();
    for request in requests {
        let state = sessions
            .entry(&request.session_id)
            .or_insert_with(|| CostState {
                session_id: request.session_id.clone(),
                total_cost_usd: 0.0,
                models: BTreeMap::new(),
                totals: SessionTotals {
                    start_time_ms: request
                        .detail
                        .timestamp
                        .as_deref()
                        .and_then(epoch_ms_from_text),
                    ..SessionTotals::default()
                },
            });
        let model = state
            .models
            .entry(request.model.clone())
            .or_insert_with(|| ModelCost {
                tokens: Tokens::default(),
                has_cache_write_split: true,
                cost_usd: 0.0,
                thinking_tokens: 0,
                web_fetches: 0,
            });
        let tokens = &mut model.tokens;
        tokens.input += request.tokens.input;
        tokens.output += request.tokens.output;
        tokens.cache_read += request.tokens.cache_read;
        tokens.cache_write_5m += request.tokens.cache_write_5m;
        tokens.cache_write_1h += request.tokens.cache_write_1h;
        tokens.web_searches += request.tokens.web_searches;
        model.thinking_tokens += request.detail.thinking_tokens;
        model.cost_usd += table.price(request).unwrap_or(0.01);
    }
    sessions
        .into_values()
        .enumerate()
        .map(|(index, mut state)| {
            if index % PRICE_GAP_EVERY == 0 {
                for model in state.models.values_mut() {
                    model.cost_usd *= 1.1;
                }
            }
            if index % UNPRICED_EVERY == 1 {
                state.models.insert(
                    "claude-invented-9".to_string(),
                    ModelCost {
                        tokens: Tokens {
                            input: 1_000,
                            output: 100,
                            ..Tokens::default()
                        },
                        has_cache_write_split: true,
                        cost_usd: 0.01,
                        thinking_tokens: 0,
                        web_fetches: 0,
                    },
                );
            }
            state.total_cost_usd = state.models.values().map(|model| model.cost_usd).sum();
            state
        })
        .collect()
}

fn shift(date: &str, days: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT date(?1, ?2)",
            rusqlite::params![date, format!("{days} days")],
            |row| row.get(0),
        )
        .unwrap()
}

fn database_bytes(path: &std::path::Path) -> u64 {
    let wal = path.with_extension("sqlite3-wal");
    std::fs::metadata(path).unwrap().len() + std::fs::metadata(wal).map_or(0, |m| m.len())
}

fn fastest_of_three(mut run: impl FnMut()) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn a_heavy_year_with_90_day_retention_stays_small_and_fast() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let account = Account {
        account_uuid: "00000000-0000-4000-8000-000000000001".to_string(),
        organization_uuid: "00000000-0000-4000-8000-000000000002".to_string(),
    };
    let attribution = Attribution::single(Some(account));
    let table = PriceTable::built_in();
    let first_day = shift(LAST_DAY, -(YEAR_DAYS as i64 - 1));
    let mut random = Random(7);

    for day in 0..YEAR_DAYS {
        let date = shift(&first_day, day as i64);
        let heavy = day >= YEAR_DAYS - RETENTION_DAYS;
        let lines = day_lines(&mut random, &table, &date, day, heavy);
        let position = FilePosition {
            path: format!("day-{day}.jsonl"),
            device: 1,
            inode: day as u64,
            first_line_hash: Some(day as u64),
            offset: 1,
        };
        store
            .save_file(
                &position,
                true,
                &lines,
                &attribution,
                &table,
                &Resolver::default(),
            )
            .unwrap();
    }
    let reports = |store: &Store| {
        GROUPINGS.map(|by| query(store, Range::All, by, &[], LAST_DAY).unwrap().rows)
    };
    let branch_and_its_sessions = |store: &Store| {
        let branches = query(store, Range::All, GroupBy::Repo, &[REPOS[0]], LAST_DAY)
            .unwrap()
            .rows;
        let branch = branches
            .iter()
            .find(|row| row.key.as_deref() == Some("ab/branch-0"))
            .map(|row| (row.requests, row.cost_usd.unwrap_or_default()))
            .unwrap();
        let sessions = query(
            store,
            Range::All,
            GroupBy::Repo,
            &[REPOS[0], "ab/branch-0"],
            LAST_DAY,
        )
        .unwrap()
        .rows;
        let summed = (
            sessions.iter().map(|row| row.requests).sum::<u64>(),
            sessions.iter().filter_map(|row| row.cost_usd).sum::<f64>(),
        );
        (branch, summed)
    };
    let before_cleanup = reports(&store);
    let sessions_before_cleanup = branch_and_its_sessions(&store);
    let stored_before_cleanup = store.request_count().unwrap();
    let bytes_before_cleanup = database_bytes(&path);

    let expired = store
        .expire_requests(RETENTION_DAYS as u32, LAST_DAY)
        .unwrap();
    drop(store);
    let free_pages: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap();
    let store = Store::open(&path).unwrap();

    let bytes = database_bytes(&path);
    assert_eq!(free_pages, 0);
    let requests = store.request_count().unwrap();
    assert!(expired > 0);
    assert_eq!(expired, stored_before_cleanup - requests);
    assert!(
        bytes < bytes_before_cleanup,
        "cleanup left the database at {bytes} bytes, from {bytes_before_cleanup}"
    );
    assert!(reports(&store) == before_cleanup);
    let ((branch_requests, branch_cost), (session_requests, session_cost)) =
        branch_and_its_sessions(&store);
    assert_eq!(session_requests, branch_requests);
    assert!((session_cost - branch_cost).abs() < 1e-6);
    assert_eq!(sessions_before_cleanup.0, (branch_requests, branch_cost));
    let today = fastest_of_three(|| {
        query(&store, Range::Today, GroupBy::Model, &[], LAST_DAY).unwrap();
    });
    let kinds: Vec<ProblemKind> = problems::collect(&store, &table)
        .unwrap()
        .iter()
        .map(|problem| problem.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            ProblemKind::UnrecognizedLines,
            ProblemKind::NoPrice,
            ProblemKind::NoPrice,
            ProblemKind::PriceGap,
            ProblemKind::PriceGap,
            ProblemKind::PriceGap,
        ]
    );
    let problems = fastest_of_three(|| {
        problems::collect(&store, &table).unwrap();
    });
    let year_by = |by: GroupBy| {
        fastest_of_three(|| {
            query(&store, Range::All, by, &[], LAST_DAY).unwrap();
        })
    };
    let year = year_by(GroupBy::Model);
    let year_by_repo = year_by(GroupBy::Repo);
    let year_by_branch = year_by(GroupBy::Branch);
    let year_drill = fastest_of_three(|| {
        query(
            &store,
            Range::All,
            GroupBy::Repo,
            &[REPOS[0], "ab/branch-0"],
            LAST_DAY,
        )
        .unwrap();
    });

    println!(
        "{:.1} MB, {} bytes per stored request, today {today:?}, year {year:?}, \
         by repo {year_by_repo:?}, by branch {year_by_branch:?}, drill {year_drill:?}, \
         problems {problems:?}",
        bytes as f64 / 1024.0 / 1024.0,
        bytes / requests
    );
    assert!(bytes <= MAX_BYTES, "database is {bytes} bytes");
    assert!(today <= MAX_TODAY, "today's query took {today:?}");
    assert!(
        today + problems <= MAX_TODAY,
        "today's query took {today:?} and collecting problems took {problems:?}"
    );
    assert!(year <= MAX_YEAR, "the yearly report took {year:?}");
    assert!(
        year_by_repo <= MAX_YEAR,
        "the yearly report by repo took {year_by_repo:?}"
    );
    assert!(
        year_by_branch <= MAX_YEAR,
        "the yearly report by branch took {year_by_branch:?}"
    );
    assert!(
        year_drill <= MAX_YEAR,
        "the drill to sessions took {year_drill:?}"
    );
}
