use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Range {
    Today,
    Week,
    Month,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    Repo,
    Branch,
    Skill,
    Agent,
    Model,
}

impl GroupBy {
    pub fn levels(self) -> &'static [Level] {
        match self {
            GroupBy::Repo => &[Level::Repo, Level::Branch, Level::Session],
            GroupBy::Branch => &[Level::Branch, Level::Session],
            GroupBy::Skill => &[Level::Skill, Level::Session],
            GroupBy::Agent => &[Level::Agent, Level::Session],
            GroupBy::Model => &[Level::Model, Level::Session],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Repo,
    Branch,
    Skill,
    Agent,
    Model,
    Session,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Repo => "repo",
            Level::Branch => "branch",
            Level::Skill => "skill",
            Level::Agent => "agent",
            Level::Model => "model",
            Level::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CostStatus {
    Estimated,
    Exact,
    Adjusted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DrillStep {
    pub level: Level,
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DataResponse {
    pub schema_version: u32,
    pub range: Range,
    pub group_by: GroupBy,
    pub drill: Vec<DrillStep>,
    pub level: Level,
    pub today: String,
    pub since: Option<String>,
    pub rows: Vec<DataRow>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DataRow {
    pub account: Option<String>,
    pub account_label: String,
    pub key: Option<String>,
    pub label: String,
    pub not_in_transcript: bool,
    pub status: Option<CostStatus>,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub web_searches: u64,
    pub cost_usd: Option<f64>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    UnrecognizedLines,
    NoPrice,
    PriceGap,
    WatchFailing,
    ReadFailing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    pub kind: ProblemKind,
    pub affects: String,
    pub since: Option<String>,
    pub cause: String,
    pub fix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Snapshot {
    pub schema_version: u32,
    pub engine_version: String,
    pub written_at_ms: i64,
    pub today: String,
    pub accounts: Vec<AccountToday>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AccountToday {
    pub account: Option<String>,
    pub account_label: String,
    pub requests: u64,
    pub cost_usd: f64,
}
