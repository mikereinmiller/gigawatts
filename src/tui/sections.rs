use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Padding, Paragraph, Row, Table};

use crate::contract::{
    AccountLimits, AccountToday, DollarPace, LimitSource, PaceState, PacedLimit, Problem,
    ProblemKind, SessionSpend, Snapshot,
};
use crate::limits::{age, credits_used, limit_name, percent, unusual_severity};
use crate::pace::USUAL_DAYS;
use crate::status::{MAY_MISS_OUTSIDE_USE_MS, has_reset, pace_phrase, resets_ms};

pub(crate) const KEY: Style = Style::new().fg(Color::Cyan);
pub(crate) const MUTED: Style = Style::new().fg(Color::DarkGray);
pub(crate) const WARNING: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
const MARKER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::DIM);
const UNDER: Color = Color::Green;
const EVEN: Color = Color::Yellow;
const OVER: Color = Color::Red;
const USUAL_EVEN_FROM: f64 = 0.9;
const USUAL_EVEN_TO: f64 = 1.1;
const NAME_WIDTH: usize = 20;
const WIDE: u16 = 100;
const BAR_WIDTH_WIDE: usize = 24;
const PERCENT_WIDTH: usize = 6;
const DETAIL_GAP: usize = 2;

type LocalMinute<'a> = &'a dyn Fn(i64) -> String;

pub(crate) fn one_account(
    frame: &mut Frame,
    area: Rect,
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
) {
    let inner_width = area.width.saturating_sub(4);
    let limits = limit_lines(account, problems, now_ms, local_minute, inner_width, false);
    let today = today_lines(account, problems, inner_width, false);
    let session_rows = match account.active_sessions.len() {
        0 => 1,
        count => count + 1,
    };
    let [limits_area, today_area, sessions_area, _] = Layout::vertical([
        Constraint::Length(limits.len() as u16 + 2),
        Constraint::Length(today.len() as u16 + 2),
        Constraint::Length(session_rows as u16 + 2),
        Constraint::Fill(1),
    ])
    .areas(area);
    let limits_title = match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            format!("Limits · {}", source_and_age(limits, now_ms))
        }
        _ => "Limits".to_string(),
    };
    frame.render_widget(
        Paragraph::new(limits).block(section(&limits_title)),
        limits_area,
    );
    frame.render_widget(Paragraph::new(today).block(section("Today")), today_area);
    sessions(frame, sessions_area, &account.active_sessions);
}

pub(crate) fn every_account(frame: &mut Frame, area: Rect, snapshot: &Snapshot, now_ms: i64) {
    let inner_width = area.width.saturating_sub(4);
    let combined: f64 = snapshot
        .accounts
        .iter()
        .map(|account| account.cost_usd)
        .sum();
    let estimated = if snapshot
        .accounts
        .iter()
        .any(|account| account.cost_estimated)
    {
        " · estimated"
    } else {
        ""
    };
    let mut blocks = Vec::new();
    for account in &snapshot.accounts {
        let mut lines = limit_lines(
            account,
            &snapshot.problems,
            now_ms,
            &|_| String::new(),
            inner_width,
            true,
        );
        lines.extend(today_lines(account, &snapshot.problems, inner_width, true));
        blocks.push((account, lines));
    }
    let mut constraints = vec![Constraint::Length(2)];
    constraints.extend(
        blocks
            .iter()
            .map(|(_, lines)| Constraint::Length(lines.len() as u16 + 2)),
    );
    constraints.push(Constraint::Fill(1));
    let areas = Layout::vertical(constraints).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" ${combined:.2}"), Modifier::BOLD),
            Span::styled(
                format!(
                    " today across {} accounts{estimated}",
                    snapshot.accounts.len()
                ),
                MUTED,
            ),
        ])),
        areas[0],
    );
    for (index, (account, lines)) in blocks.into_iter().enumerate() {
        frame.render_widget(
            Paragraph::new(lines).block(section(&account.account_label)),
            areas[index + 1],
        );
    }
}

fn section(title: &str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(MUTED)
        .padding(Padding::horizontal(1))
        .title(Span::styled(format!(" {title} "), Modifier::BOLD))
}

fn source_and_age(limits: &AccountLimits, now_ms: i64) -> String {
    let source = match limits.source {
        LimitSource::GetUsage => "get_usage",
        LimitSource::CachedUsageUtilization => "cachedUsageUtilization",
    };
    format!("from {source}, read {}", age(now_ms - limits.fetched_at_ms))
}

fn limit_lines(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    stacked: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            let stale = now_ms - limits.fetched_at_ms > MAY_MISS_OUTSIDE_USE_MS;
            if stacked {
                lines.push(Line::styled(
                    format!("Limits {}", source_and_age(limits, now_ms)),
                    MUTED,
                ));
            }
            if stale {
                lines.extend(wrapped(
                    "Read over an hour ago, so it may not include use outside Claude Code.",
                    width,
                    MUTED,
                ));
            }
            for shown in &limits.limits {
                lines.extend(limit_row(
                    shown,
                    stale,
                    stacked,
                    now_ms,
                    local_minute,
                    width,
                ));
            }
        }
        _ => lines.extend(wrapped(
            "No readings yet. The engine reads them while Claude Code is active.",
            width,
            MUTED,
        )),
    }
    for text in limit_markers(account, problems) {
        lines.extend(wrapped(&text, width, MARKER));
    }
    lines
}

