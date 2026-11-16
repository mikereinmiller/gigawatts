use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::contract::{
    AccountLimits, AccountToday, DollarPace, LimitSource, PaceState, PacedLimit, Problem,
    ProblemKind,
};
use crate::limits::{age, credits_used, limit_name, percent, unusual_severity};
use crate::pace::USUAL_DAYS;
use crate::status::{MAY_MISS_OUTSIDE_USE_MS, has_reset, pace_phrase, recent_suffix, resets_ms};

use super::HISTORY_MIN_HEIGHT;
use super::digits;
use super::sessions;
use super::style::{
    BAR, EVEN, KEY, MUTED, NORMAL, OVER, PROBLEM, STALE, STALE_TRACK, TITLE, UNDER, UNPACED,
};

const USUAL_EVEN_FROM: f64 = 0.9;
const USUAL_EVEN_TO: f64 = 1.1;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const NAME_WIDTH: usize = 20;
const COLUMNS_FROM: u16 = 100;
pub(crate) const COLUMN_GAP: u16 = 1;
const PERCENT_WIDTH: usize = 6;
const STACKED_BAR_MAX: usize = 60;
const STACKED_BAR_MIN: usize = 12;
const STATUS_GAP: usize = 2;
const STATUS_WIDTH: usize = "resets 21d 14h".len();
const SEPARATOR: &str = " · ";
const DAY_MS: i64 = 24 * 3_600_000;
const LIMITS_SHARE: u32 = 56;
const SPARK_LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SPARK_ROWS: usize = 4;
const SPARK_DAY_CELLS: usize = 2;
const SPARK_DAY_GAP: usize = 1;

pub(crate) type LocalMinute<'a> = &'a dyn Fn(i64) -> String;

pub(crate) fn roomy(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
) -> bool {
    overview_panels(
        account,
        problems,
        now_ms,
        local_minute,
        width,
        height,
        with_sessions,
    )
    .side_by_side
    .is_some()
}

pub(crate) fn one_account_height(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
) -> u16 {
    let panels = overview_panels(
        account,
        problems,
        now_ms,
        local_minute,
        width,
        height,
        with_sessions,
    );
    let panels_height = panels.height() as u16;
    let sessions = if with_sessions {
        let room = height.saturating_sub(panels_height + HISTORY_MIN_HEIGHT);
        sessions::height(
            &sessions::Running::of(account),
            width,
            room,
            panels.side_by_side.is_some(),
        )
    } else {
        0
    };
    panels_height + sessions
}

struct Panels {
    limits: Vec<Line<'static>>,
    today: Vec<Line<'static>>,
    side_by_side: Option<u16>,
}

impl Panels {
    fn height(&self) -> usize {
        match self.side_by_side {
            Some(_) => self.limits.len().max(self.today.len()) + 2,
            None => self.limits.len() + self.today.len() + 4,
        }
    }
}

fn overview_panels(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
) -> Panels {
    if width >= COLUMNS_FROM {
        let left = limits_width(width);
        let roomy = panels(account, problems, now_ms, local_minute, width, Some(left));
        let sessions = sessions::min_height(&sessions::Running::of(account), width, true) as usize;
        let below = if with_sessions {
            sessions + HISTORY_MIN_HEIGHT as usize
        } else {
            sessions.max(HISTORY_MIN_HEIGHT as usize)
        };
        let needed = roomy.height() + below;
        if needed <= height as usize {
            return roomy;
        }
    }
    panels(account, problems, now_ms, local_minute, width, None)
}

fn panels(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    side_by_side: Option<u16>,
) -> Panels {
    let Some(left) = side_by_side else {
        let inner = width.saturating_sub(4);
        return Panels {
            limits: limit_lines(
                account,
                problems,
                now_ms,
                local_minute,
                inner,
                LimitLayout::Compact,
            ),
            today: today_lines(account, problems, inner, false),
            side_by_side,
        };
    };
    let limits_inner = left.saturating_sub(4);
    let today_inner = (width - left).saturating_sub(4 + COLUMN_GAP);
    let mut limits = limit_lines(
        account,
        problems,
        now_ms,
        local_minute,
        limits_inner,
        LimitLayout::Roomy,
    );
    let mut today = today_panel(account, problems, now_ms, today_inner, true);
    limits.insert(0, Line::default());
    today.insert(0, Line::default());
    Panels {
        limits,
        today,
        side_by_side,
    }
}

