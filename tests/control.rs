use std::sync::mpsc;

use gigawatts::engine::control::{self, Command, Handshake, Request, handshake};

#[test]
fn the_engine_answers_version_itself_and_hands_refresh_to_the_main_loop() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let (sender, receiver) = mpsc::channel::<Request>();
    control::listen(&socket, "1.2.3".to_string(), move |request| {
        sender.send(request).unwrap()
    })
    .unwrap();

    assert_eq!(control::send(&socket, Command::Version).unwrap(), "1.2.3");

    let main_loop = std::thread::spawn(move || {
        let request = receiver.recv().unwrap();
        assert_eq!(request.command, Command::Refresh);
        request.reply.send("refreshed".to_string()).unwrap();
    });
    assert_eq!(
        control::send(&socket, Command::Refresh).unwrap(),
        "refreshed"
    );
    main_loop.join().unwrap();
}

#[test]
fn sending_to_a_missing_engine_fails() {
    let dir = tempfile::tempdir().unwrap();

    assert!(control::send(&dir.path().join("engine.sock"), Command::Version).is_err());
}

#[test]
fn an_engine_behind_on_release_reader_or_migrations_is_replaced() {
    assert_eq!(handshake("0.2.0+r1.m6", "0.1.9+r5.m9"), Handshake::Replace);
    assert_eq!(handshake("0.10.0+r1.m6", "0.9.0+r1.m6"), Handshake::Replace);
    assert_eq!(handshake("0.1.0+r1.m7", "0.1.0+r1.m6"), Handshake::Replace);
    assert_eq!(handshake("0.1.0+r2.m6", "0.1.0+r1.m6"), Handshake::Replace);
    assert_eq!(handshake("0.1.0+r2.m6", "0.1.0+r1.m7"), Handshake::Replace);
    assert_eq!(handshake("0.1.0+r1.m6", "0.1.0"), Handshake::Replace);
    assert_eq!(handshake("0.1.0+r1.m6", "garbled"), Handshake::Replace);
}

#[test]
fn the_same_identity_is_kept_and_a_newer_engine_is_left_alone() {
    assert_eq!(handshake("0.1.0+r1.m6", "0.1.0+r1.m6"), Handshake::Keep);
    assert_eq!(handshake("0.2.0-beta", "0.2.0-beta"), Handshake::Keep);
    assert_eq!(
        handshake("0.1.0+r1.m6", "0.1.0+r1.m7"),
        Handshake::EngineNewer
    );
    assert_eq!(
        handshake("0.1.0+r1.m6", "0.1.0+r2.m6"),
        Handshake::EngineNewer
    );
    assert_eq!(
        handshake("0.1.0+r1.m6", "0.2.0+r1.m1"),
        Handshake::EngineNewer
    );
}

#[test]
fn a_failed_send_names_the_engine_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("engine.sock");

    let error = control::send(&socket, Command::Version).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(
        error
            .to_string()
            .starts_with(&format!("can't reach the engine at {}: ", socket.display())),
        "{error}"
    );
}
