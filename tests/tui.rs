use std::collections::BTreeSet;

use gigawatts::contract::{
    AccountLimits, AccountToday, Credits, DataResponse, DataRow, DollarPace, GroupBy, Limit,
    LimitForecast, LimitPace, LimitSource, PaceBasis, PaceState, PacedLimit, Problem, ProblemKind,
    Range, SCHEMA_VERSION, SessionSpend, Share, Snapshot,
};
use gigawatts::store::{Store, TopItem, UsageAdvice};
use gigawatts::tui::{App, Flow, HistoryQuery, Spending};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;

const NOW_MS: i64 = 1_794_216_000_000;
const PERSONAL: &str = "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000f0";
const WORK: &str = "00000000-0000-4000-8000-00000000000b/00000000-0000-4000-8000-0000000000e0";
const NEW: &str = "00000000-0000-4000-8000-00000000000c/00000000-0000-4000-8000-0000000000f0";

fn utc_minute(ms: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%d %H:%M', ?1 / 1000, 'unixepoch')",
            [ms],
            |row| row.get(0),
        )
        .unwrap()
}

fn timed(
    kind: &str,
    scope: Option<&str>,
    percent: f64,
    resets_at: &str,
    pace: LimitPace,
) -> PacedLimit {
    PacedLimit {
        limit: Limit {
            kind: kind.to_string(),
            group: None,
            scope: scope.map(str::to_string),
            percent,
            severity: None,
            resets_at: Some(resets_at.to_string()),
            active: true,
            credits: None,
        },
        pace: Some(pace),
        forecast: None,
    }
}

fn pace(state: PaceState, basis: PaceBasis) -> LimitPace {
    LimitPace {
        state,
        basis,
        expected_percent: 50.0,
        window_start_ms: 0,
        window_end_ms: 0,
        provisional: false,
    }
}

fn session(label: &str, repo: &str, branch: &str, recent: f64, total: f64) -> SessionSpend {
    SessionSpend {
        session: format!("{label}-id"),
        label: label.to_string(),
        repo: repo.to_string(),
        branch: Some(branch.to_string()),
        last_15_minutes_usd: recent,
        total_usd: total,
        last_active_ms: NOW_MS - 2 * 60_000,
        today_usd: total,
        top_driver: None,
        models: Vec::new(),
    }
}

fn personal() -> AccountToday {
    let active_sessions = vec![
        session("Fix the login redirect", "orchard", "ab/login", 1.2, 8.4),
        session("Write release notes", "gigawatts", "main", 0.1, 0.5),
    ];
    AccountToday {
        account: Some(PERSONAL.to_string()),
        account_label: "personal".to_string(),
        configured_label: None,
        requests: 40,
        cost_usd: 14.2,
        pace: DollarPace {
            today_usd: 14.2,
            usual_usd: Some(7.1),
            usual_days: 20,
            clock: "09:20".to_string(),
            weekend: false,
            earlier_days_usd: vec![
                5.0, 6.2, 8.4, 4.1, 7.1, 9.3, 6.8, 7.2, 10.5, 3.9, 7.1, 7.1, 8.8, 5.5, 6.0, 9.9,
                7.6, 4.8, 6.6, 8.1,
            ],
        },
        fastest_session: active_sessions.first().cloned(),
        active_sessions,
        cost_estimated: true,
        unpriced_models: vec!["claude-opus-5-6".to_string()],
        adjusted_models: Vec::new(),
        limits: Some(AccountLimits {
            source: LimitSource::GetUsage,
            fetched_at_ms: NOW_MS - 3 * 60_000,
            limits: vec![
                timed(
                    "session",
                    None,
                    5.0,
                    "2026-11-09T12:00:00Z",
                    pace(PaceState::Even, PaceBasis::Even),
                ),
                timed(
                    "weekly_all",
                    None,
                    63.0,
                    "2026-11-12T03:00:00Z",
                    pace(PaceState::Over, PaceBasis::Usual),
                ),
                timed(
                    "weekly_scoped",
                    Some("Fable"),
                    20.0,
                    "2026-11-12T03:00:00Z",
                    pace(PaceState::Under, PaceBasis::Even),
                ),
            ],
        }),
    }
}

fn work() -> AccountToday {
    AccountToday {
        account: Some(WORK.to_string()),
        account_label: "work".to_string(),
        configured_label: None,
        requests: 2,
        cost_usd: 0.4,
        pace: DollarPace {
            today_usd: 0.4,
            usual_usd: None,
            usual_days: 0,
            clock: "09:20".to_string(),
            weekend: false,
            earlier_days_usd: Vec::new(),
        },
        fastest_session: None,
        active_sessions: Vec::new(),
        cost_estimated: false,
        unpriced_models: Vec::new(),
        adjusted_models: Vec::new(),
        limits: Some(AccountLimits {
            source: LimitSource::CachedUsageUtilization,
            fetched_at_ms: NOW_MS - 2 * 3_600_000,
            limits: vec![PacedLimit {
                limit: Limit {
                    kind: "credits".to_string(),
                    group: None,
                    scope: None,
                    percent: 20.0,
                    severity: None,
                    resets_at: None,
                    active: false,
                    credits: Some(Credits {
                        used: 437.6,
                        limit: Some(2200.0),
                        currency: "USD".to_string(),
                    }),
                },
                pace: Some(LimitPace {
                    state: PaceState::Under,
                    basis: PaceBasis::Even,
                    expected_percent: 26.0,
                    window_start_ms: 1_793_491_200_000,
                    window_end_ms: 1_796_083_200_000,
                    provisional: true,
                }),
                forecast: None,
            }],
        }),
    }
}

fn problem(kind: ProblemKind, affects: &str, since: Option<&str>, cause: &str) -> Problem {
    Problem {
        kind,
        affects: affects.to_string(),
        since: since.map(str::to_string),
        cause: cause.to_string(),
        fix: "fix it".to_string(),
        account: None,
    }
}

fn owned_by(account: &str, problem: Problem) -> Problem {
    Problem {
        account: Some(account.to_string()),
        ..problem
    }
}

fn snapshot(accounts: Vec<AccountToday>) -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        engine_version: "0.1.5+r1.m9".to_string(),
        written_at_ms: NOW_MS,
        today: "2026-11-09".to_string(),
        accounts,
        problems: vec![
            owned_by(
                PERSONAL,
                problem(
                    ProblemKind::RefreshFailing,
                    "limits for personal: showing the reading from 2026-11-09 09:17:00",
                    Some("2026-11-09 09:00:00"),
                    "Gigawatts can't refresh limits through `claude -p`: timed out",
                ),
            ),
            problem(
                ProblemKind::NoPrice,
                "cost: claude-opus-5-6 in 3 requests",
                Some("2026-11-09 08:00:00"),
                "no price for claude-opus-5-6",
            ),
            problem(
                ProblemKind::UnrecognizedLines,
                "history: any requests on those lines are missing from every total",
                None,
                "Lines of type \"x\" that carry usage (2 in 1 file)",
            ),
        ],
    }
}

fn open(shown: Snapshot, last_used: Option<String>) -> App {
    let announced = shown
        .accounts
        .iter()
        .filter_map(|account| account.account.clone())
        .collect();
    App::new(shown, last_used, Some(&announced))
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame, NOW_MS, &utc_minute))
        .unwrap();
    terminal.backend().to_string()
}

#[test]
fn the_top_panel_shows_the_current_accounts_limits_today_and_active_sessions() {
    let app = open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    );

    insta::assert_snapshot!(screen(&app, 80, 24));
}

#[test]
fn the_view_starts_on_the_account_used_most_recently_and_a_number_key_moves_to_another() {
    let mut app = open(snapshot(vec![personal(), work()]), Some(WORK.to_string()));
    let first = screen(&app, 80, 24);

    assert_eq!(app.handle(KeyCode::Char('1').into()), Flow::Continue);
    let second = screen(&app, 80, 24);
    app.handle(KeyCode::Char('2').into());
    let third = screen(&app, 80, 24);

    insta::assert_snapshot!(first);
    assert!(second.contains("Weekly, Fable"), "{second}");
    assert_eq!(third, first);
}

#[test]
fn a_stacks_every_accounts_limits_and_spend_today() {
    let mut app = open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    );

    app.handle(KeyCode::Char('a').into());

    insta::assert_snapshot!(screen(&app, 80, 24));
}

#[test]
fn an_account_that_appears_while_open_is_announced_until_the_next_key() {
    let mut app = open(snapshot(vec![personal()]), Some(PERSONAL.to_string()));
    let mut newcomer = work();
    newcomer.account = Some(NEW.to_string());
    newcomer.account_label = "side project".to_string();

    app.update(snapshot(vec![personal(), newcomer]));
    let announced = screen(&app, 80, 24);
    app.handle(KeyCode::Char('x').into());
    let after_a_key = screen(&app, 80, 24);

    assert!(
        announced.contains("New account: side project. Press 2 to show it."),
        "{announced}"
    );
    assert!(!after_a_key.contains("New account"), "{after_a_key}");
}

#[test]
fn q_and_ctrl_c_quit_and_escape_does_not() {
    let mut app = open(snapshot(vec![personal()]), None);

    assert_eq!(app.handle(KeyCode::Char('q').into()), Flow::Quit);
    assert_eq!(app.handle(KeyCode::Esc.into()), Flow::Continue);
    assert_eq!(
        app.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Flow::Quit
    );
    assert_eq!(app.handle(KeyCode::Char('c').into()), Flow::Continue);
}

#[test]
fn with_no_accounts_the_view_says_where_to_look() {
    let app = open(snapshot(Vec::new()), None);

    let shown = screen(&app, 80, 24);

    assert!(
        shown.contains("No accounts yet. `gigawatts accounts` shows what Gigawatts found."),
        "{shown}"
    );
}

#[test]
fn a_failed_read_keeps_the_last_numbers_and_says_so_until_a_read_succeeds() {
    let mut app = open(snapshot(vec![personal()]), None);

    app.failed_to_read("database is locked".to_string());
    let failing = screen(&app, 80, 24);
    app.update(snapshot(vec![personal()]));
    let recovered = screen(&app, 80, 24);

    assert!(failing.contains("$14.20"), "{failing}");
    assert!(
        failing.contains("Showing the last numbers read."),
        "{failing}"
    );
    assert!(
        failing.contains("Can't read the history database: database is locked"),
        "{failing}"
    );
    assert!(!recovered.contains("Can't read"), "{recovered}");
}

#[test]
fn when_the_shown_account_leaves_the_view_falls_back_to_the_first_account() {
    let mut app = open(snapshot(vec![personal(), work()]), Some(WORK.to_string()));
    let mut newcomer = work();
    newcomer.account = Some(NEW.to_string());
    newcomer.account_label = "side project".to_string();

    app.update(snapshot(vec![personal(), newcomer]));

    assert!(
        cell_at(&app, 80, 24, "1 personal")
            .modifier
            .contains(Modifier::REVERSED)
    );
    assert!(screen(&app, 80, 24).contains("Weekly, Fable"));
}

#[test]
fn a_limit_anthropic_marks_with_a_severity_shows_it() {
    let mut flagged = personal();
    let limits = flagged.limits.as_mut().unwrap();
    limits.limits[1].limit.severity = Some("warning".to_string());
    limits.limits[2].limit.severity = Some("normal".to_string());
    let app = open(snapshot(vec![flagged]), None);

    let shown = screen(&app, 120, 24);

    assert!(
        shown.contains("│                     over your usual pace · 50% expected · resets in 2d 17h · warning "),
        "{shown}"
    );
    assert!(!shown.contains("normal"), "{shown}");
}

#[test]
fn the_every_account_view_keeps_a_limits_severity_and_credits_at_80_columns() {
    let mut flagged = personal();
    flagged.limits.as_mut().unwrap().limits[1].limit.severity = Some("warning".to_string());
    let mut app = open(snapshot(vec![flagged, work()]), None);

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 80, 40);

    assert_eq!(stacked.matches("warning").count(), 1, "{stacked}");
    assert!(
        stacked.contains("$437.60 of $2,200 · $1,762 left · $82 a day to last"),
        "{stacked}"
    );
    assert!(stacked.contains("20%  resets 21d 14h"), "{stacked}");
}

