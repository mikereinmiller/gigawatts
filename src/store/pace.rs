use rusqlite::{OptionalExtension, params};

use super::{Result, Store};
use crate::contract::{SessionSpend, Share};
use crate::sources::claude_json::Account;

const SPEND_TODAY_BY: &str = "
    SELECT session.uuid, {label}, coalesce(sum(requests.cost_usd), 0)
    FROM requests INDEXED BY requests_by_day
    JOIN sessions AS session ON session.id = requests.session_id
    {joins}
    WHERE requests.day = date(?2 / 1000, 'unixepoch', 'localtime')
      AND requests.account_id IS ?3
      AND requests.session_id IN (
          SELECT session_id FROM requests INDEXED BY requests_by_day
          WHERE day >= date((?1 + 1) / 1000, 'unixepoch', 'localtime')
            AND time_ms > ?1 AND time_ms <= ?2 AND account_id IS ?3)
    GROUP BY session.uuid, 2";
const DRIVER_LABEL: &str = "CASE WHEN agent.text IS NOT NULL THEN agent.text || ' subagent'
         ELSE coalesce(skill.text, 'no skill') END";
const DRIVER_JOINS: &str = "LEFT JOIN labels AS skill ON skill.id = requests.skill_label
    LEFT JOIN labels AS agent ON agent.id = requests.agent_label";
const MODEL_LABEL: &str = "model.text";
const MODEL_JOINS: &str = "JOIN labels AS model ON model.id = requests.model_label";

