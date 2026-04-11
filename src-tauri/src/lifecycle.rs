//! Window lifecycle plumbing for the v5 main window.
//!
//! Phase 2.A scope:
//!
//! - Persist main window geometry into `internal/state.json` and
//!   restore it on the next launch.
//! - Treat the close button as "hide", the menu Quit / Cmd+Q as
//!   "exit", coordinated through `AppState::is_will_quit`.
//! - Honour `hide_dock_icon` once at startup on macOS.
//!
//! Tray and find window plumbing land in P2.B / P2.D and will reuse
//! the same persistence helpers.

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewWindow, WindowEvent,
};

use crate::storage::{
    state::{StateFile, WindowGeometry},
    AppState,
};

/// Minimum interval between two geometry persists driven by
/// Moved/Resized events, in milliseconds. macOS fires these events
/// continuously during a drag (60 Hz), so we coalesce to at most
/// 5 writes per second. CloseRequested / quit_app / ExitRequested
/// all bypass this throttle to guarantee the final position lands.
const GEOMETRY_PERSIST_THROTTLE_MS: u64 = 200;

pub const MAIN_WINDOW_LABEL: &str = "main";

// ---- main-window event handlers --------------------------------------------

/// Install all of the v5 main-window handlers:
///
/// - Moved / Resized: throttled geometry persist so a drag on macOS
///   doesn't hammer state.json at 60 Hz.
/// - CloseRequested: unthrottled persist followed by hide-instead-of-close
///   unless `is_will_quit` has been flipped by `quit_app`.
pub fn install_main_window_handlers<R: Runtime>(window: &WebviewWindow<R>) {
    let window_clone = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            let app = window_clone.app_handle().clone();
            let app_state = app.state::<AppState>();
            maybe_persist_window_geometry(&window_clone, app_state.inner());
        }
        WindowEvent::CloseRequested { api, .. } => {
            let app = window_clone.app_handle().clone();
            let app_state = app.state::<AppState>();

            // Unthrottled persist so the final position lands even
            // if the last drag event was within the throttle window.
            persist_window_geometry(&window_clone, app_state.inner());

            if app_state.is_will_quit.load(Ordering::SeqCst) {
                // Real quit path — let Tauri close the window normally.
                return;
            }

            // Default close-button behaviour: hide instead of close.
            api.prevent_close();
            let _ = window_clone.hide();
        }
        _ => {}
    });
}

// ---- geometry persistence --------------------------------------------------

/// Snapshot the window's current outer position + size into
/// `internal/state.json`. Failures are logged and swallowed because
/// losing window geometry is annoying but never user-data damaging.
///
/// The caller is responsible for rate-limiting; see
/// `maybe_persist_window_geometry` for the throttled variant used by
/// Moved/Resized handlers.
pub fn persist_window_geometry<R: Runtime>(window: &WebviewWindow<R>, app_state: &AppState) {
    let geometry = match read_geometry(window) {
        Ok(g) => g,
        Err(e) => {
            log::warn!("failed to read main window geometry: {e}");
            return;
        }
    };

    let mut state_file = StateFile::load(&app_state.paths.state_file);
    state_file.window.main = Some(geometry);
    if let Err(e) = state_file.save(&app_state.paths.state_file) {
        log::warn!("failed to persist main window geometry: {e}");
        return;
    }

    // Stamp the last-persist marker so the throttle in
    // `maybe_persist_window_geometry` skips near-duplicates for the
    // next 200 ms.
    app_state
        .last_geometry_persist_ms
        .store(now_ms(), Ordering::Relaxed);
}

/// Throttled wrapper for the Moved/Resized hot path. At most one
/// write every `GEOMETRY_PERSIST_THROTTLE_MS` milliseconds. The
/// CloseRequested / ExitRequested / quit_app paths all bypass this
/// by calling `persist_window_geometry` directly, so the user's final
/// position before exit always lands on disk.
fn maybe_persist_window_geometry<R: Runtime>(window: &WebviewWindow<R>, app_state: &AppState) {
    let now = now_ms();
    let last = app_state.last_geometry_persist_ms.load(Ordering::Relaxed);
    if now.saturating_sub(last) < GEOMETRY_PERSIST_THROTTLE_MS {
        return;
    }
    persist_window_geometry(window, app_state);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_geometry<R: Runtime>(window: &WebviewWindow<R>) -> Result<WindowGeometry, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let maximized = window.is_maximized().unwrap_or(false);

    let logical_pos: LogicalPosition<f64> = pos.to_logical(scale);
    let logical_size: LogicalSize<f64> = size.to_logical(scale);

    Ok(WindowGeometry {
        x: logical_pos.x.round() as i32,
        y: logical_pos.y.round() as i32,
        width: logical_size.width.round().max(1.0) as u32,
        height: logical_size.height.round().max(1.0) as u32,
        maximized,
    })
}

