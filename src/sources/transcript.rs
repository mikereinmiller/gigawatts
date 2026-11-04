use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{Deserializer, SeqAccess, Visitor};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub web_searches: u64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.web_searches += other.web_searches;
    }

    pub fn cache_write(&self) -> u64 {
        self.cache_write_5m + self.cache_write_1h
    }

    pub fn prompt(&self) -> u64 {
        self.input + self.cache_read + self.cache_write()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub session_id: String,
    pub request_id: String,
    pub model: String,
    pub tokens: Tokens,
    pub inference_geo: Option<String>,
    pub speed: Option<String>,
    pub detail: RequestDetail,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestDetail {
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub requested_model: Option<String>,
    pub advisor_model: Option<String>,
    pub effort: Option<String>,
    pub per_turn_effort: Option<String>,
    pub service_tier: Option<String>,
    pub stop_reason: Option<String>,
    pub entrypoint: Option<String>,
    pub version: Option<String>,
    pub user_type: Option<String>,
    pub is_sidechain: bool,
    pub agent_id: Option<String>,
    pub attribution_skill: Option<String>,
    pub attribution_agent: Option<String>,
    pub thinking_duration_ms: Option<u64>,
    pub thinking_tokens: u64,
    pub web_fetches: u64,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelCost {
    pub tokens: Tokens,
    pub has_cache_write_split: bool,
    pub cost_usd: f64,
    pub thinking_tokens: u64,
    pub web_fetches: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostState {
    pub session_id: String,
    pub total_cost_usd: f64,
    pub models: BTreeMap<String, ModelCost>,
    pub totals: SessionTotals,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionTotals {
    pub start_time_ms: Option<i64>,
    pub api_duration_ms: Option<u64>,
    pub api_duration_without_retries_ms: Option<u64>,
    pub tool_duration_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
    pub has_unknown_model_cost: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Request(Box<Request>),
    CostState(CostState),
    Title {
        session_id: String,
        title: String,
    },
    Unrecognized {
        kind: String,
        timestamp: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLine<'a> {
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    request_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<RawMessage<'a>>,
    #[serde(rename = "totalCostUSD")]
    total_cost_usd: Option<f64>,
    #[serde(borrow)]
    model_usage: Option<HashMap<Cow<'a, str>, RawModelUsage>>,
    #[serde(borrow)]
    ai_title: Option<Cow<'a, str>>,
    #[serde(borrow)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(borrow)]
    cwd: Option<Cow<'a, str>>,
    #[serde(borrow)]
    git_branch: Option<Cow<'a, str>>,
    #[serde(borrow)]
    requested_model: Option<Cow<'a, str>>,
    #[serde(borrow)]
    advisor_model: Option<Cow<'a, str>>,
    #[serde(borrow)]
    effort: Option<Cow<'a, str>>,
    #[serde(borrow)]
    per_turn_effort: Option<Cow<'a, str>>,
    #[serde(borrow)]
    entrypoint: Option<Cow<'a, str>>,
    #[serde(borrow)]
    version: Option<Cow<'a, str>>,
    #[serde(borrow)]
    user_type: Option<Cow<'a, str>>,
    is_sidechain: Option<bool>,
    #[serde(borrow)]
    agent_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    attribution_skill: Option<Cow<'a, str>>,
    #[serde(borrow)]
    attribution_agent: Option<Cow<'a, str>>,
    thinking_duration_ms: Option<u64>,
    start_time: Option<i64>,
    #[serde(rename = "totalAPIDuration")]
    total_api_duration: Option<u64>,
    #[serde(rename = "totalAPIDurationWithoutRetries")]
    total_api_duration_without_retries: Option<u64>,
    total_tool_duration: Option<u64>,
    total_duration: Option<u64>,
    total_lines_added: Option<u64>,
    total_lines_removed: Option<u64>,
    has_unknown_model_cost: Option<bool>,
}

#[derive(Deserialize)]
struct RawMessage<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    #[serde(borrow)]
    stop_reason: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "tool_names")]
    content: Vec<String>,
    #[serde(borrow)]
    usage: Option<RawUsage<'a>>,
}

#[derive(Deserialize)]
struct RawBlock<'a> {
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    name: Option<Cow<'a, str>>,
}

fn tool_names<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    deserializer.deserialize_any(ToolNames)
}

struct ToolNames;

impl<'de> Visitor<'de> for ToolNames {
    type Value = Vec<String>;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("message content")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut blocks: A) -> Result<Vec<String>, A::Error> {
        let mut names = Vec::new();
        while let Some(block) = blocks.next_element::<RawBlock<'de>>()? {
            if block.kind.as_deref() == Some("tool_use")
                && let Some(name) = block.name
            {
                names.push(name.into_owned());
            }
        }
        Ok(names)
    }

    fn visit_str<E>(self, _text: &str) -> Result<Vec<String>, E> {
        Ok(Vec::new())
    }

    fn visit_unit<E>(self) -> Result<Vec<String>, E> {
        Ok(Vec::new())
    }
}