#[test]
fn a_problem_marks_only_the_account_it_belongs_to_and_one_with_no_account_marks_every_account() {
    let mut twin = work();
    twin.account_label = "personal".to_string();
    let mut shown = snapshot(vec![personal(), twin]);
    shown.problems.push(owned_by(
        WORK,
        problem(
            ProblemKind::ReadFailing,
            "limits from /home/x/.claude-work/.claude.json: the saved reading isn't updated",
            None,
            "bad json",
        ),
    ));
    shown.problems.push(problem(
        ProblemKind::ReadFailing,
        "today's spend and history: requests in /home/x/.claude-other/projects aren't read",
        None,
        "bad login",
    ));
    let mut app = open(shown, Some(PERSONAL.to_string()));

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 100, 40);

    assert_eq!(
        stacked.matches("limits: refresh failing").count(),
        1,
        "{stacked}"
    );
    assert_eq!(
        stacked.matches("limits from /home/x").count(),
        1,
        "{stacked}"
    );
    assert!(
        stacked.find("limits: refresh failing") < stacked.find("Limits · from cachedUsage"),
        "{stacked}"
    );
    assert!(
        stacked.find("Limits · from cachedUsage") < stacked.find("limits from /home/x"),
        "{stacked}"
    );
    assert_eq!(stacked.matches("aren't read").count(), 2, "{stacked}");
}

#[test]
fn the_footer_says_when_the_engine_stops_until_it_runs_again() {
    let mut app = open(snapshot(vec![personal()]), None);

    app.set_engine_running(false);
    let stopped = screen(&app, 80, 24);
    app.set_engine_running(true);
    let running = screen(&app, 80, 24);

    assert!(
        stopped.contains("The engine stopped, so these numbers don't update."),
        "{stopped}"
    );
    assert!(
        stopped.contains("Reopen `gigawatts` to start it."),
        "{stopped}"
    );
    assert!(!running.contains("engine stopped"), "{running}");
}

#[test]
fn an_account_found_while_the_view_was_closed_is_announced_when_it_opens() {
    let announced = BTreeSet::from([PERSONAL.to_string()]);

    let app = App::new(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
        Some(&announced),
    );
    let shown = screen(&app, 80, 24);

    assert!(
        shown.contains("New account: work. Press 2 to show it."),
        "{shown}"
    );
    assert!(!shown.contains("New account: personal"), "{shown}");
}

#[test]
fn the_view_hands_back_the_announced_accounts_once_per_new_announcement() {
    let announced = BTreeSet::from([PERSONAL.to_string()]);
    let mut app = App::new(snapshot(vec![personal()]), None, Some(&announced));
    let at_open = app.take_unsaved_announced();
    let mut newcomer = work();
    newcomer.account = Some(NEW.to_string());

    app.update(snapshot(vec![personal(), newcomer]));
    let after_newcomer = app.take_unsaved_announced();
    app.update(snapshot(vec![personal()]));

    assert_eq!(at_open, None);
    assert_eq!(
        after_newcomer,
        Some(BTreeSet::from([PERSONAL.to_string(), NEW.to_string()]))
    );
    assert_eq!(app.take_unsaved_announced(), None);
}

#[test]
fn announced_accounts_survive_a_write_and_read_and_a_missing_file_reads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("announced.json");
    let accounts = BTreeSet::from([PERSONAL.to_string(), WORK.to_string()]);

    let missing = gigawatts::tui::announced::read(&path);
    gigawatts::tui::announced::write(&path, &accounts).unwrap();

    assert_eq!(missing, None);
    assert_eq!(gigawatts::tui::announced::read(&path), Some(accounts));
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn the_number_key_hint_is_left_off_for_the_account_already_shown() {
    let other = BTreeSet::from([NEW.to_string()]);

    let only = screen(
        &App::new(snapshot(vec![personal()]), None, Some(&other)),
        80,
        24,
    );
    let both = screen(
        &App::new(
            snapshot(vec![personal(), work()]),
            Some(PERSONAL.to_string()),
            Some(&other),
        ),
        80,
        24,
    );

    assert!(only.contains("New account: personal.  "), "{only}");
    assert!(both.contains("New account: personal.  "), "{both}");
    assert!(
        both.contains("New account: work. Press 2 to show it."),
        "{both}"
    );
}

#[test]
fn spend_today_says_estimated_only_while_a_session_is_unchecked() {
    let mut checked = personal();
    checked.cost_estimated = false;

    let unchecked = screen(&open(snapshot(vec![personal()]), None), 80, 24);
    let done = screen(&open(snapshot(vec![checked]), None), 80, 24);

    assert!(
        unchecked.contains("$14.20  estimated · 2.0×"),
        "{unchecked}"
    );
    assert!(done.contains("$14.20 · 2.0×"), "{done}");
}

#[test]
fn announced_accounts_that_failed_to_save_are_handed_back_again() {
    let mut app = App::new(snapshot(vec![personal()]), None, None);
    let first = app.take_unsaved_announced();

    app.failed_to_save_announced();

    assert_eq!(first, Some(BTreeSet::from([PERSONAL.to_string()])));
    assert_eq!(
        app.take_unsaved_announced(),
        Some(BTreeSet::from([PERSONAL.to_string()]))
    );
}

#[test]
fn the_footer_says_when_the_view_can_only_update_once_a_minute() {
    let mut app = open(snapshot(vec![personal()]), None);
    let watching = screen(&app, 80, 24);

    app.set_watching_snapshot(false);
    let unwatched = screen(&app, 80, 24);

    assert!(!watching.contains("once a minute"), "{watching}");
    assert!(
        unwatched.contains("Can't watch for new numbers, so these update once a minute."),
        "{unwatched}"
    );
}

#[test]
fn the_footer_says_when_config_toml_stops_loading_until_it_loads_again() {
    let mut app = open(snapshot(vec![personal()]), None);

    app.set_config_readable(false);
    let unreadable = screen(&app, 80, 24);
    app.set_config_readable(true);
    let readable = screen(&app, 80, 24);

    assert!(
        unreadable.contains("Can't read config.toml, so the view keeps the settings read before."),
        "{unreadable}"
    );
    assert!(
        unreadable.contains("`gigawatts status` shows why."),
        "{unreadable}"
    );
    assert!(!readable.contains("config.toml"), "{readable}");
}

fn cell_at(app: &App, width: u16, height: u16, text: &str) -> ratatui::buffer::Cell {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame, NOW_MS, &utc_minute))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let row: Vec<&str> = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let joined = row.concat();
        if let Some(byte) = joined.find(text) {
            let x = joined[..byte].chars().count() as u16;
            return buffer[(x, y)].clone();
        }
    }
    panic!("{text} isn't on screen");
}

#[test]
fn number_keys_pick_an_account_from_the_tab_row() {
    let mut app = open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    );
    let first = screen(&app, 80, 24);

    app.handle(KeyCode::Char('9').into());
    let after_nine = screen(&app, 80, 24);
    app.handle(KeyCode::Char('2').into());
    let second = screen(&app, 80, 24);

    assert!(first.contains("1 personal"), "{first}");
    assert!(first.contains("2 work"), "{first}");
    assert!(first.contains("Weekly, Fable"), "{first}");
    assert_eq!(after_nine, first);
    assert!(second.contains("Credits"), "{second}");
    assert!(!second.contains("Weekly, Fable"), "{second}");
}

#[test]
fn brackets_step_through_the_tabs_and_wrap_past_all() {
    let mut app = open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    );
    let mut after = |key: char| {
        app.handle(KeyCode::Char(key).into());
        screen(&app, 80, 24)
    };

    let work_tab = after(']');
    let all_tab = after(']');
    let wrapped = after(']');
    let back_to_all = after('[');
    let back_to_work = after('[');

    assert!(work_tab.contains("Credits"), "{work_tab}");
    assert!(!work_tab.contains("across 2 accounts"), "{work_tab}");
    assert!(
        all_tab.contains("$14.60 today across 2 accounts"),
        "{all_tab}"
    );
    assert!(wrapped.contains("Weekly, Fable"), "{wrapped}");
    assert!(!wrapped.contains("across 2 accounts"), "{wrapped}");
    assert!(back_to_all.contains("across 2 accounts"), "{back_to_all}");
    assert_eq!(back_to_work, work_tab);
}

#[test]
fn with_one_account_brackets_do_nothing() {
    let mut app = open(snapshot(vec![personal()]), None);
    let before = screen(&app, 80, 24);

    app.handle(KeyCode::Char(']').into());
    app.handle(KeyCode::Char('[').into());

    assert_eq!(screen(&app, 80, 24), before);
}

#[test]
fn the_account_shown_is_highlighted_in_the_tab_row() {
    let app = open(snapshot(vec![personal(), work()]), Some(WORK.to_string()));

    let shown = cell_at(&app, 80, 24, "2 work");
    let other = cell_at(&app, 80, 24, "1 personal");

    assert!(shown.modifier.contains(Modifier::REVERSED));
    assert!(!other.modifier.contains(Modifier::REVERSED));
}

#[test]
fn question_mark_opens_help_and_question_mark_or_escape_closes_it() {
    let mut app = open(snapshot(vec![personal(), work()]), None);

    app.handle(KeyCode::Char('?').into());
    let help = screen(&app, 80, 24);
    let escape = app.handle(KeyCode::Esc.into());
    let closed = screen(&app, 80, 24);
    app.handle(KeyCode::Char('?').into());
    app.handle(KeyCode::Char('?').into());
    let toggled = screen(&app, 80, 24);

    assert!(help.contains("Keys"), "{help}");
    assert!(help.contains("1-9"), "{help}");
    assert!(help.contains("show that account"), "{help}");
    assert!(help.contains("over your pace"), "{help}");
    assert!(help.contains("expected use by now"), "{help}");
    assert_eq!(escape, Flow::Continue);
    assert!(!closed.contains("show that account"), "{closed}");
    assert!(!toggled.contains("show that account"), "{toggled}");
}

#[test]
fn the_footer_lists_the_keys_that_work_on_this_screen() {
    let both = screen(&open(snapshot(vec![personal(), work()]), None), 80, 24);
    let one = screen(&open(snapshot(vec![personal()]), None), 80, 24);

    assert!(both.contains("1-2 account"), "{both}");
    assert!(both.contains("[ ] switch"), "{both}");
    assert!(both.contains("a all"), "{both}");
    assert!(both.contains("? help"), "{both}");
    assert!(both.contains("q quit"), "{both}");
    assert!(!one.contains("1-1"), "{one}");
    assert!(!one.contains("a all"), "{one}");
    assert!(!one.contains("[ ]"), "{one}");
    assert!(one.contains("? help"), "{one}");
}

#[test]
fn the_header_says_how_long_ago_the_engine_last_wrote_and_nothing_when_unknown() {
    let mut app = open(snapshot(vec![personal()]), None);
    let unknown = screen(&app, 80, 24);

    app.set_engine_written_at(Some(NOW_MS - 3 * 60_000));
    let written = screen(&app, 80, 24);

    assert!(!unknown.contains("updated"), "{unknown}");
    assert!(written.contains("updated 3 minutes ago"), "{written}");
}

#[test]
fn each_limit_shows_the_expected_percentage_and_a_countdown_to_its_reset() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 120, 24);

    assert!(
        shown.contains("│                     on even pace · 50% expected · resets in 2h 40m"),
        "{shown}"
    );
    assert!(
        shown.contains(
            "│                     over your usual pace · 50% expected · resets in 2d 17h"
        ),
        "{shown}"
    );
}

fn filling_fast() -> AccountToday {
    let mut account = personal();
    let session = &mut account.limits.as_mut().unwrap().limits[0];
    session.limit.percent = 60.0;
    session.pace = Some(pace(PaceState::Over, PaceBasis::Even));
    session.forecast = Some(LimitForecast {
        full_at_ms: NOW_MS + 80 * 60_000,
    });
    account
}

#[test]
fn a_forecast_from_a_reading_over_an_hour_old_is_greyed_like_the_rest_of_the_limit() {
    let mut account = filling_fast();
    account.limits.as_mut().unwrap().fetched_at_ms = NOW_MS - 2 * 3_600_000;
    let mut app = open(snapshot(vec![account, work()]), None);

    assert_eq!(
        cell_at(&app, 120, 24, "100% in").fg,
        ratatui::style::Color::DarkGray
    );
    assert_eq!(
        cell_at(&app, 80, 24, "100% in").fg,
        ratatui::style::Color::DarkGray
    );

    app.handle(KeyCode::Char('a').into());

    assert_eq!(
        cell_at(&app, 80, 24, "100% in").fg,
        ratatui::style::Color::DarkGray
    );
}

#[test]
fn a_credit_forecast_on_the_default_reset_has_no_caveat() {
    let mut account = work();
    account.limits.as_mut().unwrap().limits[0].forecast = Some(LimitForecast {
        full_at_ms: NOW_MS + 80 * 60_000,
    });
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 120, 24);

    assert!(shown.contains("100% in 1h 20m at 10:40 "), "{shown}");
    assert!(!shown.contains("10:40 ("), "{shown}");
}

#[test]
fn a_limit_on_course_to_fill_before_its_reset_says_when_in_the_over_pace_color() {
    let app = open(snapshot(vec![filling_fast()]), None);

    let shown = screen(&app, 120, 24);

    assert!(shown.contains("100% in 1h 20m at 10:40"), "{shown}");
    assert_eq!(
        cell_at(&app, 120, 24, "100% in").fg,
        ratatui::style::Color::LightRed
    );
    insta::assert_snapshot!(screen(&app, 80, 24));
}