pub(crate) struct Fit {
    pub(crate) room: u16,
    pub(crate) with_sessions: bool,
}

pub(crate) fn one_account(
    frame: &mut Frame,
    area: Rect,
    fit: Fit,
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
) {
    let Fit {
        room,
        with_sessions,
    } = fit;
    let panels = overview_panels(
        account,
        problems,
        now_ms,
        local_minute,
        area.width,
        room,
        with_sessions,
    );
    let [panels_area, sessions_area] = Layout::vertical([
        Constraint::Length(panels.height() as u16),
        Constraint::Fill(1),
    ])
    .areas(area);
    let (limits_area, today_area) = match panels.side_by_side {
        Some(left) => {
            let [limits_area, _, today_area] = Layout::horizontal([
                Constraint::Length(left),
                Constraint::Length(COLUMN_GAP),
                Constraint::Fill(1),
            ])
            .areas(panels_area);
            (limits_area, today_area)
        }
        None => {
            let [limits_area, today_area] = Layout::vertical([
                Constraint::Length(panels.limits.len() as u16 + 2),
                Constraint::Fill(1),
            ])
            .areas(panels_area);
            (limits_area, today_area)
        }
    };
    let limits_title = match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            format!("Limits · {}", source_and_age(limits, now_ms))
        }
        _ => "Limits".to_string(),
    };
    frame.render_widget(
        Paragraph::new(panels.limits).block(section(&limits_title)),
        limits_area,
    );
    frame.render_widget(
        Paragraph::new(panels.today).block(section("Today")),
        today_area,
    );
    if with_sessions {
        let roomy = panels.side_by_side.is_some();
        sessions::draw(
            frame,
            sessions_area,
            &sessions::Running::of(account),
            now_ms,
            roomy,
        );
    }
}

pub(crate) fn section(title: &str) -> Block<'static> {
    titled_section(Line::styled(format!(" {title} "), TITLE))
}

pub(crate) fn titled_section(title: Line<'static>) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(MUTED)
        .padding(Padding::horizontal(1))
        .title(title)
}

