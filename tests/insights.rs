use std::fs;

use gigawatts::accounts::Attribution;
use gigawatts::contract::{
    ChangeEffect, InsightRow, InsightsResponse, LaunchFacts, RowTakeaways, RunFacts, Setting,
    Shift, ShiftCause,
};
use gigawatts::ingest::ingest_tree;
use gigawatts::insights::{Mix, WeekSpend, find_shifts, query, render, takeaways};
use gigawatts::pricing::PriceTable;
use gigawatts::sources::git::Resolver;
use gigawatts::store::Store;

const OPUS: &str = "claude-opus-5-5";

struct Line<'a> {
    session: &'a str,
    timestamp: &'a str,
    skill: Option<&'a str>,
    agent: Option<&'a str>,
    agent_id: Option<&'a str>,
    model: &'a str,
    input: u64,
}

impl Default for Line<'_> {
    fn default() -> Self {
        Line {
            session: "s1",
            timestamp: "2026-01-06T12:00:00.000Z",
            skill: None,
            agent: None,
            agent_id: None,
            model: OPUS,
            input: 250_000,
        }
    }
}

impl Line<'_> {
    fn json(&self, request: usize) -> String {
        let mut line = serde_json::json!({
            "type": "assistant",
            "sessionId": self.session,
            "requestId": format!("r{request}"),
            "timestamp": self.timestamp,
            "message": {
                "model": self.model,
                "usage": {"input_tokens": self.input, "output_tokens": 0}
            }
        });
        if let Some(skill) = self.skill {
            line["attributionSkill"] = skill.into();
        }
        if let Some(agent) = self.agent {
            line["attributionAgent"] = agent.into();
            line["isSidechain"] = true.into();
        }
        if let Some(agent_id) = self.agent_id {
            line["agentId"] = agent_id.into();
        }
        line.to_string() + "\n"
    }
}

fn store_with(lines: &[Line]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let text: String = lines
        .iter()
        .enumerate()
        .map(|(n, line)| line.json(n))
        .collect();
    fs::write(projects.join("s.jsonl"), text).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite3")).unwrap();
    ingest_tree(
        &mut store,
        &projects,
        &PriceTable::built_in(),
        &Attribution::default(),
        &Resolver,
    )
    .unwrap();
    (dir, store)
}

fn january(store: &Store) -> InsightsResponse {
    query(store, Some("2026-01-05"), Some("2026-01-18"), "2026-01-18").unwrap()
}

fn row<'a>(rows: &'a [InsightRow], name: &str) -> &'a InsightRow {
    rows.iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no row {name} in {rows:#?}"))
}

fn local_time_in_utc(day: &str, clock: &str) -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1 || 'T' || ?2, 'utc')",
            [day, clock],
            |row| row.get(0),
        )
        .unwrap()
}

fn at(minute: u32) -> String {
    format!("2026-01-06T12:{minute:02}:00.000Z")
}

#[test]
fn a_session_that_goes_back_to_a_skill_runs_it_twice() {
    let times: Vec<String> = (0..4).map(at).collect();
    let (_dir, store) = store_with(&[
        Line {
            timestamp: &times[0],
            skill: Some("plan"),
            ..Line::default()
        },
        Line {
            timestamp: &times[1],
            skill: Some("implement"),
            ..Line::default()
        },
        Line {
            timestamp: &times[2],
            skill: Some("plan"),
            ..Line::default()
        },
        Line {
            timestamp: &times[3],
            skill: Some("plan"),
            ..Line::default()
        },
    ]);

    let response = january(&store);

    let plan = row(&response.skills, "plan");
    assert_eq!(plan.runs, 2);
    assert_eq!(plan.cost_usd, 3.0);
    assert_eq!(plan.cost_per_run_usd, Some(1.5));
    assert_eq!(row(&response.skills, "implement").runs, 1);
    assert_eq!(response.weeks, ["2026-01-05", "2026-01-12"]);
    assert_eq!(plan.weeks[0].runs, 2);
    assert_eq!(plan.weeks[1].runs, 0);
    assert_eq!(plan.weeks[1].cost_per_run_usd, None);
}

#[test]
fn a_subagent_working_inside_a_skill_neither_breaks_its_run_nor_hides_its_cost() {
    let times: Vec<String> = (0..3).map(at).collect();
    let (_dir, store) = store_with(&[
        Line {
            timestamp: &times[0],
            skill: Some("review"),
            ..Line::default()
        },
        Line {
            timestamp: &times[1],
            skill: Some("review"),
            agent: Some("code-reviewer"),
            agent_id: Some("a1"),
            model: "claude-fable-5-1",
            input: 100_000,
            ..Line::default()
        },
        Line {
            timestamp: &times[2],
            skill: Some("review"),
            ..Line::default()
        },
    ]);

    let response = january(&store);

    let review = row(&response.skills, "review");
    assert_eq!(review.runs, 1);
    assert_eq!(review.cost_usd, 3.0);
    assert_eq!(review.subagent_cost_usd, 1.0);
    let reviewer = row(&response.agents, "code-reviewer");
    assert_eq!(reviewer.runs, 1);
    assert_eq!(reviewer.cost_usd, 1.0);
    assert_eq!(reviewer.subagent_cost_usd, 0.0);
}

#[test]
fn each_subagent_id_is_one_run_in_the_week_it_started() {
    let sunday_noon = local_time_in_utc("2026-01-11", "12:00:00");
    let monday_noon = local_time_in_utc("2026-01-12", "12:00:00");
    let monday_one = local_time_in_utc("2026-01-12", "13:00:00");
    let (_dir, store) = store_with(&[
        Line {
            timestamp: &sunday_noon,
            agent: Some("Explore"),
            agent_id: Some("a1"),
            ..Line::default()
        },
        Line {
            timestamp: &monday_noon,
            agent: Some("Explore"),
            agent_id: Some("a1"),
            ..Line::default()
        },
        Line {
            timestamp: &monday_one,
            agent: Some("Explore"),
            agent_id: Some("a2"),
            ..Line::default()
        },
    ]);

    let response = january(&store);

    let explore = row(&response.agents, "Explore");
    assert_eq!(explore.runs, 2);
    assert_eq!(explore.weeks[0].runs, 1);
    assert_eq!(explore.weeks[0].cost_usd, 1.0);
    assert_eq!(explore.weeks[1].runs, 1);
    assert_eq!(explore.weeks[1].cost_usd, 2.0);
}

