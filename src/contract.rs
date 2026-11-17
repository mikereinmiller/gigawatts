use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

/// A preset history range in local days: `today`, `week` (Monday to today), `month` (the 1st to today), `all`, or `custom` when `--since` or `--until` was given.
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

/// What history rows group by: `repo`, `branch`, `skill`, `agent` (subagent), or `model`.
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

/// What each row in a data response is: `repo`, `branch`, `repo_branch`, `skill`, `agent`, `model`, or `session`.
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

/// How a cost was arrived at: `estimated` (priced from transcript tokens, not yet checked against Claude Code's record), `exact` (matches Claude Code's record), or `adjusted` (scaled to Claude Code's record because a price looks wrong).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CostStatus {
    Estimated,
    Exact,
    Adjusted,
}

/// One step of a `--drill` path, from the outermost grouping inward.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DrillStep {
    /// What this step narrowed to.
    pub level: Level,
    /// The stable key that identifies the row, as passed to `--drill`.
    pub key: String,
    /// The row's display name.
    pub label: String,
}

/// A history query's answer: `gigawatts data --json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DataResponse {
    /// Raised only when a field is removed, renamed, or changes meaning. New fields keep it.
    pub schema_version: u32,
    /// The range asked for.
    pub range: Range,
    /// The grouping asked for.
    pub group_by: GroupBy,
    /// The drill path asked for, outermost first.
    pub drill: Vec<DrillStep>,
    /// What each row is.
    pub level: Level,
    /// Today's local date, YYYY-MM-DD.
    pub today: String,
    /// The first local day included, YYYY-MM-DD, or null for the start of history.
    pub since: Option<String>,
    /// The last local day included, YYYY-MM-DD, or null for today.
    #[serde(default)]
    pub until: Option<String>,
    /// One row per account and group, most expensive first.
    pub rows: Vec<DataRow>,
    /// Current problems that affect these numbers.
    pub problems: Vec<Problem>,
}

/// Usage and cost for one group in one account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DataRow {
    /// The account ID, `<accountUuid>/<organizationUuid>`, or null for requests made before any login was seen.
    pub account: Option<String>,
    /// The account's display name.
    pub account_label: String,
    /// The stable key for `--drill`, or null for a row that can't be drilled into.
    pub key: Option<String>,
    /// The group's display name.
    pub label: String,
    /// True for spend Claude Code billed but never wrote to a transcript; it has a cost and no tokens.
    pub not_in_transcript: bool,
    /// How a session row's cost was arrived at; null on every other level.
    pub status: Option<CostStatus>,
    /// Model requests.
    pub requests: u64,
    /// Uncached input tokens.
    pub input_tokens: u64,
    /// Output tokens, thinking included.
    pub output_tokens: u64,
    /// Input tokens read from the prompt cache.
    pub cache_read_tokens: u64,
    /// Input tokens written to the prompt cache, 5-minute and 1-hour together.
    pub cache_write_tokens: u64,
    /// Web searches the model ran.
    pub web_searches: u64,
    /// Cost in US dollars, or null when a model in the row has no price.
    pub cost_usd: Option<f64>,
}

/// What went wrong: `unrecognized_lines`, `no_price`, `price_gap`, `watch_failing`, `read_failing`, `cleanup_failing`, `refresh_failing`, `report_unavailable`, or `price_update_failing`.
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
    PriceUpdateFailing,
}

/// Something that makes a number wrong, stale, or missing, with how to fix it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    /// What went wrong.
    pub kind: ProblemKind,
    /// Which numbers it affects, in words.
    pub affects: String,
    /// When it started, as local `YYYY-MM-DD HH:MM:SS`, or null when unknown.
    pub since: Option<String>,
    /// Why it happens, in words.
    pub cause: String,
    /// What to do about it, in words.
    pub fix: String,
    /// The account it belongs to, or absent when it affects every account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// Everything happening now, per tracked account: `gigawatts status --json`, or the engine's `snapshot.json`. The file is rewritten only after something changes, so prefer the command, which builds it fresh.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Snapshot {
    /// Raised only when a field is removed, renamed, or changes meaning. New fields keep it.
    pub schema_version: u32,
    /// The Gigawatts version that built this snapshot.
    pub engine_version: String,
    /// When this snapshot was built, Unix milliseconds. Every alert and pace in it is as of this time.
    pub written_at_ms: i64,
    /// The local date this snapshot counts as today, YYYY-MM-DD. If it's earlier than the reader's date, nothing has been spent yet today.
    pub today: String,
    /// Each tracked account's spend today, running sessions, and limits.
    pub accounts: Vec<AccountToday>,
    /// Current problems, across every account.
    pub problems: Vec<Problem>,
    /// Limits that are full, or forecast to fill before they reset and within the next 24 hours, worst first: full limits by the latest reset, then filling limits by the soonest. Empty when nothing needs attention.
    pub alerts: Vec<Alert>,
}