#[test]
fn the_every_account_view_says_when_a_limit_will_fill() {
    let mut app = open(snapshot(vec![filling_fast(), work()]), None);
    app.handle(KeyCode::Char('a').into());

    let shown = screen(&app, 80, 24);

    assert!(
        shown.contains("│                     100% in 1h 20m      "),
        "{shown}"
    );
}

#[test]
fn a_forecast_time_already_past_says_by_when() {
    let mut account = filling_fast();
    account.limits.as_mut().unwrap().limits[0].forecast = Some(LimitForecast {
        full_at_ms: NOW_MS - 10 * 60_000,
    });
    let mut app = open(snapshot(vec![account, work()]), None);

    assert!(screen(&app, 120, 24).contains("100% by 09:10"));

    app.handle(KeyCode::Char('a').into());

    assert!(screen(&app, 80, 24).contains("100% by now"));
}

#[test]
fn a_forecast_time_on_an_earlier_day_shows_its_date() {
    let mut account = filling_fast();
    account.limits.as_mut().unwrap().limits[1].forecast = Some(LimitForecast {
        full_at_ms: NOW_MS - 2 * 24 * 3_600_000,
    });
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 120, 24);

    assert!(shown.contains("100% by 2026-11-07 09:20"), "{shown}");
}

#[test]
fn a_limit_that_reset_since_its_reading_shows_no_forecast() {
    let mut account = filling_fast();
    account.limits.as_mut().unwrap().limits[0].limit.resets_at =
        Some("2026-11-09T09:00:00Z".to_string());
    let mut app = open(snapshot(vec![account, work()]), None);

    for width in [120, 80] {
        let shown = screen(&app, width, 24);
        assert!(!shown.contains("100%"), "{shown}");
    }

    app.handle(KeyCode::Char('a').into());

    let shown = screen(&app, 80, 24);
    assert!(!shown.contains("100%"), "{shown}");
}

#[test]
fn a_limit_is_colored_by_its_pace() {
    let app = open(snapshot(vec![personal()]), None);

    let even = cell_at(&app, 120, 24, " 5%");
    let over = cell_at(&app, 120, 24, "63%");
    let under = cell_at(&app, 120, 24, "20%");

    assert_eq!(even.fg, ratatui::style::Color::LightYellow);
    assert_eq!(over.fg, ratatui::style::Color::LightRed);
    assert_eq!(under.fg, ratatui::style::Color::LightGreen);
}

#[test]
fn spend_today_is_even_within_a_tenth_of_usual_and_colored_like_limit_pace() {
    let color_at = |today_usd: f64| {
        let mut account = personal();
        account.pace.today_usd = today_usd;
        cell_at(&open(snapshot(vec![account]), None), 80, 24, "× your usual").fg
    };

    assert_eq!(color_at(6.0), ratatui::style::Color::LightGreen);
    assert_eq!(color_at(6.5), ratatui::style::Color::LightYellow);
    assert_eq!(color_at(7.1), ratatui::style::Color::LightYellow);
    assert_eq!(color_at(7.8), ratatui::style::Color::LightYellow);
    assert_eq!(color_at(8.0), ratatui::style::Color::LightRed);
}

#[test]
fn spend_today_shows_the_usual_dollars_next_to_the_ratio() {
    let shown = screen(&open(snapshot(vec![personal()]), None), 80, 24);

    assert!(shown.contains("$14.20"), "{shown}");
    assert!(
        shown.contains("2.0× your usual of $7.10 by 09:20"),
        "{shown}"
    );
}

#[test]
fn every_account_view_adds_up_spend_today_across_accounts() {
    let mut app = open(snapshot(vec![personal(), work()]), None);

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 80, 40);

    assert!(
        stacked.contains("$14.60 today across 2 accounts"),
        "{stacked}"
    );
}

#[test]
fn the_first_launch_records_the_accounts_it_finds_without_announcing_them() {
    let mut app = App::new(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
        None,
    );

    let shown = screen(&app, 80, 24);

    assert!(!shown.contains("New account"), "{shown}");
    assert_eq!(
        app.take_unsaved_announced(),
        Some(BTreeSet::from([PERSONAL.to_string(), WORK.to_string()]))
    );
}

#[test]
fn a_usual_from_fewer_than_twenty_days_says_so_on_its_own_line() {
    let mut early = personal();
    early.pace.usual_days = 4;

    let shown = screen(&open(snapshot(vec![early]), None), 80, 24);

    assert!(
        shown.contains("2.0× your usual of $7.10 by 09:20 "),
        "{shown}"
    );
    assert!(
        shown.contains("usual is provisional: 4 of 20 days so far"),
        "{shown}"
    );
}

#[test]
fn the_every_account_view_closes_when_only_one_account_is_left() {
    let mut app = open(snapshot(vec![personal(), work()]), None);
    app.handle(KeyCode::Char('a').into());

    app.update(snapshot(vec![personal()]));
    let shown = screen(&app, 80, 24);

    assert!(!shown.contains("today across"), "{shown}");
    assert!(shown.contains("Running now"), "{shown}");
}

#[test]
fn every_account_fits_two_accounts_in_80_by_24() {
    let mut app = open(snapshot(vec![personal(), work()]), None);

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 80, 24);

    assert!(stacked.contains("$14.20  estimated"), "{stacked}");
    assert!(stacked.contains("Weekly, Fable"), "{stacked}");
    assert!(stacked.contains("Credits"), "{stacked}");
    assert!(stacked.contains("$0.40 · no usual yet"), "{stacked}");
}

#[test]
fn a_marker_longer_than_the_section_wraps_instead_of_losing_its_end() {
    let mut shown = snapshot(vec![personal()]);
    shown.problems.push(problem(
        ProblemKind::ReadFailing,
        "today's spend and history: requests in /home/x/.claude-other/projects aren't read",
        None,
        "bad login",
    ));

    let screen_text = screen(&open(shown, None), 80, 30);

    assert!(screen_text.contains("projects aren't"), "{screen_text}");
    assert!(screen_text.contains("│ read "), "{screen_text}");
}

#[test]
fn while_help_is_open_the_key_strip_lists_only_the_keys_that_work() {
    let mut app = open(snapshot(vec![personal(), work()]), None);

    app.handle(KeyCode::Char('?').into());
    let strip = screen(&app, 80, 24).lines().last().unwrap().to_string();

    assert!(strip.contains("? Esc close help"), "{strip}");
    assert!(strip.contains("q quit"), "{strip}");
    assert!(!strip.contains("account"), "{strip}");
    assert!(!strip.contains("a all"), "{strip}");
}

#[test]
fn the_combined_total_says_estimated_when_any_account_is() {
    let mut app = open(snapshot(vec![personal(), work()]), None);

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 80, 24);

    assert!(
        stacked.contains("$14.60 today across 2 accounts · estimated"),
        "{stacked}"
    );
}

#[test]
fn a_first_launch_before_the_engine_finds_any_account_stays_quiet_until_one_more_appears() {
    let mut app = App::new(snapshot(Vec::new()), None, None);

    app.update(snapshot(vec![personal(), work()]));
    let found = screen(&app, 80, 24);
    let mut newcomer = work();
    newcomer.account = Some(NEW.to_string());
    newcomer.account_label = "side project".to_string();
    app.update(snapshot(vec![personal(), work(), newcomer]));
    let later = screen(&app, 80, 24);

    assert!(!found.contains("New account"), "{found}");
    assert!(later.contains("New account: side project."), "{later}");
}

#[test]
fn a_corrupt_announced_file_reads_as_empty_so_every_account_is_announced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("announced.json");
    std::fs::write(&path, "not json").unwrap();

    let read = gigawatts::tui::announced::read(&path);
    let app = App::new(snapshot(vec![personal(), work()]), None, read.as_ref());
    let shown = screen(&app, 80, 24);

    assert_eq!(read, Some(BTreeSet::new()));
    assert!(shown.contains("New account: personal."), "{shown}");
    assert!(shown.contains("New account: work."), "{shown}");
}

#[test]
fn the_every_account_view_marks_a_limit_that_reset_since_its_reading() {
    let mut reset = personal();
    reset.limits.as_mut().unwrap().limits[0].limit.resets_at =
        Some("2026-11-09T09:00:00Z".to_string());
    let mut app = open(snapshot(vec![reset, work()]), None);

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 80, 40);
    let session_row = stacked
        .lines()
        .find(|line| line.contains("Session"))
        .unwrap();

    assert!(session_row.contains(" reset "), "{stacked}");
    assert!(!session_row.contains("5%"), "{stacked}");
    assert_eq!(
        cell_at(&app, 80, 40, " reset ").fg,
        ratatui::style::Color::DarkGray
    );
}

#[test]
fn a_notice_too_long_for_one_row_wraps_without_hiding_the_notices_after_it() {
    let mut app = open(snapshot(vec![personal()]), None);
    app.failed_to_read(
        "unable to open database file at /home/someone/.local/share/gigawatts/history.sqlite3 (code 14)"
            .to_string(),
    );

    let shown = screen(&app, 80, 24);

    assert!(shown.contains("(code 14)"), "{shown}");
    assert!(shown.contains("`gigawatts doctor`"), "{shown}");
}

fn row(account: &str, key: Option<&str>, label: &str, cost: f64) -> DataRow {
    DataRow {
        account: Some(account.to_string()),
        account_label: account.to_string(),
        key: key.map(str::to_string),
        label: label.to_string(),
        not_in_transcript: key.is_none(),
        status: None,
        requests: 1,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        web_searches: 0,
        cost_usd: Some(cost),
    }
}

fn top(account: &str, key: &str, label: &str, cost: f64) -> TopItem {
    TopItem {
        account: Some(account.to_string()),
        key: Some(key.to_string()),
        label: label.to_string(),
        cost_usd: cost,
    }
}

fn spending(wanted: &HistoryQuery, rows: Vec<DataRow>, top: Vec<TopItem>) -> Spending {
    Spending {
        response: DataResponse {
            schema_version: SCHEMA_VERSION,
            range: wanted.range,
            group_by: wanted.group_by,
            drill: Vec::new(),
            level: wanted.group_by.levels()[0],
            today: "2026-11-09".to_string(),
            since: Some("2026-11-03".to_string()),
            until: None,
            rows,
            problems: Vec::new(),
        },
        top,
    }
}

fn repo_branch(account: &str, repo: &str, branch: &str, cost: f64) -> DataRow {
    row(
        account,
        Some(&format!("/code/{repo}\u{1f}{branch}")),
        &format!("{repo} · {branch}"),
        cost,
    )
}

fn repos() -> Vec<DataRow> {
    vec![
        repo_branch(PERSONAL, "orchard", "ab/login", 30.0),
        repo_branch(PERSONAL, "gigawatts", "main", 6.0),
        row(PERSONAL, None, "unlogged usage", 4.0),
        repo_branch(WORK, "wiki", "main", 2.1),
    ]
}

fn top_skills() -> Vec<TopItem> {
    vec![
        top(PERSONAL, "/code/orchard", "review", 18.0),
        top(PERSONAL, "/code/gigawatts", "no skill", 3.0),
        top(WORK, "/code/wiki", "implement", 2.1),
        top(PERSONAL, "/code/orchard\u{1f}ab/login", "review", 18.0),
        top(PERSONAL, "/code/gigawatts\u{1f}main", "no skill", 3.0),
        top(WORK, "/code/wiki\u{1f}main", "implement", 2.1),
    ]
}

fn answer(app: &mut App) -> Option<HistoryQuery> {
    let wanted = app.take_history_query()?;
    app.set_history(&wanted, Ok(spending(&wanted, repos(), top_skills())));
    Some(wanted)
}

fn with_spending(mut app: App) -> App {
    answer(&mut app);
    app
}

#[test]
fn the_spending_table_sits_below_the_top_panel_when_the_window_has_room() {
    let app = with_spending(open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    ));

    insta::assert_snapshot!(screen(&app, 100, 34));
}

#[test]
fn in_80_by_24_the_spending_table_hides_behind_tab() {
    let mut app = with_spending(open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    ));
    let top = screen(&app, 80, 24);

    app.handle(KeyCode::Tab.into());
    let table = screen(&app, 80, 24);
    app.handle(KeyCode::Tab.into());
    let back = screen(&app, 80, 24);

    assert!(!top.contains("Spending"), "{top}");
    assert!(top.contains("Tab spending"), "{top}");
    insta::assert_snapshot!(table);
    assert_eq!(back, top);
}