pub(crate) fn source_and_age(limits: &AccountLimits, now_ms: i64) -> String {
    let source = match limits.source {
        LimitSource::GetUsage => "get_usage",
        LimitSource::CachedUsageUtilization => "cachedUsageUtilization",
    };
    format!("from {source}, read {}", age(now_ms - limits.fetched_at_ms))
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum LimitLayout {
    Roomy,
    Compact,
    Stacked,
}

pub(crate) fn limit_lines(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    layout: LimitLayout,
) -> Vec<Line<'static>> {
    let stacked = layout == LimitLayout::Stacked;
    let mut lines = Vec::new();
    match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            let stale = now_ms - limits.fetched_at_ms > MAY_MISS_OUTSIDE_USE_MS;
            if stacked {
                lines.push(Line::styled(
                    format!("Limits · {}", source_and_age(limits, now_ms)),
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
            for (index, shown) in limits.limits.iter().enumerate() {
                if index > 0 && layout == LimitLayout::Roomy {
                    lines.push(Line::default());
                }
                lines.extend(match layout {
                    LimitLayout::Roomy => limit_row(shown, stale, now_ms, local_minute, width),
                    LimitLayout::Compact => named_limit_row(
                        shown,
                        stale,
                        false,
                        None,
                        forecast(shown, now_ms, local_minute),
                        &detail(shown, now_ms, local_minute),
                        width,
                        false,
                    ),
                    LimitLayout::Stacked => {
                        let mut rows = named_limit_row(
                            shown,
                            stale,
                            has_reset(shown, now_ms),
                            status(shown, now_ms),
                            forecast(shown, now_ms, local_minute),
                            &flags(shown, now_ms),
                            width,
                            true,
                        );
                        let budget = credit_budget(shown, now_ms);
                        rows.extend(
                            pack(&budget, (width as usize).saturating_sub(NAME_WIDTH))
                                .into_iter()
                                .map(|packed| {
                                    Line::from(vec![
                                        Span::raw(" ".repeat(NAME_WIDTH)),
                                        Span::styled(packed, MUTED),
                                    ])
                                }),
                        );
                        rows
                    }
                });
            }
        }
        _ => lines.extend(wrapped(
            "No readings yet. The engine reads them while Claude Code is active.",
            width,
            MUTED,
        )),
    }
    for text in limit_markers(account, problems) {
        lines.extend(wrapped(&text, width, PROBLEM));
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
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
) -> Vec<Line<'static>> {
    let color = if stale { STALE } else { pace_color(shown) };
    let width = width as usize;
    let reading = format!("{}%", percent(shown.limit.percent));
    let name = clip(
        &limit_name(&shown.limit),
        width.saturating_sub(reading.chars().count() + 1).max(1),
    );
    let gap = width.saturating_sub(name.chars().count() + reading.chars().count());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(name, Modifier::BOLD),
            Span::raw(" ".repeat(gap)),
            Span::styled(reading, Style::new().fg(color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(bar(shown, width, color, stale)),
    ];
    if let Some(forecast) = forecast(shown, now_ms, local_minute) {
        lines.extend(wrapped(&forecast, width as u16, forecast_style(stale)));
    }
    let budget = credit_budget(shown, now_ms);
    let details = if budget.is_empty() {
        detail(shown, now_ms, local_minute)
    } else {
        detail_after_credits(shown, now_ms, local_minute)
    };
    for packed in pack(&budget, width)
        .into_iter()
        .chain(pack(&details, width))
    {
        lines.extend(wrapped(&packed, width as u16, MUTED));
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn named_limit_row(
    shown: &PacedLimit,
    stale: bool,
    reset: bool,
    status: Option<String>,
    forecast: Option<String>,
    parts: &[String],
    width: u16,
    status_beside: bool,
) -> Vec<Line<'static>> {
    let color = if stale || reset {
        STALE
    } else {
        pace_color(shown)
    };
    let room = (width as usize).saturating_sub(NAME_WIDTH + PERCENT_WIDTH);
    let status_fits = room >= STACKED_BAR_MIN + STATUS_GAP + STATUS_WIDTH;
    let bar_width = match (status_beside, status_fits) {
        (false, _) => room,
        (true, true) => (room - STATUS_GAP - STATUS_WIDTH).min(STACKED_BAR_MAX),
        (true, false) => room.min(STACKED_BAR_MAX),
    };
    let beside = status.is_some() && status_beside && status_fits;
    let bar_width = bar_width.max(8);
    let mut spans = vec![Span::raw(format!(
        "{:<NAME_WIDTH$}",
        clip(&limit_name(&shown.limit), NAME_WIDTH - 1)
    ))];
    spans.extend(bar(shown, bar_width, color, stale || reset));
    let reading = if reset {
        format!("{:>PERCENT_WIDTH$}", "reset")
    } else {
        format!("{:>5}%", percent(shown.limit.percent))
    };
    spans.push(Span::styled(
        reading,
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    ));
    let style = if stale { MUTED } else { NORMAL };
    let forecast_style = forecast_style(stale);
    let mut lines = Vec::new();
    let mut extras_inline = false;
    match status {
        Some(status) if beside => {
            let extras_width: usize = forecast
                .iter()
                .map(|text| text.chars().count() + 3)
                .chain(
                    parts
                        .iter()
                        .map(|text| SEPARATOR.chars().count() + text.chars().count()),
                )
                .sum();
            extras_inline = NAME_WIDTH
                + bar_width
                + PERCENT_WIDTH
                + STATUS_GAP
                + status.chars().count()
                + extras_width
                <= width as usize;
            spans.push(Span::raw(" ".repeat(STATUS_GAP)));
            spans.push(Span::styled(status, style));
            if extras_inline {
                if let Some(forecast) = &forecast {
                    spans.push(Span::styled(format!(" ({forecast})"), forecast_style));
                }
                for part in parts {
                    spans.push(Span::styled(SEPARATOR, style));
                    spans.push(Span::styled(part.clone(), style));
                }
            }
            lines.push(Line::from(spans));
        }
        Some(status) => {
            lines.push(Line::from(spans));
            for packed in wrapped(
                &status,
                (width as usize).saturating_sub(NAME_WIDTH) as u16,
                style,
            ) {
                lines.push(Line::from(
                    [vec![Span::raw(" ".repeat(NAME_WIDTH))], packed.spans].concat(),
                ));
            }
        }
        None => lines.push(Line::from(spans)),
    }
    if extras_inline {
        return lines;
    }
    if let Some(forecast) = forecast {
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(NAME_WIDTH)),
            Span::styled(forecast, forecast_style),
        ]));
    }
    for packed in pack(parts, (width as usize).saturating_sub(NAME_WIDTH)) {
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(NAME_WIDTH)),
            Span::styled(packed, style),
        ]));
    }
    lines
}

