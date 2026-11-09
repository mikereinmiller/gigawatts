use rusqlite::{OptionalExtension, params};

use super::{Result, Store};
use crate::contract::FastestSession;
use crate::sources::claude_json::Account;

#[derive(Debug, Clone, PartialEq)]
pub struct ClockSpend {
    pub clock: String,
    pub today_usd: f64,
    pub earlier_days_usd: Vec<f64>,
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
                        earlier_days_usd: Vec::new(),
                    });
                }
            },
        };
        let mut spend_by = self.connection.prepare_cached(
            "SELECT coalesce(sum(cost_usd), 0) FROM requests INDEXED BY requests_by_day
             WHERE day = ?1 AND account_id IS ?2
               AND time_ms <= unixepoch(?1 || ' ' || ?3, 'utc') * 1000 + 999",
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
        for day in days {
            earlier_days_usd
                .push(spend_by.query_row(params![day, account_id, clock], |row| row.get(0))?);
        }
        Ok(ClockSpend {
            clock: shown_clock,
            today_usd,
            earlier_days_usd,
        })
    }

    pub fn fastest_session_since(
        &self,
        account: Option<&Account>,
        since_ms: i64,
        now_ms: i64,
    ) -> Result<Option<FastestSession>> {
        let account_id = match account {
            None => None,
            Some(account) => match self.account_row_id(account)? {
                Some(id) => Some(id),
                None => return Ok(None),
            },
        };
        Ok(self
            .connection
            .prepare_cached(
                "WITH recent AS (
                     SELECT session_id, coalesce(sum(cost_usd), 0) AS usd, max(time_ms) AS last_ms
                     FROM requests INDEXED BY requests_by_day
                     WHERE day >= date(?1 / 1000, 'unixepoch', 'localtime')
                       AND time_ms > ?1 AND time_ms <= ?2 AND account_id IS ?3
                     GROUP BY session_id
                 )
                 SELECT session.uuid, coalesce(session.title, session.uuid),
                        coalesce(dir.repo_name, cwd.text, 'unknown folder'), branch.text,
                        recent.usd,
                        (SELECT coalesce(sum(cost_usd), 0) FROM requests
                         WHERE requests.session_id = recent.session_id)
                 FROM recent
                 JOIN sessions AS session ON session.id = recent.session_id
                 JOIN requests AS latest
                     ON latest.session_id = recent.session_id AND latest.time_ms = recent.last_ms
                 LEFT JOIN labels AS cwd ON cwd.id = latest.cwd_label
                 LEFT JOIN directories AS dir ON dir.cwd_label = latest.cwd_label
                 LEFT JOIN labels AS branch
                     ON branch.id = coalesce(latest.head_branch_label, latest.git_branch_label)
                 ORDER BY recent.usd DESC, recent.last_ms DESC
                 LIMIT 1",
            )?
            .query_row(params![since_ms, now_ms, account_id], |row| {
                Ok(FastestSession {
                    session: row.get(0)?,
                    label: row.get(1)?,
                    repo: row.get(2)?,
                    branch: row.get(3)?,
                    last_15_minutes_usd: row.get(4)?,
                    total_usd: row.get(5)?,
                })
            })
            .optional()?)
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
