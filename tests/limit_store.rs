use gigawatts::contract::{Limit, LimitSource};
use gigawatts::sources::claude_json::Account;
use gigawatts::store::{LimitPoint, LimitsReading, Store, UsageAdvice};

fn account(organization: &str) -> Account {
    Account {
        account_uuid: "00000000-0000-4000-8000-00000000000a".to_string(),
        organization_uuid: organization.to_string(),
    }
}

fn limit(kind: &str, scope: Option<&str>, percent: f64) -> Limit {
    Limit {
        kind: kind.to_string(),
        group: Some("weekly".to_string()),
        scope: scope.map(str::to_string),
        percent,
        severity: Some("normal".to_string()),
        resets_at: Some("2026-11-08T02:59:59.945600+00:00".to_string()),
        active: false,
        credits: None,
    }
}

fn reading(fetched_at_ms: i64, source: LimitSource, limits: Vec<Limit>) -> LimitsReading {
    LimitsReading {
        fetched_at_ms,
        source,
        limits,
    }
}

fn point(kind: &str, scope: Option<&str>, fetched_at_ms: i64, percent: f64) -> LimitPoint {
    LimitPoint {
        kind: kind.to_string(),
        scope: scope.map(str::to_string),
        fetched_at_ms,
        percent,
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    (dir, store)
}

#[test]
fn readings_are_kept_only_when_a_percentage_changes() {
    let (_dir, mut store) = open();
    let personal = account("00000000-0000-4000-8000-0000000000f0");

    for (fetched_at_ms, all, fable) in [
        (1_000, 60.0, 88.0),
        (2_000, 60.0, 88.0),
        (3_000, 61.0, 88.0),
    ] {
        store
            .record_limits(
                &personal,
                &reading(
                    fetched_at_ms,
                    LimitSource::GetUsage,
                    vec![
                        limit("weekly_all", None, all),
                        limit("weekly_scoped", Some("Fable"), fable),
                    ],
                ),
            )
            .unwrap();
    }

    assert_eq!(
        store.limit_history(&personal).unwrap(),
        vec![
            point("weekly_all", None, 1_000, 60.0),
            point("weekly_scoped", Some("Fable"), 1_000, 88.0),
            point("weekly_all", None, 3_000, 61.0),
        ]
    );
}

#[test]
fn the_latest_reading_is_the_newest_fetch_whatever_its_source() {
    let (_dir, mut store) = open();
    let personal = account("00000000-0000-4000-8000-0000000000f0");
    let newer = reading(
        5_000,
        LimitSource::GetUsage,
        vec![limit("weekly_all", None, 63.0)],
    );

    assert!(store.record_limits(&personal, &newer).unwrap());
    let older_cached = reading(
        4_000,
        LimitSource::CachedUsageUtilization,
        vec![limit("weekly_all", None, 51.0)],
    );
    assert!(!store.record_limits(&personal, &older_cached).unwrap());

    assert_eq!(
        store.latest_limits().unwrap(),
        vec![(personal.clone(), newer)]
    );
    assert_eq!(
        store.limit_history(&personal).unwrap(),
        vec![point("weekly_all", None, 5_000, 63.0)]
    );
}

#[test]
fn usage_advice_keeps_the_newest_reading_per_account_and_day() {
    let (_dir, mut store) = open();
    let personal = account("00000000-0000-4000-8000-0000000000f0");
    let work = account("00000000-0000-4000-8000-0000000000e0");
    let morning = serde_json::json!({"day": {"request_count": 12}});
    let evening = serde_json::json!({"day": {"request_count": 3_276}});

    assert_eq!(store.usage_advice(&personal, "2026-11-04").unwrap(), None);
    store
        .record_usage_advice(&personal, "2026-11-04", 1_000, &morning)
        .unwrap();
    store
        .record_usage_advice(&personal, "2026-11-04", 9_000, &evening)
        .unwrap();

    assert_eq!(
        store.usage_advice(&personal, "2026-11-04").unwrap(),
        Some(UsageAdvice {
            fetched_at_ms: 9_000,
            behaviors: evening,
        })
    );
    assert_eq!(store.usage_advice(&personal, "2026-11-05").unwrap(), None);
    assert_eq!(store.usage_advice(&work, "2026-11-04").unwrap(), None);
}

#[test]
fn forgetting_an_account_deletes_its_limits_and_advice() {
    let (_dir, mut store) = open();
    let personal = account("00000000-0000-4000-8000-0000000000f0");
    store
        .record_limits(
            &personal,
            &reading(
                1_000,
                LimitSource::GetUsage,
                vec![limit("session", None, 2.0)],
            ),
        )
        .unwrap();
    store
        .record_usage_advice(&personal, "2026-11-04", 1_000, &serde_json::json!({}))
        .unwrap();

    store.forget(&personal).unwrap();

    assert_eq!(store.latest_limits().unwrap(), vec![]);
    assert_eq!(store.limit_history(&personal).unwrap(), vec![]);
    assert_eq!(store.usage_advice(&personal, "2026-11-04").unwrap(), None);
}

#[test]
fn history_since_a_time_starts_with_the_reading_in_effect_then() {
    let (_dir, mut store) = open();
    let personal = account("00000000-0000-4000-8000-0000000000f0");
    for (fetched_at_ms, all, fable) in [
        (1_000, 10.0, 70.0),
        (2_000, 20.0, 70.0),
        (3_000, 30.0, 70.0),
        (5_000, 40.0, 80.0),
    ] {
        store
            .record_limits(
                &personal,
                &reading(
                    fetched_at_ms,
                    LimitSource::GetUsage,
                    vec![
                        limit("weekly_all", None, all),
                        limit("weekly_scoped", Some("Fable"), fable),
                    ],
                ),
            )
            .unwrap();
    }

    assert_eq!(
        store
            .limit_series(&personal, "weekly_all", None, 2_500)
            .unwrap(),
        vec![
            point("weekly_all", None, 2_000, 20.0),
            point("weekly_all", None, 3_000, 30.0),
            point("weekly_all", None, 5_000, 40.0),
        ]
    );
    assert_eq!(
        store
            .limit_series(&personal, "weekly_scoped", Some("Fable"), 2_500)
            .unwrap(),
        vec![
            point("weekly_scoped", Some("Fable"), 1_000, 70.0),
            point("weekly_scoped", Some("Fable"), 5_000, 80.0),
        ]
    );
    assert_eq!(
        store
            .limit_series(&personal, "weekly_all", None, 500)
            .unwrap()
            .len(),
        4
    );
}
