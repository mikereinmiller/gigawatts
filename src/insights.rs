use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const GUIDE: &str = include_str!("../docs/insights-guide.md");

use crate::contract::{
    Change, ChangeEffect, ChangeEvent, EventRow, InsightRow, InsightsResponse, LaunchFacts,
    RowTakeaways, RunFacts, SCHEMA_VERSION, Setting, Shift, ShiftCause, Takeaway,
    TakeawaysResponse, UndoneChange, WeekRuns,
};
use crate::data::Error;
use crate::store::{RunFactRows, RunKind, Store};

const DEFAULT_WEEKS: u32 = 8;

pub fn answer(
    store: &Store,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<InsightsResponse, Error> {
    query(store, since, until, &store.local_today()?)
}

pub fn query(
    store: &Store,
    since: Option<&str>,
    until: Option<&str>,
    today: &str,
) -> Result<InsightsResponse, Error> {
    let until = until.unwrap_or(today).to_string();
    let since = match since {
        Some(since) => since.to_string(),
        None => store.week_start_weeks_before(&until, DEFAULT_WEEKS - 1)?,
    };
    let data_starts = match store.requests_kept_from()? {
        Some(kept_from) if kept_from > since => kept_from,
        _ => since.clone(),
    };
    if data_starts > until {
        return Ok(InsightsResponse {
            schema_version: SCHEMA_VERSION,
            today: today.to_string(),
            since,
            until,
            data_starts,
            weeks: Vec::new(),
            skills: Vec::new(),
            agents: Vec::new(),
        });
    }
    let weeks = store.mondays_between(&data_starts, &until)?;
    let mut rows: Rows = BTreeMap::new();
    for count in store.run_counts(&data_starts, &until)? {
        if let Some(week) = week_of(&mut rows, &weeks, count.kind, &count.name, &count.week) {
            week.runs += count.runs;
        }
    }
    for spend in store.run_spend(&data_starts, &until)? {
        if let Some(week) = week_of(&mut rows, &weeks, spend.kind, &spend.name, &spend.week) {
            week.cost_usd += spend.cost_usd;
            week.unpriced += spend.unpriced;
            let cost = spend.cost_usd;
            let (model, effort, version) = if spend.in_subagent {
                week.subagent_cost_usd += cost;
                (
                    &mut week.subagent_model,
                    &mut week.subagent_effort,
                    &mut week.subagent_version,
                )
            } else {
                (&mut week.model, &mut week.effort, &mut week.version)
            };
            *model.entry(Some(spend.model)).or_default() += cost;
            *effort.entry(spend.effort).or_default() += cost;
            *version.entry(spend.version).or_default() += cost;
        }
    }
    let mut skills = Vec::new();
    let mut agents = Vec::new();
    for ((kind, name), spends) in rows {
        let row = insight_row(name, &spends, &weeks);
        match kind {
            RunKind::Skill => skills.push(row),
            RunKind::Agent => agents.push(row),
        }
    }
    add_facts(
        &mut skills,
        &mut agents,
        store.run_facts(&data_starts, &until)?,
    );
    sort_by_cost(&mut skills);
    sort_by_cost(&mut agents);
    Ok(InsightsResponse {
        schema_version: SCHEMA_VERSION,
        today: today.to_string(),
        since,
        until,
        data_starts,
        weeks,
        skills,
        agents,
    })
}

pub type Mix = BTreeMap<Option<String>, f64>;

#[derive(Debug, Clone, Default)]
pub struct WeekSpend {
    pub runs: u64,
    pub cost_usd: f64,
    pub unpriced: u64,
    pub model: Mix,
    pub effort: Mix,
    pub version: Mix,
    pub subagent_cost_usd: f64,
    pub subagent_model: Mix,
    pub subagent_effort: Mix,
    pub subagent_version: Mix,
}

type Rows = BTreeMap<(RunKind, String), Vec<WeekSpend>>;

fn week_of<'a>(
    rows: &'a mut Rows,
    weeks: &[String],
    kind: RunKind,
    name: &str,
    week: &str,
) -> Option<&'a mut WeekSpend> {
    let index = weeks.iter().position(|monday| monday == week)?;
    let spends = rows
        .entry((kind, name.to_string()))
        .or_insert_with(|| vec![WeekSpend::default(); weeks.len()]);
    Some(&mut spends[index])
}

