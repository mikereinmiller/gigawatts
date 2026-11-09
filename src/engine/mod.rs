pub mod control;
pub mod lock;
pub mod scheduler;
pub mod snapshot;
pub mod watcher;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::accounts::{self, Directory};
use crate::config::Config;
use crate::contract::{Problem, ProblemKind};
use crate::ingest::{self, Summary};
use crate::limits::{self, Refresher};
use crate::sources::claude_json::Account;
use crate::sources::get_usage::Claude;
use crate::sources::git::Resolver;
use crate::sources::transcript::{READER_VERSION, transcript_files};
use crate::store::{self, Store};
use control::{Command, Handshake, Request};
use lock::EngineLock;
use scheduler::{Schedule, Work, next_deadline};
use watcher::{Change, Watcher};

pub fn version() -> String {
    format!(
        "{}+r{READER_VERSION}.m{}",
        env!("CARGO_PKG_VERSION"),
        store::MIGRATIONS.len()
    )
}

const REFRESHED: &str = "refreshed";
const FAILED: &str = "failed: ";
const START_WAIT: Duration = Duration::from_secs(10);
const CLAUDE_ANSWER: Duration = Duration::from_secs(60);
const STOP_WAIT: Duration = Duration::from_secs(10);

pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn new(home: &Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
        }
    }

    pub fn config(&self) -> PathBuf {
        self.home
            .join(".config")
            .join("gigawatts")
            .join("config.toml")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.home.join(".local").join("share").join("gigawatts")
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir().join("history.sqlite3")
    }

    pub fn snapshot(&self) -> PathBuf {
        self.data_dir().join("snapshot.json")
    }

    pub fn announced(&self) -> PathBuf {
        self.data_dir().join("announced.json")
    }

    pub fn lock(&self) -> PathBuf {
        self.data_dir().join("engine.lock")
    }

    pub fn socket(&self) -> PathBuf {
        self.data_dir().join("engine.sock")
    }

    pub fn log(&self) -> PathBuf {
        self.data_dir().join("engine.log")
    }

    pub fn resolver(&self) -> Resolver {
        Resolver {
            capture_runs: Some(self.home.join("gigawatts-enterprise-capture").join("runs")),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Store(store::Error),
    Config(crate::config::Error),
    Watch(notify::Error),
    EngineNewer { engine: String },
    NoAnswer { log: PathBuf },
    Exited { log: PathBuf },
    StillRunning { pid: String },
    RefreshFailed { reply: String },
    LimitsFailed { reply: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "{error}"),
            Error::Store(error) => write!(f, "history database: {error}"),
            Error::Config(error) => write!(f, "config: {error}"),
            Error::Watch(error) => write!(f, "can't watch transcripts: {error}"),
            Error::EngineNewer { engine } => write!(
                f,
                "the running engine is version {engine}, newer than this command's {}. Reopen with the newer gigawatts",
                version()
            ),
            Error::NoAnswer { log } => write!(
                f,
                "the engine started but didn't answer within {} seconds. Its log is {}",
                START_WAIT.as_secs(),
                log.display()
            ),
            Error::Exited { log } => {
                write!(
                    f,
                    "the engine exited as it started. Its log is {}",
                    log.display()
                )
            }
            Error::LimitsFailed { reply } => {
                write!(f, "the engine couldn't refresh limits: {reply}")
            }
            Error::RefreshFailed { reply } => {
                write!(f, "the engine didn't finish reading transcripts: {reply}")
            }
            Error::StillRunning { pid } => write!(
                f,
                "the engine (process {pid}) holds the lock but didn't stop within {} seconds",
                STOP_WAIT.as_secs()
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        Error::Store(error)
    }
}

impl From<crate::config::Error> for Error {
    fn from(error: crate::config::Error) -> Self {
        Error::Config(error)
    }
}

impl From<notify::Error> for Error {
    fn from(error: notify::Error) -> Self {
        Error::Watch(error)
    }
}

pub fn read_all(
    store: &mut Store,
    directories: &[Directory],
    config: &Config,
    resolver: &Resolver,
) -> Result<Summary, ingest::Error> {
    let table = config.price_table();
    store.reprice_if_changed(&table)?;
    accounts::ingest(store, directories, config, &table, resolver)
}

pub enum Outcome {
    AlreadyRunning,
}

enum Message {
    Files(notify::Result<notify::Event>),
    Control(Request),
}

pub fn run(paths: &Paths) -> Result<Outcome, Error> {
    std::fs::create_dir_all(paths.data_dir())?;
    let Some(_lock) = EngineLock::acquire(&paths.lock())? else {
        return Ok(Outcome::AlreadyRunning);
    };
    let mut store = Store::open(&paths.database())?;
    let config = Config::load(&paths.config(), &paths.home)?;
    let (sender, receiver) = mpsc::channel();
    let watch_sender = sender.clone();
    let watcher = Watcher::new(move |event| {
        let _ = watch_sender.send(Message::Files(event));
    })?;
    let control_sender = sender;
    control::listen(&paths.socket(), version(), move |request| {
        let _ = control_sender.send(Message::Control(request));
    })?;
    let started = Instant::now();
    let mut engine = Engine {
        paths,
        config,
        started,
        claude: Claude {
            program: PathBuf::from("claude"),
            working_dir: paths.data_dir(),
            timeout: CLAUDE_ANSWER,
        },
        refresher: Refresher::default(),
        watcher,
        schedule: Schedule::started_at(started.elapsed()),
        changed: BTreeSet::new(),
        watch_problems: BTreeMap::new(),
        login_problems: BTreeMap::new(),
        usage_problems: BTreeMap::new(),
        read_problem: None,
        requests_expired: None,
        cleanup_problem: None,
    };
    engine.full_scan(&mut store, started.elapsed());
    loop {
        let now = started.elapsed();
        let message = match next_deadline(&engine.schedule, now) {
            Some(deadline) if deadline.at <= now => {
                engine.work(&mut store, deadline.work, now);
                continue;
            }
            Some(deadline) => match receiver.recv_timeout(deadline.at - now) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => unreachable!("the engine holds a sender"),
            },
            None => receiver.recv().expect("the engine holds a sender"),
        };
        match message {
            Message::Files(Ok(event)) => match watcher::classify(&event) {
                Change::Paths(paths) => engine.files_changed(paths, started.elapsed()),
                Change::FullScan => engine.full_scan(&mut store, started.elapsed()),
                Change::Removed(paths) => {
                    engine.watcher.forget(&paths);
                    engine.full_scan(&mut store, started.elapsed());
                }
                Change::Ignored => {}
            },
            Message::Files(Err(error)) => {
                eprintln!("gigawatts engine: the watcher reported {error}; scanning everything");
                engine.full_scan(&mut store, started.elapsed());
            }
            Message::Control(request) if request.command == Command::LimitsLater => {
                engine.refresh_requested_limits(&mut store);
                engine.write_snapshot(&store);
            }
            Message::Control(request) if request.command == Command::Limits => {
                let reply = match engine.refresh_requested_limits(&mut store) {
                    Some(failed) => format!("{FAILED}{failed}"),
                    None => REFRESHED.to_string(),
                };
                engine.write_snapshot(&store);
                let _ = request.reply.send(reply);
            }
            Message::Control(request) => {
                engine.full_scan(&mut store, started.elapsed());
                let reply = match &engine.read_problem {
                    Some(problem) => format!("{FAILED}{}", problem.cause),
                    None => REFRESHED.to_string(),
                };
                let _ = request.reply.send(reply);
            }
        }
    }
}

