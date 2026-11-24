use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::contract::{AccountToday, SessionSpend};
use crate::pace::FASTEST_WITHIN_MS;

use super::sections::{clip, clip_middle, titled_section};
use super::style::{BAR, KEY, MUTED, OVER, STALE, TITLE};

const SESSION_ROWS: u16 = 4;
const RULE_ROWS: u16 = 1;
const BORDER_ROWS: u16 = 2;
const BORDER_AND_PADDING_COLUMNS: u16 = 4;
const DIVIDER: &str = " │ ";
const TWO_COLUMNS_FROM: u16 = 72;
const THREE_COLUMNS_FROM: u16 = 146;
const BACKGROUND_REPO: &str = "temporary";
const UNTITLED_ID_CHARS: usize = 8;
const MODEL_PREFIX: &str = "claude-";
const IDLE_FROM_MS: i64 = 5 * 60_000;
const MINUTES_UNTIL_HOURS: i64 = 60;

#[derive(Clone, Copy)]
pub(crate) enum Cards {
    Fit { roomy: bool },
    Rows { rows: usize, gap: bool },
}

pub(crate) struct SessionsToday<'a> {
    sessions: Vec<Owned<'a>>,
    today_usd: f64,
}

impl<'a> SessionsToday<'a> {
    pub(crate) fn of(account: &'a AccountToday, now_ms: i64) -> SessionsToday<'a> {
        SessionsToday {
            sessions: owned(account, None, now_ms),
            today_usd: account.cost_usd,
        }
    }

    pub(crate) fn every(accounts: &'a [AccountToday], now_ms: i64) -> SessionsToday<'a> {
        SessionsToday {
            sessions: accounts
                .iter()
                .flat_map(|account| owned(account, Some(account.account_label.as_str()), now_ms))
                .collect(),
            today_usd: accounts.iter().map(|account| account.cost_usd).sum(),
        }
    }
}

fn owned<'a>(account: &'a AccountToday, owner: Option<&'a str>, now_ms: i64) -> Vec<Owned<'a>> {
    account
        .sessions_today
        .iter()
        .map(|spend| Owned {
            owner,
            spend,
            tier: tier(spend, account.fast_usd_per_hour, now_ms),
            fast_usd_per_hour: account.fast_usd_per_hour,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Fast,
    Running,
    Finished,
}

fn tier(spend: &SessionSpend, fast_usd_per_hour: f64, now_ms: i64) -> Tier {
    if spend.last_active_ms > 0 && now_ms - spend.last_active_ms > FASTEST_WITHIN_MS {
        Tier::Finished
    } else if hourly(spend) >= fast_usd_per_hour {
        Tier::Fast
    } else {
        Tier::Running
    }
}

#[derive(Clone, Copy)]
struct Owned<'a> {
    owner: Option<&'a str>,
    spend: &'a SessionSpend,
    tier: Tier,
    fast_usd_per_hour: f64,
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
        if self.shown.is_empty() {
            return BORDER_ROWS + self.gap + 1;
        }
        BORDER_ROWS + self.gap + rows_height(self.rows())
    }
}

fn rows_height(rows: u16) -> u16 {
    rows * SESSION_ROWS + rows.saturating_sub(1) * RULE_ROWS
}

fn rows_that_fit(room: u16, gap: u16) -> usize {
    let room = room.saturating_sub(BORDER_ROWS + gap);
    ((room + RULE_ROWS) / (SESSION_ROWS + RULE_ROWS)).max(1) as usize
}

fn plan<'a>(today: &SessionsToday<'a>, width: u16, room: u16, cards: Cards) -> Plan<'a> {
    let columns = columns(inner_width(width));
    let (mut titled, background): (Vec<Owned>, Vec<Owned>) = today
        .sessions
        .iter()
        .copied()
        .partition(|session| !is_background(session));
    titled.sort_by(by_tier_then_cost);
    let live = titled
        .iter()
        .filter(|session| session.tier != Tier::Finished)
        .count();
    let rows_wanted = live.div_ceil(columns).max(1);
    let (gap, most) = match cards {
        Cards::Fit { roomy } => (gap(roomy, room, rows_wanted), usize::MAX),
        Cards::Rows { rows, gap } => (u16::from(gap), rows),
    };
    let rows = rows_that_fit(room, gap).min(rows_wanted).min(most);
    let slots = (rows * columns).min(titled.len());
    let shown = titled[..slots].to_vec();
    let hidden = titled[slots..].to_vec();
    Plan {
        columns,
        gap,
        shown,
        hidden,
        background,
    }
}

