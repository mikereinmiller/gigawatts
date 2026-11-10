use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::contract::{AccountToday, Snapshot};
use crate::problems;
use crate::store::UsageAdvice;
mod advice;
pub mod announced;
mod digits;
mod every;
pub mod history;
pub mod palette;
mod sections;
mod sessions;
mod style;
pub mod terminal;

use history::{History, LoadedAdvice};
pub use history::{HistoryQuery, Spending};

pub(crate) const HISTORY_MIN_HEIGHT: u16 = 8;
const SESSIONS_BESIDE_SPENDING_FROM: u16 = 180;

use style::{DANGER, KEY, MUTED, NORMAL, WARNING};

pub struct App {
    snapshot: Snapshot,
    current: Option<String>,
    stacked: bool,
    help: bool,
    first_launch: bool,
    engine_written_at_ms: Option<i64>,
    known: BTreeSet<Option<String>>,
    announcements: Vec<Announcement>,
    read_error: Option<String>,
    engine_running: bool,
    watching_snapshot: bool,
    config_readable: bool,
    unsaved_announced: bool,
    history: History,
    history_shown: bool,
    compact: Cell<bool>,
    advice_open: bool,
    sessions_open: bool,
    sessions_first_row: Cell<usize>,
    advice_wanted: bool,
    advice: Option<LoadedAdvice>,
    palette: Option<palette::Palette>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

impl App {
    pub fn new(
        snapshot: Snapshot,
        last_used: Option<String>,
        announced: Option<&BTreeSet<String>>,
    ) -> App {
        let current = last_used
            .filter(|id| account_ids(&snapshot).contains(&Some(id.clone())))
            .or_else(|| snapshot.accounts.first().and_then(|a| a.account.clone()));
        let mut known: BTreeSet<Option<String>> =
            announced.into_iter().flatten().cloned().map(Some).collect();
        known.insert(None);
        let mut app = App {
            snapshot,
            current,
            stacked: false,
            help: false,
            first_launch: announced.is_none(),
            engine_written_at_ms: None,
            known,
            announcements: Vec::new(),
            read_error: None,
            engine_running: true,
            watching_snapshot: true,
            config_readable: true,
            unsaved_announced: false,
            history: History::new(),
            history_shown: false,
            compact: Cell::new(false),
            advice_open: false,
            sessions_open: false,
            sessions_first_row: Cell::new(0),
            advice_wanted: false,
            advice: None,
            palette: None,
        };
        let accounts = std::mem::take(&mut app.snapshot.accounts);
        app.record_found(&accounts);
        app.snapshot.accounts = accounts;
        app
    }

    pub fn update(&mut self, snapshot: Snapshot) {
        self.record_found(&snapshot.accounts);
        if !snapshot
            .accounts
            .iter()
            .any(|account| account.account == self.current)
        {
            self.current = snapshot.accounts.first().and_then(|a| a.account.clone());
        }
        if snapshot.accounts.len() < 2 {
            self.stacked = false;
        }
        self.snapshot = snapshot;
        self.read_error = None;
        self.history.reload();
        self.advice_wanted = self.advice_open;
    }

    pub fn take_history_query(&mut self) -> Option<HistoryQuery> {
        self.history.take_query()
    }

    pub fn set_history(&mut self, query: &HistoryQuery, loaded: Result<Spending, String>) {
        self.history.set(query, loaded);
    }

    pub fn take_advice_query(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.advice_wanted) {
            return None;
        }
        let account = self.shown().and_then(|account| account.account.clone());
        if account.is_none() {
            self.advice = Some(Ok(None));
        }
        account
    }

    pub fn set_advice(&mut self, loaded: Result<Option<UsageAdvice>, String>) {
        self.advice = Some(loaded);
    }

    fn history_account(&self) -> Option<&Option<String>> {
        if self.stacked {
            None
        } else {
            self.shown().map(|account| &account.account)
        }
    }

    fn record_found(&mut self, accounts: &[AccountToday]) {
        let found = newly_found(&mut self.known, accounts);
        if found.is_empty() {
            return;
        }
        self.unsaved_announced = true;
        if !std::mem::take(&mut self.first_launch) {
            self.announcements.extend(found);
        }
    }