fn insight_row(name: String, spends: &[WeekSpend], weeks: &[String]) -> InsightRow {
    let runs = spends.iter().map(|week| week.runs).sum();
    let cost_usd = spends.iter().map(|week| week.cost_usd).sum();
    InsightRow {
        name,
        runs,
        cost_usd,
        cost_per_run_usd: per_run(cost_usd, runs),
        subagent_cost_usd: spends.iter().map(|week| week.subagent_cost_usd).sum(),
        first_request_cost_usd: None,
        facts: None,
        weeks: weeks
            .iter()
            .zip(spends)
            .map(|(week, spend)| WeekRuns {
                week: week.clone(),
                runs: spend.runs,
                cost_usd: spend.cost_usd,
                cost_per_run_usd: per_run(spend.cost_usd, spend.runs),
            })
            .collect(),
        shifts: find_shifts(spends, weeks),
    }
}

fn add_facts(skills: &mut [InsightRow], agents: &mut [InsightRow], facts: RunFactRows) {
    for skill in skills.iter_mut() {
        let sessions = facts
            .repeats
            .iter()
            .find(|repeats| repeats.skill == skill.name);
        let mut launches: Vec<LaunchFacts> = facts
            .launches
            .iter()
            .filter(|launch| launch.skill.as_ref() == Some(&skill.name))
            .map(|launch| LaunchFacts {
                agent: launch.agent.clone(),
                launches: launch.launches,
                per_run: (skill.runs > 0).then(|| launch.launches as f64 / skill.runs as f64),
                cost_usd: launch.cost_usd,
                first_request_cost_usd: launch.first_request_cost_usd,
            })
            .collect();
        launches.sort_by(|a, b| b.launches.cmp(&a.launches).then(a.agent.cmp(&b.agent)));
        let session_costs: Vec<f64> = facts
            .session_costs
            .iter()
            .filter(|session| session.skill == skill.name)
            .map(|session| session.cost_usd)
            .collect();
        skill.facts = Some(RunFacts {
            runs_per_session: sessions
                .filter(|repeats| repeats.sessions > 0)
                .map(|repeats| skill.runs as f64 / repeats.sessions as f64),
            repeat_run_cost_usd: sessions.map_or(0.0, |repeats| repeats.repeat_run_cost_usd),
            top_tenth_session_share: top_tenth_share(session_costs),
            launches,
        });
    }
    for agent in agents.iter_mut() {
        agent.first_request_cost_usd = Some(
            facts
                .launches
                .iter()
                .filter(|launch| launch.agent == agent.name)
                .map(|launch| launch.first_request_cost_usd)
                .sum(),
        );
    }
}

fn top_tenth_share(mut session_costs: Vec<f64>) -> Option<f64> {
    let total: f64 = session_costs.iter().sum();
    if total <= 0.0 {
        return None;
    }
    session_costs.sort_by(|a, b| b.total_cmp(a));
    let top = session_costs.len().div_ceil(10);
    Some(session_costs[..top].iter().sum::<f64>() / total)
}

fn per_run(cost_usd: f64, runs: u64) -> Option<f64> {
    (runs > 0).then(|| cost_usd / runs as f64)
}

fn sort_by_cost(rows: &mut [InsightRow]) {
    rows.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd).then(a.name.cmp(&b.name)));
}

const SIDE_WEEKS: usize = 2;
const MIN_RUNS: u64 = 10;
const MIN_CHANGE: f64 = 0.30;

struct Side {
    runs: u64,
    cost_usd: f64,
    unpriced: u64,
    model: Mix,
    effort: Mix,
    version: Mix,
    subagent_model: Mix,
    subagent_effort: Mix,
    subagent_version: Mix,
}

impl Side {
    fn of(weeks: &[WeekSpend]) -> Side {
        let mut side = Side {
            runs: 0,
            cost_usd: 0.0,
            unpriced: 0,
            model: Mix::new(),
            effort: Mix::new(),
            version: Mix::new(),
            subagent_model: Mix::new(),
            subagent_effort: Mix::new(),
            subagent_version: Mix::new(),
        };
        for week in weeks {
            side.runs += week.runs;
            side.cost_usd += week.cost_usd;
            side.unpriced += week.unpriced;
            add_mix(&mut side.model, &week.model);
            add_mix(&mut side.effort, &week.effort);
            add_mix(&mut side.version, &week.version);
            add_mix(&mut side.subagent_model, &week.subagent_model);
            add_mix(&mut side.subagent_effort, &week.subagent_effort);
            add_mix(&mut side.subagent_version, &week.subagent_version);
        }
        side
    }

