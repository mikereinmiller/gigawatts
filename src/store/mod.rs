use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use rusqlite_migration::{M, Migrations};

use crate::accounts::Attribution;
use crate::config::{Switch, epoch_ms_from_text};
use crate::contract::{DataRow, Level};
use crate::pricing::PriceTable;
use crate::sources::claude_json::{Account, Login};
use crate::sources::git::{Place, Resolver};
use crate::sources::transcript::{CostState, Line, READER_VERSION, Request, Tokens};

mod limits;
mod pace;
mod problems;
mod reconcile;
mod retention;
mod totals;

pub use limits::{LimitPoint, LimitsReading, UsageAdvice};
pub use pace::{ClockSpend, CostFlags};
pub use problems::{ModelSighting, PriceGap, UnrecognizedKind};
pub use totals::Filter;

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Migration(rusqlite_migration::Error),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Sqlite(error) => write!(f, "{error}"),
            Error::Migration(error) => write!(f, "{error}"),
            Error::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Error::Sqlite(error)
    }
}

impl From<rusqlite_migration::Error> for Error {
    fn from(error: rusqlite_migration::Error) -> Self {
        Error::Migration(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePosition {
    pub path: String,
    pub device: u64,
    pub inode: u64,
    pub first_line_hash: Option<u64>,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAccount {
    pub account: Account,
    pub email: Option<String>,
    pub requests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenLogin {
    pub from_ms: i64,
    pub account: Account,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginChange {
    pub path: String,
    pub previous: Account,
    pub current: Account,
    pub changed_at_ms: i64,
}

pub struct Store {
    connection: Connection,
}

pub const MIGRATIONS: [&str; 9] = [
    include_str!("migrations/001_history.sql"),
    include_str!("migrations/002_accounts.sql"),
    include_str!("migrations/003_repricing.sql"),
    include_str!("migrations/004_login_history.sql"),
    include_str!("migrations/005_attribution.sql"),
    include_str!("migrations/006_problems.sql"),
    include_str!("migrations/007_retention.sql"),
    include_str!("migrations/008_expired_requests.sql"),
    include_str!("migrations/009_limits.sql"),
];

fn migrations() -> Migrations<'static> {
    Migrations::new(MIGRATIONS.iter().map(|sql| M::up(sql)).collect())
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut connection = Connection::open(path)?;
        restrict_to_owner(path)?;
        connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrations().to_latest(&mut connection)?;
        read_again_if_reader_changed(&mut connection)?;
        Ok(Store { connection })
    }

    pub fn position(&self, path: &str) -> Result<Option<FilePosition>> {
        let position = self
            .connection
            .query_row(
                "SELECT device, inode, first_line_hash, offset FROM file_positions WHERE path = ?1",
                [path],
                |row| {
                    Ok(FilePosition {
                        path: path.to_string(),
                        device: row.get::<_, i64>(0)? as u64,
                        inode: row.get::<_, i64>(1)? as u64,
                        first_line_hash: row.get::<_, Option<i64>>(2)?.map(|hash| hash as u64),
                        offset: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .optional()?;
        Ok(position)
    }

    pub fn save_file(
        &mut self,
        position: &FilePosition,
        from_start: bool,
        lines: &[Line],
        attribution: &Attribution,
        table: &PriceTable,
        resolver: &Resolver,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        problems::record_unrecognized(&transaction, &position.path, from_start, lines)?;
        let mut writer = Writer::new(&transaction)?;
        let mut session_accounts: HashMap<&str, Option<Account>> = HashMap::new();
        let mut moved_sessions: BTreeSet<&str> = BTreeSet::new();
        let mut partly_untracked_sessions: BTreeSet<&str> = BTreeSet::new();
        for line in lines {
            let Line::Request(request) = line else {
                continue;
            };
            let time_ms = request
                .detail
                .timestamp
                .as_deref()
                .and_then(epoch_ms_from_text);
            let account = attribution.account_at(time_ms);
            session_accounts
                .entry(&request.session_id)
                .or_insert_with(|| account.cloned());
            let account_id = account.map(|account| writer.account(account)).transpose()?;
            if let Some(account_id) = account_id {
                writer.claim_session(&request.session_id, account_id)?;
            }
            if attribution.tracks(account) && writer.expired(request)? {
                continue;
            }
            let moved = if attribution.tracks(account) {
                let moved = writer.reattribute(&request.request_id, account_id)?;
                writer.request(request, account_id, table)?;
                moved
            } else {
                partly_untracked_sessions.insert(&request.session_id);
                writer.drop_request(&request.request_id, account_id)?
            };
            if moved {
                moved_sessions.insert(&request.session_id);
            }
        }
        for session_id in partly_untracked_sessions {
            writer.set_cost_aside(session_id)?;
        }
        for line in lines {
            match line {
                Line::CostState(state) => {
                    let account = session_account(
                        &transaction,
                        &session_accounts,
                        &state.session_id,
                        attribution,
                    )?;
                    if attribution.tracks(account.as_ref()) {
                        if let Some(account) = &account {
                            let account_id = writer.account(account)?;
                            writer.claim_session(&state.session_id, account_id)?;
                        }
                        writer.cost_state(state)?;
                    }
                }
                Line::Title { session_id, title } => {
                    let account =
                        session_account(&transaction, &session_accounts, session_id, attribution)?;
                    if attribution.tracks(account.as_ref())
                        && attribution.stores_titles(account.as_ref())
                    {
                        writer.title(session_id, title)?;
                    }
                }
                Line::Request(_) | Line::Unrecognized { .. } => {}
            }
        }
        writer.resolve_places(resolver)?;
        for session_id in std::mem::take(&mut writer.touched_sessions) {
            let changed = reconcile::session(&transaction, session_id, table)?;
            writer.changed_days.extend(changed);
        }
        for day in &writer.changed_days {
            rebuild_daily_totals(&transaction, day)?;
        }
        for session_id in moved_sessions {
            if let Some(Some(account)) = session_accounts.get(session_id) {
                let account_id = writer.account(account)?;
                writer.reclaim_session(session_id, account_id)?;
            }
        }
        save_position(&transaction, position)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn record_login(&mut self, path: &str, login: &Login, now_ms: i64) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let account = &login.account;
        let account_id: i64 = transaction.query_row(
            "INSERT INTO accounts (account_uuid, organization_uuid, email) VALUES (?1, ?2, ?3)
             ON CONFLICT (account_uuid, organization_uuid) DO UPDATE SET email = excluded.email
             RETURNING id",
            params![account.account_uuid, account.organization_uuid, login.email],
            |row| row.get(0),
        )?;
        let earlier_account_id: Option<i64> = transaction
            .query_row(
                "SELECT account_id FROM directory_logins WHERE path = ?1",
                [path],
                |row| row.get(0),
            )
            .optional()?;
        let from_ms = match earlier_account_id {
            None => Some(0),
            Some(earlier) if earlier != account_id => Some(now_ms),
            Some(_) => None,
        };
        if let Some(from_ms) = from_ms {
            transaction.execute(
                "INSERT OR REPLACE INTO directory_login_history (path, from_ms, account_id)
                 VALUES (?1, ?2, ?3)",
                params![path, from_ms, account_id],
            )?;
        }
        transaction.execute(
            "INSERT INTO directory_logins (path, account_id) VALUES (?1, ?2)
             ON CONFLICT (path) DO UPDATE SET
                previous_account_id = directory_logins.account_id,
                account_id = excluded.account_id,
                changed_at_ms = ?3
             WHERE directory_logins.account_id != excluded.account_id",
            params![path, account_id, now_ms],
        )?;
        let read_logged_out = transaction.execute(
            "DELETE FROM store_settings WHERE key = ?1",
            [logged_out_key(path)],
        )? > 0;
        if read_logged_out {
            delete_positions_under(&transaction, &projects_prefix(path))?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_logged_out(&mut self, path: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO store_settings (key, value) VALUES (?1, '')",
            [logged_out_key(path)],
        )?;
        Ok(())
    }

    pub fn seen_logins(&self, path: &str) -> Result<Vec<SeenLogin>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT from_ms, account_uuid, organization_uuid
             FROM directory_login_history
             JOIN accounts ON accounts.id = directory_login_history.account_id
             WHERE path = ?1
             ORDER BY from_ms",
        )?;
        let logins = statement
            .query_map([path], |row| {
                Ok(SeenLogin {
                    from_ms: row.get(0)?,
                    account: Account {
                        account_uuid: row.get(1)?,
                        organization_uuid: row.get(2)?,
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(logins)
    }

    pub fn read_again_if_switch_changed(
        &mut self,
        directory: &str,
        projects: &Path,
        switch: Option<&Switch>,
    ) -> Result<bool> {
        let fingerprint = switch.map_or_else(
            || "none".to_string(),
            |switch| format!("{} {}", switch.at_ms, switch.earlier),
        );
        let key = format!("switch:{directory}");
        let transaction = self.connection.transaction()?;
        let stored: Option<String> = transaction
            .query_row(
                "SELECT value FROM store_settings WHERE key = ?1",
                [&key],
                |row| row.get(0),
            )
            .optional()?;
        if stored.as_deref().unwrap_or("none") == fingerprint {
            return Ok(false);
        }
        delete_positions_under(&transaction, &format!("{}/", projects.to_string_lossy()))?;
        transaction.execute(
            "INSERT INTO store_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [&key, &fingerprint],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn delete_emptied_sessions(&mut self, untracked: &HashSet<Account>) -> Result<()> {
        self.without_foreign_keys(|store| store.delete_sessions_without_requests(untracked))
    }

    fn delete_sessions_without_requests(&mut self, untracked: &HashSet<Account>) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for account in untracked {
            transaction
                .execute_batch("CREATE TEMP TABLE emptied_sessions (id INTEGER PRIMARY KEY);")?;
            transaction.execute(
                "INSERT INTO emptied_sessions
                 SELECT sessions.id FROM sessions
                 JOIN accounts ON accounts.id = sessions.account_id
                 WHERE accounts.account_uuid = ?1 AND accounts.organization_uuid = ?2
                   AND sessions.id NOT IN (SELECT session_id FROM requests)
                   AND NOT sessions.requests_expired",
                [&account.account_uuid, &account.organization_uuid],
            )?;
            transaction.execute_batch(
                "DELETE FROM not_in_transcript WHERE session_id IN (SELECT id FROM emptied_sessions);
                 DELETE FROM session_models WHERE session_id IN (SELECT id FROM emptied_sessions);
                 DELETE FROM sessions WHERE id IN (SELECT id FROM emptied_sessions);
                 DROP TABLE emptied_sessions;",
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn login_changes(&self) -> Result<Vec<LoginChange>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT path, previous.account_uuid, previous.organization_uuid,
                    current.account_uuid, current.organization_uuid, changed_at_ms
             FROM directory_logins
             JOIN accounts AS previous ON previous.id = directory_logins.previous_account_id
             JOIN accounts AS current ON current.id = directory_logins.account_id
             ORDER BY path",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(LoginChange {
                path: row.get(0)?,
                previous: Account {
                    account_uuid: row.get(1)?,
                    organization_uuid: row.get(2)?,
                },
                current: Account {
                    account_uuid: row.get(3)?,
                    organization_uuid: row.get(4)?,
                },
                changed_at_ms: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn accounts(&self) -> Result<Vec<StoredAccount>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT account_uuid, organization_uuid, email,
                    (SELECT coalesce(sum(requests), 0) FROM daily_totals
                     WHERE daily_totals.account_id = accounts.id)
             FROM accounts
             ORDER BY account_uuid, organization_uuid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredAccount {
                account: Account {
                    account_uuid: row.get(0)?,
                    organization_uuid: row.get(1)?,
                },
                email: row.get(2)?,
                requests: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn session_title(&self, uuid: &str) -> Result<Option<String>> {
        let title = self
            .connection
            .query_row(
                "SELECT title FROM sessions WHERE uuid = ?1",
                [uuid],
                |row| row.get(0),
            )
            .optional()?;
        Ok(title.flatten())
    }

    pub fn delete_titles(&mut self, account: &Account) -> Result<u64> {
        let deleted = self.connection.execute(
            "UPDATE sessions SET title = NULL
             WHERE title IS NOT NULL AND id IN (
                SELECT requests.session_id FROM requests
                JOIN accounts ON accounts.id = requests.account_id
                WHERE accounts.account_uuid = ?1 AND accounts.organization_uuid = ?2
                UNION
                SELECT sessions.id FROM sessions
                JOIN accounts ON accounts.id = sessions.account_id
                WHERE accounts.account_uuid = ?1 AND accounts.organization_uuid = ?2
             )",
            [&account.account_uuid, &account.organization_uuid],
        )?;
        Ok(deleted as u64)
    }

    pub fn resolve_unplaced_folders(&mut self, resolver: &Resolver) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let unplaced: BTreeMap<i64, String> = transaction
            .prepare(
                "SELECT DISTINCT cwd.id, cwd.text FROM daily_totals
                 JOIN labels AS cwd ON cwd.id = daily_totals.cwd_label
                 WHERE daily_totals.cwd_label NOT IN (SELECT cwd_label FROM directories)",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let unplaced_labels: Vec<i64> = unplaced.keys().copied().collect();
        let mut writer = Writer::new(&transaction)?;
        writer.cwds = unplaced;
        writer.resolve_places(resolver)?;
        let detached = writer.label(Some("detached"))?;
        drop(writer);
        let mut changed_days = BTreeSet::new();
        let mut label_detached = transaction.prepare(
            "UPDATE requests SET head_branch_label = ?2
             WHERE cwd_label = ?1 AND head_branch_label IS NULL
               AND git_branch_label = (SELECT id FROM labels WHERE text = 'HEAD')
             RETURNING day",
        )?;
        for cwd_label in unplaced_labels {
            let days = label_detached
                .query_map(params![cwd_label, detached], |row| {
                    row.get::<_, Option<String>>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            changed_days.extend(days.into_iter().flatten());
        }
        drop(label_detached);
        for day in &changed_days {
            rebuild_daily_totals(&transaction, day)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn forget(&mut self, account: &Account) -> Result<u64> {
        self.without_foreign_keys(|store| store.delete_account_rows(account))
    }

    // SQLite scans unindexed child tables per deleted parent while foreign keys are on, and the pragma can't change inside a transaction.
    fn without_foreign_keys<T>(
        &mut self,
        delete: impl FnOnce(&mut Store) -> Result<T>,
    ) -> Result<T> {
        self.connection.pragma_update(None, "foreign_keys", "OFF")?;
        let deleted = delete(self);
        self.connection.pragma_update(None, "foreign_keys", "ON")?;
        deleted
    }

    fn delete_account_rows(&mut self, account: &Account) -> Result<u64> {
        let transaction = self.connection.transaction()?;
        let Some(account_id) = transaction
            .query_row(
                "SELECT id FROM accounts WHERE account_uuid = ?1 AND organization_uuid = ?2",
                [&account.account_uuid, &account.organization_uuid],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(0);
        };
        transaction.execute(
            "CREATE TEMP TABLE forgotten_sessions AS
             SELECT session_id AS id FROM requests WHERE account_id = ?1
             UNION SELECT id FROM sessions WHERE account_id = ?1",
            [account_id],
        )?;
        let deleted: i64 = transaction.query_row(
            "SELECT coalesce(sum(requests), 0) FROM daily_totals WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "DELETE FROM daily_totals WHERE account_id = ?1",
            [account_id],
        )?;
        transaction.execute(
            "DELETE FROM request_tools WHERE request_row_id IN (
                SELECT id FROM requests WHERE account_id = ?1
             )",
            [account_id],
        )?;
        transaction.execute("DELETE FROM requests WHERE account_id = ?1", [account_id])?;
        transaction.execute_batch(
            "DELETE FROM not_in_transcript WHERE session_id IN (SELECT id FROM forgotten_sessions);
             DELETE FROM expired_requests WHERE session_id IN (SELECT id FROM forgotten_sessions);
             DELETE FROM session_models WHERE session_id IN (SELECT id FROM forgotten_sessions);
             UPDATE sessions SET
                 account_id = (SELECT account_id FROM requests
                               WHERE session_id = sessions.id ORDER BY day, id LIMIT 1),
                 total_cost_usd = NULL,
                 cost_status = NULL,
                 cost_forgotten = 1
             WHERE id IN (SELECT id FROM forgotten_sessions);
             DELETE FROM forgotten_sessions WHERE id IN (SELECT session_id FROM requests);
             DELETE FROM sessions WHERE id IN (SELECT id FROM forgotten_sessions);
             DROP TABLE forgotten_sessions;
             DELETE FROM store_settings WHERE key = 'price_table';",
        )?;
        transaction.execute(
            "UPDATE accounts SET email = NULL WHERE id = ?1",
            [account_id],
        )?;
        limits::delete_account(&transaction, account_id)?;
        delete_unused_labels(&transaction)?;
        transaction.commit()?;
        Ok(deleted as u64)
    }

    pub fn forget_positions_under(&mut self, folder: &Path) -> Result<()> {
        let transaction = self.connection.transaction()?;
        delete_positions_under(&transaction, &format!("{}/", folder.to_string_lossy()))?;
        transaction.commit()?;
        Ok(())
    }

    pub fn session_count(&self) -> Result<u64> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn reprice_if_changed(&mut self, table: &PriceTable) -> Result<bool> {
        let fingerprint = table.fingerprint();
        let transaction = self.connection.transaction()?;
        let stored: Option<String> = transaction
            .query_row(
                "SELECT value FROM store_settings WHERE key = 'price_table'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if stored.as_deref() == Some(fingerprint.as_str()) {
            return Ok(false);
        }
        let mut changed_days = BTreeSet::new();
        {
            let mut select = transaction.prepare(
                "SELECT requests.id, model.text, input_tokens, output_tokens, cache_read_tokens,
                        cache_write_5m_tokens, cache_write_1h_tokens, web_searches,
                        geo.text, speed.text, cost_usd, day
                 FROM requests
                 JOIN labels AS model ON model.id = requests.model_label
                 LEFT JOIN labels AS geo ON geo.id = requests.inference_geo_label
                 LEFT JOIN labels AS speed ON speed.id = requests.speed_label",
            )?;
            let mut update =
                transaction.prepare("UPDATE requests SET cost_usd = ?2 WHERE id = ?1")?;
            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let tokens = Tokens {
                    input: row.get::<_, i64>(2)? as u64,
                    output: row.get::<_, i64>(3)? as u64,
                    cache_read: row.get::<_, i64>(4)? as u64,
                    cache_write_5m: row.get::<_, i64>(5)? as u64,
                    cache_write_1h: row.get::<_, i64>(6)? as u64,
                    web_searches: row.get::<_, i64>(7)? as u64,
                };
                let model: String = row.get(1)?;
                let geo: Option<String> = row.get(8)?;
                let speed: Option<String> = row.get(9)?;
                let cost = table.price_tokens(&model, &tokens, geo.as_deref(), speed.as_deref());
                let old: Option<f64> = row.get(10)?;
                if cost != old {
                    update.execute(params![row.get::<_, i64>(0)?, cost])?;
                    if let Some(day) = row.get::<_, Option<String>>(11)? {
                        changed_days.insert(day);
                    }
                }
            }
        }
        let reconciled: Vec<i64> = transaction
            .prepare("SELECT id FROM sessions WHERE total_cost_usd IS NOT NULL")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for session_id in reconciled {
            changed_days.extend(reconcile::session(&transaction, session_id, table)?);
        }
        for day in &changed_days {
            rebuild_daily_totals(&transaction, day)?;
        }
        transaction.execute(
            "INSERT INTO store_settings (key, value) VALUES ('price_table', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [&fingerprint],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn request_count(&self) -> Result<u64> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn request_tools(&self, request_id: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT tool.text FROM request_tools
             JOIN requests ON requests.id = request_tools.request_row_id
             JOIN labels AS tool ON tool.id = request_tools.tool_label
             WHERE requests.request_id = ?1
             ORDER BY tool.text",
        )?;
        let tools = statement.query_map([request_id], |row| row.get(0))?;
        Ok(tools.collect::<rusqlite::Result<_>>()?)
    }

    pub fn local_today(&self) -> Result<String> {
        let today = self
            .connection
            .query_row("SELECT date('now', 'localtime')", [], |row| row.get(0))?;
        Ok(today)
    }

    pub fn week_start(&self, day: &str) -> Result<String> {
        let monday =
            self.connection
                .query_row("SELECT date(?1, '-6 days', 'weekday 1')", [day], |row| {
                    row.get(0)
                })?;
        Ok(monday)
    }

    pub fn month_start(&self, day: &str) -> Result<String> {
        let first =
            self.connection
                .query_row("SELECT date(?1, 'start of month')", [day], |row| row.get(0))?;
        Ok(first)
    }

    pub fn totals_by_model(&self, since: Option<&str>) -> Result<Vec<DataRow>> {
        self.totals(Level::Model, since, &[])
    }
}

struct Writer<'a> {
    transaction: &'a Transaction<'a>,
    labels: HashMap<String, i64>,
    sessions: HashMap<String, i64>,
    accounts: HashMap<Account, i64>,
    changed_days: BTreeSet<String>,
    touched_sessions: BTreeSet<i64>,
    cwds: BTreeMap<i64, String>,
    head_requests: BTreeMap<String, Vec<String>>,
    requests_kept_from: Option<String>,
}

impl<'a> Writer<'a> {
    fn new(transaction: &'a Transaction<'a>) -> Result<Self> {
        let requests_kept_from = transaction
            .query_row(
                "SELECT value FROM store_settings WHERE key = 'requests_kept_from'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(Writer {
            transaction,
            labels: HashMap::new(),
            sessions: HashMap::new(),
            accounts: HashMap::new(),
            changed_days: BTreeSet::new(),
            touched_sessions: BTreeSet::new(),
            cwds: BTreeMap::new(),
            head_requests: BTreeMap::new(),
            requests_kept_from,
        })
    }

    fn expired(&mut self, request: &Request) -> Result<bool> {
        let Some(kept_from) = self.requests_kept_from.clone() else {
            return Ok(false);
        };
        let expired: bool = self
            .transaction
            .prepare_cached("SELECT coalesce(date(?1, 'localtime') < ?2, 0)")?
            .query_row(params![request.detail.timestamp, kept_from], |row| {
                row.get(0)
            })?;
        if expired {
            let session_id = self.session(&request.session_id)?;
            self.transaction
                .prepare_cached(
                    "UPDATE sessions SET
                         requests_expired = 1,
                         requests_skipped = requests_skipped OR NOT EXISTS (
                             SELECT 1 FROM expired_requests
                             JOIN labels ON labels.id = expired_requests.model_label
                             WHERE expired_requests.session_id = ?1 AND labels.text = ?2
                         )
                     WHERE id = ?1",
                )?
                .execute(params![session_id, request.model])?;
        }
        Ok(expired)
    }

    fn account(&mut self, account: &Account) -> Result<i64> {
        if let Some(&id) = self.accounts.get(account) {
            return Ok(id);
        }
        let id = self.transaction.query_row(
            "INSERT INTO accounts (account_uuid, organization_uuid) VALUES (?1, ?2)
             ON CONFLICT (account_uuid, organization_uuid) DO UPDATE SET account_uuid = excluded.account_uuid
             RETURNING id",
            [&account.account_uuid, &account.organization_uuid],
            |row| row.get(0),
        )?;
        self.accounts.insert(account.clone(), id);
        Ok(id)
    }

    fn label(&mut self, text: Option<&str>) -> Result<Option<i64>> {
        let Some(text) = text else {
            return Ok(None);
        };
        if let Some(&id) = self.labels.get(text) {
            return Ok(Some(id));
        }
        let id = self.transaction.query_row(
            "INSERT INTO labels (text) VALUES (?1)
             ON CONFLICT (text) DO UPDATE SET text = excluded.text
             RETURNING id",
            [text],
            |row| row.get(0),
        )?;
        self.labels.insert(text.to_string(), id);
        Ok(Some(id))
    }

    fn session(&mut self, uuid: &str) -> Result<i64> {
        if let Some(&id) = self.sessions.get(uuid) {
            return Ok(id);
        }
        let id = self.transaction.query_row(
            "INSERT INTO sessions (uuid) VALUES (?1)
             ON CONFLICT (uuid) DO UPDATE SET uuid = excluded.uuid
             RETURNING id",
            [uuid],
            |row| row.get(0),
        )?;
        self.sessions.insert(uuid.to_string(), id);
        Ok(id)
    }

    fn request(
        &mut self,
        request: &Request,
        account_id: Option<i64>,
        table: &PriceTable,
    ) -> Result<()> {
        let detail = &request.detail;
        let session_id = self.session(&request.session_id)?;
        let model = self.label(Some(&request.model))?;
        let requested_model = self.label(detail.requested_model.as_deref())?;
        let advisor_model = self.label(detail.advisor_model.as_deref())?;
        let effort = self.label(detail.effort.as_deref())?;
        let per_turn_effort = self.label(detail.per_turn_effort.as_deref())?;
        let speed = self.label(request.speed.as_deref())?;
        let inference_geo = self.label(request.inference_geo.as_deref())?;
        let service_tier = self.label(detail.service_tier.as_deref())?;
        let stop_reason = self.label(detail.stop_reason.as_deref())?;
        let entrypoint = self.label(detail.entrypoint.as_deref())?;
        let version = self.label(detail.version.as_deref())?;
        let user_type = self.label(detail.user_type.as_deref())?;
        let cwd = self.label(detail.cwd.as_deref())?;
        let git_branch = self.label(detail.git_branch.as_deref())?;
        let skill = self.label(detail.attribution_skill.as_deref())?;
        let agent = self.label(detail.attribution_agent.as_deref())?;
        let tools = detail
            .tools
            .iter()
            .map(|tool| self.label(Some(tool)))
            .collect::<Result<Vec<_>>>()?;
        let tokens = &request.tokens;
        let mut statement = self.transaction.prepare_cached(
            "INSERT INTO requests (
                request_id, session_id, account_id, time_ms, day,
                model_label, requested_model_label, advisor_model_label, effort_label,
                per_turn_effort_label, speed_label, inference_geo_label, service_tier_label,
                stop_reason_label, entrypoint_label, version_label, user_type_label,
                cwd_label, git_branch_label, skill_label, agent_label, agent_id, is_sidechain,
                input_tokens, output_tokens, thinking_tokens, cache_read_tokens,
                cache_write_5m_tokens, cache_write_1h_tokens, web_searches, web_fetches,
                thinking_duration_ms, cost_usd
             ) VALUES (
                ?1, ?2, ?3, CAST(round(unixepoch(?4, 'subsec') * 1000) AS INTEGER), date(?4, 'localtime'),
                ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32
             )
             ON CONFLICT (request_id) DO UPDATE SET
                stop_reason_label = excluded.stop_reason_label,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                thinking_tokens = excluded.thinking_tokens,
                cache_read_tokens = excluded.cache_read_tokens,
                cache_write_5m_tokens = excluded.cache_write_5m_tokens,
                cache_write_1h_tokens = excluded.cache_write_1h_tokens,
                web_searches = excluded.web_searches,
                web_fetches = excluded.web_fetches,
                thinking_duration_ms = excluded.thinking_duration_ms,
                cost_usd = excluded.cost_usd
             WHERE excluded.output_tokens > requests.output_tokens
             RETURNING day",
        )?;
        let mut returned = statement.query(params![
            request.request_id,
            session_id,
            account_id,
            detail.timestamp,
            model,
            requested_model,
            advisor_model,
            effort,
            per_turn_effort,
            speed,
            inference_geo,
            service_tier,
            stop_reason,
            entrypoint,
            version,
            user_type,
            cwd,
            git_branch,
            skill,
            agent,
            detail.agent_id,
            detail.is_sidechain,
            tokens.input as i64,
            tokens.output as i64,
            detail.thinking_tokens as i64,
            tokens.cache_read as i64,
            tokens.cache_write_5m as i64,
            tokens.cache_write_1h as i64,
            tokens.web_searches as i64,
            detail.web_fetches as i64,
            detail.thinking_duration_ms.map(|ms| ms as i64),
            table.price(request),
        ])?;
        if let Some(row) = returned.next()?
            && let Some(day) = row.get::<_, Option<String>>(0)?
        {
            self.changed_days.insert(day);
        }
        self.touched_sessions.insert(session_id);
        if let (Some(cwd_label), Some(cwd_text)) = (cwd, detail.cwd.as_deref()) {
            self.cwds.insert(cwd_label, cwd_text.to_string());
            if detail.git_branch.as_deref() == Some("HEAD") {
                self.head_requests
                    .entry(cwd_text.to_string())
                    .or_default()
                    .push(request.request_id.clone());
            }
        }
        let mut add_tool = self.transaction.prepare_cached(
            "INSERT OR IGNORE INTO request_tools (request_row_id, tool_label)
             SELECT id, ?2 FROM requests WHERE request_id = ?1",
        )?;
        for tool in tools {
            add_tool.execute(params![request.request_id, tool])?;
        }
        Ok(())
    }

    fn reattribute(&mut self, request_id: &str, account_id: Option<i64>) -> Result<bool> {
        let mut statement = self.transaction.prepare_cached(
            "UPDATE requests SET account_id = ?2
             WHERE request_id = ?1 AND account_id IS NOT ?2
             RETURNING day",
        )?;
        let mut moved = statement.query(params![request_id, account_id])?;
        let Some(row) = moved.next()? else {
            return Ok(false);
        };
        self.changed_days.insert(row.get(0)?);
        Ok(true)
    }

    fn drop_request(&mut self, request_id: &str, account_id: Option<i64>) -> Result<bool> {
        let mut statement = self.transaction.prepare_cached(
            "DELETE FROM requests WHERE request_id = ?1 AND account_id IS NOT ?2 RETURNING day",
        )?;
        let mut dropped = statement.query(params![request_id, account_id])?;
        let Some(row) = dropped.next()? else {
            return Ok(false);
        };
        self.changed_days.insert(row.get(0)?);
        Ok(true)
    }

    fn set_cost_aside(&mut self, uuid: &str) -> Result<()> {
        let session_id: Option<i64> = self
            .transaction
            .query_row("SELECT id FROM sessions WHERE uuid = ?1", [uuid], |row| {
                row.get(0)
            })
            .optional()?;
        self.touched_sessions.extend(session_id);
        self.transaction.execute(
            "DELETE FROM not_in_transcript
             WHERE session_id = (SELECT id FROM sessions WHERE uuid = ?1)",
            [uuid],
        )?;
        self.transaction.execute(
            "DELETE FROM session_models
             WHERE session_id = (SELECT id FROM sessions WHERE uuid = ?1)",
            [uuid],
        )?;
        self.transaction.execute(
            "UPDATE sessions SET total_cost_usd = NULL, cost_status = NULL, cost_set_aside = 1
             WHERE uuid = ?1",
            [uuid],
        )?;
        Ok(())
    }

    fn reclaim_session(&mut self, uuid: &str, account_id: i64) -> Result<()> {
        let session_id = self.session(uuid)?;
        self.transaction.execute(
            "UPDATE sessions SET account_id = ?2 WHERE id = ?1",
            params![session_id, account_id],
        )?;
        Ok(())
    }

    fn claim_session(&mut self, uuid: &str, account_id: i64) -> Result<()> {
        let session_id = self.session(uuid)?;
        self.transaction.execute(
            "UPDATE sessions SET account_id = ?2 WHERE id = ?1 AND account_id IS NULL",
            params![session_id, account_id],
        )?;
        Ok(())
    }

    fn resolve_places(&mut self, resolver: &Resolver) -> Result<()> {
        for (cwd_label, cwd) in std::mem::take(&mut self.cwds) {
            let known: bool = self.transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM directories WHERE cwd_label = ?1)",
                [cwd_label],
                |row| row.get(0),
            )?;
            if known {
                continue;
            }
            let (name, path, remote) = match resolver.place(Path::new(&cwd)) {
                Place::Repo { name, path, remote } => {
                    (name, Some(path.to_string_lossy().into_owned()), remote)
                }
                Place::Temporary => ("temporary".to_string(), None, None),
                Place::Unknown => continue,
                Place::Itself(path) => (
                    path.file_name()
                        .map_or_else(|| cwd.clone(), |name| name.to_string_lossy().into_owned()),
                    Some(cwd.clone()),
                    None,
                ),
            };
            self.transaction.execute(
                "INSERT INTO directories (cwd_label, repo_name, repo_path, remote_url)
                 VALUES (?1, ?2, ?3, ?4)",
                params![cwd_label, name, path, remote],
            )?;
        }
        for (cwd, request_ids) in std::mem::take(&mut self.head_requests) {
            let branch = resolver
                .head_branch(Path::new(&cwd))
                .unwrap_or_else(|| "detached".to_string());
            let branch_label = self.label(Some(&branch))?;
            let mut statement = self.transaction.prepare_cached(
                "UPDATE requests SET head_branch_label = ?2
                 WHERE request_id = ?1 AND head_branch_label IS NULL
                 RETURNING day",
            )?;
            for request_id in request_ids {
                let mut updated = statement.query(params![request_id, branch_label])?;
                if let Some(row) = updated.next()?
                    && let Some(day) = row.get::<_, Option<String>>(0)?
                {
                    self.changed_days.insert(day);
                }
            }
        }
        Ok(())
    }

    fn cost_state(&mut self, state: &CostState) -> Result<()> {
        let session_id = self.session(&state.session_id)?;
        self.touched_sessions.insert(session_id);
        let totals = &state.totals;
        let updated = self.transaction.execute(
            "UPDATE sessions SET
                total_cost_usd = ?2, start_time_ms = ?3, api_duration_ms = ?4,
                api_duration_without_retries_ms = ?5, tool_duration_ms = ?6, duration_ms = ?7,
                lines_added = ?8, lines_removed = ?9, has_unknown_model_cost = ?10
             WHERE id = ?1 AND NOT cost_forgotten AND NOT cost_set_aside
               AND (total_cost_usd IS NULL OR total_cost_usd <= ?2)",
            params![
                session_id,
                state.total_cost_usd,
                totals.start_time_ms,
                totals.api_duration_ms.map(|ms| ms as i64),
                totals.api_duration_without_retries_ms.map(|ms| ms as i64),
                totals.tool_duration_ms.map(|ms| ms as i64),
                totals.duration_ms.map(|ms| ms as i64),
                totals.lines_added.map(|n| n as i64),
                totals.lines_removed.map(|n| n as i64),
                totals.has_unknown_model_cost,
            ],
        )?;
        if updated == 0 {
            return Ok(());
        }
        self.transaction.execute(
            "DELETE FROM session_models WHERE session_id = ?1",
            [session_id],
        )?;
        for (model, cost) in &state.models {
            let model_label = self.label(Some(model))?;
            let tokens = &cost.tokens;
            self.transaction.execute(
                "INSERT INTO session_models (
                    session_id, model_label, input_tokens, output_tokens, thinking_tokens,
                    cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
                    has_cache_write_split, web_searches, web_fetches, cost_usd
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    session_id,
                    model_label,
                    tokens.input as i64,
                    tokens.output as i64,
                    cost.thinking_tokens as i64,
                    tokens.cache_read as i64,
                    tokens.cache_write_5m as i64,
                    tokens.cache_write_1h as i64,
                    cost.has_cache_write_split,
                    tokens.web_searches as i64,
                    cost.web_fetches as i64,
                    cost.cost_usd,
                ],
            )?;
        }
        Ok(())
    }

    fn title(&mut self, session_uuid: &str, title: &str) -> Result<()> {
        let session_id = self.session(session_uuid)?;
        self.transaction.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1",
            params![session_id, title],
        )?;
        Ok(())
    }
}

fn projects_prefix(directory: &str) -> String {
    format!(
        "{}/",
        Path::new(directory).join("projects").to_string_lossy()
    )
}

fn delete_positions_under(transaction: &Transaction, prefix: &str) -> Result<()> {
    transaction.execute(
        "UPDATE sessions SET cost_set_aside = 0
         WHERE cost_set_aside AND EXISTS (
             SELECT 1 FROM file_positions
             WHERE substr(path, 1, length(?1)) = ?1 AND instr(path, sessions.uuid) > 0
         )",
        [prefix],
    )?;
    transaction.execute(
        "UPDATE sessions SET cost_forgotten = 0, account_id = NULL
         WHERE cost_forgotten AND EXISTS (
             SELECT 1 FROM file_positions
             WHERE substr(path, 1, length(?1)) = ?1 AND instr(path, sessions.uuid) > 0
         )",
        [prefix],
    )?;
    transaction.execute(
        "DELETE FROM file_positions WHERE substr(path, 1, length(?1)) = ?1",
        [prefix],
    )?;
    Ok(())
}

fn read_again_if_reader_changed(connection: &mut Connection) -> Result<()> {
    let version = READER_VERSION.to_string();
    let transaction = connection.transaction()?;
    let stored: Option<String> = transaction
        .query_row(
            "SELECT value FROM store_settings WHERE key = 'reader_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if stored.as_deref() == Some(version.as_str()) {
        return Ok(());
    }
    transaction.execute("UPDATE file_positions SET offset = 0", [])?;
    transaction.execute(
        "INSERT INTO store_settings (key, value) VALUES ('reader_version', ?1)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [&version],
    )?;
    transaction.commit()?;
    Ok(())
}

fn logged_out_key(path: &str) -> String {
    format!("logged_out:{path}")
}

fn delete_unused_labels(transaction: &Transaction) -> Result<()> {
    transaction.execute(
        "DELETE FROM directories WHERE cwd_label NOT IN (
            SELECT cwd_label FROM requests WHERE cwd_label IS NOT NULL
            UNION SELECT cwd_label FROM daily_totals WHERE cwd_label IS NOT NULL
         )",
        [],
    )?;
    transaction.execute(
        "DELETE FROM labels WHERE id NOT IN (
            SELECT label FROM (
                SELECT model_label AS label FROM requests
                UNION SELECT requested_model_label FROM requests
                UNION SELECT advisor_model_label FROM requests
                UNION SELECT effort_label FROM requests
                UNION SELECT per_turn_effort_label FROM requests
                UNION SELECT speed_label FROM requests
                UNION SELECT inference_geo_label FROM requests
                UNION SELECT service_tier_label FROM requests
                UNION SELECT stop_reason_label FROM requests
                UNION SELECT entrypoint_label FROM requests
                UNION SELECT version_label FROM requests
                UNION SELECT user_type_label FROM requests
                UNION SELECT cwd_label FROM requests
                UNION SELECT git_branch_label FROM requests
                UNION SELECT head_branch_label FROM requests
                UNION SELECT skill_label FROM requests
                UNION SELECT agent_label FROM requests
                UNION SELECT tool_label FROM request_tools
                UNION SELECT model_label FROM session_models
                UNION SELECT cwd_label FROM daily_totals
                UNION SELECT branch_label FROM daily_totals
                UNION SELECT model_label FROM not_in_transcript
                UNION SELECT model_label FROM expired_requests
                UNION SELECT skill_label FROM daily_totals
                UNION SELECT agent_label FROM daily_totals
                UNION SELECT model_label FROM daily_totals
            )
            WHERE label IS NOT NULL
         )",
        [],
    )?;
    Ok(())
}

fn restrict_to_owner(database: &Path) -> std::io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = database.as_os_str().to_owned();
        name.push(suffix);
        match std::fs::set_permissions(&name, std::fs::Permissions::from_mode(0o600)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

fn session_account(
    transaction: &Transaction,
    batch: &HashMap<&str, Option<Account>>,
    session_id: &str,
    attribution: &Attribution,
) -> Result<Option<Account>> {
    if let Some(account) = batch.get(session_id) {
        return Ok(account.clone());
    }
    let stored = transaction
        .query_row(
            "SELECT accounts.account_uuid, accounts.organization_uuid
             FROM sessions
             JOIN accounts ON accounts.id = sessions.account_id
             WHERE sessions.uuid = ?1",
            [session_id],
            |row| {
                Ok(Account {
                    account_uuid: row.get(0)?,
                    organization_uuid: row.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(stored.or_else(|| attribution.current.clone()))
}

fn rebuild_daily_totals(transaction: &Transaction, day: &str) -> Result<()> {
    transaction.execute("DELETE FROM daily_totals WHERE day = ?1", [day])?;
    transaction.execute(
        "INSERT INTO daily_totals (
            day, account_id, cwd_label, branch_label, skill_label, agent_label, model_label,
            requests, input_tokens, output_tokens, thinking_tokens, cache_read_tokens,
            cache_write_5m_tokens, cache_write_1h_tokens, web_searches, web_fetches, cost_usd
         )
         SELECT day, account_id, cwd_label, coalesce(head_branch_label, git_branch_label),
                skill_label, agent_label, model_label, count(*),
                sum(input_tokens), sum(output_tokens), sum(thinking_tokens), sum(cache_read_tokens),
                sum(cache_write_5m_tokens), sum(cache_write_1h_tokens), sum(web_searches),
                sum(web_fetches), sum(cost_usd)
         FROM requests
         WHERE day = ?1
         GROUP BY account_id, cwd_label, coalesce(head_branch_label, git_branch_label),
                  skill_label, agent_label, model_label",
        [day],
    )?;
    Ok(())
}

fn save_position(transaction: &Transaction, position: &FilePosition) -> Result<()> {
    transaction.execute(
        "INSERT INTO file_positions (path, device, inode, first_line_hash, offset)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (path) DO UPDATE SET
            device = excluded.device, inode = excluded.inode,
            first_line_hash = excluded.first_line_hash, offset = excluded.offset",
        params![
            position.path,
            position.device as i64,
            position.inode as i64,
            position.first_line_hash.map(|hash| hash as i64),
            position.offset as i64,
        ],
    )?;
    Ok(())
}