#[test]
fn a_request_with_no_time_adds_cost_but_starts_no_run() {
    let times: Vec<String> = (0..2).map(at).collect();
    let (dir, store) = store_with(&[
        Line {
            timestamp: &times[0],
            skill: Some("commit"),
            ..Line::default()
        },
        Line {
            timestamp: &times[1],
            skill: Some("pr"),
            ..Line::default()
        },
    ]);
    drop(store);
    let database = dir.path().join("history.sqlite3");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE requests SET time_ms = NULL WHERE request_id = 'r1'",
            [],
        )
        .unwrap();
    let store = Store::open(&database).unwrap();

    let response = january(&store);

    let pr = row(&response.skills, "pr");
    assert_eq!(pr.runs, 0);
    assert_eq!(pr.cost_usd, 1.0);
    assert_eq!(pr.cost_per_run_usd, None);
    assert_eq!(row(&response.skills, "commit").runs, 1);
}

#[test]
fn the_window_starts_no_earlier_than_the_kept_requests() {
    let (_dir, mut store) = store_with(&[
        Line {
            timestamp: "2026-01-06T12:00:00.000Z",
            skill: Some("plan"),
            ..Line::default()
        },
        Line {
            session: "s2",
            timestamp: "2026-01-13T12:00:00.000Z",
            skill: Some("plan"),
            ..Line::default()
        },
    ]);
    store.expire_requests(7, "2026-01-18").unwrap();

    let response = january(&store);

    assert_eq!(response.since, "2026-01-05");
    assert_eq!(response.data_starts, "2026-01-12");
    assert_eq!(response.weeks, ["2026-01-12"]);
    let plan = row(&response.skills, "plan");
    assert_eq!(plan.runs, 1);
    assert_eq!(plan.cost_usd, 1.0);
}

#[test]
fn the_default_window_is_the_last_eight_monday_weeks() {
    let (_dir, store) = store_with(&[Line {
        skill: Some("plan"),
        ..Line::default()
    }]);

    let response = query(&store, None, None, "2026-03-04").unwrap();

    assert_eq!(response.since, "2026-01-12");
    assert_eq!(response.until, "2026-03-04");
    assert_eq!(response.weeks.len(), 8);
    assert_eq!(response.weeks[7], "2026-03-02");
    assert!(response.skills.is_empty(), "{:?}", response.skills);
}

const FABLE: &str = "claude-fable-5-1";

fn mix(key: &str, cost_usd: f64) -> Mix {
    Mix::from([(Some(key.to_string()), cost_usd)])
}

fn week(runs: u64, cost_per_run: f64, model: &str, effort: &str) -> WeekSpend {
    let cost_usd = runs as f64 * cost_per_run;
    WeekSpend {
        runs,
        cost_usd,
        unpriced: 0,
        model: mix(model, cost_usd),
        effort: mix(effort, cost_usd),
        version: mix("2.1.300", cost_usd),
        ..WeekSpend::default()
    }
}

fn weeks(before: [(u64, f64); 2], after: [(u64, f64); 2]) -> Vec<WeekSpend> {
    before
        .iter()
        .map(|&(runs, cost)| week(runs, cost, FABLE, "high"))
        .chain(
            after
                .iter()
                .map(|&(runs, cost)| week(runs, cost, FABLE, "high")),
        )
        .collect()
}

#[test]
fn a_model_change_that_halves_cost_per_run_is_a_shift_with_that_cause() {
    let weeks = [
        week(5, 2.0, FABLE, "high"),
        week(5, 2.0, FABLE, "high"),
        week(5, 1.0, OPUS, "high"),
        week(5, 1.0, OPUS, "high"),
    ];

    let shifts = find_shifts(&weeks, &["w1", "w2", "w3", "w4"].map(String::from));

    let [shift] = shifts.as_slice() else {
        panic!("{shifts:#?}");
    };
    assert_eq!(shift.week, "w3");
    assert_eq!(shift.before_cost_per_run_usd, 2.0);
    assert_eq!(shift.after_cost_per_run_usd, 1.0);
    assert_eq!((shift.before_runs, shift.after_runs), (10, 10));
    assert_eq!(shift.change, -0.5);
    assert_eq!(
        shift.causes,
        [ShiftCause {
            setting: Setting::Model,
            from: Some(FABLE.into()),
            to: Some(OPUS.into()),
            in_subagents: false,
        }]
    );
}

fn labels(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("w{n}")).collect()
}

#[test]
fn a_move_of_thirty_percent_is_a_shift_and_twenty_nine_is_not() {
    let thirty = weeks([(5, 2.0), (5, 2.0)], [(5, 1.4), (5, 1.4)]);
    let twenty_nine = weeks([(5, 2.0), (5, 2.0)], [(5, 1.42), (5, 1.42)]);
    let rise = weeks([(5, 1.0), (5, 1.0)], [(5, 1.3), (5, 1.3)]);

    assert_eq!(find_shifts(&thirty, &labels(4)).len(), 1);
    assert_eq!(find_shifts(&twenty_nine, &labels(4)).len(), 0);
    assert_eq!(find_shifts(&rise, &labels(4)).len(), 1);
}

#[test]
fn nine_runs_on_one_side_is_too_few_to_flag() {
    let weeks = weeks([(5, 2.0), (4, 2.0)], [(5, 1.0), (5, 1.0)]);

    assert!(find_shifts(&weeks, &labels(4)).is_empty());
}

