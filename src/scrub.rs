use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

const ANCHOR_SUNDAY: &str = "2026-01-04";
const DAY_MS: i64 = 86_400_000;

const LINE_TYPES: [&str; 8] = [
    "assistant",
    "user",
    "system",
    "cost-state",
    "ai-title",
    "session-owner",
    "mode",
    "permission-mode",
];

const KEPT_FIELDS: [&str; 15] = [
    "type",
    "isSidechain",
    "isMeta",
    "userType",
    "entrypoint",
    "version",
    "effort",
    "perTurnEffort",
    "requestedModel",
    "advisorModel",
    "apiBlockIndex",
    "thinkingDurationMs",
    "durationMs",
    "subtype",
    "permissionMode",
];

const UUID_FIELDS: [&str; 10] = [
    "sessionId",
    "session_id",
    "uuid",
    "parentUuid",
    "logicalParentUuid",
    "sourceToolAssistantUUID",
    "promptId",
    "requestRef",
    "accountUuid",
    "organizationUuid",
];

const COST_STATE_FIELDS: [&str; 8] = [
    "totalCostUSD",
    "totalAPIDuration",
    "totalAPIDurationWithoutRetries",
    "totalToolDuration",
    "totalLinesAdded",
    "totalLinesRemoved",
    "totalDuration",
    "hasUnknownModelCost",
];

const MODEL_USAGE_FIELDS: [&str; 10] = [
    "inputTokens",
    "outputTokens",
    "thinkingTokens",
    "cacheReadInputTokens",
    "cacheCreationInputTokens",
    "cacheCreationEphemeral5mInputTokens",
    "cacheCreationEphemeral1hInputTokens",
    "webSearchRequests",
    "webFetchRequests",
    "costUSD",
];

const MESSAGE_FIELDS: [&str; 4] = ["model", "role", "type", "stop_reason"];

const USAGE_FIELDS: [&str; 8] = [
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "service_tier",
    "inference_geo",
    "speed",
    "type",
];

const CACHE_CREATION_FIELDS: [&str; 2] = ["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"];
const SERVER_TOOL_USE_FIELDS: [&str; 2] = ["web_search_requests", "web_fetch_requests"];
const OUTPUT_DETAIL_FIELDS: [&str; 1] = ["thinking_tokens"];

const KEPT_BRANCHES: [&str; 4] = ["main", "master", "HEAD", ""];
const KEPT_PATH_PARTS: [&str; 9] = [
    "",
    "Users",
    "home",
    "tmp",
    "private",
    "var",
    "folders",
    ".claude",
    "worktrees",
];
const BUILT_IN_AGENTS: [&str; 5] = [
    "general-purpose",
    "Explore",
    "Plan",
    "claude-code-guide",
    "statusline-setup",
];
const BUILT_IN_SKILLS: [&str; 11] = [
    "update-config",
    "keybindings-help",
    "simplify",
    "code-review",
    "fewer-permission-prompts",
    "loop",
    "schedule",
    "claude-api",
    "run",
    "init",
    "security-review",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Uuid,
    Request,
    Message,
    Agent,
    PathPart,
    Branch,
    Title,
    Skill,
    AgentName,
}

impl Kind {
    fn name(self, tag: u64, n: usize) -> String {
        match self {
            Kind::Uuid => format!("{n:08x}-0000-4000-8000-{tag:012x}"),
            Kind::Request => format!("req_{tag:012x}{n:06}"),
            Kind::Message => format!("msg_{tag:012x}{n:06}"),
            Kind::Agent => format!("a{tag:012x}{n:04x}"),
            Kind::PathPart => format!("dir-{n}"),
            Kind::Branch => format!("branch-{n}"),
            Kind::Title => format!("Title {n}"),
            Kind::Skill => format!("skill-{n}"),
            Kind::AgentName => format!("agent-{n}"),
        }
    }
}

#[derive(Default)]
pub struct Scrubber {
    tag: u64,
    names: HashMap<(Kind, String), String>,
    counts: HashMap<Kind, usize>,
    shift_days: Option<i64>,
}

impl Scrubber {
    pub fn for_fixture(name: &str) -> Self {
        Scrubber {
            tag: fixture_tag(name),
            ..Scrubber::default()
        }
    }

