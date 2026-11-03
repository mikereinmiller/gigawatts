use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gigawatts::check_costs::CheckReport;
use gigawatts::pricing::PriceTable;
use gigawatts::scrub::scrub_sessions;
use gigawatts::sources::transcript::Corpus;

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

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::CheckCosts { path } => check_costs(path.unwrap_or_else(default_projects_dir)),
        Command::Scrub { out, transcripts } => scrub(&transcripts, &out),
    }
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
    let report = CheckReport::build(&corpus, &PriceTable::built_in());
    print!("{}", report.render());
    if report.passes_gate() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn default_projects_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".claude").join("projects")
}
