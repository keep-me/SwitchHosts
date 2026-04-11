//! Apply-time error type.
//!
//! Distinct from [`crate::storage::error::StorageError`] because the
//! renderer expects a stable `{ success, code, message }` shape from
//! `setSystemHosts`, not the storage layer's tagged JSON. We translate
//! at the command boundary in `commands.rs`.

use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum HostsApplyError {
    /// Cannot write the system hosts file and elevation isn't available
    /// on this platform yet (Linux / Windows in P2.E.2). Renderer maps
    /// this to its existing `no_access` branch.
    ///
    /// `#[allow(dead_code)]`: only constructed by the Linux/Windows
    /// `elevate_copy` arms, so on a macOS-only `cargo check` it looks
    /// dead. P2.E.4 will exercise it on every platform.
    #[allow(dead_code)]
    #[error("no access: {message}")]
    NoAccess { message: String },

    /// User dismissed the OS authentication prompt.
    #[error("cancelled")]
    Cancelled,

    /// Filesystem / process error from a step that should normally
    /// succeed: temp file write, copy, chmod, exit code from
    /// osascript/pkexec/UAC helper.
    #[error("io: {message}")]
    Io { message: String },
}

impl HostsApplyError {
    /// Translate into the renderer's `IWriteResult` JSON shape so the
    /// existing `actions.setSystemHosts` call sites keep working
    /// without any front-end changes:
    ///
    /// ```ts
    /// { success: false, code?: string, message?: string }
    /// ```
    pub fn into_renderer_value(self) -> Value {
        let (code, message) = match self {
            HostsApplyError::NoAccess { message } => ("no_access", message),
            HostsApplyError::Cancelled => ("cancelled", "user cancelled".to_string()),
            HostsApplyError::Io { message } => ("fail", message),
        };
        json!({
            "success": false,
            "code": code,
            "message": message,
        })
    }
}
