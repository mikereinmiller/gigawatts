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
        watcher,
        schedule: Schedule::started_at(started.elapsed()),
        changed: BTreeSet::new(),
        watch_problems: BTreeMap::new(),
        login_problems: BTreeMap::new(),
        read_problem: None,
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
    watcher: Watcher,
    schedule: Schedule,
    changed: BTreeSet<PathBuf>,
    watch_problems: BTreeMap<PathBuf, Problem>,
    login_problems: BTreeMap<PathBuf, Problem>,
    read_problem: Option<Problem>,
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
            self.read_problem = Some(read_failing(store, self.read_problem.take(), &error));
        }
        self.write_snapshot(store);
    }

    fn full_scan(&mut self, store: &mut Store, now: Duration) {
        self.changed.clear();
        self.schedule.scanned(now);
        self.reload_config();
        let directories = accounts::discover(&self.paths.home, &self.config);
        self.note_unreadable_logins(store, &directories);
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
                let problem = watch_failing(
                    store,
                    previous_watch_problems.get(&projects),
                    &projects,
                    &error,
                );
                self.watch_problems.insert(projects, problem);
            }
        }
        let previous_read_problem = self.read_problem.take();
        if let Err(error) = read_all(store, &directories, &self.config, &self.paths.resolver()) {
            eprintln!("gigawatts engine: can't read transcripts: {error}");
            self.read_problem = Some(read_failing(store, previous_read_problem, &error));
        }
        self.write_snapshot(store);
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
            .chain(&self.read_problem)
            .cloned()
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

fn watch_failing(
    store: &Store,
    previous: Option<&Problem>,
    projects: &Path,
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
    }
}

fn read_failing(store: &Store, previous: Option<Problem>, error: &ingest::Error) -> Problem {
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

pub fn running_problems(paths: &Paths) -> Vec<Problem> {
    if !EngineLock::is_held(&paths.lock()).unwrap_or(false) {
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
                ProblemKind::WatchFailing | ProblemKind::ReadFailing
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
