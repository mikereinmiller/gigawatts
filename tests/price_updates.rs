use std::cell::Cell;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;

use gigawatts::accounts::Attribution;
use gigawatts::config::Config;
use gigawatts::contract::{Problem, ProblemKind};
use gigawatts::ingest::ingest_tree;
use gigawatts::price_updates::{self, Download};
use gigawatts::pricing::PriceTable;
use gigawatts::problems;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

const TIMESTAMP: &str = "2026-01-08T12:00:00.000Z";
const NOW_MS: i64 = 1_767_873_600_000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const OPUS_5_5_ROW: &str = "| Claude Opus 5.5                                                                                                                                                        | $4 / MTok             | $5 / MTok       | $8 / MTok       | $0.20 / MTok<sup>2</sup> | $20 / MTok             |";

fn saved_page() -> String {
    fs::read_to_string("tests/fixtures/pricing.md").unwrap()
}

fn page_with_opus_5_5_output(price: &str) -> String {
    let page = saved_page();
    assert!(page.contains(OPUS_5_5_ROW), "the saved page changed");
    page.replace(
        OPUS_5_5_ROW,
        &OPUS_5_5_ROW.replace("| $20 / MTok   ", &format!("| ${price} / MTok   ")),
    )
}

struct Page {
    answer: Result<String, String>,
    requests: Cell<u32>,
}

impl Page {
    fn new(text: String) -> Page {
        Page {
            answer: Ok(text),
            requests: Cell::new(0),
        }
    }

    fn failing(error: &str) -> Page {
        Page {
            answer: Err(error.to_string()),
            requests: Cell::new(0),
        }
    }
}

impl Download for Page {
    fn get(&self, url: &str) -> Result<String, String> {
        assert_eq!(
            url,
            "https://platform.claude.com/docs/en/about-claude/pricing.md"
        );
        self.requests.set(self.requests.get() + 1);
        self.answer.clone()
    }
}

fn request(session: &str, request: &str, model: &str, input: u64, output: u64) -> String {
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

fn cost_state(session: &str, model: &str, input: u64, output: u64, cost_usd: f64) -> String {
    serde_json::json!({
        "type": "cost-state",
        "sessionId": session,
        "totalCostUSD": cost_usd,
        "modelUsage": {
            model: {
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
    _dir: tempfile::TempDir,
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
            _dir: dir,
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
            &Resolver,
        )
        .unwrap();
    }

    fn with_opus_5_5_charged_25_per_million_output() -> Scratch {
        let mut scratch = Scratch::new();
        scratch.append(
            "s1.jsonl",
            &(request("s1", "r1", "claude-opus-5-5", 1_000_000, 100_000)
                + &cost_state("s1", "claude-opus-5-5", 1_000_000, 100_000, 6.5)),
        );
        scratch.append(
            "s2.jsonl",
            &request("s2", "r2", "claude-opus-5-5", 1_000_000, 100_000),
        );
        scratch.ingest();
        assert_eq!(kinds(&scratch.problems()), vec![ProblemKind::PriceGap]);
        scratch
    }

    fn update(&mut self, config: &Config, page: &Page, now_ms: i64) {
        price_updates::update_if_needed(&mut self.store, config, page, now_ms).unwrap();
    }

    fn problems(&self) -> Vec<Problem> {
        let table = self.store.price_table(&Config::default()).unwrap();
        problems::collect(&self.store, &table).unwrap()
    }

    fn total_cost(&self) -> f64 {
        self.store.totals_by_model(None).unwrap()[0]
            .cost_usd
            .unwrap()
    }
}

fn kinds(problems: &[Problem]) -> Vec<ProblemKind> {
    problems.iter().map(|problem| problem.kind).collect()
}

fn assert_cost(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn a_changed_price_on_the_page_reprices_that_models_history() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    assert_cost(scratch.total_cost(), 12.5);
    let page = Page::new(page_with_opus_5_5_output("25"));

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_eq!(page.requests.get(), 1);
    assert_cost(scratch.total_cost(), 13.0);
    assert_eq!(scratch.problems(), vec![]);
}

fn price_update_problem(scratch: &Scratch) -> Problem {
    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::PriceGap, ProblemKind::PriceUpdateFailing],
        "{problems:?}"
    );
    problems[1].clone()
}

#[test]
fn an_unparseable_page_keeps_the_current_prices_and_raises_a_problem() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let page = Page::new("# Pricing\n\nPrices moved to a new page.\n".to_string());

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_cost(scratch.total_cost(), 12.5);
    let problem = price_update_problem(&scratch);
    assert_eq!(
        problem.cause,
        "https://platform.claude.com/docs/en/about-claude/pricing.md doesn't parse: it has no table with the columns Model, Base input tokens, 5m cache writes, 1h cache writes, Cache hits and refreshes, Output tokens"
    );
    assert!(
        problem.fix.contains("`price_updates = false`"),
        "{}",
        problem.fix
    );
    assert_eq!(problem.since, scratch.store.local_datetime(NOW_MS).unwrap());
}

