use std::collections::BTreeMap;

use crate::config::{CreditReset, days_from_civil, epoch_ms_from_text};
use crate::contract::{
    DollarPace, ForecastBasis, Limit, LimitForecast, LimitPace, PaceBasis, PaceState, SessionSpend,
};
use crate::limits::REFRESH_INTERVAL_MS;
use crate::sources::claude_json::Account;
use crate::store::{self, LimitPoint, Store};

pub const USUAL_DAYS: u32 = 20;

pub fn dollar_pace(
    store: &Store,
    account: Option<&Account>,
    now_ms: i64,
) -> store::Result<DollarPace> {
    let spend = store.spend_by_clock_time(account, now_ms, USUAL_DAYS)?;
    Ok(DollarPace {
        today_usd: spend.today_usd,
        usual_usd: median(spend.earlier_days_usd.clone()),
        usual_days: spend.earlier_days_usd.len() as u32,
        earlier_days_usd: spend.earlier_days_usd.into_iter().rev().collect(),
        clock: spend.clock,
        weekend: spend.weekend,
        earlier_day_totals_usd: spend.earlier_day_totals_usd.into_iter().rev().collect(),
        earlier_days: spend.earlier_days.into_iter().rev().collect(),
    })
}

const RATE_HISTORY_MS: i64 = 28 * 24 * 3_600_000;
const RATE_MIN_POINTS: f64 = 5.0;

pub const ESTIMATED_KINDS: [&str; 2] = ["session", "weekly_all"];

pub fn usd_per_point(
    store: &Store,
    account: &Account,
    now_ms: i64,
) -> store::Result<BTreeMap<&'static str, f64>> {
    let mut rises_by_kind = Vec::new();
    for kind in ESTIMATED_KINDS {
        rises_by_kind.push((
            kind,
            store.limit_rises(account, kind, now_ms - RATE_HISTORY_MS)?,
        ));
    }
    let all_rises = rises_by_kind.iter().flat_map(|(_, rises)| rises);
    let (Some(from_ms), Some(until_ms)) = (
        all_rises.clone().map(|rise| rise.from_ms).min(),
        all_rises.map(|rise| rise.to_ms).max(),
    ) else {
        return Ok(BTreeMap::new());
    };
    let spend = store.spend_times(account, from_ms, until_ms)?;
    let mut spent_before = Vec::with_capacity(spend.len() + 1);
    spent_before.push(0.0);
    for (_, usd) in &spend {
        spent_before.push(spent_before.last().copied().unwrap_or_default() + usd);
    }
    let spent_by = |ms: i64| spent_before[spend.partition_point(|(at_ms, _)| *at_ms <= ms)];
    let mut rates = BTreeMap::new();
    for (kind, rises) in &rises_by_kind {
        let mut days: Vec<(&str, f64, f64)> = Vec::new();
        for rise in rises {
            let usd = spent_by(rise.to_ms) - spent_by(rise.from_ms);
            match days.last_mut() {
                Some((day, points, spent)) if *day == rise.day => {
                    *points += rise.points;
                    *spent += usd;
                }
                _ => days.push((&rise.day, rise.points, usd)),
            }
        }
        let highest = days
            .into_iter()
            .filter(|(_, points, _)| *points >= RATE_MIN_POINTS)
            .map(|(_, points, usd)| usd / points)
            .filter(|rate| *rate > 0.0)
            .reduce(f64::max);
        if let Some(rate) = highest {
            rates.insert(*kind, rate);
        }
    }
    Ok(rates)
}

pub fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        count if count % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.0),
    }
}

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const SESSION_WINDOW_MS: i64 = 5 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;
const USUAL_WEEKS: i64 = 4;
const EVEN_WITHIN_POINTS: f64 = 5.0;
pub const USUAL_HISTORY_MS: i64 = (USUAL_WEEKS + 1) * WEEK_MS;

