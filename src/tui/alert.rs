use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::sections::duration;
use crate::contract::Snapshot;
use crate::limits::short_limit_name;
use crate::status::{MAY_MISS_OUTSIDE_USE_MS, has_reset, resets_ms};

const QUOTE: &str = "1.21 gigawatts! ";
const RED: Color = Color::Red;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trouble {
    Out { back_at_ms: i64 },
    Filling { full_at_ms: i64 },
}

#[derive(Debug, Clone)]
pub(crate) struct Alert {
    account: Option<String>,
    account_label: String,
    limit: String,
    trouble: Trouble,
}

pub(crate) fn alerts(snapshot: &Snapshot, now_ms: i64) -> Vec<Alert> {
    let mut found = Vec::new();
    for account in &snapshot.accounts {
        let Some(limits) = &account.limits else {
            continue;
        };
        let stale = now_ms - limits.fetched_at_ms > MAY_MISS_OUTSIDE_USE_MS;
        for shown in &limits.limits {
            if has_reset(shown, now_ms) {
                continue;
            }
            let trouble = if shown.limit.percent >= 100.0 {
                resets_ms(shown).map(|back_at_ms| Trouble::Out { back_at_ms })
            } else if stale {
                None
            } else {
                shown.forecast.map(|forecast| Trouble::Filling {
                    full_at_ms: forecast.full_at_ms,
                })
            };
            if let Some(trouble) = trouble {
                found.push(Alert {
                    account: account.account.clone(),
                    account_label: account.account_label.clone(),
                    limit: short_limit_name(&shown.limit),
                    trouble,
                });
            }
        }
    }
    found.sort_by_key(|alert| match alert.trouble {
        Trouble::Out { back_at_ms } => (0, -back_at_ms),
        Trouble::Filling { full_at_ms } => (1, full_at_ms),
    });
    found
}

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
    match worst.trouble {
        Trouble::Out { back_at_ms } => {
            text.push_str(&format!(" back in {}", duration(back_at_ms - now_ms)));
        }
        Trouble::Filling { full_at_ms } => {
            text.push_str(&format!(" full in {}", rough(full_at_ms - now_ms)));
        }
    }
    if alerts.len() > 1 {
        text.push_str(&format!(" +{}", alerts.len() - 1));
    }
    vec![
        Span::styled("▐", Style::new().fg(RED)),
        Span::styled(
            format!(" {text} "),
            Style::new()
                .bg(RED)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("▌", Style::new().fg(RED)),
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