    pub fn scrub_line(&mut self, line: &str) -> Option<String> {
        let Value::Object(source) = serde_json::from_str::<Value>(line).ok()? else {
            return None;
        };
        let kind = source.get("type")?.as_str()?;
        if !LINE_TYPES.contains(&kind) {
            return None;
        }
        let scrubbed = self.scrub_object(&source);
        Some(Value::Object(scrubbed).to_string())
    }

    fn scrub_object(&mut self, source: &Map<String, Value>) -> Map<String, Value> {
        let mut out = Map::new();
        copy_scalars(source, &mut out, &KEPT_FIELDS);
        copy_scalars(source, &mut out, &COST_STATE_FIELDS);
        if source.get("type").and_then(Value::as_str) == Some("mode") {
            copy_scalars(source, &mut out, &["mode"]);
        }
        for field in UUID_FIELDS {
            self.rename_field(source, &mut out, field, Kind::Uuid);
        }
        self.rename_field(source, &mut out, "requestId", Kind::Request);
        self.rename_field(source, &mut out, "agentId", Kind::Agent);
        self.rename_field(source, &mut out, "aiTitle", Kind::Title);
        self.rename_field(source, &mut out, "gitBranch", Kind::Branch);
        self.rename_field(source, &mut out, "attributionSkill", Kind::Skill);
        self.rename_field(source, &mut out, "attributionAgent", Kind::AgentName);
        if let Some(cwd) = source.get("cwd").and_then(Value::as_str) {
            out.insert("cwd".into(), self.path(cwd).into());
        }
        if let Some(timestamp) = source.get("timestamp").and_then(Value::as_str)
            && let Some(shifted) = self.timestamp(timestamp)
        {
            out.insert("timestamp".into(), shifted.into());
        }
        if let Some(start) = source.get("startTime").and_then(Value::as_i64) {
            let days = self.shift_for(start.div_euclid(DAY_MS));
            out.insert("startTime".into(), (start + days * DAY_MS).into());
        }
        if let Some(Value::Object(usage)) = source.get("modelUsage") {
            out.insert("modelUsage".into(), model_usage(usage).into());
        }
        if let Some(Value::Object(message)) = source.get("message") {
            out.insert("message".into(), self.message(message).into());
        }
        out
    }

    fn message(&mut self, source: &Map<String, Value>) -> Map<String, Value> {
        let mut out = Map::new();
        copy_scalars(source, &mut out, &MESSAGE_FIELDS);
        self.rename_field(source, &mut out, "id", Kind::Message);
        if let Some(Value::Object(usage)) = source.get("usage") {
            out.insert("usage".into(), usage_object(usage).into());
        }
        out
    }

    fn rename_field(
        &mut self,
        source: &Map<String, Value>,
        out: &mut Map<String, Value>,
        field: &str,
        kind: Kind,
    ) {
        match source.get(field) {
            Some(Value::String(original)) => {
                let renamed = self.rename(kind, original);
                out.insert(field.into(), renamed.into());
            }
            Some(Value::Null) => {
                out.insert(field.into(), Value::Null);
            }
            _ => {}
        }
    }

    fn rename(&mut self, kind: Kind, original: &str) -> String {
        if keeps_verbatim(kind, original) {
            return original.to_string();
        }
        let key = (kind, original.to_string());
        if let Some(name) = self.names.get(&key) {
            return name.clone();
        }
        let count = self.counts.entry(kind).or_default();
        *count += 1;
        let name = kind.name(self.tag, *count);
        self.names.insert(key, name.clone());
        name
    }

    fn path(&mut self, path: &str) -> String {
        path.split('/')
            .map(|part| self.rename(Kind::PathPart, part))
            .collect::<Vec<_>>()
            .join("/")
    }

    fn timestamp(&mut self, timestamp: &str) -> Option<String> {
        let date = timestamp.get(..10)?;
        let rest = timestamp.get(10..)?;
        let day = day_number(date)?;
        let days = self.shift_for(day);
        Some(format!("{}{rest}", date_of(day + days)))
    }

    fn shift_for(&mut self, first_day: i64) -> i64 {
        *self.shift_days.get_or_insert_with(|| {
            let anchor = day_number(ANCHOR_SUNDAY).expect("anchor date");
            -(first_day - anchor).div_euclid(7) * 7
        })
    }

    pub fn file_stem(&mut self, stem: &str) -> String {
        match stem.strip_prefix("agent-") {
            Some(agent) => format!("agent-{}", self.rename(Kind::Agent, agent)),
            None => self.rename(Kind::Uuid, stem),
        }
    }
}

