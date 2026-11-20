use gigawatts::ingest::complete_lines;

#[test]
fn returns_complete_lines_after_the_saved_position() {
    let bytes = b"one\ntwo\nthree\npart";

    let read = complete_lines(bytes, 4);

    assert_eq!(read.lines, vec![&b"two"[..], &b"three"[..]]);
    assert_eq!(read.position, 14);
}

#[test]
fn leaves_a_partial_line_for_the_next_read() {
    let read = complete_lines(b"{\"type\":", 0);

    assert!(read.lines.is_empty());
    assert_eq!(read.position, 0);
}

#[test]
fn reads_nothing_when_the_position_is_at_the_end() {
    let read = complete_lines(b"one\n", 4);

    assert!(read.lines.is_empty());
    assert_eq!(read.position, 4);
}

#[test]
fn skips_empty_lines() {
    let read = complete_lines(b"one\n\ntwo\n", 0);

    assert_eq!(read.lines, vec![&b"one"[..], &b"two"[..]]);
    assert_eq!(read.position, 9);
}

mod unreadable_files {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use gigawatts::accounts::Attribution;
    use gigawatts::ingest::ingest_tree;
    use gigawatts::pricing::PriceTable;
    use gigawatts::sources::git::Resolver;
    use gigawatts::store::Store;

    fn request_line(request_id: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"s1","requestId":"{request_id}","timestamp":"2026-01-09T12:00:00.000Z","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":10}}}}}}"#
        ) + "\n"
    }

    fn ingest(projects: &std::path::Path, store: &mut Store) -> Result<(), String> {
        ingest_tree(
            store,
            projects,
            &PriceTable::built_in(),
            &Attribution::single(None),
            &Resolver,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    #[test]
    fn a_transcript_gone_before_it_is_opened_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        fs::create_dir_all(&projects).unwrap();
        std::os::unix::fs::symlink(dir.path().join("deleted"), projects.join("a.jsonl")).unwrap();
        fs::write(projects.join("b.jsonl"), request_line("r1")).unwrap();
        let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

        assert_eq!(ingest(&projects, &mut store), Ok(()));
        assert_eq!(store.request_count().unwrap(), 1);
    }

    #[test]
    fn a_transcript_gone_before_its_folder_is_listed_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

        let read = ingest(&dir.path().join("projects").join("gone.jsonl"), &mut store);

        assert_eq!(read, Ok(()));
    }

    #[test]
    fn an_unopenable_transcript_is_reported_after_the_others_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        fs::create_dir_all(&projects).unwrap();
        let locked = projects.join("a.jsonl");
        fs::write(&locked, request_line("r1")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        fs::write(projects.join("b.jsonl"), request_line("r2")).unwrap();
        let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

        let read = ingest(&projects, &mut store);

        assert!(
            read.as_ref().is_err_and(|error| error.contains("a.jsonl")),
            "{read:?}"
        );
        assert_eq!(store.request_count().unwrap(), 1);
    }

    #[test]
    fn an_unopenable_folder_is_reported_after_the_rest_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let locked = projects.join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("a.jsonl"), request_line("r1")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        fs::create_dir_all(projects.join("open")).unwrap();
        fs::write(projects.join("open").join("b.jsonl"), request_line("r2")).unwrap();
        let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

        let read = ingest(&projects, &mut store);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            read.as_ref().is_err_and(|error| error.contains("locked")),
            "{read:?}"
        );
        assert_eq!(store.request_count().unwrap(), 1);
    }
}
