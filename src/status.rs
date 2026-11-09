use crate::config::epoch_ms_from_text;
use crate::contract::{AccountToday, DollarPace, LimitPace, PaceBasis, PaceState, PacedLimit};
use crate::limits::{age, limit_name, percent};
use crate::pace::USUAL_DAYS;

pub(crate) const MAY_MISS_OUTSIDE_USE_MS: i64 = 60 * 60_000;

pub fn render_line(
    account: &AccountToday,
    now_ms: i64,
    local_minute: &dyn Fn(i64) -> String,
) -> String {
    let limit = match &account.limits {
        Some(limits) if !limits.limits.is_empty() => {
            limit_part(&limits.limits, limits.fetched_at_ms, now_ms, local_minute)
        }
        _ => "no limit readings yet".to_string(),
    };
    format!(
        "{}: {limit} · today {}",
        account.account_label,
        spend_today(account)
    )
}

pub(crate) fn spend_today(account: &AccountToday) -> String {
    let estimated = if account.cost_estimated {
        ", estimated"
    } else {
        ""
    };
    format!(
        "${:.2}{estimated}{}",
        account.cost_usd,
        against_usual(&account.pace)
    )
}

fn limit_part(
    limits: &[PacedLimit],
    fetched_at_ms: i64,
    now_ms: i64,
    local_minute: &dyn Fn(i64) -> String,
) -> String {
    let closest = limits
        .iter()
        .filter(|limit| !has_reset(limit, now_ms))
        .max_by(|a, b| a.limit.percent.total_cmp(&b.limit.percent));
    let elapsed_ms = now_ms - fetched_at_ms;
    match closest {
        None => format!(
            "limits reset since the last reading, read {}",
            age(elapsed_ms)
        ),
        Some(limit) => {
            let mut parts = vec![closest_limit(limit)];
            if let Some(resets_ms) = resets_ms(limit) {
                parts.push(format!("resets {}", local_minute(resets_ms)));
            }
            parts.push(format!("read {}", age(elapsed_ms)));
            if elapsed_ms > MAY_MISS_OUTSIDE_USE_MS {
                parts.push("may not include use outside Claude Code".to_string());
            }
            parts.join(", ")
        }
    }
}

pub(crate) fn has_reset(shown: &PacedLimit, now_ms: i64) -> bool {
    match resets_ms(shown) {
        Some(resets_ms) => resets_ms <= now_ms,
        None => shown.limit.kind == "credits",
    }
}

fn closest_limit(shown: &PacedLimit) -> String {
    let mut text = format!(
        "{} {}%",
        limit_name(&shown.limit),
        percent(shown.limit.percent)
    );
    if let Some(pace) = &shown.pace {
        text.push_str(&format!(" {}", pace_phrase(pace)));
    }
    text
}

pub(crate) fn pace_phrase(pace: &LimitPace) -> String {
    let state = match pace.state {
        PaceState::Under => "under",
        PaceState::Even => "on",
        PaceState::Over => "over",
    };
    let basis = match pace.basis {
        PaceBasis::Even => "even pace",
        PaceBasis::Usual => "your usual pace",
    };
    let mut text = format!("{state} {basis}");
    if pace.provisional {
        text.push_str(" (provisional until credit_reset is set)");
    }
    text
}

pub(crate) fn resets_ms(shown: &PacedLimit) -> Option<i64> {
    match &shown.limit.resets_at {
        Some(resets_at) => epoch_ms_from_text(resets_at),
        None => shown.pace.as_ref().map(|pace| pace.window_end_ms),
    }
}

pub(crate) fn against_usual(pace: &DollarPace) -> String {
    let Some(usual_usd) = pace.usual_usd else {
        return ", no usual yet".to_string();
    };
    let mut text = match pace.times_usual() {
        Some(times) => format!(", {times:.1}× your usual by {}", pace.clock),
        None => format!(", usual by {} is ${usual_usd:.2}", pace.clock),
    };
    if pace.usual_days < USUAL_DAYS {
        text.push_str(&format!(
            " (provisional, {} of {USUAL_DAYS} days)",
            pace.usual_days
        ));
    }
    text
}