    pub fn set_engine_written_at(&mut self, written_at_ms: Option<i64>) {
        self.engine_written_at_ms = written_at_ms;
    }

    pub fn failed_to_read(&mut self, error: String) {
        self.read_error = Some(error);
    }

    pub fn take_unsaved_announced(&mut self) -> Option<BTreeSet<String>> {
        if !std::mem::take(&mut self.unsaved_announced) {
            return None;
        }
        Some(self.known.iter().flatten().cloned().collect())
    }

    pub fn failed_to_save_announced(&mut self) {
        self.unsaved_announced = true;
    }

    pub fn set_engine_running(&mut self, running: bool) {
        self.engine_running = running;
    }

    pub fn set_watching_snapshot(&mut self, watching: bool) {
        self.watching_snapshot = watching;
    }

    pub fn set_config_readable(&mut self, readable: bool) {
        self.config_readable = readable;
    }

    pub fn handle(&mut self, key: KeyEvent) -> Flow {
        self.announcements.clear();
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl_c || key.code == KeyCode::Char('q') {
            return Flow::Quit;
        }
        match key.code {
            KeyCode::Char('?') if !self.advice_open => self.help = !self.help,
            KeyCode::Esc if self.help => self.help = false,
            KeyCode::Esc | KeyCode::Char('u') if self.advice_open => self.advice_open = false,
            _ if self.help || self.advice_open => {}
            KeyCode::Char('u') => {
                self.advice_open = true;
                self.advice_wanted = true;
                self.advice = None;
            }
            KeyCode::Esc if self.sessions_open => self.sessions_open = false,
            KeyCode::Char('s') => {
                self.sessions_open = !self.sessions_open;
                self.sessions_first_row.set(0);
            }
            KeyCode::Down if self.sessions_open => {
                self.sessions_first_row
                    .set(self.sessions_first_row.get() + 1);
            }
            KeyCode::Up if self.sessions_open => {
                let row = self.sessions_first_row.get().saturating_sub(1);
                self.sessions_first_row.set(row);
            }
            KeyCode::Right
            | KeyCode::Left
            | KeyCode::Enter
            | KeyCode::Tab
            | KeyCode::Char('g' | 'G' | 't' | 'T' | 'e')
                if self.sessions_open => {}
            KeyCode::Tab => self.history_shown = !self.history_shown,
            KeyCode::Right | KeyCode::Char('g') => self.history.next_grouping(1),
            KeyCode::Left | KeyCode::Char('G') => self.history.next_grouping(-1),
            KeyCode::Char('t') => self.history.next_range(1),
            KeyCode::Char('T') => self.history.next_range(-1),
            KeyCode::Char('e') => {
                let account = self.history_account().cloned();
                self.history.toggle_every_repo(account.as_ref());
            }
            KeyCode::Enter => {
                let account = self.history_account().cloned();
                self.history.toggle(account.as_ref());
            }
            KeyCode::Down => {
                let account = self.history_account().cloned();
                self.history.select(1, account.as_ref());
            }
            KeyCode::Up => {
                let account = self.history_account().cloned();
                self.history.select(-1, account.as_ref());
            }
            KeyCode::Char('a') if self.snapshot.accounts.len() > 1 => {
                self.stacked = !self.stacked;
            }
            KeyCode::Char(digit @ '1'..='9') => self.pick(digit),
            KeyCode::Char(']') => self.step(1),
            KeyCode::Char('[') => self.step(-1),
            _ => {}
        }
        Flow::Continue
    }

    fn pick(&mut self, digit: char) {
        let index = digit as usize - '1' as usize;
        if let Some(account) = self.snapshot.accounts.get(index) {
            self.current = account.account.clone();
            self.stacked = false;
        }
    }

    fn step(&mut self, by: isize) {
        let accounts = &self.snapshot.accounts;
        if accounts.len() < 2 {
            return;
        }
        let tabs = accounts.len() as isize + 1;
        let at = if self.stacked {
            accounts.len()
        } else {
            accounts
                .iter()
                .position(|account| account.account == self.current)
                .unwrap_or(0)
        };
        let next = (at as isize + by).rem_euclid(tabs) as usize;
        match accounts.get(next) {
            Some(account) => {
                self.current = account.account.clone();
                self.stacked = false;
            }
            None => self.stacked = true,
        }
    }