struct Engine<'a> {
    paths: &'a Paths,
    config: Config,
    started: Instant,
    claude: Claude,
    refresher: Refresher,
    watcher: Watcher,
    schedule: Schedule,
    changed: BTreeSet<PathBuf>,
    watch_problems: BTreeMap<PathBuf, Problem>,
    login_problems: BTreeMap<PathBuf, Problem>,
    usage_problems: BTreeMap<PathBuf, Problem>,
    read_problem: Option<Problem>,
    requests_expired: Option<(String, u32)>,
    cleanup_problem: Option<Problem>,
}

impl Engine<'_> {
    fn files_changed(&mut self, paths: Vec<PathBuf>, now: Duration) {
        self.changed.extend(paths);
        self.schedule.file_event(now);
    }

    fn work(&mut self, store: &mut Store, work: Work, now: Duration) {
        match work {
            Work::ReadChangedFiles => self.read_changed(store),
            Work::SafetyScan => self.full_scan(store, now),
            Work::RefreshLimits => {
                let directories = accounts::discover(&self.paths.home, &self.config);
                self.refresh_due_limits(store, &directories);
                self.write_snapshot(store);
            }
        }
    }

    fn read_changed(&mut self, store: &mut Store) {
        self.schedule.read_changed_files();
        self.reload_config();
        let directories = accounts::discover(&self.paths.home, &self.config);
        self.note_unreadable_logins(store, &directories);
        let projects: Vec<PathBuf> = directories
            .iter()
            .map(Directory::projects)
            .flat_map(|projects| [projects.canonicalize().ok(), Some(projects)])
            .flatten()
            .collect();
        let mut files = Vec::new();
        for path in std::mem::take(&mut self.changed) {
            if !projects.iter().any(|projects| path.starts_with(projects)) {
                continue;
            }
            if path.is_dir() {
                files.extend(
                    transcript_files(&path)
                        .map(|listed| listed.files)
                        .unwrap_or_default(),
                );
            } else {
                files.push(path);
            }
        }
        let table = self.config.price_table();
        let read = store
            .reprice_if_changed(&table)
            .map_err(ingest::Error::from)
            .and_then(|_| {
                accounts::ingest_files(
                    store,
                    &directories,
                    &self.config,
                    &table,
                    &self.paths.resolver(),
                    &files,
                )
            });
        if let Err(error) = read {
            eprintln!("gigawatts engine: can't read changed transcripts: {error}");
            self.read_problem = Some(read_failing(
                store,
                self.read_problem.take(),
                &error,
                &directories,
            ));
        }
        self.note_activity(store, &directories, &files);
        self.refresh_due_limits(store, &directories);
        self.expire_requests_once_a_day(store);
        self.write_snapshot(store);
    }

    fn note_activity(&mut self, store: &Store, directories: &[Directory], files: &[PathBuf]) {
        let now_ms = now_ms();
        let fetched: BTreeMap<Account, i64> = store
            .latest_limits()
            .unwrap_or_default()
            .into_iter()
            .map(|(account, reading)| (account, reading.fetched_at_ms))
            .collect();
        for directory in directories {
            let Some(account) = self.refreshable(directory) else {
                continue;
            };
            if accounts::has_files_inside(&directory.projects(), files) {
                self.refresher
                    .activity(account, now_ms, fetched.get(account).copied());
            }
        }
    }

    fn refreshable<'d>(&self, directory: &'d Directory) -> Option<&'d Account> {
        directory
            .account()
            .filter(|account| self.config.account(account).tracking)
    }

    fn forget_unrefreshable(&mut self, directories: &[Directory]) {
        let refreshable: BTreeSet<Account> = directories
            .iter()
            .filter_map(|directory| self.refreshable(directory))
            .cloned()
            .collect();
        self.refresher.keep_only(&refreshable);
    }

    fn refresh_due_limits(&mut self, store: &mut Store, directories: &[Directory]) {
        let now_ms = now_ms();
        self.forget_unrefreshable(directories);
        for account in self.refresher.due(now_ms) {
            let _ = self.refresh_limits(store, directories, &account, now_ms);
        }
        let due = self.refresher.next_due_ms(now_ms).map(|due_ms| {
            self.started.elapsed() + Duration::from_millis((due_ms - now_ms).max(0) as u64)
        });
        self.schedule.limits_due(due);
    }

    fn refresh_requested_limits(&mut self, store: &mut Store) -> Option<String> {
        let now_ms = now_ms();
        self.reload_config();
        let directories = accounts::discover(&self.paths.home, &self.config);
        self.forget_unrefreshable(&directories);
        let fresh: BTreeSet<Account> = store
            .latest_limits()
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, reading)| now_ms - reading.fetched_at_ms < limits::REFRESH_INTERVAL_MS)
            .map(|(account, _)| account)
            .collect();
        let stale: BTreeSet<Account> = directories
            .iter()
            .filter_map(|directory| self.refreshable(directory))
            .filter(|account| !fresh.contains(*account))
            .cloned()
            .collect();
        let failing: BTreeMap<Account, String> = self
            .refresher
            .failing()
            .into_iter()
            .map(|failing| (failing.account, failing.error))
            .collect();
        let mut first_failure = None;
        for account in stale {
            let refreshed = if self.refresher.attempted_within_interval(&account, now_ms) {
                failing.get(&account).cloned().map_or(Ok(()), Err)
            } else {
                self.refresh_limits(store, &directories, &account, now_ms)
            };
            if let Err(error) = refreshed {
                first_failure.get_or_insert(error);
            }
        }
        first_failure
    }

    fn refresh_limits(
        &mut self,
        store: &mut Store,
        directories: &[Directory],
        account: &Account,
        now_ms: i64,
    ) -> Result<(), String> {
        let Some(directory) = directories
            .iter()
            .find(|directory| self.refreshable(directory) == Some(account))
        else {
            return Ok(());
        };
        let refreshed = store
            .local_today()
            .map_err(limits::RefreshError::from)
            .and_then(|today| {
                limits::refresh(
                    store,
                    &self.claude,
                    directory,
                    &crate::engine::now_ms,
                    &today,
                )
            });
        match refreshed {
            Ok(()) => {
                self.refresher.succeeded(account, now_ms);
                Ok(())
            }
            Err(error) => {
                let error = error.to_string();
                eprintln!("gigawatts engine: can't refresh limits for {account}: {error}");
                self.refresher.failed(account, now_ms, &error);
                Err(error)
            }
        }
    }

    fn full_scan(&mut self, store: &mut Store, now: Duration) {
        self.changed.clear();
        self.schedule.scanned(now);
        self.reload_config();
        let directories = accounts::discover(&self.paths.home, &self.config);
        self.note_unreadable_logins(store, &directories);
        self.forget_unrefreshable(&directories);
        let previous_watch_problems = std::mem::take(&mut self.watch_problems);
        let mut watched: Vec<PathBuf> = directories.iter().map(Directory::projects).collect();
        let default_projects = self.paths.home.join(".claude").join("projects");
        if !watched.contains(&default_projects) {
            watched.push(default_projects);
        }
        for projects in watched {
            if let Err(error) = self.watcher.watch(&projects) {
                eprintln!(
                    "gigawatts engine: can't watch {}: {error}",
                    projects.display()
                );
                let account = directories
                    .iter()
                    .find(|directory| directory.projects() == projects)
                    .and_then(Directory::account);
                let problem = watch_failing(
                    store,
                    previous_watch_problems.get(&projects),
                    &projects,
                    account,
                    &error,
                );
                self.watch_problems.insert(projects, problem);
            }
        }
        let previous_read_problem = self.read_problem.take();
        let read = read_all(store, &directories, &self.config, &self.paths.resolver());
        let cached = self
            .read_cached_limits(store, &directories)
            .map_err(ingest::Error::from);
        if let Err(error) = cached.and(read) {
            eprintln!("gigawatts engine: can't read transcripts: {error}");
            self.read_problem = Some(read_failing(
                store,
                previous_read_problem,
                &error,
                &directories,
            ));
        }
        self.expire_requests_once_a_day(store);
        self.write_snapshot(store);
    }

    fn expire_requests_once_a_day(&mut self, store: &mut Store) {
        let Some(days) = self.config.request_detail_days else {
            return;
        };
        let today = match store.local_today() {
            Ok(today) => today,
            Err(error) => {
                eprintln!("gigawatts engine: can't tell today's date for cleanup: {error}");
                return;
            }
        };
        let done = (today, days);
        if self.requests_expired.as_ref() == Some(&done) {
            return;
        }
        match store.expire_requests(days, &done.0) {
            Ok(_) => {
                self.requests_expired = Some(done);
                self.cleanup_problem = None;
            }
            Err(error) => {
                eprintln!(
                    "gigawatts engine: can't delete request detail older than {days} days: {error}"
                );
                let previous = self.cleanup_problem.take();
                self.cleanup_problem = Some(cleanup_failing(store, previous, days, &error));
            }
        }
    }

    fn read_cached_limits(
        &mut self,
        store: &mut Store,
        directories: &[Directory],
    ) -> store::Result<()> {
        let unreadable = limits::read_cached(store, directories, &self.config)?;
        let previous = std::mem::take(&mut self.usage_problems);
        for unreadable in unreadable {
            let account = directories
                .iter()
                .find(|directory| directory.claude_json == unreadable.claude_json)
                .and_then(Directory::account);
            let problem = usage_unreadable(
                store,
                previous.get(&unreadable.claude_json),
                &unreadable,
                account,
            );
            self.usage_problems.insert(unreadable.claude_json, problem);
        }
        Ok(())
    }

    fn note_unreadable_logins(&mut self, store: &Store, directories: &[Directory]) {
        let previous = std::mem::take(&mut self.login_problems);
        for directory in directories {
            if let Some(unreadable) = &directory.unreadable {
                let problem =
                    login_unreadable(store, previous.get(&directory.path), directory, unreadable);
                self.login_problems.insert(directory.path.clone(), problem);
            }
        }
    }

    fn reload_config(&mut self) {
        match Config::load(&self.paths.config(), &self.paths.home) {
            Ok(config) => self.config = config,
            Err(error) => eprintln!(
                "gigawatts engine: can't read {}, keeping the settings read before: {error}",
                self.paths.config().display()
            ),
        }
    }

    fn write_snapshot(&self, store: &Store) {
        let engine_problems: Vec<Problem> = self
            .watch_problems
            .values()
            .chain(self.login_problems.values())
            .chain(self.usage_problems.values())
            .chain(&self.read_problem)
            .chain(&self.cleanup_problem)
            .cloned()
            .chain(self.refresh_problems(store))
            .collect();
        let written = snapshot::build(store, &self.config, now_ms(), &engine_problems)
            .map_err(|error| error.to_string())
            .and_then(|snapshot| {
                snapshot::write(&self.paths.snapshot(), &snapshot)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = written {
            eprintln!("gigawatts engine: can't write the snapshot: {error}");
        }
    }
}

impl Engine<'_> {
    fn refresh_problems(&self, store: &Store) -> Vec<Problem> {
        let labels: BTreeMap<Account, Option<String>> = store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .map(|stored| (stored.account, stored.email))
            .collect();
        let latest: BTreeMap<Account, i64> = store
            .latest_limits()
            .unwrap_or_default()
            .into_iter()
            .map(|(account, reading)| (account, reading.fetched_at_ms))
            .collect();
        self.refresher
            .failing()
            .into_iter()
            .map(|failing| {
                let label = self
                    .config
                    .account(&failing.account)
                    .label
                    .or_else(|| labels.get(&failing.account).cloned().flatten())
                    .unwrap_or_else(|| failing.account.to_string());
                let shown = match latest.get(&failing.account) {
                    Some(&fetched_at_ms) => format!(
                        "showing the reading from {}",
                        store
                            .local_datetime(fetched_at_ms)
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| "earlier".to_string())
                    ),
                    None => "no reading yet".to_string(),
                };
                Problem {
                    kind: ProblemKind::RefreshFailing,
                    affects: format!("limits for {label}: {shown}"),
                    since: store.local_datetime(failing.since_ms).ok().flatten(),
                    cause: format!("Gigawatts can't refresh limits through `claude -p`: {}", failing.error),
                    fix: "Check that `claude` runs from the engine's PATH and is logged in to this account. Gigawatts tries again, waiting longer after each failure, while Claude Code is active".to_string(),
                    account: Some(failing.account.to_string()),
                }
            })
            .collect()
    }
}

