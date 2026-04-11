//! v5 filesystem-backed storage layer.
//!
//! This module owns every read/write against `~/.SwitchHosts`. Tauri
//! commands in `commands.rs` access it through the `AppState` held by
//! the Tauri builder, never by reaching into the filesystem themselves.

pub mod config;
pub mod error;
pub mod paths;

pub use config::AppConfig;
pub use error::StorageError;
pub use paths::V5Paths;

use std::sync::Mutex;

/// Process-wide shared state held by Tauri as `State<'_, AppState>`.
pub struct AppState {
    pub paths: V5Paths,
    pub config: Mutex<AppConfig>,
}

impl AppState {
    /// Initialise the shared state at app startup:
    ///
    /// 1. Resolve the default v5 paths (`~/.SwitchHosts`).
    /// 2. Ensure the `internal/` directory exists.
    /// 3. Load `internal/config.json` into memory, or fall back to
    ///    defaults if the file is missing / corrupt.
    pub fn bootstrap() -> Result<Self, StorageError> {
        let paths = V5Paths::resolve_default()?;
        paths.ensure_dirs()?;
        let config = AppConfig::load(&paths.config_file);
        Ok(Self {
            paths,
            config: Mutex::new(config),
        })
    }

    /// Persist the in-memory config to disk. Called after every
    /// successful `config_set` / `config_update`.
    pub fn persist_config(&self) -> Result<(), StorageError> {
        let guard = self.config.lock().expect("config mutex poisoned");
        guard.save(&self.paths.config_file)
    }
}
