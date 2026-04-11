mod commands;
mod import_export;
mod lifecycle;
mod migration;
mod storage;

use serde_json::json;
use tauri::{Emitter, Manager, RunEvent};

use storage::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::bootstrap()
        .expect("failed to bootstrap SwitchHosts v5 storage layer");

    let app = tauri::Builder::default()
        // Single-instance MUST be the first plugin so a second
        // launch is intercepted before any other plugin starts up.
        .plugin(tauri_plugin_single_instance::init(
            |app, args, cwd| lifecycle::focus_main_on_second_instance(app, args, cwd),
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        // Popup menu item clicks are routed back to the renderer as Tauri
        // events: the menu item id equals the renderer-generated
        // `_click_evt` string, so forwarding the id verbatim as an event
        // name lets the existing `agent.once(_click_evt, handler)` pattern
        // keep working without any renderer changes.
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id.starts_with("popup_menu_item_") {
                let _ = app.emit(id, json!({ "_args": [] }));
            }
        })
        .setup(|app| {
            let main = app
                .get_webview_window(lifecycle::MAIN_WINDOW_LABEL)
                .expect("main window declared in tauri.conf.json");

            // Install Moved/Resized/CloseRequested handlers before
            // any restore work so early events (e.g. platform-driven
            // initial resize) get captured.
            lifecycle::install_main_window_handlers(&main);

            // Restore main window geometry from internal/state.json
            // (or center on first launch) and reveal the window. The
            // window starts as visible:false in tauri.conf.json so
            // there is no flicker between the default position and
            // the restored position.
            let app_state = app.state::<AppState>();
            lifecycle::restore_and_show_main(&main, app_state.inner());

            // macOS Dock icon visibility, read once from config. P2.A
            // leaves this as a no-op with a warning; P2.B will wire
            // it up alongside the tray icon.
            #[cfg(target_os = "macos")]
            {
                let hide = app_state
                    .config
                    .lock()
                    .map(|cfg| cfg.hide_dock_icon)
                    .unwrap_or(false);
                lifecycle::apply_dock_icon_policy(&app.handle(), hide);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // startup critical
            commands::ping,
            commands::get_basic_data,
            commands::migration_status,
            commands::dark_mode_toggle,
            // config
            commands::config_all,
            commands::config_get,
            commands::config_set,
            commands::config_update,
            // list / tree
            commands::get_list,
            commands::get_item_from_list,
            commands::get_content_of_list,
            commands::set_list,
            commands::move_to_trashcan,
            commands::move_many_to_trashcan,
            commands::get_trashcan_list,
            commands::clear_trashcan,
            commands::delete_item_from_trashcan,
            commands::restore_item_from_trashcan,
            // hosts content
            commands::get_hosts_content,
            commands::set_hosts_content,
            commands::get_system_hosts,
            commands::get_path_of_system_hosts,
            // apply / refresh
            commands::apply_hosts_selection,
            commands::toggle_hosts_item,
            commands::refresh_remote_hosts,
            commands::refresh_all_remote_hosts,
            commands::get_apply_history,
            commands::delete_apply_history_item,
            // cmd_after_hosts_apply history
            commands::cmd_get_history_list,
            commands::cmd_delete_history_item,
            commands::cmd_clear_history,
            // find window
            commands::find_show,
            commands::find_by,
            commands::find_add_history,
            commands::find_get_history,
            commands::find_set_history,
            commands::find_add_replace_history,
            commands::find_get_replace_history,
            commands::find_set_replace_history,
            // window / misc
            commands::hide_main_window,
            commands::focus_main_window,
            commands::quit_app,
            commands::update_tray_title,
            commands::open_url,
            commands::show_item_in_folder,
            commands::popup_menu,
            // import / export
            commands::export_data,
            commands::import_data,
            commands::import_data_from_url,
            // updater
            commands::check_update,
            commands::download_update,
            commands::install_update,
            // data dir
            commands::get_data_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Run-event hook covers two concerns that Builder's `.setup` and
    // window-level `on_window_event` can't reach:
    //   * ExitRequested — persist geometry on Cmd+Q / system shutdown
    //     paths that bypass our explicit quit_app command.
    //   * Reopen (macOS) — clicking the Dock icon for an app whose
    //     main window is hidden should re-show it. Tauri does not do
    //     this automatically; has_visible_windows == false means the
    //     OS didn't find any windows to bring forward.
    app.run(|app_handle, event| match event {
        RunEvent::ExitRequested { .. } => {
            lifecycle::persist_on_exit_requested(app_handle);
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            if !has_visible_windows {
                lifecycle::focus_main_on_second_instance(app_handle, Vec::new(), String::new());
            }
        }
        _ => {}
    });
}
