use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::contract::{
    AccountLimits, AccountToday, Credits, DollarPace, LimitSource, PaceState, PacedLimit, Problem,
    ProblemKind,
};
use crate::limits::{age, limit_name, percent, unusual_severity};
use crate::status::{STALE_AFTER_MS, has_reset, resets_ms};

use super::HISTORY_MIN_HEIGHT;
use super::digits;
use super::extras::{Extras, Step, cards_height, grow};
use super::sessions;
use super::style::{BAR, EVEN, KEY, MUTED, OVER, PROBLEM, STALE, STALE_BAR, TITLE, UNDER, UNPACED};

const USUAL_EVEN_FROM: f64 = 0.9;
const USUAL_EVEN_TO: f64 = 1.1;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub(crate) const SIDE_BY_SIDE_FROM: u16 = 90;
pub(crate) const COLUMN_GAP: u16 = 1;
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
    one_account_layout(
        account,
        problems,
        now_ms,
        local_minute,
        width,
        height,
        with_sessions,
    )
    .extras
    .spacing
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
    let layout = one_account_layout(
        account,
        problems,
        now_ms,
        local_minute,
        width,
        height,
        with_sessions,
    );
    layout.panels.height() as u16
        + cards_height(
            &sessions::SessionsToday::of(account, now_ms),
            width,
            with_sessions,
            layout.extras,
        )
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

struct OneAccountLayout {
    panels: Panels,
    extras: Extras,
}

fn one_account_layout(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
) -> OneAccountLayout {
    let build = |extras: Extras| panels(account, problems, now_ms, local_minute, width, extras);
    let fits = |extras: Extras| {
        let panels = build(extras).height() as u16;
        let cards = cards_height(
            &sessions::SessionsToday::of(account, now_ms),
            width,
            with_sessions,
            extras,
        );
        let spending = if extras.spending {
            HISTORY_MIN_HEIGHT
        } else {
            0
        };
        panels + cards + spending <= height
    };
    let mut steps = vec![Step::Spending, Step::Spacing, Step::CardsGap];
    if width >= SIDE_BY_SIDE_FROM {
        steps.push(Step::Chart);
    }
    steps.push(Step::SecondRow);
    let extras = grow(Extras::default(), &steps, fits);
    OneAccountLayout {
        panels: build(extras),
        extras,
    }
}