    fn cost_per_run(&self) -> f64 {
        self.cost_usd / self.runs as f64
    }
}

fn add_mix(into: &mut Mix, from: &Mix) {
    for (key, cost) in from {
        *into.entry(key.clone()).or_default() += cost;
    }
}

fn largest_share(mix: &Mix) -> Option<String> {
    mix.iter()
        .fold(
            None,
            |best: Option<(&Option<String>, f64)>, (key, &cost)| match best {
                Some((_, best_cost)) if best_cost >= cost => best,
                _ => Some((key, cost)),
            },
        )
        .and_then(|(key, _)| key.clone())
}

pub fn find_shifts(spends: &[WeekSpend], weeks: &[String]) -> Vec<Shift> {
    let mut flagged: Vec<(usize, Shift)> = Vec::new();
    for boundary in SIDE_WEEKS..=spends.len().saturating_sub(SIDE_WEEKS) {
        let before = Side::of(&spends[boundary - SIDE_WEEKS..boundary]);
        let after = Side::of(&spends[boundary..boundary + SIDE_WEEKS]);
        if before.runs < MIN_RUNS
            || after.runs < MIN_RUNS
            || before.cost_usd <= 0.0
            || before.unpriced > 0
            || after.unpriced > 0
        {
            continue;
        }
        let change = (after.cost_per_run() - before.cost_per_run()) / before.cost_per_run();
        if change.abs() < MIN_CHANGE - 1e-9 {
            continue;
        }
        let shift = Shift {
            week: weeks[boundary].clone(),
            before_cost_per_run_usd: before.cost_per_run(),
            after_cost_per_run_usd: after.cost_per_run(),
            before_runs: before.runs,
            after_runs: after.runs,
            change,
            causes: causes(&before, &after),
        };
        match flagged.last_mut() {
            Some((last, kept))
                if *last + 1 == boundary && (kept.change > 0.0) == (shift.change > 0.0) =>
            {
                if shift.change.abs() > kept.change.abs() {
                    *kept = shift;
                }
                *last = boundary;
            }
            _ => flagged.push((boundary, shift)),
        }
    }
    flagged.into_iter().map(|(_, shift)| shift).collect()
}

fn causes(before: &Side, after: &Side) -> Vec<ShiftCause> {
    [
        (Setting::Model, &before.model, &after.model, false),
        (Setting::Effort, &before.effort, &after.effort, false),
        (Setting::Version, &before.version, &after.version, false),
        (
            Setting::Model,
            &before.subagent_model,
            &after.subagent_model,
            true,
        ),
        (
            Setting::Effort,
            &before.subagent_effort,
            &after.subagent_effort,
            true,
        ),
        (
            Setting::Version,
            &before.subagent_version,
            &after.subagent_version,
            true,
        ),
    ]
    .into_iter()
    .filter(|(_, before, after, _)| !before.is_empty() && !after.is_empty())
    .filter_map(|(setting, before, after, in_subagents)| {
        let (from, to) = (largest_share(before), largest_share(after));
        (from != to).then_some(ShiftCause {
            setting,
            from,
            to,
            in_subagents,
        })
    })
    .collect()
}

