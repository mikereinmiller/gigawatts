use std::fs;
use std::path::PathBuf;

use gigawatts::accounts;
use gigawatts::config::Config;
use gigawatts::contract::LimitSource;
use gigawatts::limits;
use gigawatts::store::Store;

const PERSON: &str = "00000000-0000-4000-8000-00000000000a";
const OTHER_PERSON: &str = "00000000-0000-4000-8000-00000000000b";
const ENTERPRISE_ORG: &str = "00000000-0000-4000-8000-0000000000e0";
const ULTRA_ORG: &str = "00000000-0000-4000-8000-0000000000f0";
const JANUARY_1: i64 = 1_767_225_600_000;
const FEBRUARY_1: i64 = 1_769_904_000_000;

struct Home {
    _dir: tempfile::TempDir,
    path: PathBuf,
    store: Store,
    config: Config,
}

fn switched_home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("home");
    fs::create_dir_all(path.join(".claude").join("projects")).unwrap();
    let store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    let config = Config::parse(
        &format!(
            r#"
[directories."~/.claude"]
switched_at = 2026-01-08T23:51:00Z
earlier_account = "{PERSON}/{ENTERPRISE_ORG}"
"#
        ),
        &path,
    )
    .unwrap();
    Home {
        _dir: dir,
        path,
        store,
        config,
    }
}

fn write_reading(home: &Home, account_uuid: &str, fetched_at_ms: i64, percent: u32) {
    fs::write(
        home.path.join(".claude.json"),
        format!(
            r#"{{"oauthAccount":{{"accountUuid":"{PERSON}","organizationUuid":"{ULTRA_ORG}"}},
"cachedUsageUtilization":{{"accountUuid":"{account_uuid}","fetchedAtMs":{fetched_at_ms},"utilization":{{"limits":[
{{"kind":"session","group":"session","percent":{percent},"severity":"normal","resets_at":"2026-02-01T05:00:00+00:00","scope":null,"is_active":false}}]}}}}}}"#
        ),
    )
    .unwrap();
}

fn read(home: &mut Home) {
    let directories = accounts::discover(&home.path, &home.config);
    limits::read_cached(&mut home.store, &directories, &home.config).unwrap();
}

fn latest(home: &Home) -> Vec<(String, i64, LimitSource, f64)> {
    home.store
        .latest_limits()
        .unwrap()
        .into_iter()
        .map(|(account, reading)| {
            (
                account.organization_uuid,
                reading.fetched_at_ms,
                reading.source,
                reading.limits[0].percent,
            )
        })
        .collect()
}

#[test]
fn a_reading_from_before_the_switch_belongs_to_the_earlier_account() {
    let mut home = switched_home();

    write_reading(&home, PERSON, JANUARY_1, 40);
    read(&mut home);
    write_reading(&home, PERSON, FEBRUARY_1, 7);
    read(&mut home);

    assert_eq!(
        latest(&home),
        vec![
            (
                ENTERPRISE_ORG.to_string(),
                JANUARY_1,
                LimitSource::CachedUsageUtilization,
                40.0
            ),
            (
                ULTRA_ORG.to_string(),
                FEBRUARY_1,
                LimitSource::CachedUsageUtilization,
                7.0
            ),
        ]
    );
}

#[test]
fn a_reading_for_another_person_is_not_assigned() {
    let mut home = switched_home();

    write_reading(&home, OTHER_PERSON, FEBRUARY_1, 7);
    read(&mut home);

    assert_eq!(latest(&home), vec![]);
}

#[test]
fn a_reading_for_an_untracked_account_is_not_kept() {
    let mut home = switched_home();
    home.config = Config::parse(
        &format!(
            r#"
[accounts."{PERSON}/{ULTRA_ORG}"]
tracking = false
"#
        ),
        &home.path,
    )
    .unwrap();

    write_reading(&home, PERSON, FEBRUARY_1, 7);
    read(&mut home);

    assert_eq!(latest(&home), vec![]);
}

#[test]
fn a_reading_that_doesnt_parse_is_reported_with_its_file() {
    let mut home = switched_home();
    let claude_json = home.path.join(".claude.json");
    fs::write(
        &claude_json,
        format!(
            r#"{{"oauthAccount":{{"accountUuid":"{PERSON}","organizationUuid":"{ULTRA_ORG}"}},
"cachedUsageUtilization":{{"accountUuid":"{PERSON}","fetchedAtMs":{FEBRUARY_1},"utilization":{{"limits":[{{"kind":"session","percent":null}}]}}}}}}"#
        ),
    )
    .unwrap();
    let directories = accounts::discover(&home.path, &home.config);

    let unreadable = limits::read_cached(&mut home.store, &directories, &home.config).unwrap();

    assert_eq!(unreadable.len(), 1);
    assert_eq!(unreadable[0].claude_json, claude_json);
    assert!(
        unreadable[0]
            .error
            .starts_with("invalid type: null, expected f64"),
        "{}",
        unreadable[0].error
    );
    assert_eq!(latest(&home), vec![]);
}
