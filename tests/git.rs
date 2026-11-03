use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gigawatts::sources::git::{Place, Resolver};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn repo_with_worktree(root: &Path) -> (PathBuf, PathBuf) {
    let main = root.join("orchard-api");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    git(
        &main,
        &[
            "remote",
            "add",
            "origin",
            "https://user:token@example.com/corp/orchard-api.git",
        ],
    );
    git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let worktree = root.join("rel42-carts");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "ab/rel42-carts",
            worktree.to_str().unwrap(),
        ],
    );
    (main, worktree)
}

#[test]
fn a_worktree_folder_resolves_to_its_main_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let (main, worktree) = repo_with_worktree(dir.path());
    let nested = worktree.join("src");
    fs::create_dir_all(&nested).unwrap();

    let place = Resolver::default().place(&nested);

    assert_eq!(
        place,
        Place::Repo {
            name: "orchard-api".to_string(),
            path: fs::canonicalize(&main).unwrap(),
            remote: Some("https://example.com/corp/orchard-api.git".to_string()),
        }
    );
}

#[test]
fn a_folder_outside_git_counts_as_itself() {
    let dir = tempfile::tempdir_in("/var/tmp").unwrap();
    let folder = dir.path().join("notes");
    fs::create_dir_all(&folder).unwrap();

    assert_eq!(Resolver::default().place(&folder), Place::Itself(folder));
}

#[test]
fn folders_under_tmp_are_temporary() {
    assert_eq!(
        Resolver::default().place(Path::new("/tmp/gigawatts-no-such-folder")),
        Place::Temporary
    );
    assert_eq!(
        Resolver::default().place(Path::new("/private/tmp/gigawatts-no-such-folder")),
        Place::Temporary
    );
}

#[test]
fn a_missing_folder_with_no_captures_counts_as_itself() {
    let missing = Path::new("/Users/nobody/gone/folder");

    assert_eq!(
        Resolver::default().place(missing),
        Place::Itself(missing.to_path_buf())
    );
}

fn write_run(runs: &Path, time: &str, lines: &[&str]) {
    let run = runs.join(time);
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("repos.jsonl"), lines.join("\n") + "\n").unwrap();
}

#[test]
fn a_missing_folder_resolves_from_the_newest_capture_that_knows_it() {
    let dir = tempfile::tempdir().unwrap();
    let runs = dir.path().join("runs");
    write_run(
        &runs,
        "2026-10-01T09-00-00Z",
        &[
            r#"{"cwd":"/work/old-name-wt","git_common_dir":"/work/old-name/.git","remote":"git@example.com:corp/old-name.git","branch":"main"}"#,
            r#"{"cwd":"/work/blank","git_common_dir":"/work/kept/.git","remote":"https://ci:secret@example.com/corp/kept.git","branch":"main"}"#,
        ],
    );
    write_run(
        &runs,
        "2026-10-20T09-00-00Z",
        &[
            r#"{"cwd":"/work/old-name-wt","git_common_dir":"/work/new-name/.git","remote":"git@example.com:corp/new-name.git","branch":"main"}"#,
            r#"{"cwd":"/work/blank","git_common_dir":"","remote":"","branch":""}"#,
            r#"{"cwd":"/work/never","git_common_dir":"","remote":"","branch":""}"#,
        ],
    );
    let resolver = Resolver {
        capture_runs: Some(runs),
    };

    assert_eq!(
        resolver.place(Path::new("/work/old-name-wt")),
        Place::Repo {
            name: "new-name".to_string(),
            path: PathBuf::from("/work/new-name"),
            remote: Some("git@example.com:corp/new-name.git".to_string()),
        }
    );
    assert_eq!(
        resolver.place(Path::new("/work/blank")),
        Place::Repo {
            name: "kept".to_string(),
            path: PathBuf::from("/work/kept"),
            remote: Some("https://example.com/corp/kept.git".to_string()),
        }
    );
    assert_eq!(
        resolver.place(Path::new("/work/never")),
        Place::Itself(PathBuf::from("/work/never"))
    );
}

#[test]
fn head_resolves_to_the_branch_being_rebased() {
    let dir = tempfile::tempdir().unwrap();
    let (main, _worktree) = repo_with_worktree(dir.path());
    git(&main, &["checkout", "-q", "-b", "ab/rebasing"]);
    fs::write(main.join("a.txt"), "ours\n").unwrap();
    git(&main, &["add", "a.txt"]);
    git(&main, &["commit", "-q", "-m", "ours"]);
    git(&main, &["checkout", "-q", "main"]);
    fs::write(main.join("a.txt"), "theirs\n").unwrap();
    git(&main, &["add", "a.txt"]);
    git(&main, &["commit", "-q", "-m", "theirs"]);
    git(&main, &["checkout", "-q", "ab/rebasing"]);
    let conflicted = Command::new("git")
        .arg("-C")
        .arg(&main)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["rebase", "-q", "main"])
        .output()
        .unwrap();
    assert!(
        !conflicted.status.success(),
        "the rebase should stop on a conflict"
    );

    assert_eq!(
        Resolver::default().head_branch(&main),
        Some("ab/rebasing".to_string())
    );
}

#[test]
fn head_is_detached_when_git_cant_name_a_branch() {
    let dir = tempfile::tempdir().unwrap();
    let (main, _worktree) = repo_with_worktree(dir.path());
    git(&main, &["checkout", "-q", "--detach"]);

    assert_eq!(Resolver::default().head_branch(&main), None);
    assert_eq!(
        Resolver::default().head_branch(Path::new("/Users/nobody/gone")),
        None
    );
}

#[test]
fn a_rebase_started_from_a_detached_head_is_detached() {
    let dir = tempfile::tempdir().unwrap();
    let (main, _worktree) = repo_with_worktree(dir.path());
    git(&main, &["checkout", "-q", "-b", "ab/side"]);
    fs::write(main.join("a.txt"), "ours\n").unwrap();
    git(&main, &["add", "a.txt"]);
    git(&main, &["commit", "-q", "-m", "ours"]);
    git(&main, &["checkout", "-q", "main"]);
    fs::write(main.join("a.txt"), "theirs\n").unwrap();
    git(&main, &["add", "a.txt"]);
    git(&main, &["commit", "-q", "-m", "theirs"]);
    git(&main, &["checkout", "-q", "--detach", "ab/side"]);
    let conflicted = Command::new("git")
        .arg("-C")
        .arg(&main)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["rebase", "-q", "main"])
        .output()
        .unwrap();
    assert!(
        !conflicted.status.success(),
        "the rebase should stop on a conflict"
    );

    assert_eq!(Resolver::default().head_branch(&main), None);
}
