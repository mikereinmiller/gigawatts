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
