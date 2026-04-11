//! Privileged write to the system hosts file.
//!
//! Strategy:
//!
//! 1. Write the new content to a temp file (no privilege required).
//! 2. Run the platform-specific elevation helper to copy the temp
//!    file over `/etc/hosts` (or the Windows equivalent).
//! 3. Restore the original mode where applicable.
//!
//! The OS-native elevation prompt collects credentials, so the v5
//! Tauri build never asks the user to type a password into our own UI.
//! The renderer's `show_sudo_password_input` listener becomes dead
//! code on the Tauri path; it stays for the Electron build.
//!
//! P2.E.2 only ships the macOS path (osascript). Linux pkexec /
//! Windows UAC self-relaunch land in P2.E.4.

use std::path::{Path, PathBuf};

use super::error::HostsApplyError;

/// Write `content` to `target` using OS-native elevation. The caller
/// is responsible for falling back here only after a direct write
/// has failed with a permission error.
pub fn write_with_elevation(target: &Path, content: &str) -> Result<(), HostsApplyError> {
    let tmp_path = stage_temp_file(content)?;
    let result = elevate_copy(&tmp_path, target);
    // Best-effort cleanup; ignore failures because the temp directory
    // is OS-managed and the file is small.
    let _ = std::fs::remove_file(&tmp_path);
    result
}

fn stage_temp_file(content: &str) -> Result<PathBuf, HostsApplyError> {
    let mut path = std::env::temp_dir();
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    path.push(format!("swh_apply_{stamp}.hosts"));
    std::fs::write(&path, content).map_err(|e| HostsApplyError::Io {
        message: format!("staging temp file failed: {e}"),
    })?;
    Ok(path)
}

// ---- macOS: osascript --------------------------------------------------------

#[cfg(target_os = "macos")]
fn elevate_copy(src: &Path, dst: &Path) -> Result<(), HostsApplyError> {
    use std::process::Command;

    // We pass both paths through `quoted form of` so spaces and other
    // shell metacharacters in the temp dir don't break the inner shell
    // script. The outer AppleScript still needs its own backslash
    // escaping for embedded double-quotes — the temp filename only
    // contains hex digits and underscores, so the risk is theoretical
    // but the escape pass keeps the contract honest.
    let src_lit = applescript_string_literal(&src.display().to_string());
    let dst_lit = applescript_string_literal(&dst.display().to_string());

    let script = format!(
        "do shell script \"/bin/cp \" & quoted form of {src_lit} & \" \" & quoted form of {dst_lit} & \" && /bin/chmod 644 \" & quoted form of {dst_lit} with administrator privileges"
    );

    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| HostsApplyError::Io {
            message: format!("failed to launch osascript: {e}"),
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if is_user_cancelled(&stderr) {
        return Err(HostsApplyError::Cancelled);
    }
    Err(HostsApplyError::Io {
        message: format!("osascript exit {}: {}", output.status, stderr.trim()),
    })
}

#[cfg(target_os = "macos")]
fn applescript_string_literal(s: &str) -> String {
    // AppleScript string literal: wrap in double quotes, escape `"` and
    // `\`. The shell quoting is handled separately by `quoted form of`.
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "macos")]
fn is_user_cancelled(stderr: &str) -> bool {
    // osascript reports user cancellation as `(-128)` regardless of
    // locale. The textual `User canceled.` follows the localized
    // system, so checking the numeric code is the reliable signal.
    stderr.contains("(-128)") || stderr.contains("User canceled") || stderr.contains("User cancelled")
}

// ---- Linux / Windows: P2.E.4 -------------------------------------------------
//
// Both platforms compile but report `NoAccess` from the elevation step
// for now. The renderer's `no_access` branch will trigger the legacy
// password dialog on Linux, which is harmless because the dialog feeds
// back into a path that doesn't exist yet — it just shows an error.
// P2.E.4 replaces both stubs with real implementations.

#[cfg(target_os = "linux")]
fn elevate_copy(_src: &Path, _dst: &Path) -> Result<(), HostsApplyError> {
    Err(HostsApplyError::NoAccess {
        message: "Linux pkexec elevation lands in P2.E.4".to_string(),
    })
}

#[cfg(target_os = "windows")]
fn elevate_copy(_src: &Path, _dst: &Path) -> Result<(), HostsApplyError> {
    Err(HostsApplyError::NoAccess {
        message: "Windows UAC elevation lands in P2.E.4".to_string(),
    })
}