#[test]
fn spending_starts_on_this_week_by_repo_and_g_t_and_their_capitals_step_both_ways() {
    let mut app = open(snapshot(vec![personal()]), None);
    let first = app.take_history_query().unwrap();
    let mut after = |key: char| {
        app.handle(KeyCode::Char(key).into());
        let wanted = app.take_history_query().unwrap();
        (wanted.group_by, wanted.range)
    };

    assert_eq!(
        (first.group_by, first.range),
        (GroupBy::Branch, Range::Week)
    );
    assert_eq!(after('g'), (GroupBy::Skill, Range::Week));
    assert_eq!(after('g'), (GroupBy::Agent, Range::Week));
    assert_eq!(after('G'), (GroupBy::Skill, Range::Week));
    assert_eq!(after('G'), (GroupBy::Branch, Range::Week));
    assert_eq!(after('G'), (GroupBy::Model, Range::Week));
    assert_eq!(after('t'), (GroupBy::Model, Range::Month));
    assert_eq!(after('t'), (GroupBy::Model, Range::All));
    assert_eq!(after('t'), (GroupBy::Model, Range::Today));
    assert_eq!(after('T'), (GroupBy::Model, Range::All));
}

#[test]
fn each_row_shows_its_cost_share_and_the_skill_that_spent_most_in_it() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 100, 34);

    assert!(shown.contains("Spending · $40.00 this week"), "{shown}");
    assert!(shown.contains("Top skill"), "{shown}");
    assert!(
        shown.contains("$30.00  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇    75%  60% review"),
        "{shown}"
    );
    assert!(
        shown.contains("$6.00  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇    15%  50% no skill"),
        "{shown}"
    );
    assert!(
        shown.contains("$4.00  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇    10%  "),
        "{shown}"
    );
}

#[test]
fn skills_name_the_repo_that_spent_most_instead() {
    let mut app = open(snapshot(vec![personal()]), None);
    app.take_history_query();

    app.handle(KeyCode::Char('g').into());
    let wanted = app.take_history_query().unwrap();
    app.set_history(
        &wanted,
        Ok(spending(
            &wanted,
            vec![row(PERSONAL, Some("review"), "review", 10.0)],
            vec![top(PERSONAL, "review", "gigawatts", 7.0)],
        )),
    );
    let shown = screen(&app, 100, 34);

    assert!(shown.contains("Top repo"), "{shown}");
    assert!(shown.contains(" 70% gigawatts"), "{shown}");
}

#[test]
fn a_long_list_scrolls_to_keep_the_highlight_on_screen() {
    let mut app = open(snapshot(vec![personal()]), None);
    app.take_history_query();
    app.handle(KeyCode::Char('g').into());
    let wanted = app.take_history_query().unwrap();
    let many = (0..30)
        .map(|n| {
            row(
                PERSONAL,
                Some(&format!("s{n:02}")),
                &format!("s{n:02}"),
                1.0,
            )
        })
        .collect();
    app.set_history(&wanted, Ok(spending(&wanted, many, Vec::new())));
    let first = screen(&app, 100, 34);

    for _ in 0..29 {
        app.handle(KeyCode::Down.into());
    }
    let bottom = screen(&app, 100, 34);

    assert!(first.contains("s00"), "{first}");
    assert!(!first.contains("s29"), "{first}");
    assert!(!first.contains(" more"), "{first}");
    assert!(bottom.contains("s29"), "{bottom}");
    assert!(!bottom.contains("s00"), "{bottom}");
}

#[test]
fn spending_shows_the_shown_accounts_rows_and_every_account_view_labels_each_row() {
    let mut app = with_spending(open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    ));
    let personal_rows = screen(&app, 100, 34);

    app.handle(KeyCode::Char('2').into());
    let work_rows = screen(&app, 100, 34);
    app.handle(KeyCode::Char('a').into());
    let every = screen(&app, 100, 44);

    assert!(personal_rows.contains("orchard"), "{personal_rows}");
    assert!(!personal_rows.contains("wiki"), "{personal_rows}");
    assert!(work_rows.contains("wiki"), "{work_rows}");
    assert!(!work_rows.contains("orchard"), "{work_rows}");
    assert!(every.contains("orchard"), "{every}");
    assert!(every.contains("wiki"), "{every}");
    assert!(every.contains("100% implement"), "{every}");
}

#[test]
fn a_failed_spending_query_says_why_in_the_table() {
    let mut app = open(snapshot(vec![personal()]), None);
    let wanted = app.take_history_query().unwrap();
    app.set_history(&wanted, Err("database is locked".to_string()));

    let shown = screen(&app, 100, 34);

    assert!(shown.contains("database is locked"), "{shown}");
}

#[test]
fn a_snapshot_update_asks_for_spending_again() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));

    app.update(snapshot(vec![personal()]));

    assert!(answer(&mut app).is_some());
    assert!(app.take_history_query().is_none());
}

#[test]
fn escape_closes_advice_and_help_and_never_quits() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));

    app.handle(KeyCode::Char('u').into());
    let closing = app.handle(KeyCode::Esc.into());
    let nothing_open = app.handle(KeyCode::Esc.into());

    assert_eq!(closing, Flow::Continue);
    assert_eq!(nothing_open, Flow::Continue);
    assert!(!screen(&app, 100, 34).contains("Usage advice"));
}

#[test]
fn u_shows_todays_usage_advice_for_the_shown_account() {
    let mut app = open(snapshot(vec![personal()]), None);

    app.handle(KeyCode::Char('u').into());
    let wanted = app.take_advice_query();
    app.set_advice(Ok(Some(UsageAdvice {
        fetched_at_ms: NOW_MS - 12 * 60_000,
        behaviors: serde_json::json!({
            "day": {
                "session_count": 13,
                "request_count": 2319,
                "recommendations": [
                    {"key": "subagent_heavy", "pct": 94, "title": "Give \"review\" subagents a cheaper model"},
                    {"key": "long_context", "pct": 38, "title": "Start fresh when you switch tasks", "detail": "run /clear"},
                    {"key": "always_on_context", "pct": 22}
                ],
                "agents": [{"name": "review", "pct": 33}, {"name": "deployer", "pct": 6}],
                "skills": [{"name": "implement", "pct": 4}]
            },
            "week": {"session_count": 40, "request_count": 9000, "recommendations": []}
        }),
    })));
    let advice = screen(&app, 100, 34);
    app.handle(KeyCode::Esc.into());
    let closed = screen(&app, 100, 34);

    assert_eq!(wanted.as_deref(), Some(PERSONAL));
    insta::assert_snapshot!(advice);
    assert!(!closed.contains("Usage advice"), "{closed}");
}

#[test]
fn usage_advice_says_when_none_is_saved_today() {
    let mut app = open(snapshot(vec![personal()]), None);

    app.handle(KeyCode::Char('u').into());
    app.take_advice_query();
    app.set_advice(Ok(None));
    let shown = screen(&app, 80, 24);

    assert!(shown.contains("No usage advice saved today"), "{shown}");
}

#[test]
fn the_key_strip_lists_the_spending_keys() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let strip = screen(&app, 100, 34).lines().last().unwrap().to_string();

    assert!(strip.contains("← → group"), "{strip}");
    assert!(strip.contains("t time"), "{strip}");
    assert!(strip.contains("u advice"), "{strip}");
    assert!(!strip.contains("Tab"), "{strip}");
    assert!(!strip.contains("open"), "{strip}");
}

#[test]
fn spending_load_reads_the_range_and_top_skills_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    gigawatts::ingest::ingest_tree(
        &mut store,
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/projects/plain"
        )),
        &gigawatts::pricing::PriceTable::built_in(),
        &gigawatts::accounts::Attribution::default(),
        &gigawatts::sources::git::Resolver::default(),
    )
    .unwrap();
    let no_account = AccountToday {
        account: None,
        ..personal()
    };
    let mut app = open(snapshot(vec![no_account]), None);
    app.handle(KeyCode::Char('t').into());
    app.handle(KeyCode::Char('t').into());

    gigawatts::tui::history::load(&mut app, &store);
    let shown = screen(&app, 100, 34);

    assert!(app.take_history_query().is_none());
    assert!(shown.contains("all time"), "{shown}");
    assert!(shown.contains("Top skill"), "{shown}");
    assert!(shown.contains(" 70% no skill"), "{shown}");
}

#[test]
fn left_and_right_step_the_grouping_and_up_and_down_ask_for_nothing() {
    let mut app = open(snapshot(vec![personal()]), None);
    app.take_history_query();
    let mut after = |key: KeyCode| {
        app.handle(key.into());
        app.take_history_query()
            .map(|wanted| (wanted.group_by, wanted.range))
    };

    assert_eq!(after(KeyCode::Right), Some((GroupBy::Skill, Range::Week)));
    assert_eq!(after(KeyCode::Left), Some((GroupBy::Branch, Range::Week)));
    assert_eq!(after(KeyCode::Left), Some((GroupBy::Model, Range::Week)));
    assert_eq!(after(KeyCode::Down), None);
    assert_eq!(after(KeyCode::Up), None);
}

#[test]
fn up_and_down_move_a_highlight_through_the_rows() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));
    let first = cell_at(&app, 100, 34, "orchard  ");

    app.handle(KeyCode::Down.into());
    let moved_from = cell_at(&app, 100, 34, "orchard  ");
    let moved_to = cell_at(&app, 100, 34, "gigawatts  ");
    for _ in 0..5 {
        app.handle(KeyCode::Down.into());
    }
    let last = cell_at(&app, 100, 34, "unlogged usage  ");
    for _ in 0..5 {
        app.handle(KeyCode::Up.into());
    }
    let top = cell_at(&app, 100, 34, "orchard  ");

    let cyan = ratatui::style::Color::Cyan;
    assert_eq!(first.fg, cyan);
    assert!(!first.modifier.contains(Modifier::REVERSED));
    assert_ne!(moved_from.fg, cyan);
    assert_eq!(moved_to.fg, cyan);
    assert_eq!(last.fg, cyan);
    assert_eq!(top.fg, cyan);
}

#[test]
fn the_repo_tab_lists_each_repo_with_its_branches_under_it() {
    let mut app = open(snapshot(vec![personal()]), None);
    let wanted = app.take_history_query().unwrap();
    let work = |repo: &str, branch: &str, cost: f64| {
        row(
            PERSONAL,
            Some(&format!("/code/{repo}\u{1f}{branch}")),
            &format!("{repo} · {branch}"),
            cost,
        )
    };
    app.set_history(
        &wanted,
        Ok(spending(
            &wanted,
            vec![
                work(".dotfiles", "main", 388.0),
                work("wiki", "ab/milestone-04", 322.0),
                work("wiki", "ab/milestone-03", 118.0),
                work(".dotfiles", "ab/review", 52.0),
                row(PERSONAL, None, "unlogged usage", 20.0),
            ],
            vec![top(
                PERSONAL,
                "/code/wiki\u{1f}ab/milestone-04",
                "review",
                161.0,
            )],
        )),
    );
    app.handle(KeyCode::Enter.into());
    app.handle(KeyCode::Down.into());
    app.handle(KeyCode::Down.into());
    app.handle(KeyCode::Down.into());
    app.handle(KeyCode::Enter.into());

    let shown = screen(&app, 100, 44);

    insta::assert_snapshot!(shown);
}

#[test]
fn the_tabs_name_work_and_subagents_and_t_changes_the_time_frame() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 100, 34);

    assert!(
        shown.contains("← →  repo  skill  subagent  model"),
        "{shown}"
    );
    assert!(shown.contains("t  today  week  month  all"), "{shown}");
}

#[test]
fn the_repo_tab_shows_a_repos_top_three_branches_and_adds_up_the_rest() {
    let mut app = open(snapshot(vec![personal()]), None);
    let wanted = app.take_history_query().unwrap();
    let branches = [
        ("main", 40.0),
        ("ab/a", 30.0),
        ("ab/b", 20.0),
        ("ab/c", 6.0),
        ("ab/d", 4.0),
    ]
    .iter()
    .map(|(branch, cost)| {
        row(
            PERSONAL,
            Some(&format!("/code/wiki\u{1f}{branch}")),
            &format!("wiki · {branch}"),
            *cost,
        )
    })
    .collect();
    app.set_history(&wanted, Ok(spending(&wanted, branches, Vec::new())));
    app.handle(KeyCode::Enter.into());

    let shown = screen(&app, 100, 44);

    assert!(shown.contains("  - ab/b "), "{shown}");
    assert!(!shown.contains("ab/c"), "{shown}");
    assert!(
        shown.contains("  - 2 more, largest $6.00 ") && shown.contains("$10.00"),
        "{shown}"
    );
}