fn panels(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    extras: Extras,
) -> Panels {
    let top = vec![Line::default(); usize::from(extras.spacing)];
    let gap = usize::from(extras.spacing);
    if width < SIDE_BY_SIDE_FROM {
        let inner = width.saturating_sub(4);
        let rows = limit_rows(
            account,
            problems,
            now_ms,
            local_minute,
            inner,
            LimitLayout::Full,
        );
        return Panels {
            limits: [top.clone(), rows.lines(gap)].concat(),
            today: [top, today_lines(account, problems, inner)].concat(),
            side_by_side: None,
        };
    }
    let left = limits_width(width);
    let limits_inner = left.saturating_sub(4);
    let today_inner = (width - left).saturating_sub(4 + COLUMN_GAP);
    let emphasis = spent_emphasis(account);
    let mut today = top.clone();
    today.extend(large_spent(account, today_inner, emphasis));
    if extras.chart {
        today.push(Line::default());
    }
    today.extend(spent_notes(account, problems, today_inner));
    if extras.chart {
        let budget = daily_credit_budget(account, now_ms);
        today.extend(daily_chart(&account.pace, budget, today_inner, emphasis));
    }
    let rows = limit_rows(
        account,
        problems,
        now_ms,
        local_minute,
        limits_inner,
        LimitLayout::Full,
    );
    let limits = [top, rows.spread(gap, today.len() - gap)].concat();
    Panels {
        limits,
        today,
        side_by_side: Some(left),
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
    let OneAccountLayout { panels, extras } = one_account_layout(
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
            let [today_area, _, limits_area] = Layout::horizontal([
                Constraint::Length(area.width.saturating_sub(left + COLUMN_GAP)),
                Constraint::Length(COLUMN_GAP),
                Constraint::Fill(1),
            ])
            .areas(panels_area);
            (limits_area, today_area)
        }
        None => {
            let [today_area, limits_area] = Layout::vertical([
                Constraint::Length(panels.today.len() as u16 + 2),
                Constraint::Fill(1),
            ])
            .areas(panels_area);
            (limits_area, today_area)
        }
    };
    let limits_title = match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            let full = format!("Limits · {}", source_and_age(limits, now_ms));
            if full.chars().count() <= limits_area.width.saturating_sub(4) as usize {
                full
            } else {
                format!("Limits · read {}", age(now_ms - limits.fetched_at_ms))
            }
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
        sessions::draw(
            frame,
            sessions_area,
            &sessions::SessionsToday::of(account, now_ms),
            now_ms,
            extras.cards(),
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
    Full,
}

pub(crate) struct LimitRows {
    before: Vec<Line<'static>>,
    groups: Vec<Vec<Line<'static>>>,
    after: Vec<Line<'static>>,
}

impl LimitRows {
    pub(crate) fn height(&self, gap: usize) -> usize {
        self.before.len()
            + self.groups.iter().map(Vec::len).sum::<usize>()
            + gap * self.groups.len().saturating_sub(1)
            + self.after.len()
    }

    pub(crate) fn lines(self, gap: usize) -> Vec<Line<'static>> {
        let mut lines = self.before;
        for (index, group) in self.groups.into_iter().enumerate() {
            if index > 0 {
                lines.extend(vec![Line::default(); gap]);
            }
            lines.extend(group);
        }
        lines.extend(self.after);
        lines
    }

    pub(crate) fn spread(self, gap: usize, rows: usize) -> Vec<Line<'static>> {
        let gap = if self.height(gap.max(1)) <= rows {
            gap.max(1)
        } else {
            gap
        };
        self.lines(gap)
    }
}

pub(crate) fn limit_lines(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    layout: LimitLayout,
) -> Vec<Line<'static>> {
    let gap = usize::from(layout == LimitLayout::Roomy);
    limit_rows(account, problems, now_ms, local_minute, width, layout).lines(gap)
}

