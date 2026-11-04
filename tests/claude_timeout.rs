use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::time::{Duration, Instant};

use gigawatts::sources::get_usage::{self, Claude};

#[test]
fn a_claude_that_never_answers_is_stopped_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("claude");
    fs::write(&program, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let claude = Claude {
        program,
        working_dir: dir.path().to_path_buf(),
        timeout: Duration::from_secs(1),
    };
    let started = Instant::now();

    let error = get_usage::fetch(&claude, None, false).unwrap_err();

    assert_eq!(
        error.to_string(),
        "`claude -p` didn't answer within 1 seconds"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_process_claude_leaves_behind_is_stopped_and_doesnt_hold_the_answer_up() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("claude");
    let leftover = dir.path().join("leftover.pid");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\ncat > /dev/null\nsleep 30 &\necho $! > '{}'\nexit 1\n",
            leftover.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let claude = Claude {
        program,
        working_dir: dir.path().to_path_buf(),
        timeout: Duration::from_secs(5),
    };
    let started = Instant::now();

    let error = get_usage::fetch(&claude, None, false).unwrap_err();

    assert_eq!(
        error.to_string(),
        "`claude -p` exited (exit status: 1) without answering"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let pid = fs::read_to_string(&leftover).unwrap();
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .unwrap()
        .success();
    assert!(!alive, "the leftover sleep {} is still running", pid.trim());
}
