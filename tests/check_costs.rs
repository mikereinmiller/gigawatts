use gigawatts::check_costs::CheckReport;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::transcript::{Corpus, parse_line};

fn request_line(session: &str, request: &str, input_tokens: u64) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"{session}","requestId":"{request}","message":{{"model":"claude-sonnet-5-5","usage":{{"input_tokens":{input_tokens},"output_tokens":0}}}}}}"#
    )
}

fn cost_state_line(session: &str, input_tokens: u64, cost_usd: f64) -> String {
    format!(
        r#"{{"type":"cost-state","sessionId":"{session}","totalCostUSD":{cost_usd},"modelUsage":{{"claude-sonnet-5-5":{{"inputTokens":{input_tokens},"outputTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"cacheCreationEphemeral5mInputTokens":0,"cacheCreationEphemeral1hInputTokens":0,"webSearchRequests":0,"costUSD":{cost_usd}}}}}}}"#
    )
}

fn corpus(exact_sessions: usize, price_gap_sessions: usize, missing_sessions: usize) -> Corpus {
    let mut corpus = Corpus::default();
    let mut lines = Vec::new();
    for i in 0..exact_sessions {
        let session = format!("exact{i}");
        lines.push(request_line(&session, "r1", 1_000_000));
        lines.push(cost_state_line(&session, 1_000_000, 2.0));
    }
    for i in 0..price_gap_sessions {
        let session = format!("price-gap{i}");
        lines.push(request_line(&session, "r1", 1_000_000));
        lines.push(cost_state_line(&session, 1_000_000, 2.5));
    }
    for i in 0..missing_sessions {
        let session = format!("missing{i}");
        lines.push(request_line(&session, "r1", 1_000_000));
        lines.push(cost_state_line(&session, 2_000_000, 4.0));
    }
    lines.push(request_line("no-cost-state", "r1", 1_000));
    for line in &lines {
        corpus.add(parse_line(line).expect("candidate line"));
    }
    corpus
}

#[test]
fn passes_the_gate_at_95_percent_of_pairs_matching() {
    let report = CheckReport::build(&corpus(19, 1, 0), &PriceTable::built_in());

    assert_eq!(report.exact_pairs(), 19);
    assert_eq!(report.checks.len(), 20);
    assert!(report.passes_gate());
}

#[test]
fn fails_the_gate_below_95_percent() {
    let report = CheckReport::build(&corpus(18, 2, 0), &PriceTable::built_in());

    assert!(!report.passes_gate());
}

#[test]
fn counts_sessions_without_a_cost_state_instead_of_checking_them() {
    let report = CheckReport::build(&corpus(1, 0, 0), &PriceTable::built_in());

    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.sessions_without_cost_state, 1);
}

#[test]
fn spend_missing_from_the_transcript_does_not_fail_the_gate() {
    let report = CheckReport::build(&corpus(3, 0, 5), &PriceTable::built_in());

    assert!(report.passes_gate());
}

#[test]
fn summarizes_each_model_with_its_mismatch_causes() {
    let report = CheckReport::build(&corpus(3, 1, 1), &PriceTable::built_in());

    let rendered = report.render();

    assert!(rendered.contains("claude-sonnet-5-5"), "{rendered}");
    assert!(
        rendered.contains("transcript has fewer tokens than the cost-state"),
        "{rendered}"
    );
    assert!(rendered.contains("Gate: FAIL"), "{rendered}");
}

#[test]
fn reports_priced_and_recorded_dollars_per_model() {
    let report = CheckReport::build(&corpus(3, 0, 1), &PriceTable::built_in());

    let rendered = report.render();

    let sonnet_row = rendered
        .lines()
        .find(|l| l.starts_with("claude-sonnet-5-5"))
        .expect("sonnet row");
    assert!(sonnet_row.contains("$8.00"), "{sonnet_row}");
    assert!(sonnet_row.contains("$10.00"), "{sonnet_row}");
    assert!(sonnet_row.contains("80.0%"), "{sonnet_row}");
}

#[test]
fn largest_gaps_leave_out_spend_missing_from_the_transcript() {
    let report = CheckReport::build(&corpus(3, 1, 5), &PriceTable::built_in());

    let rendered = report.render();

    let gaps = rendered
        .split("Largest gaps")
        .nth(1)
        .expect("largest gaps section");
    let gaps = gaps.split("\n\n").next().unwrap();
    assert!(gaps.contains("price-ga"), "{gaps}");
    assert!(!gaps.contains("missing"), "{gaps}");
}