#[test]
fn model_and_effort_changing_together_are_two_causes() {
    let weeks = [
        week(5, 2.0, FABLE, "high"),
        week(5, 2.0, FABLE, "high"),
        week(5, 1.0, OPUS, "medium"),
        week(5, 1.0, OPUS, "medium"),
    ];

    let shifts = find_shifts(&weeks, &labels(4));

    let settings: Vec<Setting> = shifts[0].causes.iter().map(|cause| cause.setting).collect();
    assert_eq!(settings, [Setting::Model, Setting::Effort]);
    assert_eq!(shifts[0].causes[1].from.as_deref(), Some("high"));
    assert_eq!(shifts[0].causes[1].to.as_deref(), Some("medium"));
}

#[test]
fn a_shift_with_no_settings_changed_has_no_causes() {
    let weeks = weeks([(5, 2.0), (5, 2.0)], [(5, 1.0), (5, 1.0)]);

    let shifts = find_shifts(&weeks, &labels(4));

    assert_eq!(shifts.len(), 1);
    assert!(shifts[0].causes.is_empty());
}

#[test]
fn of_two_adjacent_flagged_boundaries_only_the_larger_is_reported() {
    let weeks = weeks([(10, 3.0), (10, 3.0)], [(10, 1.0), (10, 1.0)])
        .into_iter()
        .chain([week(10, 1.0, FABLE, "high")])
        .collect::<Vec<_>>();
    let weeks = [vec![week(10, 3.0, FABLE, "high")], weeks].concat();

    let shifts = find_shifts(&weeks, &labels(6));

    let [shift] = shifts.as_slice() else {
        panic!("{shifts:#?}");
    };
    assert_eq!(shift.week, "w4");
    assert_eq!(shift.change, -2.0 / 3.0);
}

#[test]
fn a_one_week_spike_is_a_rise_then_a_drop() {
    let weeks: Vec<WeekSpend> = [1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 1.0]
        .iter()
        .map(|&cost| week(10, cost, FABLE, "high"))
        .collect();

    let shifts = find_shifts(&weeks, &labels(8));

    let moves: Vec<(&str, f64)> = shifts
        .iter()
        .map(|shift| (shift.week.as_str(), shift.change))
        .collect();
    assert_eq!(moves, [("w3", 1.0), ("w5", -0.5)]);
}

#[test]
fn the_insights_response_carries_each_rows_shifts() {
    let mut lines = Vec::new();
    let times: Vec<String> = (0..40)
        .map(|n| {
            let day = 5 + (n / 10) * 7;
            format!("2026-01-{day:02}T12:{:02}:00.000Z", n % 10)
        })
        .collect();
    let sessions: Vec<String> = (0..40).map(|n| format!("s{n}")).collect();
    for n in 0..40 {
        let fable = n < 20;
        lines.push(Line {
            session: &sessions[n],
            timestamp: &times[n],
            agent: Some("code-reviewer"),
            agent_id: Some("a1"),
            model: if fable { FABLE } else { OPUS },
            input: if fable { 200_000 } else { 250_000 },
            ..Line::default()
        });
    }
    let (_dir, store) = store_with(&lines);

    let response = query(&store, Some("2026-01-05"), Some("2026-02-01"), "2026-02-01").unwrap();

    let reviewer = row(&response.agents, "code-reviewer");
    assert_eq!(reviewer.runs, 40);
    let [shift] = reviewer.shifts.as_slice() else {
        panic!("{:#?}", reviewer.shifts);
    };
    assert_eq!(shift.week, "2026-01-19");
    assert_eq!(shift.before_cost_per_run_usd, 2.0);
    assert_eq!(shift.after_cost_per_run_usd, 1.0);
    assert_eq!(shift.causes[0].to.as_deref(), Some(OPUS));
    let printed = render(&response);
    assert!(
        printed.contains("  Jan 19: moved to Opus 5.5, $2.00 to $1.00 a run, saving $43 a month\n"),
        "{printed}"
    );
}

#[test]
fn requests_on_a_model_with_no_price_never_make_a_shift() {
    let times: Vec<String> = (0..40)
        .map(|n| {
            let day = 5 + (n / 10) * 7;
            format!("2026-01-{day:02}T12:{:02}:00.000Z", n % 10)
        })
        .collect();
    let sessions: Vec<String> = (0..40).map(|n| format!("s{n}")).collect();
    let lines: Vec<Line> = (0..40)
        .map(|n| Line {
            session: &sessions[n],
            timestamp: &times[n],
            agent: Some("code-reviewer"),
            agent_id: Some("a1"),
            model: if n < 20 {
                OPUS
            } else {
                "claude-unreleased-9-9"
            },
            ..Line::default()
        })
        .collect();
    let (_dir, store) = store_with(&lines);

    let response = query(&store, Some("2026-01-05"), Some("2026-02-01"), "2026-02-01").unwrap();

    let reviewer = row(&response.agents, "code-reviewer");
    assert_eq!(reviewer.runs, 40);
    assert!(reviewer.shifts.is_empty(), "{:#?}", reviewer.shifts);
}

#[test]
fn a_range_that_ends_before_the_kept_requests_covers_no_weeks() {
    let (_dir, mut store) = store_with(&[
        Line {
            timestamp: "2026-01-06T12:00:00.000Z",
            skill: Some("plan"),
            ..Line::default()
        },
        Line {
            session: "s2",
            timestamp: "2026-01-13T12:00:00.000Z",
            skill: Some("plan"),
            ..Line::default()
        },
    ]);
    store.expire_requests(7, "2026-01-18").unwrap();

    let response = query(&store, Some("2026-01-05"), Some("2026-01-08"), "2026-01-18").unwrap();

    assert_eq!(response.data_starts, "2026-01-12");
    assert!(response.weeks.is_empty(), "{:?}", response.weeks);
    assert!(response.skills.is_empty(), "{:?}", response.skills);
    let printed = render(&response);
    assert!(
        printed.starts_with("Cost per run, 2026-01-05 to 2026-01-08\n"),
        "{printed}"
    );
    assert!(
        printed.contains("Request detail before 2026-01-12 is no longer kept."),
        "{printed}"
    );
    assert!(!printed.contains("No skill runs"), "{printed}");
}

