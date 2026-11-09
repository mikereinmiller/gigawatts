use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;

use super::{Result, Store};
use crate::contract::{Limit, LimitSource};
use crate::sources::claude_json::Account;

#[derive(Debug, Clone, PartialEq)]
pub struct UsageAdvice {
    pub fetched_at_ms: i64,
    pub behaviors: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitsReading {
    pub fetched_at_ms: i64,
    pub source: LimitSource,
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitPoint {
    pub kind: String,
    pub scope: Option<String>,
    pub fetched_at_ms: i64,
    pub percent: f64,
}

impl Store {
    pub fn record_limits(&mut self, account: &Account, reading: &LimitsReading) -> Result<bool> {
        let transaction = self.connection.transaction()?;
        let account_id = account_id(&transaction, account)?;
        let newest: Option<i64> = transaction
            .query_row(
                "SELECT fetched_at_ms FROM limit_fetches WHERE account_id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .optional()?;
        if newest.is_some_and(|newest| newest >= reading.fetched_at_ms) {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO limit_fetches (account_id, fetched_at_ms, source, limits)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id) DO UPDATE SET
                 fetched_at_ms = excluded.fetched_at_ms,
                 source = excluded.source,
                 limits = excluded.limits",
            params![
                account_id,
                reading.fetched_at_ms,
                reading.source.name(),
                serde_json::to_string(&reading.limits).expect("limits always serialize"),
            ],
        )?;
        let mut last_percent = transaction.prepare(
            "SELECT percent FROM limit_readings
             WHERE account_id = ?1 AND kind = ?2 AND scope = ?3
             ORDER BY fetched_at_ms DESC LIMIT 1",
        )?;
        let mut insert = transaction.prepare(
            "INSERT OR IGNORE INTO limit_readings
                 (account_id, kind, scope, fetched_at_ms, percent, resets_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for limit in &reading.limits {
            let scope = limit.scope.as_deref().unwrap_or_default();
            let last: Option<f64> = last_percent
                .query_row(params![account_id, limit.kind, scope], |row| row.get(0))
                .optional()?;
            if last == Some(limit.percent) {
                continue;
            }
            insert.execute(params![
                account_id,
                limit.kind,
                scope,
                reading.fetched_at_ms,
                limit.percent,
                limit.resets_at,
            ])?;
        }
        drop(last_percent);
        drop(insert);
        transaction.commit()?;
        Ok(true)
    }

    pub fn latest_limits(&self) -> Result<Vec<(Account, LimitsReading)>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT accounts.account_uuid, accounts.organization_uuid,
                    limit_fetches.fetched_at_ms, limit_fetches.source, limit_fetches.limits
             FROM limit_fetches
             JOIN accounts ON accounts.id = limit_fetches.account_id
             ORDER BY accounts.account_uuid, accounts.organization_uuid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                Account {
                    account_uuid: row.get(0)?,
                    organization_uuid: row.get(1)?,
                },
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut latest = Vec::new();
        for row in rows {
            let (account, fetched_at_ms, source, limits) = row?;
            let (Some(source), Ok(limits)) = (
                LimitSource::from_name(&source),
                serde_json::from_str::<Vec<Limit>>(&limits),
            ) else {
                continue;
            };
            latest.push((
                account,
                LimitsReading {
                    fetched_at_ms,
                    source,
                    limits,
                },
            ));
        }
        Ok(latest)
    }

    pub fn limit_history(&self, account: &Account) -> Result<Vec<LimitPoint>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT kind, scope, fetched_at_ms, percent FROM limit_readings
             JOIN accounts ON accounts.id = limit_readings.account_id
             WHERE accounts.account_uuid = ?1 AND accounts.organization_uuid = ?2
             ORDER BY fetched_at_ms, kind, scope",
        )?;
        let rows =
            statement.query_map([&account.account_uuid, &account.organization_uuid], |row| {
                Ok(LimitPoint {
                    kind: row.get(0)?,
                    scope: Some(row.get::<_, String>(1)?).filter(|scope| !scope.is_empty()),
                    fetched_at_ms: row.get(2)?,
                    percent: row.get(3)?,
                })
            })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn limit_series(
        &self,
        account: &Account,
        kind: &str,
        scope: Option<&str>,
        since_ms: i64,
    ) -> Result<Vec<LimitPoint>> {
        let mut statement = self.connection.prepare_cached(
            "WITH series AS (
                 SELECT limit_readings.* FROM limit_readings
                 JOIN accounts ON accounts.id = limit_readings.account_id
                 WHERE accounts.account_uuid = ?1 AND accounts.organization_uuid = ?2
                   AND kind = ?3 AND scope = ?4
             )
             SELECT kind, scope, fetched_at_ms, percent FROM series
             WHERE fetched_at_ms >= coalesce(
                 (SELECT max(fetched_at_ms) FROM series WHERE fetched_at_ms <= ?5), ?5)
             ORDER BY fetched_at_ms",
        )?;
        let rows = statement.query_map(
            params![
                account.account_uuid,
                account.organization_uuid,
                kind,
                scope.unwrap_or(""),
                since_ms
            ],
            |row| {
                Ok(LimitPoint {
                    kind: row.get(0)?,
                    scope: Some(row.get::<_, String>(1)?).filter(|scope| !scope.is_empty()),
                    fetched_at_ms: row.get(2)?,
                    percent: row.get(3)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn local_minute(&self, timestamp: &str) -> Result<Option<String>> {
        Ok(self.connection.query_row(
            "SELECT strftime('%Y-%m-%d %H:%M', ?1, 'localtime')",
            [timestamp],
            |row| row.get(0),
        )?)
    }

    pub fn usage_advice(&self, account: &Account, day: &str) -> Result<Option<UsageAdvice>> {
        let found: Option<(i64, String)> = self
            .connection
            .query_row(
                "SELECT usage_advice.fetched_at_ms, usage_advice.behaviors FROM usage_advice
                 JOIN accounts ON accounts.id = usage_advice.account_id
                 WHERE accounts.account_uuid = ?1 AND accounts.organization_uuid = ?2
                   AND usage_advice.day = ?3",
                [&account.account_uuid, &account.organization_uuid, day],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(found.map(|(fetched_at_ms, behaviors)| UsageAdvice {
            fetched_at_ms,
            behaviors: serde_json::from_str(&behaviors).unwrap_or(Value::Null),
        }))
    }

    pub fn record_usage_advice(
        &mut self,
        account: &Account,
        day: &str,
        fetched_at_ms: i64,
        behaviors: &Value,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let account_id = account_id(&transaction, account)?;
        transaction.execute(
            "INSERT INTO usage_advice (account_id, day, fetched_at_ms, behaviors)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id, day) DO UPDATE SET
                 fetched_at_ms = excluded.fetched_at_ms,
                 behaviors = excluded.behaviors
             WHERE excluded.fetched_at_ms > usage_advice.fetched_at_ms",
            params![account_id, day, fetched_at_ms, behaviors.to_string()],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn account_id(transaction: &Transaction, account: &Account) -> Result<i64> {
    Ok(transaction.query_row(
        "INSERT INTO accounts (account_uuid, organization_uuid) VALUES (?1, ?2)
         ON CONFLICT (account_uuid, organization_uuid) DO UPDATE SET account_uuid = excluded.account_uuid
         RETURNING id",
        [&account.account_uuid, &account.organization_uuid],
        |row| row.get(0),
    )?)
}

pub(super) fn delete_account(transaction: &Transaction, account_id: i64) -> Result<()> {
    for table in ["limit_fetches", "limit_readings", "usage_advice"] {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE account_id = ?1"),
            [account_id],
        )?;
    }
    Ok(())
}