pub(crate) fn wrapped(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let width = (width as usize).max(1);
    let mut rows: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match rows.last_mut() {
            Some(row) if row.chars().count() + 1 + word.chars().count() <= width => {
                row.push(' ');
                row.push_str(word);
            }
            _ => rows.push(word.to_string()),
        }
    }
    rows.into_iter()
        .map(|row| Line::styled(row, style))
        .collect()
}

fn limit_row(
    shown: &PacedLimit,
    stale: bool,
    stacked: bool,
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
) -> Vec<Line<'static>> {
    let reset_in_stack = stacked && has_reset(shown, now_ms);
    let color = if stale || reset_in_stack {
        Color::DarkGray
    } else {
        pace_color(shown)
    };
    let wide = width >= WIDE;
    let bar_width = if wide {
        BAR_WIDTH_WIDE
    } else {
        (width as usize)
            .saturating_sub(NAME_WIDTH + PERCENT_WIDTH)
            .max(8)
    };
    let mut spans = vec![Span::raw(format!(
        "{:<NAME_WIDTH$}",
        clip(&limit_name(&shown.limit), NAME_WIDTH - 1)
    ))];
    spans.extend(bar(shown, bar_width, color));
    let reading = if reset_in_stack {
        format!("{:>PERCENT_WIDTH$}", "reset")
    } else {
        format!("{:>5}%", percent(shown.limit.percent))
    };
    spans.push(Span::styled(
        reading,
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    ));
    let style = if stale { MUTED } else { Style::new() };
    let parts = detail(shown, now_ms, local_minute);
    let first_width = (width as usize).saturating_sub(NAME_WIDTH + bar_width + PERCENT_WIDTH);
    if wide && parts.join(" · ").chars().count() + DETAIL_GAP <= first_width {
        spans.push(Span::raw(" ".repeat(DETAIL_GAP)));
        spans.push(Span::styled(parts.join(" · "), style));
        return vec![Line::from(spans)];
    }
    let parts = if stacked { flags(shown, now_ms) } else { parts };
    let mut lines = vec![Line::from(spans)];
    for packed in pack(&parts, (width as usize).saturating_sub(NAME_WIDTH)) {
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(NAME_WIDTH)),
            Span::styled(packed, style),
        ]));
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

fn pack(parts: &[String], width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for part in parts {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 3 + part.chars().count() <= width => {
                line.push_str(" · ");
                line.push_str(part);
            }
            _ => lines.push(part.clone()),
        }
    }
    lines
}

fn pace_color(shown: &PacedLimit) -> Color {
    match shown.pace.as_ref().map(|pace| pace.state) {
        Some(PaceState::Under) => UNDER,
        Some(PaceState::Even) => EVEN,
        Some(PaceState::Over) => OVER,
        None => Color::Cyan,
    }
}

fn bar(shown: &PacedLimit, width: usize, color: Color) -> Vec<Span<'static>> {
    let filled = cells(shown.limit.percent, width);
    let expected = shown
        .pace
        .as_ref()
        .map(|pace| cells(pace.expected_percent, width).min(width.saturating_sub(1)));
    let mut spans = Vec::new();
    for index in 0..width {
        let (symbol, style) = if Some(index) == expected {
            ("┃", Style::new().add_modifier(Modifier::BOLD))
        } else if index < filled {
            ("━", Style::new().fg(color))
        } else {
            ("─", MUTED)
        };
        spans.push(Span::styled(symbol, style));
    }
    spans
}

