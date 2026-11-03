use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension, Transaction, params};

use super::Result;
use crate::contract::CostStatus;
use crate::cost::{self, Cause};
use crate::pricing::PriceTable;
use crate::sources::transcript::{CostState, ModelCost, Request, Tokens};

struct StoredRequest {
    row_id: i64,
    day: Option<String>,
    cost_usd: Option<f64>,
    request: Request,
}

pub(super) fn session(
    transaction: &Transaction,
    session_id: i64,
    table: &PriceTable,
) -> Result<BTreeSet<String>> {
    let requests = requests(transaction, session_id)?;
    let Some(state) = cost_state(transaction, session_id)? else {
        return reset_to_table_prices(transaction, &requests, table);
    };
    let request_refs: Vec<&Request> = requests.iter().map(|stored| &stored.request).collect();
    let checks = cost::reconcile(&state, &request_refs, table);

    transaction.execute(
        "DELETE FROM not_in_transcript WHERE session_id = ?1",
        [session_id],
    )?;
    let mut changed_days = BTreeSet::new();
    for check in &checks {
        let scale = match (check.cause, check.priced) {
            (Some(Cause::PriceOrRule), Some(priced))
                if priced > 0.0 && !recorded_no_cost(check) =>
            {
                check.recorded / priced
            }
            _ => 1.0,
        };
        for stored in requests
            .iter()
            .filter(|stored| stored.request.model == check.model)
        {
            let cost = table.price(&stored.request).map(|cost| cost * scale);
            if cost != stored.cost_usd {
                transaction.execute(
                    "UPDATE requests SET cost_usd = ?2 WHERE id = ?1",
                    params![stored.row_id, cost],
                )?;
                changed_days.extend(stored.day.clone());
            }
        }
        let price_gap = (check.cause == Some(Cause::PriceOrRule) || priced_above_shortfall(check))
            .then(|| check.gap());
        transaction.execute(
            "UPDATE session_models SET price_gap_usd = ?3
             WHERE session_id = ?1
               AND model_label = (SELECT id FROM labels WHERE text = ?2)",
            params![session_id, check.model, price_gap],
        )?;
        let recorded = state.models.get(&check.model);
        let unpriced_but_recorded = check.cause == Some(Cause::NoPrice) && recorded.is_some();
        if check.cause == Some(Cause::MissingFromTranscript) || unpriced_but_recorded {
            let has_cache_write_split =
                recorded.is_some_and(|recorded| recorded.has_cache_write_split);
            save_shortfall(transaction, session_id, check, has_cache_write_split)?;
        }
    }
    transaction.execute(
        "UPDATE sessions SET cost_status = ?2 WHERE id = ?1",
        params![session_id, status_name(status(&checks))],
    )?;
    Ok(changed_days)
}

fn reset_to_table_prices(
    transaction: &Transaction,
    requests: &[StoredRequest],
    table: &PriceTable,
) -> Result<BTreeSet<String>> {
    let mut changed_days = BTreeSet::new();
    for stored in requests {
        let cost = table.price(&stored.request);
        if cost != stored.cost_usd {
            transaction.execute(
                "UPDATE requests SET cost_usd = ?2 WHERE id = ?1",
                params![stored.row_id, cost],
            )?;
            changed_days.extend(stored.day.clone());
        }
    }
    Ok(changed_days)
}

pub(super) fn status_from_name(name: Option<&str>) -> CostStatus {
    match name {
        Some("exact") => CostStatus::Exact,
        Some("adjusted") => CostStatus::Adjusted,
        _ => CostStatus::Estimated,
    }
}

fn status_name(status: CostStatus) -> &'static str {
    match status {
        CostStatus::Estimated => "estimated",
        CostStatus::Exact => "exact",
        CostStatus::Adjusted => "adjusted",
    }
}

fn priced_above_shortfall(check: &cost::PairCheck) -> bool {
    check.cause == Some(Cause::MissingFromTranscript) && check.gap() > 0.0
}

fn recorded_no_cost(check: &cost::PairCheck) -> bool {
    check.cause == Some(Cause::PriceOrRule) && check.recorded == 0.0
}

fn status(checks: &[cost::PairCheck]) -> CostStatus {
    let causes: Vec<Cause> = checks.iter().filter_map(|check| check.cause).collect();
    let estimated = checks
        .iter()
        .any(|check| priced_above_shortfall(check) || recorded_no_cost(check))
        || causes.iter().any(|cause| {
            matches!(
                cause,
                Cause::NoPrice | Cause::AfterCostState | Cause::TokenMismatch
            )
        });
    if estimated {
        CostStatus::Estimated
    } else if causes.contains(&Cause::PriceOrRule) {
        CostStatus::Adjusted
    } else {
        CostStatus::Exact
    }
}

