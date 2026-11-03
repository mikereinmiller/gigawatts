use gigawatts::sources::transcript::{Corpus, Line, Tokens, parse_line};

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
fn counts_lines_mentioning_usage_that_are_not_requests_by_type() {
    let attachment = r#"{"type":"attachment","sessionId":"s1","content":{"usage":"see docs"}}"#;
    let user = r#"{"type":"user","sessionId":"s1","message":{"content":"no match here"}}"#;
    let broken = r#"{"type":"assistant","usage": "#;

    let corpus = corpus_of(&[attachment, user, broken]);

    assert_eq!(corpus.unrecognized.get("attachment"), Some(&1));
    assert_eq!(corpus.unrecognized.get("invalid JSON"), Some(&1));
    assert_eq!(corpus.unrecognized.len(), 2);
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