// ---- geometry restore at startup -------------------------------------------

/// Restore main window geometry from `internal/state.json` and then
/// reveal the window. The conf.json declaration starts the window as
/// `visible: false` so we don't get a flicker between the default
/// position and the restored position.
pub fn restore_and_show_main<R: Runtime>(window: &WebviewWindow<R>, app_state: &AppState) {
    let state = StateFile::load(&app_state.paths.state_file);
    if let Some(geom) = state.window.main {
        if geometry_is_visible_on_some_monitor(window, &geom) {
            apply_geometry(window, &geom);
        } else {
            log::info!(
                "saved main window geometry {geom:?} is off-screen — falling back to default position"
            );
            let _ = window.center();
        }
    } else {
        // First launch — keep the conf.json default centering.
        let _ = window.center();
    }

    let _ = window.show();
    let _ = window.set_focus();
}

fn apply_geometry<R: Runtime>(window: &WebviewWindow<R>, geom: &WindowGeometry) {
    let _ = window.set_position(LogicalPosition::new(geom.x as f64, geom.y as f64));
    let _ = window.set_size(LogicalSize::new(geom.width as f64, geom.height as f64));
    if geom.maximized {
        let _ = window.maximize();
    }
}

/// Conservative on-screen check: any monitor whose logical bounds
/// overlap the saved geometry counts as "visible". Multi-monitor
/// disconnects / DPI changes between launches are the main reason
/// this matters — we don't want to restore a window onto a monitor
/// that's no longer plugged in.
fn geometry_is_visible_on_some_monitor<R: Runtime>(
    window: &WebviewWindow<R>,
    geom: &WindowGeometry,
) -> bool {
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };
    for monitor in monitors {
        let scale = monitor.scale_factor();
        let pos = monitor.position().to_logical::<f64>(scale);
        let size = monitor.size().to_logical::<f64>(scale);
        let mx = pos.x;
        let my = pos.y;
        let mw = size.width;
        let mh = size.height;
        let overlaps_x = (geom.x as f64) < mx + mw && ((geom.x as f64) + geom.width as f64) > mx;
        let overlaps_y = (geom.y as f64) < my + mh && ((geom.y as f64) + geom.height as f64) > my;
        if overlaps_x && overlaps_y {
            return true;
        }
    }
    false
}

// ---- macOS dock icon -------------------------------------------------------

/// Honour `hide_dock_icon`. Only meaningful on macOS; called once at
/// startup, mirroring the Electron implementation.
///
/// **Intentionally a no-op in Phase 2.A**: setting
/// `ActivationPolicy::Accessory` without a tray icon to summon the
/// main window leaves the user with no way to get the window back
/// (no Dock icon, no tray, no menu bar). P2.B re-enables this once
/// the tray icon lands so "hide dock + use tray" actually works.
pub fn apply_dock_icon_policy<R: Runtime>(_app: &AppHandle<R>, hide: bool) {
    if hide {
        log::warn!(
            "hide_dock_icon = true is temporarily ignored — it will be honoured once the system tray lands in Phase 2.B. Without a tray, hiding the Dock icon would leave the main window unreachable."
        );
        eprintln!(
            "[v5 P2.A] hide_dock_icon is ignored in this build. Wait for Phase 2.B (tray icon) to enable it safely."
        );
    }
}

// ---- single instance handler -----------------------------------------------

/// Callback registered with `tauri-plugin-single-instance`. A second
/// invocation of SwitchHosts focuses the existing main window instead
/// of creating a duplicate.
pub fn focus_main_on_second_instance<R: Runtime>(app: &AppHandle<R>, _args: Vec<String>, _cwd: String) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ---- run-event hook for Cmd+Q / system shutdown ---------------------------

/// Hook registered with `app.run` so that geometry is persisted on
/// every exit-request path, even the ones that bypass our explicit
/// quit_app command (Cmd+Q on macOS, log-off / shutdown sequences).
pub fn persist_on_exit_requested<R: Runtime>(app: &AppHandle<R>) {
    let app_state = app.state::<AppState>();
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        persist_window_geometry(&window, app_state.inner());
    }
}
