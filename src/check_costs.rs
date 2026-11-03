use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use crate::cost::{Cause, PairCheck, Verdict, reconcile};
use crate::pricing::{PriceTable, TABLE_DATE, TABLE_SOURCE};
use crate::sources::transcript::{Corpus, Request};

const GATE_RATE: f64 = 0.95;
const LARGEST_GAPS: usize = 15;

pub struct CheckReport {
    pub files: usize,
    pub checks: Vec<PairCheck>,
    pub sessions_without_cost_state: usize,
    pub unrecognized: BTreeMap<String, usize>,
}

struct ModelSummary {
    pairs: usize,
    exact: usize,
    tokens_agree: usize,
    tokens_agree_exact: usize,
    priced: f64,
    recorded: f64,
    causes: BTreeMap<Cause, usize>,
}

impl CheckReport {
    pub fn build(corpus: &Corpus, table: &PriceTable) -> Self {
        let mut by_session: HashMap<&str, Vec<&Request>> = HashMap::new();
        for request in corpus.requests.values() {
            by_session
                .entry(request.session_id.as_str())
                .or_default()
                .push(request);
        }
        let mut checks = Vec::new();
        for state in corpus.cost_states.values() {
            let requests = by_session
                .get(state.session_id.as_str())
                .cloned()
                .unwrap_or_default();
            checks.extend(reconcile(state, &requests, table));
        }
        checks.sort_by(|a, b| (&a.session_id, &a.model).cmp(&(&b.session_id, &b.model)));
        let sessions_without_cost_state = by_session
            .keys()
            .filter(|session| !corpus.cost_states.contains_key(**session))
            .count();
        CheckReport {
            files: corpus.files,
            checks,
            sessions_without_cost_state,
            unrecognized: corpus.unrecognized.clone(),
        }
    }

