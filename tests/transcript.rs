use gigawatts::sources::transcript::{
    Corpus, Line, RequestDetail, SessionTotals, Tokens, parse_line,
};

const BLOCK: &str = r#"{"type":"assistant","sessionId":"s1","requestId":"r1","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_5m_input_tokens":20,"ephemeral_1h_input_tokens":10}}}}"#;

fn corpus_of(lines: &[&str]) -> Corpus {
    let mut corpus = Corpus::default();
    for line in lines {
        if let Some(parsed) = parse_line(line) {
            corpus.add(parsed);
        }
    }
    corpus
}

#[test]
fn content_blocks_of_one_request_count_once() {
    let corpus = corpus_of(&[BLOCK, BLOCK]);

    assert_eq!(corpus.requests.len(), 1);
}

#[test]
fn reads_the_cache_write_split() {
    let Some(Line::Request(request)) = parse_line(BLOCK) else {
        panic!("expected a request")
    };

    assert_eq!(
        request.tokens,
        Tokens {
            input: 2,
            output: 10,
            cache_read: 100,
            cache_write_5m: 20,
            cache_write_1h: 10,
            web_searches: 0
        }
    );
}

#[test]
fn cost_states_without_a_cache_write_split_count_writes_as_5_minute() {
    let line = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.5,"modelUsage":{"claude-haiku-5-5":{"inputTokens":4,"outputTokens":396,"cacheReadInputTokens":41436,"cacheCreationInputTokens":815,"webSearchRequests":1,"costUSD":0.5}}}"#;
    let Some(Line::CostState(state)) = parse_line(line) else {
        panic!("expected a cost-state")
    };

    let haiku = &state.models["claude-haiku-5-5"];
    assert!(!haiku.has_cache_write_split);
    assert_eq!(haiku.tokens.cache_write_5m, 815);
    assert_eq!(haiku.tokens.web_searches, 1);
    assert_eq!(haiku.cost_usd, 0.5);
}

#[test]
fn keeps_the_largest_cost_state_per_session() {
    let later = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":2.0,"modelUsage":{}}"#;
    let earlier = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":1.0,"modelUsage":{}}"#;

    let corpus = corpus_of(&[later, earlier]);

    assert_eq!(corpus.cost_states["s1"].total_cost_usd, 2.0);
}

#[test]
fn counts_lines_of_other_types_only_when_their_message_carries_usage() {
    let request_like = r#"{"type":"assistant-v2","sessionId":"s1","message":{"model":"claude-opus-5-5","usage":{"input_tokens":1,"output_tokens":1}}}"#;
    let summary = r#"{"type":"attachment","sessionId":"s1","attachment":{"type":"queued_command","usage":{"totalTokens":145018,"toolUses":23,"durationMs":299170}}}"#;
    let text = r#"{"type":"system","sessionId":"s1","content":"run /usage to see \"usage\""}"#;
    let user = r#"{"type":"user","sessionId":"s1","message":{"content":"no match here"}}"#;
    let broken = r#"{"type":"assistant","usage": "#;

    let corpus = corpus_of(&[request_like, summary, text, user, broken]);

    assert_eq!(corpus.unrecognized.get("assistant-v2"), Some(&1));
    assert_eq!(corpus.unrecognized.get("invalid JSON"), Some(&1));
    assert_eq!(corpus.unrecognized.len(), 2, "{:?}", corpus.unrecognized);
}

#[test]
fn counts_a_known_line_type_that_doesnt_parse() {
    let cost_state = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":1.0}"#;

    let corpus = corpus_of(&[cost_state]);

    assert_eq!(corpus.unrecognized.get("cost-state"), Some(&1));
}

