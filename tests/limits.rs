use std::path::{Path, PathBuf};

use gigawatts::contract::{Credits, Limit};
use gigawatts::sources::claude_json::{CachedUsage, read_cached_usage};
use gigawatts::sources::get_usage::{self, Usage};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/usage")
        .join(name)
}

fn get_usage_fixture(name: &str) -> Usage {
    let text = std::fs::read_to_string(fixture(name).join("get-usage.json")).unwrap();
    get_usage::parse_usage(&text).unwrap()
}

fn cached_usage_fixture(name: &str) -> CachedUsage {
    let reading = std::fs::read_to_string(fixture(name).join("cached-usage.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let claude_json = dir.path().join(".claude.json");
    std::fs::write(
        &claude_json,
        format!(r#"{{"numStartups":3,"cachedUsageUtilization":{reading}}}"#),
    )
    .unwrap();
    read_cached_usage(&claude_json).unwrap().unwrap()
}

fn enterprise_credits() -> Vec<Limit> {
    vec![Limit {
        kind: "credits".to_string(),
        group: None,
        scope: None,
        percent: 20.0,
        severity: Some("normal".to_string()),
        resets_at: None,
        active: false,
        credits: Some(Credits {
            used: 437.6,
            limit: Some(2200.0),
            currency: "USD".to_string(),
        }),
    }]
}

fn window(
    kind: &str,
    group: &str,
    scope: Option<&str>,
    percent: f64,
    severity: &str,
    resets_at: &str,
    active: bool,
) -> Limit {
    Limit {
        kind: kind.to_string(),
        group: Some(group.to_string()),
        scope: scope.map(str::to_string),
        percent,
        severity: Some(severity.to_string()),
        resets_at: Some(resets_at.to_string()),
        active,
        credits: None,
    }
}

#[test]
fn the_enterprise_response_has_only_monthly_credits_from_spend() {
    let usage = get_usage_fixture("enterprise");

    assert_eq!(usage.limits, enterprise_credits());
    assert!(usage.behaviors.is_some());
}

#[test]
fn the_enterprise_cached_reading_matches_its_response() {
    let cached = cached_usage_fixture("enterprise");

    assert_eq!(cached.limits, enterprise_credits());
    assert_eq!(cached.fetched_at_ms, 1_793_666_192_807);
    assert_eq!(cached.account_uuid, "00000001-0000-4000-8000-7171d5ca7ac2");
}

#[test]
fn the_ultra_response_has_its_three_windows_and_no_credits() {
    let usage = get_usage_fixture("ultra");

    assert_eq!(
        usage.limits,
        vec![
            window(
                "session",
                "session",
                None,
                2.0,
                "normal",
                "2026-11-04T23:59:59.945577+00:00",
                false
            ),
            window(
                "weekly_all",
                "weekly",
                None,
                63.0,
                "normal",
                "2026-11-08T02:59:59.945600+00:00",
                false
            ),
            window(
                "weekly_scoped",
                "weekly",
                Some("Fable"),
                90.0,
                "critical",
                "2026-11-08T02:59:59.945750+00:00",
                true
            ),
        ]
    );
}

#[test]
fn the_ultra_cached_reading_has_the_same_windows() {
    let cached = cached_usage_fixture("ultra");

    assert_eq!(cached.fetched_at_ms, 1_793_816_860_760);
    assert_eq!(
        cached.limits,
        vec![
            window(
                "session",
                "session",
                None,
                39.0,
                "normal",
                "2026-11-04T19:09:59.664553+00:00",
                false
            ),
            window(
                "weekly_all",
                "weekly",
                None,
                51.0,
                "normal",
                "2026-11-09T03:59:59.664575+00:00",
                false
            ),
            window(
                "weekly_scoped",
                "weekly",
                Some("Fable"),
                61.0,
                "normal",
                "2026-11-09T03:59:59.664737+00:00",
                true
            ),
        ]
    );
}

#[test]
fn credits_fall_back_to_extra_usage_when_spend_is_missing() {
    let usage = get_usage::parse_usage(
        r#"{"rate_limits":{"limits":[],"extra_usage":{"is_enabled":true,"monthly_limit":5000,"used_credits":1234,"utilization":24.68,"currency":"USD","decimal_places":2}}}"#,
    )
    .unwrap();

    assert_eq!(
        usage.limits,
        vec![Limit {
            kind: "credits".to_string(),
            group: None,
            scope: None,
            percent: 24.68,
            severity: None,
            resets_at: None,
            active: false,
            credits: Some(Credits {
                used: 12.34,
                limit: Some(50.0),
                currency: "USD".to_string(),
            }),
        }]
    );
}

#[test]
fn a_claude_json_without_a_reading_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let claude_json = dir.path().join(".claude.json");
    std::fs::write(&claude_json, r#"{"cachedUsageUtilization":null}"#).unwrap();

    assert_eq!(read_cached_usage(&claude_json).unwrap(), None);
    assert_eq!(
        read_cached_usage(&dir.path().join("missing.json")).unwrap(),
        None
    );
}

#[test]
fn the_control_response_is_found_in_the_stream() {
    let response = std::fs::read_to_string(fixture("ultra").join("get-usage.json")).unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    let stream = format!(
        "{}\n{}\n",
        r#"{"type":"system","subtype":"hook_started"}"#,
        serde_json::json!({"type":"control_response","response":{"subtype":"success","request_id":get_usage::REQUEST_ID,"response":response}})
    );

    let usage = get_usage::parse_stream(&stream).unwrap();

    assert_eq!(usage.limits.len(), 3);
}

#[test]
fn an_error_response_names_the_error() {
    let stream = serde_json::json!({"type":"control_response","response":{"subtype":"error","request_id":get_usage::REQUEST_ID,"error":"Not logged in"}})
        .to_string();

    let error = get_usage::parse_stream(&stream).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Claude Code answered with an error: Not logged in"
    );
}

#[test]
fn an_answer_that_doesnt_parse_says_why() {
    let stream = serde_json::json!({"type":"control_response","response":{"subtype":"error","request_id":get_usage::REQUEST_ID,"error":{"code":"rate_limited"}}})
        .to_string();

    let error = get_usage::parse_stream(&stream).unwrap_err();

    assert!(
        error.to_string().starts_with(
            "Claude Code's usage answer doesn't parse: invalid type: map, expected a string"
        ),
        "{error}"
    );
}