fn watch_failing(
    store: &Store,
    previous: Option<&Problem>,
    projects: &Path,
    account: Option<&Account>,
    error: &notify::Error,
) -> Problem {
    Problem {
        kind: ProblemKind::WatchFailing,
        affects: format!(
            "today's spend and history: new requests in {} show up only after a refresh",
            projects.display()
        ),
        since: since(store, previous),
        cause: format!("Gigawatts can't watch {}: {error}", projects.display()),
        fix: "On Linux, raise fs.inotify.max_user_watches. Otherwise check the folder's permissions. Then run `gigawatts ingest`".to_string(),
        account: account.map(Account::to_string),
    }
}

fn login_unreadable(
    store: &Store,
    previous: Option<&Problem>,
    directory: &Directory,
    unreadable: &str,
) -> Problem {
    Problem {
        kind: ProblemKind::ReadFailing,
        affects: format!(
            "today's spend and history: requests in {} aren't read",
            directory.projects().display()
        ),
        since: since(store, previous),
        cause: format!("Gigawatts can't read the login in {unreadable}"),
        fix: "Check that the file is readable and holds valid JSON. Then run `gigawatts ingest`"
            .to_string(),
        account: None,
    }
}

fn usage_unreadable(
    store: &Store,
    previous: Option<&Problem>,
    unreadable: &limits::UnreadableUsage,
    account: Option<&Account>,
) -> Problem {
    let claude_json = unreadable.claude_json.display();
    Problem {
        kind: ProblemKind::ReadFailing,
        affects: format!("limits from {claude_json}: the saved reading isn't updated"),
        since: since(store, previous),
        cause: format!(
            "Gigawatts can't read `cachedUsageUtilization` in {claude_json}: {}",
            unreadable.error
        ),
        fix: "Claude Code may have changed this field's format. Gigawatts keeps refreshing limits through `claude -p` while Claude Code is active, and reads the file again at the next scan".to_string(),
        account: account.map(Account::to_string),
    }
}