fn save_shortfall(
    transaction: &Transaction,
    session_id: i64,
    check: &cost::PairCheck,
    has_cache_write_split: bool,
) -> Result<()> {
    let recorded = &check.recorded_tokens;
    let transcript = &check.transcript_tokens;
    let missing = |recorded: u64, transcript: u64| recorded.saturating_sub(transcript) as i64;
    let (cache_write_5m, cache_write_1h) = if has_cache_write_split {
        (
            missing(recorded.cache_write_5m, transcript.cache_write_5m),
            missing(recorded.cache_write_1h, transcript.cache_write_1h),
        )
    } else {
        (missing(recorded.cache_write(), transcript.cache_write()), 0)
    };
    transaction.execute(
        "INSERT INTO not_in_transcript (
            session_id, model_label, day, input_tokens, output_tokens, cache_read_tokens,
            cache_write_5m_tokens, cache_write_1h_tokens, web_searches, cost_usd
         )
         SELECT ?1, labels.id,
                coalesce(
                    (SELECT max(day) FROM requests WHERE session_id = ?1),
                    (SELECT date(start_time_ms / 1000, 'unixepoch', 'localtime')
                     FROM sessions WHERE id = ?1)
                ),
                ?3, ?4, ?5, ?6, ?7, ?8, ?9
         FROM labels WHERE text = ?2",
        params![
            session_id,
            check.model,
            missing(recorded.input, transcript.input),
            missing(recorded.output, transcript.output),
            missing(recorded.cache_read, transcript.cache_read),
            cache_write_5m,
            cache_write_1h,
            missing(recorded.web_searches, transcript.web_searches),
            (-check.gap()).max(0.0),
        ],
    )?;
    Ok(())
}

fn cost_state(transaction: &Transaction, session_id: i64) -> Result<Option<CostState>> {
    let total: Option<f64> = transaction
        .query_row(
            "SELECT total_cost_usd FROM sessions WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(total_cost_usd) = total else {
        return Ok(None);
    };
    let mut statement = transaction.prepare_cached(
        "SELECT model.text, input_tokens, output_tokens, cache_read_tokens,
                cache_write_5m_tokens, cache_write_1h_tokens, web_searches, cost_usd,
                has_cache_write_split
         FROM session_models
         JOIN labels AS model ON model.id = session_models.model_label
         WHERE session_id = ?1",
    )?;
    let models = statement
        .query_map([session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ModelCost {
                    tokens: Tokens {
                        input: row.get::<_, i64>(1)? as u64,
                        output: row.get::<_, i64>(2)? as u64,
                        cache_read: row.get::<_, i64>(3)? as u64,
                        cache_write_5m: row.get::<_, i64>(4)? as u64,
                        cache_write_1h: row.get::<_, i64>(5)? as u64,
                        web_searches: row.get::<_, i64>(6)? as u64,
                    },
                    has_cache_write_split: row.get(8)?,
                    cost_usd: row.get(7)?,
                    thinking_tokens: 0,
                    web_fetches: 0,
                },
            ))
        })?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    Ok(Some(CostState {
        session_id: session_id.to_string(),
        total_cost_usd,
        models,
        totals: Default::default(),
    }))
}

fn requests(transaction: &Transaction, session_id: i64) -> Result<Vec<StoredRequest>> {
    let mut statement = transaction.prepare_cached(
        "SELECT requests.id, request_id, model.text, input_tokens, output_tokens,
                cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens, web_searches,
                geo.text, speed.text, cost_usd, day
         FROM requests
         JOIN labels AS model ON model.id = requests.model_label
         LEFT JOIN labels AS geo ON geo.id = requests.inference_geo_label
         LEFT JOIN labels AS speed ON speed.id = requests.speed_label
         WHERE session_id = ?1",
    )?;
    let requests = statement
        .query_map([session_id], |row| {
            Ok(StoredRequest {
                row_id: row.get(0)?,
                cost_usd: row.get(11)?,
                day: row.get(12)?,
                request: Request {
                    request_id: row.get(1)?,
                    model: row.get(2)?,
                    tokens: Tokens {
                        input: row.get::<_, i64>(3)? as u64,
                        output: row.get::<_, i64>(4)? as u64,
                        cache_read: row.get::<_, i64>(5)? as u64,
                        cache_write_5m: row.get::<_, i64>(6)? as u64,
                        cache_write_1h: row.get::<_, i64>(7)? as u64,
                        web_searches: row.get::<_, i64>(8)? as u64,
                    },
                    inference_geo: row.get(9)?,
                    speed: row.get(10)?,
                    ..Request::default()
                },
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(requests)
}
