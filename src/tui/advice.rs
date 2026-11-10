use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use serde_json::Value;

use super::history::LoadedAdvice;
use super::sections::section;
use super::style::{DANGER, MUTED};
use crate::limits::age;

const WIDTH: u16 = 76;

pub(crate) fn draw(frame: &mut Frame, area: Rect, advice: Option<&LoadedAdvice>, now_ms: i64) {
    let (title, lines) = match advice {
        None => (
            "Usage advice".to_string(),
            vec![Line::styled("Reading…", MUTED)],
        ),
        Some(Err(error)) => (
            "Usage advice".to_string(),
            vec![Line::styled(
                format!("Can't read usage advice: {error}"),
                DANGER,
            )],
        ),
        Some(Ok(None)) => (
            "Usage advice".to_string(),
            vec![Line::styled(
                "No usage advice saved today. The engine saves Anthropic's advice hourly while Claude Code is active.",
                MUTED,
            )],
        ),
        Some(Ok(Some(advice))) => (
            format!("Usage advice · read {}", age(now_ms - advice.fetched_at_ms)),
            periods(&advice.behaviors),
        ),
    };
    let width = WIDTH.min(area.width);
    let inner_width = width.saturating_sub(4).max(1) as usize;
    let rows: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(inner_width))
        .sum();
    let popup = super::centered(area, width, rows as u16 + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(section(&title)),
        popup,
    );
}

fn periods(behaviors: &Value) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        "Percentages are each pattern's share of your usage.",
        MUTED,
    )];
    for (key, name) in [("day", "Today"), ("week", "This week")] {
        let Some(period) = behaviors.get(key) else {
            continue;
        };
        lines.push(Line::default());
        lines.extend(period_lines(name, period));
    }
    lines
}

fn period_lines(name: &str, period: &Value) -> Vec<Line<'static>> {
    let count = |field: &str| period.get(field).and_then(Value::as_u64);
    let mut heading = vec![Span::styled(name.to_string(), Modifier::BOLD)];
    if let (Some(sessions), Some(requests)) = (count("session_count"), count("request_count")) {
        heading.push(Span::styled(
            format!(" · {sessions} sessions, {requests} requests"),
            MUTED,
        ));
    }
    let mut lines = vec![Line::from(heading)];
    let recommendations = list(period, "recommendations");
    if recommendations.is_empty() {
        lines.push(Line::styled("Nothing flagged.", MUTED));
    }
    for recommendation in recommendations {
        let pct = recommendation
            .get("pct")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let what = text(recommendation, "title")
            .or_else(|| text(recommendation, "key").map(|key| key.replace('_', " ")))
            .unwrap_or_default();
        let mut spans = vec![Span::raw(format!("{pct:>4.0}%  {what}"))];
        if let Some(detail) = text(recommendation, "detail") {
            spans.push(Span::styled(format!(" · {detail}"), MUTED));
        }
        lines.push(Line::from(spans));
    }
    for (field, name) in [
        ("agents", "Agents"),
        ("skills", "Skills"),
        ("mcp_servers", "MCP servers"),
        ("plugins", "Plugins"),
    ] {
        let shares: Vec<String> = list(period, field)
            .iter()
            .filter_map(|share| {
                let pct = share.get("pct").and_then(Value::as_f64)?;
                Some(format!("{} {pct:.0}%", text(share, "name")?))
            })
            .collect();
        if !shares.is_empty() {
            lines.push(Line::styled(
                format!("{name}: {}", shares.join(" · ")),
                MUTED,
            ));
        }
    }
    lines
}

fn list<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value
        .get(field)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn text(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}
