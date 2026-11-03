use gigawatts::scrub::{Scrubber, scrub_sessions};
use gigawatts::sources::transcript::{Line, Tokens, parse_line};
use serde_json::Value;

const ASSISTANT: &str = r#"{"parentUuid":"5b0d1c3e-1111-4a2b-9c3d-0e1f2a3b4c5d","isSidechain":false,"attributionSkill":"deploy-widgets","attributionAgent":"general-purpose","message":{"model":"claude-opus-5-5","id":"msg_01AcmeRefund","type":"message","role":"assistant","content":[{"type":"text","text":"Secret plan for the Acme rollout"},{"type":"tool_use","id":"toolu_01X","name":"Bash","input":{"command":"cat /Users/jdoe/Code/acme-billing/.env"}}],"stop_reason":"tool_use","context_management":{"note":"Acme context"},"usage":{"input_tokens":3,"cache_creation_input_tokens":1200,"cache_read_input_tokens":45000,"output_tokens":512,"output_tokens_details":{"thinking_tokens":40},"server_tool_use":{"web_search_requests":1,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":1000,"ephemeral_5m_input_tokens":200},"inference_geo":"global","iterations":[{"input_tokens":3,"output_tokens":512,"cache_read_input_tokens":45000,"cache_creation_input_tokens":1200,"cache_creation":{"ephemeral_5m_input_tokens":200,"ephemeral_1h_input_tokens":1000},"type":"message"}],"speed":"standard","fallback_credit":null}},"requestId":"req_011AcmeRequest","type":"assistant","uuid":"9f8e7d6c-2222-4b3a-8d7c-6e5f4a3b2c1d","timestamp":"2026-10-14T15:04:05.678Z","userType":"external","entrypoint":"cli","cwd":"/Users/jdoe/Code/acme-billing","sessionId":"0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3","version":"2.1.317","gitBranch":"jdoe/acme-1234-refunds","slug":"acme-refund-bear","effort":"high","wireToolInputs":{"cmd":"acme"}}"#;

const USER: &str = r#"{"parentUuid":"9f8e7d6c-2222-4b3a-8d7c-6e5f4a3b2c1d","isSidechain":false,"promptId":"7c6b5a49-4444-4c3d-9e8f-7a6b5c4d3e2f","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01X","type":"tool_result","content":"ACME_STRIPE_KEY=sk_live_refund"}]},"uuid":"1e2d3c4b-5555-4a69-8b7a-9c8d7e6f5a4b","timestamp":"2026-10-14T15:04:09.000Z","toolUseResult":{"stdout":"ACME_STRIPE_KEY=sk_live_refund"},"sourceToolAssistantUUID":"9f8e7d6c-2222-4b3a-8d7c-6e5f4a3b2c1d","userType":"external","entrypoint":"cli","cwd":"/Users/jdoe/Code/acme-billing","sessionId":"0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3","version":"2.1.317","gitBranch":"jdoe/acme-1234-refunds"}"#;

const TITLE: &str = r#"{"type":"ai-title","sessionId":"0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3","aiTitle":"Fix Acme refund rounding"}"#;

const COST_STATE: &str = r#"{"type":"cost-state","sessionId":"0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3","totalCostUSD":0.25,"totalAPIDuration":6000,"totalLinesAdded":3,"startTime":1791990245678,"modelUsage":{"claude-opus-5-5":{"inputTokens":3,"outputTokens":512,"thinkingTokens":40,"cacheReadInputTokens":45000,"cacheCreationInputTokens":1200,"webSearchRequests":1,"costUSD":0.25}},"hasUnknownModelCost":false,"lastPrompt":"refund the Acme invoices"}"#;

const SECRETS: [&str; 10] = [
    "/Users/jdoe/Code/acme-billing",
    "jdoe",
    "acme",
    "Acme",
    "ACME",
    "refund",
    "Secret plan",
    "deploy-widgets",
    "sk_live",
    "Bash",
];

fn scrub_all(lines: &[&str]) -> Vec<String> {
    let mut scrubber = Scrubber::default();
    lines
        .iter()
        .filter_map(|line| scrubber.scrub_line(line))
        .collect()
}

fn json(line: &str) -> Value {
    serde_json::from_str(line).expect("scrubbed line is JSON")
}

#[test]
fn scrubbed_lines_contain_none_of_the_source_names_or_text() {
    let scrubbed = scrub_all(&[ASSISTANT, USER, TITLE, COST_STATE]).join("\n");

    for secret in SECRETS {
        assert!(!scrubbed.contains(secret), "{secret} survived:\n{scrubbed}");
    }
}

