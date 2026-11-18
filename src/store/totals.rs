use std::collections::{BTreeMap, HashMap};

use super::{Result, Store, reconcile};
use crate::contract::{AgentModel, DataRow, Level};
use crate::sources::transcript::Tokens;

#[derive(Debug, Clone, PartialEq)]
pub struct TopItem {
    pub account: Option<String>,
    pub key: Option<String>,
    pub label: String,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionCount {
    pub account: Option<String>,
    pub key: Option<String>,
    pub sessions: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TokenMix {
    pub account: Option<String>,
    pub key: Option<String>,
    pub not_in_transcript: bool,
    pub model: String,
    pub tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyCost {
    pub account: Option<String>,
    pub day: String,
    pub key: Option<String>,
    pub label: String,
    pub not_in_transcript: bool,
    pub other: bool,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub level: Level,
    pub key: String,
}

const DAILY_TOTALS: &str = "
    SELECT account_id, cwd_label, branch_label, skill_label, agent_label, model_label,
           NULL AS session_id, 0 AS missing, requests, input_tokens, output_tokens,
           cache_read_tokens, cache_write_5m_tokens + cache_write_1h_tokens AS cache_write_tokens,
           web_searches, cost_usd, day
    FROM daily_totals
    WHERE day BETWEEN ?1 AND ?2";

const BEFORE_KEPT_REQUESTS: &str =
    "AND day < (SELECT value FROM store_settings WHERE key = 'requests_kept_from')";

const NOT_IN_TRANSCRIPT: &str = "
    SELECT sessions.account_id, missing.cwd_label, missing.branch_label,
           NULL, NULL, missing.model_label,
           missing.session_id, 1, 0, missing.input_tokens, missing.output_tokens,
           missing.cache_read_tokens, missing.cache_write_5m_tokens + missing.cache_write_1h_tokens,
           missing.web_searches, missing.cost_usd, missing.day
    FROM not_in_transcript AS missing
    JOIN sessions ON sessions.id = missing.session_id
    WHERE missing.day BETWEEN ?1 AND ?2";

const REQUESTS: &str = "
    SELECT account_id, cwd_label, coalesce(head_branch_label, git_branch_label) AS branch_label,
           skill_label, agent_label, model_label, session_id, 0 AS missing, 1 AS requests,
           input_tokens, output_tokens, cache_read_tokens,
           cache_write_5m_tokens + cache_write_1h_tokens AS cache_write_tokens,
           web_searches, cost_usd, day
    FROM requests
    WHERE day BETWEEN ?1 AND ?2";

const DAILY_MIX: &str = "
    SELECT account_id, cwd_label, branch_label, skill_label, agent_label, model_label,
           NULL AS session_id, 0 AS missing, input_tokens, output_tokens, cache_read_tokens,
           cache_write_5m_tokens, cache_write_1h_tokens
    FROM daily_totals
    WHERE day BETWEEN ?1 AND ?2";

const NOT_IN_TRANSCRIPT_MIX: &str = "
    SELECT sessions.account_id, missing.cwd_label, missing.branch_label, NULL, NULL,
           missing.model_label, missing.session_id, 1, missing.input_tokens,
           missing.output_tokens, missing.cache_read_tokens, missing.cache_write_5m_tokens,
           missing.cache_write_1h_tokens
    FROM not_in_transcript AS missing
    JOIN sessions ON sessions.id = missing.session_id
    WHERE missing.day BETWEEN ?1 AND ?2";

const REQUESTS_MIX: &str = "
    SELECT account_id, cwd_label, coalesce(head_branch_label, git_branch_label) AS branch_label,
           skill_label, agent_label, model_label, session_id, 0 AS missing, input_tokens,
           output_tokens,
           cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens
    FROM requests
    WHERE day BETWEEN ?1 AND ?2";

const SPEND_JOINS: &str = "
    LEFT JOIN accounts ON accounts.id = spend.account_id
    LEFT JOIN labels AS cwd ON cwd.id = spend.cwd_label
    LEFT JOIN directories AS dir ON dir.cwd_label = spend.cwd_label
    LEFT JOIN labels AS branch ON branch.id = spend.branch_label
    LEFT JOIN labels AS skill ON skill.id = spend.skill_label
    LEFT JOIN labels AS agent ON agent.id = spend.agent_label
    JOIN labels AS model ON model.id = spend.model_label
    LEFT JOIN sessions AS session ON session.id = spend.session_id";

fn key_sql(level: Level) -> &'static str {
    match level {
        Level::Repo => "coalesce(dir.repo_path, dir.repo_name, cwd.text, '')",
        Level::Branch => "coalesce(branch.text, 'no branch')",
        Level::RepoBranch => {
            "coalesce(dir.repo_path, dir.repo_name, cwd.text, '') || char(31)
             || coalesce(branch.text, 'no branch')"
        }
        Level::Skill => "coalesce(skill.text, 'no skill')",
        Level::Agent => "coalesce(agent.text, 'main conversation')",
        Level::Model => "model.text",
        Level::Session => "session.uuid",
    }
}

fn label_is(column: &str, empty: &str, value: &str) -> String {
    format!(
        "(spend.{column} IN (SELECT id FROM labels WHERE text = {value})
          OR (spend.{column} IS NULL AND {value} = '{empty}'))"
    )
}

fn repo_is(value: &str) -> String {
    format!(
        "(spend.cwd_label IN (
            SELECT cwd.id FROM labels AS cwd
            LEFT JOIN directories AS dir ON dir.cwd_label = cwd.id
            WHERE coalesce(dir.repo_path, dir.repo_name, cwd.text, '') = {value})
          OR (spend.cwd_label IS NULL AND {value} = ''))"
    )
}

