use std::fmt::Write;

use crate::config::Config;
use crate::contract::{DataResponse, GroupBy, Range, SCHEMA_VERSION};
use crate::sources::claude_json::Account;
use crate::store::{self, Store};

pub fn query(
    store: &Store,
    range: Range,
    group_by: GroupBy,
    today: &str,
) -> store::Result<DataResponse> {
    let since = match range {
        Range::Today => Some(today.to_string()),
        Range::Week => Some(store.week_start(today)?),
        Range::Month => Some(store.month_start(today)?),
        Range::All => None,
    };
    let rows = match group_by {
        GroupBy::Model => store.totals_by_model(since.as_deref())?,
    };
    Ok(DataResponse {
        schema_version: SCHEMA_VERSION,
        range,
        group_by,
        today: today.to_string(),
        since,
        rows,
    })
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
    let _ = writeln!(
        out,
        "Cost by model, {period}, estimated from transcript prices"
    );
    let mut account = None;
    for row in &response.rows {
        if account != Some(&row.account) {
            account = Some(&row.account);
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", row.account_label);
            let _ = writeln!(
                out,
                "{:<28} {:>9} {:>11} {:>11} {:>14} {:>12} {:>10}",
                "model", "requests", "input", "output", "cache read", "cache write", "cost"
            );
        }
        let cost = row
            .cost_usd
            .map_or("no price".to_string(), |cost| format!("${cost:.2}"));
        let _ = writeln!(
            out,
            "{:<28} {:>9} {:>11} {:>11} {:>14} {:>12} {:>10}",
            row.key,
            row.requests,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_write_tokens,
            cost
        );
    }
    if response.rows.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "No requests in this range.");
    }
    out
}
