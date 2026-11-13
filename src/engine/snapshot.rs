use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use crate::config::Config;
use crate::contract::{
    AccountLimits, AccountToday, DollarPace, GroupBy, PacedLimit, Problem, Range, SCHEMA_VERSION,
    Snapshot,
};
use crate::data;
use crate::pace;
use crate::pricing::without_date_suffix;
use crate::problems;
use crate::sources::claude_json::Account;
use crate::store::Store;

pub fn build(
    store: &Store,
    config: &Config,
    now_ms: i64,
    engine_problems: &[Problem],
) -> Result<Snapshot, data::Error> {
    let today = store.local_today()?;
    let mut response = data::query(store, Range::Today, GroupBy::Model, &[], &today)?;
    data::label_accounts(&mut response, config);
    let mut accounts: Vec<AccountToday> = store
        .accounts()?
        .into_iter()
        .filter(|stored| config.account(&stored.account).tracking)
        .map(|stored| AccountToday {
            account: Some(stored.account.to_string()),
            account_label: config
                .account(&stored.account)
                .label
                .or(stored.email)
                .unwrap_or_else(|| stored.account.to_string()),
            configured_label: config.account(&stored.account).label,
            requests: 0,
            cost_usd: 0.0,
            pace: no_pace(),
            fastest_session: None,
            active_sessions: Vec::new(),
            unpriced_models: Vec::new(),
            adjusted_models: Vec::new(),
            cost_estimated: false,
            limits: None,
        })
        .collect();
    for row in response
        .rows
        .iter()
        .filter(|row| tracked(config, row.account.as_deref()))
    {
        let position = accounts
            .iter()
            .position(|account| account.account == row.account);
        let account = match position {
            Some(position) => &mut accounts[position],
            None => {
                accounts.push(AccountToday {
                    account: row.account.clone(),
                    account_label: row.account_label.clone(),
                    configured_label: row
                        .account
                        .as_deref()
                        .and_then(|id| id.parse().ok())
                        .and_then(|account| config.account(&account).label),
                    requests: 0,
                    cost_usd: 0.0,
                    pace: no_pace(),
                    fastest_session: None,
                    active_sessions: Vec::new(),
                    unpriced_models: Vec::new(),
                    adjusted_models: Vec::new(),
                    cost_estimated: false,
                    limits: None,
                });
                accounts.last_mut().expect("just pushed")
            }
        };
        account.requests += row.requests;
        account.cost_usd += row.cost_usd.unwrap_or_default();
    }
    let table = config.price_table();
    for account in &mut accounts {
        let parsed = account
            .account
            .as_deref()
            .and_then(|account| account.parse::<Account>().ok());
        account.pace = pace::dollar_pace(store, parsed.as_ref(), now_ms)?;
        account.active_sessions = pace::active_sessions(store, parsed.as_ref(), now_ms)?;
        account.fastest_session = account.active_sessions.first().cloned();
        let flags = store.cost_flags_on(parsed.as_ref(), &today)?;
        account.unpriced_models = flags
            .unpriced_models
            .iter()
            .filter(|model| table.model(model).is_none())
            .map(|model| without_date_suffix(model).to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        account.cost_estimated = flags.estimated;
        account.adjusted_models = flags
            .adjusted_models
            .iter()
            .map(|model| without_date_suffix(model).to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    for (stored, reading) in store.latest_limits()? {
        let id = Some(stored.to_string());
        if let Some(account) = accounts.iter_mut().find(|account| account.account == id) {
            let credit_reset = config.account(&stored).credit_reset;
            let mut limits = Vec::new();
            for limit in reading.limits {
                let history = match limit.kind.as_str() {
                    "weekly_all" | "weekly_scoped" => store.limit_series(
                        &stored,
                        &limit.kind,
                        limit.scope.as_deref(),
                        now_ms - pace::USUAL_HISTORY_MS,
                    )?,
                    _ => Vec::new(),
                };
                let limit_pace = pace::limit_pace(
                    &limit,
                    reading.fetched_at_ms,
                    now_ms,
                    &history,
                    credit_reset.as_ref(),
                );
                let forecast = limit_pace.as_ref().and_then(|limit_pace| {
                    pace::limit_forecast(&limit, limit_pace, reading.fetched_at_ms)
                });
                limits.push(PacedLimit {
                    pace: limit_pace,
                    forecast,
                    limit,
                });
            }
            account.limits = Some(AccountLimits {
                source: reading.source,
                fetched_at_ms: reading.fetched_at_ms,
                limits,
            });
        }
    }
    Ok(Snapshot {
        schema_version: SCHEMA_VERSION,
        engine_version: crate::engine::version(),
        written_at_ms: now_ms,
        today,
        accounts,
        problems: [
            problems::collect(store, &config.price_table())?,
            engine_problems.to_vec(),
        ]
        .concat(),
    })
}

fn no_pace() -> DollarPace {
    DollarPace {
        today_usd: 0.0,
        usual_usd: None,
        usual_days: 0,
        clock: String::new(),
        weekend: false,
        earlier_days_usd: Vec::new(),
        earlier_day_totals_usd: Vec::new(),
        earlier_days: Vec::new(),
    }
}

fn tracked(config: &Config, account: Option<&str>) -> bool {
    account
        .and_then(|account| account.parse::<Account>().ok())
        .is_none_or(|account| config.account(&account).tracking)
}

pub fn write(path: &Path, snapshot: &Snapshot) -> io::Result<()> {
    let staged = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)?;
    serde_json::to_writer_pretty(&mut file, snapshot)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(&staged, path)
}
