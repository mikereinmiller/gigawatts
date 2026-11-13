use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Range {
    Today,
    Week,
    Month,
    All,
    #[value(skip)]
    Custom,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
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
            GroupBy::Branch => &[Level::RepoBranch, Level::Session],
            GroupBy::Skill => &[Level::Skill, Level::Session],
            GroupBy::Agent => &[Level::Agent, Level::Session],
            GroupBy::Model => &[Level::Model, Level::Session],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Repo,
    Branch,
    #[serde(rename = "repo_branch")]
    RepoBranch,
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
            Level::RepoBranch => "repo and branch",
            Level::Skill => "skill",
            Level::Agent => "agent",
            Level::Model => "model",
            Level::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CostStatus {
    Estimated,
    Exact,
    Adjusted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DrillStep {
    pub level: Level,
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DataResponse {
    pub schema_version: u32,
    pub range: Range,
    pub group_by: GroupBy,
    pub drill: Vec<DrillStep>,
    pub level: Level,
    pub today: String,
    pub since: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    pub rows: Vec<DataRow>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    CleanupFailing,
    RefreshFailing,
    ReportUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    pub kind: ProblemKind,
    pub affects: String,
    pub since: Option<String>,
    pub cause: String,
    pub fix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
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
    #[serde(default)]
    pub configured_label: Option<String>,
    pub requests: u64,
    pub cost_usd: f64,
    pub pace: DollarPace,
    pub fastest_session: Option<SessionSpend>,
    pub active_sessions: Vec<SessionSpend>,
    pub unpriced_models: Vec<String>,
    pub adjusted_models: Vec<String>,
    pub cost_estimated: bool,
    pub limits: Option<AccountLimits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountLimits {
    pub source: LimitSource,
    pub fetched_at_ms: i64,
    pub limits: Vec<PacedLimit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SeriesResponse {
    pub schema_version: u32,
    pub range: Range,
    pub today: String,
    pub since: Option<String>,
    pub until: Option<String>,
    pub level: Level,
    pub days: Vec<DayCost>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionsResponse {
    pub schema_version: u32,
    pub range: Range,
    pub today: String,
    pub since: Option<String>,
    pub until: Option<String>,
    pub drill: Vec<DrillStep>,
    pub kept_from: Option<String>,
    pub total: u64,
    #[serde(default)]
    pub per_account: Vec<AccountSessions>,
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountSessions {
    pub account: Option<String>,
    pub sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionSummary {
    pub account: Option<String>,
    pub session: String,
    pub title: Option<String>,
    pub repo: String,
    pub branch: Option<String>,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub active_minutes: u64,
    pub requests: u64,
    pub cost_usd: f64,
    pub cost_status: CostStatus,
    pub peak_context: u64,
    pub skills: Vec<Share>,
    pub models: Vec<Share>,
    pub agents: Vec<Share>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MixResponse {
    pub schema_version: u32,
    pub range: Range,
    pub today: String,
    pub since: Option<String>,
    pub until: Option<String>,
    pub level: Level,
    pub rows: Vec<CostMix>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostMix {
    pub account: Option<String>,
    pub key: Option<String>,
    pub not_in_transcript: bool,
    pub sessions: u64,
    #[serde(default)]
    pub session_cost_usd: Option<f64>,
    pub input_usd: f64,
    pub output_usd: f64,
    pub cache_write_usd: f64,
    pub cache_read_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdviceResponse {
    pub schema_version: u32,
    pub today: String,
    pub accounts: Vec<AccountAdvice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountAdvice {
    pub account: String,
    pub fetched_at_ms: i64,
    pub behaviors: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitHistoryResponse {
    pub schema_version: u32,
    pub range: Range,
    pub today: String,
    pub since: Option<String>,
    pub until: Option<String>,
    pub accounts: Vec<AccountLimitHistory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountLimitHistory {
    pub account: String,
    pub limits: Vec<LimitHistory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitHistory {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub points: Vec<LimitHistoryPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitHistoryPoint {
    pub at_ms: i64,
    pub percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentModelsResponse {
    pub schema_version: u32,
    pub range: Range,
    pub today: String,
    pub since: Option<String>,
    pub until: Option<String>,
    pub rows: Vec<AgentModel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentModel {
    pub account: Option<String>,
    pub agent_key: Option<String>,
    pub agent: String,
    pub model: String,
    pub not_in_transcript: bool,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DayCost {
    pub account: Option<String>,
    pub day: String,
    pub key: Option<String>,
    pub label: String,
    pub not_in_transcript: bool,
    pub other: bool,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitsResponse {
    pub schema_version: u32,
    pub accounts: Vec<LatestLimits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LatestLimits {
    pub account: String,
    pub account_label: String,
    pub source: LimitSource,
    pub fetched_at_ms: i64,
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PacedLimit {
    #[serde(flatten)]
    pub limit: Limit,
    pub pace: Option<LimitPace>,
    #[serde(default)]
    pub forecast: Option<LimitForecast>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitForecast {
    pub full_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DollarPace {
    pub today_usd: f64,
    pub usual_usd: Option<f64>,
    pub usual_days: u32,
    pub clock: String,
    #[serde(default)]
    pub earlier_days_usd: Vec<f64>,
    #[serde(default)]
    pub earlier_day_totals_usd: Vec<f64>,
    #[serde(default)]
    pub earlier_days: Vec<String>,
    #[serde(default)]
    pub weekend: bool,
}

impl DollarPace {
    pub fn times_usual(&self) -> Option<f64> {
        self.usual_usd
            .filter(|usual| *usual > 0.0)
            .map(|usual| self.today_usd / usual)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Limit {
    pub kind: String,
    pub group: Option<String>,
    pub scope: Option<String>,
    pub percent: f64,
    pub severity: Option<String>,
    pub resets_at: Option<String>,
    pub active: bool,
    pub credits: Option<Credits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Credits {
    pub used: f64,
    pub limit: Option<f64>,
    pub currency: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitSource {
    GetUsage,
    CachedUsageUtilization,
}

impl LimitSource {
    pub fn name(self) -> &'static str {
        match self {
            LimitSource::GetUsage => "get_usage",
            LimitSource::CachedUsageUtilization => "cached_usage_utilization",
        }
    }

    pub fn from_name(name: &str) -> Option<LimitSource> {
        [LimitSource::GetUsage, LimitSource::CachedUsageUtilization]
            .into_iter()
            .find(|source| source.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaceState {
    Under,
    Even,
    Over,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaceBasis {
    Even,
    Usual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitPace {
    pub state: PaceState,
    pub basis: PaceBasis,
    pub expected_percent: f64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionSpend {
    pub session: String,
    pub label: String,
    pub repo: String,
    pub branch: Option<String>,
    pub last_15_minutes_usd: f64,
    pub total_usd: f64,
    #[serde(default)]
    pub last_active_ms: i64,
    #[serde(default)]
    pub today_usd: f64,
    #[serde(default)]
    pub top_driver: Option<Share>,
    #[serde(default)]
    pub models: Vec<Share>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Share {
    pub label: String,
    pub share: f64,
}
