//! v5 filesystem layout.
//!
//! v5 root is fixed at `~/.SwitchHosts`. Everything v5 reads or writes
//! lives under it. Phase 1B step 1 only creates `internal/` — the main
//! storage layout (manifest.json, entries/, trashcan.json) lands in
//! subsequent steps.

use std::path::{Path, PathBuf};

use super::error::StorageError;

/// Absolute paths for the v5 data layout. Safe to clone — this is just
/// a bundle of `PathBuf`s, no filesystem handles.
#[derive(Debug, Clone)]
pub struct V5Paths {
    pub root: PathBuf,
    pub manifest_file: PathBuf,
    pub trashcan_file: PathBuf,
    pub entries_dir: PathBuf,
    pub internal: PathBuf,
    pub config_file: PathBuf,
    /// Placeholder for a later sub-step (`internal/state.json`).
    #[allow(dead_code)]
    pub state_file: PathBuf,
    pub histories_dir: PathBuf,
}

impl V5Paths {
    /// Resolve the default v5 layout: `~/.SwitchHosts`.
    pub fn resolve_default() -> Result<Self, StorageError> {
        let home = dirs::home_dir().ok_or(StorageError::HomeDirNotFound)?;
        Ok(Self::under(home.join(".SwitchHosts")))
    }

    /// Build a `V5Paths` rooted at `root`. Used by tests to point the layout
    /// at a temporary directory.
    pub fn under(root: PathBuf) -> Self {
        let manifest_file = root.join("manifest.json");
        let trashcan_file = root.join("trashcan.json");
        let entries_dir = root.join("entries");
        let internal = root.join("internal");
        let config_file = internal.join("config.json");
        let state_file = internal.join("state.json");
        let histories_dir = internal.join("histories");
        Self {
            root,
            manifest_file,
            trashcan_file,
            entries_dir,
            internal,
            config_file,
            state_file,
            histories_dir,
        }
    }

    /// Ensure every directory v5 owns exists. Does not touch files.
    pub fn ensure_dirs(&self) -> Result<(), StorageError> {
        create_dir_all(&self.root)?;
        create_dir_all(&self.entries_dir)?;
        create_dir_all(&self.internal)?;
        create_dir_all(&self.histories_dir)?;
        Ok(())
    }
}

fn create_dir_all(path: &Path) -> Result<(), StorageError> {
    std::fs::create_dir_all(path).map_err(|e| StorageError::io(path.display().to_string(), e))
}