/// A limit that needs attention: full now, or forecast to fill before it resets and within the next 24 hours.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Alert {
    /// The account ID, `<accountUuid>/<organizationUuid>`, or null for requests made before any login was seen.
    pub account: Option<String>,
    /// The account's display name.
    pub account_label: String,
    /// A short name for the limit: `Session`, `Weekly`, a weekly limit's scope such as `Fable`, `Credits`, or Anthropic's own name for a kind Gigawatts doesn't know.
    pub limit: String,
    /// Whether the limit is already full or still filling.
    pub state: AlertState,
    /// For `full`, when the limit resets; for `filling`, when it's forecast to reach 100%, or `written_at_ms` when that forecast has already passed, meaning any minute now. Unix milliseconds. Count down from the reader's clock.
    pub at_ms: i64,
}

/// `full`: the limit is at 100% and comes back at its reset. `filling`: the limit is forecast to reach 100% before it resets, within the next 24 hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Full,
    Filling,
}

/// One tracked account's day so far.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AccountToday {
    /// The account ID, `<accountUuid>/<organizationUuid>`, or null for requests made before any login was seen.
    pub account: Option<String>,
    /// The account's display name: its configured label, or else its email.
    pub account_label: String,
    /// The label set in `config.toml`, or null.
    #[serde(default)]
    pub configured_label: Option<String>,
    /// Model requests today.
    pub requests: u64,
    /// Spend today in US dollars, including spend Claude Code billed but never wrote to a transcript.
    pub cost_usd: f64,
    /// Today's spend against the usual day at this time.
    pub pace: DollarPace,
    /// The running session spending fastest over the last 15 minutes, or null when none is running.
    pub fastest_session: Option<SessionSpend>,
    /// Sessions that wrote to their transcript in the last 15 minutes, fastest spending (`last_15_minutes_usd`) first.
    pub active_sessions: Vec<SessionSpend>,
    /// Models used today that have no price, so their cost is missing until their session ends.
    pub unpriced_models: Vec<String>,
    /// Models whose cost today was scaled to Claude Code's record because their price looks wrong.
    pub adjusted_models: Vec<String>,
    /// True when some of today's cost hasn't been checked against Claude Code's record yet.
    pub cost_estimated: bool,
    /// The latest limit reading, or null when none has been read.
    pub limits: Option<AccountLimits>,
}

/// An account's latest limit reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountLimits {
    /// Where the reading came from.
    pub source: LimitSource,
    /// When Anthropic reported it, Unix milliseconds. A reading over an hour old may not include use outside Claude Code, and raises no filling alerts.
    pub fetched_at_ms: i64,
    /// Each limit Anthropic reported, with pace and forecast.
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

/// Each account's latest limits: `gigawatts data --limits --json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitsResponse {
    /// Raised only when a field is removed, renamed, or changes meaning. New fields keep it.
    pub schema_version: u32,
    /// One entry per account with a reading.
    pub accounts: Vec<LatestLimits>,
}

/// One account's latest limit reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LatestLimits {
    /// The account ID, `<accountUuid>/<organizationUuid>`.
    pub account: String,
    /// The account's display name.
    pub account_label: String,
    /// Where the reading came from.
    pub source: LimitSource,
    /// When Anthropic reported it, Unix milliseconds.
    pub fetched_at_ms: i64,
    /// Each limit Anthropic reported.
    pub limits: Vec<Limit>,
}

/// A limit with its pace and forecast. The limit's own fields sit at the top level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PacedLimit {
    #[serde(flatten)]
    pub limit: Limit,
    /// Used against expected by now, or null when the limit has no window to pace against.
    pub pace: Option<LimitPace>,
    /// When the limit reaches 100% before it resets, or null when it isn't on course to. The average rate since the window started forecasts only once use runs more than 5 points ahead of the share of the window gone, or is within 5 points of full.
    #[serde(default)]
    pub forecast: Option<LimitForecast>,
    /// For the credit limit of an account with no session or weekly limits, this machine's spend in US dollars from the start of the credit period until the reading was fetched. Null for every other limit, for credits in another currency, when this machine no longer keeps request detail back to the period's start, while some of this machine's spend in the period has no price, and when the reading is from before the current credit period.
    #[serde(default)]
    pub this_machine_usd: Option<f64>,
    /// The credits used minus `this_machine_usd`: what other machines and claude.ai used this credit period, in US dollars. Null when `this_machine_usd` is, or when the difference is under $5 or under 1% of the credits used.
    #[serde(default)]
    pub elsewhere_usd: Option<f64>,
    /// For the session and weekly limits of an account that has them, an estimate of how many points of `percent` other machines and claude.ai used this window. This machine's spend in the window is converted to points at its own dollars per point, the highest daily rate over the last 28 days among local days whose rises within a window add up to 5 points or more, and subtracted from `percent`. Expect it to be off by 2 to 3 points. Null for every other limit, when no day qualifies, when `percent` is under 10 or the estimate is under 3 points, and while some of this machine's spend in the window has no price.
    #[serde(default)]
    pub elsewhere_percent: Option<f64>,
}

