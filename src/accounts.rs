use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::{AccountSettings, Config, Switch, home_relative};
use crate::ingest::{self, Summary, ingest_tree};
use crate::pricing::PriceTable;
use crate::sources::claude_json::{Account, Login, read_login};
use crate::sources::git::Resolver;
use crate::store::{self, LoginChange, SeenLogin, Store};

#[derive(Debug, Clone, Default)]
pub struct Attribution {
    pub current: Option<Account>,
    pub switch: Option<Switch>,
    pub seen_logins: Vec<SeenLogin>,
    pub untracked: HashSet<Account>,
    pub without_titles: HashSet<Account>,
}

impl Attribution {
    pub fn single(account: Option<Account>) -> Self {
        Attribution {
            current: account,
            ..Attribution::default()
        }
    }

    pub fn tracks(&self, account: Option<&Account>) -> bool {
        account.is_none_or(|account| !self.untracked.contains(account))
    }

    pub fn stores_titles(&self, account: Option<&Account>) -> bool {
        account.is_none_or(|account| !self.without_titles.contains(account))
    }

    pub fn reads_anything(&self) -> bool {
        self.tracks(self.current.as_ref())
            || self
                .switch
                .as_ref()
                .is_some_and(|switch| self.tracks(Some(&switch.earlier)))
            || self
                .seen_logins
                .iter()
                .any(|login| self.tracks(Some(&login.account)))
    }

