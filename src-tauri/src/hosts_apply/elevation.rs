//! Privileged write to the system hosts file.
//!
//! Strategy:
//!
//! 1. Write the new content to a temp file (no privilege required).
//! 2. Run the platform-specific elevation helper to copy the temp
//!    file over `/etc/hosts` (or the Windows equivalent).
//! 3. The destination's existing mode and ownership are preserved
//!    automatically by the underlying copy primitive (POSIX `cp`
//!    truncates and writes the existing inode; Windows `copy`
//!    writes through the existing handle).
//!
//! The OS-native elevation prompt collects credentials, so the v5
//! Tauri build never asks the user to type a password into our own UI.
//! The renderer's `show_sudo_password_input` listener becomes dead
//! code on the Tauri path; it stays for the Electron build.
//!
//! Per-platform helpers:
//! - macOS: `osascript -e 'do shell script ... with administrator
//!   privileges'` (P2.E.2)
//! - Linux: `pkexec /bin/cp` (P2.E.4)
//! - Windows: `ShellExecuteExW` with `runas` verb on `cmd.exe /c
//!   copy /Y` (P2.E.4)

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

// ---- Linux: pkexec ---------------------------------------------------------

#[cfg(target_os = "linux")]
fn elevate_copy(src: &Path, dst: &Path) -> Result<(), HostsApplyError> {
    use std::process::Command;

    // pkexec runs the given binary as root after the user
    // authenticates via the desktop environment's polkit agent
    // (polkit-gnome-authentication-agent-1, lxpolkit, kde-polkit
    // and friends). All modern Linux desktops ship one out of the
    // box, so the prompt appears as a graphical dialog without us
    // having to do anything special.
    //
    // We invoke `/bin/cp` directly — no shell, no escaping —  so
    // paths with spaces or other shell metacharacters in the temp
    // dir can't break the command line. POSIX `cp` opens an
    // existing destination with `O_WRONLY|O_TRUNC` and writes
    // content into the existing inode, so the destination's mode
    // and ownership (root:root 644 for /etc/hosts on every distro
    // we ship for) are preserved through the copy.
    let output = Command::new("/usr/bin/pkexec")
        .arg("/bin/cp")
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| HostsApplyError::Io {
            message: format!("failed to launch pkexec: {e}"),
        })?;

    if output.status.success() {
        return Ok(());
    }

    // pkexec(1) exit codes:
    //   0   the operation was successful
    //   126 authentication failed (bad password) OR user dismissed
    //       the authentication dialog — both map to Cancelled
    //   127 not authorized — polkit policy refused the action
    //   anything else — exit code from the invoked program (cp)
    let code = output.status.code();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stderr_trim = stderr.trim();
    match code {
        Some(126) => Err(HostsApplyError::Cancelled),
        Some(127) => Err(HostsApplyError::NoAccess {
            message: if stderr_trim.is_empty() {
                "polkit refused the action".to_string()
            } else {
                stderr_trim.to_string()
            },
        }),
        _ => Err(HostsApplyError::Io {
            message: format!(
                "pkexec exit {}: {}",
                code.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                stderr_trim
            ),
        }),
    }
}

// ---- Windows: ShellExecuteExW with runas verb (self-relaunch) -------------
//
// We trigger UAC by relaunching *our own binary* with a magic
// `--swh-elevated-apply-hosts <src> <dst>` argv shape. The early arg
// check at the top of `crate::run` catches that shape in the elevated
// child and performs a plain `std::fs::copy(src, dst)` (which under
// the hood is `CopyFileW`, preserving the destination's NTFS ACL), then
// exits.
//
// Why self-relaunch instead of `cmd /c copy /Y "src" "dst"`:
//
//   - `cmd.exe` parses its command line with `%VAR%` expansion turned
//     on unconditionally. NTFS allows `%` in file names, and Windows
//     temp directories live under `%TEMP%` which can in principle
//     resolve to a path containing literal `%`. Going through cmd
//     would corrupt such a path with a phantom variable expansion.
//   - Spawning our own binary makes Windows parse the args via
//     `CommandLineToArgvW`, which does NOT expand `%VAR%`. The
//     elevated child sees the literal paths as `argv[2]` / `argv[3]`.
//   - Avoids a brief flash of an elevated cmd.exe console window.
//
// Path quoting in `lpParameters`: NTFS forbids `"` in file names, so
// wrapping each path in double quotes is enough for the destination
// and the temp file. The temp file name is hex digits + underscores
// only, and the system hosts path never ends in `\`, so the trailing
// `"` is never preceded by a backslash (which would otherwise be
// escaped to a literal `"` by CommandLineToArgvW).

