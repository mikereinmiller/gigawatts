use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::contract::{AccountToday, SessionSpend};

use super::sections::{clip, clip_middle, titled_section};
use super::style::{KEY, MUTED, OVER, TITLE};

const SESSION_ROWS: u16 = 4;
const RULE_ROWS: u16 = 1;
const BORDER_ROWS: u16 = 2;
const BORDER_AND_PADDING_COLUMNS: u16 = 4;
const SUMMARY_ROWS: u16 = 1;
const DIVIDER: &str = " │ ";
const TWO_COLUMNS_FROM: u16 = 72;
const THREE_COLUMNS_FROM: u16 = 146;
const BAR: &str = "▆";
const BACKGROUND_REPO: &str = "temporary";
const UNTITLED_ID_CHARS: usize = 8;
const MODEL_PREFIX: &str = "claude-";
pub(crate) const FAST_PER_HOUR: f64 = 20.0;
const IDLE_FROM_MS: i64 = 5 * 60_000;

pub(crate) struct Running<'a> {
    sessions: Vec<Owned<'a>>,
    today_usd: f64,
}

impl<'a> Running<'a> {
    pub(crate) fn of(account: &'a AccountToday) -> Running<'a> {
        Running {
            sessions: owned(account, None),
            today_usd: account.cost_usd,
        }
    }

    pub(crate) fn every(accounts: &'a [AccountToday]) -> Running<'a> {
        Running {
            sessions: accounts
                .iter()
                .flat_map(|account| owned(account, Some(account.account_label.as_str())))
                .collect(),
            today_usd: accounts.iter().map(|account| account.cost_usd).sum(),
        }
    }
}

fn owned<'a>(account: &'a AccountToday, owner: Option<&'a str>) -> Vec<Owned<'a>> {
    account
        .active_sessions
        .iter()
        .map(|spend| Owned { owner, spend })
        .collect()
}

#[derive(Clone, Copy)]
struct Owned<'a> {
    owner: Option<&'a str>,
    spend: &'a SessionSpend,
}

struct Plan<'a> {
    columns: usize,
    gap: u16,
    shown: Vec<Owned<'a>>,
    hidden: Vec<Owned<'a>>,
    background: Vec<Owned<'a>>,
}

impl Plan<'_> {
    fn is_empty(&self) -> bool {
        self.shown.is_empty() && self.background.is_empty()
    }

    fn rows(&self) -> u16 {
        self.shown.len().div_ceil(self.columns) as u16
    }

    fn has_summary(&self) -> bool {
        !self.hidden.is_empty() || !self.background.is_empty()
    }

    fn height(&self) -> u16 {
        if self.is_empty() {
            return BORDER_ROWS + self.gap + 1;
        }
        BORDER_ROWS
            + self.gap
            + rows_height(self.rows())
            + u16::from(self.has_summary()) * SUMMARY_ROWS
    }
}

fn rows_height(rows: u16) -> u16 {
    rows * SESSION_ROWS + rows.saturating_sub(1) * RULE_ROWS
}

fn rows_that_fit(room: u16, gap: u16, summary: bool) -> usize {
    let room = room.saturating_sub(BORDER_ROWS + gap + u16::from(summary) * SUMMARY_ROWS);
    ((room + RULE_ROWS) / (SESSION_ROWS + RULE_ROWS)).max(1) as usize
}

fn plan<'a>(running: &Running<'a>, width: u16, room: u16, roomy: bool) -> Plan<'a> {
    let columns = columns(inner_width(width));
    let gap = u16::from(roomy);
    let (mut titled, background): (Vec<Owned>, Vec<Owned>) = running
        .sessions
        .iter()
        .copied()
        .partition(|session| !is_background(session.spend));
    titled.sort_by(by_cost_today);
    let everything_fits =
        rows_that_fit(room, gap, false) * columns >= titled.len() && background.is_empty();
    let rows = rows_that_fit(room, gap, !everything_fits);
    let slots = (rows * columns).min(titled.len());
    let mut shown = titled[..slots].to_vec();
    let mut hidden = titled[slots..].to_vec();
    promote_fast(&mut shown, &mut hidden);
    Plan {
        columns,
        gap,
        shown,
        hidden,
        background,
    }
}

fn by_cost_today(a: &Owned, b: &Owned) -> std::cmp::Ordering {
    b.spend.today_usd.total_cmp(&a.spend.today_usd)
}

fn promote_fast<'a>(shown: &mut [Owned<'a>], hidden: &mut Vec<Owned<'a>>) {
    while let Some(fast) = hidden.iter().position(|session| is_fast(session.spend)) {
        let Some(slow) = shown.iter().rposition(|session| !is_fast(session.spend)) else {
            break;
        };
        let promoted = hidden.remove(fast);
        hidden.push(std::mem::replace(&mut shown[slow], promoted));
    }
    shown.sort_by(by_cost_today);
    hidden.sort_by(by_cost_today);
}