#[test]
fn a_range_that_starts_after_it_ends_says_so() {
    let (_dir, store) = store_with(&[Line {
        skill: Some("plan"),
        ..Line::default()
    }]);

    let response = query(&store, Some("2026-02-01"), None, "2026-01-18").unwrap();

    assert!(response.weeks.is_empty(), "{:?}", response.weeks);
    assert_eq!(
        render(&response),
        "Cost per run, 2026-02-01 to 2026-01-18\n2026-02-01 is after 2026-01-18, so there's nothing to show.\n"
    );
}

#[test]
fn a_version_change_and_spend_with_no_effort_are_named_as_causes() {
    let side = |cost_usd: f64, effort: Option<&str>, version: &str| WeekSpend {
        runs: 5,
        cost_usd,
        model: mix(OPUS, cost_usd),
        effort: Mix::from([(effort.map(String::from), cost_usd)]),
        version: mix(version, cost_usd),
        unpriced: 0,
        ..WeekSpend::default()
    };
    let weeks = [
        side(10.0, Some("high"), "2.1.300"),
        side(10.0, Some("high"), "2.1.300"),
        side(5.0, None, "2.1.310"),
        side(5.0, None, "2.1.310"),
    ];

    let shifts = find_shifts(&weeks, &labels(4));

    let [shift] = shifts.as_slice() else {
        panic!("{shifts:#?}");
    };
    assert_eq!(
        shift.causes,
        [
            ShiftCause {
                setting: Setting::Effort,
                from: Some("high".into()),
                to: None,
                in_subagents: false,
            },
            ShiftCause {
                setting: Setting::Version,
                from: Some("2.1.300".into()),
                to: Some("2.1.310".into()),
                in_subagents: false,
            },
        ]
    );
    let response = InsightsResponse {
        schema_version: 2,
        today: "2026-01-25".into(),
        since: "2026-01-05".into(),
        until: "2026-01-25".into(),
        data_starts: "2026-01-05".into(),
        weeks: labels(4),
        skills: vec![InsightRow {
            name: "plan".into(),
            runs: 20,
            cost_usd: 30.0,
            cost_per_run_usd: Some(1.5),
            subagent_cost_usd: 0.0,
            first_request_cost_usd: None,
            facts: None,
            weeks: Vec::new(),
            shifts,
        }],
        agents: Vec::new(),
    };
    let printed = render(&response);
    assert!(printed.contains("moved to no effort setting,"), "{printed}");
    assert!(!printed.contains("2.1.310"), "{printed}");
}

#[test]
fn a_model_change_in_a_skills_subagents_is_named_as_a_subagent_cause() {
    let side = |model: &str, cost_per_run: f64| {
        let own = 5.0;
        let subagents = 5.0 * cost_per_run - own;
        WeekSpend {
            runs: 5,
            cost_usd: own + subagents,
            model: mix(OPUS, own),
            effort: mix("medium", own),
            version: mix("2.1.300", own),
            subagent_cost_usd: subagents,
            subagent_model: mix(model, subagents),
            subagent_effort: mix("medium", subagents),
            subagent_version: mix("2.1.300", subagents),
            ..WeekSpend::default()
        }
    };
    let weeks = [
        side(FABLE, 4.0),
        side(FABLE, 4.0),
        side(OPUS, 2.0),
        side(OPUS, 2.0),
    ];

    let shifts = find_shifts(&weeks, &labels(4));

    let [shift] = shifts.as_slice() else {
        panic!("{shifts:#?}");
    };
    assert_eq!(
        shift.causes,
        [ShiftCause {
            setting: Setting::Model,
            from: Some(FABLE.into()),
            to: Some(OPUS.into()),
            in_subagents: true,
        }]
    );
    let response = InsightsResponse {
        schema_version: 2,
        today: "2026-01-25".into(),
        since: "2026-01-05".into(),
        until: "2026-01-25".into(),
        data_starts: "2026-01-05".into(),
        weeks: labels(4),
        skills: vec![InsightRow {
            name: "review".into(),
            runs: 20,
            cost_usd: 60.0,
            cost_per_run_usd: Some(3.0),
            subagent_cost_usd: 40.0,
            first_request_cost_usd: None,
            facts: None,
            weeks: Vec::new(),
            shifts,
        }],
        agents: Vec::new(),
    };
    let printed = render(&response);
    assert!(
        printed.contains("subagents moved to Opus 5.5, $4.00 to $2.00 a run"),
        "{printed}"
    );
}

fn facts<'a>(rows: &'a [InsightRow], name: &str) -> &'a RunFacts {
    row(rows, name)
        .facts
        .as_ref()
        .unwrap_or_else(|| panic!("no facts on {name}"))
}

fn launch<'a>(facts: &'a RunFacts, agent: &str) -> &'a LaunchFacts {
    facts
        .launches
        .iter()
        .find(|launch| launch.agent == agent)
        .unwrap_or_else(|| panic!("no {agent} launches in {facts:#?}"))
}

