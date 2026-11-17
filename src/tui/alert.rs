use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::sections::duration;
use super::style::OVER;
use crate::contract::{Alert, AlertState};

const QUOTE: &str = "1.21 gigawatts! ";

pub(crate) struct Light {
    pub(crate) with_quote: bool,
    pub(crate) with_account: bool,
}

pub(crate) fn light(
    alerts: &[Alert],
    shown_account: Option<&str>,
    stacked: bool,
    style: Light,
    now_ms: i64,
) -> Vec<Span<'static>> {
    let Some(worst) = alerts.first() else {
        return Vec::new();
    };
    let mut text = "ϟ ".to_string();
    if style.with_quote {
        text.push_str(QUOTE);
    }
    let other_account = worst.account.as_deref() != shown_account;
    if style.with_account && (stacked || other_account) {
        text.push_str(&format!("{} · ", worst.account_label));
    }
    text.push_str(&worst.limit);
    match worst.state {
        AlertState::Full => {
            text.push_str(&format!(" back in {}", duration(worst.at_ms - now_ms)));
        }
        AlertState::Filling => {
            text.push_str(&format!(" full in {}", rough(worst.at_ms - now_ms)));
        }
    }
    if alerts.len() > 1 {
        text.push_str(&format!(" +{}", alerts.len() - 1));
    }
    vec![
        Span::styled("▐", Style::new().fg(OVER)),
        Span::styled(
            format!(" {text} "),
            Style::new()
                .bg(OVER)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("▌", Style::new().fg(OVER)),
    ]
}

fn rough(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    let tens = ((minutes + 5) / 10 * 10).max(10);
    if tens >= 24 * 60 {
        let hours = (minutes + 30) / 60;
        format!("~{}d {}h", hours / 24, hours % 24)
    } else if tens >= 60 {
        format!("~{}h {}m", tens / 60, tens % 60)
    } else {
        format!("~{tens}m")
    }
}
