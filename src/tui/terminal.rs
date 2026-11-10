use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher as _};
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};

use super::{App, Flow};

const MINUTE_MS: i64 = 60_000;

pub enum Wake {
    Key(KeyEvent),
    Resize,
    SnapshotWritten,
}

pub fn run(
    app: &mut App,
    snapshot_path: &Path,
    mut rebuild: impl FnMut(&mut App),
    mut load_history: impl FnMut(&mut App),
    engine_running: impl Fn() -> bool,
    mut remember: impl FnMut(&BTreeSet<String>) -> io::Result<()>,
    local_minute: &dyn Fn(i64) -> String,
) -> io::Result<()> {
    let (wake, woken) = mpsc::channel();
    forward_terminal_events(wake.clone());
    let watcher = watch_snapshot(snapshot_path, wake);
    app.set_watching_snapshot(watcher.is_some());
    let mut terminal = ratatui::try_init().inspect_err(|_| ratatui::restore())?;
    let mut queued = None;
    let shown = loop {
        load_history(app);
        if let Err(error) = terminal.draw(|frame| app.draw(frame, now_ms(), local_minute)) {
            break Err(error);
        }
        if let Some(announced) = app.take_unsaved_announced()
            && remember(&announced).is_err()
        {
            app.failed_to_save_announced();
        }
        let next = match queued.take() {
            Some(wake) => Ok(wake),
            None => woken.recv_timeout(until_next_minute(now_ms())),
        };
        match next {
            Ok(Wake::Key(key)) => {
                if app.handle(key) == Flow::Quit {
                    break Ok(());
                }
            }
            Ok(Wake::Resize) => {}
            Ok(Wake::SnapshotWritten) | Err(RecvTimeoutError::Timeout) => {
                queued = skip_repeated_writes(&woken);
                rebuild(app);
                app.set_engine_running(engine_running());
            }
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
        }
    };
    ratatui::restore();
    shown
}

fn forward_terminal_events(wake: Sender<Wake>) {
    std::thread::spawn(move || {
        loop {
            let woke = match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => Wake::Key(key),
                Ok(Event::Resize(..)) => Wake::Resize,
                Ok(_) => continue,
                Err(_) => return,
            };
            if wake.send(woke).is_err() {
                return;
            }
        }
    });
}

fn watch_snapshot(snapshot_path: &Path, wake: Sender<Wake>) -> Option<notify::RecommendedWatcher> {
    let name = snapshot_path.file_name()?.to_owned();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| is_snapshot_write(&event, &name)) {
            let _ = wake.send(Wake::SnapshotWritten);
        }
    })
    .ok()?;
    watcher
        .watch(snapshot_path.parent()?, RecursiveMode::NonRecursive)
        .ok()?;
    Some(watcher)
}

pub fn is_snapshot_write(event: &notify::Event, snapshot_name: &OsStr) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && event
            .paths
            .iter()
            .any(|path| path.file_name() == Some(snapshot_name))
}

pub fn skip_repeated_writes(woken: &Receiver<Wake>) -> Option<Wake> {
    loop {
        match woken.try_recv() {
            Ok(Wake::SnapshotWritten) => continue,
            Ok(other) => return Some(other),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
        }
    }
}

pub fn until_next_minute(now_ms: i64) -> Duration {
    let wait_ms = MINUTE_MS - now_ms.rem_euclid(MINUTE_MS);
    Duration::from_millis(wait_ms as u64)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}
