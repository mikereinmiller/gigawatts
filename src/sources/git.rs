use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Repo {
        name: String,
        path: PathBuf,
        remote: Option<String>,
    },
    Temporary,
    Itself(PathBuf),
    Unknown,
}

impl Place {
    fn repo(main_checkout: PathBuf, remote: Option<String>) -> Place {
        let name = main_checkout
            .file_name()
            .map_or_else(
                || main_checkout.to_string_lossy(),
                |name| name.to_string_lossy(),
            )
            .into_owned();
        Place::Repo {
            name,
            path: main_checkout,
            remote: remote.filter(|remote| !remote.is_empty()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Resolver {
    pub capture_runs: Option<PathBuf>,
}

impl Resolver {
    pub fn place(&self, cwd: &Path) -> Place {
        let found = if cwd.is_dir() {
            let Ok(common_dir) = git_output(
                cwd,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            ) else {
                return Place::Unknown;
            };
            common_dir.map(|common_dir| {
                let remote = git_output(cwd, &["remote", "get-url", "origin"])
                    .ok()
                    .flatten()
                    .map(|remote| without_credentials(&remote));
                Place::repo(main_checkout(Path::new(&common_dir)), remote)
            })
        } else {
            self.captured_place(cwd)
        };
        found.unwrap_or_else(|| {
            if is_temporary(cwd) {
                Place::Temporary
            } else {
                Place::Itself(cwd.to_path_buf())
            }
        })
    }

    pub fn head_branch(&self, cwd: &Path) -> Option<String> {
        if !cwd.is_dir() {
            return None;
        }
        let git_dir = PathBuf::from(
            git_output(cwd, &["rev-parse", "--path-format=absolute", "--git-dir"])
                .ok()
                .flatten()?,
        );
        ["rebase-merge", "rebase-apply"].iter().find_map(|state| {
            let head_name = std::fs::read_to_string(git_dir.join(state).join("head-name")).ok()?;
            let branch = head_name.trim().strip_prefix("refs/heads/")?;
            Some(branch.to_string())
        })
    }

    fn captured_place(&self, cwd: &Path) -> Option<Place> {
        let mut runs: Vec<PathBuf> = std::fs::read_dir(self.capture_runs.as_ref()?)
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        runs.sort();
        runs.iter().rev().find_map(|run| {
            let text = std::fs::read_to_string(run.join("repos.jsonl")).ok()?;
            text.lines()
                .filter_map(|line| serde_json::from_str::<CapturedRepo>(line).ok())
                .find(|captured| {
                    Path::new(&captured.cwd) == cwd && !captured.git_common_dir.is_empty()
                })
                .map(|captured| {
                    Place::repo(
                        main_checkout(Path::new(&captured.git_common_dir)),
                        Some(without_credentials(&captured.remote)),
                    )
                })
        })
    }
}

#[derive(Deserialize)]
struct CapturedRepo {
    cwd: String,
    #[serde(default)]
    git_common_dir: String,
    #[serde(default)]
    remote: String,
}

fn git_output(cwd: &Path, args: &[&str]) -> std::io::Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    let text = String::from_utf8(output.stdout).unwrap_or_default();
    let text = text.trim();
    Ok((output.status.success() && !text.is_empty()).then(|| text.to_string()))
}

fn main_checkout(common_dir: &Path) -> PathBuf {
    match (common_dir.file_name(), common_dir.parent()) {
        (Some(name), Some(parent)) if name == ".git" => parent.to_path_buf(),
        _ => common_dir.to_path_buf(),
    }
}

fn without_credentials(remote: &str) -> String {
    let Some((scheme, rest)) = remote.split_once("://") else {
        return remote.to_string();
    };
    let host_start = rest.find('/').unwrap_or(rest.len());
    match rest[..host_start].rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://{host}{}", &rest[host_start..]),
        None => remote.to_string(),
    }
}

fn is_temporary(cwd: &Path) -> bool {
    cwd.starts_with("/tmp") || cwd.starts_with("/private/tmp")
}
