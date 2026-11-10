use std::collections::{HashMap, HashSet};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::App;
use super::sections::{KEY, MUTED, WARNING, section};
use crate::contract::{DataResponse, DataRow, GroupBy, Level, Range};
use crate::data;
use crate::sources::claude_json::Account;
use crate::store::{Store, TopItem, UsageAdvice};

const GROUPINGS: [GroupBy; 4] = [
    GroupBy::Branch,
    GroupBy::Skill,
    GroupBy::Agent,
    GroupBy::Model,
];
const RANGES: [Range; 4] = [Range::Today, Range::Week, Range::Month, Range::All];
const BAR_WIDTH: usize = 10;
const WIDE: u16 = 90;
const BRANCHES_PER_REPO: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryQuery {
    pub range: Range,
    pub group_by: GroupBy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spending {
    pub response: DataResponse,
    pub top: Vec<TopItem>,
}

enum Entry<'a> {
    Row(&'a DataRow),
    Repo {
        account: Option<String>,
        key: &'a str,
        name: &'a str,
        cost: f64,
        open: bool,
    },
    Branch(&'a DataRow, &'a str),
    OtherBranches {
        count: usize,
        cost: f64,
        largest: f64,
    },
}

struct RepoGroup<'a> {
    account: Option<String>,
    key: &'a str,
    name: &'a str,
    cost: f64,
    branches: Vec<(&'a DataRow, &'a str)>,
}

pub(crate) struct History {
    range: Range,
    group_by: GroupBy,
    selected: usize,
    open_repos: HashSet<(Option<String>, String)>,
    wanted: bool,
    loaded: Option<Result<Spending, String>>,
}

impl History {
    pub(crate) fn new() -> History {
        History {
            range: Range::Week,
            group_by: GroupBy::Branch,
            selected: 0,
            open_repos: HashSet::new(),
            wanted: true,
            loaded: None,
        }
    }

    pub(crate) fn take_query(&mut self) -> Option<HistoryQuery> {
        if !std::mem::take(&mut self.wanted) {
            return None;
        }
        Some(HistoryQuery {
            range: self.range,
            group_by: self.group_by,
        })
    }

    pub(crate) fn set(&mut self, query: &HistoryQuery, loaded: Result<Spending, String>) {
        if query.range == self.range && query.group_by == self.group_by {
            self.loaded = Some(loaded);
        }
    }

    pub(crate) fn reload(&mut self) {
        self.wanted = true;
    }

    pub(crate) fn next_grouping(&mut self, by: isize) {
        self.group_by = step(&GROUPINGS, self.group_by, by);
        self.loaded = None;
        self.selected = 0;
        self.wanted = true;
    }

    pub(crate) fn next_range(&mut self, by: isize) {
        self.range = step(&RANGES, self.range, by);
        self.selected = 0;
        self.wanted = true;
    }

