use std::path::PathBuf;

use gigawatts::engine::watcher::{Change, classify};
use notify::event::{
    AccessKind, AccessMode, CreateKind, DataChange, Flag, MetadataKind, ModifyKind, RemoveKind,
    RenameMode,
};
use notify::{Event, EventKind};

fn event(kind: EventKind, path: &str) -> Event {
    Event::new(kind).add_path(PathBuf::from(path))
}

#[test]
fn appends_and_new_transcripts_are_read_again() {
    let appended = event(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        "/p/s1.jsonl",
    );
    let created = event(EventKind::Create(CreateKind::File), "/p/s2.jsonl");
    let renamed = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::To)),
        "/p/s3.jsonl",
    );

    assert_eq!(
        classify(&appended),
        Change::Paths(vec![PathBuf::from("/p/s1.jsonl")])
    );
    assert_eq!(
        classify(&created),
        Change::Paths(vec![PathBuf::from("/p/s2.jsonl")])
    );
    assert_eq!(
        classify(&renamed),
        Change::Paths(vec![PathBuf::from("/p/s3.jsonl")])
    );
}

#[test]
fn a_new_folder_is_read_for_any_transcripts_already_in_it() {
    let created = event(EventKind::Create(CreateKind::Folder), "/p/new-project");

    assert_eq!(
        classify(&created),
        Change::Paths(vec![PathBuf::from("/p/new-project")])
    );
}

#[test]
fn reads_metadata_removals_and_other_files_are_ignored() {
    let ignored = [
        event(
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            "/p/s1.jsonl",
        ),
        event(
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            "/p/s1.jsonl",
        ),
        event(
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
            "/p/s1.jsonl",
        ),
        event(EventKind::Remove(RemoveKind::File), "/p/s1.jsonl"),
        event(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            "/p/s1.meta.json",
        ),
    ];

    for event in ignored {
        assert_eq!(classify(&event), Change::Ignored, "{event:?}");
    }
}

#[test]
fn dropped_events_ask_for_a_full_scan() {
    let dropped = Event::new(EventKind::Other).set_flag(Flag::Rescan);

    assert_eq!(classify(&dropped), Change::FullScan);
}

#[test]
fn a_projects_folder_created_where_one_was_missing_asks_for_a_full_scan() {
    let created = event(EventKind::Create(CreateKind::Folder), "/h/.claude/projects");

    assert_eq!(classify(&created), Change::FullScan);
}

#[test]
fn a_claude_folder_created_in_the_home_folder_asks_for_a_full_scan() {
    let created = event(EventKind::Create(CreateKind::Folder), "/h/.claude");

    assert_eq!(classify(&created), Change::FullScan);
}

#[test]
fn a_removed_projects_or_claude_folder_is_forgotten_and_rescanned() {
    let projects = event(EventKind::Remove(RemoveKind::Folder), "/h/.claude/projects");
    let claude = event(EventKind::Remove(RemoveKind::Folder), "/h/.claude");

    assert_eq!(
        classify(&projects),
        Change::Removed(vec![PathBuf::from("/h/.claude/projects")])
    );
    assert_eq!(
        classify(&claude),
        Change::Removed(vec![PathBuf::from("/h/.claude")])
    );
}

#[test]
fn a_renamed_projects_or_claude_folder_is_forgotten_and_rescanned() {
    let projects = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        "/h/.claude/projects",
    );
    let claude = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
        "/h/.claude",
    );

    assert_eq!(
        classify(&projects),
        Change::Removed(vec![PathBuf::from("/h/.claude/projects")])
    );
    assert_eq!(
        classify(&claude),
        Change::Removed(vec![PathBuf::from("/h/.claude")])
    );
}
