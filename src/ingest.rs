use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read as _, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::accounts::Attribution;
use crate::config::epoch_ms_from_text;
use crate::pricing::PriceTable;
use crate::sources::git::Resolver;
use crate::sources::transcript::{Line, Request, parse_line, transcript_files};
use crate::store::{self, FilePosition, Store};

pub struct Read<'a> {
    pub lines: Vec<&'a [u8]>,
    pub position: usize,
}

pub fn complete_lines(bytes: &[u8], position: usize) -> Read<'_> {
    let unread = bytes.get(position..).unwrap_or_default();
    let Some(last_newline) = unread.iter().rposition(|&byte| byte == b'\n') else {
        return Read {
            lines: Vec::new(),
            position,
        };
    };
    let lines = unread[..last_newline]
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    Read {
        lines,
        position: position + last_newline + 1,
    }
}

#[derive(Debug)]
pub enum Error {
    Io(PathBuf, io::Error),
    Store(store::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(path, error) => write!(f, "{}: {error}", path.display()),
            Error::Store(error) => write!(f, "history database: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        Error::Store(error)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub files: usize,
    pub files_with_new_lines: usize,
    pub unrecognized: BTreeMap<String, usize>,
}

pub fn ingest_tree(
    store: &mut Store,
    root: &Path,
    table: &PriceTable,
    attribution: &Attribution,
    resolver: &Resolver,
) -> Result<Summary, Error> {
    let files = if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        transcript_files(root).map_err(|error| Error::Io(root.to_path_buf(), error))?
    };
    store.resolve_unplaced_folders(resolver)?;
    let files: Vec<PathBuf> = files
        .into_iter()
        .filter(|path| !is_set_aside(path))
        .collect();
    if root.is_dir() {
        store.forget_unrecognized_outside(root, &files)?;
    }
    let mut summary = Summary::default();
    for path in &files {
        summary.files += 1;
        ingest_file(store, path, table, attribution, resolver, &mut summary)?;
    }
    Ok(summary)
}

fn is_set_aside(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name.contains(".superseded-") || name.contains(".orphaned-")
}

fn ingest_file(
    store: &mut Store,
    path: &Path,
    table: &PriceTable,
    attribution: &Attribution,
    resolver: &Resolver,
    summary: &mut Summary,
) -> Result<(), Error> {
    let io_error = |error| Error::Io(path.to_path_buf(), error);
    let mut file = File::open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    let key = path.to_string_lossy();
    let first_line_hash = first_line_hash(&file).map_err(io_error)?;
    let start = match store.position(&key)? {
        Some(saved)
            if saved.device == metadata.dev()
                && saved.inode == metadata.ino()
                && saved.first_line_hash == first_line_hash
                && saved.offset <= metadata.len() =>
        {
            saved.offset
        }
        _ => 0,
    };
    if start == metadata.len() {
        return Ok(());
    }
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(start)).map_err(io_error)?;
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let read = complete_lines(&bytes, 0);
    if read.lines.is_empty() {
        return Ok(());
    }
    let mut lines = Vec::new();
    for line in &read.lines {
        match parse_line(&String::from_utf8_lossy(line)) {
            Some(Line::Unrecognized { kind, timestamp }) => {
                *summary.unrecognized.entry(kind.clone()).or_default() += 1;
                lines.push(Line::Unrecognized { kind, timestamp });
            }
            Some(Line::Request(request)) if !has_usable_timestamp(&request) => {
                let kind = "assistant without a timestamp".to_string();
                *summary.unrecognized.entry(kind.clone()).or_default() += 1;
                lines.push(Line::Unrecognized {
                    kind,
                    timestamp: None,
                });
            }
            Some(parsed) => lines.push(parsed),
            None => {}
        }
    }
    let position = FilePosition {
        path: key.into_owned(),
        device: metadata.dev(),
        inode: metadata.ino(),
        first_line_hash,
        offset: start + read.position as u64,
    };
    store.save_file(&position, start == 0, &lines, attribution, table, resolver)?;
    summary.files_with_new_lines += 1;
    Ok(())
}

fn has_usable_timestamp(request: &Request) -> bool {
    request
        .detail
        .timestamp
        .as_deref()
        .and_then(epoch_ms_from_text)
        .is_some()
}

fn first_line_hash(file: &File) -> io::Result<Option<u64>> {
    let mut first_line = Vec::new();
    BufReader::new(file).read_until(b'\n', &mut first_line)?;
    if first_line.last() != Some(&b'\n') {
        return Ok(None);
    }
    let fnv_1a = first_line
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    Ok(Some(fnv_1a))
}