pub(crate) fn limit_rows(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    layout: LimitLayout,
) -> LimitRows {
    let mut before = Vec::new();
    let mut groups = Vec::new();
    match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            let stale = now_ms - limits.fetched_at_ms > STALE_AFTER_MS;
            for shown in &limits.limits {
                groups.push(match layout {
                    LimitLayout::Roomy => {
                        limit_row(shown, stale, now_ms, local_minute, width, false)
                    }
                    LimitLayout::Full => limit_row(shown, stale, now_ms, local_minute, width, true),
                });
            }
        }
        _ => before.extend(wrapped(
            "No readings yet. The engine reads them while Claude Code is active.",
            width,
            MUTED,
        )),
    }
    let after = limit_markers(account, problems)
        .iter()
        .flat_map(|text| wrapped(text, width, PROBLEM))
        .collect();
    LimitRows {
        before,
        groups,
        after,
    }
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
    details_on_name_line: bool,
) -> Vec<Line<'static>> {
    let reset = has_reset(shown, now_ms);
    let color = if stale || reset {
        STALE
    } else {
        pace_color(shown)
    };
    let width = width as usize;
    let reading = if reset {
        "reset".to_string()
    } else {
        format!("{}%", percent(shown.limit.percent))
    };
    let estimate = elsewhere_estimate(shown, now_ms)
        .map(|estimate| format!(" {estimate}"))
        .unwrap_or_default();
    let reading_width = reading.chars().count() + estimate.chars().count();
    let forecast = forecast(shown, now_ms, local_minute);
    let mut budget = credit_budget(shown, now_ms);
    let mut details = if budget.is_empty() {
        detail(shown, now_ms, local_minute)
    } else {
        detail_after_credits(shown, now_ms, local_minute)
    };
    let forecast_leads = if details_on_name_line || budget.is_empty() {
        &mut details
    } else {
        &mut budget
    };
    forecast_leads.splice(0..0, forecast.iter().cloned());
    let name = clip(
        &limit_name(&shown.limit),
        width.saturating_sub(reading_width + 1).max(1),
    );
    let mut used = name.chars().count();
    let mut head = vec![Span::styled(name, Modifier::BOLD)];
    while let Some(part) = details.first().filter(|_| details_on_name_line) {
        let wanted = SEPARATOR.chars().count() + part.chars().count();
        if used + wanted + 1 + reading_width > width {
            break;
        }
        let part = details.remove(0);
        head.push(Span::styled(SEPARATOR, MUTED));
        head.extend(detail_spans(&part, forecast.as_deref(), MUTED, stale));
        used += wanted;
    }
    head.push(Span::raw(
        " ".repeat(width.saturating_sub(used + reading_width)),
    ));
    head.push(Span::styled(
        reading,
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    ));
    head.push(Span::styled(estimate, elsewhere_style(color)));
    let mut lines = vec![
        Line::from(head),
        Line::from(bar(shown, width, color, stale, reset)),
    ];
    for packed in pack(&budget, width)
        .into_iter()
        .chain(pack(&details, width))
    {
        for line in wrapped(&packed, width as u16, MUTED) {
            lines.push(Line::from(detail_spans(
                &line.to_string(),
                forecast.as_deref(),
                MUTED,
                stale,
            )));
        }
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

fn bar(
    shown: &PacedLimit,
    width: usize,
    color: Color,
    stale: bool,
    reset: bool,
) -> Vec<Span<'static>> {
    let color = if stale || reset { STALE_BAR } else { color };
    let filled = if reset {
        0
    } else {
        cells(shown.limit.percent, width)
    };
    let elsewhere_from = elsewhere_share(shown).map_or(filled, |elsewhere| {
        filled.saturating_sub(cells(elsewhere, width))
    });
    let expected = shown
        .pace
        .as_ref()
        .filter(|_| !reset)
        .map(|pace| cells(pace.expected_percent, width).min(width.saturating_sub(1)));
    let mut spans = Vec::new();
    for index in 0..width {
        let (symbol, style) = if Some(index) == expected {
            (BAR, Style::new().add_modifier(Modifier::BOLD))
        } else if index < elsewhere_from {
            (BAR, Style::new().fg(color))
        } else if index < filled {
            (BAR, Style::new().fg(color).add_modifier(Modifier::DIM))
        } else {
            (BAR, MUTED)
        };
        spans.push(Span::styled(symbol, style));
    }
    spans
}

fn elsewhere_share(shown: &PacedLimit) -> Option<f64> {
    let credits_cap = shown
        .limit
        .credits
        .as_ref()
        .and_then(|credits| credits.limit);
    shown
        .elsewhere_usd
        .zip(credits_cap)
        .map(|(elsewhere, cap)| elsewhere / cap * 100.0)
        .or(shown.elsewhere_percent)
}

