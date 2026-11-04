use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Transaction, params};

use super::{Result, Store};
use crate::config::epoch_ms_from_text;
use crate::pricing::{PriceTable, without_date_suffix};
use crate::sources::transcript::Line;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecognizedKind {
    pub kind: String,
    pub lines: u64,
    pub files: u64,
    pub first_seen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSighting {
    pub model: String,
    pub requests: u64,
    pub sessions: u64,
    pub first_seen: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceGap {
    pub model: String,
    pub sessions: u64,
    pub gap_usd: f64,
    pub first_seen: Option<String>,
}

pub(super) fn record_unrecognized(
    transaction: &Transaction,
    path: &str,
    from_start: bool,
    lines: &[Line],
) -> Result<()> {
    if from_start {
        transaction.execute("DELETE FROM unrecognized_lines WHERE path = ?1", [path])?;
    }
    let mut kinds: BTreeMap<&str, (i64, Option<i64>)> = BTreeMap::new();
    for line in lines {
        if let Line::Unrecognized { kind, timestamp } = line {
            let time_ms = timestamp.as_deref().and_then(epoch_ms_from_text);
            let (count, first_ms) = kinds.entry(kind).or_default();
            *count += 1;
            *first_ms = earliest(*first_ms, time_ms);
        }
    }
    for (kind, (count, first_ms)) in kinds {
        transaction.execute(
            "INSERT INTO unrecognized_lines (path, kind, count, first_seen_ms)
             VALUES (?1, ?2, ?3, coalesce(?4, CAST(unixepoch('subsec') * 1000 AS INTEGER)))
             ON CONFLICT (path, kind) DO UPDATE SET
                 count = count + excluded.count,
                 first_seen_ms = min(first_seen_ms, excluded.first_seen_ms)",
            params![path, kind, count, first_ms],
        )?;
    }
    Ok(())
}

fn earliest(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn forget_unrecognized(transaction: &Transaction, path: &str) -> Result<()> {
    transaction.execute("DELETE FROM unrecognized_lines WHERE path = ?1", [path])?;
    transaction.execute(
        "UPDATE file_positions SET offset = 0 WHERE path = ?1",
        [path],
    )?;
    Ok(())
}

impl Store {
    pub fn forget_unrecognized_outside_folders(&mut self, folders: &[PathBuf]) -> Result<()> {
        let prefixes: Vec<String> = folders
            .iter()
            .map(|folder| format!("{}/", folder.to_string_lossy()))
            .collect();
        let transaction = self.connection.transaction()?;
        let stored: Vec<String> = transaction
            .prepare("SELECT DISTINCT path FROM unrecognized_lines")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let outside = stored
            .iter()
            .filter(|path| !prefixes.iter().any(|prefix| path.starts_with(prefix)));
        for path in outside {
            forget_unrecognized(&transaction, path)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn forget_unrecognized_outside(&mut self, root: &Path, files: &[PathBuf]) -> Result<()> {
        let prefix = format!("{}/", root.to_string_lossy());
        let present: HashSet<String> = files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect();
        let transaction = self.connection.transaction()?;
        let stored: Vec<String> = transaction
            .prepare("SELECT DISTINCT path FROM unrecognized_lines WHERE substr(path, 1, length(?1)) = ?1")?
            .query_map([&prefix], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for path in stored.iter().filter(|path| !present.contains(*path)) {
            forget_unrecognized(&transaction, path)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn unrecognized_kinds(&self) -> Result<Vec<UnrecognizedKind>> {
        let mut statement = self.connection.prepare(
            "SELECT kind, sum(count), count(*),
                    datetime(min(first_seen_ms) / 1000, 'unixepoch', 'localtime')
             FROM unrecognized_lines
             WHERE count > 0
             GROUP BY kind
             ORDER BY kind",
        )?;
        let kinds = statement
            .query_map([], |row| {
                Ok(UnrecognizedKind {
                    kind: row.get(0)?,
                    lines: row.get::<_, i64>(1)? as u64,
                    files: row.get::<_, i64>(2)? as u64,
                    first_seen: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(kinds)
    }

    pub fn models_without_price(&self, table: &PriceTable) -> Result<Vec<ModelSighting>> {
        let mut sightings: BTreeMap<String, (u64, u64, Option<i64>)> = BTreeMap::new();
        let mut requests = self.connection.prepare(
            "SELECT model.text, count(*), min(requests.time_ms)
             FROM requests
             JOIN labels AS model ON model.id = requests.model_label
             WHERE requests.cost_usd IS NULL
             GROUP BY requests.model_label",
        )?;
        let rows = requests.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get(2)?))
        })?;
        for row in rows {
            let (model, count, first_ms) = row?;
            if table.model(&model).is_none() {
                let sighting = sightings
                    .entry(without_date_suffix(&model).to_string())
                    .or_default();
                sighting.0 += count as u64;
                sighting.2 = earliest(sighting.2, first_ms);
            }
        }
        for (label, model) in self.session_model_labels()? {
            if table.model(&model).is_some() {
                continue;
            }
            let (sessions, first_ms): (i64, Option<i64>) = self.connection.query_row(
                "SELECT count(*), min(sessions.start_time_ms)
                 FROM session_models
                 JOIN sessions ON sessions.id = session_models.session_id
                 WHERE session_models.model_label = ?1",
                [label],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let sighting = sightings
                .entry(without_date_suffix(&model).to_string())
                .or_default();
            sighting.1 += sessions as u64;
            sighting.2 = earliest(sighting.2, first_ms);
        }
        sightings
            .into_iter()
            .map(|(model, (requests, sessions, first_ms))| {
                Ok(ModelSighting {
                    model,
                    requests,
                    sessions,
                    first_seen: first_ms
                        .map(|ms| self.local_datetime(ms))
                        .transpose()?
                        .flatten(),
                })
            })
            .collect()
    }

    fn session_model_labels(&self) -> Result<Vec<(i64, String)>> {
        let mut statement = self.connection.prepare(
            "WITH RECURSIVE seen (label) AS (
                 SELECT min(model_label) FROM session_models
                 UNION ALL
                 SELECT (SELECT min(model_label) FROM session_models WHERE model_label > seen.label)
                 FROM seen
                 WHERE seen.label IS NOT NULL
             )
             SELECT labels.id, labels.text FROM seen JOIN labels ON labels.id = seen.label",
        )?;
        let labels = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(labels)
    }

    fn local_datetime(&self, ms: i64) -> Result<Option<String>> {
        Ok(self.connection.query_row(
            "SELECT datetime(?1 / 1000, 'unixepoch', 'localtime')",
            [ms],
            |row| row.get(0),
        )?)
    }

    pub fn price_gaps(&self) -> Result<Vec<PriceGap>> {
        let mut statement = self.connection.prepare(
            "SELECT model.text, count(*), sum(abs(price_gap_usd)),
                    datetime(
                        min(coalesce(
                            sessions.start_time_ms,
                            (SELECT min(time_ms) FROM requests
                             WHERE requests.session_id = sessions.id)
                        )) / 1000,
                        'unixepoch', 'localtime'
                    )
             FROM session_models
             JOIN sessions ON sessions.id = session_models.session_id
             JOIN labels AS model ON model.id = session_models.model_label
             WHERE price_gap_usd IS NOT NULL
             GROUP BY model.text
             ORDER BY model.text",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PriceGap {
                model: row.get(0)?,
                sessions: row.get::<_, i64>(1)? as u64,
                gap_usd: row.get(2)?,
                first_seen: row.get(3)?,
            })
        })?;
        let mut gaps: BTreeMap<String, PriceGap> = BTreeMap::new();
        for row in rows {
            let row = row?;
            let model = without_date_suffix(&row.model).to_string();
            match gaps.get_mut(&model) {
                Some(gap) => {
                    gap.sessions += row.sessions;
                    gap.gap_usd += row.gap_usd;
                    gap.first_seen = match (gap.first_seen.take(), row.first_seen) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
                None => {
                    gaps.insert(model.clone(), PriceGap { model, ..row });
                }
            }
        }
        Ok(gaps.into_values().collect())
    }
}