#[test]
fn repos_start_closed_and_enter_opens_and_closes_the_highlighted_one() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));
    let closed = screen(&app, 100, 34);

    app.handle(KeyCode::Enter.into());
    let opened = screen(&app, 100, 34);
    app.handle(KeyCode::Down.into());
    app.handle(KeyCode::Enter.into());
    let on_branch = screen(&app, 100, 34);
    app.update(snapshot(vec![personal()]));
    answer(&mut app);
    let refreshed = screen(&app, 100, 34);
    app.handle(KeyCode::Up.into());
    app.handle(KeyCode::Enter.into());
    let closed_again = screen(&app, 100, 34);

    assert!(closed.contains("│ ▸ orchard "), "{closed}");
    assert!(closed.contains("│ ▸ gigawatts "), "{closed}");
    assert!(!closed.contains("- ab/login"), "{closed}");
    assert!(closed.contains("│   unlogged usage "), "{closed}");
    assert!(opened.contains("│ ▾ orchard "), "{opened}");
    assert!(opened.contains("│     - ab/login "), "{opened}");
    let table = |screen: &str| -> String {
        let lines: Vec<&str> = screen.lines().collect();
        lines[..lines.len() - 1].join("\n")
    };
    assert_eq!(table(&on_branch), table(&opened));
    assert_eq!(table(&refreshed), table(&opened));
    assert_eq!(closed_again, closed);
}

#[test]
fn the_key_strip_says_what_enter_does_on_the_highlighted_row() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));
    let strip = |app: &App| screen(app, 100, 34).lines().last().unwrap().to_string();

    let on_closed = strip(&app);
    app.handle(KeyCode::Enter.into());
    let on_open = strip(&app);
    app.handle(KeyCode::Down.into());
    let on_branch = strip(&app);
    app.handle(KeyCode::Char('g').into());
    answer(&mut app);
    let on_skills = strip(&app);

    assert!(on_closed.contains("Enter branches"), "{on_closed}");
    assert!(on_open.contains("Enter hide"), "{on_open}");
    assert!(!on_branch.contains("Enter"), "{on_branch}");
    assert!(!on_skills.contains("Enter"), "{on_skills}");
}

#[test]
fn e_opens_every_repo_then_closes_them_and_keeps_the_highlight_on_its_repo() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));
    let strip = |app: &App| screen(app, 100, 34).lines().last().unwrap().to_string();
    app.handle(KeyCode::Down.into());
    let before = strip(&app);

    app.handle(KeyCode::Char('e').into());
    let expanded = screen(&app, 100, 34);
    let highlighted_open = cell_at(&app, 100, 34, "gigawatts  ");
    let after_expand = strip(&app);
    app.handle(KeyCode::Char('e').into());
    let collapsed = screen(&app, 100, 34);
    let highlighted_closed = cell_at(&app, 100, 34, "gigawatts  ");
    app.handle(KeyCode::Char('g').into());
    answer(&mut app);
    let on_skills = strip(&app);

    assert!(before.contains("e expand all"), "{before}");
    assert!(expanded.contains("▾ orchard"), "{expanded}");
    assert!(expanded.contains("▾ gigawatts"), "{expanded}");
    assert!(expanded.contains("    - ab/login "), "{expanded}");
    assert!(expanded.contains("    - main "), "{expanded}");
    assert_eq!(highlighted_open.fg, ratatui::style::Color::Cyan);
    assert!(after_expand.contains("e collapse all"), "{after_expand}");
    assert!(collapsed.contains("▸ orchard"), "{collapsed}");
    assert!(!collapsed.contains("- ab/login"), "{collapsed}");
    assert_eq!(highlighted_closed.fg, ratatui::style::Color::Cyan);
    assert!(!on_skills.contains("e expand"), "{on_skills}");
}

#[test]
fn enter_opens_the_highlighted_repo_after_switching_to_an_account_with_fewer_rows() {
    let mut app = with_spending(open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    ));
    app.handle(KeyCode::Down.into());
    app.handle(KeyCode::Down.into());

    app.handle(KeyCode::Char('2').into());
    let strip = screen(&app, 100, 34).lines().last().unwrap().to_string();
    app.handle(KeyCode::Enter.into());
    let opened = screen(&app, 100, 34);

    assert!(strip.contains("Enter branches"), "{strip}");
    assert!(opened.contains("│ ▾ wiki "), "{opened}");
}

#[test]
fn the_key_strip_keeps_a_later_key_that_fits_after_one_that_does_not() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let strip = screen(&app, 50, 60).lines().last().unwrap().to_string();

    assert!(!strip.contains("Enter branches"), "{strip}");
    assert!(strip.contains("u advice"), "{strip}");
    assert!(strip.contains("q quit"), "{strip}");
}

#[test]
fn a_notice_that_the_numbers_stopped_updating_is_red_and_a_new_account_is_not_a_warning() {
    let mut app = open(snapshot(vec![personal()]), None);
    app.set_engine_running(false);
    app.failed_to_read("disk I/O error".to_string());

    let stopped = cell_at(&app, 80, 30, "The engine stopped");
    let unreadable = cell_at(&app, 80, 30, "Can't read the history database");
    let announced = App::new(snapshot(vec![personal()]), None, Some(&BTreeSet::new()));
    let new_account = cell_at(&announced, 80, 24, "New account");

    assert_eq!(stopped.fg, ratatui::style::Color::LightRed);
    assert!(stopped.modifier.contains(Modifier::BOLD));
    assert_eq!(unreadable.fg, ratatui::style::Color::LightRed);
    assert_eq!(new_account.fg, ratatui::style::Color::Cyan);
}

#[test]
fn a_limit_with_no_pace_is_drawn_in_the_terminals_own_color_not_the_key_color() {
    let mut account = personal();
    account.limits.as_mut().unwrap().limits[0].pace = None;
    let app = open(snapshot(vec![account]), None);

    let reading = cell_at(&app, 120, 24, " 5%");

    assert_eq!(reading.fg, ratatui::style::Color::Reset);
}

#[test]
fn the_every_account_view_names_each_limits_source_the_way_one_account_does() {
    let mut app = open(snapshot(vec![personal(), work()]), None);

    app.handle(KeyCode::Char('a').into());
    let stacked = screen(&app, 80, 40);

    assert!(
        stacked.contains("Limits · from get_usage, read 3 minutes ago"),
        "{stacked}"
    );
}

#[test]
fn a_percentage_that_rounds_to_a_whole_number_shows_no_decimal() {
    let mut account = personal();
    let shown = &mut account.limits.as_mut().unwrap().limits[1];
    shown.limit.percent = 12.04;
    shown.pace.as_mut().unwrap().expected_percent = 35.04;
    let app = open(snapshot(vec![account]), None);

    let screen = screen(&app, 120, 24);

    assert!(screen.contains("   12% │"), "{screen}");
    assert!(
        screen.contains("│                     over your usual pace · 35% expected"),
        "{screen}"
    );
}

#[test]
fn at_100_columns_and_wider_limits_and_today_sit_side_by_side() {
    let app = open(snapshot(vec![personal()]), None);

    let wide = screen(&app, 120, 40);
    let narrow = screen(&app, 80, 40);

    assert!(
        wide.lines()
            .any(|line| line.contains("╭ Limits") && line.contains("╭ Today")),
        "{wide}"
    );
    assert!(
        !narrow
            .lines()
            .any(|line| line.contains("╭ Limits") && line.contains("╭ Today")),
        "{narrow}"
    );
}

#[test]
fn each_limit_gets_a_name_row_a_full_width_thick_bar_and_its_details_below() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 150, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let name_row = lines
        .iter()
        .position(|line| line.contains("│ Session "))
        .unwrap();
    let bar_cells = lines[name_row + 1]
        .chars()
        .filter(|cell| *cell == '▇')
        .count();

    assert!(bar_cells >= 60, "{shown}");
    assert!(
        lines[name_row + 2].contains("on even pace · 50% expected · resets in 2h 40m at 12:00"),
        "{shown}"
    );
}

#[test]
fn todays_spend_is_drawn_in_large_digits_above_how_it_compares_with_usual() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 120, 40);

    assert!(shown.contains("▄█▄ ▀█  █ █   ▀▀█ █▀█"), "{shown}");
    assert!(shown.contains("█    █  █ █     █ █ █"), "{shown}");
    assert!(shown.contains("▀█▄  █  ▀▀█   █▀▀ █ █"), "{shown}");
    assert!(shown.contains("  █  █    █   █   █ █"), "{shown}");
    assert!(shown.contains("▀█▀ ▄█▄   █ ▄ █▄▄ █▄█"), "{shown}");
    assert!(
        shown.contains("2.0× your usual of $7.10 by 09:20"),
        "{shown}"
    );
}

#[test]
fn on_a_weekend_the_sparkline_says_it_compares_weekend_days() {
    let mut account = personal();
    account.pace.weekend = true;
    account.pace.earlier_days_usd.truncate(6);
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 120, 40);

    assert!(shown.contains("Last 6 weekend days by 09:20"), "{shown}");
}

#[test]
fn a_sparkline_shows_the_earlier_days_by_this_clock_time_ending_with_today() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 120, 40);

    assert!(shown.contains("Last 20 workdays by 09:20"), "{shown}");
    assert!(!shown.contains("days ago"), "{shown}");
    assert!(shown.contains("▃▄▅▃▅▆▄▅▆▃▅▅▅▄▄▆▅▃▄▅█"), "{shown}");
    assert_eq!(
        last_cell_of(&app, 120, 40, "▄▅█").fg,
        ratatui::style::Color::LightRed
    );
    assert_eq!(
        cell_at(&app, 120, 40, "▃▄▅▃").fg,
        ratatui::style::Color::Cyan
    );
}

fn last_cell_of(app: &App, width: u16, height: u16, text: &str) -> ratatui::buffer::Cell {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame, NOW_MS, &utc_minute))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let joined: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(byte) = joined.find(text) {
            let x = (joined[..byte].chars().count() + text.chars().count() - 1) as u16;
            return buffer[(x, y)].clone();
        }
    }
    panic!("{text} isn't on screen");
}

#[test]
fn section_titles_are_bold_in_the_terminals_own_color_not_the_border_color() {
    let app = open(snapshot(vec![personal()]), None);

    let title = cell_at(&app, 120, 40, "Limits ·");

    assert_eq!(title.fg, ratatui::style::Color::Reset);
    assert!(title.modifier.contains(Modifier::BOLD));
}

#[test]
fn todays_large_digits_and_the_ratio_under_them_carry_the_pace_color_like_the_mock() {
    let app = open(snapshot(vec![personal()]), None);

    let digits = cell_at(&app, 120, 40, "▄█▄ ▀█");
    let times = cell_at(&app, 120, 40, "2.0×");
    let words = cell_at(&app, 120, 40, " your usual of ");
    let usual = cell_at(&app, 120, 40, "$7.10 by");

    assert_eq!(digits.fg, ratatui::style::Color::LightRed);
    assert!(digits.modifier.contains(Modifier::BOLD));
    assert_eq!(times.fg, ratatui::style::Color::LightRed);
    assert!(times.modifier.contains(Modifier::BOLD));
    assert_eq!(words.fg, ratatui::style::Color::LightRed);
    assert!(!words.modifier.contains(Modifier::BOLD));
    assert!(usual.modifier.contains(Modifier::BOLD));
}

fn gruvbox(app: &mut App) {
    app.set_palette(
        gigawatts::tui::palette::Rgb(0xeb, 0xdb, 0xb2),
        gigawatts::tui::palette::Rgb(0x28, 0x28, 0x28),
    );
}

#[test]
fn with_the_terminals_colors_known_every_muted_part_shares_one_soft_shade() {
    let mut app = open(snapshot(vec![personal()]), None);
    gruvbox(&mut app);
    let muted = ratatui::style::Color::Rgb(0x63, 0x5e, 0x51);

    let track = cell_at(&app, 150, 40, "▇ │ │");
    let border = cell_at(&app, 150, 40, "╭ Limits");
    let details = cell_at(&app, 150, 40, "on even pace · 50% expected");
    let label = cell_at(&app, 150, 40, "Spent so far");

    assert_eq!(track.fg, muted);
    assert_eq!(border.fg, muted);
    assert_eq!(details.fg, muted);
    assert_eq!(label.fg, muted);
}

#[test]
fn without_the_terminals_colors_muted_parts_fall_back_to_dark_gray() {
    let app = open(snapshot(vec![personal()]), None);

    let track = cell_at(&app, 150, 40, "▇ │ │");
    let details = cell_at(&app, 150, 40, "on even pace · 50% expected");

    assert_eq!(track.fg, ratatui::style::Color::DarkGray);
    assert_eq!(details.fg, ratatui::style::Color::DarkGray);
}

#[test]
fn each_limits_percentage_ends_its_name_row_in_bold_pace_color() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 150, 40);
    let name_row = shown
        .lines()
        .find(|line| line.contains("│ Session "))
        .unwrap();
    let reading = cell_at(&app, 150, 40, "5% │");

    assert!(name_row.contains(" 5% │"), "{shown}");
    assert_eq!(reading.fg, ratatui::style::Color::LightYellow);
    assert!(reading.modifier.contains(Modifier::BOLD));
}

