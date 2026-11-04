use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Paths(Vec<PathBuf>),
    FullScan,
    Removed(Vec<PathBuf>),
    Ignored,
}

pub fn classify(event: &Event) -> Change {
    if event.need_rescan() {
        return Change::FullScan;
    }
    let wanted: Vec<PathBuf> = match event.kind {
        EventKind::Create(CreateKind::Folder)
            if event
                .paths
                .iter()
                .any(|path| is_projects(path) || is_claude(path)) =>
        {
            return Change::FullScan;
        }
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
            if event
                .paths
                .iter()
                .any(|path| is_projects(path) || is_claude(path)) =>
        {
            return Change::Removed(event.paths.clone());
        }
        EventKind::Create(CreateKind::Folder) => event.paths.clone(),
        EventKind::Create(_)
        | EventKind::Modify(
            ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any | ModifyKind::Other,
        )
        | EventKind::Any
        | EventKind::Other => event
            .paths
            .iter()
            .filter(|path| is_transcript(path))
            .cloned()
            .collect(),
        EventKind::Access(_)
        | EventKind::Modify(ModifyKind::Metadata(_))
        | EventKind::Remove(_) => Vec::new(),
    };
    if wanted.is_empty() {
        Change::Ignored
    } else {
        Change::Paths(wanted)
    }
}

fn is_projects(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "projects")
}

fn is_claude(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".claude")
}

fn is_transcript(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "jsonl")
}

pub struct Watcher {
    inner: RecommendedWatcher,
    watched: BTreeSet<PathBuf>,
    waiting: BTreeMap<PathBuf, PathBuf>,
}

impl Watcher {
    pub fn new(
        handler: impl Fn(notify::Result<Event>) + Send + 'static,
    ) -> notify::Result<Watcher> {
        Ok(Watcher {
            inner: notify::recommended_watcher(handler)?,
            watched: BTreeSet::new(),
            waiting: BTreeMap::new(),
        })
    }

    pub fn watch(&mut self, projects: &Path) -> notify::Result<()> {
        if projects.is_dir() && !self.watched.contains(projects) {
            self.inner.watch(projects, RecursiveMode::Recursive)?;
            self.watched.insert(projects.to_path_buf());
        }
        let waiting_in = if projects.is_dir() {
            None
        } else {
            projects
                .ancestors()
                .skip(1)
                .take(2)
                .find(|dir| dir.is_dir())
        };
        let before = self.waiting.get(projects).cloned();
        if before.as_deref() == waiting_in {
            return Ok(());
        }
        match waiting_in {
            Some(dir) => {
                if !self.is_waiting_in(dir) {
                    self.inner.watch(dir, RecursiveMode::NonRecursive)?;
                }
                self.waiting
                    .insert(projects.to_path_buf(), dir.to_path_buf());
            }
            None => {
                self.waiting.remove(projects);
            }
        }
        match before {
            Some(before) if !self.is_waiting_in(&before) => self.inner.unwatch(&before),
            _ => Ok(()),
        }
    }

    pub fn forget(&mut self, removed: &[PathBuf]) {
        for path in removed {
            let was_watched = self.watched.remove(path);
            let was_waiting_in = self.is_waiting_in(path);
            self.waiting.retain(|_, waiting_in| waiting_in != path);
            if was_watched || was_waiting_in {
                // inotify drops the watch on a deleted folder itself, so unwatch can fail.
                let _ = self.inner.unwatch(path);
            }
        }
    }

    fn is_waiting_in(&self, dir: &Path) -> bool {
        self.waiting.values().any(|waiting_in| waiting_in == dir)
    }
}
