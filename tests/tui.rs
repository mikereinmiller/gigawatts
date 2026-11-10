use std::collections::BTreeSet;

use gigawatts::contract::{
    AccountLimits, AccountToday, Credits, DataResponse, DataRow, DollarPace, GroupBy, Limit,
    LimitPace, LimitSource, PaceBasis, PaceState, PacedLimit, Problem, ProblemKind, Range,
    SCHEMA_VERSION, SessionSpend, Snapshot,
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
fn the_fastest_session_is_bold_and_the_others_are_not() {
    let app = open(snapshot(vec![personal()]), None);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    terminal
        .draw(|frame| app.draw(frame, NOW_MS, &utc_minute))
        .unwrap();

    let buffer = terminal.backend().buffer();
    let row = |text: &str| {
        (0..buffer.area.height)
            .find(|&y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains(text)
            })
            .unwrap()
    };
    let bold = Modifier::BOLD;
    assert!(
        buffer[(2, row("Fix the login redirect"))]
            .modifier
            .contains(bold)
    );
    assert!(
        !buffer[(2, row("Write release notes"))]
            .modifier
            .contains(bold)
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
        shown.contains("63%  over your usual pace · 50% expected · resets in 2d 17h · warning "),
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
    assert!(stacked.contains("437.60 of 2200.00 USD"), "{stacked}");
    assert!(!stacked.contains("resets in"), "{stacked}");
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
        stacked.find("limits: refresh failing") < stacked.find("Limits from cachedUsage"),
        "{stacked}"
    );
    assert!(
        stacked.find("Limits from cachedUsage") < stacked.find("limits from /home/x"),
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
        shown.contains("5%  on even pace · 50% expected · resets in 2h 40m"),
        "{shown}"
    );
    assert!(
        shown.contains("63%  over your usual pace · 50% expected · resets in 2d 17h"),
        "{shown}"
    );
}

#[test]
fn a_limit_is_colored_by_its_pace() {
    let app = open(snapshot(vec![personal()]), None);

    let even = cell_at(&app, 120, 24, " 5%");
    let over = cell_at(&app, 120, 24, "63%");
    let under = cell_at(&app, 120, 24, "20%");

    assert_eq!(even.fg, ratatui::style::Color::Yellow);
    assert_eq!(over.fg, ratatui::style::Color::Red);
    assert_eq!(under.fg, ratatui::style::Color::Green);
}

#[test]
fn spend_today_is_even_within_a_tenth_of_usual_and_colored_like_limit_pace() {
    let color_at = |today_usd: f64| {
        let mut account = personal();
        account.pace.today_usd = today_usd;
        cell_at(&open(snapshot(vec![account]), None), 80, 24, "× your usual").fg
    };

    assert_eq!(color_at(6.0), ratatui::style::Color::Green);
    assert_eq!(color_at(6.5), ratatui::style::Color::Yellow);
    assert_eq!(color_at(7.1), ratatui::style::Color::Yellow);
    assert_eq!(color_at(7.8), ratatui::style::Color::Yellow);
    assert_eq!(color_at(8.0), ratatui::style::Color::Red);
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
fn active_sessions_have_column_headings() {
    let shown = screen(&open(snapshot(vec![personal()]), None), 100, 24);

    assert!(shown.contains("Session"), "{shown}");
    assert!(shown.contains("Repo · branch"), "{shown}");
    assert!(shown.contains("Last 15 min"), "{shown}");
    assert!(shown.contains("Total"), "{shown}");
    assert!(shown.contains("orchard · ab/login"), "{shown}");
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
    assert!(shown.contains("Active sessions"), "{shown}");
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
        shown.contains("$30.00 ━━━━━━━──   75%  60% review"),
        "{shown}"
    );
    assert!(
        shown.contains("$6.00 ━────────   15%  50% no skill"),
        "{shown}"
    );
    assert!(shown.contains("$4.00 ━────────   10%      "), "{shown}");
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

    assert!(first.modifier.contains(Modifier::REVERSED));
    assert!(!moved_from.modifier.contains(Modifier::REVERSED));
    assert!(moved_to.modifier.contains(Modifier::REVERSED));
    assert!(last.modifier.contains(Modifier::REVERSED));
    assert!(top.modifier.contains(Modifier::REVERSED));
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

    let shown = screen(&app, 100, 34);

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

    let shown = screen(&app, 100, 34);

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
    assert!(highlighted_open.modifier.contains(Modifier::REVERSED));
    assert!(after_expand.contains("e collapse all"), "{after_expand}");
    assert!(collapsed.contains("▸ orchard"), "{collapsed}");
    assert!(!collapsed.contains("- ab/login"), "{collapsed}");
    assert!(highlighted_closed.modifier.contains(Modifier::REVERSED));
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