pub fn render(response: &InsightsResponse) -> String {
    let mut out = String::new();
    let first = if response.weeks.is_empty() {
        &response.since
    } else {
        &response.data_starts
    };
    let _ = writeln!(out, "Cost per run, {first} to {}", response.until);
    if response.data_starts != response.since {
        let _ = writeln!(
            out,
            "Request detail before {} is no longer kept.",
            response.data_starts
        );
    } else if response.weeks.is_empty() {
        let _ = writeln!(
            out,
            "{} is after {}, so there's nothing to show.",
            response.since, response.until
        );
    }
    if response.weeks.is_empty() {
        return out;
    }
    let notes = takeaways(response);
    let width = response
        .skills
        .iter()
        .chain(&response.agents)
        .map(|row| row.name.chars().count())
        .max()
        .unwrap_or(0)
        .max(16);
    for (heading, rows) in [("skill", &response.skills), ("subagent", &response.agents)] {
        let _ = writeln!(out);
        if rows.is_empty() {
            let _ = writeln!(out, "No {heading} runs in this range.");
            continue;
        }
        let _ = write!(
            out,
            "{heading:<width$} {:>6} {:>10} {:>9}",
            "runs", "cost", "per run"
        );
        for week in &response.weeks {
            let _ = write!(out, " {:>8}", week.get(5..).unwrap_or(week));
        }
        let _ = writeln!(out);
        for row in rows {
            let _ = write!(
                out,
                "{:<width$} {:>6} {:>10} {:>9}",
                row.name,
                row.runs,
                format!("${:.2}", row.cost_usd),
                dollars(row.cost_per_run_usd)
            );
            for week in &row.weeks {
                let _ = write!(out, " {:>8}", dollars(week.cost_per_run_usd));
            }
            let _ = writeln!(out);
            if let Some(facts) = &row.facts {
                render_facts(&mut out, facts, row.cost_usd);
            }
            if let Some(row_notes) =
                notes.skills.iter().chain(&notes.agents).find(|entry| {
                    entry.name == row.name && entry.subagent == (heading == "subagent")
                })
            {
                render_row_notes(&mut out, row_notes);
            }
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Week columns are cost per run, by the Monday each week starts. A skill's cost includes the subagents it ran, so the two tables overlap."
    );
    let _ = writeln!(
        out,
        "Agents: read `gigawatts insights --help` before proposing changes."
    );
    out
}

fn render_facts(out: &mut String, facts: &RunFacts, cost_usd: f64) {
    let mut parts = Vec::new();
    if let Some(runs_per_session) = facts.runs_per_session {
        parts.push(format!("{runs_per_session:.1} runs a session"));
    }
    if cost_usd > 0.0 {
        parts.push(format!(
            "{:.0}% of cost in repeat runs",
            facts.repeat_run_cost_usd / cost_usd * 100.0
        ));
    }
    for launch in &facts.launches {
        if let Some(per_run) = launch.per_run {
            parts.push(format!("{per_run:.1} {} launches a run", launch.agent));
        }
    }
    if !parts.is_empty() {
        let _ = writeln!(out, "  {}", parts.join(", "));
    }
}

const DAYS_PER_MONTH: f64 = 365.25 / 12.0;
const MIN_MONTHLY_USD: f64 = 20.0;
const MIN_TAKEAWAY_RUNS: u64 = 10;

fn monthly_usd(shift: &Shift, response: &InsightsResponse) -> f64 {
    let after_days = match response.weeks.iter().position(|week| *week == shift.week) {
        Some(boundary) => (boundary..boundary + SIDE_WEEKS)
            .map(|index| days_covered(response, index))
            .sum(),
        None => SIDE_WEEKS as i64 * 7,
    };
    (shift.after_cost_per_run_usd - shift.before_cost_per_run_usd).abs()
        * shift.after_runs as f64
        * DAYS_PER_MONTH
        / after_days.max(1) as f64
}

fn days_covered(response: &InsightsResponse, index: usize) -> i64 {
    let Some(monday) = response.weeks.get(index) else {
        return 0;
    };
    match (
        day_number(monday),
        day_number(&response.data_starts),
        day_number(&response.until),
    ) {
        (Some(monday), Some(data_starts), Some(until)) => {
            (until.min(monday + 6) - data_starts.max(monday) + 1).max(0)
        }
        _ => 7,
    }
}

fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    Some(crate::config::days_from_civil(year, month, day))
}

pub fn takeaways(response: &InsightsResponse) -> TakeawaysResponse {
    let days: i64 = (0..response.weeks.len())
        .map(|index| days_covered(response, index))
        .sum();
    let months = days.max(1) as f64 / DAYS_PER_MONTH;
    let ranked = |rows: &[InsightRow], subagent: bool| {
        let mut ranked: Vec<RowTakeaways> = rows
            .iter()
            .map(|row| {
                let takeaways = row_takeaways(row, subagent, months, response);
                RowTakeaways {
                    name: row.name.clone(),
                    subagent,
                    to_save_usd: takeaways
                        .iter()
                        .filter_map(|item| item.monthly_usd)
                        .reduce(f64::max),
                    takeaways,
                    changes: row_changes(row, subagent, response),
                }
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.to_save_usd
                .unwrap_or(-1.0)
                .total_cmp(&a.to_save_usd.unwrap_or(-1.0))
        });
        ranked
    };
    TakeawaysResponse {
        skills: ranked(&response.skills, false),
        agents: ranked(&response.agents, true),
        events: change_events(response),
    }
}

