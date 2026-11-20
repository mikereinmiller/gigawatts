use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use gigawatts::accounts::Attribution;
use gigawatts::config::Config;
use gigawatts::contract::{Limit, LimitSource};
use gigawatts::engine::{self, Paths};
use gigawatts::pricing::PriceTable;
use gigawatts::sources::claude_json::{Account, Login};
use gigawatts::sources::git::Resolver;
use gigawatts::sources::transcript::{
    CostState, Line, ModelCost, Request, RequestDetail, SessionTotals, Tokens,
};
use gigawatts::store::{FilePosition, LimitsReading, Store};
use gigawatts::tui::{App, palette::Rgb};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

const USAGE: &str = "usage:
  demo seed <home>                            write a demo history into <home>
  demo tui <home> <columns> <rows> <out.html>  render the terminal view as HTML
  demo report <home> <port>                    serve the report page until stopped";

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const SESSION_WINDOW_MS: i64 = 5 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;
const HISTORY_DAYS: i64 = 75;
const READING_EVERY_MS: i64 = 10 * MINUTE_MS;
const READINGS_FOR_DAYS: i64 = 28;

const OPUS: &str = "claude-opus-5-5";
const FABLE: &str = "claude-fable-5-2";
const SONNET: &str = "claude-sonnet-5-5";
const HAIKU: &str = "claude-haiku-5-5";

const REPOS: [(&str, &[&str]); 5] = [
    (
        "orchard",
        &[
            "main",
            "feat/csv-export",
            "fix/checkout-flake",
            "feat/gift-cards",
        ],
    ),
    (
        "lumen-api",
        &["main", "feat/rate-limits", "fix/webhook-retries"],
    ),
    (
        "tidepool",
        &["main", "perf/search-index", "feat/saved-filters"],
    ),
    ("field-notes", &["main", "docs/getting-started"]),
    ("gigawatts", &["main"]),
];
const SKILLS: [Option<&str>; 6] = [
    None,
    None,
    Some("code-review"),
    Some("write-tests"),
    Some("simplify"),
    Some("release-notes"),
];
const TITLES: [&str; 12] = [
    "Add pagination to the orders list",
    "Track down the slow dashboard query",
    "Upgrade the payments SDK",
    "Clean up the feature flag checks",
    "Add retries to the webhook sender",
    "Write tests for the coupon rules",
    "Move image uploads to background jobs",
    "Fix the timezone bug in reports",
    "Draft the API changelog",
    "Split the settings page into tabs",
    "Add an index for the audit log",
    "Review the onboarding emails PR",
];

struct Today {
    title: &'static str,
    repo: &'static str,
    branch: &'static str,
    skill: Option<&'static str>,
    model: &'static str,
    started_minutes_ago: i64,
    ended_minutes_ago: i64,
    requests: usize,
}

