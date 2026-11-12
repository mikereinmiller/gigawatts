use std::fmt::{self, Write};

use crate::config::Config;
use crate::contract::{
    CostStatus, DataResponse, DataRow, DrillStep, GroupBy, Level, Problem, Range, SCHEMA_VERSION,
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
    let since = range_start(store, range, today)?;
    let mut response = query_between(store, since.as_deref(), None, group_by, drill, today)?;
    response.range = range;
    Ok(response)
}

pub fn query_between(
    store: &Store,
    since: Option<&str>,
    until: Option<&str>,
    group_by: GroupBy,
    drill: &[&str],
    today: &str,
) -> Result<DataResponse, Error> {
    query_scoped(store, since, until, &[], group_by, drill, today)
}

fn query_scoped(
    store: &Store,
    since: Option<&str>,
    until: Option<&str>,
    scope: &[Filter],
    group_by: GroupBy,
    drill: &[&str],
    today: &str,
) -> Result<DataResponse, Error> {
    let (steps, level) = resolve_drill(store, group_by, drill, since, until, scope)?;
    let rows = store.totals(level, since, until, &scoped(scope, &steps))?;
    Ok(DataResponse {
        schema_version: SCHEMA_VERSION,
        range: Range::Custom,
        group_by,
        drill: steps,
        level,
        today: today.to_string(),
        since: since.map(str::to_string),
        until: until.map(str::to_string),
        rows,
        problems: Vec::new(),
    })
}

pub fn local_date(text: &str) -> Result<String, String> {
    let not_a_date = || format!("{text} isn't a date. Use YYYY-MM-DD, like 2026-10-01");
    let parsed: toml::value::Datetime = text.parse().map_err(|_| not_a_date())?;
    match parsed {
        toml::value::Datetime {
            date: Some(_),
            time: None,
            offset: None,
        } if parsed.to_string() == text => Ok(text.to_string()),
        _ => Err(not_a_date()),
    }
}

pub enum Days {
    Range(Range),
    Between(Option<String>, Option<String>),
}

pub fn answer(
    store: &Store,
    config: &Config,
    days: &Days,
    scope: &[Filter],
    group_by: GroupBy,
    drill: &[&str],
    engine_problems: Vec<Problem>,
) -> Result<DataResponse, Error> {
    let today = store.local_today()?;
    let scope = &resolve_scope(store, scope)?;
    let (range, since, until) = bounds(store, days, &today)?;
    let mut response = query_scoped(
        store,
        since.as_deref(),
        until.as_deref(),
        scope,
        group_by,
        drill,
        &today,
    )?;
    response.range = range;
    response.problems = crate::problems::collect(store, &config.price_table())?;
    response.problems.extend(engine_problems);
    label_accounts(&mut response, config);
    Ok(response)
}

fn resolve_drill(
    store: &Store,
    group_by: GroupBy,
    drill: &[&str],
    since: Option<&str>,
    until: Option<&str>,
    scope: &[Filter],
) -> Result<(Vec<DrillStep>, Level), Error> {
    let levels = group_by.levels();
    if drill.len() >= levels.len() {
        return Err(Error::TooDeep { group_by });
    }
    let mut steps: Vec<DrillStep> = Vec::new();
    for (&level, name) in levels.iter().zip(drill) {
        let keys = store.keys(level, since, until, &scoped(scope, &steps))?;
        steps.push(find_step(level, name, &keys)?);
    }
    Ok((steps, levels[drill.len()]))
}

type Bounds = (Range, Option<String>, Option<String>);

fn bounds(store: &Store, days: &Days, today: &str) -> Result<Bounds, Error> {
    Ok(match days {
        Days::Range(range) => (*range, range_start(store, *range, today)?, None),
        Days::Between(since, until) => (Range::Custom, since.clone(), until.clone()),
    })
}

fn range_start(store: &Store, range: Range, today: &str) -> Result<Option<String>, Error> {
    Ok(match range {
        Range::Today => Some(today.to_string()),
        Range::Week => Some(store.week_start(today)?),
        Range::Month => Some(store.month_start(today)?),
        Range::All | Range::Custom => None,
    })
}

fn resolve_scope(store: &Store, scope: &[Filter]) -> Result<Vec<Filter>, Error> {
    let mut resolved: Vec<Filter> = Vec::new();
    for filter in scope {
        let keys = store.keys(filter.level, None, None, &resolved)?;
        let step = find_step(filter.level, &filter.key, &keys)?;
        resolved.push(Filter {
            level: filter.level,
            key: step.key,
        });
    }
    Ok(resolved)
}

fn scoped(scope: &[Filter], steps: &[DrillStep]) -> Vec<Filter> {
    let drilled = steps.iter().map(|step| Filter {
        level: step.level,
        key: step.key.clone(),
    });
    scope.iter().cloned().chain(drilled).collect()
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
    let period = match (&response.since, &response.until) {
        (Some(since), None) if since == &response.today => format!("today, {since}"),
        (Some(since), None) => format!("{since} to {}", response.today),
        (Some(since), Some(until)) => format!("{since} to {until}"),
        (None, Some(until)) => format!("all history through {until}"),
        (None, None) => "all history".to_string(),
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

pub fn row_name(row: &DataRow, level: Level) -> String {
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