    fn rows<'a>(&'a self, account: Option<&Option<String>>) -> Vec<&'a DataRow> {
        match &self.loaded {
            Some(Ok(spending)) => spending
                .response
                .rows
                .iter()
                .filter(|row| account.is_none_or(|account| &row.account == account))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn entries<'a>(&'a self, account: Option<&Option<String>>) -> Vec<Entry<'a>> {
        let rows = self.rows(account);
        if self.group_by != GroupBy::Branch {
            return rows.into_iter().map(Entry::Row).collect();
        }
        let mut repos: Vec<RepoGroup> = Vec::new();
        let mut missing = Vec::new();
        for row in rows {
            let (Some(key), Some((repo, branch))) = (&row.key, row.label.rsplit_once(" · "))
            else {
                missing.push(Entry::Row(row));
                continue;
            };
            let repo_key = key.split('\u{1f}').next().unwrap_or_default();
            let cost = row.cost_usd.unwrap_or(0.0);
            match repos
                .iter_mut()
                .find(|group| group.account == row.account && group.key == repo_key)
            {
                Some(group) => {
                    group.cost += cost;
                    group.branches.push((row, branch));
                }
                None => repos.push(RepoGroup {
                    account: row.account.clone(),
                    key: repo_key,
                    name: repo,
                    cost,
                    branches: vec![(row, branch)],
                }),
            }
        }
        repos.sort_by(|a, b| b.cost.total_cmp(&a.cost));
        let mut entries = Vec::new();
        for group in repos {
            let open = self
                .open_repos
                .contains(&(group.account.clone(), group.key.to_string()));
            entries.push(Entry::Repo {
                account: group.account,
                key: group.key,
                name: group.name,
                cost: group.cost,
                open,
            });
            if !open {
                continue;
            }
            let mut branches = group.branches.into_iter();
            entries.extend(
                branches
                    .by_ref()
                    .take(BRANCHES_PER_REPO)
                    .map(|(row, branch)| Entry::Branch(row, branch)),
            );
            let others: Vec<&DataRow> = branches.map(|(row, _)| row).collect();
            if !others.is_empty() {
                entries.push(Entry::OtherBranches {
                    count: others.len(),
                    cost: sum_cost(&others),
                    largest: others
                        .iter()
                        .filter_map(|row| row.cost_usd)
                        .fold(0.0, f64::max),
                });
            }
        }
        entries.extend(missing);
        entries
    }

    pub(crate) fn enter_does(&self, account: Option<&Option<String>>) -> Option<&'static str> {
        match self.entries(account).get(self.highlighted(account)) {
            Some(Entry::Repo { open: false, .. }) => Some("branches"),
            Some(Entry::Repo { open: true, .. }) => Some("hide"),
            _ => None,
        }
    }

    pub(crate) fn toggle(&mut self, account: Option<&Option<String>>) {
        let Some(Entry::Repo { account, key, .. }) = self
            .entries(account)
            .into_iter()
            .nth(self.highlighted(account))
        else {
            return;
        };
        let repo = (account, key.to_string());
        if !self.open_repos.remove(&repo) {
            self.open_repos.insert(repo);
        }
    }

    fn repos(&self, account: Option<&Option<String>>) -> Vec<(bool, (Option<String>, String))> {
        self.entries(account)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Repo {
                    account, key, open, ..
                } => Some((open, (account, key.to_string()))),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn every_repo_does(&self, account: Option<&Option<String>>) -> Option<&'static str> {
        let repos = self.repos(account);
        if repos.is_empty() {
            None
        } else if repos.iter().any(|(open, _)| !open) {
            Some("expand all")
        } else {
            Some("collapse all")
        }
    }

    pub(crate) fn toggle_every_repo(&mut self, account: Option<&Option<String>>) {
        let repos = self.repos(account);
        let highlighted = self.highlighted_repo(account);
        let open = repos.iter().any(|(open, _)| !open);
        for (_, repo) in repos {
            if open {
                self.open_repos.insert(repo);
            } else {
                self.open_repos.remove(&repo);
            }
        }
        if let Some(repo) = highlighted {
            let entries = self.entries(account);
            self.selected = entries
                .iter()
                .position(|entry| {
                    matches!(entry, Entry::Repo { account, key, .. }
                        if *account == repo.0 && *key == repo.1)
                })
                .unwrap_or(self.selected);
        }
    }

    fn highlighted_repo(
        &self,
        account: Option<&Option<String>>,
    ) -> Option<(Option<String>, String)> {
        self.entries(account)
            .into_iter()
            .take(self.highlighted(account) + 1)
            .filter_map(|entry| match entry {
                Entry::Repo { account, key, .. } => Some((account, key.to_string())),
                _ => None,
            })
            .next_back()
    }

    pub(crate) fn select(&mut self, by: isize, account: Option<&Option<String>>) {
        let last = self.entries(account).len().saturating_sub(1) as isize;
        self.selected = (self.highlighted(account) as isize + by).clamp(0, last) as usize;
    }

    fn highlighted(&self, account: Option<&Option<String>>) -> usize {
        self.selected
            .min(self.entries(account).len().saturating_sub(1))
    }

    pub(crate) fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        account: Option<&Option<String>>,
        labels: &HashMap<Option<String>, String>,
    ) {
        let rows = self.rows(account);
        let total = sum_cost(&rows);
        let spending = match &self.loaded {
            Some(Ok(spending)) => Some(spending),
            _ => None,
        };
        let block = section(&self.title(spending.map(|_| total)));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [tabs, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
        frame.render_widget(Paragraph::new(self.tabs(tabs.width)), tabs);
        let message = match &self.loaded {
            None => Some(Line::styled("Reading…", MUTED)),
            Some(Err(error)) => Some(Line::styled(error.clone(), WARNING)),
            Some(Ok(_)) if rows.is_empty() => Some(Line::styled("No spend in this range.", MUTED)),
            Some(Ok(_)) => None,
        };
        if let Some(message) = message {
            frame.render_widget(Paragraph::new(message), body);
            return;
        }
        let Some(spending) = spending else {
            return;
        };
        let level = spending.response.level;
        let every_account = account.is_none();
        let account_label = |account: &Option<String>, fallback: &str| {
            labels
                .get(account)
                .cloned()
                .unwrap_or_else(|| fallback.to_string())
        };
        let entries = self.entries(account);
        let indent = if self.group_by == GroupBy::Branch {
            "  "
        } else {
            ""
        };
        let right = |text: String| Cell::from(Text::from(text).alignment(Alignment::Right));
        let line = |name: Span<'static>, account_label: String, cost: f64, top: String| {
            let mut cells = vec![Cell::from(name)];
            if every_account {
                cells.push(Cell::from(Span::styled(account_label, MUTED)));
            }
            cells.push(right(format!("${cost:.2}")));
            cells.push(Cell::from(share_bar(cost, total)));
            cells.push(right(percent_of(cost, total)));
            cells.push(Cell::from(Span::styled(top, MUTED)));
            Row::new(cells)
        };
        let table_rows: Vec<Row> = entries
            .iter()
            .map(|entry| match entry {
                Entry::Row(row) => line(
                    Span::raw(format!("{indent}{}", data::row_name(row, level))),
                    account_label(&row.account, &row.account_label),
                    row.cost_usd.unwrap_or(0.0),
                    top_of(row, &spending.top),
                ),
                Entry::Repo {
                    account,
                    key,
                    name,
                    cost,
                    open,
                } => line(
                    Span::styled(
                        format!("{} {name}", if *open { "▾" } else { "▸" }),
                        Modifier::BOLD,
                    ),
                    account_label(account, ""),
                    *cost,
                    top_for(account, key, *cost, &spending.top),
                ),
                Entry::Branch(row, branch) => line(
                    Span::raw(format!("    - {branch}")),
                    String::new(),
                    row.cost_usd.unwrap_or(0.0),
                    top_of(row, &spending.top),
                ),
                Entry::OtherBranches {
                    count,
                    cost,
                    largest,
                } => line(
                    Span::styled(format!("    - {count} more, largest ${largest:.2}"), MUTED),
                    String::new(),
                    *cost,
                    String::new(),
                ),
            })
            .collect();
        let heading = |text: &str, alignment: Alignment| {
            Cell::from(Text::from(text.to_string()).alignment(alignment))
        };
        let mut header = vec![heading(level_heading(level), Alignment::Left)];
        let mut widths = vec![Constraint::Fill(1)];
        if every_account {
            header.push(heading("Account", Alignment::Left));
            widths.push(Constraint::Length(12));
        }
        header.push(heading("Cost", Alignment::Right));
        widths.push(Constraint::Length(10));
        header.push(heading("", Alignment::Left));
        widths.push(Constraint::Length(BAR_WIDTH as u16));
        header.push(heading("", Alignment::Right));
        widths.push(Constraint::Length(4));
        header.push(heading(top_heading(self.group_by), Alignment::Left));
        widths.push(Constraint::Length(if body.width >= WIDE { 32 } else { 24 }));
        let selected = self.selected.min(table_rows.len().saturating_sub(1));
        let table = Table::new(table_rows, widths)
            .header(Row::new(header).style(MUTED))
            .row_highlight_style(Modifier::REVERSED);
        let mut state = TableState::default().with_selected(Some(selected));
        frame.render_stateful_widget(table, body, &mut state);
    }

    fn title(&self, total: Option<f64>) -> String {
        let range = match self.range {
            Range::Today => "today",
            Range::Week => "this week",
            Range::Month => "this month",
            Range::All => "all time",
        };
        match total {
            Some(total) => format!("Spending · ${total:.2} {range}"),
            None => format!("Spending · {range}"),
        }
    }

    fn tabs(&self, width: u16) -> Line<'static> {
        let tab = |name: &'static str, on: bool| {
            if on {
                Span::styled(
                    format!(" {name} "),
                    KEY.add_modifier(Modifier::REVERSED | Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {name} "), Style::new())
            }
        };
        let mut left = vec![Span::styled("← → ", KEY.add_modifier(Modifier::BOLD))];
        for grouping in GROUPINGS {
            left.push(tab(tab_name(grouping), grouping == self.group_by));
        }
        let mut right = vec![Span::styled("t ", KEY.add_modifier(Modifier::BOLD))];
        for range in RANGES {
            let name = match range {
                Range::Today => "today",
                Range::Week => "week",
                Range::Month => "month",
                Range::All => "all",
            };
            right.push(tab(name, range == self.range));
        }
        let used: usize = left.iter().chain(&right).map(Span::width).sum();
        let mut spans = left;
        spans.push(Span::raw(
            " ".repeat((width as usize).saturating_sub(used).max(2)),
        ));
        spans.extend(right);
        Line::from(spans)
    }
}