#[derive(Deserialize)]
struct RawUsage<'a> {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    cache_creation: Option<RawCacheCreation>,
    server_tool_use: Option<RawServerToolUse>,
    output_tokens_details: Option<RawOutputDetails>,
    #[serde(borrow)]
    service_tier: Option<Cow<'a, str>>,
    #[serde(borrow)]
    inference_geo: Option<Cow<'a, str>>,
    #[serde(borrow)]
    speed: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct RawCacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

#[derive(Deserialize)]
struct RawServerToolUse {
    #[serde(default)]
    web_search_requests: u64,
    #[serde(default)]
    web_fetch_requests: u64,
}

#[derive(Deserialize)]
struct RawOutputDetails {
    #[serde(default)]
    thinking_tokens: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawModelUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    cache_creation_ephemeral5m_input_tokens: Option<u64>,
    cache_creation_ephemeral1h_input_tokens: Option<u64>,
    #[serde(default)]
    web_search_requests: u64,
    #[serde(default)]
    thinking_tokens: u64,
    #[serde(default)]
    web_fetch_requests: u64,
    #[serde(rename = "costUSD", default)]
    cost_usd: f64,
}

const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Raise it whenever `parse_line` reads lines it used to reject, so every stored transcript is read again.
pub const READER_VERSION: u32 = 1;

pub fn is_candidate(line: &str) -> bool {
    line.contains("\"usage\"") || line.contains("\"cost-state\"") || line.contains("\"ai-title\"")
}

pub fn parse_line(line: &str) -> Option<Line> {
    if !is_candidate(line) {
        return None;
    }
    let Ok(raw) = serde_json::from_str::<RawLine>(line) else {
        return Some(Line::Unrecognized {
            kind: "invalid JSON".to_string(),
            timestamp: None,
        });
    };
    let kind = raw.kind.as_deref().unwrap_or("untyped").to_string();
    let timestamp = raw.timestamp.as_deref().map(str::to_string);
    let unrecognized = Line::Unrecognized {
        kind: kind.clone(),
        timestamp,
    };
    let parsed = match kind.as_str() {
        "assistant" if is_synthetic(&raw) => return None,
        "assistant" => request_from(raw),
        "cost-state" => cost_state_from(raw),
        "ai-title" => title_from(raw),
        _ if carries_usage(&raw) => return Some(unrecognized),
        _ => return None,
    };
    Some(parsed.unwrap_or(unrecognized))
}

fn carries_usage(raw: &RawLine) -> bool {
    raw.message
        .as_ref()
        .is_some_and(|message| message.usage.is_some())
}

fn is_synthetic(raw: &RawLine) -> bool {
    raw.message.as_ref().and_then(|m| m.model.as_deref()) == Some(SYNTHETIC_MODEL)
}

fn request_from(raw: RawLine) -> Option<Line> {
    let message = raw.message?;
    let usage = message.usage?;
    let model = message.model?;
    let request_id = raw.request_id.or(message.id)?;
    let cache_write = match &usage.cache_creation {
        Some(split) => (
            split.ephemeral_5m_input_tokens,
            split.ephemeral_1h_input_tokens,
        ),
        None => (usage.cache_creation_input_tokens, 0),
    };
    let (web_searches, web_fetches) = usage
        .server_tool_use
        .map_or((0, 0), |s| (s.web_search_requests, s.web_fetch_requests));
    let tokens = Tokens {
        input: usage.input_tokens,
        output: usage.output_tokens,
        cache_read: usage.cache_read_input_tokens,
        cache_write_5m: cache_write.0,
        cache_write_1h: cache_write.1,
        web_searches,
    };
    let detail = RequestDetail {
        timestamp: owned(raw.timestamp),
        cwd: owned(raw.cwd),
        git_branch: owned(raw.git_branch),
        requested_model: owned(raw.requested_model),
        advisor_model: owned(raw.advisor_model),
        effort: owned(raw.effort),
        per_turn_effort: owned(raw.per_turn_effort),
        service_tier: owned(usage.service_tier),
        stop_reason: owned(message.stop_reason),
        entrypoint: owned(raw.entrypoint),
        version: owned(raw.version),
        user_type: owned(raw.user_type),
        is_sidechain: raw.is_sidechain.unwrap_or(false),
        agent_id: owned(raw.agent_id),
        attribution_skill: owned(raw.attribution_skill),
        attribution_agent: owned(raw.attribution_agent),
        thinking_duration_ms: raw.thinking_duration_ms,
        thinking_tokens: usage.output_tokens_details.map_or(0, |d| d.thinking_tokens),
        web_fetches,
        tools: message.content,
    };
    Some(Line::Request(Box::new(Request {
        session_id: raw.session_id?.into_owned(),
        request_id: request_id.into_owned(),
        model: model.into_owned(),
        tokens,
        inference_geo: owned(usage.inference_geo),
        speed: owned(usage.speed),
        detail,
    })))
}