fn cells(percent: f64, width: usize) -> usize {
    ((percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize
}

fn detail(shown: &PacedLimit, now_ms: i64, local_minute: LocalMinute) -> Vec<String> {
    if has_reset(shown, now_ms) {
        return vec![reset_detail(shown, now_ms, local_minute)];
    }
    let mut parts: Vec<String> = shown
        .limit
        .credits
        .as_ref()
        .map(|credits| credits_spent(shown, credits))
        .into_iter()
        .collect();
    parts.extend(detail_after_credits(shown, now_ms, local_minute));
    parts
}

fn reset_detail(shown: &PacedLimit, now_ms: i64, local_minute: LocalMinute) -> String {
    let Some(resets_ms) = resets_ms(shown) else {
        return "reset, not read since".to_string();
    };
    let when = if now_ms - resets_ms < DAY_MS {
        clock(resets_ms, local_minute)
    } else {
        local_minute(resets_ms)
    };
    if when.is_empty() {
        format!("reset {} ago, not read since", duration(now_ms - resets_ms))
    } else {
        format!("reset at {when}, not read since")
    }
}

fn elsewhere(shown: &PacedLimit) -> String {
    shown
        .elsewhere_usd
        .map(|elsewhere| format!(" ({} elsewhere)", money(elsewhere, "USD", false)))
        .unwrap_or_default()
}

fn elsewhere_estimate(shown: &PacedLimit, now_ms: i64) -> Option<String> {
    if has_reset(shown, now_ms) {
        return None;
    }
    shown
        .elsewhere_percent
        .map(|points| format!("(~{points:.0}%)"))
}

fn elsewhere_style(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::DIM)
}

fn detail_after_credits(shown: &PacedLimit, now_ms: i64, local_minute: LocalMinute) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(resets_ms) = resets_ms(shown) {
        parts.push(resets(resets_ms, now_ms, local_minute));
    }
    parts.extend(unusual_severity(&shown.limit));
    parts
}

fn detail_spans(
    text: &str,
    forecast: Option<&str>,
    style: Style,
    stale: bool,
) -> Vec<Span<'static>> {
    let warnings = forecast.into_iter().filter(|phrase| !phrase.is_empty());
    let mut spans = Vec::new();
    let mut rest = text;
    for phrase in warnings {
        let Some(at) = rest.find(phrase) else {
            continue;
        };
        if at > 0 {
            spans.push(Span::styled(rest[..at].to_string(), style));
        }
        spans.push(Span::styled(phrase.to_string(), forecast_style(stale)));
        rest = &rest[at + phrase.len()..];
    }
    spans.push(Span::styled(rest.to_string(), style));
    spans
}

fn credit_budget(shown: &PacedLimit, now_ms: i64) -> Vec<String> {
    if has_reset(shown, now_ms) {
        return Vec::new();
    }
    let Some(credits) = shown.limit.credits.as_ref() else {
        return Vec::new();
    };
    let currency = &credits.currency;
    let mut parts = vec![credits_spent(shown, credits)];
    if let Some((per_day, _)) = credits_per_day(shown, now_ms) {
        parts.push(format!("{} / day", money(per_day, currency, false)));
    }
    parts
}

fn credits_spent(shown: &PacedLimit, credits: &Credits) -> String {
    let used = money(credits.used, &credits.currency, true);
    match credits.limit {
        Some(cap) => format!(
            "{used} of {}{}",
            money(cap, &credits.currency, false),
            elsewhere(shown)
        ),
        None => format!("{used} used{}", elsewhere(shown)),
    }
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
    Some(format!("100% {when}"))
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
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans = vec![Span::styled(
        format!("${:.2}", account.cost_usd),
        Modifier::BOLD,
    )];
    spans.push(Span::styled(" · ", MUTED));
    spans.extend(against_usual(&account.pace));
    lines.push(Line::from(spans));
    for text in spend_markers(account, problems) {
        lines.extend(wrapped(&text, width, PROBLEM));
    }
    lines
}

pub(crate) fn today_panel(
    account: &AccountToday,
    problems: &[Problem],
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = large_spent(account, width, spent_emphasis(account));
    lines.push(Line::default());
    lines.extend(spent_notes(account, problems, width));
    lines
}

pub(crate) fn spent_emphasis(account: &AccountToday) -> Style {
    Style::new()
        .fg(usual_color(&account.pace).unwrap_or(Color::Reset))
        .add_modifier(Modifier::BOLD)
}