/// When a limit is on course to reach 100% before its reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitForecast {
    /// When it's forecast to reach 100%, Unix milliseconds.
    pub full_at_ms: i64,
    /// Which rate the forecast used.
    #[serde(default)]
    pub basis: ForecastBasis,
}

/// `window`: the average rate since the window started. `recent`: the last 30 minutes' rate, used for the 5-hour session limit when it fills sooner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForecastBasis {
    #[default]
    Window,
    Recent,
}

/// Today's spend against the usual day by the same clock time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DollarPace {
    /// Spend today by `clock`, US dollars.
    pub today_usd: f64,
    /// The median spend by `clock` over the earlier days, or null with no earlier days.
    pub usual_usd: Option<f64>,
    /// How many earlier days the usual comes from, up to 20.
    pub usual_days: u32,
    /// The local time of day the comparison is made at, HH:MM.
    pub clock: String,
    /// Each earlier day's spend by `clock`, oldest first.
    #[serde(default)]
    pub earlier_days_usd: Vec<f64>,
    /// Each earlier day's whole-day spend, oldest first.
    #[serde(default)]
    pub earlier_day_totals_usd: Vec<f64>,
    /// The earlier days' local dates, YYYY-MM-DD, oldest first. Only days of the same kind as today (workdays or weekend days) with any usage count.
    #[serde(default)]
    pub earlier_days: Vec<String>,
    /// True when today is a Saturday or Sunday, so the usual comes from weekend days.
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

/// One usage limit as Anthropic reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Limit {
    /// Which limit: `session` (5-hour), `weekly_all`, `weekly_scoped`, `credits`, or another kind Anthropic adds.
    pub kind: String,
    /// Anthropic's grouping for the limit, or null.
    pub group: Option<String>,
    /// What a scoped limit covers, such as a model name, or null.
    pub scope: Option<String>,
    /// How much is used, 0 to 100.
    pub percent: f64,
    /// Anthropic's severity, such as `normal`, or null.
    pub severity: Option<String>,
    /// When the window resets, an ISO 8601 time, or null when Anthropic gives none.
    pub resets_at: Option<String>,
    /// True when Anthropic marks this the limit currently in effect.
    pub active: bool,
    /// Dollar amounts for a credit limit, or null.
    pub credits: Option<Credits>,
}

/// Money used against a credit limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Credits {
    /// Amount used this period, in `currency`.
    pub used: f64,
    /// The period's cap, or null when there is none.
    pub limit: Option<f64>,
    /// ISO 4217 currency code, such as `USD`.
    pub currency: String,
}

/// Where a limit reading came from: `get_usage` (fresh, asked of Claude Code) or `cached_usage_utilization` (Claude Code's own cached copy, possibly days old).
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

/// `under`, `even`, or `over`: used compared with expected by now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaceState {
    Under,
    Even,
    Over,
}

/// What expected means: `even` (a straight line across the window) or `usual` (your own usual progress at this point in the week).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaceBasis {
    Even,
    Usual,
}

/// A limit's use against what's expected by now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitPace {
    /// Under, at, or over expected.
    pub state: PaceState,
    /// What expected is based on.
    pub basis: PaceBasis,
    /// The percent expected by now, 0 to 100.
    pub expected_percent: f64,
    /// When the limit's window started, Unix milliseconds.
    pub window_start_ms: i64,
    /// When the limit's window ends and it resets, Unix milliseconds.
    pub window_end_ms: i64,
    /// True while too little history exists for the usual to be reliable.
    pub provisional: bool,
}

/// A Claude Code session that wrote to its transcript in the last 15 minutes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionSpend {
    /// The session ID.
    pub session: String,
    /// The session's title, or its ID when it has none.
    pub label: String,
    /// The repository name, or the folder when it isn't a repository.
    pub repo: String,
    /// The git branch, or null.
    pub branch: Option<String>,
    /// Spend in the last 15 minutes, US dollars. Times 4 is the hourly rate.
    pub last_15_minutes_usd: f64,
    /// The session's spend since it started, US dollars.
    pub total_usd: f64,
    /// When it last wrote to its transcript, Unix milliseconds.
    #[serde(default)]
    pub last_active_ms: i64,
    /// The session's spend today, US dollars.
    #[serde(default)]
    pub today_usd: f64,
    /// The skill or subagent that spent most of today's spend, with its share, or null.
    #[serde(default)]
    pub top_driver: Option<Share>,
    /// Today's spend split by model.
    #[serde(default)]
    pub models: Vec<Share>,
}

/// A label and its share of a total.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Share {
    /// What the share belongs to.
    pub label: String,
    /// The fraction of the total, 0 to 1.
    pub share: f64,
}