#[cfg(target_os = "windows")]
fn elevate_copy(src: &Path, dst: &Path) -> Result<(), HostsApplyError> {
    use std::ffi::{OsStr, OsString};
    use std::iter;
    use std::mem;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_CANCELLED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    fn to_wide(os: &OsStr) -> Vec<u16> {
        os.encode_wide().chain(iter::once(0)).collect()
    }

    // Resolve our own binary path. `current_exe` returns the
    // SwitchHosts.exe location both in dev (`target\debug\...`) and
    // in the bundled installer.
    let exe = std::env::current_exe().map_err(|e| HostsApplyError::Io {
        message: format!("current_exe failed: {e}"),
    })?;

    // Build the parameter string before spawning the worker thread.
    // OsString concatenation preserves the platform-native encoding,
    // so non-ASCII path components round-trip cleanly through the
    // subsequent UTF-16 conversion.
    let mut params = OsString::new();
    params.push("--swh-elevated-apply-hosts \"");
    params.push(src.as_os_str());
    params.push("\" \"");
    params.push(dst.as_os_str());
    params.push("\"");

    let verb = to_wide(OsStr::new("runas"));
    let file = to_wide(exe.as_os_str());
    let params_w = to_wide(&params);

    // Captured into the worker for use in the failure-path error
    // message — we don't want to keep the &Path borrow alive across
    // the thread boundary.
    let src_display = src.to_string_lossy().into_owned();
    let dst_display = dst.to_string_lossy().into_owned();

    // ShellExecuteExW must be called from a thread that has been
    // initialised as a single-threaded apartment (STA). The Tokio
    // worker threads our async commands run on do not carry COM
    // state, so we hop onto a fresh OS thread that we initialise
    // ourselves. The thread is short-lived (one elevated copy +
    // wait + cleanup) and the join blocks the calling task,
    // matching the synchronous semantics elsewhere in the apply
    // pipeline.
    let outcome = std::thread::spawn(move || -> Result<(), HostsApplyError> {
        unsafe {
            // CoInitializeEx returns:
            //   S_OK            (0)        — first init on this thread
            //   S_FALSE         (1)        — already inited in same mode
            //   RPC_E_CHANGED_MODE (negative) — already inited in MTA
            //
            // S_OK and S_FALSE both increment the per-thread COM
            // refcount and require a matching CoUninitialize. We
            // gate cleanup on `hr >= 0` so we only release a refcount
            // we actually took. RPC_E_CHANGED_MODE means the existing
            // initialisation is fine for our purposes (the OS still
            // dispatches the verb), and we leave its refcount alone.
            let hr = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
            let we_inited = hr >= 0;

            // Cleanup helper closures so every error path drops the
            // process handle and the COM refcount in the right order.
            let mut info: SHELLEXECUTEINFOW = mem::zeroed();
            info.cbSize = mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = params_w.as_ptr();
            info.nShow = SW_HIDE;

            let success = ShellExecuteExW(&mut info);
            if success == 0 {
                let last_err = GetLastError();
                if we_inited {
                    CoUninitialize();
                }
                if last_err == ERROR_CANCELLED {
                    return Err(HostsApplyError::Cancelled);
                }
                return Err(HostsApplyError::Io {
                    message: format!(
                        "ShellExecuteExW failed: GetLastError={last_err}; src={src_display}, dst={dst_display}"
                    ),
                });
            }

            // SEE_MASK_NOCLOSEPROCESS guarantees `hProcess` is set
            // when the call succeeds. We defensively guard against a
            // null handle anyway, then synchronously wait for the
            // elevated cmd to terminate.
            let process = info.hProcess;
            if process.is_null() {
                if we_inited {
                    CoUninitialize();
                }
                return Err(HostsApplyError::Io {
                    message: "ShellExecuteExW returned a null process handle".to_string(),
                });
            }

            let wait = WaitForSingleObject(process, INFINITE);
            if wait != WAIT_OBJECT_0 {
                CloseHandle(process);
                if we_inited {
                    CoUninitialize();
                }
                return Err(HostsApplyError::Io {
                    message: format!("WaitForSingleObject returned {wait}"),
                });
            }

            let mut exit_code: u32 = 0;
            let got_exit = GetExitCodeProcess(process, &mut exit_code as *mut u32);
            CloseHandle(process);
            if we_inited {
                CoUninitialize();
            }
            if got_exit == 0 {
                return Err(HostsApplyError::Io {
                    message: "GetExitCodeProcess failed".to_string(),
                });
            }
            // `cmd /c copy` returns 0 on success, 1 on failure.
            // Non-zero is treated as a generic Io failure; the user
            // sees a "fail" code in the renderer rather than a
            // misleading "no_access" branch.
            if exit_code == 0 {
                Ok(())
            } else {
                Err(HostsApplyError::Io {
                    message: format!(
                        "elevated copy failed: cmd /c copy exit code {exit_code}; src={src_display}, dst={dst_display}"
                    ),
                })
            }
        }
    })
    .join();

    match outcome {
        Ok(result) => result,
        Err(_panic) => Err(HostsApplyError::Io {
            message: "elevation worker thread panicked".to_string(),
        }),
    }
}