fn inner_width(width: u16) -> u16 {
    width.saturating_sub(BORDER_AND_PADDING_COLUMNS)
}

fn columns(inner: u16) -> usize {
    if inner >= THREE_COLUMNS_FROM {
        3
    } else if inner >= TWO_COLUMNS_FROM {
        2
    } else {
        1
    }
}

fn hourly(session: &SessionSpend) -> f64 {
    session.last_15_minutes_usd * 4.0
}

fn is_fast(session: &SessionSpend) -> bool {
    hourly(session) >= FAST_PER_HOUR
}

fn is_background(session: &SessionSpend) -> bool {
    session.repo == BACKGROUND_REPO && !is_fast(session)
}

pub(crate) fn min_height(running: &Running, width: u16, roomy: bool) -> u16 {
    plan(running, width, 0, roomy).height()
}

pub(crate) fn height(running: &Running, width: u16, room: u16, roomy: bool) -> u16 {
    plan(running, width, room, roomy).height()
}

pub(crate) fn draw(frame: &mut Frame, area: Rect, running: &Running, now_ms: i64, roomy: bool) {
    let plan = plan(running, area.width, area.height, roomy);
    let mut lines = card_lines(&plan, running, area.width, now_ms);
    if plan.has_summary() {
        lines.push(summary(&plan.hidden, &plan.background));
    }
    frame.render_widget(
        Paragraph::new(lines).block(titled_section(title(running))),
        area,
    );
}

pub(crate) fn draw_list(
    frame: &mut Frame,
    area: Rect,
    running: &Running,
    now_ms: i64,
    roomy: bool,
    first_row: &Cell<usize>,
) {
    let plan = every_session(running, area.width, roomy);
    let visible_rows = rows_that_fit(area.height, plan.gap, false);
    let last_first_row = (plan.rows() as usize).saturating_sub(visible_rows);
    first_row.set(first_row.get().min(last_first_row));
    let skipped = first_row.get() as u16 * (SESSION_ROWS + RULE_ROWS);
    let mut lines = card_lines(&plan, running, area.width, now_ms);
    lines.drain(plan.gap as usize..(plan.gap + skipped) as usize);
    frame.render_widget(
        Paragraph::new(lines).block(titled_section(title(running))),
        area,
    );
}

fn every_session<'a>(running: &Running<'a>, width: u16, roomy: bool) -> Plan<'a> {
    let mut shown = running.sessions.clone();
    shown.sort_by(by_cost_today);
    Plan {
        columns: columns(inner_width(width)),
        gap: u16::from(roomy),
        shown,
        hidden: Vec::new(),
        background: Vec::new(),
    }
}

fn card_lines(plan: &Plan, running: &Running, width: u16, now_ms: i64) -> Vec<Line<'static>> {
    let inner = inner_width(width) as usize;
    let mut lines = vec![Line::default(); plan.gap as usize];
    if plan.is_empty() {
        lines.push(Line::styled("Nothing in the last 15 minutes.", MUTED));
    }
    let widths = column_widths(inner, plan.columns);
    for (row, sessions) in plan.shown.chunks(plan.columns).enumerate() {
        if row > 0 {
            lines.push(Line::styled("─".repeat(inner), MUTED));
        }
        let cells: Vec<[Line<'static>; 4]> = sessions
            .iter()
            .zip(&widths)
            .map(|(session, width)| cell(*session, running.today_usd, *width, now_ms))
            .collect();
        for index in 0..SESSION_ROWS as usize {
            let mut spans = Vec::new();
            for (column, cell) in cells.iter().enumerate() {
                if column > 0 {
                    spans.push(Span::styled(DIVIDER, MUTED));
                }
                spans.extend(cell[index].spans.iter().cloned());
            }
            lines.push(Line::from(spans));
        }
    }
    lines
}

fn column_widths(inner: usize, columns: usize) -> Vec<usize> {
    let room = inner.saturating_sub(DIVIDER.chars().count() * (columns - 1));
    let mut widths = vec![room / columns; columns];
    widths[columns - 1] += room % columns;
    widths
}

fn title(running: &Running) -> Line<'static> {
    let sessions = &running.sessions;
    if sessions.is_empty() {
        return Line::styled(" Running now ", TITLE);
    }
    let spent: f64 = sessions.iter().map(|session| session.spend.today_usd).sum();
    Line::from(vec![
        Span::styled(" Running now", TITLE),
        Span::styled(format!(" · {} · ", count(sessions.len(), "session")), MUTED),
        Span::styled(format!("${spent:.2}"), KEY.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" of today's ${:.2} ", running.today_usd), MUTED),
    ])
}

