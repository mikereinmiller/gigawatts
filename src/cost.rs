use std::collections::BTreeMap;

use crate::pricing::PriceTable;
use crate::sources::transcript::{CostState, Request, Tokens};

const RELATIVE_TOLERANCE: f64 = 0.001;
const ABSOLUTE_TOLERANCE_USD: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Exact,
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cause {
    NoPrice,
    MissingFromTranscript,
    AfterCostState,
    TokenMismatch,
    PriceOrRule,
}

impl Cause {
    pub fn describe(&self) -> &'static str {
        match self {
            Cause::NoPrice => "no price for this model",
            Cause::MissingFromTranscript => "transcript has fewer tokens than the cost-state",
            Cause::AfterCostState => "transcript has requests after the last cost-state",
            Cause::TokenMismatch => "token counts disagree in both directions",
            Cause::PriceOrRule => "same tokens, different cost: a price or pricing rule",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairCheck {
    pub session_id: String,
    pub model: String,
    pub priced: Option<f64>,
    pub recorded: f64,
    pub transcript_tokens: Tokens,
    pub recorded_tokens: Tokens,
    pub verdict: Verdict,
    pub cause: Option<Cause>,
}

impl PairCheck {
    pub fn gap(&self) -> f64 {
        self.priced.unwrap_or(0.0) - self.recorded
    }

    pub fn tokens_agree(&self) -> bool {
        token_pairs(&self.transcript_tokens, &self.recorded_tokens)
            .iter()
            .all(|(t, r)| t == r)
    }
}

pub fn matches(priced: f64, recorded: f64) -> bool {
    let tolerance = (recorded.abs() * RELATIVE_TOLERANCE).max(ABSOLUTE_TOLERANCE_USD);
    (priced - recorded).abs() <= tolerance
}

pub fn reconcile(state: &CostState, requests: &[&Request], table: &PriceTable) -> Vec<PairCheck> {
    let mut by_model: BTreeMap<&str, Vec<&Request>> = BTreeMap::new();
    for request in requests {
        by_model
            .entry(request.model.as_str())
            .or_default()
            .push(request);
    }
    for model in state.models.keys() {
        by_model.entry(model.as_str()).or_default();
    }

    by_model
        .into_iter()
        .map(|(model, model_requests)| check_pair(state, model, &model_requests, table))
        .collect()
}

fn check_pair(
    state: &CostState,
    model: &str,
    requests: &[&Request],
    table: &PriceTable,
) -> PairCheck {
    let mut transcript_tokens = Tokens::default();
    for request in requests {
        transcript_tokens.add(&request.tokens);
    }
    let priced = table
        .model(model)
        .and_then(|_| requests.iter().map(|r| table.price(r)).sum());
    let recorded = state.models.get(model);
    let recorded_cost = recorded.map_or(0.0, |m| m.cost_usd);
    let recorded_tokens = recorded.map(|m| m.tokens.clone()).unwrap_or_default();

    let cause = match priced {
        None => Some(Cause::NoPrice),
        Some(cost) if matches(cost, recorded_cost) => None,
        Some(_) => Some(token_cause(&transcript_tokens, &recorded_tokens)),
    };
    PairCheck {
        session_id: state.session_id.clone(),
        model: model.to_string(),
        priced,
        recorded: recorded_cost,
        transcript_tokens,
        recorded_tokens,
        verdict: if cause.is_none() {
            Verdict::Exact
        } else {
            Verdict::Mismatch
        },
        cause,
    }
}

fn token_pairs(transcript: &Tokens, recorded: &Tokens) -> [(u64, u64); 5] {
    [
        (transcript.input, recorded.input),
        (transcript.output, recorded.output),
        (transcript.cache_read, recorded.cache_read),
        (transcript.cache_write(), recorded.cache_write()),
        (transcript.web_searches, recorded.web_searches),
    ]
}

fn token_cause(transcript: &Tokens, recorded: &Tokens) -> Cause {
    let pairs = token_pairs(transcript, recorded);
    if pairs.iter().all(|(t, r)| t == r) {
        Cause::PriceOrRule
    } else if pairs.iter().all(|(t, r)| t <= r) {
        Cause::MissingFromTranscript
    } else if pairs.iter().all(|(t, r)| t >= r) {
        Cause::AfterCostState
    } else {
        Cause::TokenMismatch
    }
}
