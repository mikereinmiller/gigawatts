use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::contract::{AccountToday, Snapshot};
use crate::limits::age;

use super::HISTORY_MIN_HEIGHT;
use super::sections::{
    COLUMN_GAP, LimitLayout, LocalMinute, limit_lines, limits_width, section, source_and_age,
    today_lines, today_panel,
};
use super::sessions::{self, Running};
use super::style::MUTED;

const ROOMY_FROM: u16 = 100;
const THREE_COLUMNS_FROM: u16 = 150;
const BORDER_ROWS: usize = 2;
const BORDER_AND_PADDING_COLUMNS: u16 = 4;
const STRIP_ROWS: u16 = 1;
const SUMMARY_ROWS: u16 = 2;

pub(crate) struct Fit {
    pub(crate) roomy: bool,
    with_sessions: bool,
}

struct Pane {
    title: String,
    lines: Vec<Line<'static>>,
}

struct Column {
    width: u16,
    today: Pane,
    limits: Pane,
}

struct Overview {
    columns: Vec<Column>,
}

impl Overview {
    fn today_height(&self) -> u16 {
        tallest(self.columns.iter().map(|column| &column.today))
    }

    fn limits_height(&self) -> u16 {
        tallest(self.columns.iter().map(|column| &column.limits))
    }

    fn height(&self) -> u16 {
        STRIP_ROWS + self.today_height() + self.limits_height()
    }
}

fn tallest<'a>(panes: impl Iterator<Item = &'a Pane>) -> u16 {
    panes
        .map(|pane| pane.lines.len() + BORDER_ROWS)
        .max()
        .unwrap_or(0) as u16
}

pub(crate) fn fit(
    snapshot: &Snapshot,
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
) -> Fit {
    let running = Running::every(&snapshot.accounts);
    if let Some(overview) = roomy(snapshot, now_ms, local_minute, width) {
        let sessions = sessions::min_height(&running, width, true);
        let below = if with_sessions {
            sessions + HISTORY_MIN_HEIGHT
        } else {
            sessions.max(HISTORY_MIN_HEIGHT)
        };
        if overview.height() + below <= height {
            return Fit {
                roomy: true,
                with_sessions,
            };
        }
    }
    let needed =
        compact_height(snapshot, now_ms, width) + sessions::min_height(&running, width, false);
    Fit {
        roomy: false,
        with_sessions: with_sessions && needed <= height,
    }
}

pub(crate) fn height(
    snapshot: &Snapshot,
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
) -> u16 {
    let fit = fit(snapshot, now_ms, local_minute, width, height, with_sessions);
    let overview = match roomy(snapshot, now_ms, local_minute, width) {
        Some(overview) if fit.roomy => overview.height(),
        _ => {
            compact_height(snapshot, now_ms, width)
                + compact_spacing(snapshot, now_ms, width, height, fit.with_sessions)
                    .rows(snapshot.accounts.len() as u16)
        }
    };
    let sessions = if fit.with_sessions {
        let room = height.saturating_sub(overview + HISTORY_MIN_HEIGHT);
        sessions::height(&Running::every(&snapshot.accounts), width, room, fit.roomy)
    } else {
        0
    };
    overview + sessions
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    frame: &mut Frame,
    area: Rect,
    room: u16,
    snapshot: &Snapshot,
    now_ms: i64,
    local_minute: LocalMinute,
    with_sessions: bool,
) {
    let fit = fit(
        snapshot,
        now_ms,
        local_minute,
        area.width,
        room,
        with_sessions,
    );
    let rest = match roomy(snapshot, now_ms, local_minute, area.width) {
        Some(overview) if fit.roomy => draw_roomy(frame, area, snapshot, overview),
        _ => draw_compact(
            frame,
            area,
            snapshot,
            now_ms,
            compact_spacing(snapshot, now_ms, area.width, room, fit.with_sessions),
        ),
    };
    if fit.with_sessions {
        sessions::draw(
            frame,
            rest,
            &Running::every(&snapshot.accounts),
            now_ms,
            fit.roomy,
        );
    }
}

fn column_widths(width: u16, accounts: usize) -> Option<Vec<u16>> {
    match accounts {
        2 if width >= ROOMY_FROM => {
            let left = limits_width(width);
            Some(vec![left, width.saturating_sub(left + COLUMN_GAP)])
        }
        3 if width >= THREE_COLUMNS_FROM => {
            let room = width.saturating_sub(2 * COLUMN_GAP);
            let mut widths = vec![room / 3; 3];
            widths[2] += room % 3;
            Some(widths)
        }
        _ => None,
    }
}

fn roomy(
    snapshot: &Snapshot,
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
) -> Option<Overview> {
    let widths = column_widths(width, snapshot.accounts.len())?;
    let columns = snapshot
        .accounts
        .iter()
        .zip(widths)
        .map(|(account, width)| Column {
            width,
            today: Pane {
                title: format!("{} · today", account.account_label),
                lines: padded(today_panel(
                    account,
                    &snapshot.problems,
                    inner(width),
                    false,
                )),
            },
            limits: Pane {
                title: limits_title(account, now_ms, inner(width)),
                lines: padded(limit_lines(
                    account,
                    &snapshot.problems,
                    now_ms,
                    local_minute,
                    inner(width),
                    LimitLayout::Roomy,
                )),
            },
        })
        .collect();
    Some(Overview { columns })
}

fn padded(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let mut padded = vec![Line::default()];
    padded.extend(lines);
    padded
}

fn inner(width: u16) -> u16 {
    width.saturating_sub(BORDER_AND_PADDING_COLUMNS)
}