fn filter_sql(level: Level, n: usize) -> String {
    let value = format!("?{n}");
    match level {
        Level::Repo => repo_is(&value),
        Level::Branch => label_is("branch_label", "no branch", &value),
        Level::RepoBranch => format!(
            "({} AND {})",
            repo_is(&format!("substr(?{n}, 1, instr(?{n}, char(31)) - 1)")),
            label_is(
                "branch_label",
                "no branch",
                &format!("substr(?{n}, instr(?{n}, char(31)) + 1)")
            )
        ),
        Level::Skill => label_is("skill_label", "no skill", &value),
        Level::Agent => label_is("agent_label", "main conversation", &value),
        Level::Model => format!("spend.model_label IN (SELECT id FROM labels WHERE text = ?{n})"),
        Level::Session => {
            format!("spend.session_id IN (SELECT id FROM sessions WHERE uuid = ?{n})")
        }
    }
}

fn label_sql(level: Level) -> &'static str {
    match level {
        Level::Repo => "coalesce(dir.repo_name, cwd.text, 'unknown folder')",
        Level::RepoBranch => {
            "coalesce(dir.repo_name, cwd.text, 'unknown folder') || ' · '
             || coalesce(branch.text, 'no branch')"
        }
        Level::Agent => "coalesce(agent.text, 'you (no subagent)')",
        Level::Session => {
            "coalesce(session.title, session.uuid, 'older sessions, detail not kept')"
        }
        _ => key_sql(level),
    }
}

fn level_joins(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::Repo => (
            "cwd_label",
            "LEFT JOIN labels AS cwd ON cwd.id = spend.cwd_label
             LEFT JOIN directories AS dir ON dir.cwd_label = spend.cwd_label",
        ),
        Level::Branch => (
            "branch_label",
            "LEFT JOIN labels AS branch ON branch.id = spend.branch_label",
        ),
        Level::RepoBranch => (
            "cwd_label, branch_label",
            "LEFT JOIN labels AS cwd ON cwd.id = spend.cwd_label
             LEFT JOIN directories AS dir ON dir.cwd_label = spend.cwd_label
             LEFT JOIN labels AS branch ON branch.id = spend.branch_label",
        ),
        Level::Skill => (
            "skill_label",
            "LEFT JOIN labels AS skill ON skill.id = spend.skill_label",
        ),
        Level::Agent => (
            "agent_label",
            "LEFT JOIN labels AS agent ON agent.id = spend.agent_label",
        ),
        Level::Model => (
            "model_label",
            "JOIN labels AS model ON model.id = spend.model_label",
        ),
        Level::Session => (
            "session_id",
            "LEFT JOIN sessions AS session ON session.id = spend.session_id",
        ),
    }
}

pub(super) fn day_bounds<'a>(since: Option<&'a str>, until: Option<&'a str>) -> [&'a str; 2] {
    [since.unwrap_or(""), until.unwrap_or("9999-12-31")]
}