#[test]
fn a_price_ten_times_the_current_one_keeps_the_current_prices_and_raises_a_problem() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let page = Page::new(page_with_opus_5_5_output("200"));

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_cost(scratch.total_cost(), 12.5);
    assert_eq!(
        price_update_problem(&scratch).cause,
        "https://platform.claude.com/docs/en/about-claude/pricing.md lists claude-opus-5-5's output price as $200 per million tokens, against $20 now, which looks wrong, so Gigawatts kept its current prices"
    );
}

#[test]
fn a_failed_download_keeps_the_current_prices_and_raises_a_problem() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let page = Page::failing("curl: (6) Could not resolve host");

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_cost(scratch.total_cost(), 12.5);
    assert_eq!(
        price_update_problem(&scratch).cause,
        "Gigawatts can't download https://platform.claude.com/docs/en/about-claude/pricing.md: curl: (6) Could not resolve host"
    );
}

#[test]
fn a_failure_keeps_its_first_time_and_clears_once_a_download_works() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    scratch.update(&Config::default(), &Page::failing("offline"), NOW_MS);
    scratch.update(
        &Config::default(),
        &Page::failing("offline"),
        NOW_MS + DAY_MS,
    );

    assert_eq!(
        price_update_problem(&scratch).since,
        scratch.store.local_datetime(NOW_MS).unwrap()
    );

    let page = Page::new(page_with_opus_5_5_output("25"));
    scratch.update(&Config::default(), &page, NOW_MS + 2 * DAY_MS);

    assert_eq!(scratch.problems(), vec![]);
}

#[test]
fn without_a_price_problem_nothing_is_downloaded() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &request("s1", "r1", "claude-opus-5-5", 1_000_000, 100_000),
    );
    scratch.ingest();
    let page = Page::new(saved_page());

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_eq!(page.requests.get(), 0);
}

#[test]
fn with_price_updates_off_nothing_is_downloaded_and_an_earlier_failure_clears() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    scratch.update(&Config::default(), &Page::failing("offline"), NOW_MS);
    let off = Config::parse("price_updates = false\n", scratch.projects.as_path()).unwrap();
    let page = Page::new(page_with_opus_5_5_output("25"));

    scratch.update(&off, &page, NOW_MS + 2 * DAY_MS);

    assert_eq!(page.requests.get(), 0);
    assert_eq!(kinds(&scratch.problems()), vec![ProblemKind::PriceGap]);
}

#[test]
fn a_price_problem_downloads_at_most_once_a_day() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let page = Page::failing("offline");

    scratch.update(&Config::default(), &page, NOW_MS);
    scratch.update(&Config::default(), &page, NOW_MS + DAY_MS - 1);
    assert_eq!(page.requests.get(), 1);

    scratch.update(&Config::default(), &page, NOW_MS + DAY_MS);
    assert_eq!(page.requests.get(), 2);
}