fn fixture_tag(name: &str) -> u64 {
    let fnv_1a = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    fnv_1a & 0xffff_ffff_ffff
}

fn keeps_verbatim(kind: Kind, original: &str) -> bool {
    match kind {
        Kind::Branch => KEPT_BRANCHES.contains(&original),
        Kind::PathPart => KEPT_PATH_PARTS.contains(&original),
        Kind::Skill => BUILT_IN_SKILLS.contains(&original),
        Kind::AgentName => BUILT_IN_AGENTS.contains(&original),
        _ => false,
    }
}

fn copy_scalars(source: &Map<String, Value>, out: &mut Map<String, Value>, fields: &[&str]) {
    for field in fields {
        if let Some(value @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))) =
            source.get(*field)
        {
            out.insert((*field).into(), value.clone());
        }
    }
}

fn usage_object(source: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    copy_scalars(source, &mut out, &USAGE_FIELDS);
    copy_nested(source, &mut out, "cache_creation", &CACHE_CREATION_FIELDS);
    copy_nested(source, &mut out, "server_tool_use", &SERVER_TOOL_USE_FIELDS);
    copy_nested(
        source,
        &mut out,
        "output_tokens_details",
        &OUTPUT_DETAIL_FIELDS,
    );
    if let Some(Value::Array(iterations)) = source.get("iterations") {
        let iterations: Vec<Value> = iterations
            .iter()
            .filter_map(Value::as_object)
            .map(|iteration| usage_object(iteration).into())
            .collect();
        out.insert("iterations".into(), iterations.into());
    }
    out
}

fn copy_nested(
    source: &Map<String, Value>,
    out: &mut Map<String, Value>,
    field: &str,
    fields: &[&str],
) {
    if let Some(Value::Object(nested)) = source.get(field) {
        let mut copy = Map::new();
        copy_scalars(nested, &mut copy, fields);
        out.insert(field.into(), copy.into());
    }
}

fn model_usage(source: &Map<String, Value>) -> Map<String, Value> {
    source
        .iter()
        .filter_map(|(model, usage)| {
            let mut copy = Map::new();
            copy_scalars(usage.as_object()?, &mut copy, &MODEL_USAGE_FIELDS);
            Some((model.clone(), copy.into()))
        })
        .collect()
}

fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn date_of(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn scrub_sessions(transcripts: &[PathBuf], out: &Path) -> io::Result<()> {
    let fixture = out
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut scrubber = Scrubber::for_fixture(fixture);
    for transcript in transcripts {
        scrub_session(&mut scrubber, transcript, out)?;
    }
    Ok(())
}

fn scrub_session(scrubber: &mut Scrubber, transcript: &Path, out: &Path) -> io::Result<()> {
    let stem = file_stem(transcript)?;
    let renamed_session = scrubber.file_stem(&stem);
    fs::create_dir_all(out)?;
    scrub_file(
        scrubber,
        transcript,
        &out.join(format!("{renamed_session}.jsonl")),
    )?;
    let subagents = transcript.with_file_name(&stem).join("subagents");
    if !subagents.is_dir() {
        return Ok(());
    }
    let out_subagents = out.join(&renamed_session).join("subagents");
    fs::create_dir_all(&out_subagents)?;
    let mut agent_files: Vec<_> = fs::read_dir(&subagents)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    agent_files.retain(|path| path.extension().is_some_and(|ext| ext == "jsonl"));
    agent_files.sort();
    for agent_file in agent_files {
        let renamed = scrubber.file_stem(&file_stem(&agent_file)?);
        scrub_file(
            scrubber,
            &agent_file,
            &out_subagents.join(format!("{renamed}.jsonl")),
        )?;
    }
    Ok(())
}

fn file_stem(path: &Path) -> io::Result<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .ok_or_else(|| io::Error::other(format!("{}: not a transcript file name", path.display())))
}

fn scrub_file(scrubber: &mut Scrubber, source: &Path, out: &Path) -> io::Result<()> {
    let reader = BufReader::new(fs::File::open(source)?);
    let created = fs::File::create_new(out)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", out.display())))?;
    let mut writer = io::BufWriter::new(created);
    for line in reader.split(b'\n') {
        let bytes = line?;
        if let Some(scrubbed) = scrubber.scrub_line(&String::from_utf8_lossy(&bytes)) {
            writeln!(writer, "{scrubbed}")?;
        }
    }
    writer.flush()
}