fn spend_source(level: Level, filters: &[Filter]) -> String {
    let spend = if level == Level::Session {
        format!("{REQUESTS} UNION ALL {DAILY_TOTALS} {BEFORE_KEPT_REQUESTS}")
    } else {
        DAILY_TOTALS.to_string()
    };
    if only_unlogged_filters(filters) {
        format!("{spend} UNION ALL {NOT_IN_TRANSCRIPT}")
    } else {
        spend
    }
}

fn only_unlogged_filters(filters: &[Filter]) -> bool {
    filters.iter().all(|filter| {
        matches!(
            filter.level,
            Level::Model | Level::Repo | Level::Branch | Level::RepoBranch
        )
    })
}

fn unlogged_row(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::Model | Level::Session => (key_sql(level), label_sql(level)),
        _ => ("NULL", "'unlogged usage'"),
    }
}

fn shown_apart_sql(level: Level) -> &'static str {
    match level {
        Level::Repo | Level::Branch | Level::RepoBranch => {
            "(spend.missing AND spend.cwd_label IS NULL)"
        }
        _ => "spend.missing",
    }
}

pub(super) fn where_sql(filters: &[Filter]) -> String {
    let conditions: Vec<String> = filters
        .iter()
        .enumerate()
        .map(|(n, filter)| filter_sql(filter.level, n + 3))
        .collect();
    if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    }
}

impl Store {
    pub fn keys(
        &self,
        level: Level,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
    ) -> Result<Vec<(Option<String>, String)>> {
        let spend = match level {
            Level::Session => REQUESTS.to_string(),
            Level::Model | Level::Repo | Level::Branch | Level::RepoBranch => {
                format!("{DAILY_TOTALS} UNION ALL {NOT_IN_TRANSCRIPT}")
            }
            _ => DAILY_TOTALS.to_string(),
        };
        let (columns, joins) = level_joins(level);
        let columns: Vec<String> = columns
            .split(", ")
            .map(|column| format!("spend.{column} AS {column}"))
            .collect();
        let sql = format!(
            "SELECT DISTINCT {key}, {label}
             FROM (SELECT DISTINCT {columns} FROM ({spend}) AS spend
                   {where_clause}) AS spend
             {joins}",
            columns = columns.join(", "),
            key = key_sql(level),
            label = label_sql(level),
            where_clause = where_sql(filters),
        );
        let mut values: Vec<&str> = day_bounds(since, until).to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let keys = statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(keys)
    }