#[test]
fn side_by_side_limits_and_today_leave_a_blank_row_under_their_titles() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 150, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let title_row = lines
        .iter()
        .position(|line| line.contains("╭ Limits") && line.contains("╭ Today"))
        .unwrap();
    let padding: String = lines[title_row + 1]
        .chars()
        .filter(|cell| !matches!(cell, '│' | ' ' | '"'))
        .collect();

    assert_eq!(padding, "", "{shown}");
    assert!(lines[title_row + 2].contains("│ Session "), "{shown}");
    assert!(lines[title_row + 2].contains("│ Spent so far "), "{shown}");
}

#[test]
fn side_by_side_limits_sit_one_blank_row_apart() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 150, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let row = |name: &str| lines.iter().position(|line| line.contains(name)).unwrap();

    assert_eq!(
        row("│ Weekly, all models ") - row("│ Session "),
        4,
        "{shown}"
    );
    assert_eq!(
        row("│ Weekly, Fable ") - row("│ Weekly, all models "),
        4,
        "{shown}"
    );
}

#[test]
fn todays_digits_sit_right_under_spent_so_far() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 150, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let row = |text: &str| {
        lines
            .iter()
            .position(|line| line.contains(text))
            .unwrap_or_else(|| panic!("{text} isn't on screen:\n{shown}"))
    };

    assert_eq!(row("▄█▄ ▀█  █ █") - row("Spent so far"), 1, "{shown}");
}

fn cell_after(
    app: &App,
    width: u16,
    height: u16,
    text: &str,
    offset: u16,
) -> ratatui::buffer::Cell {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame, NOW_MS, &utc_minute))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let joined: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(byte) = joined.find(text) {
            let x = joined[..byte].chars().count() as u16 + offset;
            return buffer[(x, y)].clone();
        }
    }
    panic!("{text} isn't on screen");
}

#[test]
fn a_spending_bar_fills_with_the_key_color_and_its_track_is_muted_at_the_same_height() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let last_filled = cell_after(&app, 100, 44, "$30.00 ", 8 + 21);
    let first_empty = cell_after(&app, 100, 44, "$30.00 ", 8 + 22);

    assert_eq!(last_filled.symbol(), "▇");
    assert_eq!(last_filled.fg, ratatui::style::Color::Cyan);
    assert_eq!(first_empty.symbol(), "▇");
    assert_eq!(first_empty.fg, ratatui::style::Color::DarkGray);
}

#[test]
fn spending_bars_are_a_fixed_30_cells_and_spare_width_stays_after_the_last_column() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 200, 44);
    let row = shown
        .lines()
        .find(|line| line.contains("▸ orchard"))
        .unwrap();

    let spending = &row[row.find("▸ orchard").unwrap()..];
    assert_eq!(spending.matches('▇').count(), 29, "{shown}");
    assert!(row.contains("▸ orchard             $30.00  "), "{shown}");
}

#[test]
fn a_name_or_top_item_too_long_for_its_column_ends_with_an_ellipsis() {
    let mut app = open(snapshot(vec![personal()]), None);
    app.take_history_query();
    app.handle(KeyCode::Char('g').into());
    let wanted = app.take_history_query().unwrap();
    let skill = "a-very-long-skill-name-that-runs-past-forty-columns";
    app.set_history(
        &wanted,
        Ok(spending(
            &wanted,
            vec![row(PERSONAL, Some(skill), skill, 10.0)],
            vec![top(
                PERSONAL,
                skill,
                "a-repository-with-a-really-long-name-indeed",
                7.0,
            )],
        )),
    );

    let shown = screen(&app, 100, 34);

    assert!(
        shown.contains("a-very-long-skill-name-that-run… "),
        "{shown}"
    );
    assert!(
        shown.contains("70% a-repository-with-a-really-…"),
        "{shown}"
    );
}

#[test]
fn the_top_skill_header_lines_up_with_its_values() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 100, 44);
    let column = |row: &str, text: &str| {
        let line = shown.lines().find(|line| line.contains(row)).unwrap();
        line[..line.find(text).unwrap()].chars().count()
    };

    assert_eq!(
        column("Top skill", "Top skill"),
        column("▸ orchard", "60% review"),
        "{shown}"
    );
}

#[test]
fn a_long_repo_and_branch_is_cut_from_the_middle() {
    let mut account = personal();
    account.active_sessions[0].repo = "inventory".to_string();
    account.active_sessions[0].branch = Some(format!(
        "cloud-agent/inv-917/{}update-dep-rails-8.1.4.1-5-action_view",
        "x".repeat(120)
    ));
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 120, 44);
    let line = shown
        .lines()
        .find(|line| line.contains("inventory · cloud-agent/"))
        .unwrap();

    assert!(line.contains("…"), "{shown}");
    assert!(line.contains("action_view │"), "{shown}");
}

#[test]
fn from_180_columns_active_sessions_and_spending_sit_side_by_side_and_below_that_they_stack() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let wide = screen(&app, 200, 44);
    let narrow = screen(&app, 150, 44);
    let side_by_side = |shown: &str| {
        shown
            .lines()
            .any(|line| line.contains("╭ Running now") && line.contains("╭ Spending"))
    };

    assert!(side_by_side(&wide), "{wide}");
    assert!(!side_by_side(&narrow), "{narrow}");
}

#[test]
fn the_spending_box_ends_after_its_rows_instead_of_running_to_the_bottom() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 150, 60);
    let lines: Vec<&str> = shown.lines().collect();
    let last_row = lines
        .iter()
        .position(|line| line.contains("unlogged usage"))
        .unwrap();

    assert!(lines[last_row + 1].contains("╰"), "{shown}");
}

#[test]
fn a_session_with_no_title_yet_is_called_untitled_with_a_short_id() {
    let mut account = personal();
    account.active_sessions[0].session = "624c5b71-1579-43af-9d9a-1882a9e5dabb".to_string();
    account.active_sessions[0].label = account.active_sessions[0].session.clone();
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 120, 44);

    assert!(shown.contains("untitled session · 624c5b71 "), "{shown}");
    assert!(!shown.contains("1882a9e5dabb"), "{shown}");
}

#[test]
fn with_sessions_beside_spending_a_200_by_40_window_keeps_the_side_by_side_top_row() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 200, 40);

    assert!(
        shown
            .lines()
            .any(|line| line.contains("╭ Limits") && line.contains("╭ Today")),
        "{shown}"
    );
    assert!(shown.contains("▄█▄ ▀█  █ █   ▀▀█ █▀█"), "{shown}");
    assert!(
        shown
            .lines()
            .any(|line| line.contains("╭ Running now") && line.contains("╭ Spending")),
        "{shown}"
    );
}

fn card_session(label: &str, today: f64, last_15_minutes: f64) -> SessionSpend {
    let mut spend = session(label, "orchard", "main", last_15_minutes, today + 1.0);
    spend.today_usd = today;
    spend
}

#[test]
fn each_running_session_is_a_card_with_its_cost_today_where_and_share_of_today() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 100, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let top = lines
        .iter()
        .position(|line| line.contains("│ Fix the login redirect "))
        .unwrap();

    assert!(
        lines[top - 2].contains("╭ Running now · 2 sessions · $8.90 of today's $14.20 ─"),
        "{shown}"
    );
    assert!(
        lines[top].contains(" $8.40 │ Write release notes "),
        "{shown}"
    );
    assert!(lines[top + 1].contains("│ orchard · ab/login "), "{shown}");
    assert!(lines[top + 2].contains("  59% of today │"), "{shown}");
}

#[test]
fn a_session_names_the_skill_or_subagent_that_spent_most_and_its_top_model_when_it_used_several() {
    let mut account = personal();
    account.active_sessions[0].top_driver = Some(Share {
        label: "review".to_string(),
        share: 0.6,
    });
    account.active_sessions[0].models = vec![
        Share {
            label: "claude-opus-5-5".to_string(),
            share: 0.8,
        },
        Share {
            label: "claude-sonnet-5-5".to_string(),
            share: 0.2,
        },
    ];
    account.active_sessions[1].top_driver = Some(Share {
        label: "code-reviewer subagent".to_string(),
        share: 1.0,
    });
    account.active_sessions[1].models = vec![Share {
        label: "claude-opus-5-5".to_string(),
        share: 1.0,
    }];
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 40);

    assert!(shown.contains("│ 60% review · 80% opus-5-5 "), "{shown}");
    assert!(shown.contains("│ 100% code-reviewer subagent "), "{shown}");
    assert!(!shown.contains("100% opus-5-5"), "{shown}");
}

#[test]
fn cards_go_by_cost_today_and_a_session_spending_fast_always_gets_one() {
    let mut account = personal();
    account.cost_usd = 40.0;
    account.active_sessions = vec![
        card_session("Session A", 9.0, 0.5),
        card_session("Session B", 8.0, 0.5),
        card_session("Session C", 7.0, 0.5),
        card_session("Session D", 6.0, 0.5),
        card_session("Session E", 0.5, 6.0),
    ];
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 34);

    assert!(shown.contains("│ Session A "), "{shown}");
    assert!(shown.contains("│ Session E "), "{shown}");
    assert!(shown.contains("▲ $24/hr now"), "{shown}");
    assert!(!shown.contains("Session D"), "{shown}");
    assert!(shown.contains("+ "), "{shown}");
}

#[test]
fn fast_sessions_beyond_the_cards_are_flagged_in_the_line_that_adds_up_the_rest() {
    let mut account = personal();
    account.cost_usd = 40.0;
    account.active_sessions = vec![
        card_session("Session A", 9.0, 6.0),
        card_session("Session B", 8.0, 6.0),
        card_session("Session C", 7.0, 6.0),
        card_session("Session D", 6.0, 6.0),
        card_session("Session E", 5.0, 6.0),
        card_session("Session F", 0.5, 0.5),
    ];
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 34);

    assert!(
        shown.contains("+ 4 more · $18.50 today · ▲ 3 at $20/hr+"),
        "{shown}"
    );
}

#[test]
fn sessions_that_do_not_fit_and_background_runs_add_up_on_one_line() {
    let mut account = personal();
    account.cost_usd = 40.0;
    let mut background = card_session("624c5b71-0000", 0.3, 0.1);
    background.label = background.session.clone();
    background.repo = "temporary".to_string();
    account.active_sessions = vec![
        card_session("Session A", 9.0, 0.5),
        card_session("Session B", 8.0, 0.5),
        card_session("Session C", 7.0, 0.5),
        background,
    ];
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 34);

    assert!(
        shown.contains("+ 1 more · $7.00 today   1 background run · $0.30 today"),
        "{shown}"
    );
    assert!(!shown.contains("untitled session"), "{shown}");
}

#[test]
fn a_background_run_spending_fast_gets_its_own_card() {
    let mut account = personal();
    account.cost_usd = 40.0;
    let mut runaway = card_session("624c5b71-0000", 5.0, 12.5);
    runaway.label = runaway.session.clone();
    runaway.repo = "temporary".to_string();
    account.active_sessions = vec![card_session("Session A", 9.0, 0.5), runaway];
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 34);

    assert!(shown.contains("untitled session · 624c5b71"), "{shown}");
    assert!(shown.contains("▲ $50/hr now"), "{shown}");
    assert!(!shown.contains("background run"), "{shown}");
}

#[test]
fn a_session_with_nothing_new_for_five_minutes_says_it_is_idle() {
    let mut account = personal();
    account.active_sessions[1].last_active_ms = NOW_MS - 9 * 60_000;
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 40);

    assert!(shown.contains("idle 9m │"), "{shown}");
    assert_eq!(shown.matches("idle").count(), 1, "{shown}");
}

#[test]
fn a_session_from_an_engine_that_did_not_record_last_activity_shows_no_idle_time() {
    let mut account = personal();
    account.active_sessions[1].last_active_ms = 0;
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 40);

    assert!(!shown.contains("idle"), "{shown}");
}

#[test]
fn with_nothing_running_the_section_says_so_in_one_line() {
    let mut account = personal();
    account.active_sessions.clear();
    let app = open(snapshot(vec![account]), None);

    let shown = screen(&app, 100, 40);

    assert!(shown.contains("╭ Running now ─"), "{shown}");
    assert!(shown.contains("Nothing in the last 15 minutes."), "{shown}");
}

#[test]
fn s_shows_every_running_session_in_the_spending_tables_place_and_esc_goes_back() {
    let mut account = personal();
    account.cost_usd = 40.0;
    account.active_sessions = (0..6)
        .map(|n| card_session(&format!("Session {n}"), 6.0 - n as f64, 0.5))
        .collect();
    let mut app = with_spending(open(snapshot(vec![account]), None));

    let before = screen(&app, 100, 40);
    app.handle(KeyCode::Char('s').into());
    let open_list = screen(&app, 100, 40);
    app.handle(KeyCode::Esc.into());
    let after = screen(&app, 100, 40);

    assert!(!before.contains("Session 5"), "{before}");
    assert!(before.contains("╭ Spending"), "{before}");
    for n in 0..6 {
        assert!(
            open_list.contains(&format!("│ Session {n} ")),
            "{open_list}"
        );
    }
    assert!(!open_list.contains("╭ Spending"), "{open_list}");
    assert!(after.contains("╭ Spending"), "{after}");
}

