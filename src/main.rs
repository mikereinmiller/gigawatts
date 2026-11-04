use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use gigawatts::accounts::{self, Directory};
use gigawatts::check_costs::CheckReport;
use gigawatts::config::{self, Config};
use gigawatts::contract::{GroupBy, Range};
use gigawatts::problems;
use gigawatts::scrub::scrub_sessions;
use gigawatts::sources::git::Resolver;
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
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Price every transcript request and compare with Claude Code's recorded session costs
    CheckCosts {
        /// A transcript file or a folder of them (default: ~/.claude/projects)
        path: Option<PathBuf>,
    },
    /// Read new transcript lines into the history database (temporary, until the engine does it)
    Ingest,
    /// Print cost history from the history database
    Data {
        #[arg(long, value_enum, default_value = "today")]
        range: Range,
        #[arg(long, value_enum, default_value = "model")]
        by: GroupBy,
        /// Narrow to one row, by name or key: a repo, then its branch; any other grouping, its sessions. Repeat to go deeper
        #[arg(long)]
        drill: Vec<String>,
    },
    /// List every current problem with its cause and fix, and exit 1 if there are any
    Doctor,
    /// List Claude Code config directories, their accounts, and whether each is tracked
    Accounts {
        #[command(subcommand)]
        action: Option<AccountsAction>,
    },
    /// Write allowlisted copies of transcripts and their subagents for use as one test fixture
    #[command(hide = true)]
    Scrub {
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
    match command {
        Command::CheckCosts { path } => check_costs(path.unwrap_or_else(default_projects_dir)),
        Command::Ingest => ingest(),
        Command::Data { range, by, drill } => data(range, by, &drill),
        Command::Doctor => doctor(),
        Command::Accounts { action } => match action {
            None => list_accounts(),
            Some(AccountsAction::Add { path }) => add_account_path(&path),
            Some(AccountsAction::Track { account, state }) => set_tracking(&account, state.is_on()),
            Some(AccountsAction::Titles { account, state }) => set_titles(&account, state.is_on()),
            Some(AccountsAction::Forget { account }) => forget(&account),
        },
        Command::Scrub { out, transcripts } => scrub(&transcripts, &out),
    }
}

fn ingest() -> ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let mut store = match open_store() {
        Ok(store) => store,
        Err(code) => return code,
    };
    let directories = accounts::discover(&home(), &config);
    report_unreadable(&directories);
    let table = config.price_table();
    if let Err(error) = store.reprice_if_changed(&table) {
        eprintln!("gigawatts: can't reprice stored requests: {error}");
        return ExitCode::from(2);
    }
    let resolver = Resolver {
        capture_runs: Some(home().join("gigawatts-enterprise-capture").join("runs")),
    };
    let summary = match accounts::ingest(&mut store, &directories, &config, &table, &resolver) {
        Ok(summary) => summary,
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

fn data(range: Range, by: GroupBy, drill: &[String]) -> ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let store = match open_store() {
        Ok(store) => store,
        Err(code) => return code,
    };
    let drill: Vec<&str> = drill.iter().map(String::as_str).collect();
    let response = store
        .local_today()
        .map_err(gigawatts::data::Error::from)
        .and_then(|today| gigawatts::data::query(&store, range, by, &drill, &today))
        .and_then(|mut response| {
            response.problems = problems::collect(&store, &config.price_table())?;
            Ok(response)
        });
    match response {
        Ok(mut response) => {
            gigawatts::data::label_accounts(&mut response, &config);
            print!("{}", gigawatts::data::render(&response));
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

fn doctor() -> ExitCode {
    let (Ok(config), Ok(store)) = (load_config(), open_store()) else {
        return ExitCode::from(2);
    };
    match problems::collect(&store, &config.price_table()) {
        Ok(problems) => {
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
        println!("Turned tracking off for {}. Its history stays.", row.label);
        return ExitCode::SUCCESS;
    }
    let (Ok(config), Ok(mut store)) = (load_config(), open_store()) else {
        return ExitCode::from(2);
    };
    let directories = accounts::discover(&home(), &config);
    match accounts::read_again_for(&mut store, &row.account, &directories, &config) {
        Ok(()) => {
            println!(
                "Turned tracking on for {}. The next ingest reads its transcripts from the start.",
                row.label
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gigawatts: can't reset read positions: {error}");
            ExitCode::from(2)
        }
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
    let deleted = open_store().and_then(|mut store| {
        store.delete_titles(&row.account).map_err(|error| {
            eprintln!("gigawatts: can't delete titles: {error}");
            ExitCode::from(2)
        })
    });
    match deleted {
        Ok(count) => {
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
    let forgotten = open_store().and_then(|mut store| {
        store.forget(&row.account).map_err(|error| {
            eprintln!("gigawatts: tracking is off, but deleting the history failed: {error}");
            ExitCode::from(2)
        })
    });
    match forgotten {
        Ok(count) => {
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
    home().join(".config").join("gigawatts").join("config.toml")
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

fn open_store() -> Result<Store, ExitCode> {
    let path = database_path();
    Store::open(&path).map_err(|error| {
        eprintln!("gigawatts: can't open {}: {error}", path.display());
        ExitCode::from(2)
    })
}

fn database_path() -> PathBuf {
    home()
        .join(".local")
        .join("share")
        .join("gigawatts")
        .join("history.sqlite3")
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