#[test]
fn a_return_to_a_skill_is_a_repeat_run_with_its_own_launches() {
    let times: Vec<String> = (0..7).map(at).collect();
    let main = |minute: usize, skill: &'static str| Line {
        timestamp: &times[minute],
        skill: Some(skill),
        ..Line::default()
    };
    let reviewer = |minute: usize, agent_id: &'static str, input: u64| Line {
        timestamp: &times[minute],
        skill: Some("review"),
        agent: Some("code-reviewer"),
        agent_id: Some(agent_id),
        input,
        ..Line::default()
    };
    let (_dir, store) = store_with(&[
        main(0, "review"),
        reviewer(1, "a1", 500_000),
        reviewer(2, "a1", 250_000),
        main(3, "commit"),
        main(4, "review"),
        reviewer(5, "a2", 750_000),
        reviewer(6, "a2", 250_000),
    ]);

    let response = january(&store);

    let review = row(&response.skills, "review");
    assert_eq!(review.runs, 2);
    assert_eq!(review.cost_usd, 9.0);
    let review = facts(&response.skills, "review");
    assert_eq!(review.runs_per_session, Some(2.0));
    assert_eq!(review.repeat_run_cost_usd, 5.0);
    assert_eq!(review.top_tenth_session_share, Some(1.0));
    assert_eq!(
        review.launches,
        [LaunchFacts {
            agent: "code-reviewer".into(),
            launches: 2,
            per_run: Some(1.0),
            cost_usd: 7.0,
            first_request_cost_usd: 5.0,
        }]
    );
    let reviewer = row(&response.agents, "code-reviewer");
    assert_eq!(reviewer.facts, None);
    assert_eq!(reviewer.first_request_cost_usd, Some(5.0));
    assert_eq!(row(&response.skills, "commit").first_request_cost_usd, None);
    let printed = render(&response);
    assert!(
        printed.contains(
            "  2.0 runs a session, 56% of cost in repeat runs, 1.0 code-reviewer launches a run\n"
        ),
        "{printed}"
    );
}

#[test]
fn one_subagent_id_in_two_sessions_is_two_launches() {
    let lines: Vec<Line> = ["s1", "s2"]
        .into_iter()
        .flat_map(|session| {
            [
                Line {
                    session,
                    timestamp: "2026-01-06T12:00:00.000Z",
                    skill: Some("review"),
                    ..Line::default()
                },
                Line {
                    session,
                    timestamp: "2026-01-06T12:01:00.000Z",
                    skill: Some("review"),
                    agent: Some("code-reviewer"),
                    agent_id: Some("a1"),
                    ..Line::default()
                },
            ]
        })
        .collect();
    let (_dir, store) = store_with(&lines);

    let response = january(&store);

    let review = facts(&response.skills, "review");
    assert_eq!(review.runs_per_session, Some(1.0));
    assert_eq!(review.repeat_run_cost_usd, 0.0);
    let launches = launch(review, "code-reviewer");
    assert_eq!(launches.launches, 2);
    assert_eq!(launches.per_run, Some(1.0));
    assert_eq!(launches.first_request_cost_usd, 2.0);
}

#[test]
fn a_subagent_request_after_its_skill_moved_on_stays_in_the_run_that_launched_it() {
    let times: Vec<String> = (0..4).map(at).collect();
    let (_dir, store) = store_with(&[
        Line {
            timestamp: &times[0],
            skill: Some("review"),
            ..Line::default()
        },
        Line {
            timestamp: &times[1],
            skill: Some("commit"),
            ..Line::default()
        },
        Line {
            timestamp: &times[2],
            skill: Some("review"),
            agent: Some("code-reviewer"),
            agent_id: Some("a1"),
            ..Line::default()
        },
        Line {
            timestamp: &times[3],
            skill: Some("commit"),
            ..Line::default()
        },
    ]);

    let response = january(&store);

    assert_eq!(row(&response.skills, "review").runs, 1);
    let review = facts(&response.skills, "review");
    assert_eq!(review.repeat_run_cost_usd, 0.0);
    assert_eq!(launch(review, "code-reviewer").launches, 1);
    assert_eq!(row(&response.skills, "commit").runs, 1);
}

#[test]
fn the_costliest_tenth_of_ten_sessions_is_the_single_costliest_one() {
    let sessions: Vec<String> = (0..10).map(|n| format!("s{n}")).collect();
    let lines: Vec<Line> = sessions
        .iter()
        .enumerate()
        .map(|(n, session)| Line {
            session,
            skill: Some("plan"),
            input: if n == 0 { 2_750_000 } else { 250_000 },
            ..Line::default()
        })
        .collect();
    let (_dir, store) = store_with(&lines);

    let response = january(&store);

    let plan = facts(&response.skills, "plan");
    assert_eq!(plan.top_tenth_session_share, Some(0.55));
    assert!(plan.launches.is_empty(), "{:?}", plan.launches);
}

fn shift(before: f64, after: f64, after_runs: u64, causes: Vec<ShiftCause>) -> Shift {
    Shift {
        week: "2026-01-19".into(),
        before_cost_per_run_usd: before,
        after_cost_per_run_usd: after,
        before_runs: 10,
        after_runs,
        change: (after - before) / before,
        causes,
    }
}

fn cause(setting: Setting, from: &str, to: &str, in_subagents: bool) -> ShiftCause {
    ShiftCause {
        setting,
        from: Some(from.into()),
        to: Some(to.into()),
        in_subagents,
    }
}

fn shifted_row(name: &str, shifts: Vec<Shift>) -> InsightRow {
    InsightRow {
        name: name.into(),
        runs: 20,
        cost_usd: 40.0,
        cost_per_run_usd: Some(2.0),
        subagent_cost_usd: 0.0,
        first_request_cost_usd: None,
        facts: None,
        weeks: Vec::new(),
        shifts,
    }
}