#[test]
fn skips_synthetic_messages_claude_code_writes_for_errors() {
    let synthetic = r#"{"type":"assistant","sessionId":"s1","requestId":"r9","message":{"model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}}}"#;

    let corpus = corpus_of(&[synthetic]);

    assert!(corpus.requests.is_empty());
    assert!(corpus.unrecognized.is_empty());
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gigawatts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn reads_subagent_transcripts_under_their_parent_session() {
    let root = scratch_dir("walk");
    let session_dir = root.join("project").join("s1");
    std::fs::create_dir_all(session_dir.join("subagents")).unwrap();
    std::fs::write(root.join("project").join("s1.jsonl"), BLOCK).unwrap();
    std::fs::write(
        session_dir.join("subagents").join("agent-x.jsonl"),
        BLOCK.replace(r#""requestId":"r1""#, r#""requestId":"r2""#),
    )
    .unwrap();
    std::fs::write(session_dir.join("notes.txt"), BLOCK).unwrap();

    let corpus = Corpus::read(&root).expect("read");

    assert_eq!(corpus.files, 2);
    let mut keys: Vec<_> = corpus.requests.keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            ("s1".to_string(), "r1".to_string()),
            ("s1".to_string(), "r2".to_string())
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_that_cannot_be_read_names_itself_in_the_error() {
    let root = scratch_dir("missing");
    let missing = root.join("gone.jsonl");

    let error = Corpus::default().read_file(&missing).unwrap_err();

    assert!(
        error.to_string().contains(&missing.display().to_string()),
        "{error}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn reads_every_numeric_and_categorical_request_field() {
    let line = r#"{"type":"assistant","sessionId":"s1","requestId":"r1","timestamp":"2026-01-08T23:47:05.959Z","cwd":"/Users/dir-1/dir-2","gitBranch":"branch-1","requestedModel":"claude-opus-5-5","advisorModel":"claude-fable-5-1","effort":"medium","perTurnEffort":"high","entrypoint":"cli","version":"2.1.295","userType":"external","isSidechain":true,"agentId":"a1","attributionSkill":"skill-1","attributionAgent":"agent-1","thinkingDurationMs":590,"message":{"model":"claude-opus-5-5","stop_reason":"tool_use","content":[{"type":"text","text":"x"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a"}},{"type":"tool_use","id":"t2","name":"mcp__server-1__tool-1","input":{}}],"usage":{"input_tokens":2,"output_tokens":178,"output_tokens_details":{"thinking_tokens":29},"server_tool_use":{"web_search_requests":1,"web_fetch_requests":3},"service_tier":"standard","inference_geo":"not_available","speed":"standard"}}}"#;
    let Some(Line::Request(request)) = parse_line(line) else {
        panic!("expected a request")
    };

    assert_eq!(
        request.detail,
        RequestDetail {
            timestamp: Some("2026-01-08T23:47:05.959Z".to_string()),
            cwd: Some("/Users/dir-1/dir-2".to_string()),
            git_branch: Some("branch-1".to_string()),
            requested_model: Some("claude-opus-5-5".to_string()),
            advisor_model: Some("claude-fable-5-1".to_string()),
            effort: Some("medium".to_string()),
            per_turn_effort: Some("high".to_string()),
            service_tier: Some("standard".to_string()),
            stop_reason: Some("tool_use".to_string()),
            entrypoint: Some("cli".to_string()),
            version: Some("2.1.295".to_string()),
            user_type: Some("external".to_string()),
            is_sidechain: true,
            agent_id: Some("a1".to_string()),
            attribution_skill: Some("skill-1".to_string()),
            attribution_agent: Some("agent-1".to_string()),
            thinking_duration_ms: Some(590),
            thinking_tokens: 29,
            web_fetches: 3,
            tools: vec!["Read".to_string(), "mcp__server-1__tool-1".to_string()],
        }
    );
    assert_eq!(request.tokens.web_searches, 1);
}

#[test]
fn reads_session_totals_from_a_cost_state() {
    let line = r#"{"type":"cost-state","sessionId":"s1","startTime":1767916011592,"totalAPIDuration":232195,"totalAPIDurationWithoutRetries":180238,"totalCostUSD":0.5,"totalDuration":5491096,"totalLinesAdded":12,"totalLinesRemoved":3,"totalToolDuration":29233,"hasUnknownModelCost":false,"modelUsage":{"claude-haiku-5-5":{"inputTokens":1176,"outputTokens":21,"thinkingTokens":7,"webFetchRequests":2,"costUSD":0.5}}}"#;
    let Some(Line::CostState(state)) = parse_line(line) else {
        panic!("expected a cost-state")
    };

    assert_eq!(
        state.totals,
        SessionTotals {
            start_time_ms: Some(1767916011592),
            api_duration_ms: Some(232195),
            api_duration_without_retries_ms: Some(180238),
            tool_duration_ms: Some(29233),
            duration_ms: Some(5491096),
            lines_added: Some(12),
            lines_removed: Some(3),
            has_unknown_model_cost: Some(false),
        }
    );
    let haiku = &state.models["claude-haiku-5-5"];
    assert_eq!((haiku.thinking_tokens, haiku.web_fetches), (7, 2));
}

#[test]
fn reads_the_session_title() {
    let line = r#"{"type":"ai-title","sessionId":"s1","aiTitle":"Title 1"}"#;

    assert_eq!(
        parse_line(line),
        Some(Line::Title {
            session_id: "s1".to_string(),
            title: "Title 1".to_string()
        })
    );
}

#[test]
fn reads_the_title_its_user_gave_the_session() {
    let line = r#"{"type":"custom-title","sessionId":"s1","customTitle":"Login page"}"#;

    assert_eq!(
        parse_line(line),
        Some(Line::CustomTitle {
            session_id: "s1".to_string(),
            title: "Login page".to_string()
        })
    );
}

#[test]
fn a_request_whose_content_is_text_has_no_tools() {
    let line = r#"{"type":"assistant","sessionId":"s1","requestId":"r1","message":{"model":"claude-opus-5-5","content":"plain text","usage":{"input_tokens":2,"output_tokens":10}}}"#;
    let Some(Line::Request(request)) = parse_line(line) else {
        panic!("expected a request")
    };

    assert!(request.detail.tools.is_empty());
}