fn limits_title(account: &AccountToday, now_ms: i64, width: u16) -> String {
    let label = &account.account_label;
    let Some(limits) = account
        .limits
        .as_ref()
        .filter(|limits| !limits.limits.is_empty())
    else {
        return format!("{label} · limits");
    };
    let full = format!("{label} · limits {}", source_and_age(limits, now_ms));
    if full.chars().count() <= width as usize {
        full
    } else {
        format!(
            "{label} · limits read {}",
            age(now_ms - limits.fetched_at_ms)
        )
    }
}

fn strip(snapshot: &Snapshot) -> Line<'static> {
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
    Line::from(vec![
        Span::styled(format!(" ${combined:.2}"), Modifier::BOLD),
        Span::styled(
            format!(
                " today across {} accounts{estimated}",
                snapshot.accounts.len()
            ),
            MUTED,
        ),
    ])
}

fn draw_roomy(frame: &mut Frame, area: Rect, snapshot: &Snapshot, overview: Overview) -> Rect {
    let [strip_area, today_row, limits_row, rest] = Layout::vertical([
        Constraint::Length(STRIP_ROWS),
        Constraint::Length(overview.today_height()),
        Constraint::Length(overview.limits_height()),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(strip(snapshot)), strip_area);
    let widths: Vec<u16> = overview.columns.iter().map(|column| column.width).collect();
    let today_areas = columns(today_row, &widths);
    let limits_areas = columns(limits_row, &widths);
    for ((column, today_area), limits_area) in overview
        .columns
        .into_iter()
        .zip(today_areas)
        .zip(limits_areas)
    {
        render(frame, column.today, today_area);
        render(frame, column.limits, limits_area);
    }
    rest
}

fn columns(row: Rect, widths: &[u16]) -> Vec<Rect> {
    let mut constraints = Vec::new();
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            constraints.push(Constraint::Length(COLUMN_GAP));
        }
        constraints.push(Constraint::Length(*width));
    }
    Layout::horizontal(constraints)
        .split(row)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn render(frame: &mut Frame, pane: Pane, area: Rect) {
    frame.render_widget(Paragraph::new(pane.lines).block(section(&pane.title)), area);
}

fn compact_lines(
    snapshot: &Snapshot,
    now_ms: i64,
    width: u16,
    before_today: bool,
) -> Vec<(&AccountToday, Vec<Line<'static>>)> {
    let inner_width = inner(width);
    snapshot
        .accounts
        .iter()
        .map(|account| {
            let mut lines = limit_lines(
                account,
                &snapshot.problems,
                now_ms,
                &|_| String::new(),
                inner_width,
                LimitLayout::Stacked,
            );
            if before_today {
                lines.push(Line::default());
            }
            lines.extend(today_lines(account, &snapshot.problems, inner_width, true));
            (account, lines)
        })
        .collect()
}

fn compact_height(snapshot: &Snapshot, now_ms: i64, width: u16) -> u16 {
    let blocks: usize = compact_lines(snapshot, now_ms, width, false)
        .iter()
        .map(|(_, lines)| lines.len() + BORDER_ROWS)
        .sum();
    blocks as u16 + SUMMARY_ROWS
}

#[derive(Clone, Copy, Default)]
struct Spacing {
    between_boxes: bool,
    before_today: bool,
}

impl Spacing {
    fn rows(self, accounts: u16) -> u16 {
        let between = if self.between_boxes {
            accounts.saturating_sub(1)
        } else {
            0
        };
        let before = if self.before_today { accounts } else { 0 };
        between + before
    }
}

fn compact_spacing(
    snapshot: &Snapshot,
    now_ms: i64,
    width: u16,
    room: u16,
    with_sessions: bool,
) -> Spacing {
    if !with_sessions {
        return Spacing::default();
    }
    let accounts = snapshot.accounts.len() as u16;
    let overview = compact_height(snapshot, now_ms, width);
    let unspaced_room = room.saturating_sub(overview + HISTORY_MIN_HEIGHT);
    let sessions = sessions::height(
        &Running::every(&snapshot.accounts),
        width,
        unspaced_room,
        false,
    );
    let base = overview + sessions + HISTORY_MIN_HEIGHT;
    [
        Spacing {
            between_boxes: true,
            before_today: true,
        },
        Spacing {
            between_boxes: true,
            before_today: false,
        },
    ]
    .into_iter()
    .find(|spacing| base + spacing.rows(accounts) <= room)
    .unwrap_or_default()
}

fn draw_compact(
    frame: &mut Frame,
    area: Rect,
    snapshot: &Snapshot,
    now_ms: i64,
    spacing: Spacing,
) -> Rect {
    let blocks = compact_lines(snapshot, now_ms, area.width, spacing.before_today);
    let gap = u16::from(spacing.between_boxes);
    let bottom = area.bottom();
    let row = |y: u16, height: u16| Rect {
        y,
        height: height.min(bottom.saturating_sub(y)),
        ..area
    };
    frame.render_widget(Paragraph::new(strip(snapshot)), row(area.y, SUMMARY_ROWS));
    let mut y = area.y + SUMMARY_ROWS;
    for (index, (account, lines)) in blocks.into_iter().enumerate() {
        if index > 0 {
            y += gap;
        }
        let block = row(y, (lines.len() + BORDER_ROWS) as u16);
        if block.height == 0 {
            break;
        }
        y += block.height;
        frame.render_widget(
            Paragraph::new(lines).block(section(&account.account_label)),
            block,
        );
    }
    row(y.min(bottom), bottom.saturating_sub(y))
}