fn step<T: Copy + PartialEq>(all: &[T], current: T, by: isize) -> T {
    let at = all.iter().position(|item| *item == current).unwrap_or(0) as isize;
    all[(at + by).rem_euclid(all.len() as isize) as usize]
}

fn tab_name(group_by: GroupBy) -> &'static str {
    match group_by {
        GroupBy::Repo | GroupBy::Branch => "repo",
        GroupBy::Skill => "skill",
        GroupBy::Agent => "subagent",
        GroupBy::Model => "model",
    }
}

fn sum_cost(rows: &[&DataRow]) -> f64 {
    rows.iter()
        .filter_map(|row| row.cost_usd)
        .fold(0.0, |total, cost| total + cost)
}

fn within(group_by: GroupBy) -> Level {
    match group_by {
        GroupBy::Repo | GroupBy::Branch | GroupBy::Agent | GroupBy::Model => Level::Skill,
        GroupBy::Skill => Level::Repo,
    }
}

fn top_heading(group_by: GroupBy) -> &'static str {
    match within(group_by) {
        Level::Skill => "Top skill",
        _ => "Top repo",
    }
}

fn top_of(row: &DataRow, top: &[TopItem]) -> String {
    match (&row.key, row.not_in_transcript) {
        (Some(key), false) => top_for(&row.account, key, row.cost_usd.unwrap_or(0.0), top),
        _ => String::new(),
    }
}

