use std::collections::BTreeMap;

use gigawatts::cost::{Cause, Verdict, reconcile};
use gigawatts::pricing::PriceTable;
use gigawatts::sources::transcript::{CostState, ModelCost, Request, Tokens};

const SONNET: &str = "claude-sonnet-5-5";

fn tokens(input: u64) -> Tokens {
    Tokens {
        input,
        ..Tokens::default()
    }
}

fn request(model: &str, tokens: Tokens) -> Request {
    Request {
        session_id: "s1".to_string(),
        request_id: format!("r{}", tokens.input),
        model: model.to_string(),
        tokens,
        inference_geo: Some("global".to_string()),
        speed: Some("standard".to_string()),
    }
}

fn cost_state(model: &str, tokens: Tokens, cost_usd: f64) -> CostState {
    CostState {
        session_id: "s1".to_string(),
        total_cost_usd: cost_usd,
        models: BTreeMap::from([(
            model.to_string(),
            ModelCost {
                tokens,
                has_cache_write_split: true,
                cost_usd,
            },
        )]),
    }
}

#[test]
fn a_session_priced_to_its_recorded_cost_is_exact() {
    let requests = [
        request(SONNET, tokens(1_000_000)),
        request(SONNET, tokens(500_000)),
    ];
    let state = cost_state(SONNET, tokens(1_500_000), 3.0);

    let checks = reconcile(
        &state,
        &requests.iter().collect::<Vec<_>>(),
        &PriceTable::built_in(),
    );

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].verdict, Verdict::Exact);
    assert_eq!(checks[0].cause, None);
    assert_eq!(checks[0].priced, Some(3.0));
}

#[test]
fn a_gap_under_a_tenth_of_a_cent_is_exact_on_a_small_session() {
    let requests = [request(SONNET, tokens(50_000))];
    let state = cost_state(SONNET, tokens(50_000), 0.1009);

    let checks = reconcile(
        &state,
        &requests.iter().collect::<Vec<_>>(),
        &PriceTable::built_in(),
    );

    assert_eq!(checks[0].verdict, Verdict::Exact);
}

#[test]
fn a_gap_over_a_tenth_of_a_percent_is_a_mismatch_on_a_large_session() {
    let requests = [request(SONNET, tokens(5_000_000))];
    let state = cost_state(SONNET, tokens(5_000_000), 10.02);

    let checks = reconcile(
        &state,
        &requests.iter().collect::<Vec<_>>(),
        &PriceTable::built_in(),
    );

    assert_eq!(checks[0].verdict, Verdict::Mismatch);
    assert_eq!(checks[0].cause, Some(Cause::PriceOrRule));
    assert!((checks[0].gap() - (-0.02)).abs() < 1e-9);
}

#[test]
fn recorded_tokens_beyond_the_transcript_mean_requests_are_missing_from_it() {
    let requests = [request(SONNET, tokens(1_000_000))];
    let state = cost_state(SONNET, tokens(1_500_000), 3.0);

    let checks = reconcile(
        &state,
        &requests.iter().collect::<Vec<_>>(),
        &PriceTable::built_in(),
    );

    assert_eq!(checks[0].cause, Some(Cause::MissingFromTranscript));
}

#[test]
fn transcript_tokens_beyond_the_cost_state_mean_requests_came_after_it() {
    let requests = [
        request(SONNET, tokens(1_000_000)),
        request("claude-haiku-5-5", tokens(1_000)),
    ];
    let state = cost_state(SONNET, tokens(500_000), 1.0);

    let checks = reconcile(
        &state,
        &requests.iter().collect::<Vec<_>>(),
        &PriceTable::built_in(),
    );

    let causes: Vec<_> = checks.iter().map(|c| (c.model.as_str(), c.cause)).collect();
    assert_eq!(
        causes,
        [
            ("claude-haiku-5-5", None),
            (SONNET, Some(Cause::AfterCostState))
        ]
    );
}

#[test]
fn a_model_without_a_price_is_named_as_the_cause() {
    let requests = [request("claude-opus-9", tokens(1_000))];
    let state = cost_state("claude-opus-9", tokens(1_000), 1.0);

    let checks = reconcile(
        &state,
        &requests.iter().collect::<Vec<_>>(),
        &PriceTable::built_in(),
    );

    assert_eq!(checks[0].priced, None);
    assert_eq!(checks[0].verdict, Verdict::Mismatch);
    assert_eq!(checks[0].cause, Some(Cause::NoPrice));
}

#[test]
fn a_billed_model_with_no_transcript_requests_is_missing_from_the_transcript() {
    let state = cost_state("claude-haiku-5-5", tokens(100_000), 0.01);

    let checks = reconcile(&state, &[], &PriceTable::built_in());

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].priced, Some(0.0));
    assert_eq!(checks[0].cause, Some(Cause::MissingFromTranscript));
}
