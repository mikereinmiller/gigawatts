use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::path::PathBuf;

use crate::accounts::{Directory, attribution_for};
use crate::config::Config;
use crate::contract::{Limit, LimitSource};
use crate::sources::claude_json::{Account, read_cached_usage, read_login};
use crate::sources::get_usage::{self, Runner};
use crate::store::{self, LimitsReading, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableUsage {
    pub claude_json: PathBuf,
    pub error: String,
}

pub fn read_cached(
    store: &mut Store,
    directories: &[Directory],
    config: &Config,
) -> store::Result<Vec<UnreadableUsage>> {
    let mut unreadable = Vec::new();
    for directory in directories
        .iter()
        .filter(|directory| directory.unreadable.is_none())
    {
        let cached = match read_cached_usage(&directory.claude_json) {
            Ok(Some(cached)) => cached,
            Ok(None) => continue,
            Err(error) => {
                unreadable.push(UnreadableUsage {
                    claude_json: directory.claude_json.clone(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        let mut attribution = attribution_for(&directory.path, directories, config);
        attribution.seen_logins = store.seen_logins(&directory.path.to_string_lossy())?;
        let Some(account) = attribution
            .account_at(Some(cached.fetched_at_ms))
            .filter(|account| account.account_uuid == cached.account_uuid)
            .cloned()
        else {
            continue;
        };
        if !attribution.tracks(Some(&account)) {
            continue;
        }
        store.record_limits(
            &account,
            &LimitsReading {
                fetched_at_ms: cached.fetched_at_ms,
                source: LimitSource::CachedUsageUtilization,
                limits: cached.limits,
            },
        )?;
    }
    Ok(unreadable)
}

pub const REFRESH_INTERVAL_MS: i64 = 10 * 60_000;
pub const ADVICE_INTERVAL_MS: i64 = 60 * 60_000;
pub const LONGEST_RETRY_MS: i64 = 60 * 60_000;
pub const ACTIVE_FOR_MS: i64 = 10 * 60_000;

#[derive(Debug, Default)]
pub struct Refresher {
    accounts: BTreeMap<Account, Tracked>,
}

#[derive(Debug)]
struct Tracked {
    active_until_ms: i64,
    next_ms: i64,
    last_attempt_ms: Option<i64>,
    failures: u32,
    failing: Option<(i64, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failing {
    pub account: Account,
    pub since_ms: i64,
    pub error: String,
}

impl Refresher {
    pub fn activity(&mut self, account: &Account, now_ms: i64, last_fetched_ms: Option<i64>) {
        let tracked = self.tracked(account, now_ms, last_fetched_ms);
        tracked.active_until_ms = tracked.active_until_ms.max(now_ms + ACTIVE_FOR_MS);
    }

    pub fn due(&self, now_ms: i64) -> Vec<Account> {
        self.accounts
            .iter()
            .filter(|(_, tracked)| tracked.next_ms <= now_ms && now_ms < tracked.active_until_ms)
            .map(|(account, _)| account.clone())
            .collect()
    }

    pub fn next_due_ms(&self, now_ms: i64) -> Option<i64> {
        self.accounts
            .values()
            .filter(|tracked| now_ms < tracked.active_until_ms)
            .filter(|tracked| tracked.next_ms < tracked.active_until_ms)
            .map(|tracked| tracked.next_ms)
            .min()
    }

    pub fn attempted_within_interval(&self, account: &Account, now_ms: i64) -> bool {
        self.accounts
            .get(account)
            .and_then(|tracked| tracked.last_attempt_ms)
            .is_some_and(|attempt| now_ms - attempt < REFRESH_INTERVAL_MS)
    }

    pub fn succeeded(&mut self, account: &Account, now_ms: i64) {
        let tracked = self.tracked(account, now_ms, None);
        tracked.last_attempt_ms = Some(now_ms);
        tracked.next_ms = now_ms + REFRESH_INTERVAL_MS;
        tracked.failures = 0;
        tracked.failing = None;
    }

    pub fn failed(&mut self, account: &Account, now_ms: i64, error: &str) {
        let tracked = self.tracked(account, now_ms, None);
        tracked.last_attempt_ms = Some(now_ms);
        tracked.failures += 1;
        let doublings = (tracked.failures - 1).min(10);
        let wait = (REFRESH_INTERVAL_MS << doublings).min(LONGEST_RETRY_MS);
        tracked.next_ms = now_ms + wait;
        let since_ms = tracked.failing.as_ref().map_or(now_ms, |(since, _)| *since);
        tracked.failing = Some((since_ms, error.to_string()));
    }

    pub fn keep_only(&mut self, refreshable: &BTreeSet<Account>) {
        self.accounts
            .retain(|account, _| refreshable.contains(account));
    }

    pub fn retry_after_ms(&self, account: &Account) -> Option<i64> {
        let tracked = self.accounts.get(account)?;
        Some(tracked.next_ms - tracked.last_attempt_ms?)
    }

    pub fn failing(&self) -> Vec<Failing> {
        self.accounts
            .iter()
            .filter_map(|(account, tracked)| {
                let (since_ms, error) = tracked.failing.clone()?;
                Some(Failing {
                    account: account.clone(),
                    since_ms,
                    error,
                })
            })
            .collect()
    }

    fn tracked(
        &mut self,
        account: &Account,
        now_ms: i64,
        last_fetched_ms: Option<i64>,
    ) -> &mut Tracked {
        self.accounts
            .entry(account.clone())
            .or_insert_with(|| Tracked {
                active_until_ms: now_ms,
                next_ms: last_fetched_ms.map_or(now_ms, |fetched| fetched + REFRESH_INTERVAL_MS),
                last_attempt_ms: None,
                failures: 0,
                failing: None,
            })
    }
}

#[derive(Debug)]
pub enum RefreshError {
    Usage(get_usage::Error),
    Store(store::Error),
    LoggedOut,
}

impl fmt::Display for RefreshError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RefreshError::Usage(error) => write!(f, "{error}"),
            RefreshError::Store(error) => {
                write!(f, "can't save limits in the history database: {error}")
            }
            RefreshError::LoggedOut => write!(f, "no account is logged in to this directory"),
        }
    }
}

impl std::error::Error for RefreshError {}

impl From<get_usage::Error> for RefreshError {
    fn from(error: get_usage::Error) -> Self {
        RefreshError::Usage(error)
    }
}

impl From<store::Error> for RefreshError {
    fn from(error: store::Error) -> Self {
        RefreshError::Store(error)
    }
}

pub fn refresh(
    store: &mut Store,
    runner: &dyn Runner,
    directory: &Directory,
    now_ms: &dyn Fn() -> i64,
    today: &str,
) -> Result<(), RefreshError> {
    let account = directory.account().ok_or(RefreshError::LoggedOut)?.clone();
    let asked_ms = now_ms();
    let with_behaviors = store
        .usage_advice(&account, today)?
        .is_none_or(|advice| asked_ms - advice.fetched_at_ms >= ADVICE_INTERVAL_MS);
    let usage = get_usage::fetch(runner, directory.claude_config_dir(), with_behaviors)?;
    let now_ms = now_ms();
    let still_logged_in = read_login(&directory.claude_json)
        .ok()
        .flatten()
        .is_some_and(|login| login.account == account);
    if !still_logged_in {
        return Ok(());
    }
    store.record_limits(
        &account,
        &LimitsReading {
            fetched_at_ms: now_ms,
            source: LimitSource::GetUsage,
            limits: usage.limits,
        },
    )?;
    if let Some(behaviors) = usage.behaviors {
        store.record_usage_advice(&account, today, now_ms, &behaviors)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitsRow {
    pub label: String,
    pub source: LimitSource,
    pub fetched_at_ms: i64,
    pub fetched_local: String,
    pub limits: Vec<ShownLimit>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShownLimit {
    pub limit: Limit,
    pub resets_local: Option<String>,
}

pub fn rows(store: &Store, config: &Config) -> store::Result<Vec<LimitsRow>> {
    let emails: BTreeMap<Account, Option<String>> = store
        .accounts()?
        .into_iter()
        .map(|stored| (stored.account, stored.email))
        .collect();
    let mut rows = Vec::new();
    for (account, reading) in store.latest_limits()? {
        let settings = config.account(&account);
        if !settings.tracking {
            continue;
        }
        let mut limits = Vec::new();
        for limit in reading.limits {
            let resets_local = match &limit.resets_at {
                Some(resets_at) => store.local_minute(resets_at)?,
                None => None,
            };
            limits.push(ShownLimit {
                limit,
                resets_local,
            });
        }
        rows.push(LimitsRow {
            label: settings
                .label
                .or_else(|| emails.get(&account).cloned().flatten())
                .unwrap_or_else(|| account.to_string()),
            source: reading.source,
            fetched_at_ms: reading.fetched_at_ms,
            fetched_local: store
                .local_datetime(reading.fetched_at_ms)?
                .map(|time| time.chars().take(16).collect())
                .unwrap_or_default(),
            limits,
        });
    }
    Ok(rows)
}

pub fn render(rows: &[LimitsRow], now_ms: i64) -> String {
    if rows.is_empty() {
        return "No limit readings yet. The engine reads them while Claude Code is active.\n"
            .to_string();
    }
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let source = match row.source {
            LimitSource::GetUsage => "get_usage",
            LimitSource::CachedUsageUtilization => "cachedUsageUtilization",
        };
        let _ = writeln!(
            out,
            "{}, from {source} at {} ({})",
            row.label,
            row.fetched_local,
            age(now_ms - row.fetched_at_ms)
        );
        for shown in &row.limits {
            let _ = writeln!(
                out,
                "  {:<20}{:>4}%  {}",
                limit_name(&shown.limit),
                percent(shown.limit.percent),
                detail(shown)
            );
        }
    }
    out
}

pub(crate) fn limit_name(limit: &Limit) -> String {
    let scope = limit.scope.as_deref();
    match (limit.kind.as_str(), scope) {
        ("session", None) => "Session".to_string(),
        ("weekly_all", None) => "Weekly, all models".to_string(),
        ("weekly_scoped", Some(scope)) => format!("Weekly, {scope}"),
        ("credits", None) => "Credits".to_string(),
        (kind, Some(scope)) => format!("{kind}, {scope}"),
        (kind, None) => kind.to_string(),
    }
}

pub(crate) fn percent(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn detail(shown: &ShownLimit) -> String {
    let mut parts = Vec::new();
    if let Some(credits) = &shown.limit.credits {
        parts.push(match credits.limit {
            Some(limit) => format!("{:.2} of {limit:.2} {}", credits.used, credits.currency),
            None => format!("{:.2} {} used", credits.used, credits.currency),
        });
    }
    if let Some(resets) = &shown.resets_local {
        parts.push(format!("resets {resets}"));
    }
    if let Some(severity) = shown
        .limit
        .severity
        .as_deref()
        .filter(|severity| *severity != "normal")
    {
        parts.push(severity.to_string());
    }
    parts.join(", ")
}

pub(crate) fn age(elapsed_ms: i64) -> String {
    let minutes = elapsed_ms.max(0) / 60_000;
    let (count, unit) = match minutes {
        0 => return "just now".to_string(),
        1..60 => (minutes, "minute"),
        60..2880 => (minutes / 60, "hour"),
        _ => (minutes / 1440, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}