    fn running(&self) -> sessions::Running<'_> {
        match (self.stacked, self.shown()) {
            (false, Some(account)) => sessions::Running::of(account),
            _ => sessions::Running::every(&self.snapshot.accounts),
        }
    }

    fn shown(&self) -> Option<&AccountToday> {
        self.snapshot
            .accounts
            .iter()
            .find(|account| account.account == self.current)
            .or(self.snapshot.accounts.first())
    }

    pub fn set_palette(&mut self, text: palette::Rgb, background: palette::Rgb) {
        self.palette = Some(palette::Palette::between(text, background));
    }

    pub fn draw(&self, frame: &mut Frame, now_ms: i64, local_minute: &dyn Fn(i64) -> String) {
        let notices = self.notices(frame.area().width);
        let [top, body, bottom, keys] = Layout::vertical([
            Constraint::Length(2 + self.announcements.len() as u16),
            Constraint::Fill(1),
            Constraint::Length(notices.len() as u16),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        frame.render_widget(Paragraph::new(self.header(now_ms, top.width)), top);
        let sessions_open = self.sessions_open && self.shown().is_some();
        let sessions_beside =
            !sessions_open && self.shown().is_some() && body.width >= SESSIONS_BESIDE_SPENDING_FROM;
        let top_height = match (self.stacked, self.shown()) {
            (_, None) => 1,
            (true, Some(_)) => every::height(
                &self.snapshot,
                now_ms,
                local_minute,
                body.width,
                body.height,
                !sessions_beside && !sessions_open,
            ),
            (false, Some(account)) => sections::one_account_height(
                account,
                &self.snapshot.problems,
                now_ms,
                local_minute,
                body.width,
                body.height,
                !sessions_beside && !sessions_open,
            ),
        };
        let compact = body.height < top_height + HISTORY_MIN_HEIGHT;
        self.compact.set(compact);
        let roomy = match (self.stacked, self.shown()) {
            (false, Some(account)) => {
                !compact
                    && sections::roomy(
                        account,
                        &self.snapshot.problems,
                        now_ms,
                        local_minute,
                        body.width,
                        body.height,
                        !sessions_beside && !sessions_open,
                    )
            }
            (true, Some(_)) => {
                !compact
                    && every::fit(
                        &self.snapshot,
                        now_ms,
                        local_minute,
                        body.width,
                        body.height,
                        !sessions_beside && !sessions_open,
                    )
                    .roomy
            }
            _ => false,
        };
        let sessions_beside = sessions_beside && !compact;
        let (overview, bottom_row) = match (compact, self.history_shown || sessions_open) {
            (false, _) => {
                let [overview, rest] =
                    Layout::vertical([Constraint::Length(top_height), Constraint::Fill(1)])
                        .areas(body);
                (Some(overview), Some(rest))
            }
            (true, false) => (Some(body), None),
            (true, true) => (None, Some(body)),
        };
        if let Some(overview) = overview {
            self.draw_overview(
                frame,
                overview,
                body.height,
                now_ms,
                local_minute,
                !sessions_beside && !sessions_open,
            );
        }
        if let (true, Some(rest)) = (sessions_open, bottom_row) {
            sessions::draw_list(
                frame,
                rest,
                &self.running(),
                now_ms,
                roomy,
                &self.sessions_first_row,
            );
        } else if let (Some(rest), Some(_)) = (bottom_row, self.shown()) {
            let history_height = self.history.height(self.history_account(), roomy);
            let sessions_width = sections::limits_width(rest.width);
            let height = if sessions_beside {
                history_height.max(sessions::height(
                    &self.running(),
                    sessions_width,
                    rest.height,
                    roomy,
                ))
            } else {
                history_height
            };
            let [row, _] = Layout::vertical([
                Constraint::Length(height.min(rest.height)),
                Constraint::Fill(1),
            ])
            .areas(rest);
            let history_area = if sessions_beside {
                let [sessions_area, _, history_area] = Layout::horizontal([
                    Constraint::Length(sessions_width),
                    Constraint::Length(sections::COLUMN_GAP),
                    Constraint::Fill(1),
                ])
                .areas(row);
                sessions::draw(frame, sessions_area, &self.running(), now_ms, roomy);
                history_area
            } else {
                row
            };
            let labels: HashMap<Option<String>, String> = self
                .snapshot
                .accounts
                .iter()
                .map(|account| (account.account.clone(), account.account_label.clone()))
                .collect();
            self.history
                .draw(frame, history_area, self.history_account(), &labels, roomy);
        }
        frame.render_widget(Paragraph::new(notices), bottom);
        frame.render_widget(Paragraph::new(self.key_strip(keys.width)), keys);
        if self.help {
            sections::help(frame, frame.area());
        }
        if self.advice_open {
            advice::draw(frame, frame.area(), self.advice.as_ref(), now_ms);
        }
        palette::apply(frame.buffer_mut(), self.palette.as_ref());
    }

    fn draw_overview(
        &self,
        frame: &mut Frame,
        body: Rect,
        room: u16,
        now_ms: i64,
        local_minute: &dyn Fn(i64) -> String,
        with_sessions: bool,
    ) {
        match (self.stacked, self.shown()) {
            (_, None) => frame.render_widget(
                Paragraph::new(
                    " No accounts yet. `gigawatts accounts` shows what Gigawatts found.",
                ),
                body,
            ),
            (true, Some(_)) => every::draw(
                frame,
                body,
                room,
                &self.snapshot,
                now_ms,
                local_minute,
                with_sessions,
            ),
            (false, Some(account)) => sections::one_account(
                frame,
                body,
                sections::Fit {
                    room,
                    with_sessions,
                },
                account,
                &self.snapshot.problems,
                now_ms,
                local_minute,
            ),
        }
    }

    fn header(&self, now_ms: i64, width: u16) -> Vec<Line<'static>> {
        let updated = match self.engine_written_at_ms {
            Some(written_at_ms) => {
                format!("updated {} ", crate::limits::age(now_ms - written_at_ms))
            }
            None => String::new(),
        };
        let title = " Gigawatts";
        let gap = (width as usize).saturating_sub(title.len() + updated.chars().count());
        let mut lines = vec![
            Line::from(vec![
                Span::styled(title, KEY.add_modifier(Modifier::BOLD)),
                Span::raw(" ".repeat(gap)),
                Span::styled(updated, MUTED),
            ]),
            self.account_tabs(),
        ];
        lines.extend(
            self.announcements.iter().map(|found| {
                Line::styled(self.announcement(found), KEY.add_modifier(Modifier::BOLD))
            }),
        );
        lines
    }

    fn account_tabs(&self) -> Line<'static> {
        let accounts = &self.snapshot.accounts;
        if accounts.len() == 1 {
            return Line::styled(format!(" {}", accounts[0].account_label), Modifier::BOLD);
        }
        let mut spans = vec![Span::raw(" ")];
        for (index, account) in accounts.iter().enumerate() {
            let number = if index < 9 {
                format!("{} ", index + 1)
            } else {
                String::new()
            };
            let text = format!(" {number}{} ", account.account_label);
            let on = !self.stacked && account.account == self.current;
            spans.push(tab(text, on));
            spans.push(Span::raw(" "));
        }
        if accounts.len() > 1 {
            spans.push(tab(" a all ".to_string(), self.stacked));
        }
        Line::from(spans)
    }

    fn notices(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut notice = |text: String, style: Style| {
            for row in sections::wrapped(&text, width.saturating_sub(1), style) {
                lines.push(Line::styled(format!(" {row}"), style));
            }
        };
        if let Some(error) = &self.read_error {
            notice("Showing the last numbers read.".to_string(), DANGER);
            notice(format!("Can't read the history database: {error}"), DANGER);
        }
        if !self.engine_running {
            notice(
                "The engine stopped, so these numbers don't update.".to_string(),
                DANGER,
            );
            notice("Reopen `gigawatts` to start it.".to_string(), DANGER);
        }
        if !self.watching_snapshot {
            notice(
                "Can't watch for new numbers, so these update once a minute.".to_string(),
                WARNING,
            );
        }
        if !self.config_readable {
            notice(
                "Can't read config.toml, so the view keeps the settings read before.".to_string(),
                WARNING,
            );
            notice("`gigawatts status` shows why.".to_string(), WARNING);
        }
        let problems = problems::footer(&self.snapshot.problems);
        if !problems.is_empty() {
            notice(problems.trim().to_string(), WARNING);
        }
        lines
    }

    fn key_strip(&self, width: u16) -> Line<'static> {
        let count = self.snapshot.accounts.len();
        let mut keys = Vec::new();
        let mut last = Vec::new();
        if self.help {
            keys.push(("? Esc".to_string(), "close help"));
        } else if self.advice_open {
            keys.push(("u Esc".to_string(), "close advice"));
        } else {
            if count > 1 {
                keys.push((format!("1-{}", count.min(9)), "account"));
                keys.push(("[ ]".to_string(), "switch"));
                keys.push((
                    "a".to_string(),
                    if self.stacked { "one account" } else { "all" },
                ));
            }
            if self.compact.get() && self.history_shown {
                keys.splice(0..0, self.history_keys());
            } else {
                keys.extend(self.history_keys());
            }
            last.push(("?".to_string(), "help"));
        }
        last.push(("q".to_string(), "quit"));
        let entry = |(key, what): &(String, &str)| key.chars().count() + what.chars().count() + 4;
        let gap_after_last_key = 3;
        let mut room = (width as usize + gap_after_last_key)
            .saturating_sub(1 + last.iter().map(entry).sum::<usize>());
        let mut shown = Vec::new();
        for item in keys {
            if entry(&item) > room {
                continue;
            }
            room -= entry(&item);
            shown.push(item);
        }
        shown.extend(last);
        let mut spans = vec![Span::raw(" ")];
        for (key, what) in shown {
            spans.push(Span::styled(key, KEY.add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!(" {what}   "), MUTED));
        }
        Line::from(spans)
    }

    fn history_keys(&self) -> Vec<(String, &'static str)> {
        if self.shown().is_none() {
            return Vec::new();
        }
        let compact = self.compact.get();
        if self.sessions_open {
            return vec![("↑ ↓".to_string(), "scroll"), ("s Esc".to_string(), "back")];
        }
        if compact && !self.history_shown {
            return vec![
                ("Tab".to_string(), "spending"),
                ("u".to_string(), "advice"),
                ("s".to_string(), "sessions"),
            ];
        }
        let mut keys = Vec::new();
        if compact {
            keys.push(("Tab".to_string(), "limits"));
        }
        keys.push(("← →".to_string(), "group"));
        keys.push(("t".to_string(), "time"));
        if let Some(does) = self.history.enter_does(self.history_account()) {
            keys.push(("Enter".to_string(), does));
        }
        keys.push(("u".to_string(), "advice"));
        keys.push(("s".to_string(), "sessions"));
        if let Some(does) = self.history.every_repo_does(self.history_account()) {
            keys.push(("e".to_string(), does));
        }
        keys
    }

    fn announcement(&self, found: &Announcement) -> String {
        let accounts = &self.snapshot.accounts;
        let on_screen = self.stacked
            || accounts.len() == 1
            || self
                .shown()
                .is_some_and(|shown| shown.account == found.account);
        let number = accounts
            .iter()
            .position(|account| account.account == found.account)
            .filter(|index| *index < 9);
        match number {
            Some(index) if !on_screen => format!(
                " New account: {}. Press {} to show it.",
                found.label,
                index + 1
            ),
            _ => format!(" New account: {}.", found.label),
        }
    }
}

fn tab(text: String, on: bool) -> Span<'static> {
    if on {
        Span::styled(text, KEY.add_modifier(Modifier::REVERSED | Modifier::BOLD))
    } else {
        Span::styled(text, NORMAL)
    }
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

struct Announcement {
    account: Option<String>,
    label: String,
}

fn newly_found(
    known: &mut BTreeSet<Option<String>>,
    accounts: &[AccountToday],
) -> Vec<Announcement> {
    accounts
        .iter()
        .filter(|account| known.insert(account.account.clone()))
        .map(|account| Announcement {
            account: account.account.clone(),
            label: account.account_label.clone(),
        })
        .collect()
}

fn account_ids(snapshot: &Snapshot) -> BTreeSet<Option<String>> {
    snapshot
        .accounts
        .iter()
        .map(|account| account.account.clone())
        .collect()
}
