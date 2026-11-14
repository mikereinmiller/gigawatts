use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use gigawatts::accounts::{self, Directory};
use gigawatts::check_costs::CheckReport;
use gigawatts::config::{self, Config};
use gigawatts::contract::{AccountToday, DataResponse, GroupBy, LimitsResponse, Range, Snapshot};
use gigawatts::data::Days;
use gigawatts::engine;
use gigawatts::pace;
use gigawatts::problems;
use gigawatts::scrub::scrub_sessions;
use gigawatts::service::{Manager, Service};
use gigawatts::sources::claude_json::Account;
use gigawatts::sources::transcript::Corpus;
use gigawatts::store::Store;

#[derive(Parser)]
#[command(
    name = "gigawatts",
    version,
    about = "A local usage monitor for Claude Code"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Price every transcript request and compare with Claude Code's recorded session costs
    CheckCosts {
        /// A transcript file or a folder of them (default: ~/.claude/projects)
        path: Option<PathBuf>,
    },
    /// Read new transcript lines into the history database, or ask the running engine to
    Ingest,
    /// Print cost history from the history database
    Data {
        #[arg(long, value_enum, default_value = "today")]
        range: Range,
        /// The first day to include, as YYYY-MM-DD in local time (default: the first day of history)
        #[arg(long, value_parser = gigawatts::data::local_date, conflicts_with = "range")]
        since: Option<String>,
        /// The last day to include, as YYYY-MM-DD in local time (default: today)
        #[arg(long, value_parser = gigawatts::data::local_date, conflicts_with = "range")]
        until: Option<String>,
        #[arg(long, value_enum, default_value = "model")]
        by: GroupBy,
        /// Narrow to one row, by name or key: a repo, then its branch; any other grouping, its sessions. Repeat to go deeper
        #[arg(long)]
        drill: Vec<String>,
        /// Print each account's latest limits instead, after the running engine refreshes any over 10 minutes old
        #[arg(long, conflicts_with_all = ["range", "since", "until", "by", "drill"])]
        limits: bool,
        /// Print versioned JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Print the closest limit with its pace and today's spend against usual, then ask the engine to refresh limits
    Status {
        /// The account to show, by ID or label (default: the one used most recently)
        #[arg(long, conflicts_with = "all")]
        account: Option<String>,
        /// Print one line per account
        #[arg(long)]
        all: bool,
    },
    /// List every current problem with its cause and fix, and exit 1 if there are any
    Doctor,
    /// Open the report page the engine serves in the default browser, starting the engine if it isn't running
    Report,
    /// List Claude Code config directories, their accounts, and whether each is tracked
    Accounts {
        #[command(subcommand)]
        action: Option<AccountsAction>,
    },
    /// Start the background engine at login, or stop doing so
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Stop the background engine
    Stop,
    /// Print the JSON Schema for the snapshot and `data` responses
    Schema,
    /// Start the background engine if it isn't running, replacing an older one
    #[command(hide = true)]
    Engine {
        /// Run the engine in this process instead of in the background
        #[arg(long)]
        foreground: bool,
    },
    /// Write allowlisted copies of transcripts and their subagents for use as one test fixture
    #[command(hide = true)]
    Scrub {
        /// Scrub `get_usage` responses and `cachedUsageUtilization` readings instead of transcripts
        #[arg(long)]
        usage: bool,
        /// The project folder to write the scrubbed sessions into
        out: PathBuf,
        /// Session transcripts, `<session id>.jsonl`, renamed with one shared map
        #[arg(required = true)]
        transcripts: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AccountsAction {
    /// Track a config directory outside the home folder
    Add { path: PathBuf },
    /// Turn tracking on or off for an account, by ID or label
    Track { account: String, state: OnOff },
    /// Turn storing session titles on or off for an account; turning it off offers to delete stored titles
    Titles { account: String, state: OnOff },
    /// Delete an account's history and stop tracking it
    Forget { account: String },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Add a systemd user unit or a launchd agent that starts the engine at login, and start it now
    Install,
    /// Remove the login service, which also stops the engine it started
    Uninstall,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

impl OnOff {
    fn is_on(self) -> bool {
        matches!(self, OnOff::On)
    }
}

fn main() -> ExitCode {
    let command = Cli::parse().command;
    if std::env::var_os("HOME").is_none_or(|home| home.is_empty()) {
        eprintln!(
            "gigawatts: HOME isn't set, so there's no place for the config or the history database."
        );
        return ExitCode::from(2);
    }
    let Some(command) = command else {
        return view();
    };
    match command {
        Command::CheckCosts { path } => check_costs(path.unwrap_or_else(default_projects_dir)),
        Command::Ingest => ingest(),
        Command::Data {
            limits: true, json, ..
        } => limits(json),
        Command::Data {
            range,
            since,
            until,
            by,
            drill,
            json,
            ..
        } => {
            let days = if since.is_some() || until.is_some() {
                Days::Between(since, until)
            } else {
                Days::Range(range)
            };
            data(days, by, &drill, json)
        }
        Command::Status { account, all } => status(account.as_deref(), all),
        Command::Doctor => doctor(),
        Command::Report => report(),
        Command::Accounts { action } => match action {
            None => list_accounts(),
            Some(AccountsAction::Add { path }) => add_account_path(&path),
            Some(AccountsAction::Track { account, state }) => set_tracking(&account, state.is_on()),
            Some(AccountsAction::Titles { account, state }) => set_titles(&account, state.is_on()),
            Some(AccountsAction::Forget { account }) => forget(&account),
        },
        Command::Service { action } => match action {
            ServiceAction::Install => install_service(),
            ServiceAction::Uninstall => uninstall_service(),
        },
        Command::Stop => stop(),
        Command::Schema => schema(),
        Command::Engine { foreground } => {
            if foreground {
                run_engine()
            } else {
                start_engine()
            }
        }
        Command::Scrub {
            usage: false,
            out,
            transcripts,
        } => scrub(&transcripts, &out),
        Command::Scrub {
            usage: true,
            out,
            transcripts,
        } => scrub_usage(&transcripts, &out),
    }
}

fn ingest() -> ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let directories = accounts::discover(&home(), &config);
    report_unreadable(&directories);
    match engine::refresh_if_running(&paths(), &executable()) {
        Ok(true) => {
            println!("The engine is running, so it read the new transcript lines.");
            return ExitCode::SUCCESS;
        }
        Ok(false) => {}
        Err(error) => {
            eprintln!("gigawatts: {error}");
            return ExitCode::from(2);
        }
    }
    let mut store = match open_store() {
        Ok(store) => store,
        Err(code) => return code,
    };
    let read = engine::read_all(&mut store, &directories, &config, &paths().resolver());
    let cached = gigawatts::limits::read_cached(&mut store, &directories, &config)
        .map_err(gigawatts::ingest::Error::from);
    let summary = match cached.and_then(|unreadable| read.map(|summary| (summary, unreadable))) {
        Ok((summary, unreadable)) => {
            report_unreadable_usage(&unreadable);
            summary
        }
        Err(error @ gigawatts::ingest::Error::Store(_)) => {
            eprintln!("gigawatts: can't ingest: {error}");
            return ExitCode::from(2);
        }
        Err(error) => {
            eprintln!("gigawatts: read every other transcript, but can't read {error}");
            return ExitCode::from(2);
        }
    };
    let requests = match store.request_count() {
        Ok(requests) => requests,
        Err(error) => {
            eprintln!("gigawatts: ingested, but can't count the stored requests: {error}");
            return ExitCode::from(2);
        }
    };
    println!(
        "Read new lines from {} of {} transcripts. The history database holds {requests} requests.",
        summary.files_with_new_lines, summary.files
    );
    for (kind, count) in &summary.unrecognized {
        println!("Unrecognized lines mentioning usage, {kind}: {count}");
    }
    ExitCode::SUCCESS
}

fn data(days: Days, by: GroupBy, drill: &[String], json: bool) -> ExitCode {
    if let Days::Between(Some(since), Some(until)) = &days
        && until < since
    {
        eprintln!("gigawatts: --until {until} is before --since {since}.");
        return ExitCode::from(2);
    }
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let store = match open_store() {
        Ok(store) => store,
        Err(code) => return code,
    };
    let drill: Vec<&str> = drill.iter().map(String::as_str).collect();
    let response = gigawatts::data::answer(
        &store,
        &config,
        &days,
        &[],
        by,
        &drill,
        engine::running_problems(&paths()),
    );
    match response {
        Ok(response) => {
            if json {
                println!("{}", to_json(&response));
            } else {
                print!("{}", gigawatts::data::render(&response));
            }
            ExitCode::SUCCESS
        }
        Err(gigawatts::data::Error::Store(error)) => {
            eprintln!("gigawatts: can't query the history database: {error}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("gigawatts: {error}");
            ExitCode::from(2)
        }
    }
}

fn limits(json: bool) -> ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    if let Err(error) = engine::refresh_limits_if_running(&paths(), &executable()) {
        eprintln!("gigawatts: {error}. Showing the saved readings.");
    }
    let store = match open_store() {
        Ok(store) => store,
        Err(code) => return code,
    };
    match gigawatts::limits::rows(&store, &config) {
        Ok(rows) if json => {
            println!("{}", to_json(&gigawatts::limits::response(&rows)));
            ExitCode::SUCCESS
        }
        Ok(rows) => {
            print!("{}", gigawatts::limits::render(&rows, now_ms()));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts: can't read limits from the history database: {error}");
            ExitCode::from(2)
        }
    }
}

fn status(name: Option<&str>, all: bool) -> ExitCode {
    let chosen = match name {
        Some(name) => match find_account(name) {
            Ok(row) if !row.tracking => {
                eprintln!(
                    "gigawatts: {name} isn't tracked. `gigawatts accounts track {name} on` turns it on."
                );
                return ExitCode::from(2);
            }
            Ok(row) => Some(row),
            Err(code) => return code,
        },
        None => None,
    };
    let (Ok(config), Ok(store)) = (load_config(), open_store()) else {
        return ExitCode::from(2);
    };
    let now = now_ms();
    let read = engine::snapshot::build(&store, &config, now, &[])
        .map_err(|error| error.to_string())
        .and_then(|snapshot| match &chosen {
            Some(row) => Ok((snapshot, Some(row.account.clone()))),
            None => {
                let tracked: Vec<_> = snapshot
                    .accounts
                    .iter()
                    .filter_map(|account| account.account.as_deref()?.parse().ok())
                    .collect();
                let last_used = store
                    .last_used_account_among(&tracked)
                    .map_err(|e| e.to_string())?;
                Ok((snapshot, last_used))
            }
        });
    let (snapshot, wanted) = match read {
        Ok(read) => read,
        Err(error) => {
            eprintln!("gigawatts: can't read the history database: {error}");
            return ExitCode::from(2);
        }
    };
    let id = wanted.map(|account| account.to_string());
    let matching = snapshot
        .accounts
        .iter()
        .find(|account| id.is_some() && account.account == id);
    let not_seen_yet = match (&chosen, matching) {
        (Some(row), None) => match pace::dollar_pace(&store, Some(&row.account), now) {
            Ok(pace) => Some(AccountToday {
                account: Some(row.account.to_string()),
                account_label: row.label.clone(),
                configured_label: None,
                requests: 0,
                cost_usd: 0.0,
                pace,
                fastest_session: None,
                active_sessions: Vec::new(),
                unpriced_models: Vec::new(),
                adjusted_models: Vec::new(),
                cost_estimated: false,
                limits: None,
            }),
            Err(error) => {
                eprintln!("gigawatts: can't read the history database: {error}");
                return ExitCode::from(2);
            }
        },
        _ => None,
    };
    let shown: Vec<_> = match (all, matching) {
        (true, _) => snapshot.accounts.iter().collect(),
        (false, matching) => matching
            .or(not_seen_yet.as_ref())
            .or(snapshot.accounts.first())
            .into_iter()
            .collect(),
    };
    if shown.is_empty() {
        println!("No accounts yet. Run `gigawatts accounts` to see what Gigawatts found.");
    }
    let local_minute = local_minute_from(&store);
    for account in shown {
        println!(
            "{}",
            gigawatts::status::render_line(account, now, &local_minute)
        );
    }
    if let Err(error) = engine::ask_for_limits_if_running(&paths(), &executable()) {
        eprintln!("gigawatts: can't ask the engine to refresh limits: {error}");
    }
    ExitCode::SUCCESS
}

fn view() -> ExitCode {
    let paths = paths();
    if let Err(error) = engine::ensure_running(&paths, &executable()) {
        eprintln!("gigawatts: {error}");
        return ExitCode::from(2);
    }
    if let Err(error) = engine::ask_for_limits_if_running(&paths, &executable()) {
        eprintln!("gigawatts: can't ask the engine to refresh limits: {error}");
    }
    let (Ok(mut config), Ok(store)) = (load_config(), open_store()) else {
        return ExitCode::from(2);
    };
    let build = |config: &Config| {
        engine::snapshot::build(&store, config, now_ms(), &engine::running_problems(&paths))
    };
    let first = match build(&config) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("gigawatts: can't read the history database: {error}");
            return ExitCode::from(2);
        }
    };
    let tracked = accounts_in(&first);
    let last_used = match store.last_used_account_among(&tracked) {
        Ok(last_used) => last_used.map(|account| account.to_string()),
        Err(error) => {
            eprintln!("gigawatts: can't read the history database: {error}");
            return ExitCode::from(2);
        }
    };
    let announced_path = paths.announced();
    let announced = gigawatts::tui::announced::read(&announced_path);
    let mut app = gigawatts::tui::App::new(first, last_used, announced.as_ref());
    app.set_engine_written_at(engine_written_at(&paths.snapshot()));
    app.set_report_port(config.report_port());
    let local_minute = local_minute_from(&store);
    let rebuild = |app: &mut gigawatts::tui::App| {
        let reloaded = Config::load(&paths.config(), &home());
        app.set_config_readable(reloaded.is_ok());
        if let Ok(reloaded) = reloaded {
            config = reloaded;
        }
        app.set_report_port(config.report_port());
        match build(&config) {
            Ok(snapshot) => app.update(snapshot),
            Err(error) => app.failed_to_read(error.to_string()),
        }
        app.set_engine_written_at(engine_written_at(&paths.snapshot()));
    };
    let remember =
        |announced: &BTreeSet<String>| gigawatts::tui::announced::write(&announced_path, announced);
    match gigawatts::tui::terminal::run(
        &mut app,
        &paths.snapshot(),
        rebuild,
        |app: &mut gigawatts::tui::App| gigawatts::tui::history::load(app, &store),
        || engine::is_running(&paths),
        remember,
        &local_minute,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("gigawatts: can't draw in this terminal: {error}");
            ExitCode::from(2)
        }
    }
}

