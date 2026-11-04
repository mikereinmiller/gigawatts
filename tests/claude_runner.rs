use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::Duration;

use gigawatts::sources::get_usage::{self, Claude};

fn claude_echoing_its_config_dir(dir: &Path) -> Claude {
    let program = dir.join("claude");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' \"{{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{{\\\"subtype\\\":\\\"error\\\",\\\"request_id\\\":\\\"{}\\\",\\\"error\\\":\\\"${{CLAUDE_CONFIG_DIR-unset}} ${{ANTHROPIC_API_KEY-unset}} ${{ANTHROPIC_AUTH_TOKEN-unset}} ${{CLAUDE_CODE_OAUTH_TOKEN-unset}}\\\"}}}}\"\n",
            get_usage::REQUEST_ID
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    Claude {
        program,
        working_dir: dir.to_path_buf(),
        timeout: Duration::from_secs(10),
    }
}

#[test]
fn claude_runs_without_an_inherited_config_dir_or_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_echoing_its_config_dir(dir.path());
    // SAFETY: this file's only test, so no other thread reads the environment.
    unsafe {
        std::env::set_var("CLAUDE_CONFIG_DIR", "/elsewhere/.claude-personal");
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-elsewhere");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "elsewhere");
        std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "elsewhere");
    };

    let default = get_usage::fetch(&claude, None, false).unwrap_err();
    let other =
        get_usage::fetch(&claude, Some(Path::new("/home/me/.claude-work")), false).unwrap_err();

    assert_eq!(
        default.to_string(),
        "Claude Code answered with an error: unset unset unset unset"
    );
    assert_eq!(
        other.to_string(),
        "Claude Code answered with an error: /home/me/.claude-work unset unset unset"
    );
}
