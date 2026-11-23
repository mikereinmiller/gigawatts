use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::contract::{AccountToday, Problem, Snapshot};
use crate::limits::age;

use super::HISTORY_MIN_HEIGHT;
use super::digits;
use super::extras::{Extras, Step, cards_height, grow};
use super::sections::{
    COLUMN_GAP, LimitLayout, LocalMinute, SIDE_BY_SIDE_FROM, limit_lines, limit_rows, limits_width,
    section, source_and_age, spent_emphasis, spent_notes, styled_spans, titled_section,
    today_lines, today_panel, usual_notes_width,
};
use super::sessions::{self, SessionsToday};
use super::style::{MUTED, TITLE};

const ROOMY_FROM: u16 = 100;
const THREE_COLUMNS_FROM: u16 = 150;
const BORDER_ROWS: usize = 2;
const BORDER_AND_PADDING_COLUMNS: u16 = 4;
const PANE_TOP_GAPS: u16 = 2;
const TODAY_MIN_WIDTH: usize = 34;
const TODAY_MAX_WIDTH: usize = 44;
const DIVIDER: &str = " │ ";

pub(crate) struct Fit {
    pub(crate) roomy: bool,
    with_sessions: bool,
    extras: Extras,
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
        self.today_height() + self.limits_height()
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
    let running = SessionsToday::every(&snapshot.accounts, now_ms);
    let boxes_rows = |extras: Extras, with_sessions: bool| {
        boxes_height(snapshot, now_ms, local_minute, width)
            + cards_height(&running, width, with_sessions, extras)
            + if extras.spending {
                HISTORY_MIN_HEIGHT
            } else {
                0
            }
    };
    let boxes_with_sessions = with_sessions && boxes_rows(Extras::default(), true) <= height;
    let boxes = if boxes_with_sessions {
        grow(
            Extras::default(),
            &[Step::Spending, Step::CardsGap, Step::SecondRow],
            |extras| boxes_rows(extras, true) <= height,
        )
    } else {
        Extras::default()
    };
    if let Some(overview) = roomy(snapshot, now_ms, local_minute, width) {
        let most_the_boxes_show = if with_sessions {
            sessions::rows_reserved(2, true)
        } else {
            0
        };
        if overview.height() + most_the_boxes_show + HISTORY_MIN_HEIGHT <= height {
            return Fit {
                roomy: true,
                with_sessions,
                extras: Extras {
                    spending: true,
                    cards_gap: true,
                    second_row: true,
                    ..Extras::default()
                },
            };
        }
    }
    Fit {
        roomy: false,
        with_sessions: boxes_with_sessions,
        extras: boxes,
    }
}