const TODAY: [Today; 5] = [
    Today {
        title: "Add CSV export to the orders page",
        repo: "orchard",
        branch: "feat/csv-export",
        skill: None,
        model: OPUS,
        started_minutes_ago: 95,
        ended_minutes_ago: 1,
        requests: 85,
    },
    Today {
        title: "Speed up the search index rebuild",
        repo: "tidepool",
        branch: "perf/search-index",
        skill: None,
        model: FABLE,
        started_minutes_ago: 50,
        ended_minutes_ago: 4,
        requests: 40,
    },
    Today {
        title: "Review the rate limit PR",
        repo: "lumen-api",
        branch: "feat/rate-limits",
        skill: Some("code-review"),
        model: OPUS,
        started_minutes_ago: 70,
        ended_minutes_ago: 6,
        requests: 34,
    },
    Today {
        title: "Fix the flaky checkout test",
        repo: "orchard",
        branch: "fix/checkout-flake",
        skill: Some("write-tests"),
        model: OPUS,
        started_minutes_ago: 85,
        ended_minutes_ago: 9,
        requests: 50,
    },
    Today {
        title: "Write release notes for 2.4",
        repo: "field-notes",
        branch: "main",
        skill: Some("release-notes"),
        model: SONNET,
        started_minutes_ago: 230,
        ended_minutes_ago: 205,
        requests: 22,
    },
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let done = match args.as_slice() {
        ["seed", home] => seed(Path::new(home)),
        ["tui", home, columns, rows, out] => match (columns.parse(), rows.parse()) {
            (Ok(columns), Ok(rows)) => render_tui(Path::new(home), columns, rows, Path::new(out)),
            _ => Err("columns and rows must be numbers".to_string()),
        },
        ["report", home, port] => match port.parse() {
            Ok(port) => serve_report(Path::new(home), port),
            Err(_) => Err("port must be a number".to_string()),
        },
        _ => Err(USAGE.to_string()),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn account() -> Account {
    Account {
        account_uuid: "00000000-0000-4000-8000-0000000000d1".to_string(),
        organization_uuid: "00000000-0000-4000-8000-0000000000d2".to_string(),
    }
}

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

    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

struct Session {
    id: String,
    title: String,
    repo: &'static str,
    branch: &'static str,
    skill: Option<&'static str>,
    model: &'static str,
    start_ms: i64,
    end_ms: i64,
    requests: usize,
}

fn seed(home: &Path) -> Result<(), String> {
    let paths = Paths::new(home);
    let now_ms = now_ms();
    refuse_existing_history(&paths)?;
    refuse_until_todays_sessions_fit(now_ms)?;
    let mut store = Store::open(&paths.database()).map_err(|error| error.to_string())?;
    let config_dir = home.join(".claude");
    store
        .record_login(
            &config_dir.to_string_lossy(),
            &Login {
                account: account(),
                email: Some("you@example.com".to_string()),
            },
            now_ms - HISTORY_DAYS * DAY_MS,
        )
        .map_err(|error| error.to_string())?;
    write_config(&paths)?;

    let table = PriceTable::built_in();
    let mut random = Random(1_21);
    let sessions = earlier_sessions(&mut random, &store, now_ms)?
        .into_iter()
        .chain(todays_sessions(now_ms))
        .collect::<Vec<_>>();
    let attribution = Attribution::single(Some(account()));
    let mut spent: Vec<(i64, f64, bool)> = Vec::new();
    for (index, session) in sessions.iter().enumerate() {
        let requests = requests_for(&mut random, session);
        for request in &requests {
            let at_ms = parse_ms(request.detail.timestamp.as_deref().unwrap_or_default());
            let usd = table.price(request).unwrap_or(0.0);
            spent.push((at_ms, usd, request.model == FABLE));
        }
        let mut lines = vec![Line::Title {
            session_id: session.id.clone(),
            title: session.title.clone(),
        }];
        lines.push(Line::CostState(cost_state(&table, &requests)));
        lines.splice(
            1..1,
            requests.into_iter().map(|r| Line::Request(Box::new(r))),
        );
        let position = FilePosition {
            path: config_dir
                .join("projects")
                .join(format!("-Users-you-Code-{}", session.repo))
                .join(format!("{}.jsonl", session.id))
                .to_string_lossy()
                .into_owned(),
            device: 1,
            inode: index as u64 + 1,
            first_line_hash: Some(index as u64 + 1),
            offset: 1,
        };
        store
            .save_file(&position, true, &lines, &attribution, &table, &Resolver)
            .map_err(|error| error.to_string())?;
    }
    spent.sort_by_key(|(at_ms, ..)| *at_ms);
    record_limits(&mut store, &spent, now_ms)?;
    println!(
        "wrote {} sessions and {} requests to {}",
        sessions.len(),
        spent.len(),
        paths.database().display()
    );
    Ok(())
}

fn refuse_existing_history(paths: &Paths) -> Result<(), String> {
    for path in [paths.database(), paths.config()] {
        if path.exists() {
            return Err(format!(
                "{} already exists: seed only writes into a home with no Gigawatts history or config",
                path.display()
            ));
        }
    }
    Ok(())
}

fn refuse_until_todays_sessions_fit(now_ms: i64) -> Result<(), String> {
    let earliest_minutes_ago = TODAY
        .iter()
        .map(|today| today.started_minutes_ago)
        .max()
        .unwrap_or(0);
    let earliest_ms = now_ms - earliest_minutes_ago * MINUTE_MS;
    if local_date(earliest_ms)? == local_date(now_ms)? {
        return Ok(());
    }
    Err(format!(
        "today's demo sessions start up to {earliest_minutes_ago} minutes ago, which is still yesterday; run seed later in the day"
    ))
}

fn write_config(paths: &Paths) -> Result<(), String> {
    let path = paths.config();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
    let text = format!(
        "report_port = 0\nprice_updates = false\n\n[accounts.\"{}\"]\nlabel = \"Personal (Max)\"\n",
        account()
    );
    std::fs::write(&path, text).map_err(|error| error.to_string())
}

fn earlier_sessions(
    random: &mut Random,
    store: &Store,
    now_ms: i64,
) -> Result<Vec<Session>, String> {
    let today_start_ms = local_midnight_ms(store, now_ms)?;
    let mut sessions = Vec::new();
    for days_ago in 1..=HISTORY_DAYS {
        let day_start_ms = today_start_ms - days_ago * DAY_MS;
        let weekday = local_weekday(day_start_ms + 12 * HOUR_MS)?;
        let weekend = weekday == 0 || weekday == 6;
        let count = if weekend {
            random.below(3)
        } else {
            random.between(4, 7)
        };
        for n in 0..count {
            let (repo, branches) = *random.pick(&REPOS);
            let start_ms =
                day_start_ms + random.between(8 * 60, 18 * 60) as i64 * MINUTE_MS + n as i64;
            let minutes = random.between(15, 110) as i64;
            sessions.push(Session {
                id: session_id(days_ago as u64, n),
                title: random.pick(&TITLES).to_string(),
                repo,
                branch: random.pick(branches),
                skill: *random.pick(&SKILLS),
                model: if random.below(6) == 0 { FABLE } else { OPUS },
                start_ms,
                end_ms: start_ms + minutes * MINUTE_MS,
                requests: (minutes as usize * random.between(13, 22) as usize) / 10,
            });
        }
    }
    Ok(sessions)
}

fn todays_sessions(now_ms: i64) -> Vec<Session> {
    TODAY
        .iter()
        .enumerate()
        .map(|(n, today)| Session {
            id: session_id(0, n as u64),
            title: today.title.to_string(),
            repo: today.repo,
            branch: today.branch,
            skill: today.skill,
            model: today.model,
            start_ms: now_ms - today.started_minutes_ago * MINUTE_MS,
            end_ms: now_ms - today.ended_minutes_ago * MINUTE_MS,
            requests: today.requests,
        })
        .collect()
}

fn session_id(day: u64, n: u64) -> String {
    format!("{day:08x}-d000-4000-8000-{n:012x}")
}

fn requests_for(random: &mut Random, session: &Session) -> Vec<Request> {
    let span_ms = session.end_ms - session.start_ms;
    (0..session.requests)
        .map(|n| {
            let at_ms = if n + 1 == session.requests {
                session.end_ms
            } else {
                session.start_ms + span_ms * n as i64 / session.requests as i64
            };
            let agent = match random.below(10) {
                0..=1 => Some(("Explore", HAIKU)),
                2 => Some(("general-purpose", SONNET)),
                3 if session.skill == Some("write-tests") => Some(("test-runner", SONNET)),
                _ => None,
            };
            let model = agent.map_or(session.model, |(_, model)| model);
            let output = random.between(150, 2_400);
            Request {
                session_id: session.id.clone(),
                request_id: format!("req_demo{:08x}{n:06x}", random.next()),
                model: model.to_string(),
                tokens: Tokens {
                    input: random.between(3, 60),
                    output,
                    cache_read: random.between(30_000, 150_000),
                    cache_write_5m: 0,
                    cache_write_1h: random.between(0, 9_000),
                    web_searches: 0,
                },
                inference_geo: Some("not_available".to_string()),
                speed: Some("standard".to_string()),
                detail: RequestDetail {
                    timestamp: Some(utc_text(at_ms)),
                    cwd: Some(format!("/Users/you/Code/{}", session.repo)),
                    git_branch: Some(session.branch.to_string()),
                    requested_model: Some(model.to_string()),
                    effort: Some("medium".to_string()),
                    stop_reason: Some("tool_use".to_string()),
                    entrypoint: Some("cli".to_string()),
                    version: Some("2.1.316".to_string()),
                    user_type: Some("external".to_string()),
                    is_sidechain: agent.is_some(),
                    agent_id: agent.map(|_| format!("a{:016x}", random.next())),
                    attribution_skill: session.skill.map(str::to_string),
                    attribution_agent: agent.map(|(name, _)| name.to_string()),
                    thinking_duration_ms: Some(random.below(4_000)),
                    thinking_tokens: output / 4,
                    tools: vec![random.pick(&["Read", "Edit", "Bash", "Grep"]).to_string()],
                    ..RequestDetail::default()
                },
            }
        })
        .collect()
}

fn cost_state(table: &PriceTable, requests: &[Request]) -> CostState {
    let mut models: BTreeMap<String, ModelCost> = BTreeMap::new();
    for request in requests {
        let model = models.entry(request.model.clone()).or_insert(ModelCost {
            tokens: Tokens::default(),
            has_cache_write_split: true,
            cost_usd: 0.0,
            thinking_tokens: 0,
            web_fetches: 0,
        });
        model.tokens.input += request.tokens.input;
        model.tokens.output += request.tokens.output;
        model.tokens.cache_read += request.tokens.cache_read;
        model.tokens.cache_write_1h += request.tokens.cache_write_1h;
        model.thinking_tokens += request.detail.thinking_tokens;
        model.cost_usd += table.price(request).unwrap_or(0.0);
    }
    CostState {
        session_id: requests[0].session_id.clone(),
        total_cost_usd: models.values().map(|model| model.cost_usd).sum(),
        models,
        totals: SessionTotals {
            start_time_ms: requests[0].detail.timestamp.as_deref().map(parse_ms),
            ..SessionTotals::default()
        },
    }
}

fn record_limits(store: &mut Store, spent: &[(i64, f64, bool)], now_ms: i64) -> Result<(), String> {
    let weekly_reset_ms = now_ms + 2 * DAY_MS + 14 * HOUR_MS - now_ms.rem_euclid(HOUR_MS);
    let week_start = |at_ms: i64| {
        weekly_reset_ms - WEEK_MS * ((weekly_reset_ms - at_ms + WEEK_MS - 1) / WEEK_MS)
    };
    let session_starts = session_window_starts(spent);
    let session_start = |at_ms: i64| {
        session_starts
            .iter()
            .rev()
            .find(|start| **start <= at_ms && at_ms < **start + SESSION_WINDOW_MS)
            .copied()
    };
    let spent_between = |from_ms: i64, to_ms: i64, fable_only: bool| -> f64 {
        spent
            .iter()
            .filter(|(at_ms, _, fable)| {
                *at_ms >= from_ms && *at_ms <= to_ms && (*fable || !fable_only)
            })
            .map(|(_, usd, _)| usd)
            .sum()
    };
    let full_weeks = (1..=3).map(|weeks_ago| {
        let start = weekly_reset_ms - (weeks_ago + 1) * WEEK_MS;
        spent_between(start, start + WEEK_MS, false)
    });
    let weekly_budget = full_weeks.fold(0.0, f64::max) / 0.88;
    let fable_budget = weekly_budget * 0.3;
    let session_budget = session_starts
        .iter()
        .map(|start| spent_between(*start, start + SESSION_WINDOW_MS, false))
        .fold(0.0, f64::max)
        / 0.82;

    let first_ms = now_ms - READINGS_FOR_DAYS * DAY_MS;
    let mut reading_times: Vec<i64> = (first_ms - first_ms.rem_euclid(READING_EVERY_MS)..now_ms)
        .step_by(READING_EVERY_MS as usize)
        .collect();
    reading_times.push(now_ms - MINUTE_MS);
    for at_ms in reading_times {
        let week_from = week_start(at_ms);
        let resets_weekly = week_from + WEEK_MS;
        let (session_percent, session_resets) = match session_start(at_ms) {
            Some(start) => (
                percent(spent_between(start, at_ms, false), session_budget),
                start + SESSION_WINDOW_MS,
            ),
            None => (0.0, at_ms + SESSION_WINDOW_MS),
        };
        let limit = |kind: &str, group: &str, scope: Option<&str>, percent: f64, resets_ms| Limit {
            kind: kind.to_string(),
            group: Some(group.to_string()),
            scope: scope.map(str::to_string),
            percent,
            severity: Some("normal".to_string()),
            resets_at: Some(utc_text(resets_ms)),
            active: true,
            credits: None,
        };
        let reading = LimitsReading {
            fetched_at_ms: at_ms,
            source: LimitSource::GetUsage,
            limits: vec![
                limit("session", "session", None, session_percent, session_resets),
                limit(
                    "weekly_all",
                    "weekly",
                    None,
                    percent(spent_between(week_from, at_ms, false), weekly_budget),
                    resets_weekly,
                ),
                limit(
                    "weekly_scoped",
                    "weekly",
                    Some("Fable"),
                    percent(spent_between(week_from, at_ms, true), fable_budget),
                    resets_weekly,
                ),
            ],
        };
        store
            .record_limits(&account(), &reading)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn session_window_starts(spent: &[(i64, f64, bool)]) -> Vec<i64> {
    let mut starts: Vec<i64> = Vec::new();
    for (at_ms, ..) in spent {
        if starts
            .last()
            .is_none_or(|start| *at_ms >= start + SESSION_WINDOW_MS)
        {
            starts.push(*at_ms - at_ms.rem_euclid(HOUR_MS));
        }
    }
    starts
}

fn percent(usd: f64, budget: f64) -> f64 {
    (usd / budget * 100.0).round().min(100.0)
}

fn render_tui(home: &Path, columns: u16, rows: u16, out: &Path) -> Result<(), String> {
    let paths = Paths::new(home);
    let store = Store::open_read_only(&paths.database()).map_err(|error| error.to_string())?;
    let config = Config::load(&paths.config(), home).map_err(|error| error.to_string())?;
    let now_ms = now_ms();
    let snapshot =
        engine::snapshot::build(&store, &config, now_ms, &[]).map_err(|error| error.to_string())?;
    let known: BTreeSet<String> = snapshot
        .accounts
        .iter()
        .filter_map(|account| account.account.clone())
        .collect();
    let mut app = App::new(snapshot, Some(account().to_string()), Some(&known));
    app.set_engine_written_at(Some(now_ms - 20_000));
    app.set_report_port(Some(7878));
    app.set_palette(TEXT, BACKGROUND);
    let local_minute = |ms: i64| {
        store
            .local_datetime(ms)
            .ok()
            .flatten()
            .map(|time| time.chars().take(16).collect())
            .unwrap_or_default()
    };
    let mut terminal =
        Terminal::new(TestBackend::new(columns, rows)).map_err(|error| error.to_string())?;
    for _ in 0..3 {
        gigawatts::tui::history::load(&mut app, &store);
        terminal
            .draw(|frame| app.draw(frame, now_ms, &local_minute))
            .map_err(|error| error.to_string())?;
    }
    std::fs::write(out, html(terminal.backend().buffer())).map_err(|error| error.to_string())
}

const TEXT: Rgb = Rgb(0xd8, 0xdb, 0xe2);
const BACKGROUND: Rgb = Rgb(0x16, 0x18, 0x1d);

fn css(color: Color, default: Rgb) -> String {
    let Rgb(red, green, blue) = match color {
        Color::Reset => default,
        Color::Black => Rgb(0x16, 0x18, 0x1d),
        Color::Red => Rgb(0xe0, 0x6c, 0x75),
        Color::Green => Rgb(0x98, 0xc3, 0x79),
        Color::Yellow => Rgb(0xe5, 0xc0, 0x7b),
        Color::Blue => Rgb(0x61, 0xaf, 0xef),
        Color::Magenta => Rgb(0xc6, 0x78, 0xdd),
        Color::Cyan => Rgb(0x56, 0xb6, 0xc2),
        Color::Gray => Rgb(0xab, 0xb2, 0xbf),
        Color::DarkGray => Rgb(0x5c, 0x63, 0x70),
        Color::LightRed => Rgb(0xff, 0x7a, 0x85),
        Color::LightGreen => Rgb(0xa8, 0xe0, 0x8a),
        Color::LightYellow => Rgb(0xf5, 0xd6, 0x8e),
        Color::LightBlue => Rgb(0x7c, 0xc4, 0xff),
        Color::LightMagenta => Rgb(0xdb, 0x93, 0xf0),
        Color::LightCyan => Rgb(0x7f, 0xd8, 0xe3),
        Color::White => Rgb(0xff, 0xff, 0xff),
        Color::Rgb(red, green, blue) => Rgb(red, green, blue),
        Color::Indexed(index) => indexed(index),
    };
    format!("#{red:02x}{green:02x}{blue:02x}")
}

fn indexed(index: u8) -> Rgb {
    match index {
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            Rgb(level, level, level)
        }
        16..=231 => {
            let step = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            let n = index - 16;
            Rgb(step(n / 36), step(n / 6 % 6), step(n % 6))
        }
        _ => TEXT,
    }
}

fn html(buffer: &Buffer) -> String {
    let mut rows = String::new();
    let width = buffer.area.width as usize;
    for line in buffer.content.chunks(width) {
        let mut row = String::new();
        let mut skip = 0;
        for cell in line {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = cell.symbol();
            skip = unicode_width(symbol).saturating_sub(1);
            let class = if skip > 0 { "cell wide" } else { "cell" };
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (fg, bg) = (css(cell.fg, TEXT), css(cell.bg, BACKGROUND));
            let (fg, bg) = if reversed { (bg, fg) } else { (fg, bg) };
            let mut style = format!("color:{fg};");
            if cell.bg != Color::Reset || reversed {
                style.push_str(&format!("background:{bg};"));
            }
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str("font-weight:700;");
            }
            if cell.modifier.contains(Modifier::DIM) {
                style.push_str("opacity:.6;");
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                style.push_str("font-style:italic;");
            }
            if cell.modifier.contains(Modifier::UNDERLINED) {
                style.push_str("text-decoration:underline;");
            }
            match drawn(symbol, &fg) {
                Some(fill) => row.push_str(&format!(
                    "<span class=\"{class}\" style=\"{style}background:{fill};\"></span>"
                )),
                None => row.push_str(&format!(
                    "<span class=\"{class}\" style=\"{style}\">{}</span>",
                    escape(symbol)
                )),
            }
        }
        rows.push_str(&format!("<div class=\"row\">{row}</div>"));
    }
    format!(
        "<!doctype html><meta charset=\"utf-8\"><style>{STYLE}</style>\
         <div class=\"window\"><div class=\"bar\"><i></i><i></i><i></i><b>gigawatts</b></div>\
         <div class=\"screen\">{rows}</div></div>"
    )
}

fn drawn(symbol: &str, color: &str) -> Option<String> {
    block_fill(symbol, color).or_else(|| box_line(symbol, color))
}

fn box_line(symbol: &str, color: &str) -> Option<String> {
    let (up, down, left, right) = match symbol {
        "│" => (true, true, false, false),
        "─" => (false, false, true, true),
        "┌" | "╭" => (false, true, false, true),
        "┐" | "╮" => (false, true, true, false),
        "└" | "╰" => (true, false, false, true),
        "┘" | "╯" => (true, false, true, false),
        "├" => (true, true, false, true),
        "┤" => (true, true, true, false),
        "┬" => (false, true, true, true),
        "┴" => (true, false, true, true),
        "┼" => (true, true, true, true),
        _ => return None,
    };
    let half = "calc(50% + 0.5px)";
    let layer = |position: &str, size: String| {
        format!("linear-gradient({color}, {color}) {position} / {size} no-repeat")
    };
    let layers: Vec<String> = [
        (up, layer("50% 0", format!("1px {half}"))),
        (down, layer("50% 100%", format!("1px {half}"))),
        (left, layer("0 50%", format!("{half} 1px"))),
        (right, layer("100% 50%", format!("{half} 1px"))),
    ]
    .into_iter()
    .filter_map(|(wanted, layer)| wanted.then_some(layer))
    .collect();
    Some(layers.join(", "))
}

fn block_fill(symbol: &str, color: &str) -> Option<String> {
    let (direction, share) = match symbol {
        "█" => ("to top", 100.0),
        "▇" => ("to top", 87.5),
        "▆" => ("to top", 75.0),
        "▅" => ("to top", 62.5),
        "▄" => ("to top", 50.0),
        "▃" => ("to top", 37.5),
        "▂" => ("to top", 25.0),
        "▁" => ("to top", 12.5),
        "▀" => ("to bottom", 50.0),
        "▌" => ("to right", 50.0),
        "▐" => ("to left", 50.0),
        _ => return None,
    };
    Some(format!(
        "linear-gradient({direction}, {color} {share}%, transparent {share}%)"
    ))
}

fn unicode_width(symbol: &str) -> usize {
    symbol.chars().next().map_or(1, |c| {
        if ('\u{1100}'..='\u{1FAFF}').contains(&c) && !('\u{2000}'..='\u{2BFF}').contains(&c) {
            2
        } else {
            1
        }
    })
}

fn escape(symbol: &str) -> String {
    match symbol {
        "<" => "&lt;".to_string(),
        ">" => "&gt;".to_string(),
        "&" => "&amp;".to_string(),
        " " | "" => "&nbsp;".to_string(),
        other => other.to_string(),
    }
}

const STYLE: &str = "
html, body { margin: 0; background: transparent; }
body { padding: 32px; display: inline-block; }
.window { background: #16181d; border-radius: 10px; overflow: hidden;
  box-shadow: 0 0 0 1px #2c3038, 0 18px 50px rgba(0,0,0,.45); }
.bar { height: 30px; background: #22252c; display: flex; align-items: center; padding: 0 12px;
  gap: 8px; position: relative; }
.bar i { width: 12px; height: 12px; border-radius: 50%; display: block; }
.bar i:nth-child(1) { background: #ff5f57; } .bar i:nth-child(2) { background: #febc2e; }
.bar i:nth-child(3) { background: #28c840; }
.bar b { position: absolute; left: 0; right: 0; text-align: center; font: 600 12px -apple-system,
  sans-serif; color: #8b919c; pointer-events: none; }
.screen { padding: 10px 14px 12px; font: 14px/17px Menlo, 'SF Mono', monospace; color: #d8dbe2;
  white-space: pre; }
.row { height: 17px; }
.cell { display: inline-block; width: 9px; height: 17px; vertical-align: top;
  overflow: visible; text-align: center; }
.wide { width: 18px; }
";

fn serve_report(home: &Path, port: u16) -> Result<(), String> {
    let listener = gigawatts::report::bind(port).map_err(|error| error.to_string())?;
    gigawatts::report::serve(listener, Paths::new(home)).map_err(|error| error.to_string())?;
    println!("serving the demo report at http://127.0.0.1:{port}");
    loop {
        std::thread::park();
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn sqlite() -> rusqlite::Connection {
    rusqlite::Connection::open_in_memory().expect("an in-memory database always opens")
}

fn utc_text(ms: i64) -> String {
    sqlite()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1 / 1000.0, 'unixepoch')",
            [ms],
            |row| row.get(0),
        )
        .expect("strftime formats any time")
}

fn parse_ms(text: &str) -> i64 {
    sqlite()
        .query_row(
            "SELECT CAST(round((julianday(?1) - 2440587.5) * 86400000) AS INTEGER)",
            [text],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn local_midnight_ms(store: &Store, now_ms: i64) -> Result<i64, String> {
    let today = store.local_today().map_err(|error| error.to_string())?;
    Ok(sqlite()
        .query_row(
            "SELECT CAST(unixepoch(?1 || ' 00:00:00', 'utc') AS INTEGER) * 1000",
            [today],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(now_ms - now_ms.rem_euclid(DAY_MS)))
}

fn local_date(ms: i64) -> Result<String, String> {
    sqlite()
        .query_row(
            "SELECT date(?1 / 1000, 'unixepoch', 'localtime')",
            [ms],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn local_weekday(ms: i64) -> Result<u8, String> {
    sqlite()
        .query_row(
            "SELECT CAST(strftime('%w', ?1 / 1000, 'unixepoch', 'localtime') AS INTEGER)",
            [ms],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}
