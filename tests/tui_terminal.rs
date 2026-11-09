use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use gigawatts::tui::terminal::{Wake, is_snapshot_write, skip_repeated_writes, until_next_minute};
use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RenameMode};
use notify::{Event, EventKind};
use ratatui::crossterm::event::KeyCode;

fn event(kind: EventKind, paths: &[&str]) -> Event {
    paths.iter().fold(Event::new(kind), |event, path| {
        event.add_path(PathBuf::from(path))
    })
}

const SNAPSHOT: &str = "snapshot.json";

#[test]
fn the_engines_rename_onto_the_snapshot_wakes_the_view() {
    let rename_both = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        &["/d/snapshot.json.tmp", "/d/snapshot.json"],
    );
    let rename_to = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::To)),
        &["/d/snapshot.json"],
    );
    let created = event(EventKind::Create(CreateKind::File), &["/d/snapshot.json"]);

    assert!(is_snapshot_write(&rename_both, OsStr::new(SNAPSHOT)));
    assert!(is_snapshot_write(&rename_to, OsStr::new(SNAPSHOT)));
    assert!(is_snapshot_write(&created, OsStr::new(SNAPSHOT)));
}

#[test]
fn reading_the_snapshot_or_writing_the_staged_copy_or_database_does_not() {
    let read = event(
        EventKind::Access(AccessKind::Close(AccessMode::Read)),
        &["/d/snapshot.json"],
    );
    let opened = event(
        EventKind::Access(AccessKind::Open(AccessMode::Any)),
        &["/d/snapshot.json"],
    );
    let staged = event(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        &["/d/snapshot.json.tmp"],
    );
    let database = event(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        &["/d/history.sqlite3-wal"],
    );

    for ignored in [read, opened, staged, database] {
        assert!(
            !is_snapshot_write(&ignored, OsStr::new(SNAPSHOT)),
            "{ignored:?}"
        );
    }
}

#[test]
fn the_view_wakes_at_the_next_minute_boundary() {
    assert_eq!(
        until_next_minute(1_794_216_000_000),
        Duration::from_secs(60)
    );
    assert_eq!(
        until_next_minute(1_794_216_000_001),
        Duration::from_millis(59_999)
    );
    assert_eq!(
        until_next_minute(1_794_216_045_500),
        Duration::from_millis(14_500)
    );
}

#[test]
fn back_to_back_snapshot_writes_rebuild_once_and_keep_the_next_key() {
    let (wake, woken) = std::sync::mpsc::channel();
    wake.send(Wake::SnapshotWritten).unwrap();
    wake.send(Wake::SnapshotWritten).unwrap();
    wake.send(Wake::Key(KeyCode::Char('q').into())).unwrap();
    wake.send(Wake::SnapshotWritten).unwrap();

    let next = skip_repeated_writes(&woken);

    assert!(matches!(next, Some(Wake::Key(key)) if key.code == KeyCode::Char('q')));
    assert!(matches!(woken.try_recv(), Ok(Wake::SnapshotWritten)));
    assert!(skip_repeated_writes(&woken).is_none());
}