fn rows_with_kind(response: &InsightsResponse) -> impl Iterator<Item = (&InsightRow, bool)> {
    let skills = response.skills.iter().map(|row| (row, false));
    let agents = response.agents.iter().map(|row| (row, true));
    skills.chain(agents)
}

fn takeaway(title: String, detail: String, monthly_usd: Option<f64>) -> Takeaway {
    Takeaway {
        title,
        detail,
        monthly_usd,
    }
}

fn row_takeaways(
    row: &InsightRow,
    subagent: bool,
    months: f64,
    response: &InsightsResponse,
) -> Vec<Takeaway> {
    if row.runs < MIN_TAKEAWAY_RUNS {
        let plural = if row.runs == 1 { "" } else { "s" };
        return vec![takeaway(
            "Too few runs to judge".into(),
            format!("Only {} run{plural} in these weeks.", row.runs),
            None,
        )];
    }
    let monthly = |usd: f64| usd / months;
    let mut found = Vec::new();
    if let Some(facts) = &row.facts
        && row.cost_usd > 0.0
    {
        let share = facts.repeat_run_cost_usd / row.cost_usd;
        if share >= 0.25 && monthly(facts.repeat_run_cost_usd) >= MIN_MONTHLY_USD {
            found.push(takeaway(
                format!("Repeat runs in the same session ({})", share_text(share)),
                format!(
                    "{} of its cost goes to running {} again in the same session, at {:.1} runs a session. Check whether a later run redoes work that didn't change, and have it look only at what's new.",
                    share_text(share),
                    row.name,
                    facts.runs_per_session.unwrap_or(0.0)
                ),
                Some(monthly(facts.repeat_run_cost_usd)),
            ));
        }
        for launch in &facts.launches {
            if let Some(per_run) = launch.per_run
                && per_run >= 1.5
                && monthly(launch.cost_usd) >= MIN_MONTHLY_USD
            {
                found.push(takeaway(
                    format!("{per_run:.1} {} launches a run", launch.agent),
                    format!(
                        "Each run launches {per_run:.1} {} subagents. Check whether every run needs that many, such as one on small inputs.",
                        launch.agent
                    ),
                    Some(monthly(launch.cost_usd)),
                ));
            }
            if launch.cost_usd > 0.0 {
                let share = launch.first_request_cost_usd / launch.cost_usd;
                if share >= 0.25 && monthly(launch.first_request_cost_usd) >= MIN_MONTHLY_USD {
                    found.push(takeaway(
                        format!(
                            "{}'s first requests are {} of its cost",
                            launch.agent,
                            share_text(share)
                        ),
                        format!(
                            "Each {} launch pays first for the context it starts with. Check what {} hands it: a path or a slice instead of the whole input.",
                            launch.agent, row.name
                        ),
                        Some(monthly(launch.first_request_cost_usd)),
                    ));
                }
            }
        }
        let sessions = facts
            .runs_per_session
            .filter(|per| *per > 0.0)
            .map_or(0.0, |per| row.runs as f64 / per);
        if let Some(top) = facts.top_tenth_session_share
            && top >= 0.5
            && sessions >= 10.0
            && monthly(row.cost_usd * top) >= MIN_MONTHLY_USD
        {
            found.push(takeaway(
                format!("Costliest tenth of sessions spent {}", share_text(top)),
                "A few sessions cost far more than the rest. Open them from Where it went, on the Skill tab, to see what they did differently.".into(),
                Some(monthly(row.cost_usd * top)),
            ));
        }
    }
    if subagent
        && let Some(first) = row.first_request_cost_usd
        && row.cost_usd > 0.0
        && first / row.cost_usd >= 0.25
        && monthly(first) >= MIN_MONTHLY_USD
    {
        found.push(takeaway(
            format!(
                "First requests are {} of its cost",
                share_text(first / row.cost_usd)
            ),
            "Each launch pays first for the context it starts with. Check what the skills that launch it hand it: a path or a slice instead of the whole input.".into(),
            Some(monthly(first)),
        ));
    }
    for shift in &row.shifts {
        let size = monthly_usd(shift, response);
        if shift.change < 0.0
            || size < MIN_MONTHLY_USD
            || reversal(row, subagent, shift, response).is_some()
        {
            continue;
        }
        let percent = format!("{:.0}%", shift.change * 100.0);
        let date = short_date(&shift.week);
        let cause = change_cause(&shift.causes, subagent);
        let (title, detail) = if settings_changed(&shift.causes).next().is_some() {
            (
                format!(
                    "Cost per run up {percent} since {date}, {}",
                    lowercase_first(&cause)
                ),
                "If that change wasn't on purpose, setting it back would save this.".to_string(),
            )
        } else {
            (
                format!("Cost per run up {percent} since {date}, no model or effort change"),
                format!(
                    "Nothing in the model or effort explains it. Check edits to the {} around {date}.",
                    if subagent { "subagent" } else { "skill" }
                ),
            )
        };
        found.push(takeaway(title, detail, Some(size)));
    }
    if found.is_empty() {
        return vec![takeaway(
            "Nothing stands out".into(),
            "No single pattern takes a large share of its cost, and its cost per run hasn't risen enough to flag.".into(),
            None,
        )];
    }
    found.sort_by(|a, b| {
        b.monthly_usd
            .unwrap_or(0.0)
            .total_cmp(&a.monthly_usd.unwrap_or(0.0))
    });
    found
}