pub fn limit_pace(
    limit: &Limit,
    fetched_at_ms: i64,
    now_ms: i64,
    history: &[LimitPoint],
    credit_reset: Option<&CreditReset>,
) -> Option<LimitPace> {
    let resets_ms = limit.resets_at.as_deref().and_then(epoch_ms_from_text);
    let (start_ms, end_ms, provisional) = match (limit.kind.as_str(), resets_ms) {
        ("session", Some(resets_ms)) => (resets_ms - SESSION_WINDOW_MS, resets_ms, false),
        ("weekly_all" | "weekly_scoped", Some(resets_ms)) => {
            (resets_ms - WEEK_MS, resets_ms, false)
        }
        ("credits", None) => {
            let (start_ms, end_ms) =
                credit_month(credit_reset.unwrap_or(&CreditReset::PROVISIONAL), now_ms);
            if fetched_at_ms < start_ms {
                return None;
            }
            (start_ms, end_ms, credit_reset.is_none())
        }
        _ => return None,
    };
    if now_ms < start_ms || now_ms >= end_ms {
        return None;
    }
    let elapsed_ms = now_ms - start_ms;
    let usual = match limit.kind.as_str() {
        "weekly_all" | "weekly_scoped" => {
            usual_weekly_percent(limit, start_ms, elapsed_ms, history)
        }
        _ => None,
    };
    let (basis, expected_percent) = match usual {
        Some(usual) => (PaceBasis::Usual, usual),
        None => (
            PaceBasis::Even,
            elapsed_ms as f64 * 100.0 / (end_ms - start_ms) as f64,
        ),
    };
    let state = if (limit.percent - expected_percent).abs() <= EVEN_WITHIN_POINTS {
        PaceState::Even
    } else if limit.percent < expected_percent {
        PaceState::Under
    } else {
        PaceState::Over
    };
    Some(LimitPace {
        state,
        basis,
        expected_percent,
        window_start_ms: start_ms,
        window_end_ms: end_ms,
        provisional,
    })
}

const FORECAST_AFTER_WINDOW_SHARE: f64 = 0.1;
pub const RECENT_MS: i64 = 30 * 60_000;
const FETCH_LATE_MS: i64 = 5 * 60_000;

pub fn limit_forecast(
    limit: &Limit,
    pace: &LimitPace,
    fetched_at_ms: i64,
    recent: &[LimitPoint],
) -> Option<LimitForecast> {
    if limit.percent <= 0.0 || limit.percent >= 100.0 {
        return None;
    }
    let window = window_full_at(limit, pace, fetched_at_ms).map(|full_at_ms| LimitForecast {
        full_at_ms,
        basis: ForecastBasis::Window,
    });
    let recent =
        recent_full_at(limit, pace, fetched_at_ms, recent).map(|full_at_ms| LimitForecast {
            full_at_ms,
            basis: ForecastBasis::Recent,
        });
    [window, recent]
        .into_iter()
        .flatten()
        .filter(|forecast| forecast.full_at_ms < pace.window_end_ms)
        .min_by_key(|forecast| forecast.full_at_ms)
}

fn window_full_at(limit: &Limit, pace: &LimitPace, fetched_at_ms: i64) -> Option<i64> {
    let elapsed_ms = (fetched_at_ms - pace.window_start_ms) as f64;
    let window_ms = (pace.window_end_ms - pace.window_start_ms) as f64;
    if elapsed_ms < window_ms * FORECAST_AFTER_WINDOW_SHARE {
        return None;
    }
    let elapsed_percent = elapsed_ms * 100.0 / window_ms;
    let near_full = 100.0 - limit.percent <= EVEN_WITHIN_POINTS;
    if !near_full && limit.percent - elapsed_percent <= EVEN_WITHIN_POINTS {
        return None;
    }
    Some(pace.window_start_ms + (elapsed_ms * 100.0 / limit.percent).round() as i64)
}

