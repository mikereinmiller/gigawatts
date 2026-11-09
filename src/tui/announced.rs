use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

pub fn read(path: &Path) -> Option<BTreeSet<String>> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        read => Some(
            read.ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default(),
        ),
    }
}

pub fn write(path: &Path, accounts: &BTreeSet<String>) -> io::Result<()> {
    let staged = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)?;
    serde_json::to_writer_pretty(&mut file, accounts)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(&staged, path)
}
