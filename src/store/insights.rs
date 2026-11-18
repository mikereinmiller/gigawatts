use super::{Result, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunKind {
    Skill,
    Agent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunCount {
    pub kind: RunKind,
    pub name: String,
    pub week: String,
    pub runs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunSpend {
    pub kind: RunKind,
    pub name: String,
    pub week: String,
    pub model: String,
    pub effort: Option<String>,
    pub version: Option<String>,
    pub in_subagent: bool,
    pub cost_usd: f64,
    pub unpriced: u64,
}

const WEEK: &str = "date(day, '-6 days', 'weekday 1')";
const SESSIONS_IN_RANGE: &str =
    "session_id IN (SELECT session_id FROM requests WHERE day BETWEEN ?1 AND ?2)";

fn kind_from(text: &str) -> RunKind {
    if text == "skill" {
        RunKind::Skill
    } else {
        RunKind::Agent
    }
}

impl Store {
    pub fn run_counts(&self, since: &str, until: &str) -> Result<Vec<RunCount>> {
        let sql = format!(
            "WITH ordered AS (
                SELECT skill_label, day,
                       LAG(skill_label) OVER (PARTITION BY session_id ORDER BY time_ms, id)
                           AS previous
                FROM requests
                WHERE agent_id IS NULL AND time_ms IS NOT NULL AND {SESSIONS_IN_RANGE}
             ),
             skill_runs AS (
                SELECT 'skill' AS kind, skill_label AS label, day FROM ordered
                WHERE skill_label IS NOT NULL AND previous IS NOT skill_label
             ),
             agent_runs AS (
                SELECT 'agent' AS kind, agent_label AS label, min(day) AS day FROM requests
                WHERE agent_id IS NOT NULL AND agent_label IS NOT NULL AND {SESSIONS_IN_RANGE}
                GROUP BY session_id, agent_id, agent_label
             ),
             runs AS (SELECT * FROM skill_runs UNION ALL SELECT * FROM agent_runs)
             SELECT runs.kind, labels.text, {WEEK}, count(*)
             FROM runs JOIN labels ON labels.id = runs.label
             WHERE day BETWEEN ?1 AND ?2
             GROUP BY runs.kind, labels.text, {WEEK}"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let counts = statement.query_map([since, until], |row| {
            Ok(RunCount {
                kind: kind_from(&row.get::<_, String>(0)?),
                name: row.get(1)?,
                week: row.get(2)?,
                runs: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(counts.collect::<rusqlite::Result<_>>()?)
    }

    pub fn run_spend(&self, since: &str, until: &str) -> Result<Vec<RunSpend>> {
        let sql = format!(
            "WITH spend AS (
                SELECT 'skill' AS kind, skill_label AS label, day, model_label, effort_label,
                       version_label, agent_id IS NOT NULL AS in_subagent, cost_usd
                FROM requests WHERE skill_label IS NOT NULL AND day BETWEEN ?1 AND ?2
                UNION ALL
                SELECT 'agent', agent_label, day, model_label, effort_label, version_label, 0,
                       cost_usd
                FROM requests WHERE agent_label IS NOT NULL AND day BETWEEN ?1 AND ?2
             )
             SELECT spend.kind, name.text, {WEEK}, model.text, effort.text, version.text,
                    spend.in_subagent, total(spend.cost_usd), count(*) - count(spend.cost_usd)
             FROM spend
             JOIN labels AS name ON name.id = spend.label
             JOIN labels AS model ON model.id = spend.model_label
             LEFT JOIN labels AS effort ON effort.id = spend.effort_label
             LEFT JOIN labels AS version ON version.id = spend.version_label
             GROUP BY spend.kind, name.text, {WEEK}, model.text, effort.text, version.text,
                      spend.in_subagent"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let spend = statement.query_map([since, until], |row| {
            Ok(RunSpend {
                kind: kind_from(&row.get::<_, String>(0)?),
                name: row.get(1)?,
                week: row.get(2)?,
                model: row.get(3)?,
                effort: row.get(4)?,
                version: row.get(5)?,
                in_subagent: row.get(6)?,
                cost_usd: row.get(7)?,
                unpriced: row.get::<_, i64>(8)? as u64,
            })
        })?;
        Ok(spend.collect::<rusqlite::Result<_>>()?)
    }

    pub fn week_start_weeks_before(&self, day: &str, weeks: u32) -> Result<String> {
        let monday = self.connection.query_row(
            "SELECT date(?1, '-6 days', 'weekday 1', ?2)",
            rusqlite::params![day, format!("-{} days", weeks * 7)],
            |row| row.get(0),
        )?;
        Ok(monday)
    }

    pub fn mondays_between(&self, since: &str, until: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare_cached(
            "WITH RECURSIVE week (monday) AS (
                SELECT date(?1, '-6 days', 'weekday 1')
                UNION ALL
                SELECT date(monday, '+7 days') FROM week
                WHERE date(monday, '+7 days') <= ?2
             )
             SELECT monday FROM week",
        )?;
        let mondays = statement.query_map([since, until], |row| row.get(0))?;
        Ok(mondays.collect::<rusqlite::Result<_>>()?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillRepeats {
    pub skill: String,
    pub sessions: u64,
    pub repeat_run_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpend {
    pub skill: Option<String>,
    pub agent: String,
    pub launches: u64,
    pub cost_usd: f64,
    pub first_request_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillSessionCost {
    pub skill: String,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunFactRows {
    pub repeats: Vec<SkillRepeats>,
    pub launches: Vec<LaunchSpend>,
    pub session_costs: Vec<SkillSessionCost>,
}

impl Store {
    pub fn run_facts(&self, since: &str, until: &str) -> Result<RunFactRows> {
        Ok(RunFactRows {
            repeats: self.skill_repeats(since, until)?,
            launches: self.launch_spend(since, until)?,
            session_costs: self.skill_session_costs(since, until)?,
        })
    }

    fn skill_repeats(&self, since: &str, until: &str) -> Result<Vec<SkillRepeats>> {
        let sql = format!(
            "WITH main AS (
                SELECT id, LAG(skill_label) OVER (PARTITION BY session_id ORDER BY time_ms, id)
                           AS previous
                FROM requests
                WHERE agent_id IS NULL AND time_ms IS NOT NULL AND {SESSIONS_IN_RANGE}
             ),
             labeled AS (
                SELECT requests.session_id, requests.skill_label, requests.time_ms, requests.id,
                       requests.day, requests.cost_usd,
                       main.id IS NOT NULL AND main.previous IS NOT requests.skill_label
                           AS starts_run
                FROM requests LEFT JOIN main ON main.id = requests.id
                WHERE requests.skill_label IS NOT NULL AND requests.time_ms IS NOT NULL
                  AND requests.{SESSIONS_IN_RANGE}
             ),
             numbered AS (
                SELECT *, sum(starts_run) OVER (
                           PARTITION BY session_id, skill_label ORDER BY time_ms, id
                       ) AS run
                FROM labeled
             )
             SELECT labels.text,
                    count(DISTINCT CASE WHEN starts_run THEN session_id END),
                    total(CASE WHEN run > 1 THEN cost_usd END)
             FROM numbered JOIN labels ON labels.id = numbered.skill_label
             WHERE day BETWEEN ?1 AND ?2
             GROUP BY labels.text"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let repeats = statement.query_map([since, until], |row| {
            Ok(SkillRepeats {
                skill: row.get(0)?,
                sessions: row.get::<_, i64>(1)? as u64,
                repeat_run_cost_usd: row.get(2)?,
            })
        })?;
        Ok(repeats.collect::<rusqlite::Result<_>>()?)
    }

    fn launch_spend(&self, since: &str, until: &str) -> Result<Vec<LaunchSpend>> {
        let sql = format!(
            "WITH ordered AS (
                SELECT session_id, agent_id, agent_label, skill_label, day, cost_usd,
                       ROW_NUMBER() OVER (
                           PARTITION BY session_id, agent_id
                           ORDER BY time_ms IS NULL, time_ms, id
                       ) AS position
                FROM requests
                WHERE agent_id IS NOT NULL AND agent_label IS NOT NULL AND {SESSIONS_IN_RANGE}
             ),
             launches AS (
                SELECT session_id, agent_id, agent_label, skill_label, day AS started,
                       cost_usd AS first_cost
                FROM ordered WHERE position = 1
             ),
             spent AS (
                SELECT session_id, agent_id, total(cost_usd) AS cost_usd FROM ordered
                WHERE day BETWEEN ?1 AND ?2
                GROUP BY session_id, agent_id
             )
             SELECT skill.text, agent.text, count(*), total(spent.cost_usd),
                    total(launches.first_cost)
             FROM launches
             JOIN labels AS agent ON agent.id = launches.agent_label
             LEFT JOIN labels AS skill ON skill.id = launches.skill_label
             LEFT JOIN spent USING (session_id, agent_id)
             WHERE started BETWEEN ?1 AND ?2
             GROUP BY skill.text, agent.text"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let launches = statement.query_map([since, until], |row| {
            Ok(LaunchSpend {
                skill: row.get(0)?,
                agent: row.get(1)?,
                launches: row.get::<_, i64>(2)? as u64,
                cost_usd: row.get(3)?,
                first_request_cost_usd: row.get(4)?,
            })
        })?;
        Ok(launches.collect::<rusqlite::Result<_>>()?)
    }

    fn skill_session_costs(&self, since: &str, until: &str) -> Result<Vec<SkillSessionCost>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT labels.text, total(cost_usd) FROM requests
             JOIN labels ON labels.id = requests.skill_label
             WHERE day BETWEEN ?1 AND ?2
             GROUP BY labels.text, session_id",
        )?;
        let costs = statement.query_map([since, until], |row| {
            Ok(SkillSessionCost {
                skill: row.get(0)?,
                cost_usd: row.get(1)?,
            })
        })?;
        Ok(costs.collect::<rusqlite::Result<_>>()?)
    }
}
