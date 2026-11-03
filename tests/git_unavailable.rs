use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use gigawatts::accounts::Attribution;
use gigawatts::contract::{GroupBy, Range};
use gigawatts::data::query;
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

fn request_line(request_id: &str, cwd: &Path) -> String {
    serde_json::json!({
        "type": "assistant",
        "sessionId": "s1",
        "requestId": request_id,
        "timestamp": "2026-01-08T12:00:00.000Z",
        "cwd": cwd,
        "gitBranch": "main",
        "message": {
            "model": "claude-opus-5-5",
            "usage": {"input_tokens": 1, "output_tokens": 10}
        }
    })
    .to_string()
        + "\n"
}

fn ingest(store: &mut Store, projects: &Path) {
    ingest_tree(
        store,
        projects,
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver::default(),
    )
    .unwrap();
}

fn repo_labels(store: &Store) -> Vec<String> {
    query(store, Range::All, GroupBy::Repo, &[], "2026-01-08")
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.label)
        .collect()
}

#[test]
fn a_folder_seen_while_git_cant_start_is_placed_once_git_can() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("orchard-api");
    let nested = repo.join("src");
    fs::create_dir_all(&nested).unwrap();
    let status = Command::new("git")
        .args(["init", "-q"])
        .arg(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let transcript = projects.join("s1.jsonl");
    fs::write(&transcript, request_line("r1", &nested)).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let path = std::env::var_os("PATH").unwrap();

    // Safe: this file holds a single test, so no other thread reads the environment.
    unsafe { std::env::set_var("PATH", "") };
    ingest(&mut store, &projects);
    unsafe { std::env::set_var("PATH", &path) };
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap()
        .write_all(request_line("r2", &nested).as_bytes())
        .unwrap();
    ingest(&mut store, &projects);

    assert_eq!(repo_labels(&store), vec!["orchard-api".to_string()]);
}