    pub fn totals(
        &self,
        level: Level,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
    ) -> Result<Vec<DataRow>> {
        let source = spend_source(level, filters);
        let (missing_key, missing_label) = unlogged_row(level);
        let separate = shown_apart_sql(level);
        let status = if level == Level::Session {
            "session.cost_status"
        } else {
            "NULL"
        };
        let where_clause = where_sql(filters);
        let sql = format!(
            "SELECT accounts.account_uuid || '/' || accounts.organization_uuid,
                    coalesce(accounts.email, accounts.account_uuid || '/' || accounts.organization_uuid, 'No account'),
                    CASE WHEN {separate} THEN {missing_key} ELSE {key} END AS row_key,
                    CASE WHEN {separate} THEN {missing_label} ELSE {label} END,
                    {separate} AS shown_apart, {status},
                    sum(requests), sum(input_tokens), sum(output_tokens), sum(cache_read_tokens),
                    sum(cache_write_tokens), sum(web_searches), sum(cost_usd)
             FROM ({source}) AS spend
             {SPEND_JOINS}
             {where_clause}
             GROUP BY spend.account_id, shown_apart, row_key
             ORDER BY spend.account_id IS NULL, accounts.account_uuid,
                      accounts.organization_uuid, shown_apart, row_key IS NULL,
                      sum(cost_usd) DESC, 4",
            key = key_sql(level),
            label = label_sql(level),
        );
        let mut values: Vec<&str> = day_bounds(since, until).to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            let key: Option<String> = row.get(2)?;
            let status: Option<String> = row.get(5)?;
            Ok(DataRow {
                account: row.get(0)?,
                account_label: row.get(1)?,
                status: (level == Level::Session && key.is_some())
                    .then(|| reconcile::status_from_name(status.as_deref())),
                key,
                label: row.get(3)?,
                not_in_transcript: row.get(4)?,
                requests: row.get::<_, i64>(6)? as u64,
                input_tokens: row.get::<_, i64>(7)? as u64,
                output_tokens: row.get::<_, i64>(8)? as u64,
                cache_read_tokens: row.get::<_, i64>(9)? as u64,
                cache_write_tokens: row.get::<_, i64>(10)? as u64,
                web_searches: row.get::<_, i64>(11)? as u64,
                cost_usd: row.get(12)?,
                paid_for: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn daily_costs(
        &self,
        level: Level,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
        top_n: usize,
    ) -> Result<Vec<DailyCost>> {
        let source = spend_source(level, filters);
        let (missing_key, missing_label) = unlogged_row(level);
        let separate = shown_apart_sql(level);
        let where_clause = where_sql(filters);
        let sql = format!(
            "SELECT accounts.account_uuid || '/' || accounts.organization_uuid,
                    spend.day,
                    CASE WHEN {separate} THEN {missing_key} ELSE {key} END AS row_key,
                    CASE WHEN {separate} THEN {missing_label} ELSE {label} END,
                    {separate} AS shown_apart,
                    coalesce(sum(cost_usd), 0)
             FROM ({source}) AS spend
             {SPEND_JOINS}
             {where_clause}
             GROUP BY spend.account_id, spend.day, shown_apart, row_key
             ORDER BY spend.day",
            key = key_sql(level),
            label = label_sql(level),
        );
        let mut values: Vec<&str> = day_bounds(since, until).to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let days = statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                Ok(DailyCost {
                    account: row.get(0)?,
                    day: row.get(1)?,
                    key: row.get(2)?,
                    label: row.get(3)?,
                    not_in_transcript: row.get(4)?,
                    other: false,
                    cost_usd: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(fold_past_top(days, top_n))
    }

    pub fn token_mix(
        &self,
        level: Level,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
    ) -> Result<Vec<TokenMix>> {
        let spend = if level == Level::Session {
            format!("{REQUESTS_MIX} UNION ALL {DAILY_MIX} {BEFORE_KEPT_REQUESTS}")
        } else {
            DAILY_MIX.to_string()
        };
        let source = if only_unlogged_filters(filters) {
            format!("{spend} UNION ALL {NOT_IN_TRANSCRIPT_MIX}")
        } else {
            spend
        };
        let (missing_key, _) = unlogged_row(level);
        let separate = shown_apart_sql(level);
        let sql = format!(
            "SELECT accounts.account_uuid || '/' || accounts.organization_uuid,
                    CASE WHEN {separate} THEN {missing_key} ELSE {key} END AS row_key,
                    {separate} AS shown_apart, model.text AS model_key,
                    sum(input_tokens), sum(output_tokens), sum(cache_read_tokens),
                    sum(cache_write_5m_tokens), sum(cache_write_1h_tokens)
             FROM ({source}) AS spend
             {SPEND_JOINS}
             {where_clause}
             GROUP BY spend.account_id, shown_apart, row_key, model_key",
            key = key_sql(level),
            where_clause = where_sql(filters),
        );
        let bounds = day_bounds(since, until);
        let mut values: Vec<&str> = bounds.to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok(TokenMix {
                account: row.get(0)?,
                key: row.get(1)?,
                not_in_transcript: row.get(2)?,
                model: row.get(3)?,
                tokens: Tokens {
                    input: row.get::<_, i64>(4)? as u64,
                    output: row.get::<_, i64>(5)? as u64,
                    cache_read: row.get::<_, i64>(6)? as u64,
                    cache_write_5m: row.get::<_, i64>(7)? as u64,
                    cache_write_1h: row.get::<_, i64>(8)? as u64,
                    web_searches: 0,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn session_counts(
        &self,
        level: Level,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
    ) -> Result<Vec<SessionCount>> {
        let (level_columns, joins) = level_joins(level);
        let mut columns = vec!["account_id", "session_id"];
        for column in level_columns.split(", ") {
            if !columns.contains(&column) {
                columns.push(column);
            }
        }
        let columns: Vec<String> = columns
            .iter()
            .map(|column| format!("spend.{column} AS {column}"))
            .collect();
        let sql = format!(
            "SELECT accounts.account_uuid || '/' || accounts.organization_uuid,
                    {key} AS row_key, count(DISTINCT spend.session_id)
             FROM (SELECT DISTINCT {columns} FROM ({REQUESTS}) AS spend
                   {where_clause}) AS spend
             LEFT JOIN accounts ON accounts.id = spend.account_id
             {joins}
             GROUP BY spend.account_id, row_key",
            columns = columns.join(", "),
            key = key_sql(level),
            where_clause = where_sql(filters),
        );
        let bounds = day_bounds(since, until);
        let mut values: Vec<&str> = bounds.to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok(SessionCount {
                account: row.get(0)?,
                key: row.get(1)?,
                sessions: row.get::<_, i64>(2)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn agent_models(
        &self,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
    ) -> Result<Vec<AgentModel>> {
        let level = Level::Agent;
        let separate = shown_apart_sql(level);
        let sql = format!(
            "SELECT accounts.account_uuid || '/' || accounts.organization_uuid,
                    CASE WHEN {separate} THEN NULL ELSE {key} END AS agent_key,
                    CASE WHEN {separate} THEN 'unlogged usage' ELSE {label} END,
                    model.text AS model_key, {separate} AS shown_apart,
                    coalesce(sum(cost_usd), 0)
             FROM ({source}) AS spend
             {SPEND_JOINS}
             {where_clause}
             GROUP BY spend.account_id, shown_apart, agent_key, model_key
             ORDER BY spend.account_id IS NULL, accounts.account_uuid,
                      accounts.organization_uuid, sum(cost_usd) DESC",
            source = spend_source(level, filters),
            key = key_sql(level),
            label = label_sql(level),
            where_clause = where_sql(filters),
        );
        let mut values: Vec<&str> = day_bounds(since, until).to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok(AgentModel {
                account: row.get(0)?,
                agent_key: row.get(1)?,
                agent: row.get(2)?,
                model: row.get(3)?,
                not_in_transcript: row.get(4)?,
                cost_usd: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn top_within(
        &self,
        level: Level,
        within: Level,
        since: Option<&str>,
    ) -> Result<Vec<TopItem>> {
        let sql = format!(
            "SELECT accounts.account_uuid || '/' || accounts.organization_uuid,
                    {key} AS row_key, {within_label}, sum(cost_usd)
             FROM ({DAILY_TOTALS}) AS spend
             {SPEND_JOINS}
             GROUP BY spend.account_id, row_key, {within_key}
             ORDER BY sum(cost_usd) DESC",
            key = key_sql(level),
            within_label = label_sql(within),
            within_key = key_sql(within),
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let pairs = statement.query_map(day_bounds(since, None), |row| {
            Ok(TopItem {
                account: row.get(0)?,
                key: row.get(1)?,
                label: row.get(2)?,
                cost_usd: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
            })
        })?;
        let mut seen = std::collections::HashSet::new();
        let mut top = Vec::new();
        for pair in pairs {
            let pair = pair?;
            if seen.insert((pair.account.clone(), pair.key.clone())) {
                top.push(pair);
            }
        }
        Ok(top)
    }
}

type Series = (Option<String>, bool, Option<String>);

fn fold_past_top(days: Vec<DailyCost>, top_n: usize) -> Vec<DailyCost> {
    let series_of = |day: &DailyCost| -> Series {
        (day.account.clone(), day.not_in_transcript, day.key.clone())
    };
    let mut totals: BTreeMap<Series, f64> = BTreeMap::new();
    for day in &days {
        *totals.entry(series_of(day)).or_default() += day.cost_usd;
    }
    let mut ranked: Vec<(Series, f64)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut kept_per_account: HashMap<Option<String>, usize> = HashMap::new();
    let mut rank: HashMap<Series, usize> = HashMap::new();
    for (position, (series, _)) in ranked.into_iter().enumerate() {
        let kept = kept_per_account.entry(series.0.clone()).or_default();
        if *kept < top_n {
            *kept += 1;
            rank.insert(series, position);
        }
    }
    let mut folded: Vec<DailyCost> = Vec::new();
    let mut other: BTreeMap<(Option<String>, String), f64> = BTreeMap::new();
    for day in days {
        if rank.contains_key(&series_of(&day)) {
            folded.push(day);
        } else {
            *other.entry((day.account, day.day)).or_default() += day.cost_usd;
        }
    }
    folded.extend(
        other
            .into_iter()
            .map(|((account, day), cost_usd)| DailyCost {
                account,
                day,
                key: None,
                label: "other".to_string(),
                not_in_transcript: false,
                other: true,
                cost_usd,
            }),
    );
    folded.sort_by_cached_key(|day| {
        (
            day.account.is_none(),
            day.account.clone(),
            day.day.clone(),
            day.other,
            rank.get(&series_of(day)).copied().unwrap_or(usize::MAX),
        )
    });
    folded
}