#[test]
fn a_change_rows_share_names_the_others_and_small_ones_are_left_out() {
    let version = |in_subagents| cause(Setting::Version, "2.1.300", "2.1.310", in_subagents);
    let response = InsightsResponse {
        schema_version: 2,
        today: "2026-02-01".into(),
        since: "2026-01-05".into(),
        until: "2026-02-01".into(),
        data_starts: "2026-01-05".into(),
        weeks: labels(4),
        skills: vec![
            shifted_row(
                "review",
                vec![shift(
                    4.0,
                    2.0,
                    10,
                    vec![version(false), cause(Setting::Model, FABLE, OPUS, true)],
                )],
            ),
            shifted_row("pr", vec![shift(0.17, 0.23, 17, vec![version(false)])]),
        ],
        agents: vec![
            shifted_row(
                "code-reviewer",
                vec![shift(
                    2.0,
                    1.0,
                    40,
                    vec![cause(Setting::Model, FABLE, OPUS, false), version(false)],
                )],
            ),
            shifted_row(
                "audit-reviewer",
                vec![shift(2.0, 1.0, 20, vec![version(false)])],
            ),
        ],
    };

    let printed = render(&response);
    let answer = takeaways(&response);

    for line in [
        "  Jan 19: subagents moved to Opus 5.5, $4.00 to $2.00 a run, saving $43 a month\n",
        "  Jan 19: moved to Opus 5.5, $2.00 to $1.00 a run, saving $87 a month\n",
        "  Jan 19: no model or effort change, $2.00 to $1.00 a run, saving $43 a month\n",
    ] {
        assert!(printed.contains(line), "{line} not in {printed}");
    }
    assert!(!printed.contains("$0.17 to $0.23"), "{printed}");
    let shared = |rows: &[RowTakeaways], name: &str| -> Vec<String> {
        rows.iter().find(|row| row.name == name).unwrap().changes[0]
            .shared_with
            .clone()
    };
    assert_eq!(shared(&answer.skills, "review"), ["code-reviewer"]);
    assert_eq!(shared(&answer.agents, "code-reviewer"), ["review"]);
    assert!(shared(&answer.agents, "audit-reviewer").is_empty());
}

fn dated_shift(
    week: &str,
    before: f64,
    after: f64,
    after_runs: u64,
    causes: Vec<ShiftCause>,
) -> Shift {
    Shift {
        week: week.into(),
        ..shift(before, after, after_runs, causes)
    }
}

fn eight_weeks(skills: Vec<InsightRow>, agents: Vec<InsightRow>) -> InsightsResponse {
    InsightsResponse {
        schema_version: 2,
        today: "2026-11-18".into(),
        since: "2026-09-28".into(),
        until: "2026-11-18".into(),
        data_starts: "2026-09-28".into(),
        weeks: labels(8),
        skills,
        agents,
    }
}

fn with_launches(name: &str, launch_cost_usd: f64) -> InsightRow {
    InsightRow {
        runs: 800,
        cost_usd: 2700.0,
        facts: Some(RunFacts {
            runs_per_session: Some(2.0),
            repeat_run_cost_usd: 1140.0,
            top_tenth_session_share: Some(0.32),
            launches: vec![LaunchFacts {
                agent: "code-reviewer".into(),
                launches: 1440,
                per_run: Some(1.8),
                cost_usd: launch_cost_usd,
                first_request_cost_usd: 164.0,
            }],
        }),
        ..shifted_row(name, Vec::new())
    }
}

type ChangeSummary<'a> = (&'a str, ChangeEffect, &'a str, Option<&'a str>);

fn summaries(row: &RowTakeaways) -> Vec<ChangeSummary<'_>> {
    row.changes
        .iter()
        .map(|change| {
            (
                change.week.as_str(),
                change.effect,
                change.cause.as_str(),
                change.reversed_on.as_deref(),
            )
        })
        .collect()
}

fn titles(row: &RowTakeaways) -> Vec<&str> {
    row.takeaways
        .iter()
        .map(|item| item.title.as_str())
        .collect()
}

#[test]
fn each_row_carries_its_takeaways_and_changes_ranked_by_what_it_could_save() {
    let to_fable = vec![
        cause(Setting::Model, OPUS, FABLE, false),
        cause(Setting::Effort, "medium", "high", false),
    ];
    let to_opus = vec![
        cause(Setting::Model, FABLE, OPUS, false),
        cause(Setting::Effort, "high", "medium", false),
    ];
    let response = eight_weeks(
        vec![
            InsightRow {
                runs: 44,
                ..shifted_row(
                    "handoff",
                    vec![dated_shift("2026-11-09", 2.11, 5.66, 31, Vec::new())],
                )
            },
            with_launches("review", 2064.0),
        ],
        vec![InsightRow {
            runs: 2273,
            ..shifted_row(
                "code-reviewer",
                vec![
                    dated_shift("2026-10-12", 0.50, 1.55, 182, to_fable),
                    dated_shift("2026-11-09", 1.53, 0.95, 1300, to_opus),
                ],
            )
        }],
    );

    let answer = takeaways(&response);

    let [review, handoff] = answer.skills.as_slice() else {
        panic!("{:#?}", answer.skills);
    };
    assert_eq!(review.name, "review");
    assert_eq!(review.to_save_usd.map(f64::round), Some(1122.0));
    assert_eq!(
        titles(review),
        [
            "1.8 code-reviewer launches a run",
            "Repeat runs in the same session (42%)"
        ]
    );
    assert!(review.changes.is_empty(), "{:?}", review.changes);
    assert_eq!(handoff.to_save_usd.map(f64::round), Some(239.0));
    assert_eq!(
        summaries(handoff),
        [(
            "2026-11-09",
            ChangeEffect::Costing,
            "No model or effort change",
            None
        )]
    );
    let [reviewer] = answer.agents.as_slice() else {
        panic!("{:#?}", answer.agents);
    };
    assert_eq!(reviewer.to_save_usd, None);
    assert_eq!(titles(reviewer), ["Nothing stands out"]);
    assert_eq!(
        summaries(reviewer),
        [
            (
                "2026-11-09",
                ChangeEffect::Saving,
                "Moved to Opus 5.5, medium effort",
                None
            ),
            (
                "2026-10-12",
                ChangeEffect::Reversed,
                "Moved to Fable 5.1, high effort",
                Some("2026-11-09")
            ),
        ]
    );
    let printed = render(&response);
    for line in [
        "  At stake: 1.8 code-reviewer launches a run, about $1,122 a month (+1 more)\n",
        "  At stake: Cost per run up 168% since Nov 9, no model or effort change, about $239 a month\n",
        "  Nov 9: no model or effort change, $2.11 to $5.66 a run, costing $239 a month\n",
        "  Oct 12: moved to Fable 5.1, high effort, $0.50 to $1.55 a run, reversed Nov 9\n",
    ] {
        assert!(printed.contains(line), "{line} not in {printed}");
    }
}

