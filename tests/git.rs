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

    let place = Resolver.place(&nested);

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

    assert_eq!(Resolver.place(&folder), Place::Itself(folder));
}

#[test]
fn folders_under_tmp_are_temporary() {
    assert_eq!(
        Resolver.place(Path::new("/tmp/gigawatts-no-such-folder")),
        Place::Temporary
    );
    assert_eq!(
        Resolver.place(Path::new("/private/tmp/gigawatts-no-such-folder")),
        Place::Temporary
    );
}

#[test]
fn a_missing_folder_outside_any_checkout_counts_as_itself() {
    let missing = Path::new("/Users/nobody/gone/folder");

    assert_eq!(
        Resolver.place(missing),
        Place::Itself(missing.to_path_buf())
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

    assert_eq!(Resolver.head_branch(&main), Some("ab/rebasing".to_string()));
}

#[test]
fn head_is_detached_when_git_cant_name_a_branch() {
    let dir = tempfile::tempdir().unwrap();
    let (main, _worktree) = repo_with_worktree(dir.path());
    git(&main, &["checkout", "-q", "--detach"]);

    assert_eq!(Resolver.head_branch(&main), None);
    assert_eq!(Resolver.head_branch(Path::new("/Users/nobody/gone")), None);
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

    assert_eq!(Resolver.head_branch(&main), None);
}

fn main_checkout(root: &Path, name: &str) -> PathBuf {
    let main = root.join(name);
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    git(
        &main,
        &[
            "remote",
            "add",
            "origin",
            &format!("git@example.com:corp/{name}.git"),
        ],
    );
    git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
    main
}

#[test]
fn a_missing_folder_named_after_a_sibling_checkout_resolves_to_the_longest_match() {
    let dir = tempfile::tempdir_in("/var/tmp").unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    main_checkout(&root, "inventory");
    main_checkout(&root, "inventory-cloud");
    fs::create_dir_all(root.join("check")).unwrap();
    let place = |name: &str| Resolver.place(&root.join(name));
    let repo = |name: &str| Place::Repo {
        name: name.to_string(),
        path: root.join(name),
        remote: Some(format!("git@example.com:corp/{name}.git")),
    };

    assert_eq!(place("inventory-ab-inv-917"), repo("inventory"));
    assert_eq!(
        place("inventory-cloud-agent-inv-1209"),
        repo("inventory-cloud")
    );
    assert_eq!(
        place("check-x"),
        Place::Itself(root.join("check-x")),
        "a sibling outside git doesn't count"
    );
    assert_eq!(
        place("inventoryx"),
        Place::Itself(root.join("inventoryx")),
        "the name must continue with a dash"
    );
}

#[test]
fn a_missing_folder_beside_a_worktree_of_another_repo_counts_as_itself() {
    let dir = tempfile::tempdir_in("/var/tmp").unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    repo_with_worktree(&root);

    let place = Resolver.place(&root.join("rel42-carts-old"));

    assert_eq!(place, Place::Itself(root.join("rel42-carts-old")));
}

#[test]
fn a_missing_subfolder_of_a_deleted_worktree_resolves_through_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    main_checkout(&root, "dotfiles");

    let place = Resolver.place(&root.join("dotfiles-ab-fixup/claude/skills/deployer"));

    assert_eq!(
        place,
        Place::Repo {
            name: "dotfiles".to_string(),
            path: root.join("dotfiles"),
            remote: Some("git@example.com:corp/dotfiles.git".to_string()),
        }
    );
}
