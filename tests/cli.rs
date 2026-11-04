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

#[test]
fn schema_prints_the_snapshot_and_data_schemas() {
    let dir = tempfile::tempdir().unwrap();

    let output = gigawatts(Some(dir.path()), dir.path(), &["schema"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let schemas: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schemas["snapshot"]["title"], "Snapshot");
    assert_eq!(schemas["data"]["title"], "DataResponse");
    for name in ["snapshot", "data"] {
        let required = schemas[name]["required"].as_array().unwrap();
        assert!(required.contains(&"schema_version".into()), "{name}");
    }
    assert_eq!(
        schemas["snapshot"]["properties"]["engine_version"]["type"],
        "string"
    );
    let account_fields = schemas["snapshot"]["$defs"]["AccountToday"]["required"]
        .as_array()
        .unwrap();
    assert!(
        account_fields.contains(&"account".into()),
        "{account_fields:?}"
    );
}

#[test]
fn ingest_says_it_read_the_rest_when_one_transcript_is_unreadable() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = home.join(".claude").join("projects").join("p");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"00000000-0000-4000-8000-00000000000a","organizationUuid":"00000000-0000-4000-8000-0000000000f0"}}"#,
    )
    .unwrap();
    let locked = project.join("a.jsonl");
    fs::write(&locked, "").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    fs::write(project.join("b.jsonl"), "").unwrap();

    let output = gigawatts(Some(&home), dir.path(), &["ingest"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("gigawatts: read every other transcript, but can't read ")
            && stderr.contains("a.jsonl"),
        "{stderr}"
    );
}