#[test]
fn a_model_new_to_the_page_gets_its_price() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &request("s1", "r1", "claude-opus-5-6", 1_000_000, 100_000),
    );
    scratch.ingest();
    assert_eq!(kinds(&scratch.problems()), vec![ProblemKind::NoPrice]);
    let new_row = OPUS_5_5_ROW
        .replace("Claude Opus 5.5", "Claude Opus 5.6")
        .replace("| $20 / MTok   ", "| $30 / MTok   ");
    let page = Page::new(saved_page().replace(OPUS_5_5_ROW, &format!("{new_row}\n{OPUS_5_5_ROW}")));

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_cost(scratch.total_cost(), 7.0);
    assert_eq!(scratch.problems(), vec![]);
}

#[test]
fn the_saved_page_matches_the_built_in_table_so_nothing_is_kept_from_it() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let page = Page::new(saved_page());

    scratch.update(&Config::default(), &page, NOW_MS);

    let downloaded = scratch.store.downloaded_prices().unwrap().unwrap();
    assert_eq!(downloaded.fetched_at_ms, NOW_MS);
    assert_eq!(downloaded.models, Default::default());
    assert_eq!(kinds(&scratch.problems()), vec![ProblemKind::PriceGap]);
}

#[test]
fn the_page_parses_every_model_and_haikus_long_prompt_price() {
    let models = price_updates::parse_page(&saved_page()).unwrap();
    let table = PriceTable::built_in();

    assert_eq!(models.len(), 21);
    for (model, prices) in &models {
        let built_in = table.model(model).unwrap_or_else(|| panic!("{model}"));
        assert_eq!(prices.standard, built_in.standard, "{model}");
        assert_eq!(prices.long_prompt, built_in.long_prompt, "{model}");
        assert_eq!(prices.fast, built_in.fast, "{model}");
    }
}

#[test]
fn a_model_new_to_the_page_gets_its_fast_mode_price() {
    let mut scratch = Scratch::new();
    let fast_request = request("s1", "r1", "claude-opus-5-6", 1_000_000, 100_000).replace(
        r#""output_tokens":100000"#,
        r#""output_tokens":100000,"speed":"fast""#,
    );
    assert!(fast_request.contains(r#""speed":"fast""#));
    scratch.append("s1.jsonl", &fast_request);
    scratch.ingest();
    let new_row = OPUS_5_5_ROW
        .replace("Claude Opus 5.5", "Claude Opus 5.6")
        .replace("| $20 / MTok   ", "| $30 / MTok   ");
    let fast_row = "| Claude Opus 5.5                 | $8 USD / MTok  | $40 USD / MTok |";
    let page = saved_page()
        .replace(OPUS_5_5_ROW, &format!("{new_row}\n{OPUS_5_5_ROW}"))
        .replace(
            fast_row,
            &format!(
                "{fast_row}\n| Claude Opus 5.6                 | $12 USD / MTok | $60 USD / MTok |"
            ),
        );
    assert!(
        page.contains("$12 USD"),
        "the saved page's fast table changed"
    );

    scratch.update(&Config::default(), &Page::new(page), NOW_MS);

    assert_cost(scratch.total_cost(), 18.0);
    assert_eq!(scratch.problems(), vec![]);
}

#[test]
fn an_override_counts_as_the_current_price_when_checking_the_page() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let config = Config::parse(
        "[prices.claude-opus-5-5]\ninput = 4.0\ncache_write_5m = 5.0\ncache_write_1h = 8.0\ncache_read = 0.2\noutput = 150.0\n",
        scratch.projects.as_path(),
    )
    .unwrap();
    let page = Page::new(page_with_opus_5_5_output("200"));

    scratch.update(&config, &page, NOW_MS);

    let downloaded = scratch.store.downloaded_prices().unwrap().unwrap();
    assert_eq!(downloaded.models["claude-opus-5-5"].standard.output, 200.0);
}

#[test]
fn a_price_under_a_fifth_of_the_current_one_keeps_the_current_prices() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let page = Page::new(page_with_opus_5_5_output("3"));

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_cost(scratch.total_cost(), 12.5);
    assert_eq!(
        price_update_problem(&scratch).cause,
        "https://platform.claude.com/docs/en/about-claude/pricing.md lists claude-opus-5-5's output price as $3 per million tokens, against $20 now, which looks wrong, so Gigawatts kept its current prices"
    );
}