pub(crate) fn height(
    snapshot: &Snapshot,
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    height: u16,
    with_sessions: bool,
    beside: u16,
) -> u16 {
    let fit = fit(snapshot, now_ms, local_minute, width, height, with_sessions);
    let overview = match roomy(snapshot, now_ms, local_minute, width) {
        Some(overview) if fit.roomy => make_room(overview, height, beside).height(),
        _ => boxes_height(snapshot, now_ms, local_minute, width),
    };
    let running = SessionsToday::every(&snapshot.accounts, now_ms);
    overview + cards_height(&running, width, fit.with_sessions, fit.extras)
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
    beside: u16,
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
        Some(overview) if fit.roomy => draw_roomy(frame, area, make_room(overview, room, beside)),
        _ => draw_boxes(
            frame,
            area,
            account_boxes(snapshot, now_ms, local_minute, area.width),
        ),
    };
    if fit.with_sessions {
        sessions::draw(
            frame,
            rest,
            &SessionsToday::every(&snapshot.accounts, now_ms),
            now_ms,
            fit.extras.cards(),
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
                lines: padded(today_panel(account, &snapshot.problems, inner(width))),
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

fn make_room(mut overview: Overview, height: u16, beside: u16) -> Overview {
    let short = (overview.height() + beside).saturating_sub(height);
    if short == 0 || short > PANE_TOP_GAPS {
        return overview;
    }
    for column in &mut overview.columns {
        column.limits.lines.remove(0);
        if short == PANE_TOP_GAPS {
            column.today.lines.remove(0);
        }
    }
    overview
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

fn draw_roomy(frame: &mut Frame, area: Rect, overview: Overview) -> Rect {
    let [today_row, limits_row, rest] = Layout::vertical([
        Constraint::Length(overview.today_height()),
        Constraint::Length(overview.limits_height()),
        Constraint::Fill(1),
    ])
    .areas(area);
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

struct AccountBox {
    title: Line<'static>,
    lines: Vec<Line<'static>>,
}

fn account_boxes(
    snapshot: &Snapshot,
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
) -> Vec<AccountBox> {
    let today_width = today_width(snapshot);
    snapshot
        .accounts
        .iter()
        .map(|account| {
            account_box(
                account,
                &snapshot.problems,
                now_ms,
                local_minute,
                width,
                today_width,
            )
        })
        .collect()
}

fn today_width(snapshot: &Snapshot) -> u16 {
    snapshot
        .accounts
        .iter()
        .map(usual_notes_width)
        .max()
        .unwrap_or(0)
        .clamp(TODAY_MIN_WIDTH, TODAY_MAX_WIDTH) as u16
}

fn account_box(
    account: &AccountToday,
    problems: &[Problem],
    now_ms: i64,
    local_minute: LocalMinute,
    width: u16,
    today_width: u16,
) -> AccountBox {
    let inner_width = inner(width);
    let title = box_title(account, now_ms, inner_width);
    let layout = LimitLayout::Full;
    if width < SIDE_BY_SIDE_FROM {
        let mut lines = today_lines(account, problems, inner_width);
        lines.extend(
            limit_rows(account, problems, now_ms, local_minute, inner_width, layout).lines(0),
        );
        return AccountBox { title, lines };
    }
    let today = today_column(account, problems, today_width);
    let limits_width = inner_width.saturating_sub(today_width + DIVIDER.chars().count() as u16);
    let limits = limit_rows(
        account,
        problems,
        now_ms,
        local_minute,
        limits_width,
        layout,
    )
    .spread(0, today.len());
    let rows = today.len().max(limits.len());
    let lines = (0..rows)
        .map(|row| {
            let left = today.get(row).cloned().unwrap_or_default();
            let pad = (today_width as usize).saturating_sub(left.width());
            let mut spans = styled_spans(left);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(DIVIDER, MUTED));
            spans.extend(styled_spans(limits.get(row).cloned().unwrap_or_default()));
            Line::from(spans)
        })
        .collect();
    AccountBox { title, lines }
}

fn small_spent(account: &AccountToday) -> Option<[String; digits::SMALL_ROWS]> {
    let [first, second, third] = digits::small(&format!("{:.2}", account.cost_usd))?;
    Some([
        format!("$ {first}"),
        format!("  {second}"),
        format!("  {third}"),
    ])
}

fn today_column(account: &AccountToday, problems: &[Problem], width: u16) -> Vec<Line<'static>> {
    let emphasis = spent_emphasis(account);
    let small = small_spent(account).filter(|rows| rows[0].chars().count() <= width as usize);
    let mut lines = if let Some(rows) = small {
        let mut lines = vec![Line::default()];
        lines.extend(rows.into_iter().map(|row| Line::styled(row, emphasis)));
        lines
    } else {
        vec![Line::from(vec![
            Span::styled(format!("${:.2}", account.cost_usd), emphasis),
            Span::styled(" today", MUTED),
        ])]
    };
    lines.extend(spent_notes(account, problems, width));
    lines
}

fn box_title(account: &AccountToday, now_ms: i64, width: u16) -> Line<'static> {
    let label = account.account_label.clone();
    let room = (width as usize).saturating_sub(label.chars().count());
    let source = account
        .limits
        .as_ref()
        .filter(|limits| !limits.limits.is_empty())
        .map(|limits| {
            let full = format!(" · limits {}", source_and_age(limits, now_ms));
            let short = format!(" · limits read {}", age(now_ms - limits.fetched_at_ms));
            [full, short]
                .into_iter()
                .find(|text| text.chars().count() <= room)
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Line::from(vec![
        Span::styled(format!(" {label}"), TITLE),
        Span::styled(source, MUTED),
        Span::raw(" "),
    ])
}

fn boxes_height(snapshot: &Snapshot, now_ms: i64, local_minute: LocalMinute, width: u16) -> u16 {
    account_boxes(snapshot, now_ms, local_minute, width)
        .iter()
        .map(|account| account.lines.len() + BORDER_ROWS)
        .sum::<usize>() as u16
}

fn draw_boxes(frame: &mut Frame, area: Rect, boxes: Vec<AccountBox>) -> Rect {
    let bottom = area.bottom();
    let row = |y: u16, height: u16| Rect {
        y,
        height: height.min(bottom.saturating_sub(y)),
        ..area
    };
    let mut y = area.y;
    for account in boxes {
        let block = row(y, (account.lines.len() + BORDER_ROWS) as u16);
        if block.height == 0 {
            break;
        }
        y += block.height;
        frame.render_widget(
            Paragraph::new(account.lines).block(titled_section(account.title)),
            block,
        );
    }
    row(y.min(bottom), bottom.saturating_sub(y))
}