    pub fn account_at(&self, time_ms: Option<i64>) -> Option<&Account> {
        let Some(time_ms) = time_ms else {
            return self.current.as_ref();
        };
        let Some(switch) = &self.switch else {
            return self
                .seen_logins
                .iter()
                .rev()
                .find(|login| login.from_ms <= time_ms)
                .map(|login| &login.account)
                .or(self.current.as_ref());
        };
        if time_ms < switch.at_ms {
            return Some(&switch.earlier);
        }
        let running_at_switch = self
            .seen_logins
            .iter()
            .rev()
            .find(|login| login.from_ms <= switch.at_ms)
            .filter(|login| login.account != switch.earlier);
        let mut started_after_switch = self
            .seen_logins
            .iter()
            .filter(|login| login.from_ms > switch.at_ms);
        started_after_switch
            .clone()
            .rev()
            .find(|login| login.from_ms <= time_ms)
            .or(running_at_switch)
            .or_else(|| started_after_switch.next())
            .map(|login| &login.account)
            .or(self.current.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    pub path: PathBuf,
    pub claude_json: PathBuf,
    pub login: Option<Login>,
    pub unreadable: Option<String>,
}

impl Directory {
    fn read(path: PathBuf, claude_json: &Path) -> Directory {
        let (login, unreadable) = match read_login(claude_json) {
            Ok(login) => (login, None),
            Err(error) => (None, Some(format!("{}: {error}", claude_json.display()))),
        };
        Directory {
            path,
            claude_json: claude_json.to_path_buf(),
            login,
            unreadable,
        }
    }

    pub fn projects(&self) -> PathBuf {
        self.path.join("projects")
    }

    // Claude Code reads ~/.claude.json only when CLAUDE_CONFIG_DIR is unset.
    pub fn claude_config_dir(&self) -> Option<&Path> {
        self.claude_json
            .starts_with(&self.path)
            .then_some(self.path.as_path())
    }

    pub fn account(&self) -> Option<&Account> {
        self.login.as_ref().map(|login| &login.account)
    }
}

pub fn discover(home: &Path, config: &Config) -> Vec<Directory> {
    let mut found: Vec<Directory> = Vec::new();
    let mut add = |directory: Directory| {
        if !found
            .iter()
            .any(|seen| same_directory(&seen.path, &directory.path))
        {
            found.push(directory);
        }
    };
    let default = home.join(".claude");
    if default.is_dir() {
        add(Directory::read(default, &home.join(".claude.json")));
    }
    for path in config_dirs_in(home) {
        let claude_json = path.join(".claude.json");
        add(Directory::read(path, &claude_json));
    }
    for path in config.extra_paths.iter().filter(|path| path.is_dir()) {
        add(Directory::read(path.clone(), &path.join(".claude.json")));
    }
    found
}

fn config_dirs_in(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("projects").is_dir() && path.join(".claude.json").is_file())
        .collect();
    paths.sort();
    paths
}

fn same_directory(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub fn attribution_for(path: &Path, directories: &[Directory], config: &Config) -> Attribution {
    let current = directories
        .iter()
        .find(|directory| same_directory(&directory.path, path))
        .and_then(|directory| directory.account().cloned());
    let switch = config.switch(path).cloned();
    let with_setting = |off: fn(&AccountSettings) -> bool| -> HashSet<Account> {
        config
            .configured_accounts()
            .filter(|account| off(&config.account(account)))
            .cloned()
            .collect()
    };
    Attribution {
        untracked: with_setting(|settings| !settings.tracking),
        without_titles: with_setting(|settings| !settings.store_titles),
        current,
        switch,
        seen_logins: Vec::new(),
    }
}

pub fn ingest(
    store: &mut Store,
    directories: &[Directory],
    config: &Config,
    table: &PriceTable,
    resolver: &Resolver,
) -> Result<Summary, ingest::Error> {
    let mut total = Summary::default();
    let mut first_error = None;
    let now_ms = now_ms();
    let mut kept_folders: Vec<PathBuf> = directories
        .iter()
        .filter(|directory| directory.unreadable.is_some())
        .map(Directory::projects)
        .collect();
    for directory in directories
        .iter()
        .filter(|directory| directory.unreadable.is_none())
    {
        let read = ingest_directory(
            store,
            directory,
            directories,
            config,
            table,
            resolver,
            now_ms,
            None,
        );
        match read {
            Ok(Some(summary)) => {
                kept_folders.push(directory.projects());
                total.add(summary);
            }
            Ok(None) => {}
            Err(error) => {
                kept_folders.push(directory.projects());
                first_error.get_or_insert(error);
            }
        }
    }
    store.forget_unrecognized_outside_folders(&kept_folders)?;
    first_error.map_or(Ok(total), Err)
}

pub fn ingest_files(
    store: &mut Store,
    directories: &[Directory],
    config: &Config,
    table: &PriceTable,
    resolver: &Resolver,
    files: &[PathBuf],
) -> Result<Summary, ingest::Error> {
    let mut total = Summary::default();
    let mut first_error = None;
    let now_ms = now_ms();
    for directory in directories
        .iter()
        .filter(|directory| directory.unreadable.is_none())
    {
        let inside = files_inside(&directory.projects(), files);
        if inside.is_empty() {
            continue;
        }
        let read = ingest_directory(
            store,
            directory,
            directories,
            config,
            table,
            resolver,
            now_ms,
            Some(&inside),
        );
        match read {
            Ok(Some(summary)) => total.add(summary),
            Ok(None) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(total), Err)
}

pub fn has_files_inside(projects: &Path, files: &[PathBuf]) -> bool {
    !files_inside(projects, files).is_empty()
}

fn files_inside(projects: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let canonical = projects.canonicalize().ok();
    files
        .iter()
        .filter_map(|file| {
            let relative = file
                .strip_prefix(projects)
                .ok()
                .or_else(|| file.strip_prefix(canonical.as_ref()?).ok())?;
            Some(projects.join(relative))
        })
        .filter(|file| file.is_file())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn ingest_directory(
    store: &mut Store,
    directory: &Directory,
    directories: &[Directory],
    config: &Config,
    table: &PriceTable,
    resolver: &Resolver,
    now_ms: i64,
    only: Option<&[PathBuf]>,
) -> Result<Option<Summary>, ingest::Error> {
    let mut attribution = attribution_for(&directory.path, directories, config);
    let path = directory.path.to_string_lossy();
    if let Some(login) = &directory.login {
        let recorded = Login {
            account: login.account.clone(),
            email: login
                .email
                .clone()
                .filter(|_| attribution.tracks(Some(&login.account))),
        };
        store.record_login(&path, &recorded, now_ms)?;
    } else {
        store.record_logged_out(&path)?;
    }
    attribution.seen_logins = store.seen_logins(&path)?;
    let projects = directory.projects();
    let read_again =
        store.read_again_if_switch_changed(&path, &projects, config.switch(&directory.path))?;
    if !attribution.reads_anything() || !projects.is_dir() {
        return Ok(None);
    }
    let summary = match only {
        Some(files) if !read_again => {
            let mut summary = Summary::default();
            let mut first_error = None;
            for file in files {
                match ingest_tree(store, file, table, &attribution, resolver) {
                    Ok(read) => summary.add(read),
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
            summary
        }
        _ => ingest_tree(store, &projects, table, &attribution, resolver)?,
    };
    if read_again {
        store.delete_emptied_sessions(&attribution.untracked)?;
    }
    Ok(Some(summary))
}

pub fn read_again_for(
    store: &mut Store,
    account: &Account,
    directories: &[Directory],
    config: &Config,
) -> store::Result<()> {
    for directory in directories {
        let configured_earlier = config.switch(&directory.path).map(|switch| &switch.earlier);
        let seen = store
            .seen_logins(&directory.path.to_string_lossy())?
            .iter()
            .any(|login| &login.account == account);
        if directory.account() == Some(account) || configured_earlier == Some(account) || seen {
            store.forget_positions_under(&directory.projects())?;
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub account: Account,
    pub label: String,
    pub tracking: bool,
    pub directories: Vec<PathBuf>,
    pub requests: u64,
}

pub fn list(
    store: &Store,
    directories: &[Directory],
    config: &Config,
) -> store::Result<Vec<AccountRow>> {
    let mut emails: BTreeMap<Account, Option<String>> = BTreeMap::new();
    let mut requests: BTreeMap<Account, u64> = BTreeMap::new();
    for stored in store.accounts()? {
        requests.insert(stored.account.clone(), stored.requests);
        emails.insert(stored.account, stored.email);
    }
    for directory in directories {
        if let Some(login) = &directory.login {
            emails.insert(login.account.clone(), login.email.clone());
        }
    }
    for account in config.configured_accounts() {
        emails.entry(account.clone()).or_default();
    }
    let rows = emails
        .into_iter()
        .map(|(account, email)| {
            let settings = config.account(&account);
            AccountRow {
                label: settings
                    .label
                    .or(email)
                    .unwrap_or_else(|| account.to_string()),
                tracking: settings.tracking,
                directories: directories
                    .iter()
                    .filter(|directory| directory.account() == Some(&account))
                    .map(|directory| directory.path.clone())
                    .collect(),
                requests: requests.get(&account).copied().unwrap_or(0),
                account,
            }
        })
        .collect();
    Ok(rows)
}

pub fn render(
    rows: &[AccountRow],
    changes: &[LoginChange],
    directories: &[Directory],
    home: &Path,
) -> String {
    let mut out = String::new();
    for row in rows {
        let tracking = if row.tracking {
            "tracked"
        } else {
            "not tracked"
        };
        let _ = writeln!(out, "{}, {tracking}, {} requests", row.label, row.requests);
        let _ = writeln!(out, "  ID {}", row.account);
        if row.directories.is_empty() {
            let _ = writeln!(out, "  No config directory here is logged in to it now");
        }
        for path in &row.directories {
            let _ = writeln!(out, "  {}", home_relative(path, home));
        }
        let _ = writeln!(out);
    }
    for directory in directories
        .iter()
        .filter(|directory| directory.login.is_none() && directory.unreadable.is_none())
    {
        let _ = writeln!(
            out,
            "{} isn't logged in. Its requests are stored without an account.",
            home_relative(&directory.path, home)
        );
    }
    let label = |account: &Account| {
        rows.iter()
            .find(|row| &row.account == account)
            .map_or_else(|| account.to_string(), |row| row.label.clone())
    };
    for change in changes {
        let _ = writeln!(
            out,
            "{} changed login from {} to {}. Requests from before Gigawatts saw the change stay under {}. To split at the exact time, set switched_at and earlier_account for this directory in config.toml.",
            home_relative(Path::new(&change.path), home),
            label(&change.previous),
            label(&change.current),
            label(&change.previous)
        );
    }
    out
}
