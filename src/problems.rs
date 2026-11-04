use crate::contract::{Problem, ProblemKind};
use crate::pricing::{PriceTable, TABLE_DATE, TABLE_SOURCE};
use crate::store::{self, ModelSighting, PriceGap, Store, UnrecognizedKind};

const CONFIG_PATH: &str = "~/.config/gigawatts/config.toml";

pub fn collect(store: &Store, table: &PriceTable) -> store::Result<Vec<Problem>> {
    let mut problems: Vec<Problem> = store
        .unrecognized_kinds()?
        .iter()
        .map(unrecognized_lines)
        .collect();
    problems.extend(store.models_without_price(table)?.iter().map(no_price));
    problems.extend(store.price_gaps()?.iter().map(price_gap));
    Ok(problems)
}

fn unrecognized_lines(unrecognized: &UnrecognizedKind) -> Problem {
    let missing_requests = "history: any requests on those lines are missing from every total";
    let (subject, affects) = match unrecognized.kind.as_str() {
        "invalid JSON" => (
            "Transcript lines mentioning usage that don't parse".to_string(),
            missing_requests,
        ),
        "assistant without a timestamp" => (
            "Assistant lines with usage but no timestamp".to_string(),
            "history: those requests are missing from every total, because they can't be placed on a day",
        ),
        "assistant" => (
            "Assistant lines that don't parse".to_string(),
            missing_requests,
        ),
        "cost-state" => (
            "Cost-state lines that don't parse".to_string(),
            "cost: those sessions stay estimated instead of matching Claude Code's record",
        ),
        "ai-title" => (
            "Title lines that don't parse".to_string(),
            "session titles: those sessions show without one",
        ),
        "untyped" => (
            "Lines with no type that carry usage".to_string(),
            missing_requests,
        ),
        kind => (
            format!("Lines of type {kind:?} that carry usage"),
            missing_requests,
        ),
    };
    Problem {
        kind: ProblemKind::UnrecognizedLines,
        affects: affects.to_string(),
        since: unrecognized.first_seen.clone(),
        cause: format!(
            "{subject} ({} in {})",
            unrecognized.lines,
            count(unrecognized.files, "file")
        ),
        fix: "A Claude Code update probably changed the transcript format. Update Gigawatts to a version that reads it.".to_string(),
    }
}

fn no_price(sighting: &ModelSighting) -> Problem {
    let affects = if sighting.requests > 0 {
        format!(
            "cost: {} in {}",
            sighting.model,
            count(sighting.requests, "request")
        )
    } else {
        format!(
            "cost: what Claude Code recorded for {} in {}",
            sighting.model,
            count(sighting.sessions, "session")
        )
    };
    Problem {
        kind: ProblemKind::NoPrice,
        affects,
        since: sighting.first_seen.clone(),
        cause: format!("no price for {}", sighting.model),
        fix: format!(
            "Add [prices.{}] to {CONFIG_PATH} with input, cache_write_5m, cache_write_1h, cache_read, and output in dollars per million tokens, from {TABLE_SOURCE}, then run `gigawatts ingest`. Until then, the cost Claude Code recorded for it counts as not in transcript.",
            toml_key(&sighting.model)
        ),
    }
}

fn price_gap(gap: &PriceGap) -> Problem {
    Problem {
        kind: ProblemKind::PriceGap,
        affects: format!("cost: {} in {}", gap.model, count(gap.sessions, "session")),
        since: gap.first_seen.clone(),
        cause: format!(
            "{}'s prices differ from Claude Code's record by at least {}",
            gap.model,
            usd(gap.gap_usd)
        ),
        fix: format!(
            "Compare its price in the built-in table (copied {TABLE_DATE}) and {CONFIG_PATH} with {TABLE_SOURCE}, then add or correct [prices.{}] and run `gigawatts ingest`. Until then, sessions whose tokens match Claude Code's record are scaled to its totals and marked adjusted, and the rest stay estimated.",
            toml_key(&gap.model)
        ),
    }
}

fn count(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn usd(amount: f64) -> String {
    if amount >= 0.01 {
        format!("${amount:.2}")
    } else {
        format!("${amount:.4}")
    }
}

fn toml_key(model: &str) -> String {
    let bare = model
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if bare {
        model.to_string()
    } else {
        format!("{model:?}")
    }
}

pub fn render(problems: &[Problem]) -> String {
    if problems.is_empty() {
        return "No problems.\n".to_string();
    }
    let mut out = String::new();
    for (index, problem) in problems.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{}\n", problem.cause));
        out.push_str(&format!("  {}\n", problem.affects));
        if let Some(since) = &problem.since {
            out.push_str(&format!("  Since: {since}\n"));
        }
        out.push_str(&format!("  Fix: {}\n", problem.fix));
    }
    out
}

pub fn footer(problems: &[Problem]) -> String {
    match problems.len() {
        0 => String::new(),
        1 => "\n1 problem. `gigawatts doctor` lists it.\n".to_string(),
        n => format!("\n{n} problems. `gigawatts doctor` lists them.\n"),
    }
}