#[test]
fn rows_with_nothing_to_save_come_after_the_rest() {
    let response = eight_weeks(
        vec![
            shifted_row("commit", Vec::new()),
            with_launches("small", 500.0),
            with_launches("large", 3000.0),
        ],
        Vec::new(),
    );

    let answer = takeaways(&response);

    let names: Vec<&str> = answer.skills.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["large", "small", "commit"]);
}

#[test]
fn a_row_with_fewer_than_ten_runs_is_too_thin_to_judge() {
    let response = eight_weeks(
        vec![InsightRow {
            runs: 9,
            ..with_launches("review", 2064.0)
        }],
        Vec::new(),
    );

    let answer = takeaways(&response);

    let [row] = answer.skills.as_slice() else {
        panic!("{:?}", answer.skills);
    };
    assert_eq!(titles(row), ["Too few runs to judge"]);
    assert_eq!(row.to_save_usd, None);
}

type EventSummary<'a> = (
    &'a str,
    ChangeEffect,
    &'a str,
    Vec<&'a str>,
    i64,
    Option<(&'a str, &'a str)>,
);

fn reviewer_switch_and_back() -> InsightsResponse {
    let to_fable = |in_subagents| {
        vec![
            cause(Setting::Model, OPUS, FABLE, in_subagents),
            cause(Setting::Effort, "medium", "high", in_subagents),
        ]
    };
    let to_opus = vec![
        cause(Setting::Model, FABLE, OPUS, false),
        cause(Setting::Effort, "high", "medium", false),
    ];
    eight_weeks(
        vec![
            shifted_row(
                "review",
                vec![dated_shift("2026-10-12", 0.73, 3.60, 60, to_fable(true))],
            ),
            InsightRow {
                runs: 44,
                ..shifted_row(
                    "handoff",
                    vec![dated_shift("2026-11-09", 2.11, 5.66, 31, Vec::new())],
                )
            },
        ],
        vec![
            InsightRow {
                runs: 2273,
                ..shifted_row(
                    "code-reviewer",
                    vec![
                        dated_shift("2026-10-12", 0.50, 1.55, 182, to_fable(false)),
                        dated_shift("2026-11-09", 1.53, 0.95, 1300, to_opus.clone()),
                    ],
                )
            },
            shifted_row(
                "audit-reviewer",
                vec![dated_shift("2026-11-09", 1.69, 0.86, 160, to_opus)],
            ),
        ],
    )
}

#[test]
fn a_rise_is_reversed_when_a_row_it_moved_switches_back() {
    let answer = takeaways(&reviewer_switch_and_back());

    let review = answer
        .skills
        .iter()
        .find(|row| row.name == "review")
        .unwrap();
    assert_eq!(
        summaries(review),
        [(
            "2026-10-12",
            ChangeEffect::Reversed,
            "Subagents moved to Fable 5.1, high effort",
            Some("2026-11-09")
        )]
    );
    assert_eq!(titles(review), ["Nothing stands out"]);
}

#[test]
fn changes_are_listed_once_per_event_with_the_rise_they_undid() {
    let answer = takeaways(&reviewer_switch_and_back());

    let events: Vec<EventSummary> = answer
        .events
        .iter()
        .map(|event| {
            (
                event.week.as_str(),
                event.effect,
                event.cause.as_str(),
                event.rows.iter().map(|row| row.name.as_str()).collect(),
                event.monthly_usd.round() as i64,
                event
                    .undid
                    .as_ref()
                    .map(|undid| (undid.week.as_str(), undid.cause.as_str())),
            )
        })
        .collect();
    assert_eq!(
        events,
        [
            (
                "2026-11-09",
                ChangeEffect::Saving,
                "Moved to Opus 5.5, medium effort",
                vec!["code-reviewer", "audit-reviewer"],
                1639,
                Some(("2026-10-12", "Moved to Fable 5.1, high effort")),
            ),
            (
                "2026-11-09",
                ChangeEffect::Costing,
                "No model or effort change",
                vec!["handoff"],
                239,
                None,
            ),
        ]
    );
    assert!(answer.events[0].rows.iter().all(|row| row.subagent));
    assert!(!answer.events[1].rows[0].subagent);
}

fn skill_titles(row: InsightRow) -> Vec<String> {
    let answer = takeaways(&eight_weeks(vec![row], Vec::new()));
    titles(&answer.skills[0])
        .into_iter()
        .map(String::from)
        .collect()
}

fn with_facts(change: impl FnOnce(&mut RunFacts)) -> InsightRow {
    let mut row = with_launches("review", 400.0);
    change(row.facts.as_mut().unwrap());
    row
}

#[test]
fn a_launch_whose_first_requests_are_a_quarter_of_its_cost_is_a_takeaway() {
    let quarter = skill_titles(with_facts(|facts| {
        facts.launches[0].first_request_cost_usd = 100.0
    }));
    let under = skill_titles(with_facts(|facts| {
        facts.launches[0].first_request_cost_usd = 96.0
    }));

    assert!(
        quarter.contains(&"code-reviewer's first requests are 25% of its cost".to_string()),
        "{quarter:?}"
    );
    assert!(
        !under.iter().any(|title| title.contains("first requests")),
        "{under:?}"
    );
}

#[test]
fn the_costliest_tenth_is_a_takeaway_at_half_the_cost_and_ten_sessions() {
    let half = skill_titles(with_facts(|facts| {
        facts.top_tenth_session_share = Some(0.5)
    }));
    let under = skill_titles(with_facts(|facts| {
        facts.top_tenth_session_share = Some(0.49)
    }));
    let nine_sessions = skill_titles(InsightRow {
        runs: 18,
        ..with_facts(|facts| facts.top_tenth_session_share = Some(0.9))
    });

    assert!(
        half.contains(&"Costliest tenth of sessions spent 50%".to_string()),
        "{half:?}"
    );
    for titles in [under, nine_sessions] {
        assert!(
            !titles
                .iter()
                .any(|title| title.starts_with("Costliest tenth")),
            "{titles:?}"
        );
    }
}

