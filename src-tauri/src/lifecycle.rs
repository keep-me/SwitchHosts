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
    webview::WebviewWindowBuilder, AppHandle, LogicalPosition, LogicalSize, Manager, Monitor,
    Runtime, WebviewUrl, WebviewWindow, WindowEvent,
};

use crate::storage::{
    state::{StateFile, WindowGeometry},
    AppState, StorageError,
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

// ---- main window creation --------------------------------------------------

/// Create the main window programmatically, baking any saved geometry
/// into the `WebviewWindowBuilder` so the window appears at its final
/// position on the very first frame. We deliberately *don't* declare
/// the main window in `tauri.conf.json` — doing it there means the
/// window is created with the conf-level defaults (centered, default
/// size), and a subsequent `set_position` call from `setup` can't
/// avoid a one-frame flash between the default position and the
/// restored position.
///
/// Called once from `lib.rs::run` inside the Builder's setup hook.
/// Returns the freshly-created main window; the caller installs
/// event handlers on it.
pub fn create_main_window<R: Runtime>(
    app: &AppHandle<R>,
    app_state: &AppState,
) -> Result<WebviewWindow<R>, StorageError> {
    let state = StateFile::load(&app_state.paths.state_file);
    let saved = state.window.main;

    let monitors = app.available_monitors().unwrap_or_default();
    let saved_on_screen = saved
        .as_ref()
        .map(|g| geometry_is_visible_on_monitors(&monitors, g))
        .unwrap_or(false);

    let builder = WebviewWindowBuilder::new(
        app,
        MAIN_WINDOW_LABEL,
        WebviewUrl::App("/".into()),
    )
    .title("SwitchHosts")
    .min_inner_size(300.0, 200.0)
    .resizable(true);

    #[cfg(target_os = "macos")]
    let mut builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(12.0, 14.0));
    #[cfg(not(target_os = "macos"))]
    let mut builder = builder.decorations(false).shadow(true);

    if saved_on_screen {
        let geom = saved.as_ref().unwrap();
        builder = builder
            .position(geom.x as f64, geom.y as f64)
            .inner_size(geom.width as f64, geom.height as f64);
    } else {
        if saved.is_some() {
            log::info!(
                "saved main window geometry is off-screen — falling back to centered default"
            );
        }
        builder = builder.inner_size(800.0, 480.0).center();
    }

    let window = builder
        .build()
        .map_err(|e| StorageError::Io {
            path: MAIN_WINDOW_LABEL.to_string(),
            reason: e.to_string(),
        })?;

    // Apply maximize after build so it takes priority over the baked
    // initial size without flashing — the window hasn't been painted
    // yet at this point.
    if let Some(geom) = saved {
        if saved_on_screen && geom.maximized {
            let _ = window.maximize();
        }
    }

    Ok(window)
}

/// Conservative on-screen check: any monitor whose logical bounds
/// overlap the saved geometry counts as "visible". Multi-monitor
/// disconnects / DPI changes between launches are the main reason
/// this matters — we don't want to restore a window onto a monitor
/// that's no longer plugged in.
fn geometry_is_visible_on_monitors(monitors: &[Monitor], geom: &WindowGeometry) -> bool {
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

/// Honour `hide_dock_icon`. Only meaningful on macOS — switches the
/// app between `Regular` (Dock icon visible, full menu bar) and
/// `Accessory` (no Dock icon, tray-only). Safe to call at runtime
/// because P2.B installed a tray icon as a permanent way to summon
/// the window back.
#[cfg(target_os = "macos")]
pub fn apply_dock_icon_policy<R: Runtime>(app: &AppHandle<R>, hide: bool) {
    let policy = if hide {
        tauri::ActivationPolicy::Accessory
    } else {
        tauri::ActivationPolicy::Regular
    };
    if let Err(e) = app.set_activation_policy(policy) {
        log::warn!("failed to set activation policy: {e}");
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