fn cell(owned: Owned, today_usd: f64, width: usize, now_ms: i64) -> [Line<'static>; 4] {
    let session = owned.spend;
    let place = match owned.owner {
        Some(owner) => format!("{owner} · {}", place(session)),
        None => place(session),
    };
    [
        title_line(session, width),
        Line::styled(padded(clip_middle(&place, width), width), MUTED),
        share_line(session.today_usd, today_usd, width),
        driver_line(session, width, now_ms),
    ]
}

fn padded(text: String, width: usize) -> String {
    format!("{text:<width$}")
}

fn title_line(session: &SessionSpend, width: usize) -> Line<'static> {
    let cost = format!("${:.2}", session.today_usd);
    let room = width.saturating_sub(cost.chars().count() + 1);
    Line::from(vec![
        Span::styled(
            padded(clip(&title_of(session), room), room + 1),
            Modifier::BOLD,
        ),
        Span::styled(cost, KEY.add_modifier(Modifier::BOLD)),
    ])
}

fn place(session: &SessionSpend) -> String {
    match &session.branch {
        Some(branch) => format!("{} · {branch}", session.repo),
        None => session.repo.clone(),
    }
}

fn share_line(session_usd: f64, today_usd: f64, width: usize) -> Line<'static> {
    let share = if today_usd > 0.0 {
        (session_usd / today_usd).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let percent = format!("  {:.0}%", share * 100.0);
    let of_today = " of today";
    let bar = width.saturating_sub(percent.chars().count() + of_today.len());
    let filled = (share * bar as f64).round() as usize;
    Line::from(vec![
        Span::styled(BAR.repeat(filled), KEY),
        Span::styled(BAR.repeat(bar - filled), MUTED),
        Span::styled(percent, Modifier::BOLD),
        Span::styled(of_today, MUTED),
    ])
}

fn driver_line(session: &SessionSpend, width: usize, now_ms: i64) -> Line<'static> {
    let idle_ms = now_ms - session.last_active_ms;
    let state = if is_fast(session) {
        Some(Span::styled(
            format!("▲ ${:.0}/hr now", hourly(session)),
            Style::new().fg(OVER).add_modifier(Modifier::BOLD),
        ))
    } else if session.last_active_ms > 0 && idle_ms >= IDLE_FROM_MS {
        Some(Span::styled(format!("idle {}m", idle_ms / 60_000), MUTED))
    } else {
        None
    };
    let state_width = state.as_ref().map_or(0, |span| span.width() + 2);
    let room = width.saturating_sub(state_width);
    let mut spans = driver(session, room);
    if let Some(state) = state {
        spans.push(Span::raw("  "));
        spans.push(state);
    }
    Line::from(spans)
}

fn driver(session: &SessionSpend, room: usize) -> Vec<Span<'static>> {
    let skill = session
        .top_driver
        .as_ref()
        .map(|top| format!("{:.0}% {}", top.share * 100.0, top.label))
        .unwrap_or_default();
    let model = match session.models.as_slice() {
        [top, _, ..] => {
            let name = top.label.strip_prefix(MODEL_PREFIX).unwrap_or(&top.label);
            format!(" · {:.0}% {name}", top.share * 100.0)
        }
        _ => String::new(),
    };
    let skill = clip(&skill, room);
    let model = clip(&model, room - skill.chars().count());
    let used = skill.chars().count() + model.chars().count();
    vec![
        Span::raw(skill),
        Span::styled(model, MUTED),
        Span::raw(" ".repeat(room - used)),
    ]
}

fn summary(hidden: &[Owned], background: &[Owned]) -> Line<'static> {
    let today = |sessions: &[Owned]| -> f64 {
        sessions.iter().map(|session| session.spend.today_usd).sum()
    };
    let mut spans = Vec::new();
    if !hidden.is_empty() {
        spans.push(Span::styled(
            format!("+ {} more · ${:.2} today", hidden.len(), today(hidden)),
            MUTED,
        ));
        let fast = hidden
            .iter()
            .filter(|session| is_fast(session.spend))
            .count();
        if fast > 0 {
            spans.push(Span::styled(" · ", MUTED));
            spans.push(Span::styled(
                format!("▲ {fast} at ${FAST_PER_HOUR:.0}/hr+"),
                Style::new().fg(OVER).add_modifier(Modifier::BOLD),
            ));
        }
    }
    if !background.is_empty() {
        if !spans.is_empty() {
            spans.push(Span::styled("   ", MUTED));
        }
        spans.push(Span::styled(
            format!(
                "{} · ${:.2} today",
                count(background.len(), "background run"),
                today(background)
            ),
            MUTED,
        ));
    }
    Line::from(spans)
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn title_of(session: &SessionSpend) -> String {
    if session.label == session.session {
        let short: String = session.session.chars().take(UNTITLED_ID_CHARS).collect();
        format!("untitled session · {short}")
    } else {
        session.label.clone()
    }
}
