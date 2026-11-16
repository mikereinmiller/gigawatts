use crate::contract::{AccountToday, Alert, AlertState};
use crate::limits::short_limit_name;
use crate::status::{MAY_MISS_OUTSIDE_USE_MS, has_reset, resets_ms};

pub fn alerts(accounts: &[AccountToday], now_ms: i64) -> Vec<Alert> {
    let mut found = Vec::new();
    for account in accounts {
        let Some(limits) = &account.limits else {
            continue;
        };
        let stale = now_ms - limits.fetched_at_ms > MAY_MISS_OUTSIDE_USE_MS;
        for shown in &limits.limits {
            if has_reset(shown, now_ms) {
                continue;
            }
            let trouble = if shown.limit.percent >= 100.0 {
                resets_ms(shown).map(|back_at_ms| (AlertState::Full, back_at_ms))
            } else if stale {
                None
            } else {
                shown
                    .forecast
                    .map(|forecast| (AlertState::Filling, forecast.full_at_ms.max(now_ms)))
            };
            if let Some((state, at_ms)) = trouble {
                found.push(Alert {
                    account: account.account.clone(),
                    account_label: account.account_label.clone(),
                    limit: short_limit_name(&shown.limit),
                    state,
                    at_ms,
                });
            }
        }
    }
    found.sort_by_key(|alert| match alert.state {
        AlertState::Full => (0, -alert.at_ms),
        AlertState::Filling => (1, alert.at_ms),
    });
    found
}
