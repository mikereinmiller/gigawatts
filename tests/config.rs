use std::path::Path;

use gigawatts::config::{self, Config};
use gigawatts::pricing::{FastPrices, Prices};
use gigawatts::sources::claude_json::Account;

const HOME: &str = "/home/me";

fn work() -> Account {
    "00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000e0"
        .parse()
        .unwrap()
}

#[test]
fn adding_a_path_keeps_comments_and_writes_it_relative_to_home() {
    let text = "# my settings\nextra_paths = [\"/srv/claude\"] # shared box\n";

    let edited =
        config::add_extra_path(text, Path::new("/home/me/work-claude"), Path::new(HOME)).unwrap();

    assert_eq!(
        edited,
        "# my settings\nextra_paths = [\"/srv/claude\", \"~/work-claude\"] # shared box\n"
    );
}

#[test]
fn adding_a_path_twice_lists_it_once() {
    let once =
        config::add_extra_path("", Path::new("/home/me/work-claude"), Path::new(HOME)).unwrap();

    let twice =
        config::add_extra_path(&once, Path::new("/home/me/work-claude"), Path::new(HOME)).unwrap();

    assert_eq!(twice, "extra_paths = [\"~/work-claude\"]\n");
}

#[test]
fn turning_tracking_off_writes_a_key_under_the_account_id() {
    let text = "# keep me\n";

    let edited = config::set_account_flag(text, &work(), "tracking", false).unwrap();

    assert_eq!(
        edited,
        "# keep me\n\n[accounts.\"00000000-0000-4000-8000-00000000000a/00000000-0000-4000-8000-0000000000e0\"]\ntracking = false\n"
    );
    let parsed = Config::parse(&edited, Path::new(HOME)).unwrap();
    assert!(!parsed.account(&work()).tracking);
}

#[test]
fn turning_tracking_back_on_keeps_the_label() {
    let text = format!(
        "[accounts.\"{}\"]\nlabel = \"Work\"\ntracking = false\n",
        work()
    );

    let edited = config::set_account_flag(&text, &work(), "tracking", true).unwrap();

    assert_eq!(
        edited,
        format!(
            "[accounts.\"{}\"]\nlabel = \"Work\"\ntracking = true\n",
            work()
        )
    );
}

#[test]
fn an_account_switch_needs_a_time_zone() {
    let text = "[directories.\"~/.claude\"]\nswitched_at = 2026-11-10T17:00:00\nearlier_account = \"a/b\"\n";

    let error = Config::parse(text, Path::new(HOME)).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("needs a date, a time, and a time zone"),
        "{error}"
    );
}

#[test]
fn an_account_switch_with_an_offset_is_converted_to_utc() {
    let text = "[directories.\"~/.claude\"]\nswitched_at = 2026-11-10T12:00:00-05:00\nearlier_account = \"a/b\"\n";

    let config = Config::parse(text, Path::new(HOME)).unwrap();

    assert_eq!(
        config.switch(Path::new("/home/me/.claude")).unwrap().at_ms,
        1_794_330_000_000
    );
}

#[test]
fn a_price_override_replaces_the_built_in_price_and_adds_unknown_models() {
    let text = r#"
[prices.claude-opus-5-5]
input = 3.0
cache_write_5m = 3.75
cache_write_1h = 6.0
cache_read = 0.3
output = 15.0

[prices.claude-opus-5-6]
input = 6.0
cache_write_5m = 7.5
cache_write_1h = 12.0
cache_read = 0.6
output = 30.0
"#;

    let table = Config::parse(text, Path::new(HOME)).unwrap().price_table();

    let standard = |model: &str| table.model(model).unwrap().standard;
    assert_eq!(
        standard("claude-opus-5-5-20261001"),
        Prices {
            input: 3.0,
            cache_write_5m: 3.75,
            cache_write_1h: 6.0,
            cache_read: 0.3,
            output: 15.0
        }
    );
    assert_eq!(
        standard("claude-opus-5-6"),
        Prices {
            input: 6.0,
            cache_write_5m: 7.5,
            cache_write_1h: 12.0,
            cache_read: 0.6,
            output: 30.0
        }
    );
    assert_eq!(
        standard("claude-sonnet-5-5"),
        Prices {
            input: 2.0,
            cache_write_5m: 2.5,
            cache_write_1h: 4.0,
            cache_read: 0.1,
            output: 10.0
        }
    );
}