fn settings_changed(causes: &[ShiftCause]) -> impl Iterator<Item = &ShiftCause> {
    causes
        .iter()
        .filter(|cause| cause.setting != Setting::Version)
}

fn reversed_by<'a>(shift: &Shift, shifts: &[&'a Shift]) -> Option<&'a str> {
    let changed: Vec<&ShiftCause> = settings_changed(&shift.causes).collect();
    if changed.is_empty() {
        return None;
    }
    shifts
        .iter()
        .filter(|later| {
            later.week > shift.week
                && changed.iter().all(|cause| {
                    later.causes.iter().any(|back| {
                        back.setting == cause.setting
                            && back.from == cause.to
                            && back.to == cause.from
                    })
                })
        })
        .map(|later| later.week.as_str())
        .min()
}

fn sharers<'a>(
    row: &InsightRow,
    subagent: bool,
    shift: &Shift,
    response: &'a InsightsResponse,
) -> Vec<(&'a InsightRow, bool)> {
    rows_with_kind(response)
        .filter(|(other, _)| other.name != row.name)
        .filter(|&(other, other_subagent)| {
            other
                .shifts
                .iter()
                .any(|theirs| same_change((row, subagent, shift), (other, other_subagent, theirs)))
        })
        .collect()
}

fn reversal(
    row: &InsightRow,
    subagent: bool,
    shift: &Shift,
    response: &InsightsResponse,
) -> Option<String> {
    if shift.change <= 0.0 {
        return None;
    }
    let back_down = row
        .shifts
        .iter()
        .filter(|later| {
            later.week > shift.week && later.after_cost_per_run_usd <= shift.before_cost_per_run_usd
        })
        .map(|later| later.week.as_str())
        .min();
    let mut pool: Vec<&Shift> = row.shifts.iter().collect();
    for (other, _) in sharers(row, subagent, shift, response) {
        pool.extend(other.shifts.iter());
    }
    [back_down, reversed_by(shift, &pool)]
        .into_iter()
        .flatten()
        .min()
        .map(String::from)
}

