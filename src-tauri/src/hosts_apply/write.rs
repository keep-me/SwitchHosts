//! System hosts write orchestration.
//!
//! Mirrors the Electron `setSystemHosts` flow in
//! [src/main/actions/hosts/setSystemHosts.ts]:
//!
//! 1. Normalize line endings to LF in memory.
//! 2. If `write_mode == "append"`, splice the new content under the
//!    `# --- SWITCHHOSTS_CONTENT_START ---` marker, dropping anything
//!    that was previously below it.
//! 3. Convert to platform-native line endings for the on-disk content.
//! 4. Read the current system hosts file. If the new payload is
//!    byte-identical (compared via stable hash), short-circuit with
//!    success — avoids triggering an OS auth prompt for a no-op.
//! 5. Try a direct write. On `PermissionDenied`, fall through to the
//!    elevation helper. The renderer's password dialog flow is
//!    deliberately *not* invoked: we let the OS prompt the user.
//! 6. On success, return both the previous and the new content so the
//!    calling command can append two history entries (matches Electron).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use super::elevation::write_with_elevation;
use super::error::HostsApplyError;

const CONTENT_START_MARKER: &str = "# --- SWITCHHOSTS_CONTENT_START ---";

pub struct ApplyOutcome {
    pub previous_content: String,
    pub new_content: String,
    /// True when the file was already up-to-date and no write happened.
    /// Renderer-visible result is still success in that case, but the
    /// caller can skip recording redundant history entries.
    pub unchanged: bool,
}

/// Write `aggregated_content` to the system hosts file using the
/// configured `write_mode`. Returns the previous + new content on
/// success so the caller can persist apply history.
pub fn apply_to_system_hosts(
    aggregated_content: &str,
    write_mode: &str,
) -> Result<ApplyOutcome, HostsApplyError> {
    let target = system_hosts_path();
    let content_lf = normalize_line_endings(aggregated_content);

    let previous_raw = read_system_hosts(target).unwrap_or_default();
    let previous_lf = normalize_line_endings(&previous_raw);

    let final_content_lf = if write_mode == "append" {
        make_append_content(&previous_lf, &content_lf)
    } else {
        content_lf.clone()
    };

    let disk_content = restore_line_endings(&final_content_lf);

    if hash_str(&previous_raw) == hash_str(&disk_content) {
        return Ok(ApplyOutcome {
            previous_content: previous_lf,
            new_content: final_content_lf,
            unchanged: true,
        });
    }

    match std::fs::write(target, disk_content.as_bytes()) {
        Ok(()) => Ok(ApplyOutcome {
            previous_content: previous_lf,
            new_content: final_content_lf,
            unchanged: false,
        }),
        Err(e) if is_permission_denied(&e) => {
            write_with_elevation(Path::new(target), &disk_content)?;
            Ok(ApplyOutcome {
                previous_content: previous_lf,
                new_content: final_content_lf,
                unchanged: false,
            })
        }
        Err(e) => Err(HostsApplyError::Io {
            message: format!("write {target}: {e}"),
        }),
    }
}

fn read_system_hosts(target: &str) -> Result<String, HostsApplyError> {
    match std::fs::read_to_string(target) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(HostsApplyError::Io {
            message: format!("read {target}: {e}"),
        }),
    }
}

fn is_permission_denied(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied
}

pub fn system_hosts_path() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        r"C:\Windows\System32\drivers\etc\hosts"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "/etc/hosts"
    }
}

// ---- line ending normalisation ---------------------------------------------

fn normalize_line_endings(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(target_os = "windows")]
fn restore_line_endings(s: &str) -> String {
    s.replace('\n', "\r\n")
}

#[cfg(not(target_os = "windows"))]
fn restore_line_endings(s: &str) -> String {
    s.to_string()
}

// ---- append-mode helper ----------------------------------------------------

fn make_append_content(previous_lf: &str, new_content_lf: &str) -> String {
    let head = match previous_lf.find(CONTENT_START_MARKER) {
        Some(idx) => previous_lf[..idx].trim_end().to_string(),
        None => previous_lf.to_string(),
    };

    if new_content_lf.is_empty() {
        return format!("{head}\n");
    }

    format!("{head}\n\n{CONTENT_START_MARKER}\n\n{new_content_lf}")
}

// ---- comparison hash --------------------------------------------------------

/// Stable in-process content hash. We don't need cryptographic
/// strength — only "are these two byte sequences the same" — so a
/// `DefaultHasher` is plenty and avoids pulling md5/sha into Cargo.toml.
fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}