    pub fn exact_pairs(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.verdict == Verdict::Exact)
            .count()
    }

    pub fn match_rate(&self) -> f64 {
        rate(self.exact_pairs(), self.checks.len())
    }

    pub fn passes_gate(&self) -> bool {
        let agreeing: Vec<&PairCheck> = self.agreeing_pairs();
        let exact = agreeing
            .iter()
            .filter(|c| c.verdict == Verdict::Exact)
            .count();
        !agreeing.is_empty() && rate(exact, agreeing.len()) >= GATE_RATE
    }

    fn agreeing_pairs(&self) -> Vec<&PairCheck> {
        self.checks.iter().filter(|c| c.tokens_agree()).collect()
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        self.render_header(&mut out);
        self.render_models(&mut out);
        self.render_largest_gaps(&mut out);
        self.render_unrecognized(&mut out);
        self.render_gate(&mut out);
        out
    }

    fn render_header(&self, out: &mut String) {
        let sessions = self
            .checks
            .iter()
            .map(|c| &c.session_id)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let _ = writeln!(
            out,
            "Checked {} session-model pairs in {} sessions from {} files. {} sessions have no cost-state and were skipped.",
            self.checks.len(),
            sessions,
            self.files,
            self.sessions_without_cost_state
        );
        let _ = writeln!(
            out,
            "Prices: built-in table copied {TABLE_DATE} from {TABLE_SOURCE}"
        );
        let _ = writeln!(out);
    }

    fn render_models(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "{:<28} {:>6} {:>6} {:>7}   {:>17} {:>12} {:>12} {:>8}",
            "model", "pairs", "exact", "rate", "same-token exact", "priced", "recorded", "share"
        );
        for (model, summary) in self.model_summaries() {
            let _ = writeln!(
                out,
                "{:<28} {:>6} {:>6} {:>6.1}%   {:>9}/{:<7} {:>12} {:>12} {:>7.1}%",
                model,
                summary.pairs,
                summary.exact,
                rate(summary.exact, summary.pairs) * 100.0,
                summary.tokens_agree_exact,
                summary.tokens_agree,
                format!("${:.2}", summary.priced),
                format!("${:.2}", summary.recorded),
                share(summary.priced, summary.recorded) * 100.0
            );
            for (cause, count) in &summary.causes {
                let _ = writeln!(out, "    {count:>5} {}", cause.describe());
            }
        }
        let _ = writeln!(out);
    }

    fn model_summaries(&self) -> BTreeMap<&str, ModelSummary> {
        let mut summaries: BTreeMap<&str, ModelSummary> = BTreeMap::new();
        for check in &self.checks {
            let summary = summaries
                .entry(check.model.as_str())
                .or_insert(ModelSummary {
                    pairs: 0,
                    exact: 0,
                    tokens_agree: 0,
                    tokens_agree_exact: 0,
                    priced: 0.0,
                    recorded: 0.0,
                    causes: BTreeMap::new(),
                });
            let exact = check.verdict == Verdict::Exact;
            summary.pairs += 1;
            summary.exact += usize::from(exact);
            summary.priced += check.priced.unwrap_or(0.0);
            summary.recorded += check.recorded;
            if check.tokens_agree() {
                summary.tokens_agree += 1;
                summary.tokens_agree_exact += usize::from(exact);
            }
            if let Some(cause) = check.cause {
                *summary.causes.entry(cause).or_default() += 1;
            }
        }
        summaries
    }

    fn render_largest_gaps(&self, out: &mut String) {
        let mut unexplained: Vec<&PairCheck> = self
            .checks
            .iter()
            .filter(|c| c.verdict == Verdict::Mismatch)
            .filter(|c| c.cause != Some(Cause::MissingFromTranscript))
            .collect();
        if unexplained.is_empty() {
            return;
        }
        unexplained.sort_by(|a, b| b.gap().abs().total_cmp(&a.gap().abs()));
        let _ = writeln!(out, "Largest gaps");
        let _ = writeln!(
            out,
            "{:<10} {:<26} {:>10} {:>10} {:>10} {:>8}  cause",
            "session", "model", "priced", "recorded", "gap", "gap %"
        );
        for check in unexplained.into_iter().take(LARGEST_GAPS) {
            let priced = check.priced.map_or("-".to_string(), |p| format!("{p:.4}"));
            let percent = if check.recorded == 0.0 {
                "-".to_string()
            } else {
                format!("{:.2}%", check.gap() / check.recorded * 100.0)
            };
            let _ = writeln!(
                out,
                "{:<10} {:<26} {:>10} {:>10.4} {:>10.4} {:>8}  {}",
                short(&check.session_id),
                check.model,
                priced,
                check.recorded,
                check.gap(),
                percent,
                check.cause.map_or("", |c| c.describe())
            );
        }
        let _ = writeln!(out);
    }

    fn render_unrecognized(&self, out: &mut String) {
        if self.unrecognized.is_empty() {
            return;
        }
        let counts: Vec<String> = self
            .unrecognized
            .iter()
            .map(|(kind, n)| format!("{kind} {n}"))
            .collect();
        let _ = writeln!(
            out,
            "Lines mentioning usage that were neither requests nor cost-state: {}",
            counts.join(", ")
        );
        let _ = writeln!(out);
    }

    fn render_gate(&self, out: &mut String) {
        let agreeing = self.agreeing_pairs();
        let agreeing_exact = agreeing
            .iter()
            .filter(|c| c.verdict == Verdict::Exact)
            .count();
        let _ = writeln!(
            out,
            "Gate: {}. {} of {} pairs whose transcript tokens equal the cost-state's ({:.1}%) match within max(0.1%, $0.001); the gate needs 95%.",
            if self.passes_gate() { "PASS" } else { "FAIL" },
            agreeing_exact,
            agreeing.len(),
            rate(agreeing_exact, agreeing.len()) * 100.0
        );
        let _ = writeln!(
            out,
            "All pairs: {} of {} match ({:.1}%). The rest is mostly spend Claude Code billed outside the transcript.",
            self.exact_pairs(),
            self.checks.len(),
            self.match_rate() * 100.0
        );
    }
}

fn rate(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

fn share(priced: f64, recorded: f64) -> f64 {
    if recorded == 0.0 {
        0.0
    } else {
        priced / recorded
    }
}

fn short(session_id: &str) -> &str {
    session_id.get(..8).unwrap_or(session_id)
}
