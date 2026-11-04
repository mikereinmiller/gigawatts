use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use crate::config::Config;
use crate::contract::{AccountToday, GroupBy, Problem, Range, SCHEMA_VERSION, Snapshot};
use crate::data;
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
            requests: 0,
            cost_usd: 0.0,
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
                    requests: 0,
                    cost_usd: 0.0,
                });
                accounts.last_mut().expect("just pushed")
            }
        };
        account.requests += row.requests;
        account.cost_usd += row.cost_usd.unwrap_or_default();
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