fn large_spent(account: &AccountToday, width: u16, emphasis: Style) -> Vec<Line<'static>> {
    let spent = format!("${:.2}", account.cost_usd);
    match digits::large(&spent).filter(|rows| rows[0].chars().count() <= width as usize) {
        Some(rows) => rows
            .into_iter()
            .map(|row| Line::styled(row, emphasis))
            .collect(),
        None => vec![Line::styled(spent, emphasis)],
    }
}

pub(crate) fn spent_notes(
    account: &AccountToday,
    problems: &[Problem],
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(against_usual(&account.pace))];
    if let Some(projected) = projected_day_usd(account) {
        lines.push(Line::styled(
            format!("on pace for ~${projected:.0} today"),
            MUTED,
        ));
    }
    for text in spend_markers(account, problems) {
        lines.extend(wrapped(&text, width, PROBLEM));
    }
    lines
        .into_iter()
        .flat_map(|line| wrapped_line(line, width as usize))
        .collect()
}

pub(crate) fn usual_notes_width(account: &AccountToday) -> usize {
    let usual: usize = against_usual(&account.pace).iter().map(Span::width).sum();
    let pace = projected_day_usd(account)
        .map(|projected| {
            format!("on pace for ~${projected:.0} today")
                .chars()
                .count()
        })
        .unwrap_or(0);
    usual.max(pace)
}

fn projected_day_usd(account: &AccountToday) -> Option<f64> {
    let usual_by_now = account.pace.usual_usd?;
    let usual_day = crate::pace::median(account.pace.earlier_day_totals_usd.clone())?;
    Some(account.cost_usd + (usual_day - usual_by_now).max(0.0))
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
    if let Some(usual) = crate::pace::median(totals.clone()) {
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

pub(crate) fn help(
    frame: &mut Frame,
    area: Rect,
    report_port: Option<u16>,
    fast_usd_per_hour: f64,
) {
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
        key("s", "list every session today"),
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
        Line::from(vec![
            Span::styled(format!("  {:<8}", "▇▇▇"), Style::new().fg(UNDER)),
            Span::raw("under  "),
            Span::styled("▇▇▇ ", Style::new().fg(EVEN)),
            Span::raw("on  "),
            Span::styled("▇▇▇ ", Style::new().fg(OVER)),
            Span::raw("over your pace"),
        ]),
        color(
            "▇▇▇",
            Style::new().fg(STALE_BAR),
            "a reading over an hour old",
        ),
        color(
            "▇",
            Style::new().add_modifier(Modifier::BOLD),
            "expected use by now",
        ),
        color(
            "▇▇▇",
            Style::new().fg(EVEN).add_modifier(Modifier::DIM),
            "used elsewhere, like claude.ai (~13% is estimated)",
        ),
        Line::from(vec![
            Span::styled(
                format!("  {:<8}", "▲"),
                Style::new().fg(OVER).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "a session spending {} or more",
                sessions::dollars_an_hour(fast_usd_per_hour)
            )),
        ]),
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

pub(crate) fn styled_spans(line: Line<'static>) -> Vec<Span<'static>> {
    let base = line.style;
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content, base.patch(span.style)))
        .collect()
}

fn wrapped_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let cells: Vec<(char, Style)> = styled_spans(line)
        .into_iter()
        .flat_map(|span| {
            let style = span.style;
            span.content
                .chars()
                .map(move |c| (c, style))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let words = cells
        .split(|(c, _)| *c == ' ')
        .filter(|word| !word.is_empty())
        .flat_map(|word| word.chunks(width.max(1)));
    for word in words {
        match rows.last_mut() {
            Some(row) if row.len() + 1 + word.len() <= width => {
                row.push((' ', Style::new()));
                row.extend_from_slice(word);
            }
            _ => rows.push(word.to_vec()),
        }
    }
    rows.into_iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, style) in row {
                match spans.last_mut() {
                    Some(span) if span.style == style => span.content.to_mut().push(c),
                    _ => spans.push(Span::styled(c.to_string(), style)),
                }
            }
            Line::from(spans)
        })
        .collect()
}
