use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

const QUEUED: &str = "queued";
const QUICK_ANSWER: Duration = Duration::from_secs(5);
const FULL_SCAN_ANSWER: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Version,
    Refresh,
    Limits,
    LimitsLater,
    Stop,
}

impl Command {
    fn word(self) -> &'static str {
        match self {
            Command::Version => "version",
            Command::Refresh => "refresh",
            Command::Limits => "limits",
            Command::LimitsLater => "limits-later",
            Command::Stop => "stop",
        }
    }

    fn from_word(word: &str) -> Option<Command> {
        [
            Command::Version,
            Command::Refresh,
            Command::Limits,
            Command::LimitsLater,
            Command::Stop,
        ]
        .into_iter()
        .find(|command| command.word() == word)
    }

    fn answer_within(self) -> Duration {
        match self {
            Command::Refresh | Command::Limits => FULL_SCAN_ANSWER,
            Command::Version | Command::LimitsLater | Command::Stop => QUICK_ANSWER,
        }
    }
}

pub struct Request {
    pub command: Command,
    pub reply: mpsc::Sender<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Handshake {
    Keep,
    Replace,
    EngineNewer,
}

pub fn handshake(ours: &str, engine: &str) -> Handshake {
    if ours.trim() == engine.trim() {
        return Handshake::Keep;
    }
    let (Some(ours), Some(engine)) = (Identity::parse(ours), Identity::parse(engine)) else {
        return Handshake::Replace;
    };
    let engine_newer = engine.release > ours.release
        || (engine.release == ours.release
            && engine.reader >= ours.reader
            && engine.migrations >= ours.migrations);
    if engine_newer {
        Handshake::EngineNewer
    } else {
        Handshake::Replace
    }
}

struct Identity {
    release: Vec<u64>,
    reader: u64,
    migrations: u64,
}

impl Identity {
    fn parse(version: &str) -> Option<Identity> {
        let (release, store) = version.trim().split_once("+r")?;
        let (reader, migrations) = store.split_once(".m")?;
        Some(Identity {
            release: release
                .split('.')
                .map(|part| part.parse().ok())
                .collect::<Option<_>>()?,
            reader: reader.parse().ok()?,
            migrations: migrations.parse().ok()?,
        })
    }
}

pub fn listen(
    socket: &Path,
    version: String,
    forward: impl Fn(Request) + Send + Sync + 'static,
) -> io::Result<()> {
    match std::fs::remove_file(socket) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    let socket = socket.to_path_buf();
    let forward = Arc::new(forward);
    std::thread::Builder::new()
        .name("control".to_string())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let forward = Arc::clone(&forward);
                let socket = socket.clone();
                let version = version.clone();
                let _ = std::thread::Builder::new()
                    .name("control client".to_string())
                    .spawn(move || answer(stream, &version, &socket, forward.as_ref()));
            }
        })?;
    Ok(())
}

fn answer(stream: UnixStream, version: &str, socket: &Path, forward: &dyn Fn(Request)) {
    let _ = stream.set_read_timeout(Some(QUICK_ANSWER));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let mut stream = stream;
    let reply = match Command::from_word(line.trim()) {
        Some(Command::Version) => version.to_string(),
        Some(Command::Stop) => {
            let _ = writeln!(stream, "stopping");
            let _ = stream.flush();
            let _ = std::fs::remove_file(socket);
            std::process::exit(0);
        }
        Some(Command::LimitsLater) => {
            let (reply, _) = mpsc::channel();
            forward(Request {
                command: Command::LimitsLater,
                reply,
            });
            QUEUED.to_string()
        }
        Some(command) => {
            let (reply, answer) = mpsc::channel();
            forward(Request { command, reply });
            answer
                .recv()
                .unwrap_or_else(|_| "the engine stopped before answering".to_string())
        }
        None => format!("unknown command {:?}", line.trim()),
    };
    let _ = writeln!(stream, "{reply}");
}

pub fn send(socket: &Path, command: Command) -> io::Result<String> {
    exchange(socket, command).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("can't reach the engine at {}: {error}", socket.display()),
        )
    })
}

fn exchange(socket: &Path, command: Command) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(command.answer_within()))?;
    writeln!(stream, "{}", command.word())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    if reply.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the engine closed the connection without answering",
        ));
    }
    Ok(reply.trim_end().to_string())
}