pub(crate) fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
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
        None => UNPACED,
    }
}

fn bar(shown: &PacedLimit, width: usize, color: Color, stale: bool) -> Vec<Span<'static>> {
    let track = if stale { STALE_TRACK } else { BAR };
    let filled = cells(shown.limit.percent, width);
    let expected = shown
        .pace
        .as_ref()
        .map(|pace| cells(pace.expected_percent, width).min(width.saturating_sub(1)));
    let mut spans = Vec::new();
    for index in 0..width {
        let (symbol, style) = if Some(index) == expected {
            (BAR, Style::new().add_modifier(Modifier::BOLD))
        } else if index < filled {
            (BAR, Style::new().fg(color))
        } else {
            (track, MUTED)
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
    parts.extend(detail_after_credits(shown, now_ms, local_minute));
    parts
}

fn detail_after_credits(shown: &PacedLimit, now_ms: i64, local_minute: LocalMinute) -> Vec<String> {
    let mut parts = Vec::new();
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

fn status(shown: &PacedLimit, now_ms: i64) -> Option<String> {
    if has_reset(shown, now_ms) {
        return None;
    }
    resets_ms(shown).map(|resets_ms| format!("resets {}", duration(resets_ms - now_ms)))
}

fn credit_budget(shown: &PacedLimit, now_ms: i64) -> Vec<String> {
    if has_reset(shown, now_ms) {
        return Vec::new();
    }
    let Some(credits) = shown.limit.credits.as_ref() else {
        return Vec::new();
    };
    let currency = &credits.currency;
    let Some(cap) = credits.limit else {
        return vec![format!("{} used", money(credits.used, currency, true))];
    };
    let left = (cap - credits.used).max(0.0);
    let mut parts = vec![
        format!(
            "{} of {}",
            money(credits.used, currency, true),
            money(cap, currency, false)
        ),
        format!("{} left", money(left, currency, false)),
    ];
    if let Some((per_day, _)) = credits_per_day(shown, now_ms) {
        parts.push(format!("{} a day to last", money(per_day, currency, false)));
    }
    parts
}

fn money(amount: f64, currency: &str, cents: bool) -> String {
    let text = if cents {
        format!("{amount:.2}")
    } else {
        format!("{amount:.0}")
    };
    let (whole, fraction) = text.split_at(text.find('.').unwrap_or(text.len()));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if currency == "USD" {
        format!("${grouped}{fraction}")
    } else {
        format!("{grouped}{fraction} {currency}")
    }
}

fn flags(shown: &PacedLimit, now_ms: i64) -> Vec<String> {
    if has_reset(shown, now_ms) {
        return Vec::new();
    }
    unusual_severity(&shown.limit).into_iter().collect()
}

fn forecast_style(stale: bool) -> Style {
    Style::new().fg(if stale { STALE } else { OVER })
}

fn forecast(shown: &PacedLimit, now_ms: i64, local_minute: LocalMinute) -> Option<String> {
    let forecast = shown.forecast?;
    let full_at_ms = forecast.full_at_ms;
    if has_reset(shown, now_ms) {
        return None;
    }
    let clock = clock(full_at_ms, local_minute);
    let when = if full_at_ms > now_ms {
        countdown(full_at_ms, now_ms, local_minute)
    } else if clock.is_empty() {
        "by now".to_string()
    } else if now_ms - full_at_ms >= DAY_MS {
        format!("by {}", local_minute(full_at_ms))
    } else {
        format!("by {clock}")
    };
    Some(format!("100% {when}{}", recent_suffix(&forecast)))
}

fn resets(resets_ms: i64, now_ms: i64, local_minute: LocalMinute) -> String {
    format!("resets {}", countdown(resets_ms, now_ms, local_minute))
}

fn clock(ms: i64, local_minute: LocalMinute) -> String {
    local_minute(ms).chars().skip(11).collect()
}

fn countdown(resets_ms: i64, now_ms: i64, local_minute: LocalMinute) -> String {
    let minutes = (resets_ms - now_ms).max(0) / 60_000;
    let countdown = duration(resets_ms - now_ms);
    let clock = clock(resets_ms, local_minute);
    if minutes < 1440 && !clock.is_empty() {
        format!("in {countdown} at {clock}")
    } else {
        format!("in {countdown}")
    }
}

pub(crate) fn duration(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0..60 => format!("{minutes}m"),
        60..1440 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}

pub(crate) fn limits_width(width: u16) -> u16 {
    (u32::from(width) * LIMITS_SHARE / 100) as u16
}

pub(crate) fn today_lines(
    account: &AccountToday,
    problems: &[Problem],
    width: u16,
    stacked: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if stacked {
        let header = match usual_days_so_far(&account.pace) {
            Some(days) => format!("Today · usual from {days} of {USUAL_DAYS} days"),
            None => "Today".to_string(),
        };
        lines.push(Line::styled(header, MUTED));
    }
    let mut spans = vec![Span::styled(
        format!("${:.2}", account.cost_usd),
        Modifier::BOLD,
    )];
    if account.cost_estimated {
        spans.push(Span::styled("  estimated", Style::new().fg(EVEN)));
    }
    spans.push(Span::styled(" · ", MUTED));
    spans.extend(against_usual(&account.pace));
    lines.push(Line::from(spans));
    if !stacked && let Some(provisional) = provisional_usual(&account.pace) {
        lines.push(Line::styled(provisional, MUTED));
    }
    for text in spend_markers(account, problems) {
        lines.extend(wrapped(&text, width, PROBLEM));
    }
    lines
}

pub(crate) fn today_panel(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    width: u16,
    with_chart: bool,
) -> Vec<Line<'static>> {
    let spent = format!("${:.2}", account.cost_usd);
    let emphasis = Style::new()
        .fg(usual_color(&account.pace).unwrap_or(Color::Reset))
        .add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    match digits::large(&spent).filter(|rows| rows[0].chars().count() <= width as usize) {
        Some(rows) => lines.extend(rows.into_iter().map(|row| Line::styled(row, emphasis))),
        None => lines.push(Line::styled(spent, emphasis)),
    }
    lines.push(Line::default());
    lines.push(Line::from(against_usual(&account.pace)));
    if account.cost_estimated {
        lines.extend(wrapped(
            "estimated until today's sessions are checked",
            width,
            Style::new().fg(EVEN),
        ));
    }
    if let Some(provisional) = provisional_usual(&account.pace) {
        lines.push(Line::styled(provisional, MUTED));
    }
    for text in spend_markers(account, problems) {
        lines.extend(wrapped(&text, width, PROBLEM));
    }
    if with_chart {
        let budget = daily_credit_budget(account, now_ms);
        lines.extend(daily_chart(&account.pace, budget, width, emphasis));
    }
    lines
}

fn usual_days_so_far(pace: &DollarPace) -> Option<u32> {
    (pace.usual_usd.is_some() && pace.usual_days < USUAL_DAYS).then_some(pace.usual_days)
}

fn provisional_usual(pace: &DollarPace) -> Option<String> {
    usual_days_so_far(pace)
        .map(|days| format!("usual is provisional: {days} of {USUAL_DAYS} days so far"))
}

fn daily_chart(
    pace: &DollarPace,
    budget: Option<f64>,
    width: u16,
    today_style: Style,
) -> Vec<Line<'static>> {
    let totals = &pace.earlier_day_totals_usd;
    let day_width = SPARK_DAY_CELLS + SPARK_DAY_GAP;
    let shown = totals
        .len()
        .min(((width as usize + SPARK_DAY_GAP) / day_width).saturating_sub(1));
    if shown == 0 {
        return Vec::new();
    }
    let earlier = &totals[totals.len() - shown..];
    let biggest_day = earlier.iter().copied().fold(pace.today_usd, f64::max);
    let highest = budget.map_or(biggest_day, |budget| budget.max(biggest_day));
    let steps = SPARK_ROWS * SPARK_LEVELS.len();
    let height = |usd: f64| {
        if highest > 0.0 {
            ((usd / highest * steps as f64).round() as usize).min(steps)
        } else {
            0
        }
    };
    let chart_width = (shown + 1) * day_width - SPARK_DAY_GAP;
    let mut title = format!("Last {shown} {}", day_kind(pace));
    if let Some(usual) = crate::pace::median(earlier.to_vec()) {
        title.push_str(&format!(" · usual day ${usual:.0}"));
    }
    let scale = format!("max ${biggest_day:.0}");
    let room = chart_width.saturating_sub(title.chars().count());
    let mut lines = vec![Line::default(), Line::styled(title, MUTED)];
    if room > scale.len() {
        let last = lines.last_mut().expect("title line");
        last.spans
            .push(Span::styled(format!("{scale:>room$}"), MUTED));
    }
    if let Some(budget) = budget {
        lines.push(Line::styled(
            format!("┄┄ ${budget:.0} a day lasts the credits"),
            MUTED,
        ));
    }
    let budget_level = budget.map(height);
    let over_budget = |usd: f64| budget.is_some_and(|budget| usd > budget);
    for row in (0..SPARK_ROWS).rev() {
        let level_row =
            budget_level.is_some_and(|level| level.div_ceil(SPARK_LEVELS.len()).max(1) - 1 == row);
        let blank = if level_row { "┄" } else { " " };
        let column = |usd: f64, style: Style| {
            let cell = spark_cell(height(usd), row);
            if cell == ' ' {
                Span::styled(blank.repeat(SPARK_DAY_CELLS), MUTED)
            } else {
                Span::styled(cell.to_string().repeat(SPARK_DAY_CELLS), style)
            }
        };
        let mut spans = Vec::new();
        for usd in earlier {
            let style = if over_budget(*usd) {
                Style::new().fg(OVER)
            } else {
                KEY
            };
            spans.push(column(*usd, style));
            spans.push(Span::styled(blank.repeat(SPARK_DAY_GAP), MUTED));
        }
        spans.push(column(pace.today_usd, today_style));
        lines.push(Line::from(spans));
    }
    let start = pace
        .earlier_days
        .len()
        .checked_sub(shown)
        .and_then(|first| pace.earlier_days.get(first))
        .and_then(|day| short_date(day))
        .filter(|date| date.len() + 1 + "today".len() <= chart_width)
        .unwrap_or_default();
    let today_at = chart_width - start.chars().count();
    lines.push(Line::styled(
        format!("{start}{:>today_at$}", "today"),
        MUTED,
    ));
    lines
}