fn gap(roomy: bool, room: u16, rows_wanted: usize) -> u16 {
    let with_gap = rows_that_fit(room, 1);
    if roomy && rows_wanted > with_gap && rows_that_fit(room, 0) > with_gap {
        return 0;
    }
    u16::from(roomy)
}

fn by_tier_then_cost(a: &Owned, b: &Owned) -> std::cmp::Ordering {
    a.tier
        .cmp(&b.tier)
        .then_with(|| b.spend.today_usd.total_cmp(&a.spend.today_usd))
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

pub(crate) fn dollars_an_hour(rate: f64) -> String {
    if rate.fract() == 0.0 {
        format!("${rate:.0}/hr")
    } else {
        format!("${rate:.2}/hr")
    }
}

fn hourly(session: &SessionSpend) -> f64 {
    session.last_15_minutes_usd * 4.0
}

fn is_background(session: &Owned) -> bool {
    session.spend.repo == BACKGROUND_REPO && session.tier != Tier::Fast
}

pub(crate) fn one_row_height(gap: bool) -> u16 {
    rows_reserved(1, gap)
}

pub(crate) fn rows_reserved(rows: u16, gap: bool) -> u16 {
    BORDER_ROWS + u16::from(gap) + rows_height(rows)
}

pub(crate) fn two_rows_height(today: &SessionsToday, width: u16, roomy: bool) -> u16 {
    let mut plan = plan(today, width, u16::MAX, Cards::Fit { roomy });
    plan.shown.truncate(2 * plan.columns);
    plan.height()
}

pub(crate) fn height(today: &SessionsToday, width: u16, room: u16, cards: Cards) -> u16 {
    plan(today, width, room, cards).height()
}

pub(crate) fn draw(
    frame: &mut Frame,
    area: Rect,
    today: &SessionsToday,
    now_ms: i64,
    cards: Cards,
) {
    let plan = plan(today, area.width, area.height, cards);
    let mut lines = card_lines(&plan, today, area.width, now_ms);
    let mut block = titled_section(title(today));
    if plan.has_summary() {
        let summary = summary(&plan.hidden, &plan.background);
        if plan.shown.is_empty() {
            lines.push(summary);
        } else {
            block = block.title_bottom(in_border(summary));
        }
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(crate) fn draw_list(
    frame: &mut Frame,
    area: Rect,
    today: &SessionsToday,
    now_ms: i64,
    roomy: bool,
    first_row: &Cell<usize>,
) {
    let plan = every_session(today, area.width, roomy);
    let visible_rows = rows_that_fit(area.height, plan.gap);
    let last_first_row = (plan.rows() as usize).saturating_sub(visible_rows);
    first_row.set(first_row.get().min(last_first_row));
    let skipped = first_row.get() as u16 * (SESSION_ROWS + RULE_ROWS);
    let mut lines = card_lines(&plan, today, area.width, now_ms);
    lines.drain(plan.gap as usize..(plan.gap + skipped) as usize);
    frame.render_widget(
        Paragraph::new(lines).block(titled_section(title(today))),
        area,
    );
}

fn every_session<'a>(today: &SessionsToday<'a>, width: u16, roomy: bool) -> Plan<'a> {
    let mut shown = today.sessions.clone();
    shown.sort_by(by_tier_then_cost);
    Plan {
        columns: columns(inner_width(width)),
        gap: u16::from(roomy),
        shown,
        hidden: Vec::new(),
        background: Vec::new(),
    }
}

fn card_lines(plan: &Plan, today: &SessionsToday, width: u16, now_ms: i64) -> Vec<Line<'static>> {
    let inner = inner_width(width) as usize;
    let mut lines = vec![Line::default(); plan.gap as usize];
    if plan.is_empty() {
        lines.push(Line::styled("No sessions today", MUTED));
    }
    let widths = column_widths(inner, plan.columns);
    for (row, sessions) in plan.shown.chunks(plan.columns).enumerate() {
        if row > 0 {
            lines.push(Line::styled("─".repeat(inner), MUTED));
        }
        let cells: Vec<[Line<'static>; 4]> = sessions
            .iter()
            .zip(&widths)
            .map(|(session, width)| cell(*session, today.today_usd, *width, now_ms))
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

fn title(today: &SessionsToday) -> Line<'static> {
    if today.sessions.is_empty() {
        return Line::styled(" Sessions today ", TITLE);
    }
    let running = today
        .sessions
        .iter()
        .filter(|session| session.tier != Tier::Finished)
        .count();
    Line::from(vec![
        Span::styled(" Sessions today", TITLE),
        Span::styled(format!(" · {running} running · "), MUTED),
        Span::styled(
            format!("${:.2} ", today.today_usd),
            KEY.add_modifier(Modifier::BOLD),
        ),
    ])
}

fn cell(owned: Owned, today_usd: f64, width: usize, now_ms: i64) -> [Line<'static>; 4] {
    let session = owned.spend;
    let place = match owned.owner {
        Some(owner) => format!("{owner} · {}", place(session)),
        None => place(session),
    };
    let look = Look::of(owned.tier);
    [
        title_line(session, width, look),
        Line::styled(padded(clip_middle(&place, width), width), MUTED),
        share_line(session.today_usd, today_usd, width, look),
        driver_line(owned, width, now_ms, look),
    ]
}

#[derive(Clone, Copy)]
struct Look {
    text: Style,
    key: Style,
}

impl Look {
    fn of(tier: Tier) -> Look {
        match tier {
            Tier::Finished => Look {
                text: Style::new().fg(STALE),
                key: Style::new().fg(STALE),
            },
            Tier::Fast | Tier::Running => Look {
                text: Style::new(),
                key: KEY,
            },
        }
    }
}

fn padded(text: String, width: usize) -> String {
    format!("{text:<width$}")
}

fn title_line(session: &SessionSpend, width: usize, look: Look) -> Line<'static> {
    let cost = format!("${:.2}", session.today_usd);
    let room = width.saturating_sub(cost.chars().count() + 1);
    Line::from(vec![
        Span::styled(
            padded(clip(&title_of(session), room), room + 1),
            look.text.add_modifier(Modifier::BOLD),
        ),
        Span::styled(cost, look.key.add_modifier(Modifier::BOLD)),
    ])
}

fn place(session: &SessionSpend) -> String {
    match &session.branch {
        Some(branch) => format!("{} · {branch}", session.repo),
        None => session.repo.clone(),
    }
}

fn share_line(session_usd: f64, today_usd: f64, width: usize, look: Look) -> Line<'static> {
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
        Span::styled(BAR.repeat(filled), look.key),
        Span::styled(BAR.repeat(bar - filled), MUTED),
        Span::styled(percent, look.text.add_modifier(Modifier::BOLD)),
        Span::styled(of_today, MUTED),
    ])
}

