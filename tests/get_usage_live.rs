use std::path::PathBuf;
use std::time::Duration;

use gigawatts::sources::get_usage::{self, Claude};

#[test]
#[ignore = "runs the real `claude -p` against this machine's login; run with --ignored --nocapture"]
fn the_installed_claude_answers_get_usage_with_limits() {
    let claude = Claude {
        program: PathBuf::from("claude"),
        working_dir: std::env::temp_dir(),
        timeout: Duration::from_secs(60),
    };

    let usage = get_usage::fetch(&claude, None, true).unwrap();

    for limit in &usage.limits {
        println!("{limit:?}");
    }
    assert!(!usage.limits.is_empty());
    assert!(usage.behaviors.is_some());
}
