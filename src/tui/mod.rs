use std::collections::BTreeSet;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::contract::{AccountToday, Snapshot};
use crate::problems;
pub mod announced;
mod sections;
pub mod terminal;

use sections::{KEY, MUTED, WARNING};

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
            KeyCode::Char('?') => self.help = !self.help,
            KeyCode::Esc if self.help => self.help = false,
            KeyCode::Esc => return Flow::Quit,
            _ if self.help => {}
            KeyCode::Char('a') if self.snapshot.accounts.len() > 1 => {
                self.stacked = !self.stacked;
            }
            KeyCode::Char(digit @ '1'..='9') => self.pick(digit),
            KeyCode::Right => self.step(1),
            KeyCode::Left => self.step(-1),
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

    fn shown(&self) -> Option<&AccountToday> {
        self.snapshot
            .accounts
            .iter()
            .find(|account| account.account == self.current)
            .or(self.snapshot.accounts.first())
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
        match (self.stacked, self.shown()) {
            (_, None) => frame.render_widget(
                Paragraph::new(
                    " No accounts yet. `gigawatts accounts` shows what Gigawatts found.",
                ),
                body,
            ),
            (true, Some(_)) => sections::every_account(frame, body, &self.snapshot, now_ms),
            (false, Some(account)) => sections::one_account(
                frame,
                body,
                account,
                &self.snapshot.problems,
                now_ms,
                local_minute,
            ),
        }
        frame.render_widget(Paragraph::new(notices), bottom);
        frame.render_widget(Paragraph::new(self.key_strip()), keys);
        if self.help {
            sections::help(frame, frame.area());
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
            self.announcements
                .iter()
                .map(|found| Line::styled(self.announcement(found), WARNING)),
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
        let mut warn = |text: String| {
            for row in sections::wrapped(&text, width.saturating_sub(1), WARNING) {
                lines.push(Line::styled(format!(" {row}"), WARNING));
            }
        };
        if let Some(error) = &self.read_error {
            warn("Showing the last numbers read.".to_string());
            warn(format!("Can't read the history database: {error}"));
        }
        if !self.engine_running {
            warn("The engine stopped, so these numbers don't update.".to_string());
            warn("Reopen `gigawatts` to start it.".to_string());
        }
        if !self.watching_snapshot {
            warn("Can't watch for new numbers, so these update once a minute.".to_string());
        }
        if !self.config_readable {
            warn("Can't read config.toml, so the view keeps the settings read before.".to_string());
            warn("`gigawatts status` shows why.".to_string());
        }
        let problems = problems::footer(&self.snapshot.problems);
        if !problems.is_empty() {
            warn(problems.trim().to_string());
        }
        lines
    }

    fn key_strip(&self) -> Line<'static> {
        let count = self.snapshot.accounts.len();
        let mut keys = Vec::new();
        if self.help {
            keys.push(("? Esc".to_string(), "close help"));
        } else if count > 1 {
            keys.push((format!("1-{}", count.min(9)), "account"));
            keys.push(("← →".to_string(), "switch"));
            keys.push((
                "a".to_string(),
                if self.stacked { "one account" } else { "all" },
            ));
        }
        if !self.help {
            keys.push(("?".to_string(), "help"));
        }
        keys.push(("q".to_string(), "quit"));
        let mut spans = vec![Span::raw(" ")];
        for (key, what) in keys {
            spans.push(Span::styled(key, KEY.add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!(" {what}   "), MUTED));
        }
        Line::from(spans)
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
        Span::styled(text, Style::new())
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