fn daily_credit_budget(account: &AccountToday, now_ms: i64) -> Option<f64> {
    account
        .limits
        .as_ref()?
        .limits
        .iter()
        .find_map(|shown| credits_per_day(shown, now_ms).filter(|(_, currency)| currency == "USD"))
        .map(|(per_day, _)| per_day)
}

fn credits_per_day(shown: &PacedLimit, now_ms: i64) -> Option<(f64, String)> {
    if has_reset(shown, now_ms) {
        return None;
    }
    let credits = shown.limit.credits.as_ref()?;
    let left = (credits.limit? - credits.used).max(0.0);
    let days_left = (resets_ms(shown)? - now_ms) as f64 / DAY_MS as f64;
    (days_left >= 1.0 && left > 0.0).then(|| (left / days_left, credits.currency.clone()))
}

fn short_date(day: &str) -> Option<String> {
    let (_, month_day) = day.split_once('-')?;
    let (month, day) = month_day.split_once('-')?;
    let month = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    Some(format!("{month} {}", day.parse::<u32>().ok()?))
}

fn spark_cell(height: usize, row: usize) -> char {
    let levels = SPARK_LEVELS.len();
    let floor = row * levels;
    match height.saturating_sub(floor) {
        0 if row == 0 => SPARK_LEVELS[0],
        0 => ' ',
        filled => SPARK_LEVELS[filled.min(levels) - 1],
    }
}