#[test]
fn a_scrubbed_request_prices_the_same() {
    let scrubbed = scrub_all(&[ASSISTANT]);

    let Some(Line::Request(request)) = parse_line(&scrubbed[0]) else {
        panic!("expected a request: {}", scrubbed[0])
    };
    assert_eq!(request.model, "claude-opus-5-5");
    assert_eq!(request.inference_geo.as_deref(), Some("global"));
    assert_eq!(request.speed.as_deref(), Some("standard"));
    assert_eq!(
        request.tokens,
        Tokens {
            input: 3,
            output: 512,
            cache_read: 45000,
            cache_write_5m: 200,
            cache_write_1h: 1000,
            web_searches: 1,
        }
    );
}

#[test]
fn renames_each_id_and_name_the_same_way_everywhere() {
    let scrubbed = scrub_all(&[ASSISTANT, USER, TITLE, COST_STATE]);
    let assistant = json(&scrubbed[0]);
    let user = json(&scrubbed[1]);
    let title = json(&scrubbed[2]);
    let cost_state = json(&scrubbed[3]);

    let session = &assistant["sessionId"];
    assert_ne!(session, "0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3");
    assert_eq!(&user["sessionId"], session);
    assert_eq!(&title["sessionId"], session);
    assert_eq!(&cost_state["sessionId"], session);
    assert_eq!(user["parentUuid"], assistant["uuid"]);
    assert_eq!(user["sourceToolAssistantUUID"], assistant["uuid"]);
    assert_eq!(user["cwd"], assistant["cwd"]);
    assert_eq!(user["gitBranch"], assistant["gitBranch"]);
    assert_ne!(assistant["requestId"], "req_011AcmeRequest");
    assert_ne!(assistant["message"]["id"], "msg_01AcmeRefund");
}

#[test]
fn keeps_the_shape_of_paths_and_built_in_names() {
    let scrubbed = scrub_all(&[ASSISTANT]);
    let assistant = json(&scrubbed[0]);

    let cwd = assistant["cwd"].as_str().unwrap();
    assert!(cwd.starts_with("/Users/"), "{cwd}");
    assert_eq!(cwd.split('/').count(), 5, "{cwd}");
    assert_eq!(assistant["attributionAgent"], "general-purpose");
    assert_ne!(assistant["attributionSkill"], "deploy-widgets");
}

#[test]
fn keeps_main_and_renames_other_branches() {
    let on_main = ASSISTANT.replace("jdoe/acme-1234-refunds", "main");

    let scrubbed = scrub_all(&[&on_main]);

    assert_eq!(json(&scrubbed[0])["gitBranch"], "main");
}

#[test]
fn shifts_timestamps_by_whole_weeks_keeping_the_weekday_and_clock_time() {
    let scrubbed = scrub_all(&[ASSISTANT, USER, COST_STATE]);

    assert_eq!(json(&scrubbed[0])["timestamp"], "2026-01-07T15:04:05.678Z");
    assert_eq!(json(&scrubbed[1])["timestamp"], "2026-01-07T15:04:09.000Z");
    let weeks_ms: i64 = 40 * 7 * 86_400_000;
    assert_eq!(
        json(&scrubbed[2])["startTime"],
        1_791_990_245_678_i64 - weeks_ms
    );
}

#[test]
fn drops_message_text_tool_io_and_fields_it_does_not_know() {
    let scrubbed = scrub_all(&[ASSISTANT, USER, COST_STATE]);
    let assistant = json(&scrubbed[0]);
    let user = json(&scrubbed[1]);
    let cost_state = json(&scrubbed[2]);

    assert!(assistant["message"].get("content").is_none());
    assert!(assistant["message"].get("context_management").is_none());
    assert!(assistant.get("slug").is_none());
    assert!(assistant.get("wireToolInputs").is_none());
    assert_eq!(
        assistant["message"]["usage"]["output_tokens_details"]["thinking_tokens"],
        40
    );
    assert!(user["message"].get("content").is_none());
    assert!(user.get("toolUseResult").is_none());
    assert!(cost_state.get("lastPrompt").is_none());
    assert_eq!(
        cost_state["modelUsage"]["claude-opus-5-5"]["thinkingTokens"],
        40
    );
}

#[test]
fn drops_line_types_it_does_not_know() {
    let attachment = r#"{"type":"attachment","sessionId":"s1","attachment":{"content":"Acme"}}"#;

    assert!(scrub_all(&[attachment]).is_empty());
}