fn read_failing(
    store: &Store,
    previous: Option<Problem>,
    error: &ingest::Error,
    directories: &[Directory],
) -> Problem {
    let account = match error {
        ingest::Error::Io(path, _) | ingest::Error::Folder(path, _) => directories
            .iter()
            .find(|directory| path.starts_with(&directory.path))
            .and_then(Directory::account),
        ingest::Error::Store(_) => None,
    };
    let (affects, cause, fix) = match error {
        ingest::Error::Io(..) => (
            "today's spend and history: requests in that transcript are missing",
            format!("Gigawatts can't read a transcript: {error}"),
            "Check the file's permissions. Then run `gigawatts ingest`",
        ),
        ingest::Error::Folder(..) => (
            "today's spend and history: requests in transcripts in that folder are missing",
            format!("Gigawatts can't open a folder of transcripts: {error}"),
            "Check the folder's permissions. Then run `gigawatts ingest`",
        ),
        ingest::Error::Store(_) => (
            "today's spend and history: new requests aren't saved",
            format!("Gigawatts can't use the {error}"),
            "Check the free disk space and that nothing else holds the history database open. Then run `gigawatts ingest`",
        ),
    };
    Problem {
        kind: ProblemKind::ReadFailing,
        affects: affects.to_string(),
        since: since(store, previous.as_ref()),
        cause,
        fix: fix.to_string(),
        account: account.map(Account::to_string),
    }
}