fn change_events(response: &InsightsResponse) -> Vec<ChangeEvent> {
    let mut groups: Vec<(Vec<RowShift>, bool, ChangeEvent)> = Vec::new();
    for (row, subagent) in rows_with_kind(response) {
        for shift in &row.shifts {
            let size = monthly_usd(shift, response);
            if size < MIN_MONTHLY_USD {
                continue;
            }
            let rising = shift.change > 0.0;
            let member = EventRow {
                name: row.name.clone(),
                subagent,
            };
            let reversed_on = reversal(row, subagent, shift, response);
            let existing = groups.iter_mut().find(|(members, _, _)| {
                members
                    .iter()
                    .any(|&theirs| same_change((row, subagent, shift), theirs))
            });
            match existing {
                Some((members, _, event)) => {
                    members.push((row, subagent, shift));
                    event.rows.push(member);
                    if size > event.monthly_usd {
                        event.monthly_usd = size;
                        event.cause = change_cause(&shift.causes, subagent);
                    }
                    if event.reversed_on.is_none() {
                        event.reversed_on = reversed_on;
                    }
                }
                None => groups.push((
                    vec![(row, subagent, shift)],
                    rising,
                    ChangeEvent {
                        week: shift.week.clone(),
                        cause: change_cause(&shift.causes, subagent),
                        rows: vec![member],
                        effect: ChangeEffect::Costing,
                        monthly_usd: size,
                        reversed_on,
                        undid: None,
                    },
                )),
            }
        }
    }
    let mut events: Vec<ChangeEvent> = groups
        .into_iter()
        .map(|(_, rising, mut event)| {
            event.effect = match (rising, &event.reversed_on) {
                (false, _) => ChangeEffect::Saving,
                (true, Some(_)) => ChangeEffect::Reversed,
                (true, None) => ChangeEffect::Costing,
            };
            event
        })
        .collect();
    let reversed: Vec<ChangeEvent> = events
        .iter()
        .filter(|event| event.effect == ChangeEffect::Reversed)
        .cloned()
        .collect();
    for undone in reversed {
        let Some(week) = &undone.reversed_on else {
            continue;
        };
        let undoer = events.iter_mut().find(|event| {
            event.week == *week
                && event.effect == ChangeEffect::Saving
                && event.undid.is_none()
                && event
                    .rows
                    .iter()
                    .any(|row| undone.rows.iter().any(|other| other.name == row.name))
        });
        if let Some(undoer) = undoer {
            undoer.undid = Some(UndoneChange {
                week: undone.week.clone(),
                cause: undone.cause.clone(),
            });
            events.retain(|event| event != &undone);
        }
    }
    events.sort_by(|a, b| {
        b.week
            .cmp(&a.week)
            .then(b.monthly_usd.total_cmp(&a.monthly_usd))
    });
    events
}

fn setting_text(cause: &ShiftCause) -> String {
    match (cause.setting, &cause.to) {
        (Setting::Model, Some(model)) => pretty_model(model),
        (Setting::Model, None) => "no recorded model".to_string(),
        (_, Some(effort)) => format!("{effort} effort"),
        (_, None) => "no effort setting".to_string(),
    }
}

