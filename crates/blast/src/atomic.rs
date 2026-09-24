//! Write-then-rename, because every reader here is another agent.
//!
//! Peer records are read by processes we do not control, at moments we do
//! not choose. A half-written file is a parse error in somebody else's
//! session, so nothing is ever written in place: a temp file in the same
//! directory takes the bytes, and the rename publishes them in one step the
//! filesystem guarantees.

use std::fs;
use std::io::Write;
use std::path::Path;

pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp = dir.join(format!(".{}.{}.tmp", file_name(path), stamp));
    {
        let mut handle = fs::File::create(&temp)?;
        handle.write_all(bytes)?;
        handle.sync_all()?;
    }
    fs::rename(&temp, path)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "blast".into())
}
