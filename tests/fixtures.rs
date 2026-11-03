use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gigawatts::check_costs::CheckReport;
use gigawatts::ingest::complete_lines;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::transcript::{Corpus, parse_line};
use proptest::prelude::*;
use proptest::sample::Index;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/projects");

fn check_costs(fixture: &str) -> String {
    let corpus = Corpus::read(&Path::new(FIXTURES).join(fixture)).expect("fixture");
    CheckReport::build(&corpus, &PriceTable::built_in()).render()
}

#[test]
fn plain_session() {
    insta::assert_snapshot!(check_costs("plain"));
}

#[test]
fn session_with_subagents() {
    insta::assert_snapshot!(check_costs("subagents"));
}

#[test]
fn session_without_a_cost_state() {
    insta::assert_snapshot!(check_costs("no-cost-state"));
}

#[test]
fn long_haiku_prompt() {
    insta::assert_snapshot!(check_costs("long-prompt"));
}

struct Fixture {
    bytes: Vec<u8>,
    whole: Corpus,
}

fn fixtures() -> &'static [Fixture] {
    static FILES: OnceLock<Vec<Fixture>> = OnceLock::new();
    FILES.get_or_init(|| {
        let files = jsonl_files(Path::new(FIXTURES));
        assert!(!files.is_empty(), "no fixtures under {FIXTURES}");
        files
            .into_iter()
            .map(|path| Fixture {
                bytes: std::fs::read(&path).expect("fixture"),
                whole: Corpus::read(&path).expect("fixture"),
            })
            .collect()
    })
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).expect("fixture folder") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            files.extend(jsonl_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
    files
}

fn read_in_appends<'a>(bytes: &'a [u8], cuts: &[usize]) -> (Corpus, Vec<&'a [u8]>) {
    let mut corpus = Corpus::default();
    let mut lines = Vec::new();
    let mut position = 0;
    for &end in cuts.iter().chain([&bytes.len()]) {
        let read = complete_lines(&bytes[..end], position);
        for line in read.lines {
            if let Some(parsed) = parse_line(&String::from_utf8_lossy(line)) {
                corpus.add(parsed);
            }
            lines.push(line);
        }
        position = read.position;
    }
    (corpus, lines)
}

proptest! {
    #[test]
    fn reading_a_fixture_in_arbitrary_appends_gives_the_same_totals(
        file in any::<Index>(),
        cuts in prop::collection::vec(any::<Index>(), 0..40),
    ) {
        let fixture = file.get(fixtures());
        let mut cuts: Vec<usize> = cuts.iter().map(|cut| cut.index(fixture.bytes.len() + 1)).collect();
        cuts.sort();

        let (appended, lines) = read_in_appends(&fixture.bytes, &cuts);

        let every_line: Vec<&[u8]> = fixture.bytes.split(|&byte| byte == b'\n').filter(|line| !line.is_empty()).collect();
        prop_assert_eq!(lines, every_line);
        prop_assert_eq!(&appended.requests, &fixture.whole.requests);
        prop_assert_eq!(&appended.cost_states, &fixture.whole.cost_states);
        prop_assert_eq!(&appended.unrecognized, &fixture.whole.unrecognized);
    }
}