fn recent_full_at(
    limit: &Limit,
    pace: &LimitPace,
    fetched_at_ms: i64,
    recent: &[LimitPoint],
) -> Option<i64> {
    let since_ms = fetched_at_ms - RECENT_MS;
    if limit.kind != "session" || since_ms < pace.window_start_ms {
        return None;
    }
    let then = recent
        .iter()
        .filter(|point| point.fetched_at_ms <= since_ms)
        .max_by_key(|point| point.fetched_at_ms)?;
    if since_ms - then.fetched_at_ms > REFRESH_INTERVAL_MS + FETCH_LATE_MS {
        return None;
    }
    let then_percent = if then.fetched_at_ms < pace.window_start_ms {
        0.0
    } else {
        then.percent
    };
    let rise = limit.percent - then_percent;
    if rise <= 0.0 {
        return None;
    }
    let left_ms = (100.0 - limit.percent) * RECENT_MS as f64 / rise;
    Some(fetched_at_ms + left_ms.round() as i64)
}

fn usual_weekly_percent(
    limit: &Limit,
    start_ms: i64,
    elapsed_ms: i64,
    history: &[LimitPoint],
) -> Option<f64> {
    let points: Vec<&LimitPoint> = history
        .iter()
        .filter(|point| point.kind == limit.kind && point.scope == limit.scope)
        .collect();
    let first_ms = points.iter().map(|point| point.fetched_at_ms).min()?;
    if first_ms > start_ms - USUAL_WEEKS * WEEK_MS {
        return None;
    }
    let mut past_weeks = Vec::new();
    for weeks_back in 1..=USUAL_WEEKS {
        let week_start_ms = start_ms - weeks_back * WEEK_MS;
        let in_week: Vec<&&LimitPoint> = points
            .iter()
            .filter(|point| (week_start_ms..week_start_ms + WEEK_MS).contains(&point.fetched_at_ms))
            .collect();
        if in_week.is_empty() {
            return None;
        }
        let by_this_point = in_week
            .into_iter()
            .filter(|point| point.fetched_at_ms <= week_start_ms + elapsed_ms)
            .max_by_key(|point| point.fetched_at_ms)
            .map_or(0.0, |point| point.percent);
        past_weeks.push(by_this_point);
    }
    median(past_weeks)
}

fn credit_month(reset: &CreditReset, now_ms: i64) -> (i64, i64) {
    let offset_ms = i64::from(reset.offset_minutes) * 60_000;
    let (year, month, _) = civil_from_days((now_ms + offset_ms).div_euclid(DAY_MS));
    let reset_in = |year: i64, month: u8| {
        let day = reset.day.min(days_in_month(year, month));
        days_from_civil(year, month, day) * DAY_MS + i64::from(reset.minute_of_day) * 60_000
            - offset_ms
    };
    let (mut year, mut month) = (year, month);
    if reset_in(year, month) > now_ms {
        (year, month) = previous_month(year, month);
    }
    let (next_year, next_month) = next_month(year, month);
    (reset_in(year, month), reset_in(next_year, next_month))
}

fn previous_month(year: i64, month: u8) -> (i64, u8) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

fn next_month(year: i64, month: u8) -> (i64, u8) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn days_in_month(year: i64, month: u8) -> u8 {
    let (next_year, next_month) = next_month(year, month);
    (days_from_civil(next_year, next_month, 1) - days_from_civil(year, month, 1)) as u8
}

// Howard Hinnant's civil_from_days, the inverse of days_from_civil.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
    let month = (if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    }) as u8;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub const FASTEST_WITHIN_MS: i64 = 15 * 60_000;

pub fn fastest_session(
    store: &Store,
    account: Option<&Account>,
    now_ms: i64,
) -> store::Result<Option<SessionSpend>> {
    Ok(active_sessions(store, account, now_ms)?.into_iter().next())
}

pub fn active_sessions(
    store: &Store,
    account: Option<&Account>,
    now_ms: i64,
) -> store::Result<Vec<SessionSpend>> {
    store.sessions_spending_since(account, now_ms - FASTEST_WITHIN_MS, now_ms)
}

pub fn sessions_today(
    store: &Store,
    account: Option<&Account>,
    now_ms: i64,
) -> store::Result<Vec<SessionSpend>> {
    let mut sessions =
        store.sessions_spending_today(account, now_ms - FASTEST_WITHIN_MS, now_ms)?;
    sessions.sort_by(|a, b| {
        b.today_usd
            .total_cmp(&a.today_usd)
            .then_with(|| b.last_active_ms.cmp(&a.last_active_ms))
    });
    Ok(sessions)
}
