use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gigawatts::check_costs::CheckReport;
use gigawatts::pricing::PriceTable;
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
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::CheckCosts { path } => check_costs(path.unwrap_or_else(default_projects_dir)),
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