#[test]
fn a_subagent_whose_first_requests_are_a_quarter_of_its_cost_is_a_takeaway() {
    let agent = |first: f64| {
        let answer = takeaways(&eight_weeks(
            Vec::new(),
            vec![InsightRow {
                cost_usd: 400.0,
                first_request_cost_usd: Some(first),
                ..shifted_row("code-reviewer", Vec::new())
            }],
        ));
        titles(&answer.agents[0])
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    };

    assert_eq!(agent(100.0), ["First requests are 25% of its cost"]);
    assert_eq!(agent(96.0), ["Nothing stands out"]);
}

#[test]
fn monthly_figures_scale_by_the_days_the_weeks_cover() {
    let mondays = [
        "2026-09-28",
        "2026-10-05",
        "2026-10-12",
        "2026-10-19",
        "2026-10-26",
        "2026-11-02",
        "2026-11-09",
        "2026-11-16",
    ];
    let response = InsightsResponse {
        data_starts: "2026-10-01".into(),
        weeks: mondays.map(String::from).to_vec(),
        ..eight_weeks(
            vec![InsightRow {
                shifts: vec![dated_shift("2026-11-09", 2.0, 4.0, 40, Vec::new())],
                ..with_launches("review", 2064.0)
            }],
            Vec::new(),
        )
    };

    let answer = takeaways(&response);

    let review = &answer.skills[0];
    assert_eq!(review.to_save_usd.map(f64::round), Some(1282.0));
    assert_eq!(review.changes[0].monthly_usd.round(), 244.0);
}

fn event_rows(response: &InsightsResponse) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = takeaways(response)
        .events
        .iter()
        .map(|event| {
            let mut names: Vec<String> = event.rows.iter().map(|row| row.name.clone()).collect();
            names.sort();
            names
        })
        .collect();
    groups.sort();
    groups
}

#[test]
fn a_model_change_in_a_skill_and_its_subagents_is_one_event_with_the_subagent() {
    let response = eight_weeks(
        vec![shifted_row(
            "review",
            vec![dated_shift(
                "2026-11-09",
                4.0,
                2.0,
                40,
                vec![
                    cause(Setting::Model, FABLE, OPUS, false),
                    cause(Setting::Model, FABLE, OPUS, true),
                ],
            )],
        )],
        vec![shifted_row(
            "code-reviewer",
            vec![dated_shift(
                "2026-11-09",
                2.0,
                1.0,
                40,
                vec![cause(Setting::Model, FABLE, OPUS, false)],
            )],
        )],
    );

    assert_eq!(event_rows(&response), [["code-reviewer", "review"]]);
    let answer = takeaways(&response);
    assert_eq!(answer.skills[0].changes[0].shared_with, ["code-reviewer"]);
}

#[test]
fn a_drop_with_no_cause_groups_a_skill_with_the_subagent_it_launches_only() {
    let drop = || vec![dated_shift("2026-11-09", 4.0, 2.0, 40, Vec::new())];
    let response = eight_weeks(
        vec![
            InsightRow {
                shifts: drop(),
                ..with_launches("review", 400.0)
            },
            shifted_row("pr", drop()),
        ],
        vec![shifted_row("code-reviewer", drop())],
    );

    assert_eq!(
        event_rows(&response),
        [vec!["code-reviewer", "review"], vec!["pr"]]
    );
    let answer = takeaways(&response);
    let shared = |name: &str| {
        answer
            .skills
            .iter()
            .chain(&answer.agents)
            .find(|row| row.name == name)
            .unwrap()
            .changes[0]
            .shared_with
            .clone()
    };
    assert_eq!(shared("review"), ["code-reviewer"]);
    assert_eq!(shared("code-reviewer"), ["review"]);
    assert!(shared("pr").is_empty());
}

#[test]
fn a_skill_change_caused_only_by_its_subagents_says_so_in_changes() {
    let response = eight_weeks(
        vec![shifted_row(
            "review",
            vec![dated_shift(
                "2026-11-09",
                4.0,
                2.0,
                40,
                vec![cause(Setting::Model, FABLE, OPUS, true)],
            )],
        )],
        Vec::new(),
    );

    let answer = takeaways(&response);

    let causes: Vec<&str> = answer
        .events
        .iter()
        .map(|event| event.cause.as_str())
        .collect();
    assert_eq!(causes, ["Subagents moved to Opus 5.5"]);
}

#[test]
fn a_rise_with_no_cause_is_reversed_when_a_later_drop_takes_it_below_where_it_started() {
    let response = eight_weeks(
        Vec::new(),
        vec![
            InsightRow {
                runs: 358,
                ..shifted_row(
                    "audit-reviewer",
                    vec![
                        dated_shift("2026-10-19", 1.32, 2.27, 40, Vec::new()),
                        dated_shift("2026-11-09", 1.69, 0.86, 160, Vec::new()),
                    ],
                )
            },
            InsightRow {
                runs: 337,
                ..shifted_row(
                    "implement-helper",
                    vec![
                        dated_shift("2026-10-19", 1.91, 2.70, 40, Vec::new()),
                        dated_shift("2026-11-09", 2.70, 2.07, 40, Vec::new()),
                    ],
                )
            },
        ],
    );

    let answer = takeaways(&response);

    let row = |name: &str| answer.agents.iter().find(|row| row.name == name).unwrap();
    assert_eq!(
        summaries(row("audit-reviewer"))[1],
        (
            "2026-10-19",
            ChangeEffect::Reversed,
            "No model or effort change",
            Some("2026-11-09")
        )
    );
    assert!(!titles(row("audit-reviewer"))[0].starts_with("Cost per run up"));
    assert_eq!(
        summaries(row("implement-helper"))[1].1,
        ChangeEffect::Costing
    );
}