#[derive(Debug, Clone, PartialEq)]
pub struct ClockSpend {
    pub clock: String,
    pub today_usd: f64,
    pub earlier_days_usd: Vec<f64>,
    pub earlier_day_totals_usd: Vec<f64>,
    pub earlier_days: Vec<String>,
    pub weekend: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostFlags {
    pub unpriced_models: Vec<String>,
    pub adjusted_models: Vec<String>,
    pub estimated: bool,
}

impl Store {
    pub fn spend_by_clock_time(
        &self,
        account: Option<&Account>,
        now_ms: i64,
        earlier_days: u32,
    ) -> Result<ClockSpend> {
        let (today, clock, weekend): (String, String, bool) = self.connection.query_row(
            "SELECT date(?1 / 1000, 'unixepoch', 'localtime'),
                    time(?1 / 1000, 'unixepoch', 'localtime'),
                    strftime('%w', ?1 / 1000, 'unixepoch', 'localtime') IN ('0', '6')",
            [now_ms],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let shown_clock = clock.chars().take(5).collect();
        let account_id = match account {
            None => None,
            Some(account) => match self.account_row_id(account)? {
                Some(id) => Some(id),
                None => {
                    return Ok(ClockSpend {
                        clock: shown_clock,
                        today_usd: 0.0,
                        weekend,
                        earlier_days_usd: Vec::new(),
                        earlier_day_totals_usd: Vec::new(),
                        earlier_days: Vec::new(),
                    });
                }
            },
        };
        let mut spend_by = self.connection.prepare_cached(
            "SELECT coalesce(sum(cost_usd)
                        FILTER (WHERE time_ms <= unixepoch(?1 || ' ' || ?3, 'utc') * 1000 + 999), 0),
                    coalesce(sum(cost_usd), 0)
             FROM requests INDEXED BY requests_by_day
             WHERE day = ?1 AND account_id IS ?2",
        )?;
        let today_usd: f64 =
            spend_by.query_row(params![today, account_id, clock], |row| row.get(0))?;
        let mut days = self.connection.prepare_cached(
            "SELECT DISTINCT day FROM daily_totals
             WHERE account_id IS ?1 AND day < ?2
               AND day >= coalesce(
                   (SELECT value FROM store_settings WHERE key = 'requests_kept_from'), '')
               AND (strftime('%w', day) IN ('0', '6')) = ?3
             ORDER BY day DESC
             LIMIT ?4",
        )?;
        let days: Vec<String> = days
            .query_map(params![account_id, today, weekend, earlier_days], |row| {
                row.get(0)
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut earlier_days_usd = Vec::new();
        let mut earlier_day_totals_usd = Vec::new();
        for day in &days {
            let (by_clock, whole_day): (f64, f64) = spend_by
                .query_row(params![day, account_id, clock], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
            earlier_days_usd.push(by_clock);
            earlier_day_totals_usd.push(whole_day);
        }
        Ok(ClockSpend {
            clock: shown_clock,
            today_usd,
            earlier_days_usd,
            earlier_day_totals_usd,
            earlier_days: days,
            weekend,
        })
    }

    pub fn sessions_spending_since(
        &self,
        account: Option<&Account>,
        since_ms: i64,
        now_ms: i64,
    ) -> Result<Vec<SessionSpend>> {
        self.sessions_spending(account, since_ms, since_ms, since_ms, now_ms)
    }

    pub fn sessions_spending_today(
        &self,
        account: Option<&Account>,
        fast_since_ms: i64,
        now_ms: i64,
    ) -> Result<Vec<SessionSpend>> {
        let first_today_ms: Option<i64> = self.connection.query_row(
            "SELECT min(time_ms) FROM requests INDEXED BY requests_by_day
             WHERE day = date(?1 / 1000, 'unixepoch', 'localtime')",
            [now_ms],
            |row| row.get(0),
        )?;
        let Some(first_today_ms) = first_today_ms else {
            return Ok(Vec::new());
        };
        let listed_after_ms = first_today_ms - 1;
        self.sessions_spending(
            account,
            listed_after_ms.min(fast_since_ms),
            listed_after_ms,
            fast_since_ms,
            now_ms,
        )
    }

    fn sessions_spending(
        &self,
        account: Option<&Account>,
        scan_from_ms: i64,
        listed_after_ms: i64,
        fast_since_ms: i64,
        now_ms: i64,
    ) -> Result<Vec<SessionSpend>> {
        let account_id = match account {
            None => None,
            Some(account) => match self.account_row_id(account)? {
                Some(id) => Some(id),
                None => return Ok(Vec::new()),
            },
        };
        let mut sessions = self
            .connection
            .prepare_cached(
                "WITH recent AS (
                     -- SQLite fills the bare `id` from the row that max(time_ms) picks.
                     SELECT session_id,
                            coalesce(sum(cost_usd) FILTER (WHERE time_ms > ?4), 0) AS usd,
                            max(time_ms) AS last_ms, id AS last_id
                     FROM requests INDEXED BY requests_by_day
                     WHERE day >= date((?1 + 1) / 1000, 'unixepoch', 'localtime')
                       AND time_ms > ?1 AND time_ms <= ?2 AND account_id IS ?3
                     GROUP BY session_id
                     HAVING max(time_ms) > ?5
                 )
                 SELECT session.uuid, coalesce(session.title, session.uuid),
                        coalesce(dir.repo_name, cwd.text, 'unknown folder'), branch.text,
                        recent.usd,
                        (SELECT coalesce(sum(cost_usd), 0) FROM requests
                         WHERE requests.session_id = recent.session_id),
                        recent.last_ms,
                        (SELECT coalesce(sum(cost_usd), 0) FROM requests
                         WHERE requests.session_id = recent.session_id
                           AND requests.account_id IS ?3
                           AND requests.day = date(?2 / 1000, 'unixepoch', 'localtime'))
                 FROM recent
                 JOIN sessions AS session ON session.id = recent.session_id
                 JOIN requests AS latest ON latest.id = recent.last_id
                 LEFT JOIN labels AS cwd ON cwd.id = latest.cwd_label
                 LEFT JOIN directories AS dir ON dir.cwd_label = latest.cwd_label
                 LEFT JOIN labels AS branch
                     ON branch.id = coalesce(latest.head_branch_label, latest.git_branch_label)
                 ORDER BY recent.usd DESC, recent.last_ms DESC",
            )?
            .query_map(
                params![
                    scan_from_ms,
                    now_ms,
                    account_id,
                    fast_since_ms,
                    listed_after_ms
                ],
                |row| {
                    Ok(SessionSpend {
                        session: row.get(0)?,
                        label: row.get(1)?,
                        repo: row.get(2)?,
                        branch: row.get(3)?,
                        last_15_minutes_usd: row.get(4)?,
                        total_usd: row.get(5)?,
                        last_active_ms: row.get(6)?,
                        today_usd: row.get(7)?,
                        top_driver: None,
                        models: Vec::new(),
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<SessionSpend>>>()?;
        let window = (scan_from_ms, now_ms, account_id);
        for (uuid, share) in self.shares_today(window, DRIVER_LABEL, DRIVER_JOINS, &sessions)? {
            let session = sessions.iter_mut().find(|session| session.session == uuid);
            if let Some(session) = session.filter(|session| session.top_driver.is_none()) {
                session.top_driver = Some(share);
            }
        }
        for (uuid, share) in self.shares_today(window, MODEL_LABEL, MODEL_JOINS, &sessions)? {
            if let Some(session) = sessions.iter_mut().find(|session| session.session == uuid) {
                session.models.push(share);
            }
        }
        Ok(sessions)
    }

    fn shares_today(
        &self,
        (since_ms, now_ms, account_id): (i64, i64, Option<i64>),
        label: &str,
        joins: &str,
        sessions: &[SessionSpend],
    ) -> Result<Vec<(String, Share)>> {
        let sql = SPEND_TODAY_BY
            .replace("{label}", label)
            .replace("{joins}", joins);
        let mut rows = self
            .connection
            .prepare_cached(&sql)?
            .query_map(params![since_ms, now_ms, account_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
        Ok(rows
            .into_iter()
            .filter_map(|(uuid, label, usd)| {
                let today_usd = sessions
                    .iter()
                    .find(|session| session.session == uuid)?
                    .today_usd;
                let share = if today_usd > 0.0 {
                    usd / today_usd
                } else {
                    0.0
                };
                Some((uuid, Share { label, share }))
            })
            .collect())
    }

    pub fn cost_flags_on(&self, account: Option<&Account>, day: &str) -> Result<CostFlags> {
        let account_id = match account {
            None => None,
            Some(account) => match self.account_row_id(account)? {
                Some(id) => Some(id),
                None => return Ok(CostFlags::default()),
            },
        };
        let models = |sql: &str| -> Result<Vec<String>> {
            Ok(self
                .connection
                .prepare_cached(sql)?
                .query_map(params![day, account_id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        };
        Ok(CostFlags {
            unpriced_models: models(
                "SELECT DISTINCT model.text
                 FROM requests INDEXED BY requests_by_day
                 JOIN labels AS model ON model.id = requests.model_label
                 JOIN sessions ON sessions.id = requests.session_id
                 WHERE requests.day = ?1 AND requests.account_id IS ?2
                   AND requests.cost_usd IS NULL AND sessions.cost_status IS NULL
                 ORDER BY model.text",
            )?,
            adjusted_models: models(
                "SELECT DISTINCT model.text
                 FROM session_models
                 JOIN labels AS model ON model.id = session_models.model_label
                 WHERE price_gap_usd IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM not_in_transcript AS missing
                       WHERE missing.session_id = session_models.session_id
                         AND missing.model_label = session_models.model_label)
                   AND session_id IN (
                       SELECT session_id FROM requests INDEXED BY requests_by_day
                       WHERE day = ?1 AND account_id IS ?2)
                 ORDER BY model.text",
            )?,
            estimated: self
                .connection
                .prepare_cached(
                    "SELECT EXISTS (
                         SELECT 1 FROM requests INDEXED BY requests_by_day
                         JOIN sessions ON sessions.id = requests.session_id
                         WHERE requests.day = ?1 AND requests.account_id IS ?2
                           AND coalesce(sessions.cost_status, 'estimated') = 'estimated')",
                )?
                .query_row(params![day, account_id], |row| row.get(0))?,
        })
    }

    pub fn last_used_account_among(&self, candidates: &[Account]) -> Result<Option<Account>> {
        let mut ids = Vec::new();
        for candidate in candidates {
            if let Some(id) = self.account_row_id(candidate)? {
                ids.push((id, candidate));
            }
        }
        if ids.is_empty() {
            return Ok(None);
        }
        let id_list = ids
            .iter()
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let latest_day: Option<String> = self
            .connection
            .query_row(
                &format!(
                    "SELECT day FROM daily_totals WHERE account_id IN ({id_list})
                     ORDER BY day DESC LIMIT 1"
                ),
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(day) = latest_day else {
            return Ok(None);
        };
        let latest_id: i64 = self.connection.query_row(
            &format!(
                "SELECT account_id FROM (
                     SELECT account_id, time_ms FROM requests INDEXED BY requests_by_day
                     WHERE day = ?1 AND account_id IN ({id_list})
                     UNION ALL
                     SELECT account_id, NULL FROM daily_totals
                     WHERE day = ?1 AND account_id IN ({id_list})
                 )
                 ORDER BY time_ms IS NULL, time_ms DESC
                 LIMIT 1"
            ),
            [&day],
            |row| row.get(0),
        )?;
        Ok(ids
            .into_iter()
            .find(|(id, _)| *id == latest_id)
            .map(|(_, account)| account.clone()))
    }

    pub fn spend_between(
        &self,
        account: &Account,
        from_ms: i64,
        until_ms: i64,
    ) -> Result<Option<f64>> {
        let Some(account_id) = self.account_row_id(account)? else {
            return Ok(Some(0.0));
        };
        Ok(self
            .connection
            .prepare_cached(
                "SELECT CASE
                     WHEN coalesce((SELECT value FROM store_settings WHERE key = 'requests_kept_from'), '')
                          > date(?2 / 1000, 'unixepoch', 'localtime') THEN NULL
                     WHEN EXISTS (
                         SELECT 1 FROM requests INDEXED BY requests_by_day
                         JOIN sessions ON sessions.id = requests.session_id
                         WHERE requests.day BETWEEN date(?2 / 1000, 'unixepoch', 'localtime')
                                                AND date(?3 / 1000, 'unixepoch', 'localtime')
                           AND requests.time_ms >= ?2 AND requests.time_ms <= ?3
                           AND requests.account_id = ?1
                           AND requests.cost_usd IS NULL AND sessions.cost_status IS NULL) THEN NULL
                     ELSE (SELECT coalesce(sum(cost_usd), 0) FROM requests INDEXED BY requests_by_day
                           WHERE day BETWEEN date(?2 / 1000, 'unixepoch', 'localtime')
                                         AND date(?3 / 1000, 'unixepoch', 'localtime')
                             AND time_ms >= ?2 AND time_ms <= ?3 AND account_id = ?1)
                        + (SELECT coalesce(sum(missing.cost_usd), 0) FROM not_in_transcript AS missing
                           JOIN sessions ON sessions.id = missing.session_id
                           WHERE missing.day BETWEEN date(?2 / 1000, 'unixepoch', 'localtime')
                                                 AND date(?3 / 1000, 'unixepoch', 'localtime')
                             AND sessions.account_id = ?1
                             AND coalesce(
                                     (SELECT max(time_ms) FROM requests
                                      WHERE requests.session_id = missing.session_id),
                                     sessions.start_time_ms)
                                 BETWEEN ?2 AND ?3)
                 END",
            )?
            .query_row(params![account_id, from_ms, until_ms], |row| row.get(0))?)
    }

    pub fn spend_times(
        &self,
        account: &Account,
        from_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<(i64, f64)>> {
        let Some(account_id) = self.account_row_id(account)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare_cached(
            "SELECT spent_at_ms, cost_usd FROM (
                 SELECT time_ms AS spent_at_ms, cost_usd FROM requests INDEXED BY requests_by_day
                 WHERE day BETWEEN date(?2 / 1000, 'unixepoch', 'localtime')
                               AND date(?3 / 1000, 'unixepoch', 'localtime')
                   AND account_id = ?1 AND cost_usd IS NOT NULL
                 UNION ALL
                 SELECT coalesce(
                            (SELECT max(time_ms) FROM requests
                             WHERE requests.session_id = missing.session_id),
                            sessions.start_time_ms),
                        missing.cost_usd
                 FROM not_in_transcript AS missing
                 JOIN sessions ON sessions.id = missing.session_id
                 WHERE missing.day BETWEEN date(?2 / 1000, 'unixepoch', 'localtime')
                                       AND date(?3 / 1000, 'unixepoch', 'localtime')
                   AND sessions.account_id = ?1
             )
             WHERE spent_at_ms >= ?2 AND spent_at_ms <= ?3
             ORDER BY spent_at_ms",
        )?;
        let rows = statement.query_map(params![account_id, from_ms, until_ms], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn account_row_id(&self, account: &Account) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM accounts WHERE account_uuid = ?1 AND organization_uuid = ?2",
                [&account.account_uuid, &account.organization_uuid],
                |row| row.get(0),
            )
            .optional()?)
    }
}