fn top_for(account: &Option<String>, key: &str, cost: f64, top: &[TopItem]) -> String {
    if cost <= 0.0 {
        return String::new();
    }
    top.iter()
        .find(|item| &item.account == account && item.key.as_deref() == Some(key))
        .map_or(String::new(), |item| {
            format!("{:>4} {}", percent_of(item.cost_usd, cost), item.label)
        })
}

fn percent_of(part: f64, whole: f64) -> String {
    if whole <= 0.0 {
        return String::new();
    }
    format!("{:.0}%", (part / whole * 100.0).clamp(0.0, 100.0))
}

fn share_bar(cost: f64, total: f64) -> Line<'static> {
    let bar = BAR_WIDTH - 1;
    let filled = if total > 0.0 {
        ((cost / total).clamp(0.0, 1.0) * bar as f64).round() as usize
    } else {
        0
    };
    Line::from(vec![
        Span::styled("━".repeat(filled), KEY),
        Span::styled("─".repeat(bar - filled), MUTED),
    ])
}

fn level_heading(level: Level) -> &'static str {
    match level {
        Level::Repo => "Repo",
        Level::Branch => "Branch",
        Level::RepoBranch => "Repo",
        Level::Skill => "Skill",
        Level::Agent => "Subagent",
        Level::Model => "Model",
        Level::Session => "Session",
    }
}

pub fn load(app: &mut App, store: &Store) {
    if let Some(wanted) = app.take_history_query() {
        let loaded = spending(store, &wanted).map_err(|error| error.to_string());
        app.set_history(&wanted, loaded);
    }
    if let Some(account) = app.take_advice_query() {
        let loaded = match account.parse::<Account>() {
            Ok(account) => store
                .local_today()
                .and_then(|today| store.usage_advice(&account, &today))
                .map_err(|error| error.to_string()),
            Err(_) => Ok(None),
        };
        app.set_advice(loaded);
    }
}

fn spending(store: &Store, wanted: &HistoryQuery) -> Result<Spending, data::Error> {
    let today = store.local_today()?;
    let response = data::query(store, wanted.range, wanted.group_by, &[], &today)?;
    let mut top = store.top_within(
        response.level,
        within(wanted.group_by),
        response.since.as_deref(),
    )?;
    if wanted.group_by == GroupBy::Branch {
        top.extend(store.top_within(Level::Repo, Level::Skill, response.since.as_deref())?);
    }
    Ok(Spending { response, top })
}

pub(crate) type LoadedAdvice = Result<Option<UsageAdvice>, String>;
