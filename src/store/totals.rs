use super::{Result, Store, reconcile};
use crate::contract::{DataRow, Level};

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

const NOT_IN_TRANSCRIPT: &str = "
    SELECT sessions.account_id, NULL, NULL, NULL, NULL, missing.model_label,
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

fn key_sql(level: Level) -> &'static str {
    match level {
        Level::Repo => "coalesce(dir.repo_path, dir.repo_name, cwd.text, '')",
        Level::Branch => "coalesce(branch.text, 'no branch')",
        Level::Skill => "coalesce(skill.text, 'no skill')",
        Level::Agent => "coalesce(agent.text, 'main conversation')",
        Level::Model => "model.text",
        Level::Session => "session.uuid",
    }
}

fn filter_sql(level: Level, n: usize) -> String {
    let label_is = |column: &str, empty: &str| {
        format!(
            "(spend.{column} IN (SELECT id FROM labels WHERE text = ?{n})
              OR (spend.{column} IS NULL AND ?{n} = '{empty}'))"
        )
    };
    match level {
        Level::Repo => format!(
            "(spend.cwd_label IN (
                SELECT cwd.id FROM labels AS cwd
                LEFT JOIN directories AS dir ON dir.cwd_label = cwd.id
                WHERE coalesce(dir.repo_path, dir.repo_name, cwd.text, '') = ?{n})
              OR (spend.cwd_label IS NULL AND ?{n} = ''))"
        ),
        Level::Branch => label_is("branch_label", "no branch"),
        Level::Skill => label_is("skill_label", "no skill"),
        Level::Agent => label_is("agent_label", "main conversation"),
        Level::Model => format!("spend.model_label IN (SELECT id FROM labels WHERE text = ?{n})"),
        Level::Session => {
            format!("spend.session_id IN (SELECT id FROM sessions WHERE uuid = ?{n})")
        }
    }
}

fn label_sql(level: Level) -> &'static str {
    match level {
        Level::Repo => "coalesce(dir.repo_name, cwd.text, 'unknown folder')",
        Level::Session => "coalesce(session.title, session.uuid)",
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
            Level::Model => format!("{DAILY_TOTALS} UNION ALL {NOT_IN_TRANSCRIPT}"),
            _ => DAILY_TOTALS.to_string(),
        };
        let (column, joins) = level_joins(level);
        let sql = format!(
            "SELECT DISTINCT {key}, {label}
             FROM (SELECT DISTINCT spend.{column} AS {column} FROM ({spend}) AS spend
                   {where_clause}) AS spend
             {joins}",
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
            REQUESTS
        } else {
            DAILY_TOTALS
        };
        let only_models = filters.iter().all(|filter| filter.level == Level::Model);
        let source = if only_models {
            format!("{spend} UNION ALL {NOT_IN_TRANSCRIPT}")
        } else {
            spend.to_string()
        };
        let (missing_key, missing_label) = match level {
            Level::Model | Level::Session => (key_sql(level), label_sql(level)),
            _ => ("NULL", "'not in transcript'"),
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
                    CASE WHEN spend.missing THEN {missing_key} ELSE {key} END AS row_key,
                    CASE WHEN spend.missing THEN {missing_label} ELSE {label} END,
                    spend.missing, {status},
                    sum(requests), sum(input_tokens), sum(output_tokens), sum(cache_read_tokens),
                    sum(cache_write_tokens), sum(web_searches), sum(cost_usd)
             FROM ({source}) AS spend
             LEFT JOIN accounts ON accounts.id = spend.account_id
             LEFT JOIN labels AS cwd ON cwd.id = spend.cwd_label
             LEFT JOIN directories AS dir ON dir.cwd_label = spend.cwd_label
             LEFT JOIN labels AS branch ON branch.id = spend.branch_label
             LEFT JOIN labels AS skill ON skill.id = spend.skill_label
             LEFT JOIN labels AS agent ON agent.id = spend.agent_label
             JOIN labels AS model ON model.id = spend.model_label
             LEFT JOIN sessions AS session ON session.id = spend.session_id
             {where_clause}
             GROUP BY spend.account_id, spend.missing, row_key
             ORDER BY spend.account_id IS NULL, accounts.account_uuid,
                      accounts.organization_uuid, spend.missing, sum(cost_usd) DESC, 4",
            key = key_sql(level),
            label = label_sql(level),
        );
        let mut values: Vec<Option<&str>> = vec![since];
        values.extend(filters.iter().map(|filter| Some(filter.key.as_str())));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            let status: Option<String> = row.get(5)?;
            Ok(DataRow {
                account: row.get(0)?,
                account_label: row.get(1)?,
                key: row.get(2)?,
                label: row.get(3)?,
                not_in_transcript: row.get(4)?,
                status: (level == Level::Session)
                    .then(|| reconcile::status_from_name(status.as_deref())),
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
}
