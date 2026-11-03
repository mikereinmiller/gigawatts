use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Range {
    Today,
    Week,
    Month,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    Model,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataResponse {
    pub schema_version: u32,
    pub range: Range,
    pub group_by: GroupBy,
    pub today: String,
    pub since: Option<String>,
    pub rows: Vec<DataRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataRow {
    pub account: Option<String>,
    pub account_label: String,
    pub key: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub web_searches: u64,
    pub cost_usd: Option<f64>,
}
