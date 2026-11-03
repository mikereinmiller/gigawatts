use std::path::PathBuf;

use gigawatts::sources::transcript::Corpus;

#[test]
#[ignore = "reads the local ~/.claude/projects, or each folder in GIGAWATTS_CORPUS; run with --ignored --nocapture"]
fn parses_the_whole_local_corpus() {
    let roots: Vec<PathBuf> = match std::env::var_os("GIGAWATTS_CORPUS") {
        Some(paths) => std::env::split_paths(&paths).collect(),
        None => {
            let home = std::env::var_os("HOME").expect("HOME");
            vec![PathBuf::from(home).join(".claude").join("projects")]
        }
    };

    for root in roots {
        let corpus = Corpus::read(&root).expect("corpus");

        println!(
            "{}: {} files, {} requests, {} cost-states",
            root.display(),
            corpus.files,
            corpus.requests.len(),
            corpus.cost_states.len()
        );
        for (kind, count) in &corpus.unrecognized {
            println!("unrecognized {kind}: {count}");
        }
        assert!(corpus.files > 0, "no transcripts under {}", root.display());
    }
}
