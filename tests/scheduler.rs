use std::time::Duration;

use gigawatts::engine::scheduler::{Deadline, Schedule, Work, next_deadline};

fn minutes(count: u64) -> Duration {
    Duration::from_secs(count * 60)
}

fn seconds(count: u64) -> Duration {
    Duration::from_secs(count)
}

#[test]
fn nothing_is_scheduled_before_any_file_event() {
    let schedule = Schedule::started_at(seconds(0));

    assert_eq!(next_deadline(&schedule, minutes(30)), None);
}

#[test]
fn safety_net_scans_stop_ten_minutes_after_the_last_event() {
    let mut schedule = Schedule::started_at(seconds(0));
    schedule.file_event(seconds(0));
    schedule.read_changed_files();
    for scan_at in [minutes(3), minutes(6), minutes(9)] {
        assert_eq!(
            next_deadline(&schedule, scan_at - seconds(1)),
            Some(Deadline {
                at: scan_at,
                work: Work::SafetyScan
            })
        );
        schedule.scanned(scan_at);
    }

    assert_eq!(next_deadline(&schedule, minutes(9)), None);
    assert_eq!(next_deadline(&schedule, minutes(10)), None);
    assert_eq!(next_deadline(&schedule, minutes(60)), None);
}

#[test]
fn a_new_event_restarts_the_safety_net() {
    let mut schedule = Schedule::started_at(seconds(0));
    schedule.file_event(seconds(0));
    schedule.read_changed_files();
    schedule.scanned(minutes(9));
    assert_eq!(next_deadline(&schedule, minutes(11)), None);

    schedule.file_event(minutes(20));
    schedule.read_changed_files();

    assert_eq!(
        next_deadline(&schedule, minutes(20)),
        Some(Deadline {
            at: minutes(23),
            work: Work::SafetyScan
        })
    );
}

#[test]
fn changed_files_are_read_one_second_after_the_first_unread_event() {
    let mut schedule = Schedule::started_at(seconds(0));
    schedule.file_event(seconds(10));
    schedule.file_event(seconds(10) + Duration::from_millis(600));

    assert_eq!(
        next_deadline(&schedule, seconds(10) + Duration::from_millis(700)),
        Some(Deadline {
            at: seconds(11),
            work: Work::ReadChangedFiles
        })
    );
}

#[test]
fn the_earlier_of_a_read_and_a_scan_comes_first() {
    let mut schedule = Schedule::started_at(seconds(0));
    schedule.file_event(seconds(0));
    schedule.read_changed_files();
    schedule.file_event(minutes(2) + seconds(59) + Duration::from_millis(500));

    assert_eq!(
        next_deadline(
            &schedule,
            minutes(2) + seconds(59) + Duration::from_millis(600)
        ),
        Some(Deadline {
            at: minutes(3),
            work: Work::SafetyScan
        })
    );

    schedule.scanned(minutes(3));
    schedule.file_event(minutes(4));

    assert_eq!(
        next_deadline(&schedule, minutes(4)),
        Some(Deadline {
            at: minutes(4) + seconds(1),
            work: Work::ReadChangedFiles
        })
    );
}

#[test]
fn a_limit_refresh_runs_while_active_and_never_once_idle() {
    let mut schedule = Schedule::started_at(seconds(0));
    schedule.file_event(seconds(0));
    schedule.read_changed_files();
    schedule.scanned(minutes(2));

    schedule.limits_due(Some(minutes(2) + seconds(30)));
    assert_eq!(
        next_deadline(&schedule, minutes(2)),
        Some(Deadline {
            at: minutes(2) + seconds(30),
            work: Work::RefreshLimits
        })
    );

    schedule.limits_due(Some(minutes(12)));
    schedule.scanned(minutes(9));
    assert_eq!(next_deadline(&schedule, minutes(9)), None);
    assert_eq!(next_deadline(&schedule, minutes(60)), None);
}