fn cells(percent: f64, width: usize) -> usize {
    ((percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize
}

fn detail(shown: &PacedLimit, now_ms: i64, local_minute: LocalMinute) -> Vec<String> {
    if has_reset(shown, now_ms) {
        return vec!["reset since this reading".to_string()];
    }
    let mut parts: Vec<String> = credits_used(&shown.limit).into_iter().collect();
    if let Some(pace) = &shown.pace {
        parts.push(pace_phrase(pace));
        parts.push(format!("{}% expected", percent(pace.expected_percent)));
    }
    if let Some(resets_ms) = resets_ms(shown) {
        parts.push(resets(resets_ms, now_ms, local_minute));
    }
    parts.extend(unusual_severity(&shown.limit));
    parts
}

fn flags(shown: &PacedLimit, now_ms: i64) -> Vec<String> {
    if has_reset(shown, now_ms) {
        return Vec::new();
    }
    let provisional = shown
        .pace
        .as_ref()
        .filter(|pace| pace.provisional)
        .map(|_| "pace provisional until credit_reset is set".to_string());
    credits_used(&shown.limit)
        .into_iter()
        .chain(provisional)
        .chain(unusual_severity(&shown.limit))
        .collect()
}

fn resets(resets_ms: i64, now_ms: i64, local_minute: LocalMinute) -> String {
    let minutes = (resets_ms - now_ms).max(0) / 60_000;
    let countdown = match minutes {
        0..60 => format!("{minutes}m"),
        60..1440 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes % 1440 / 60),
    };
    let clock: String = local_minute(resets_ms).chars().skip(11).collect();
    if minutes < 1440 && !clock.is_empty() {
        format!("resets in {countdown} at {clock}")
    } else {
        format!("resets in {countdown}")
    }
}

fn today_lines(
    account: &AccountToday,
    problems: &[Problem],
    width: u16,
    stacked: bool,
) -> Vec<Line<'static>> {
    let mut spans = Vec::new();
    if stacked {
        spans.push(Span::styled(format!("{:<NAME_WIDTH$}", "Today"), MUTED));
    }
    spans.push(Span::styled(
        format!("${:.2}", account.cost_usd),
        Modifier::BOLD,
    ));
    if account.cost_estimated {
        spans.push(Span::styled("  estimated", Style::new().fg(EVEN)));
    }
    spans.push(Span::styled(" · ", MUTED));
    spans.push(against_usual(&account.pace));
    let mut lines = vec![Line::from(spans)];
    if account.pace.usual_usd.is_some() && account.pace.usual_days < USUAL_DAYS {
        let indent = if stacked { NAME_WIDTH } else { 0 };
        lines.push(Line::styled(
            format!(
                "{}usual is provisional: {} of {USUAL_DAYS} days so far",
                " ".repeat(indent),
                account.pace.usual_days
            ),
            MUTED,
        ));
    }
    for text in spend_markers(account, problems) {
        lines.extend(wrapped(&text, width, MARKER));
    }
    lines
}

fn against_usual(pace: &DollarPace) -> Span<'static> {
    let Some(usual_usd) = pace.usual_usd else {
        return Span::styled("no usual yet", MUTED);
    };
    match pace.times_usual() {
        Some(times) => Span::styled(
            format!(
                "{times:.1}× your usual of ${usual_usd:.2} by {}",
                pace.clock
            ),
            Style::new().fg(match times {
                ..USUAL_EVEN_FROM => UNDER,
                ..=USUAL_EVEN_TO => EVEN,
                _ => OVER,
            }),
        ),
        None => Span::raw(format!("usual by {} is ${usual_usd:.2}", pace.clock)),
    }
}

fn sessions(frame: &mut Frame, area: Rect, sessions: &[SessionSpend]) {
    let block = section("Active sessions · last 15 minutes");
    if sessions.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled("None in the last 15 minutes.", MUTED)).block(block),
            area,
        );
        return;
    }
    let money = |amount: f64| Cell::from(Text::from(format!("${amount:.2}")).right_aligned());
    let rows = sessions.iter().enumerate().map(|(index, session)| {
        let place = match &session.branch {
            Some(branch) => format!("{} · {branch}", session.repo),
            None => session.repo.clone(),
        };
        let fastest = index == 0;
        let row = Row::new(vec![
            Cell::from(if fastest { "▸" } else { " " }),
            Cell::from(session.label.clone()),
            Cell::from(Span::styled(place, MUTED)),
            money(session.last_15_minutes_usd),
            money(session.total_usd),
        ]);
        if fastest {
            row.style(KEY.add_modifier(Modifier::BOLD))
        } else {
            row
        }
    });
    let heading = |text: &'static str, alignment: Alignment| {
        Cell::from(Text::from(text).alignment(alignment))
    };
    let header = Row::new(vec![
        heading("", Alignment::Left),
        heading("Session", Alignment::Left),
        heading("Repo · branch", Alignment::Left),
        heading("Last 15 min", Alignment::Right),
        heading("Total", Alignment::Right),
    ])
    .style(MUTED);
    let widths = [
        Constraint::Length(1),
        Constraint::Fill(3),
        Constraint::Fill(2),
        Constraint::Length(11),
        Constraint::Length(9),
    ];
    frame.render_widget(Table::new(rows, widths).header(header).block(block), area);
}

pub(crate) fn help(frame: &mut Frame, area: Rect) {
    let key = |key: &'static str, what: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {key:<8}"), KEY.add_modifier(Modifier::BOLD)),
            Span::raw(what),
        ])
    };
    let color = |symbol: &'static str, style: Style, what: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {symbol:<8}"), style),
            Span::raw(what),
        ])
    };
    let lines = vec![
        key("1-9", "show that account"),
        key("← →", "step through the tabs, including all"),
        key("a", "show every account, or go back to one"),
        key("?", "open or close this help"),
        key("q", "quit (Esc and Ctrl-C too)"),
        Line::default(),
        color("━━━", Style::new().fg(UNDER), "under your pace"),
        color("━━━", Style::new().fg(EVEN), "on pace"),
        color("━━━", Style::new().fg(OVER), "over your pace"),
        color("━━━", MUTED, "a reading over an hour old"),
        color(
            "┃",
            Style::new().add_modifier(Modifier::BOLD),
            "expected use by now",
        ),
        color("▸", KEY, "the session spending fastest"),
    ];
    let popup = super::centered(area, 56, lines.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(section("Keys")), popup);
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
