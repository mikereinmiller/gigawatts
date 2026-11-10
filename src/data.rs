use std::fmt::{self, Write};

use crate::config::Config;
use crate::contract::{
    CostStatus, DataResponse, DataRow, DrillStep, GroupBy, Level, Range, SCHEMA_VERSION,
};
use crate::sources::claude_json::Account;
use crate::store::{self, Filter, Store};

#[derive(Debug)]
pub enum Error {
    Store(store::Error),
    TooDeep {
        group_by: GroupBy,
    },
    NotFound {
        level: Level,
        name: String,
    },
    Ambiguous {
        level: Level,
        name: String,
        keys: Vec<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Store(error) => write!(f, "{error}"),
            Error::TooDeep { group_by } => {
                let levels: Vec<_> = group_by.levels().iter().map(|level| level.name()).collect();
                write!(f, "drilling goes {}, and no further", levels.join(" to "))
            }
            Error::NotFound { level, name } => {
                write!(f, "no {} named {name:?} in this range", level.name())
            }
            Error::Ambiguous { level, name, keys } => write!(
                f,
                "more than one {} is named {name:?}. Drill by one of these instead: {}",
                level.name(),
                keys.join(", ")
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        Error::Store(error)
    }
}

pub fn query(
    store: &Store,
    range: Range,
    group_by: GroupBy,
    drill: &[&str],
    today: &str,
) -> Result<DataResponse, Error> {
    let since = match range {
        Range::Today => Some(today.to_string()),
        Range::Week => Some(store.week_start(today)?),
        Range::Month => Some(store.month_start(today)?),
        Range::All => None,
    };
    let levels = group_by.levels();
    if drill.len() >= levels.len() {
        return Err(Error::TooDeep { group_by });
    }
    let mut steps: Vec<DrillStep> = Vec::new();
    for (&level, name) in levels.iter().zip(drill) {
        let keys = store.keys(level, since.as_deref(), &filters(&steps))?;
        steps.push(find_step(level, name, &keys)?);
    }
    let level = levels[drill.len()];
    let rows = store.totals(level, since.as_deref(), &filters(&steps))?;
    Ok(DataResponse {
        schema_version: SCHEMA_VERSION,
        range,
        group_by,
        drill: steps,
        level,
        today: today.to_string(),
        since,
        rows,
        problems: Vec::new(),
    })
}

fn filters(steps: &[DrillStep]) -> Vec<Filter> {
    steps
        .iter()
        .map(|step| Filter {
            level: step.level,
            key: step.key.clone(),
        })
        .collect()
}

fn find_step(
    level: Level,
    name: &str,
    keys: &[(Option<String>, String)],
) -> Result<DrillStep, Error> {
    let mut matches: Vec<&(Option<String>, String)> = keys
        .iter()
        .filter(|(key, label)| key.as_deref() == Some(name) || label == name)
        .collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    matches.dedup_by(|a, b| a.0 == b.0);
    match matches.as_slice() {
        [] => Err(Error::NotFound {
            level,
            name: name.to_string(),
        }),
        [(key, label)] => Ok(DrillStep {
            level,
            key: key.clone().unwrap_or_default(),
            label: label.clone(),
        }),
        _ => Err(Error::Ambiguous {
            level,
            name: name.to_string(),
            keys: matches.iter().filter_map(|(key, _)| key.clone()).collect(),
        }),
    }
}

pub fn label_accounts(response: &mut DataResponse, config: &Config) {
    for row in &mut response.rows {
        let configured = row
            .account
            .as_deref()
            .and_then(|id| id.parse::<Account>().ok())
            .and_then(|account| config.account(&account).label);
        if let Some(label) = configured {
            row.account_label = label;
        }
    }
}

pub fn render(response: &DataResponse) -> String {
    let mut out = String::new();
    let period = match &response.since {
        Some(since) if since == &response.today => format!("today, {since}"),
        Some(since) => format!("{since} to {}", response.today),
        None => "all history".to_string(),
    };
    let within = if response.drill.is_empty() {
        String::new()
    } else {
        let path: Vec<&str> = response
            .drill
            .iter()
            .map(|step| step.label.as_str())
            .collect();
        format!(" in {}", path.join(" > "))
    };
    let _ = writeln!(out, "Cost by {}{within}, {period}", response.level.name());
    let width = response
        .rows
        .iter()
        .map(|row| row_name(row, response.level).chars().count())
        .max()
        .unwrap_or(0)
        .max(28);
    let sessions = response.level == Level::Session;
    let mut account = None;
    for row in &response.rows {
        if account != Some(&row.account) {
            account = Some(&row.account);
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", row.account_label);
            let _ = write!(
                out,
                "{:<width$} {:>9} {:>11} {:>11} {:>14} {:>12} {:>10}",
                response.level.name(),
                "requests",
                "input",
                "output",
                "cache read",
                "cache write",
                "cost"
            );
            if sessions {
                let _ = write!(out, "  status");
            }
            let _ = writeln!(out);
        }
        let cost = row
            .cost_usd
            .map_or("no price".to_string(), |cost| format!("${cost:.2}"));
        let _ = write!(
            out,
            "{:<width$} {:>9} {:>11} {:>11} {:>14} {:>12} {:>10}",
            row_name(row, response.level),
            row.requests,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_write_tokens,
            cost
        );
        if let Some(status) = row.status {
            let _ = write!(out, "  {}", status_name(status));
        }
        let _ = writeln!(out);
    }
    if response.rows.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "No requests in this range.");
    }
    out.push_str(&crate::problems::footer(&response.problems));
    out
}

fn row_name(row: &DataRow, level: Level) -> String {
    if row.not_in_transcript && matches!(level, Level::Model | Level::Session) {
        format!("{}, unlogged", row.label)
    } else {
        row.label.clone()
    }
}

fn status_name(status: CostStatus) -> &'static str {
    match status {
        CostStatus::Estimated => "estimated",
        CostStatus::Exact => "exact",
        CostStatus::Adjusted => "adjusted",
    }
}
