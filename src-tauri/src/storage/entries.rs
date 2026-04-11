//! `~/.SwitchHosts/entries/<id>.hosts` reader and writer.
//!
//! Every local/remote node owns one file under `entries/`. Filenames
//! are `<node-id>.hosts`; moves and renames of the node never rename
//! the file. Content is UTF-8 with LF line endings; the apply pipeline
//! converts to the platform-native newlines right before writing the
//! system hosts file.

use std::path::{Path, PathBuf};

use super::atomic::atomic_write;
use super::error::StorageError;

/// Resolve the path for a node's content file. `id` is expected to be
/// a UUID or similar opaque identifier — we validate it is a "simple"
/// name to defend against path traversal before concatenating.
pub fn entry_path(entries_dir: &Path, id: &str) -> Result<PathBuf, StorageError> {
    validate_id(id)?;
    Ok(entries_dir.join(format!("{id}.hosts")))
}

fn validate_id(id: &str) -> Result<(), StorageError> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
        || id.contains("..")
    {
        return Err(StorageError::InvalidConfigValue {
            key: "entry_id".into(),
            reason: format!("illegal entry id: {id:?}"),
        });
    }
    Ok(())
}

/// Read a node's content. Missing file → empty string (matches Electron
/// `swhdb.collection.hosts.find(...)?.content ?? ""`).
pub fn read_entry(entries_dir: &Path, id: &str) -> Result<String, StorageError> {
    let path = entry_path(entries_dir, id)?;
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| {
        StorageError::io(path.display().to_string(), e)
    })
}

pub fn write_entry(entries_dir: &Path, id: &str, content: &str) -> Result<(), StorageError> {
    let path = entry_path(entries_dir, id)?;
    atomic_write(&path, content.as_bytes())
}

/// Delete a node's content file. No-op if the file is already gone.
/// Used by trashcan "delete permanently" later in Phase 1B / Phase 2.
#[allow(dead_code)]
pub fn delete_entry(entries_dir: &Path, id: &str) -> Result<(), StorageError> {
    let path = entry_path(entries_dir, id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StorageError::io(path.display().to_string(), e)),
    }
}