#[test]
fn in_80_by_24_the_sessions_list_reaches_every_session_and_background_run_with_down() {
    let mut account = personal();
    account.cost_usd = 40.0;
    let mut background = card_session("624c5b71-0000", 0.3, 0.1);
    background.label = background.session.clone();
    background.repo = "temporary".to_string();
    account.active_sessions = (0..6)
        .map(|n| card_session(&format!("Session {n}"), 6.0 - n as f64, 0.5))
        .chain([background])
        .collect();
    let mut app = with_spending(open(snapshot(vec![account]), None));

    app.handle(KeyCode::Char('s').into());
    let mut seen = screen(&app, 80, 24);
    for _ in 0..10 {
        app.handle(KeyCode::Down.into());
        seen.push_str(&screen(&app, 80, 24));
    }
    let bottom = screen(&app, 80, 24);
    app.handle(KeyCode::Up.into());
    let one_up = screen(&app, 80, 24);

    for n in 0..6 {
        assert!(seen.contains(&format!("│ Session {n} ")), "{seen}");
    }
    assert!(seen.contains("untitled session · 624c5b71"), "{seen}");
    assert!(!seen.contains("more ·"), "{seen}");
    assert_ne!(one_up, bottom, "{one_up}");
}

#[test]
fn with_the_sessions_list_open_the_spending_keys_do_nothing_and_quit_still_shows() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));
    let before = screen(&app, 100, 40);

    app.handle(KeyCode::Char('s').into());
    let open_list = screen(&app, 100, 40);
    for key in ['t', 'g', 'e'] {
        app.handle(KeyCode::Char(key).into());
    }
    app.handle(KeyCode::Esc.into());

    assert!(open_list.contains("↑ ↓ scroll   s Esc back"), "{open_list}");
    assert!(open_list.contains("q quit"), "{open_list}");
    assert_eq!(screen(&app, 100, 40), before);
}

#[test]
fn in_80_by_24_tab_does_nothing_while_the_sessions_list_is_open() {
    let mut app = with_spending(open(snapshot(vec![personal()]), None));
    let before = screen(&app, 80, 24);

    app.handle(KeyCode::Char('s').into());
    app.handle(KeyCode::Tab.into());
    app.handle(KeyCode::Esc.into());

    assert_eq!(screen(&app, 80, 24), before);
}

#[test]
fn beside_spending_the_running_now_pane_splits_where_limits_and_today_do_and_matches_spendings_height()
 {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 200, 44);
    let lines: Vec<&str> = shown.lines().collect();
    let column = |line: &str, text: &str| line[..line.find(text).unwrap()].chars().count();
    let top = lines.iter().find(|line| line.contains("╭ Today")).unwrap();
    let bottom = lines
        .iter()
        .position(|line| line.contains("╭ Running now") && line.contains("╭ Spending"))
        .unwrap();
    let closing = lines[bottom..]
        .iter()
        .find(|line| line.contains('╰'))
        .unwrap();

    assert_eq!(
        column(lines[bottom], "╭ Spending"),
        column(top, "╭ Today"),
        "{shown}"
    );
    assert_eq!(closing.matches('╰').count(), 2, "{shown}");
}

#[test]
fn the_running_now_heading_has_a_blank_row_under_it_and_its_dollars_stand_out() {
    let app = open(snapshot(vec![personal()]), None);

    let shown = screen(&app, 100, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let heading = lines
        .iter()
        .position(|line| line.contains("╭ Running now"))
        .unwrap();
    let cost = cell_at(&app, 100, 40, "$8.40 │");
    let running = cell_at(&app, 100, 40, "$8.90 of");

    assert!(
        lines[heading + 1].trim_matches(['│', ' ', '"']).is_empty(),
        "{shown}"
    );
    assert!(
        lines[heading + 2].contains("│ Fix the login redirect"),
        "{shown}"
    );
    assert_eq!(cost.fg, ratatui::style::Color::Cyan);
    assert!(cost.modifier.contains(Modifier::BOLD));
    assert_eq!(running.fg, ratatui::style::Color::Cyan);
}

#[test]
fn with_limits_and_today_side_by_side_the_spending_heading_has_a_blank_row_under_it() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 100, 44);
    let lines: Vec<&str> = shown.lines().collect();
    let heading = lines
        .iter()
        .position(|line| line.contains("╭ Spending"))
        .unwrap();

    assert!(
        lines
            .iter()
            .any(|line| line.contains("╭ Today") && line.contains("╭ Limits")),
        "{shown}"
    );
    assert!(
        lines[heading + 1].trim_matches(['│', ' ', '"']).is_empty(),
        "{shown}"
    );
    assert!(lines[heading + 2].contains("← →  repo"), "{shown}");
}

#[test]
fn with_limits_and_today_stacked_no_pane_heading_has_a_blank_row_under_it() {
    let app = with_spending(open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    ));

    let shown = screen(&app, 100, 34);
    let lines: Vec<&str> = shown.lines().collect();
    let below = |heading: &str| {
        let at = lines
            .iter()
            .position(|line| line.contains(heading))
            .unwrap();
        lines[at + 1]
    };

    assert!(
        below("╭ Running now").contains("│ Fix the login redirect"),
        "{shown}"
    );
    assert!(below("╭ Spending").contains("← →  repo"), "{shown}");
}

fn every_account_screen(width: u16, height: u16) -> String {
    let mut app = with_spending(open(snapshot(vec![personal(), work()]), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);
    screen(&app, width, height)
}

#[test]
fn with_room_the_all_view_gives_each_account_a_column_with_its_today_over_its_limits() {
    let one = screen(&open(snapshot(vec![personal()]), None), 100, 56);
    let shown = every_account_screen(100, 56);
    let lines: Vec<&str> = shown.lines().collect();
    let column = |line: &str, text: &str| line[..line.find(text).unwrap()].chars().count();
    let today = lines
        .iter()
        .position(|line| line.contains("╭ personal · today"))
        .unwrap();
    let limits = lines
        .iter()
        .position(|line| line.contains("╭ personal · limits"))
        .unwrap();
    let single_today = one.lines().find(|line| line.contains("╭ Today")).unwrap();

    assert!(
        lines[today - 1].contains(" $14.60 today across 2 accounts · estimated"),
        "{shown}"
    );
    assert!(lines[today].contains("╭ work · today"), "{shown}");
    assert_eq!(
        column(lines[today], "╭ work · today"),
        column(single_today, "╭ Today"),
        "{shown}"
    );
    assert!(lines[limits].contains("╭ work · limits"), "{shown}");
    assert!(limits > today, "{shown}");
    assert!(shown.contains("▄█▄ ▀█  █ █   ▀▀█ █▀█"), "{shown}");
    assert!(
        shown.contains("2.0× your usual of $7.10 by 09:20"),
        "{shown}"
    );
    assert!(!shown.contains("workdays by"), "{shown}");
}

#[test]
fn three_accounts_get_three_columns_from_150_columns_and_the_compact_boxes_below_that() {
    let mut third = work();
    third.account = Some(NEW.to_string());
    third.account_label = "side".to_string();
    let mut app = with_spending(open(snapshot(vec![personal(), work(), third]), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);

    let wide = screen(&app, 160, 70);
    let narrow = screen(&app, 120, 70);

    assert!(
        wide.lines().any(|line| line.contains("╭ personal · today")
            && line.contains("╭ work · today")
            && line.contains("╭ side · today")),
        "{wide}"
    );
    assert!(!narrow.contains("· today ─"), "{narrow}");
    assert!(narrow.contains("today across 3 accounts"), "{narrow}");
}

#[test]
fn with_room_each_accounts_limits_sit_side_by_side_where_limits_and_today_split() {
    let one = screen(&open(snapshot(vec![personal()]), None), 100, 56);
    let every = every_account_screen(100, 56);
    let column = |shown: &str, text: &str| {
        let line = shown.lines().find(|line| line.contains(text)).unwrap();
        line[..line.find(text).unwrap()].chars().count()
    };
    let lines: Vec<&str> = every.lines().collect();
    let heading = lines
        .iter()
        .position(|line| line.contains("╭ personal · limits from get_usage, read 3 minutes ago"))
        .unwrap();

    assert!(lines[heading].contains("╭ work · limits "), "{every}");
    assert_eq!(
        column(&every, "╭ work · limits"),
        column(&one, "╭ Today"),
        "{every}"
    );
    assert!(lines[heading + 2].contains("│ Session "), "{every}");
    assert!(every.contains("on even pace · 50% expected"), "{every}");
}

#[test]
fn the_all_view_shows_every_accounts_running_sessions_with_the_account_named() {
    let shown = every_account_screen(100, 56);

    assert!(
        shown.contains("╭ Running now · 2 sessions · $8.90 of today's $14.60"),
        "{shown}"
    );
    assert!(
        shown.contains("│ personal · orchard · ab/login "),
        "{shown}"
    );
    assert!(shown.contains("╭ Spending"), "{shown}");
}

#[test]
fn from_180_columns_the_all_view_puts_running_now_beside_spending() {
    let shown = every_account_screen(200, 50);

    assert!(
        shown
            .lines()
            .any(|line| line.contains("╭ Running now") && line.contains("╭ Spending")),
        "{shown}"
    );
}

#[test]
fn a_limits_heading_too_long_for_its_pane_keeps_the_age_and_drops_the_source() {
    let shown = every_account_screen(100, 56);

    assert!(
        shown.contains("╭ work · limits read 2 hours ago ─"),
        "{shown}"
    );
    assert!(
        shown.contains("╭ personal · limits from get_usage, read 3 minutes ago"),
        "{shown}"
    );
}

#[test]
fn a_limit_detail_longer_than_its_pane_wraps_instead_of_being_cut() {
    let mut flagged = work();
    flagged.limits.as_mut().unwrap().limits[0].limit.severity =
        Some("close to the monthly credit cap your organization's admin set".to_string());
    let mut app = with_spending(open(snapshot(vec![personal(), flagged]), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);
    let shown = screen(&app, 100, 56);

    assert!(
        shown.contains("│ 437.60 of 2200.00 USD · under even pace │"),
        "{shown}"
    );
    assert!(
        shown.contains("│ 26% expected · resets in 21d 14h "),
        "{shown}"
    );
    assert!(
        shown.contains("│ close to the monthly credit cap"),
        "{shown}"
    );
    assert!(shown.contains("organization's admin set"), "{shown}");
}

#[test]
fn a_window_wider_than_1170_columns_still_splits_limits_and_today() {
    let app = with_spending(open(snapshot(vec![personal()]), None));

    let shown = screen(&app, 1_200, 44);

    let top = shown.lines().find(|line| line.contains("╭ Today")).unwrap();
    let top = top.trim_start_matches('"');
    assert_eq!(
        top[..top.find("╭ Today").unwrap()].chars().count(),
        672 + 1,
        "{top}"
    );
}

fn three_accounts_screen(width: u16, height: u16) -> String {
    let mut side = personal();
    side.account = Some(NEW.to_string());
    side.account_label = "side".to_string();
    let mut app = with_spending(open(snapshot(vec![personal(), work(), side]), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);
    screen(&app, width, height)
}

fn line_with<'a>(shown: &'a str, text: &str) -> &'a str {
    shown.lines().find(|line| line.contains(text)).unwrap()
}

#[test]
fn stacked_limits_show_their_reset_beside_a_bar_of_at_most_60_cells() {
    let shown = three_accounts_screen(150, 50);
    let session = line_with(&shown, "Session ");

    assert!(session.contains("5%  resets 2h 40m"), "{shown}");
    let cells = session.chars().filter(|c| "▇▂".contains(*c)).count();
    assert_eq!(cells, 60, "{shown}");
}

#[test]
fn stacked_bars_draw_a_full_height_track_unless_the_reading_is_stale() {
    let shown = three_accounts_screen(150, 50);
    let fresh = line_with(&shown, "Session ");
    let stale = line_with(&shown, "Credits ");

    assert!(fresh.contains('▇'), "{shown}");
    assert!(!fresh.contains('▂'), "{shown}");
    assert!(stale.contains('▂'), "{shown}");
}

#[test]
fn stacked_limit_bars_line_up_across_accounts() {
    let shown = three_accounts_screen(80, 40);
    let percent_column = |text: &str| {
        let line = line_with(&shown, text);
        line[..line.find('%').unwrap()].chars().count()
    };

    assert_eq!(
        percent_column("Session "),
        percent_column("Credits "),
        "{shown}"
    );
    assert_eq!(
        percent_column("Weekly, all models"),
        percent_column("Credits "),
        "{shown}"
    );
}

#[test]
fn a_stacked_limit_too_narrow_for_its_status_puts_it_on_the_next_row() {
    let shown = three_accounts_screen(50, 40);

    assert!(!line_with(&shown, "Session ").contains("resets"), "{shown}");
    assert!(
        shown.contains("│                     resets 2h 40m"),
        "{shown}"
    );
}

#[test]
fn stacked_accounts_get_a_blank_row_between_them_when_the_window_has_room() {
    let roomy = three_accounts_screen(150, 50);
    let tight = three_accounts_screen(80, 40);
    let gap_before = |shown: &str, label: &str| {
        let lines: Vec<&str> = shown.lines().collect();
        let at = lines.iter().position(|line| line.contains(label)).unwrap();
        lines[at - 1]
            .trim_matches(|c| c == '"' || c == ' ')
            .is_empty()
    };

    assert!(gap_before(&roomy, "╭ work "), "{roomy}");
    assert!(gap_before(&roomy, "╭ side "), "{roomy}");
    assert!(!gap_before(&tight, "╭ work "), "{tight}");
}

#[test]
fn stacked_accounts_too_tall_for_the_window_cut_the_last_box_not_the_first() {
    let shown = three_accounts_screen(80, 26);
    let personal_box: String = shown
        .lines()
        .skip_while(|line| !line.contains("╭ personal "))
        .take_while(|line| !line.contains("╭ work "))
        .collect();

    assert!(personal_box.contains("Weekly, Fable"), "{shown}");
    assert!(personal_box.contains("$14.20  estimated"), "{shown}");
    assert!(shown.contains("╭ side "), "{shown}");
}

#[test]
fn stacked_limit_bars_leave_a_gap_above_so_bars_on_next_rows_stay_apart() {
    let stacked = three_accounts_screen(150, 50);
    let one = screen(&open(snapshot(vec![personal()]), None), 80, 24);

    for name in [
        "Session ",
        "Weekly, all models",
        "Weekly, Fable",
        "Credits ",
    ] {
        let row = line_with(&stacked, name);
        assert!(row.contains('▇'), "{stacked}");
        assert!(!row.contains(['█', '░', '▌']), "{stacked}");
    }
    assert!(line_with(&one, "Session ").contains('▇'), "{one}");
    assert!(
        !line_with(&one, "Session ").contains(['█', '░', '▌']),
        "{one}"
    );
}

fn with_a_filling_limit_screen(width: u16, height: u16) -> String {
    let mut fast = filling_fast();
    fast.account = Some(NEW.to_string());
    fast.account_label = "side".to_string();
    let mut app = with_spending(open(snapshot(vec![personal(), work(), fast]), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);
    screen(&app, width, height)
}

#[test]
fn stacked_limits_with_room_add_their_forecast_to_the_reset_in_parentheses() {
    let shown = with_a_filling_limit_screen(170, 50);
    let side: String = shown
        .lines()
        .skip_while(|line| !line.contains("╭ side "))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        line_with(&side, "Session ").contains("60%  resets 2h 40m (100% in 1h 20m)"),
        "{shown}"
    );
    assert_eq!(shown.matches("100% in 1h 20m").count(), 1, "{shown}");
}

#[test]
fn stacked_limits_without_room_put_their_forecast_on_the_next_row() {
    let shown = with_a_filling_limit_screen(80, 40);

    assert!(!shown.contains("(100% in"), "{shown}");
    assert!(
        shown.contains("│                     100% in 1h 20m "),
        "{shown}"
    );
}

#[test]
fn stacked_credits_put_their_budget_on_its_own_row_at_any_width() {
    for width in [80, 170] {
        let shown = with_a_filling_limit_screen(width, 50);

        assert!(
            line_with(&shown, "Credits ")
                .trim_end_matches(['"', '│', ' '])
                .ends_with("resets 21d 14h"),
            "{shown}"
        );
        assert!(
            shown.contains(
                "│                     $437.60 of $2,200 · $1,762 left · $82 a day to last"
            ),
            "{shown}"
        );
    }
}

#[test]
fn stacked_today_has_its_own_header_and_starts_at_the_left_edge() {
    let mut young = personal();
    young.pace.usual_days = 8;
    let mut app = open(snapshot(vec![young, work()]), None);
    app.handle(KeyCode::Char('a').into());
    let shown = screen(&app, 80, 40);
    let lines: Vec<&str> = shown.lines().collect();
    let header = lines
        .iter()
        .position(|line| line.contains("│ Today · usual from 8 of 20 days "))
        .unwrap_or_else(|| panic!("{shown}"));

    assert!(lines[header + 1].contains("│ $14.20  estimated"), "{shown}");
    let work_box = lines
        .iter()
        .position(|line| line.contains("╭ work "))
        .unwrap();
    assert!(
        lines[work_box..]
            .iter()
            .any(|line| line.contains("│ Today  ")),
        "{shown}"
    );
    assert!(!shown.contains("usual is provisional"), "{shown}");
}

#[test]
fn stacked_today_gets_a_blank_row_above_it_only_after_the_boxes_have_their_gaps() {
    let today_gap = |shown: &str| {
        let lines: Vec<&str> = shown.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.contains("│ Today"))
            .unwrap();
        lines[at - 1]
            .trim_matches(|c| c == '"' || c == ' ' || c == '│')
            .is_empty()
    };
    let box_gap = |shown: &str| {
        let lines: Vec<&str> = shown.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.contains("╭ work "))
            .unwrap();
        lines[at - 1]
            .trim_matches(|c| c == '"' || c == ' ')
            .is_empty()
    };
    let roomy = with_a_filling_limit_screen(100, 58);
    let tighter = with_a_filling_limit_screen(100, 56);

    assert!(today_gap(&roomy) && box_gap(&roomy), "{roomy}");
    assert!(!today_gap(&tighter) && box_gap(&tighter), "{tighter}");
}