fn cleanup_failing(
    store: &Store,
    previous: Option<Problem>,
    days: u32,
    error: &store::Error,
) -> Problem {
    Problem {
        kind: ProblemKind::CleanupFailing,
        affects: format!(
            "the history file's size: request detail older than {days} days isn't deleted"
        ),
        since: since(store, previous.as_ref()),
        cause: format!("Gigawatts can't delete old request detail: {error}"),
        fix: "Check the free disk space and that nothing else holds the history database open. Gigawatts tries again after the next transcript change".to_string(),
        account: None,
    }
}

fn since(store: &Store, previous: Option<&Problem>) -> Option<String> {
    previous
        .and_then(|problem| problem.since.clone())
        .or_else(|| store.local_datetime(now_ms()).ok().flatten())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

pub enum Started {
    AlreadyRunning,
    Started,
    Replaced { old: String },
}

pub fn ensure_running(paths: &Paths, executable: &Path) -> Result<Started, Error> {
    let mut started = Started::Started;
    if let Ok(engine) = control::send(&paths.socket(), Command::Version) {
        match control::handshake(&version(), &engine) {
            Handshake::Keep => return Ok(Started::AlreadyRunning),
            Handshake::EngineNewer => return Err(Error::EngineNewer { engine }),
            Handshake::Replace => {
                stop(paths)?;
                started = Started::Replaced { old: engine };
            }
        }
    }
    start(paths, executable)?;
    Ok(started)
}

fn start(paths: &Paths, executable: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(paths.data_dir())?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(paths.log())?;
    let mut engine = std::process::Command::new(executable)
        .args(["engine", "--foreground"])
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + START_WAIT;
    while Instant::now() < deadline {
        if control::send(&paths.socket(), Command::Version).is_ok_and(|engine| engine == version())
        {
            return Ok(());
        }
        if engine.try_wait()?.is_some_and(|status| !status.success()) {
            return Err(Error::Exited { log: paths.log() });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(Error::NoAnswer { log: paths.log() })
}

pub fn replace_if_older(paths: &Paths, executable: &Path) -> Result<(), Error> {
    let Ok(engine) = control::send(&paths.socket(), Command::Version) else {
        return Ok(());
    };
    match control::handshake(&version(), &engine) {
        Handshake::Keep => Ok(()),
        Handshake::EngineNewer => Err(Error::EngineNewer { engine }),
        Handshake::Replace => {
            stop(paths)?;
            start(paths, executable)
        }
    }
}

pub fn stop_unless_newer(paths: &Paths) -> Result<bool, Error> {
    if let Ok(engine) = control::send(&paths.socket(), Command::Version)
        && control::handshake(&version(), &engine) == Handshake::EngineNewer
    {
        return Err(Error::EngineNewer { engine });
    }
    stop(paths)
}

pub fn refresh_if_running(paths: &Paths, executable: &Path) -> Result<bool, Error> {
    if !EngineLock::is_held(&paths.lock())? {
        return Ok(false);
    }
    wait_until_answering(paths)?;
    replace_if_older(paths, executable)?;
    let reply = control::send(&paths.socket(), Command::Refresh)?;
    if reply != REFRESHED {
        return Err(Error::RefreshFailed {
            reply: reply.trim_start_matches(FAILED).to_string(),
        });
    }
    Ok(true)
}

pub fn refresh_limits_if_running(paths: &Paths, executable: &Path) -> Result<bool, Error> {
    if !EngineLock::is_held(&paths.lock())? {
        return Ok(false);
    }
    wait_until_answering(paths)?;
    replace_if_older(paths, executable)?;
    let reply = control::send(&paths.socket(), Command::Limits)?;
    if reply != REFRESHED {
        return Err(Error::LimitsFailed {
            reply: reply.trim_start_matches(FAILED).to_string(),
        });
    }
    Ok(true)
}

pub fn ask_for_limits_if_running(paths: &Paths, executable: &Path) -> Result<bool, Error> {
    if !EngineLock::is_held(&paths.lock())? {
        return Ok(false);
    }
    wait_until_answering(paths)?;
    replace_if_older(paths, executable)?;
    control::send(&paths.socket(), Command::LimitsLater)?;
    Ok(true)
}

pub fn wait_until_answering(paths: &Paths) -> Result<(), Error> {
    let deadline = Instant::now() + START_WAIT;
    while control::send(&paths.socket(), Command::Version).is_err() {
        if Instant::now() >= deadline {
            return Err(Error::NoAnswer { log: paths.log() });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

pub fn is_running(paths: &Paths) -> bool {
    EngineLock::is_held(&paths.lock()).unwrap_or(false)
}

pub fn running_problems(paths: &Paths) -> Vec<Problem> {
    if !is_running(paths) {
        return Vec::new();
    }
    #[derive(serde::Deserialize)]
    struct Written {
        problems: Vec<Problem>,
    }
    std::fs::read_to_string(paths.snapshot())
        .ok()
        .and_then(|text| serde_json::from_str::<Written>(&text).ok())
        .map(|written| written.problems)
        .unwrap_or_default()
        .into_iter()
        .filter(|problem| {
            matches!(
                problem.kind,
                ProblemKind::WatchFailing
                    | ProblemKind::ReadFailing
                    | ProblemKind::CleanupFailing
                    | ProblemKind::RefreshFailing
            )
        })
        .collect()
}

pub fn stop(paths: &Paths) -> Result<bool, Error> {
    let mut told = control::send(&paths.socket(), Command::Stop).is_ok();
    if !told && !EngineLock::is_held(&paths.lock())? {
        return Ok(false);
    }
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if !EngineLock::is_held(&paths.lock())? {
            return Ok(true);
        }
        if !told {
            told = control::send(&paths.socket(), Command::Stop).is_ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let pid = std::fs::read_to_string(paths.lock()).unwrap_or_default();
    Err(Error::StillRunning {
        pid: pid.trim().to_string(),
    })
}