fn day_kind(pace: &DollarPace) -> &'static str {
    if pace.weekend {
        "weekend days"
    } else {
        "workdays"
    }
}

pub(crate) fn against_usual(pace: &DollarPace) -> Vec<Span<'static>> {
    let Some(usual_usd) = pace.usual_usd else {
        return vec![Span::styled("no usual yet", MUTED)];
    };
    match pace.times_usual() {
        Some(times) => {
            let plain = Style::new().fg(usual_color(pace).unwrap_or(Color::Reset));
            let strong = plain.add_modifier(Modifier::BOLD);
            vec![
                Span::styled(format!("{times:.1}×"), strong),
                Span::styled(" your usual of ", plain),
                Span::styled(format!("${usual_usd:.2}"), strong),
                Span::styled(format!(" by {}", pace.clock), plain),
            ]
        }
        None => vec![Span::raw(format!(
            "usual by {} is ${usual_usd:.2}",
            pace.clock
        ))],
    }
}

fn usual_color(pace: &DollarPace) -> Option<Color> {
    pace.times_usual().map(|times| match times {
        ..USUAL_EVEN_FROM => UNDER,
        ..=USUAL_EVEN_TO => EVEN,
        _ => OVER,
    })
}

pub(crate) fn clip_middle(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width || width == 0 {
        return text.to_string();
    }
    let kept = width - 1;
    let head = kept.div_ceil(2);
    let tail = kept - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

pub(crate) fn help(frame: &mut Frame, area: Rect, report_port: Option<u16>) {
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
    let mut lines = vec![
        key("1-9", "show that account; with up to 8, the next shows all"),
        key("[ ]", "step through the account tabs, including all"),
        key("a", "toggle the all view, with 9 or more accounts"),
        key("Tab", "switch to spending in a small window"),
        key("← → g", "spending by repo, skill, subagent, model"),
        key("t T", "spending today, this week, this month, all time"),
        key("↑ ↓", "highlight a spending row"),
        key("Enter", "show or hide a repo's branches"),
        key("e", "show or hide every repo's branches"),
        key("s", "list every running session"),
        key("u", "show Anthropic's usage advice for today"),
    ];
    if let Some(port) = report_port {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<8}", "report"),
                KEY.add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("http://127.0.0.1:{port}")),
        ]));
    }
    lines.extend([
        key("?", "open or close this help"),
        key("Esc", "close help, advice, or the sessions list"),
        key("q", "quit (Ctrl-C too)"),
        Line::default(),
        color("▇▇▇", Style::new().fg(UNDER), "under your pace"),
        color("▇▇▇", Style::new().fg(EVEN), "on pace"),
        color("▇▇▇", Style::new().fg(OVER), "over your pace"),
        color("▇▇▇", Style::new().fg(STALE), "a reading over an hour old"),
        color(
            "▇",
            Style::new().add_modifier(Modifier::BOLD),
            "expected use by now",
        ),
        color(
            "▲",
            Style::new().fg(OVER).add_modifier(Modifier::BOLD),
            "a session spending $20/hr or more",
        ),
    ]);
    let popup = super::centered(area, 64, lines.len() as u16 + 2);
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

pub(crate) fn spend_markers(account: &AccountToday, problems: &[Problem]) -> Vec<String> {
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
