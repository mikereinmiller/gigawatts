use std::collections::BTreeSet;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::contract::{
    AccountLimits, AccountToday, LimitSource, Problem, ProblemKind, SessionSpend, Snapshot,
};
use crate::limits::{age, credits_used, limit_name, percent, unusual_severity};
use crate::problems;
pub mod announced;
pub mod terminal;

use crate::status::{MAY_MISS_OUTSIDE_USE_MS, has_reset, pace_phrase, resets_ms, spend_today};

const SESSION_LABEL_WIDTH: usize = 28;
const SESSION_PLACE_WIDTH: usize = 24;

pub struct App {
    snapshot: Snapshot,
    current: Option<String>,
    stacked: bool,
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
    pub fn new(snapshot: Snapshot, last_used: Option<String>, announced: &BTreeSet<String>) -> App {
        let current = last_used
            .filter(|id| account_ids(&snapshot).contains(&Some(id.clone())))
            .or_else(|| snapshot.accounts.first().and_then(|a| a.account.clone()));
        let mut known: BTreeSet<Option<String>> = announced.iter().cloned().map(Some).collect();
        known.insert(None);
        let announcements = newly_found(&mut known, &snapshot.accounts);
        let unsaved_announced = !announcements.is_empty();
        App {
            snapshot,
            current,
            stacked: false,
            known,
            announcements,
            read_error: None,
            engine_running: true,
            watching_snapshot: true,
            config_readable: true,
            unsaved_announced,
        }
    }

    pub fn update(&mut self, snapshot: Snapshot) {
        let found = newly_found(&mut self.known, &snapshot.accounts);
        self.unsaved_announced |= !found.is_empty();
        self.announcements.extend(found);
        if !snapshot
            .accounts
            .iter()
            .any(|account| account.account == self.current)
        {
            self.current = snapshot.accounts.first().and_then(|a| a.account.clone());
        }
        self.snapshot = snapshot;
        self.read_error = None;
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
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Flow::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Flow::Quit;
            }
            KeyCode::Tab => self.next_account(),
            KeyCode::Char('a') => self.stacked = !self.stacked,
            _ => {}
        }
        Flow::Continue
    }

    fn next_account(&mut self) {
        let accounts = &self.snapshot.accounts;
        if accounts.is_empty() {
            return;
        }
        let next = accounts
            .iter()
            .position(|account| account.account == self.current)
            .map_or(0, |index| (index + 1) % accounts.len());
        self.current = accounts[next].account.clone();
    }

    fn shown(&self) -> Option<&AccountToday> {
        self.snapshot
            .accounts
            .iter()
            .find(|account| account.account == self.current)
            .or(self.snapshot.accounts.first())
    }

    pub fn draw(&self, frame: &mut Frame, now_ms: i64, local_minute: &dyn Fn(i64) -> String) {
        let footer = self.footer();
        let [body, bottom] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(footer.len() as u16)])
                .areas(frame.area());
        let mut lines = vec![self.header()];
        lines.extend(
            self.announcements
                .iter()
                .map(|found| Line::styled(self.announcement(found), Modifier::BOLD)),
        );
        lines.push(Line::default());
        let problems = &self.snapshot.problems;
        match (self.stacked, self.shown()) {
            (_, None) => lines.push(Line::raw(
                "No accounts yet. `gigawatts accounts` shows what Gigawatts found.",
            )),
            (true, Some(_)) => {
                for (index, account) in self.snapshot.accounts.iter().enumerate() {
                    if index > 0 {
                        lines.push(Line::default());
                    }
                    lines.push(Line::styled(account.account_label.clone(), Modifier::BOLD));
                    lines.extend(limit_lines(account, problems, now_ms, local_minute));
                    lines.extend(today_lines(account, problems));
                }
            }
            (false, Some(account)) => {
                lines.extend(limit_lines(account, problems, now_ms, local_minute));
                lines.push(Line::default());
                lines.extend(today_lines(account, problems));
                lines.push(Line::default());
                lines.extend(session_lines(&account.active_sessions));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        frame.render_widget(Paragraph::new(footer), bottom);
    }

    fn footer(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(error) = &self.read_error {
            lines.push(Line::styled(
                "Showing the last numbers read.",
                Modifier::BOLD,
            ));
            lines.push(Line::styled(
                format!("Can't read the history database: {error}"),
                Modifier::BOLD,
            ));
        }
        if !self.engine_running {
            lines.push(Line::styled(
                "The engine stopped, so these numbers don't update.",
                Modifier::BOLD,
            ));
            lines.push(Line::styled(
                "Reopen `gigawatts` to start it.",
                Modifier::BOLD,
            ));
        }
        if !self.watching_snapshot {
            lines.push(Line::styled(
                "Can't watch for new numbers, so these update once a minute.",
                Modifier::BOLD,
            ));
        }
        if !self.config_readable {
            lines.push(Line::styled(
                "Can't read config.toml, so the view keeps the settings read before.",
                Modifier::BOLD,
            ));
            lines.push(Line::styled(
                "`gigawatts status` shows why.",
                Modifier::BOLD,
            ));
        }
        let problems = problems::footer(&self.snapshot.problems);
        if !problems.is_empty() {
            lines.push(Line::styled(problems.trim().to_string(), Modifier::BOLD));
        }
        lines
    }

    fn announcement(&self, found: &Announcement) -> String {
        let on_screen = self.stacked
            || self.snapshot.accounts.len() == 1
            || self
                .shown()
                .is_some_and(|shown| shown.account == found.account);
        if on_screen {
            format!("New account: {}.", found.label)
        } else {
            format!("New account: {}. Tab cycles to it.", found.label)
        }
    }

    fn header(&self) -> Line<'static> {
        let title = match (self.stacked, self.shown()) {
            (true, _) => "Gigawatts · every account".to_string(),
            (false, Some(account)) => format!("Gigawatts · {}", account.account_label),
            (false, None) => "Gigawatts".to_string(),
        };
        let keys = match self.snapshot.accounts.len() {
            0 | 1 => "q quit",
            _ if self.stacked => "a one account · q quit",
            _ => "Tab next account · a all · q quit",
        };
        Line::from(vec![
            Span::styled(title, Modifier::BOLD),
            Span::raw("   "),
            Span::styled(keys, Modifier::DIM),
        ])
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

fn limit_lines(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: &dyn Fn(i64) -> String,
) -> Vec<Line<'static>> {
    let mut lines = match &account.limits {
        Some(limits) if !limits.limits.is_empty() => readings(limits, now_ms, local_minute),
        _ => vec![Line::raw(
            "Limits: no readings yet. The engine reads them while Claude Code is active.",
        )],
    };
    lines.extend(limit_markers(account, problems).into_iter().map(marker));
    lines
}