#[test]
fn scrubs_a_session_with_its_subagents_into_one_project_folder() {
    let root = std::env::temp_dir().join(format!("gigawatts-scrub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let session = "0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3";
    let source = root.join("source");
    let subagents = source.join(session).join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::write(
        source.join(format!("{session}.jsonl")),
        format!("{ASSISTANT}\n{TITLE}\n"),
    )
    .unwrap();
    let subagent_line = ASSISTANT
        .replace("req_011AcmeRequest", "req_011AcmeSubagent")
        .replace(
            r#""isSidechain":false"#,
            r#""isSidechain":true,"agentId":"a0123456789abcdef""#,
        );
    std::fs::write(
        subagents.join("agent-a0123456789abcdef.jsonl"),
        format!("{subagent_line}\n"),
    )
    .unwrap();
    std::fs::write(subagents.join("agent-a0123456789abcdef.meta.json"), "{}").unwrap();
    let out = root.join("out");

    scrub_sessions(&[source.join(format!("{session}.jsonl"))], &out).expect("scrub");

    let parent_lines: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    assert_eq!(parent_lines.len(), 1);
    let parent = json(parent_lines[0].lines().next().unwrap());
    let renamed_session = parent["sessionId"].as_str().unwrap();
    let agent_files: Vec<_> = std::fs::read_dir(out.join(renamed_session).join("subagents"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(agent_files.len(), 1, "{agent_files:?}");
    let agent_line = json(std::fs::read_to_string(&agent_files[0]).unwrap().trim());
    assert_eq!(agent_line["sessionId"], renamed_session);
    let agent_id = agent_line["agentId"].as_str().unwrap();
    assert_eq!(
        agent_files[0].file_name().unwrap().to_str().unwrap(),
        format!("agent-{agent_id}.jsonl")
    );
    assert!(out.join(format!("{renamed_session}.jsonl")).is_file());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn ids_from_two_fixtures_never_collide() {
    let mut first = Scrubber::for_fixture("plain");
    let mut second = Scrubber::for_fixture("subagents");

    let a = json(&first.scrub_line(ASSISTANT).unwrap());
    let b = json(&second.scrub_line(ASSISTANT).unwrap());

    for field in ["sessionId", "uuid", "requestId"] {
        assert_ne!(a[field], b[field], "{field}");
    }
    assert_ne!(a["message"]["id"], b["message"]["id"]);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("gigawatts-scrub-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn sessions_scrubbed_together_share_one_rename_map() {
    let root = scratch("together");
    let first = root.join("0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3.jsonl");
    let second = root.join("1b2c3d4e-6666-4f5a-b6c7-d8e9f0a1b2c3.jsonl");
    std::fs::write(&first, format!("{ASSISTANT}\n")).unwrap();
    let other_session = ASSISTANT
        .replace(
            "0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3",
            "1b2c3d4e-6666-4f5a-b6c7-d8e9f0a1b2c3",
        )
        .replace("req_011AcmeRequest", "req_011AcmeOther")
        .replace("msg_01AcmeRefund", "msg_01AcmeOther");
    std::fs::write(&second, format!("{other_session}\n")).unwrap();
    let out = root.join("project");

    scrub_sessions(&[first, second], &out).expect("scrub");

    let lines: Vec<Value> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| {
            json(
                std::fs::read_to_string(entry.unwrap().path())
                    .unwrap()
                    .trim(),
            )
        })
        .collect();
    assert_eq!(lines.len(), 2);
    assert_ne!(lines[0]["sessionId"], lines[1]["sessionId"]);
    assert_ne!(lines[0]["requestId"], lines[1]["requestId"]);
    assert_eq!(lines[0]["cwd"], lines[1]["cwd"]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn refuses_to_overwrite_a_scrubbed_file() {
    let root = scratch("overwrite");
    let transcript = root.join("0a1b2c3d-3333-4e5f-a6b7-c8d9e0f1a2b3.jsonl");
    std::fs::write(&transcript, format!("{ASSISTANT}\n")).unwrap();
    let out = root.join("project");
    scrub_sessions(std::slice::from_ref(&transcript), &out).expect("first scrub");
    let written = std::fs::read_dir(&out)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let before = std::fs::read_to_string(&written).unwrap();

    let error = scrub_sessions(&[transcript], &out).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(std::fs::read_to_string(&written).unwrap(), before);
    std::fs::remove_dir_all(&root).unwrap();
}