fn engine_written_at(snapshot_path: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(snapshot_path).ok()?;
    let written: serde_json::Value = serde_json::from_str(&text).ok()?;
    written["written_at_ms"].as_i64()
}

fn accounts_in(snapshot: &Snapshot) -> Vec<Account> {
    snapshot
        .accounts
        .iter()
        .filter_map(|account| account.account.as_deref()?.parse().ok())
        .collect()
}

fn local_minute_from(store: &Store) -> impl Fn(i64) -> String + '_ {
    |ms: i64| {
        store
            .local_datetime(ms)
            .ok()
            .flatten()
            .map(|time| time.chars().take(16).collect())
            .unwrap_or_default()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn doctor() -> ExitCode {
    let (Ok(config), Ok(store)) = (load_config(), open_store()) else {
        return ExitCode::from(2);
    };
    match problems::collect(&store, &config.price_table()) {
        Ok(mut problems) => {
            problems.extend(engine::running_problems(&paths()));
            print!("{}", problems::render(&problems));
            if problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("gigawatts: can't read problems from the history database: {error}");
            ExitCode::from(2)
        }
    }
}

fn report() -> ExitCode {
    let Ok(config) = load_config() else {
        return ExitCode::from(2);
    };
    let Some(port) = config.report_port() else {
        eprintln!(
            "gigawatts: the report page is off, because `report_port = 0` in ~/.config/gigawatts/config.toml. Remove that line or set a port to turn it on."
        );
        return ExitCode::from(2);
    };
    if let Err(error) = engine::ensure_running(&paths(), &executable()) {
        eprintln!("gigawatts: {error}");
        return ExitCode::from(2);
    }
    if !answering(port) {
        eprintln!(
            "gigawatts: the report page isn't answering on port {port}. The engine reads `report_port` only when it starts, so after changing it run `gigawatts stop` and `gigawatts report`. Otherwise `gigawatts doctor` lists why."
        );
        return ExitCode::from(2);
    }
    let url = format!("http://127.0.0.1:{port}");
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(opener).arg(&url).status() {
        Ok(status) if status.success() => {
            println!("Opened {url} in your browser.");
            ExitCode::SUCCESS
        }
        _ => {
            println!("Couldn't open a browser. The report is at {url}");
            ExitCode::from(2)
        }
    }
}

fn answering(port: u16) -> bool {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if serves_the_report(&address) {
            return true;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn serves_the_report(address: &std::net::SocketAddr) -> bool {
    use std::io::{Read as _, Write as _};
    let wait = std::time::Duration::from_millis(500);
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(address, wait) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(wait));
    let request = format!(
        "GET /api/now HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
        address.port()
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    let reply = String::from_utf8_lossy(&reply);
    reply.starts_with("HTTP/1.1 200 ") && reply.contains("\"schema_version\"")
}

fn list_accounts() -> ExitCode {
    let Ok(config) = load_config() else {
        return ExitCode::from(2);
    };
    let Ok(store) = open_store() else {
        return ExitCode::from(2);
    };
    let directories = accounts::discover(&home(), &config);
    report_unreadable(&directories);
    let listing = accounts::list(&store, &directories, &config)
        .and_then(|rows| Ok((rows, store.login_changes()?)));
    match listing {
        Ok((rows, changes)) => {
            print!(
                "{}",
                accounts::render(&rows, &changes, &directories, &home())
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts: can't read accounts from the history database: {error}");
            ExitCode::from(2)
        }
    }
}

fn add_account_path(path: &Path) -> ExitCode {
    let path = match std::path::absolute(path) {
        Ok(path) if path.join("projects").is_dir() => path,
        _ => {
            eprintln!(
                "gigawatts: {} isn't a Claude Code config directory: it has no projects folder",
                path.display()
            );
            return ExitCode::from(2);
        }
    };
    let edited = edit_config(|text| config::add_extra_path(text, &path, &home()));
    if edited.is_ok() {
        println!(
            "Added {} to extra_paths in {}.",
            path.display(),
            config_path().display()
        );
        refresh_running_engine();
    }
    edited.map_or_else(|code| code, |()| ExitCode::SUCCESS)
}

fn set_tracking(name: &str, on: bool) -> ExitCode {
    let Ok(row) = find_account(name) else {
        return ExitCode::from(2);
    };
    if let Err(code) =
        edit_config(|text| config::set_account_flag(text, &row.account, "tracking", on))
    {
        return code;
    }
    if !on {
        refresh_running_engine();
        println!("Turned tracking off for {}. Its history stays.", row.label);
        return ExitCode::SUCCESS;
    }
    let read_again = with_engine_paused(|| {
        let config = load_config()?;
        let mut store = open_store()?;
        let directories = accounts::discover(&home(), &config);
        accounts::read_again_for(&mut store, &row.account, &directories, &config).map_err(|error| {
            eprintln!("gigawatts: can't reset read positions: {error}");
            ExitCode::from(2)
        })
    });
    match read_again {
        Ok(((), engine_read)) => {
            if engine_read {
                println!(
                    "Turned tracking on for {}. The engine read its transcripts from the start.",
                    row.label
                );
            } else {
                println!(
                    "Turned tracking on for {}. The next ingest reads its transcripts from the start.",
                    row.label
                );
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn set_titles(name: &str, on: bool) -> ExitCode {
    let Ok(row) = find_account(name) else {
        return ExitCode::from(2);
    };
    if let Err(code) =
        edit_config(|text| config::set_account_flag(text, &row.account, "store_titles", on))
    {
        return code;
    }
    if on {
        println!(
            "Session titles will be stored for {} from now on.",
            row.label
        );
        return ExitCode::SUCCESS;
    }
    println!(
        "Session titles won't be stored for {} from now on.",
        row.label
    );
    if !confirm(&format!(
        "Delete the titles already stored for {}?",
        row.label
    )) {
        return ExitCode::SUCCESS;
    }
    let deleted = with_engine_paused(|| {
        open_store()?.delete_titles(&row.account).map_err(|error| {
            eprintln!("gigawatts: can't delete titles: {error}");
            ExitCode::from(2)
        })
    });
    match deleted {
        Ok((count, _)) => {
            println!("Deleted {count} session titles.");
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn forget(name: &str) -> ExitCode {
    let Ok(row) = find_account(name) else {
        return ExitCode::from(2);
    };
    let question = format!(
        "Delete all {} requests of history for {} and stop tracking it? This can't be undone.",
        row.requests, row.label
    );
    if !confirm(&question) {
        return ExitCode::SUCCESS;
    }
    if let Err(code) =
        edit_config(|text| config::set_account_flag(text, &row.account, "tracking", false))
    {
        return code;
    }
    let forgotten = with_engine_paused(|| {
        open_store()?.forget(&row.account).map_err(|error| {
            eprintln!("gigawatts: tracking is off, but deleting the history failed: {error}");
            ExitCode::from(2)
        })
    });
    match forgotten {
        Ok((count, _)) => {
            println!(
                "Deleted {count} requests for {} and turned its tracking off.",
                row.label
            );
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn find_account(name: &str) -> Result<accounts::AccountRow, ExitCode> {
    let config = load_config()?;
    let store = open_store()?;
    let directories = accounts::discover(&home(), &config);
    let rows = accounts::list(&store, &directories, &config).map_err(|error| {
        eprintln!("gigawatts: can't read accounts from the history database: {error}");
        ExitCode::from(2)
    })?;
    let mut matches: Vec<_> = rows
        .into_iter()
        .filter(|row| row.account.to_string() == name || row.label == name)
        .collect();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => {
            eprintln!(
                "gigawatts: no account has the ID or label {name:?}. `gigawatts accounts` lists them."
            );
            Err(ExitCode::from(2))
        }
        _ => {
            eprintln!("gigawatts: more than one account is labeled {name:?}. Use its ID instead.");
            Err(ExitCode::from(2))
        }
    }
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && answer.trim().eq_ignore_ascii_case("y")
}

fn edit_config(edit: impl FnOnce(&str) -> Result<String, config::Error>) -> Result<(), ExitCode> {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            eprintln!("gigawatts: can't read {}: {error}", path.display());
            return Err(ExitCode::from(2));
        }
    };
    let edited = edit(&text).map_err(|error| {
        eprintln!("gigawatts: can't edit {}: {error}", path.display());
        ExitCode::from(2)
    })?;
    let target = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let staged = target.with_extension("toml.tmp");
    let written = target
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&staged, edited))
        .and_then(|()| std::fs::rename(&staged, &target));
    written.map_err(|error| {
        eprintln!("gigawatts: can't write {}: {error}", path.display());
        ExitCode::from(2)
    })
}

fn load_config() -> Result<Config, ExitCode> {
    let path = config_path();
    Config::load(&path, &home()).map_err(|error| {
        eprintln!("gigawatts: can't read {}: {error}", path.display());
        ExitCode::from(2)
    })
}

fn config_path() -> PathBuf {
    paths().config()
}

fn report_unreadable(directories: &[Directory]) {
    for directory in directories {
        if let Some(problem) = &directory.unreadable {
            eprintln!(
                "gigawatts: can't read the login in {problem}. Skipping that directory until it can be read."
            );
        }
    }
}

fn report_unreadable_usage(unreadable: &[gigawatts::limits::UnreadableUsage]) {
    for unreadable in unreadable {
        eprintln!(
            "gigawatts: can't read `cachedUsageUtilization` in {}: {}. Keeping the limits saved before.",
            unreadable.claude_json.display(),
            unreadable.error
        );
    }
}

fn open_store() -> Result<Store, ExitCode> {
    if let Err(error) = engine::replace_if_older(&paths(), &executable()) {
        eprintln!("gigawatts: {error}");
        return Err(ExitCode::from(2));
    }
    let path = database_path();
    Store::open(&path).map_err(|error| {
        eprintln!("gigawatts: can't open {}: {error}", path.display());
        ExitCode::from(2)
    })
}

fn database_path() -> PathBuf {
    paths().database()
}

fn paths() -> engine::Paths {
    engine::Paths::new(&home())
}

fn stop() -> ExitCode {
    match engine::stop(&paths()) {
        Ok(true) => {
            println!("Stopped the engine. The flux capacitor is cooling down.");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("The engine isn't running.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts: {error}");
            ExitCode::from(2)
        }
    }
}

fn install_service() -> ExitCode {
    let Ok(service) = login_service() else {
        return ExitCode::from(2);
    };
    let was_running = match engine::stop_unless_newer(&paths()) {
        Ok(was_running) => was_running,
        Err(error) => {
            eprintln!("gigawatts: {error}");
            return ExitCode::from(2);
        }
    };
    match service.install(&mut run_command) {
        Ok(()) => {
            if let Err(error) = engine::wait_until_answering(&paths()) {
                eprintln!(
                    "gigawatts: installed {}, but {error}",
                    service.file().display()
                );
                return ExitCode::from(2);
            }
            println!(
                "Installed {}. The engine starts at login, and the service started it now.",
                service.file().display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts: can't install the login service: {error}");
            if was_running && let Err(error) = engine::ensure_running(&paths(), &executable()) {
                eprintln!("gigawatts: can't start the engine again: {error}");
            }
            ExitCode::from(2)
        }
    }
}

fn uninstall_service() -> ExitCode {
    let Ok(service) = login_service() else {
        return ExitCode::from(2);
    };
    match service.uninstall(&mut run_command) {
        Ok(true) => {
            println!(
                "Removed {}. The engine no longer starts at login.",
                service.file().display()
            );
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("No login service is installed.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts: can't remove the login service: {error}");
            ExitCode::from(2)
        }
    }
}

fn login_service() -> Result<Service, ExitCode> {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.trim().parse().ok());
    let Some(uid) = uid else {
        eprintln!("gigawatts: can't find your user ID with `id -u`.");
        return Err(ExitCode::from(2));
    };
    Ok(Service {
        manager: Manager::for_this_os(),
        home: home(),
        executable: executable(),
        path: std::env::var("PATH").unwrap_or_default(),
        uid,
    })
}

fn run_command(args: &[&str]) -> std::io::Result<()> {
    let output = std::process::Command::new(args[0])
        .args(&args[1..])
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn schema() -> ExitCode {
    let for_output = || {
        schemars::generate::SchemaSettings::default()
            .for_serialize()
            .into_generator()
    };
    let schemas = serde_json::json!({
        "snapshot": for_output().into_root_schema_for::<Snapshot>(),
        "data": for_output().into_root_schema_for::<DataResponse>(),
        "limits": for_output().into_root_schema_for::<LimitsResponse>(),
    });
    println!("{}", to_json(&schemas));
    ExitCode::SUCCESS
}

fn to_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).expect("contract types always serialize")
}

fn run_engine() -> ExitCode {
    match engine::run(&paths()) {
        Ok(engine::Outcome::AlreadyRunning) => {
            eprintln!("gigawatts: an engine is already running.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts engine: {error}");
            ExitCode::from(2)
        }
    }
}

fn with_engine_paused<T>(
    write: impl FnOnce() -> Result<T, ExitCode>,
) -> Result<(T, bool), ExitCode> {
    let paths = paths();
    let was_running = match engine::stop_unless_newer(&paths) {
        Ok(was_running) => was_running,
        Err(error) => {
            eprintln!("gigawatts: {error}");
            return Err(ExitCode::from(2));
        }
    };
    let written = write();
    if was_running && let Err(error) = engine::ensure_running(&paths, &executable()) {
        eprintln!("gigawatts: can't start the engine again: {error}");
    }
    written.map(|value| (value, refresh_running_engine()))
}

fn refresh_running_engine() -> bool {
    match engine::refresh_if_running(&paths(), &executable()) {
        Ok(refreshed) => refreshed,
        Err(engine::Error::RefreshFailed { reply }) => {
            eprintln!(
                "gigawatts: the engine read the change, but not every transcript: {reply}. `gigawatts doctor` lists what failed."
            );
            true
        }
        Err(error) => {
            eprintln!("gigawatts: the running engine didn't read the change: {error}");
            false
        }
    }
}

fn executable() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gigawatts"))
}

fn start_engine() -> ExitCode {
    match engine::ensure_running(&paths(), &executable()) {
        Ok(engine::Started::AlreadyRunning) => println!("The engine is already running."),
        Ok(engine::Started::Started) => println!("Started the engine."),
        Ok(engine::Started::Replaced { old }) => {
            println!(
                "Stopped engine version {old} and started {}.",
                engine::version()
            )
        }
        Err(error) => {
            eprintln!("gigawatts: {error}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn scrub(transcripts: &[PathBuf], out: &std::path::Path) -> ExitCode {
    match scrub_sessions(transcripts, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("gigawatts: can't scrub into {}: {error}", out.display());
            ExitCode::from(2)
        }
    }
}

fn scrub_usage(files: &[PathBuf], out: &std::path::Path) -> ExitCode {
    match gigawatts::scrub::scrub_usage_files(files, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("gigawatts: can't scrub into {}: {error}", out.display());
            ExitCode::from(2)
        }
    }
}

fn check_costs(path: PathBuf) -> ExitCode {
    let corpus = match Corpus::read(&path) {
        Ok(corpus) => corpus,
        Err(error) => {
            eprintln!("gigawatts: can't read {}: {error}", path.display());
            return ExitCode::from(2);
        }
    };
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let report = CheckReport::build(&corpus, &config.price_table());
    print!("{}", report.render());
    if report.passes_gate() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn default_projects_dir() -> PathBuf {
    home().join(".claude").join("projects")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("main exits before any command when HOME isn't set")
}