fn change_cause(causes: &[ShiftCause], subagent: bool) -> String {
    let (own, subagents): (Vec<&ShiftCause>, Vec<&ShiftCause>) =
        settings_changed(causes).partition(|cause| subagent || !cause.in_subagents);
    let list = |causes: &[&ShiftCause]| -> String {
        causes
            .iter()
            .map(|cause| setting_text(cause))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (own.is_empty(), subagents.is_empty()) {
        (true, true) => "No model or effort change".to_string(),
        (false, true) => format!("Moved to {}", list(&own)),
        (true, false) => format!("Subagents moved to {}", list(&subagents)),
        (false, false) => format!("Moved to {}; subagents to {}", list(&own), list(&subagents)),
    }
}

type CauseKey = Vec<(&'static str, Option<String>)>;

fn cause_key(causes: &[ShiftCause]) -> CauseKey {
    let mut key: CauseKey = settings_changed(causes)
        .map(|cause| {
            let setting = if cause.setting == Setting::Model {
                "model"
            } else {
                "effort"
            };
            (setting, cause.to.clone())
        })
        .collect();
    key.sort();
    key.dedup();
    key
}

fn launches(skill: &InsightRow, agent: &InsightRow) -> bool {
    skill.facts.as_ref().is_some_and(|facts| {
        facts
            .launches
            .iter()
            .any(|launch| launch.agent == agent.name)
    })
}

fn linked(row: &InsightRow, subagent: bool, other: &InsightRow, other_subagent: bool) -> bool {
    match (subagent, other_subagent) {
        (false, true) => launches(row, other),
        (true, false) => launches(other, row),
        _ => false,
    }
}

type RowShift<'a> = (&'a InsightRow, bool, &'a Shift);

fn same_change(
    (row, subagent, shift): RowShift,
    (other, other_subagent, theirs): RowShift,
) -> bool {
    let key = cause_key(&shift.causes);
    theirs.week == shift.week
        && (theirs.change < 0.0) == (shift.change < 0.0)
        && cause_key(&theirs.causes) == key
        && (!key.is_empty() || linked(row, subagent, other, other_subagent))
}

fn row_changes(row: &InsightRow, subagent: bool, response: &InsightsResponse) -> Vec<Change> {
    let mut changes: Vec<Change> = row
        .shifts
        .iter()
        .filter(|shift| monthly_usd(shift, response) >= MIN_MONTHLY_USD)
        .map(|shift| {
            let reversed_on = reversal(row, subagent, shift, response);
            let effect = match (&reversed_on, shift.change < 0.0) {
                (_, true) => ChangeEffect::Saving,
                (Some(_), false) => ChangeEffect::Reversed,
                (None, false) => ChangeEffect::Costing,
            };
            let shared_with = sharers(row, subagent, shift, response)
                .into_iter()
                .map(|(other, _)| other.name.clone())
                .collect();
            Change {
                week: shift.week.clone(),
                cause: change_cause(&shift.causes, subagent),
                before_cost_per_run_usd: shift.before_cost_per_run_usd,
                after_cost_per_run_usd: shift.after_cost_per_run_usd,
                change: shift.change,
                monthly_usd: monthly_usd(shift, response),
                effect,
                reversed_on,
                shared_with,
            }
        })
        .collect();
    changes.sort_by(|a, b| b.week.cmp(&a.week));
    changes
}

fn effect_text(change: &Change) -> String {
    match (change.effect, &change.reversed_on) {
        (ChangeEffect::Reversed, Some(week)) => format!("reversed {}", short_date(week)),
        (ChangeEffect::Saving, _) => format!("saving {} a month", usd(change.monthly_usd)),
        _ => format!("costing {} a month", usd(change.monthly_usd)),
    }
}

fn render_row_notes(out: &mut String, notes: &RowTakeaways) {
    let sized: Vec<&Takeaway> = notes
        .takeaways
        .iter()
        .filter(|item| item.monthly_usd.is_some())
        .collect();
    if let Some(first) = sized.first() {
        let more = match sized.len() - 1 {
            0 => String::new(),
            more => format!(" (+{more} more)"),
        };
        let _ = writeln!(
            out,
            "  At stake: {}, about {} a month{more}",
            first.title,
            usd(first.monthly_usd.unwrap_or(0.0))
        );
    }
    for change in &notes.changes {
        let _ = writeln!(
            out,
            "  {}: {}, ${:.2} to ${:.2} a run, {}",
            short_date(&change.week),
            lowercase_first(&change.cause),
            change.before_cost_per_run_usd,
            change.after_cost_per_run_usd,
            effect_text(change)
        );
    }
}

pub fn pretty_model(label: &str) -> String {
    let Some(rest) = label.strip_prefix("claude-") else {
        return label.to_string();
    };
    let mut parts = rest.split('-');
    let (Some(family), Some(major)) = (parts.next(), parts.next()) else {
        return label.to_string();
    };
    if family.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) || major.is_empty() {
        return label.to_string();
    }
    let minor = parts
        .next()
        .filter(|part| part.len() == 1 && part.chars().all(|c| c.is_ascii_digit()));
    let mut name = capitalized(family);
    name.push(' ');
    name.push_str(major);
    if let Some(minor) = minor {
        name.push('.');
        name.push_str(minor);
    }
    name
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn short_date(day: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = day.get(5..7).and_then(|month| month.parse::<usize>().ok());
    let date = day.get(8..10).and_then(|date| date.parse::<u32>().ok());
    match (month, date) {
        (Some(month @ 1..=12), Some(date)) => format!("{} {date}", MONTHS[month - 1]),
        _ => day.to_string(),
    }
}

fn share_text(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

fn usd(value: f64) -> String {
    let whole = value.round() as i64;
    let digits = whole.abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{}${grouped}", if whole < 0 { "-" } else { "" })
}

fn dollars(cost: Option<f64>) -> String {
    cost.map_or("-".to_string(), |cost| format!("${cost:.2}"))
}
