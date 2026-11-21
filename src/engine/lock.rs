use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

const CHECK_WAIT: Duration = Duration::from_millis(100);

pub struct EngineLock {
    _file: File,
}

impl EngineLock {
    pub fn acquire(path: &Path) -> io::Result<Option<EngineLock>> {
        Self::acquire_within(path, CHECK_WAIT)
    }

    pub fn acquire_within(path: &Path, wait: Duration) -> io::Result<Option<EngineLock>> {
        let mut file = open(path)?;
        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    file.set_len(0)?;
                    writeln!(file, "{}", std::process::id())?;
                    return Ok(Some(EngineLock { _file: file }));
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(error)) => return Err(error),
            }
        }
    }

    pub fn is_held(path: &Path) -> io::Result<bool> {
        if !path.exists() {
            return Ok(false);
        }
        match open(path)?.try_lock_shared() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}