fn reset_session_screen(width: u16, height: u16) -> String {
    let mut reset = personal();
    reset.limits.as_mut().unwrap().limits[0].limit.resets_at =
        Some("2026-11-09T09:00:00Z".to_string());
    let mut app = open(snapshot(vec![reset, work()]), None);
    app.handle(KeyCode::Char('a').into());
    screen(&app, width, height)
}

#[test]
fn a_stacked_limit_that_reset_lines_up_with_the_limits_around_it() {
    for width in [80, 150] {
        let shown = reset_session_screen(width, 40);
        let column = |name: &str, reading: &str| {
            let line = line_with(&shown, name);
            line[..line.find(reading).unwrap()].chars().count()
        };

        assert_eq!(
            column("Session ", "reset") + "reset".len(),
            column("Weekly, all models", "%") + 1,
            "{shown}"
        );
        let cells = line_with(&shown, "Session ")
            .chars()
            .filter(|c| "▇▂".contains(*c))
            .count();
        assert!(cells <= 60, "{shown}");
    }
}

fn bar_cells(app: &App, width: u16, height: u16, name: &str) -> Vec<ratatui::buffer::Cell> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame, NOW_MS, &utc_minute))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let y = (0..buffer.area.height)
        .find(|y| {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, *y)].symbol())
                .collect();
            row.contains(name)
        })
        .unwrap();
    (22..buffer.area.width)
        .map(|x| buffer[(x, y)].clone())
        .take_while(|cell| cell.symbol() != " ")
        .collect()
}

#[test]
fn a_stale_stacked_bar_still_shows_how_much_is_filled() {
    let mut app = open(snapshot(vec![personal(), work()]), None);
    app.handle(KeyCode::Char('a').into());

    let cells = bar_cells(&app, 80, 40, "Credits ");
    let first = cells.first().unwrap();
    let last = cells.last().unwrap();

    assert!(
        (first.symbol(), first.fg) != (last.symbol(), last.fg),
        "{cells:?}"
    );
}

fn work_credits_screen(width: u16, change: impl FnOnce(&mut PacedLimit)) -> String {
    let mut account = work();
    change(&mut account.limits.as_mut().unwrap().limits[0]);
    let mut app = open(snapshot(vec![personal(), account]), None);
    app.handle(KeyCode::Char('a').into());
    screen(&app, width, 40)
}

#[test]
fn a_stacked_credit_budget_too_long_for_its_row_wraps() {
    let shown = work_credits_screen(50, |_| {});

    assert!(
        shown.contains("│                     $437.60 of $2,200 "),
        "{shown}"
    );
    assert!(
        shown.contains("│                     $1,762 left "),
        "{shown}"
    );
    assert!(
        shown.contains("│                     $82 a day to last "),
        "{shown}"
    );
}

#[test]
fn credits_with_no_cap_show_only_what_was_used() {
    let shown = work_credits_screen(80, |limit| {
        limit.limit.credits.as_mut().unwrap().limit = None;
    });

    assert!(
        shown.contains("│                     $437.60 used "),
        "{shown}"
    );
    assert!(!shown.contains(" left"), "{shown}");
}

#[test]
fn credits_in_another_currency_put_its_code_after_each_amount() {
    let shown = work_credits_screen(80, |limit| {
        limit.limit.credits.as_mut().unwrap().currency = "EUR".to_string();
    });

    assert!(
        shown.contains("437.60 EUR of 2,200 EUR · 1,762 EUR left"),
        "{shown}"
    );
    assert!(shown.contains("82 EUR a day to last"), "{shown}");
}

#[test]
fn credits_under_a_day_from_their_reset_leave_out_the_daily_figure() {
    let shown = work_credits_screen(80, |limit| {
        limit.pace.as_mut().unwrap().window_end_ms = NOW_MS + 12 * 3_600_000;
    });

    assert!(
        shown.contains("$437.60 of $2,200 · $1,762 left "),
        "{shown}"
    );
    assert!(!shown.contains("a day to last"), "{shown}");
}

#[test]
fn credits_over_their_cap_show_none_left_and_no_daily_figure() {
    let shown = work_credits_screen(80, |limit| {
        limit.limit.credits.as_mut().unwrap().used = 2300.0;
    });

    assert!(shown.contains("$2,300.00 of $2,200 · $0 left "), "{shown}");
    assert!(!shown.contains("a day to last"), "{shown}");
}

#[test]
fn credits_in_the_millions_group_every_three_digits() {
    let shown = work_credits_screen(80, |limit| {
        let credits = limit.limit.credits.as_mut().unwrap();
        credits.used = 1_234_567.89;
        credits.limit = Some(2_000_000.0);
    });

    assert!(
        shown.contains("$1,234,567.89 of $2,000,000 · $765,432 left"),
        "{shown}"
    );
}

#[test]
fn stacked_spacing_never_hides_a_running_session_card() {
    let mut busy = personal();
    busy.active_sessions = (0..8)
        .map(|n| session(&format!("Task {n}"), "orchard", "main", 0.1, 1.0 + n as f64))
        .collect();
    let mut side = personal();
    side.account = Some(NEW.to_string());
    side.account_label = "side".to_string();
    side.active_sessions = Vec::new();
    let mut app = with_spending(open(snapshot(vec![busy, work(), side]), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);

    let shown = screen(&app, 100, 53);
    let lines: Vec<&str> = shown.lines().collect();
    let work_box = lines
        .iter()
        .position(|line| line.contains("╭ work "))
        .unwrap();

    assert_eq!(
        shown.matches("personal · orchard · main").count(),
        4,
        "{shown}"
    );
    assert!(
        !lines[work_box - 1]
            .trim_matches(|c| c == '"' || c == ' ')
            .is_empty(),
        "{shown}"
    );
}

#[test]
fn help_explains_the_every_account_bar_too() {
    let mut app = open(snapshot(vec![personal(), work()]), None);
    app.handle(KeyCode::Char('?').into());

    let shown = screen(&app, 80, 40);

    assert!(shown.contains("▇       expected use by now"), "{shown}");
}

#[test]
fn stacked_spacing_never_hides_a_session_card_beside_the_spending_table() {
    let mut busy = personal();
    busy.active_sessions = (0..8)
        .map(|n| session(&format!("Task {n}"), "orchard", "main", 0.1, 1.0 + n as f64))
        .collect();
    let mut accounts = vec![busy, work()];
    for (id, label) in [(NEW, "side"), (PERSONAL, "spare")] {
        let mut extra = personal();
        extra.account = Some(format!("{id}-{label}"));
        extra.account_label = label.to_string();
        extra.active_sessions = Vec::new();
        accounts.push(extra);
    }
    let mut app = with_spending(open(snapshot(accounts), None));
    app.handle(KeyCode::Char('a').into());
    answer(&mut app);

    for height in 50..80 {
        let shown = screen(&app, 200, height);
        let lines: Vec<&str> = shown.lines().collect();
        let work_box = lines
            .iter()
            .position(|line| line.contains("╭ work "))
            .unwrap();
        assert!(
            !lines[work_box - 1]
                .trim_matches(|c| c == '"' || c == ' ')
                .is_empty(),
            "height {height}\n{shown}"
        );
    }
}
