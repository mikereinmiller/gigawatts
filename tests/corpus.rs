use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use gigawatts::accounts::Attribution;
use gigawatts::ingest::ingest_tree;
use gigawatts::pricing::PriceTable;
use gigawatts::sources::transcript::{Corpus, Request};
use gigawatts::store::Store;

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

#[test]
#[ignore = "reads the local ~/.claude/projects; run with --ignored --nocapture"]
fn the_history_store_matches_the_transcripts_by_model() {
    let home = std::env::var_os("HOME").expect("HOME");
    let root = PathBuf::from(home).join(".claude").join("projects");
    let corpus = Corpus::read(&root).expect("corpus");
    let table = PriceTable::built_in();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();

    ingest_tree(&mut store, &root, &table, &Attribution::default()).expect("ingest");

    let mut by_request_id: HashMap<&str, &Request> = HashMap::new();
    for request in corpus.requests.values() {
        by_request_id
            .entry(request.request_id.as_str())
            .and_modify(|kept| {
                if request.tokens.output > kept.tokens.output {
                    *kept = request;
                }
            })
            .or_insert(request);
    }
    let mut expected: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    for request in by_request_id.values() {
        let entry = expected.entry(request.model.as_str()).or_default();
        entry.0 += 1;
        entry.1 += table.price(request).unwrap_or(0.0);
    }
    for row in store.totals_by_model(None).unwrap() {
        let (requests, cost) = expected[row.key.as_str()];
        let stored = row.cost_usd.unwrap_or(0.0);
        println!(
            "{}: {} stored vs {requests} in transcripts, ${stored:.4} vs ${cost:.4}",
            row.key, row.requests
        );
        assert_eq!(row.requests, requests, "{}", row.key);
        assert!((stored - cost).abs() < 1e-6, "{}", row.key);
    }
}
