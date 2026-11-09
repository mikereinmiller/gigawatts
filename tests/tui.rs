use std::collections::BTreeSet;

use gigawatts::contract::{
    AccountLimits, AccountToday, Credits, DollarPace, Limit, LimitPace, LimitSource, PaceBasis,
    PaceState, PacedLimit, Problem, ProblemKind, SCHEMA_VERSION, SessionSpend, Snapshot,
};
use gigawatts::tui::{App, Flow};
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
fn q_escape_and_ctrl_c_quit() {
    let mut app = open(snapshot(vec![personal()]), None);

    assert_eq!(app.handle(KeyCode::Char('q').into()), Flow::Quit);
    assert_eq!(app.handle(KeyCode::Esc.into()), Flow::Quit);
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
fn number_keys_pick_an_account_from_the_tab_row_and_tab_does_nothing() {
    let mut app = open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    );
    let first = screen(&app, 80, 24);

    app.handle(KeyCode::Tab.into());
    let after_tab = screen(&app, 80, 24);
    app.handle(KeyCode::Char('9').into());
    let after_nine = screen(&app, 80, 24);
    app.handle(KeyCode::Char('2').into());
    let second = screen(&app, 80, 24);

    assert!(first.contains("1 personal"), "{first}");
    assert!(first.contains("2 work"), "{first}");
    assert!(first.contains("Weekly, Fable"), "{first}");
    assert_eq!(after_tab, first);
    assert_eq!(after_nine, first);
    assert!(second.contains("Credits"), "{second}");
    assert!(!second.contains("Weekly, Fable"), "{second}");
}

#[test]
fn arrow_keys_step_through_the_tabs_and_wrap_past_all() {
    let mut app = open(
        snapshot(vec![personal(), work()]),
        Some(PERSONAL.to_string()),
    );
    let mut after = |key: KeyCode| {
        app.handle(key.into());
        screen(&app, 80, 24)
    };

    let work_tab = after(KeyCode::Right);
    let all_tab = after(KeyCode::Right);
    let wrapped = after(KeyCode::Right);
    let back_to_all = after(KeyCode::Left);
    let back_to_work = after(KeyCode::Left);

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
fn with_one_account_arrow_keys_do_nothing() {
    let mut app = open(snapshot(vec![personal()]), None);
    let before = screen(&app, 80, 24);

    app.handle(KeyCode::Right.into());
    app.handle(KeyCode::Left.into());

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
    assert!(both.contains("← → switch"), "{both}");
    assert!(both.contains("a all"), "{both}");
    assert!(both.contains("? help"), "{both}");
    assert!(both.contains("q quit"), "{both}");
    assert!(!one.contains("1-1"), "{one}");
    assert!(!one.contains("a all"), "{one}");
    assert!(!one.contains("← →"), "{one}");
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