#[test]
fn a_price_override_keyed_by_a_dated_model_id_applies_to_that_model() {
    let text = r#"
[prices.claude-opus-5-5-20261001]
input = 3.0
cache_write_5m = 3.75
cache_write_1h = 6.0
cache_read = 0.3
output = 15.0
"#;

    let table = Config::parse(text, Path::new(HOME)).unwrap().price_table();

    assert_eq!(
        table.model("claude-opus-5-5-20261001").unwrap().standard,
        Prices {
            input: 3.0,
            cache_write_5m: 3.75,
            cache_write_1h: 6.0,
            cache_read: 0.3,
            output: 15.0
        }
    );
}

#[test]
fn a_price_override_keeps_the_built_in_fast_and_long_prompt_prices() {
    let text = r#"
[prices.claude-opus-5-5]
input = 3.0
cache_write_5m = 3.75
cache_write_1h = 6.0
cache_read = 0.3
output = 15.0

[prices.claude-haiku-5-5]
input = 0.2
cache_write_5m = 0.25
cache_write_1h = 0.4
cache_read = 0.02
output = 1.0
"#;

    let table = Config::parse(text, Path::new(HOME)).unwrap().price_table();

    assert_eq!(
        table.model("claude-opus-5-5").unwrap().fast,
        Some(FastPrices {
            input: 8.0,
            output: 40.0
        })
    );
    assert_eq!(
        table.model("claude-haiku-5-5").unwrap().long_prompt,
        Some(Prices {
            input: 0.50,
            cache_write_5m: 0.625,
            cache_write_1h: 1.0,
            cache_read: 0.05,
            output: 2.50
        })
    );
}

#[test]
fn request_detail_is_kept_forever_unless_a_number_of_days_is_set() {
    let unset = Config::parse("", Path::new(HOME)).unwrap();
    let set = Config::parse("request_detail_days = 90\n", Path::new(HOME)).unwrap();

    assert_eq!(
        (unset.request_detail_days, set.request_detail_days),
        (None, Some(90))
    );
}

#[test]
fn keeping_zero_days_of_request_detail_is_refused() {
    let error = Config::parse("request_detail_days = 0\n", Path::new(HOME)).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("request_detail_days must be 1 or more"),
        "{error}"
    );
}

#[test]
fn a_credit_reset_without_a_time_zone_is_refused() {
    let text = format!(
        "[accounts.\"{}\"]\ncredit_reset = 2026-11-01T00:00:00\n",
        work()
    );

    let error = Config::parse(&text, Path::new(HOME)).unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "credit_reset for {} needs a date, a time, and a time zone, like 2026-11-01T00:00:00Z",
            work()
        )
    );
}

#[test]
fn the_report_port_defaults_to_7878_and_zero_turns_it_off() {
    let home = Path::new(HOME);

    assert_eq!(Config::parse("", home).unwrap().report_port(), Some(7878));
    assert_eq!(Config::default().report_port(), Some(7878));
    assert_eq!(
        Config::parse("report_port = 9000\n", home)
            .unwrap()
            .report_port(),
        Some(9000)
    );
    assert_eq!(
        Config::parse("report_port = 0\n", home)
            .unwrap()
            .report_port(),
        None
    );
    assert!(Config::parse("report_port = 70000\n", home).is_err());
}

#[test]
fn price_updates_are_on_unless_the_config_turns_them_off() {
    let unset = Config::parse("", Path::new(HOME)).unwrap();
    let off = Config::parse("price_updates = false\n", Path::new(HOME)).unwrap();

    assert!(unset.price_updates());
    assert!(!off.price_updates());
}

#[test]
fn a_session_counts_as_spending_fast_from_20_dollars_an_hour_unless_the_config_says_otherwise() {
    let home = Path::new(HOME);

    assert_eq!(Config::parse("", home).unwrap().fast_usd_per_hour(), 20.0);
    assert_eq!(
        Config::parse("fast_usd_per_hour = 40\n", home)
            .unwrap()
            .fast_usd_per_hour(),
        40.0
    );
    let error = Config::parse("fast_usd_per_hour = 0.0\n", home).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("fast_usd_per_hour must be more than 0"),
        "{error}"
    );
}
