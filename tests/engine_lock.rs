use gigawatts::engine::lock::EngineLock;

#[test]
fn a_second_engine_cant_take_the_lock_until_the_first_lets_go() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.lock");

    let first = EngineLock::acquire(&path).unwrap();
    assert!(first.is_some());
    assert!(EngineLock::acquire(&path).unwrap().is_none());
    assert!(EngineLock::is_held(&path).unwrap());

    drop(first);

    assert!(!EngineLock::is_held(&path).unwrap());
    assert!(EngineLock::acquire(&path).unwrap().is_some());
}

#[test]
fn a_starting_engine_waits_out_a_check_that_holds_the_lock_for_a_moment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.lock");
    let checker = std::fs::File::create(&path).unwrap();
    checker.lock_shared().unwrap();
    let released = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(checker);
    });

    let engine = EngineLock::acquire(&path).unwrap();

    released.join().unwrap();
    assert!(engine.is_some());
}

#[test]
fn a_check_while_another_check_holds_the_lock_sees_no_engine() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.lock");
    let other_check = std::fs::File::create(&path).unwrap();
    other_check.lock_shared().unwrap();

    assert!(!EngineLock::is_held(&path).unwrap());
}
