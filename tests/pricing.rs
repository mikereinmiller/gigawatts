use gigawatts::pricing::PriceTable;
use gigawatts::sources::transcript::{Line, Request, Tokens, parse_line};

#[test]
fn prices_a_single_request_session_to_its_recorded_cost() {
    let line = r#"{"type":"assistant","sessionId":"s1","requestId":"r1","message":{"model":"claude-haiku-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":18597,"cache_read_input_tokens":9882,"output_tokens":244,"output_tokens_details":{"thinking_tokens":150},"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":18597,"ephemeral_5m_input_tokens":0},"inference_geo":"global","speed":"standard"}}}"#;
    let Some(Line::Request(request)) = parse_line(line) else {
        panic!("expected a request line");
    };

    let cost = PriceTable::built_in().price(&request).expect("priced");

    assert!((cost - 0.00394042).abs() < 1e-12, "cost was {cost}");
}

fn request(model: &str, tokens: Tokens, inference_geo: &str, speed: &str) -> Request {
    Request {
        session_id: "s1".to_string(),
        request_id: "r1".to_string(),
        model: model.to_string(),
        tokens,
        inference_geo: Some(inference_geo.to_string()),
        speed: Some(speed.to_string()),
        ..Request::default()
    }
}

fn price(request: &Request) -> f64 {
    PriceTable::built_in().price(request).expect("priced")
}

fn assert_cost(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn haiku_5_5_prompts_over_100k_tokens_pay_the_long_prompt_prices() {
    let tokens = Tokens {
        input: 50_000,
        cache_read: 50_001,
        output: 1_000,
        ..Tokens::default()
    };

    assert_cost(
        price(&request("claude-haiku-5-5", tokens, "global", "standard")),
        0.03000005,
    );
}

#[test]
fn haiku_5_5_prompts_of_exactly_100k_tokens_pay_the_standard_prices() {
    let tokens = Tokens {
        input: 50_000,
        cache_write_5m: 50_000,
        ..Tokens::default()
    };

    assert_cost(
        price(&request("claude-haiku-5-5", tokens, "global", "standard")),
        0.01125,
    );
}

#[test]
fn us_inference_costs_one_point_one_times_every_token_type() {
    let tokens = Tokens {
        input: 1_000_000,
        cache_read: 1_000_000,
        output: 1_000_000,
        ..Tokens::default()
    };

    assert_cost(
        price(&request("claude-sonnet-5-5", tokens, "us", "standard")),
        13.31,
    );
}

#[test]
fn fast_mode_replaces_input_and_output_prices_and_scales_cache_prices_with_input() {
    let tokens = Tokens {
        input: 1_000_000,
        cache_write_5m: 1_000_000,
        cache_write_1h: 1_000_000,
        cache_read: 1_000_000,
        output: 1_000_000,
        web_searches: 0,
    };

    assert_cost(
        price(&request("claude-opus-5-5", tokens, "global", "fast")),
        74.4,
    );
}

#[test]
fn fast_mode_and_us_inference_stack() {
    let tokens = Tokens {
        input: 1_000_000,
        output: 1_000_000,
        ..Tokens::default()
    };

    assert_cost(
        price(&request("claude-opus-5-5", tokens, "us", "fast")),
        52.8,
    );
}

#[test]
fn web_searches_cost_a_cent_each_on_top_of_tokens() {
    let tokens = Tokens {
        input: 10_000,
        web_searches: 3,
        ..Tokens::default()
    };

    assert_cost(
        price(&request("claude-haiku-5-5", tokens, "global", "standard")),
        0.031,
    );
}

#[test]
fn dated_model_ids_use_the_undated_price() {
    let tokens = Tokens {
        input: 1_000_000,
        ..Tokens::default()
    };

    assert_cost(
        price(&request(
            "claude-haiku-4-5-20251001",
            tokens,
            "global",
            "standard",
        )),
        1.0,
    );
}

#[test]
fn unknown_models_have_no_price() {
    let unknown = request(
        "claude-opus-9",
        Tokens {
            input: 1,
            ..Tokens::default()
        },
        "global",
        "standard",
    );

    assert_eq!(PriceTable::built_in().price(&unknown), None);
}