#[test]
fn a_new_model_over_five_times_the_most_expensive_price_keeps_it_unpriced() {
    let mut scratch = Scratch::new();
    scratch.append(
        "s1.jsonl",
        &request("s1", "r1", "claude-opus-5-6", 1_000_000, 100_000),
    );
    scratch.ingest();
    let new_row = OPUS_5_5_ROW
        .replace("Claude Opus 5.5", "Claude Opus 5.6")
        .replace("| $20 / MTok   ", "| $400 / MTok  ");
    let page = Page::new(saved_page().replace(OPUS_5_5_ROW, &format!("{new_row}\n{OPUS_5_5_ROW}")));

    scratch.update(&Config::default(), &page, NOW_MS);

    let problems = scratch.problems();
    assert_eq!(
        kinds(&problems),
        vec![ProblemKind::NoPrice, ProblemKind::PriceUpdateFailing]
    );
    assert_eq!(
        problems[1].cause,
        "https://platform.claude.com/docs/en/about-claude/pricing.md lists claude-opus-5-6's output price as $400 per million tokens, against $75 for the most expensive model, which looks wrong, so Gigawatts kept its current prices"
    );
}

#[test]
fn a_pricing_rule_gigawatts_doesnt_know_makes_the_page_unparseable() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    let long_row = page_with_opus_5_5_output("25")
        .lines()
        .find(|line| line.starts_with("| Claude Opus 5.5 "))
        .unwrap()
        .replacen(
            "Claude Opus 5.5 ",
            "Claude Opus 5.5 (for prompts over 200,000 tokens)",
            1,
        );
    let page =
        Page::new(saved_page().replace(OPUS_5_5_ROW, &format!("{OPUS_5_5_ROW}\n{long_row}")));

    scratch.update(&Config::default(), &page, NOW_MS);

    assert_cost(scratch.total_cost(), 12.5);
    assert!(
        price_update_problem(&scratch)
            .cause
            .ends_with("doesn't parse: Claude Opus 5.5 (for prompts over 200,000 tokens) has a pricing rule Gigawatts doesn't know"),
        "{:?}",
        price_update_problem(&scratch).cause
    );
}

#[test]
fn a_failure_after_the_price_problems_cleared_starts_its_own_time() {
    let mut scratch = Scratch::with_opus_5_5_charged_25_per_million_output();
    scratch.update(&Config::default(), &Page::failing("offline"), NOW_MS);
    let overridden = Config::parse(
        "[prices.claude-opus-5-5]\ninput = 4.0\ncache_write_5m = 5.0\ncache_write_1h = 8.0\ncache_read = 0.2\noutput = 25.0\n",
        scratch.projects.as_path(),
    )
    .unwrap();
    let reprice = |scratch: &mut Scratch, config: &Config| {
        let table = scratch.store.price_table(config).unwrap();
        scratch.store.reprice_if_changed(&table).unwrap();
    };
    reprice(&mut scratch, &overridden);
    scratch.update(&overridden, &Page::failing("offline"), NOW_MS + 2 * DAY_MS);
    reprice(&mut scratch, &Config::default());

    scratch.update(
        &Config::default(),
        &Page::failing("offline"),
        NOW_MS + 4 * DAY_MS,
    );

    assert_eq!(
        price_update_problem(&scratch).since,
        scratch.store.local_datetime(NOW_MS + 4 * DAY_MS).unwrap()
    );
}
