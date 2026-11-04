use std::path::PathBuf;

use gigawatts::scrub::Scrubber;
use serde_json::Value;

const ENTERPRISE_RUN: &str = "2026-11-03T00-36-23Z";

fn capture_run() -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME is set");
    PathBuf::from(home)
        .join("gigawatts-enterprise-capture")
        .join("runs")
        .join(ENTERPRISE_RUN)
}

fn read(path: &std::path::Path) -> Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap()
}

fn strings_at(value: &Value, pointer_lists: &[&str], field: &str) -> Vec<String> {
    let mut found = Vec::new();
    for period in ["day", "week"] {
        for list in pointer_lists {
            let pointer = format!("/behaviors/{period}/{list}");
            for item in value
                .pointer(&pointer)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(text) = item.get(field).and_then(Value::as_str) {
                    found.push(text.to_string());
                }
            }
        }
    }
    found
}

fn fix_messages(value: &Value) -> Vec<String> {
    ["day", "week"]
        .iter()
        .filter_map(|period| value.pointer(&format!("/behaviors/{period}/fix/messages")))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "reads ~/gigawatts-enterprise-capture on the work Mac; run with --ignored"]
fn scrubbing_the_enterprise_capture_drops_its_names_and_text() {
    let run = capture_run();
    let get_usage = read(&run.join("get-usage.json"));
    let cached_usage = read(&run.join("cached-usage.json"));
    let mut names = strings_at(
        &get_usage,
        &["agents", "skills", "plugins", "mcp_servers"],
        "name",
    );
    names.retain(|name| !["general-purpose", "run"].contains(&name.as_str()));
    let mut text = strings_at(&get_usage, &["recommendations"], "title");
    text.extend(strings_at(&get_usage, &["recommendations"], "detail"));
    text.extend(fix_messages(&get_usage));
    let account_uuid = cached_usage["accountUuid"].as_str().unwrap().to_string();
    assert!(!names.is_empty() && !text.is_empty());

    let mut scrubber = Scrubber::for_fixture("enterprise");
    let scrubbed_get_usage = scrubber.scrub_usage(&get_usage).to_string();
    let scrubbed_cached = scrubber.scrub_usage(&cached_usage).to_string();

    assert!(
        !scrubbed_cached.contains(&account_uuid),
        "the account ID survived:\n{scrubbed_cached}"
    );
    for name in &names {
        assert!(
            !scrubbed_get_usage.contains(&format!("\"{name}\"")),
            "{name:?} survived:\n{scrubbed_get_usage}"
        );
    }
    for text in &text {
        assert!(
            !scrubbed_get_usage.contains(text.as_str()),
            "{text:?} survived:\n{scrubbed_get_usage}"
        );
    }
}
