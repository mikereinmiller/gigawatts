use std::collections::HashMap;

use rusqlite::OptionalExtension as _;

use super::totals::{day_bounds, where_sql};
use super::{Filter, Result, Store, reconcile};
use crate::contract::{AccountSessions, SessionSummary, Share};

const ACTIVE_GAP_MS: i64 = 10 * 60_000;

const RANGE_REQUESTS: &str = "
    SELECT *, coalesce(head_branch_label, git_branch_label) AS branch_label
    FROM requests
    WHERE day BETWEEN ?1 AND ?2";

struct Listed {
    session_id: i64,
    account_id: Option<i64>,
    summary: SessionSummary,
}

struct Detail {
    time_ms: Option<i64>,
    cost_usd: f64,
    skill: String,
    model: String,
    agent: String,
    context: u64,
    repo: String,
    branch: Option<String>,
}

impl Store {
    pub fn requests_kept_from(&self) -> Result<Option<String>> {
        let kept = self
            .connection
            .query_row(
                "SELECT value FROM store_settings WHERE key = 'requests_kept_from'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(kept)
    }

    pub fn sessions(
        &self,
        since: Option<&str>,
        until: Option<&str>,
        filters: &[Filter],
        limit: usize,
    ) -> Result<(Vec<AccountSessions>, Vec<SessionSummary>)> {
        let sql = format!(
            "SELECT spend.account_id, accounts.account_uuid || '/' || accounts.organization_uuid,
                    session.uuid, session.title, session.cost_status,
                    coalesce(spend.cost_usd, 0), spend.requests, spend.session_id
             FROM (SELECT spend.session_id, spend.account_id,
                          sum(spend.cost_usd) AS cost_usd, count(*) AS requests
                   FROM ({RANGE_REQUESTS}) AS spend
                   {where_clause}
                   GROUP BY spend.session_id, spend.account_id) AS spend
             JOIN sessions AS session ON session.id = spend.session_id
             LEFT JOIN accounts ON accounts.id = spend.account_id
             ORDER BY spend.cost_usd DESC, session.uuid",
            where_clause = where_sql(filters),
        );
        let bounds = day_bounds(since, until);
        let mut values: Vec<&str> = bounds.to_vec();
        values.extend(filters.iter().map(|filter| filter.key.as_str()));
        let mut statement = self.connection.prepare_cached(&sql)?;
        let listed = statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                let status: Option<String> = row.get(4)?;
                Ok(Listed {
                    session_id: row.get(7)?,
                    account_id: row.get(0)?,
                    summary: SessionSummary {
                        account: row.get(1)?,
                        session: row.get(2)?,
                        title: row.get(3)?,
                        repo: String::new(),
                        branch: None,
                        first_ms: None,
                        last_ms: None,
                        active_minutes: 0,
                        requests: row.get::<_, i64>(6)? as u64,
                        cost_usd: row.get(5)?,
                        cost_status: reconcile::status_from_name(status.as_deref()),
                        peak_context: 0,
                        skills: Vec::new(),
                        models: Vec::new(),
                        agents: Vec::new(),
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut per_account: Vec<AccountSessions> = Vec::new();
        for entry in &listed {
            let account = &entry.summary.account;
            match per_account
                .iter_mut()
                .find(|count| &count.account == account)
            {
                Some(count) => count.sessions += 1,
                None => per_account.push(AccountSessions {
                    account: account.clone(),
                    sessions: 1,
                }),
            }
        }
        let mut sessions = Vec::new();
        let mut taken: HashMap<Option<i64>, usize> = HashMap::new();
        for listed in listed {
            let taken_for_account = taken.entry(listed.account_id).or_default();
            if *taken_for_account == limit {
                continue;
            }
            *taken_for_account += 1;
            let details = self.session_requests(&listed, bounds, filters)?;
            sessions.push(summarize(listed.summary, &details));
        }
        Ok((per_account, sessions))
    }

    fn session_requests(
        &self,
        listed: &Listed,
        bounds: [&str; 2],
        filters: &[Filter],
    ) -> Result<Vec<Detail>> {
        let session_at = filters.len() + 3;
        let filtered = match where_sql(filters) {
            conditions if conditions.is_empty() => "WHERE".to_string(),
            conditions => format!("{conditions} AND"),
        };
        let sql = format!(
            "SELECT spend.time_ms, coalesce(spend.cost_usd, 0),
                    coalesce(skill.text, 'no skill'), model.text,
                    coalesce(agent.text, 'you (no subagent)'),
                    spend.input_tokens + spend.cache_read_tokens
                        + spend.cache_write_5m_tokens + spend.cache_write_1h_tokens,
                    coalesce(dir.repo_name, cwd.text, 'unknown folder'),
                    coalesce(head.text, branch.text)
             FROM ({RANGE_REQUESTS}) AS spend
             JOIN labels AS model ON model.id = spend.model_label
             LEFT JOIN labels AS skill ON skill.id = spend.skill_label
             LEFT JOIN labels AS agent ON agent.id = spend.agent_label
             LEFT JOIN labels AS cwd ON cwd.id = spend.cwd_label
             LEFT JOIN directories AS dir ON dir.cwd_label = spend.cwd_label
             LEFT JOIN labels AS head ON head.id = spend.head_branch_label
             LEFT JOIN labels AS branch ON branch.id = spend.git_branch_label
             {filtered} spend.session_id = ?{session_at}
               AND spend.account_id IS ?{account_at}
             ORDER BY spend.time_ms",
            account_at = session_at + 1,
        );
        let mut values: Vec<rusqlite::types::Value> = bounds
            .iter()
            .map(|bound| bound.to_string().into())
            .collect();
        values.extend(filters.iter().map(|filter| filter.key.clone().into()));
        values.push(listed.session_id.into());
        values.push(listed.account_id.into());
        let mut statement = self.connection.prepare_cached(&sql)?;
        let details = statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                Ok(Detail {
                    time_ms: row.get(0)?,
                    cost_usd: row.get(1)?,
                    skill: row.get(2)?,
                    model: row.get(3)?,
                    agent: row.get(4)?,
                    context: row.get::<_, i64>(5)? as u64,
                    repo: row.get(6)?,
                    branch: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(details)
    }
}

fn summarize(mut summary: SessionSummary, details: &[Detail]) -> SessionSummary {
    let times: Vec<i64> = details.iter().filter_map(|detail| detail.time_ms).collect();
    summary.first_ms = times.first().copied();
    summary.last_ms = times.last().copied();
    let active_ms: i64 = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap <= ACTIVE_GAP_MS)
        .sum();
    summary.active_minutes = (active_ms / 60_000) as u64;
    summary.peak_context = details
        .iter()
        .filter(|detail| detail.agent == "you (no subagent)")
        .map(|detail| detail.context)
        .max()
        .unwrap_or(0);
    summary.repo = most_common(details.iter().map(|detail| detail.repo.clone()))
        .unwrap_or_else(|| "unknown folder".to_string());
    summary.branch = most_common(details.iter().filter_map(|detail| detail.branch.clone()));
    summary.skills = shares(details, |detail| &detail.skill);
    summary.models = shares(details, |detail| &detail.model);
    summary.agents = shares(details, |detail| &detail.agent);
    summary
}

fn most_common(values: impl Iterator<Item = String>) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(value, _)| value)
}

fn shares(details: &[Detail], key: impl Fn(&Detail) -> &String) -> Vec<Share> {
    let total: f64 = details.iter().map(|detail| detail.cost_usd).sum();
    let mut costs: HashMap<&String, f64> = HashMap::new();
    for detail in details {
        *costs.entry(key(detail)).or_default() += detail.cost_usd;
    }
    let mut shares: Vec<Share> = costs
        .into_iter()
        .map(|(label, cost)| Share {
            label: label.clone(),
            share: if total > 0.0 { cost / total } else { 0.0 },
        })
        .collect();
    shares.sort_by(|a, b| {
        b.share
            .total_cmp(&a.share)
            .then_with(|| a.label.cmp(&b.label))
    });
    shares
}
