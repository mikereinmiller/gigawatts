use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn gigawatts(home: Option<&Path>, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gigawatts"));
    command.args(args).current_dir(cwd).env_remove("HOME");
    if let Some(home) = home {
        command.env("HOME", home);
    }
    command.output().unwrap()
}

#[test]
fn without_home_nothing_is_written_and_the_command_fails() {
    let dir = tempfile::tempdir().unwrap();

    let output = gigawatts(None, dir.path(), &["data"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("HOME isn't set"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn editing_a_symlinked_config_writes_through_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let dotfiles = dir.path().join("dotfiles");
    let extra = dir.path().join("work-claude");
    fs::create_dir_all(home.join(".config").join("gigawatts")).unwrap();
    fs::create_dir_all(&dotfiles).unwrap();
    fs::create_dir_all(extra.join("projects")).unwrap();
    let target = dotfiles.join("config.toml");
    fs::write(&target, "# kept in dotfiles\n").unwrap();
    let link = home.join(".config").join("gigawatts").join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let output = gigawatts(
        Some(&home),
        dir.path(),
        &["accounts", "add", &extra.to_string_lossy()],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let written = fs::read_to_string(&target).unwrap();
    assert!(written.contains("# kept in dotfiles\n"), "{written}");
    assert!(written.contains("work-claude"), "{written}");
}