fn readings(
    limits: &AccountLimits,
    now_ms: i64,
    local_minute: &dyn Fn(i64) -> String,
) -> Vec<Line<'static>> {
    let elapsed_ms = now_ms - limits.fetched_at_ms;
    let stale = elapsed_ms > MAY_MISS_OUTSIDE_USE_MS;
    let style = if stale {
        Style::new().add_modifier(Modifier::DIM)
    } else {
        Style::new()
    };
    let source = match limits.source {
        LimitSource::GetUsage => "get_usage",
        LimitSource::CachedUsageUtilization => "cachedUsageUtilization",
    };
    let mut heading = format!("Limits from {source}, read {}", age(elapsed_ms));
    if stale {
        heading.push_str(", may not include use outside Claude Code");
    }
    let mut lines = vec![Line::styled(heading, style)];
    for shown in &limits.limits {
        let name = limit_name(&shown.limit);
        let detail = if has_reset(shown, now_ms) {
            "reset since this reading".to_string()
        } else {
            let mut parts: Vec<String> = credits_used(&shown.limit).into_iter().collect();
            if let Some(pace) = &shown.pace {
                parts.push(pace_phrase(pace));
            }
            if let Some(resets_ms) = resets_ms(shown) {
                parts.push(format!("resets {}", local_minute(resets_ms)));
            }
            parts.extend(unusual_severity(&shown.limit));
            parts.join(", ")
        };
        lines.push(Line::styled(
            format!("  {name:<20}{:>5}%  {detail}", percent(shown.limit.percent)),
            style,
        ));
    }
    lines
}

fn today_lines(account: &AccountToday, problems: &[Problem]) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw(format!("Today {}", spend_today(account)))];
    lines.extend(spend_markers(account, problems).into_iter().map(marker));
    lines
}

fn session_lines(sessions: &[SessionSpend]) -> Vec<Line<'static>> {
    if sessions.is_empty() {
        return vec![Line::raw("No sessions active in the last 15 minutes.")];
    }
    let mut lines = vec![Line::raw("Active sessions, spend in the last 15 minutes")];
    for (index, session) in sessions.iter().enumerate() {
        let place = match &session.branch {
            Some(branch) => format!("{} · {branch}", session.repo),
            None => session.repo.clone(),
        };
        let fastest = index == 0;
        let text = format!(
            "{} {:<label$} {:<place$} {:>7}  ${:.2} total",
            if fastest { "▸" } else { " " },
            clip(&session.label, SESSION_LABEL_WIDTH),
            clip(&place, SESSION_PLACE_WIDTH),
            format!("${:.2}", session.last_15_minutes_usd),
            session.total_usd,
            label = SESSION_LABEL_WIDTH,
            place = SESSION_PLACE_WIDTH,
        );
        lines.push(if fastest {
            Line::styled(text, Modifier::BOLD)
        } else {
            Line::raw(text)
        });
    }
    lines
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width - 1).collect();
    format!("{kept}…")
}

fn marker(text: String) -> Line<'static> {
    Line::styled(format!("  {text}"), Modifier::DIM)
}

fn limit_markers(account: &AccountToday, problems: &[Problem]) -> Vec<String> {
    affecting(account, problems)
        .filter_map(|problem| match problem.kind {
            ProblemKind::RefreshFailing => Some(match &problem.since {
                Some(since) => format!(
                    "limits: refresh failing since {}",
                    since.chars().take(16).collect::<String>()
                ),
                None => "limits: refresh failing".to_string(),
            }),
            ProblemKind::ReadFailing if problem.affects.starts_with("limits") => {
                Some(problem.affects.clone())
            }
            _ => None,
        })
        .collect()
}

fn affecting<'a>(
    account: &AccountToday,
    problems: &'a [Problem],
) -> impl Iterator<Item = &'a Problem> {
    let id = account.account.clone();
    problems
        .iter()
        .filter(move |problem| problem.account.is_none() || problem.account == id)
}

fn spend_markers(account: &AccountToday, problems: &[Problem]) -> Vec<String> {
    let estimated = account
        .unpriced_models
        .iter()
        .map(|model| format!("cost leaves out {model} until its session ends: no price"));
    let adjusted = account
        .adjusted_models
        .iter()
        .map(|model| format!("cost adjusted: {model}'s prices differ from Claude Code's record"));
    let unread = affecting(account, problems)
        .filter(|problem| {
            matches!(
                problem.kind,
                ProblemKind::WatchFailing | ProblemKind::ReadFailing
            ) && problem.affects.starts_with("today's spend")
        })
        .map(|problem| problem.affects.clone());
    estimated.chain(adjusted).chain(unread).collect()
}