fn driver_line(owned: Owned, width: usize, now_ms: i64, look: Look) -> Line<'static> {
    let session = owned.spend;
    let idle_minutes = (now_ms - session.last_active_ms) / 60_000;
    let state = match owned.tier {
        Tier::Fast => Some(Span::styled(
            format!("▲ ${:.0}/hr now", hourly(session)),
            Style::new().fg(OVER).add_modifier(Modifier::BOLD),
        )),
        Tier::Finished => Some(Span::styled(
            format!("last active {}", ago(idle_minutes)),
            MUTED,
        )),
        Tier::Running
            if session.last_active_ms > 0 && now_ms - session.last_active_ms >= IDLE_FROM_MS =>
        {
            Some(Span::styled(format!("idle {idle_minutes}m"), MUTED))
        }
        Tier::Running => None,
    };
    let state_width = state.as_ref().map_or(0, |span| span.width() + 2);
    let room = width.saturating_sub(state_width);
    let mut spans = driver(session, room, look);
    if let Some(state) = state {
        spans.push(Span::raw("  "));
        spans.push(state);
    }
    Line::from(spans)
}

fn ago(minutes: i64) -> String {
    if minutes < MINUTES_UNTIL_HOURS {
        format!("{minutes}m ago")
    } else {
        format!("{}h ago", minutes / 60)
    }
}

fn driver(session: &SessionSpend, room: usize, look: Look) -> Vec<Span<'static>> {
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
        Span::styled(skill, look.text),
        Span::styled(model, MUTED),
        Span::raw(" ".repeat(room - used)),
    ]
}

fn in_border(line: Line<'static>) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(line.spans);
    spans.push(Span::raw(" "));
    Line::from(spans)
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
        let fast: Vec<&Owned> = hidden
            .iter()
            .filter(|session| session.tier == Tier::Fast)
            .collect();
        if let Some(lowest) = fast
            .iter()
            .map(|session| session.fast_usd_per_hour)
            .reduce(f64::min)
        {
            spans.push(Span::styled(" · ", MUTED));
            spans.push(Span::styled(
                format!("▲ {} at {}+", fast.len(), dollars_an_hour(lowest)),
                Style::new().fg(OVER).add_modifier(Modifier::BOLD),
            ));
        }
    }
    if !background.is_empty() {
        if !hidden.is_empty() {
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