fn owned(text: Option<Cow<str>>) -> Option<String> {
    text.map(Cow::into_owned)
}

fn title_from(raw: RawLine) -> Option<Line> {
    Some(Line::Title {
        session_id: raw.session_id?.into_owned(),
        title: raw.ai_title?.into_owned(),
    })
}

fn cost_state_from(raw: RawLine) -> Option<Line> {
    let models = raw
        .model_usage?
        .into_iter()
        .map(|(model, usage)| (model.into_owned(), model_cost_from(usage)))
        .collect();
    let totals = SessionTotals {
        start_time_ms: raw.start_time,
        api_duration_ms: raw.total_api_duration,
        api_duration_without_retries_ms: raw.total_api_duration_without_retries,
        tool_duration_ms: raw.total_tool_duration,
        duration_ms: raw.total_duration,
        lines_added: raw.total_lines_added,
        lines_removed: raw.total_lines_removed,
        has_unknown_model_cost: raw.has_unknown_model_cost,
    };
    Some(Line::CostState(CostState {
        session_id: raw.session_id?.into_owned(),
        total_cost_usd: raw.total_cost_usd.unwrap_or(0.0),
        models,
        totals,
    }))
}

fn model_cost_from(usage: RawModelUsage) -> ModelCost {
    let split = usage
        .cache_creation_ephemeral5m_input_tokens
        .zip(usage.cache_creation_ephemeral1h_input_tokens);
    let (cache_write_5m, cache_write_1h) = split.unwrap_or((usage.cache_creation_input_tokens, 0));
    ModelCost {
        tokens: Tokens {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_read: usage.cache_read_input_tokens,
            cache_write_5m,
            cache_write_1h,
            web_searches: usage.web_search_requests,
        },
        has_cache_write_split: split.is_some(),
        cost_usd: usage.cost_usd,
        thinking_tokens: usage.thinking_tokens,
        web_fetches: usage.web_fetch_requests,
    }
}

#[derive(Debug, Default)]
pub struct Corpus {
    pub files: usize,
    pub requests: HashMap<(String, String), Request>,
    pub cost_states: HashMap<String, CostState>,
    pub unrecognized: BTreeMap<String, usize>,
}

impl Corpus {
    pub fn read(root: &Path) -> io::Result<Corpus> {
        let mut corpus = Corpus::default();
        if root.is_file() {
            corpus.read_file(root)?;
            return Ok(corpus);
        }
        let listed = transcript_files(root)?;
        if let Some((folder, error)) = listed.unreadable_folder {
            return Err(io::Error::new(
                error.kind(),
                format!("{}: {error}", folder.display()),
            ));
        }
        for path in listed.files {
            corpus.read_file(&path)?;
        }
        Ok(corpus)
    }

    pub fn read_file(&mut self, path: &Path) -> io::Result<()> {
        self.read_lines(path)
            .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))
    }

    fn read_lines(&mut self, path: &Path) -> io::Result<()> {
        self.files += 1;
        let reader = BufReader::new(File::open(path)?);
        for line in reader.split(b'\n') {
            let bytes = line?;
            let text = String::from_utf8_lossy(&bytes);
            if let Some(parsed) = parse_line(&text) {
                self.add(parsed);
            }
        }
        Ok(())
    }

    pub fn add(&mut self, line: Line) {
        match line {
            Line::Request(request) => {
                let key = (request.session_id.clone(), request.request_id.clone());
                self.requests.insert(key, *request);
            }
            Line::CostState(state) => {
                let newer = self
                    .cost_states
                    .get(&state.session_id)
                    .is_none_or(|kept| state.total_cost_usd >= kept.total_cost_usd);
                if newer {
                    self.cost_states.insert(state.session_id.clone(), state);
                }
            }
            Line::Title { .. } => {}
            Line::Unrecognized { kind, .. } => *self.unrecognized.entry(kind).or_default() += 1,
        }
    }
}

#[derive(Debug, Default)]
pub struct TranscriptFiles {
    pub files: Vec<PathBuf>,
    pub unreadable_folder: Option<(PathBuf, io::Error)>,
}

pub fn transcript_files(root: &Path) -> io::Result<TranscriptFiles> {
    let mut files = Vec::new();
    let mut unreadable_folder = None;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Err(error) if dir != root => {
                if error.kind() != io::ErrorKind::NotFound {
                    unreadable_folder.get_or_insert((dir, error));
                }
                continue;
            }
            entries => entries?,
        };
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(TranscriptFiles {
        files,
        unreadable_folder,
    })
}
