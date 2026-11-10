use super::{Result, Store, reconcile};
use crate::contract::{DataRow, Level};

#[derive(Debug, Clone, PartialEq)]
pub struct TopItem {
    pub account: Option<String>,
    pub key: Option<String>,
    pub label: String,
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
           web_searches, cost_usd
    FROM daily_totals
    WHERE day >= coalesce(?1, '')";

const BEFORE_KEPT_REQUESTS: &str =
    "AND day < (SELECT value FROM store_settings WHERE key = 'requests_kept_from')";

const NOT_IN_TRANSCRIPT: &str = "
    SELECT sessions.account_id, missing.cwd_label, missing.branch_label,
           NULL, NULL, missing.model_label,
           missing.session_id, 1, 0, missing.input_tokens, missing.output_tokens,
           missing.cache_read_tokens, missing.cache_write_5m_tokens + missing.cache_write_1h_tokens,
           missing.web_searches, missing.cost_usd
    FROM not_in_transcript AS missing
    JOIN sessions ON sessions.id = missing.session_id
    WHERE missing.day >= coalesce(?1, '')";

const REQUESTS: &str = "
    SELECT account_id, cwd_label, coalesce(head_branch_label, git_branch_label) AS branch_label,
           skill_label, agent_label, model_label, session_id, 0 AS missing, 1 AS requests,
           input_tokens, output_tokens, cache_read_tokens,
           cache_write_5m_tokens + cache_write_1h_tokens AS cache_write_tokens,
           web_searches, cost_usd
    FROM requests
    WHERE day >= coalesce(?1, '')";

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

fn where_sql(filters: &[Filter]) -> String {
    let conditions: Vec<String> = filters
        .iter()
        .enumerate()
        .map(|(n, filter)| filter_sql(filter.level, n + 2))
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
        let mut values: Vec<Option<&str>> = vec![since];
        values.extend(filters.iter().map(|filter| Some(filter.key.as_str())));
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
        filters: &[Filter],
    ) -> Result<Vec<DataRow>> {
        let spend = if level == Level::Session {
            format!("{REQUESTS} UNION ALL {DAILY_TOTALS} {BEFORE_KEPT_REQUESTS}")
        } else {
            DAILY_TOTALS.to_string()
        };
        let only_models = filters.iter().all(|filter| {
            matches!(
                filter.level,
                Level::Model | Level::Repo | Level::Branch | Level::RepoBranch
            )
        });
        let source = if only_models {
            format!("{spend} UNION ALL {NOT_IN_TRANSCRIPT}")
        } else {
            spend
        };
        let (missing_key, missing_label) = match level {
            Level::Model | Level::Session => (key_sql(level), label_sql(level)),
            _ => ("NULL", "'unlogged usage'"),
        };
        let separate = match level {
            Level::Repo | Level::Branch | Level::RepoBranch => {
                "(spend.missing AND spend.cwd_label IS NULL)"
            }
            _ => "spend.missing",
        };
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
        let mut values: Vec<Option<&str>> = vec![since];
        values.extend(filters.iter().map(|filter| Some(filter.key.as_str())));
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
        let pairs = statement.query_map([since], |row| {
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
