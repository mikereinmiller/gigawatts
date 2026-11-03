use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub session_id: String,
    pub request_id: String,
    pub model: String,
    pub tokens: Tokens,
    pub inference_geo: Option<String>,
    pub speed: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelCost {
    pub tokens: Tokens,
    pub has_cache_write_split: bool,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostState {
    pub session_id: String,
    pub total_cost_usd: f64,
    pub models: BTreeMap<String, ModelCost>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Request(Request),
    CostState(CostState),
    Unrecognized(String),
}

#[derive(Deserialize)]
struct RawLine<'a> {
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "sessionId")]
    session_id: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "requestId")]
    request_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<RawMessage<'a>>,
    #[serde(rename = "totalCostUSD")]
    total_cost_usd: Option<f64>,
    #[serde(borrow, rename = "modelUsage")]
    model_usage: Option<HashMap<Cow<'a, str>, RawModelUsage>>,
}

#[derive(Deserialize)]
struct RawMessage<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    #[serde(borrow)]
    usage: Option<RawUsage<'a>>,
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
    #[serde(rename = "costUSD", default)]
    cost_usd: f64,
}

const SYNTHETIC_MODEL: &str = "<synthetic>";

pub fn is_candidate(line: &str) -> bool {
    line.contains("\"usage\"") || line.contains("\"cost-state\"")
}

pub fn parse_line(line: &str) -> Option<Line> {
    if !is_candidate(line) {
        return None;
    }
    let Ok(raw) = serde_json::from_str::<RawLine>(line) else {
        return Some(Line::Unrecognized("invalid JSON".to_string()));
    };
    let kind = raw.kind.as_deref().unwrap_or("untyped").to_string();
    let parsed = match kind.as_str() {
        "assistant" if is_synthetic(&raw) => return None,
        "assistant" => request_from(raw),
        "cost-state" => cost_state_from(raw),
        _ => None,
    };
    Some(parsed.unwrap_or(Line::Unrecognized(kind)))
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
    let tokens = Tokens {
        input: usage.input_tokens,
        output: usage.output_tokens,
        cache_read: usage.cache_read_input_tokens,
        cache_write_5m: cache_write.0,
        cache_write_1h: cache_write.1,
        web_searches: usage.server_tool_use.map_or(0, |s| s.web_search_requests),
    };
    Some(Line::Request(Request {
        session_id: raw.session_id?.into_owned(),
        request_id: request_id.into_owned(),
        model: model.into_owned(),
        tokens,
        inference_geo: usage.inference_geo.map(Cow::into_owned),
        speed: usage.speed.map(Cow::into_owned),
    }))
}

fn cost_state_from(raw: RawLine) -> Option<Line> {
    let models = raw
        .model_usage?
        .into_iter()
        .map(|(model, usage)| (model.into_owned(), model_cost_from(usage)))
        .collect();
    Some(Line::CostState(CostState {
        session_id: raw.session_id?.into_owned(),
        total_cost_usd: raw.total_cost_usd.unwrap_or(0.0),
        models,
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
        for path in transcript_files(root)? {
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
                self.requests.insert(key, request);
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
            Line::Unrecognized(kind) => *self.unrecognized.entry(kind).or_default() += 1,
        }
    }
}

fn transcript_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}
