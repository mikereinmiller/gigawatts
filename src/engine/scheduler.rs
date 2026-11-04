use std::time::Duration;

pub const READ_DELAY: Duration = Duration::from_secs(1);
pub const SAFETY_SCAN_INTERVAL: Duration = Duration::from_secs(3 * 60);
pub const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    ReadChangedFiles,
    SafetyScan,
    RefreshLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    pub at: Duration,
    pub work: Work,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    last_scan: Duration,
    last_event: Option<Duration>,
    first_unread_event: Option<Duration>,
    limits_due: Option<Duration>,
}

impl Schedule {
    pub fn started_at(now: Duration) -> Schedule {
        Schedule {
            last_scan: now,
            last_event: None,
            first_unread_event: None,
            limits_due: None,
        }
    }

    pub fn limits_due(&mut self, at: Option<Duration>) {
        self.limits_due = at;
    }

    pub fn file_event(&mut self, now: Duration) {
        if self.idle_at().is_none_or(|idle_at| now >= idle_at) {
            self.last_scan = now;
        }
        self.last_event = Some(now);
        self.first_unread_event.get_or_insert(now);
    }

    pub fn read_changed_files(&mut self) {
        self.first_unread_event = None;
    }

    pub fn scanned(&mut self, now: Duration) {
        self.last_scan = now;
        self.first_unread_event = None;
    }

    fn idle_at(&self) -> Option<Duration> {
        self.last_event.map(|last_event| last_event + IDLE_AFTER)
    }
}

pub fn next_deadline(schedule: &Schedule, now: Duration) -> Option<Deadline> {
    let read = schedule
        .first_unread_event
        .map(|first_unread_event| Deadline {
            at: first_unread_event + READ_DELAY,
            work: Work::ReadChangedFiles,
        });
    let scan = schedule.idle_at().and_then(|idle_at| {
        let scan_at = schedule.last_scan + SAFETY_SCAN_INTERVAL;
        (now < idle_at && scan_at < idle_at).then_some(Deadline {
            at: scan_at,
            work: Work::SafetyScan,
        })
    });
    let limits = schedule.limits_due.and_then(|due| {
        let idle_at = schedule.idle_at()?;
        (now < idle_at && due < idle_at).then_some(Deadline {
            at: due,
            work: Work::RefreshLimits,
        })
    });
    [read, scan, limits]
        .into_iter()
        .flatten()
        .min_by_key(|deadline| deadline.at)
}
