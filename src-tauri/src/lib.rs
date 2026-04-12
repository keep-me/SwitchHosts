mod commands;
mod hosts_apply;
mod http;
mod import_export;
mod lifecycle;
mod migration;
mod refresh;
mod storage;
mod tray;

use serde_json::json;
use tauri::{Emitter, Listener, Manager, RunEvent};

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
            // Tray menu items are routed to the tray module's
            // dispatch table; popup_menu items are forwarded back to
            // the renderer as same-named Tauri events. Order matters
            // only because tray ids are short and never collide with
            // the long renderer-generated `popup_menu_item_*` ids.
            if id.starts_with("tray-") {
                tray::handle_menu_event(app, id);
                return;
            }
            if id.starts_with("popup_menu_item_") {
                let _ = app.emit(id, json!({ "_args": [] }));
            }
        })
        .setup(|app| {
            // Build the main window programmatically with any saved
            // geometry baked into the builder. Doing this in Rust (as
            // opposed to tauri.conf.json) is the only way to avoid
            // the one-frame flash between the default center position
            // and the restored position on macOS: set_position on a
            // window declared by conf.json doesn't always take effect
            // before the compositor paints the first frame.
            let app_handle = app.handle().clone();
            let app_state = app.state::<AppState>();
            let main = lifecycle::create_main_window(&app_handle, app_state.inner())?;

            // Handlers are installed right after build, before any
            // user interaction, so no Moved/Resized events are lost.
            lifecycle::install_main_window_handlers(&main);
            let _ = main.set_focus();

            // Tray icon must exist before we honour `hide_dock_icon`,
            // otherwise an `Accessory` activation policy on macOS would
            // strand the user (no Dock icon, no tray to summon the
            // window back).
            tray::install_tray(&app_handle)?;

            #[cfg(target_os = "macos")]
            {
                let hide = app_state
                    .config
                    .lock()
                    .map(|cfg| cfg.hide_dock_icon)
                    .unwrap_or(false);
                lifecycle::apply_dock_icon_policy(&app.handle(), hide);
            }

            // The tray window (P2.B.2) and a few existing dialogs
            // (SetWriteMode, SudoPasswordInput) all broadcast
            // `events.active_main_window` when they want the main
            // window to come forward. The Electron build had a
            // matching `message.on('active_main_window', onActive)`
            // handler in `src/main/main.ts`; we mirror it via the
            // global event bus so the renderer's existing call sites
            // keep working unchanged.
            let active_main_app = app_handle.clone();
            app.listen("active_main_window", move |_event| {
                lifecycle::focus_main_on_second_instance(
                    &active_main_app,
                    Vec::new(),
                    String::new(),
                );
            });

            // Background scanner for remote-hosts auto refresh.
            // Wakes every 60s, replaces `src/main/libs/cron.ts`.
            refresh::start_background_scanner(app_handle.clone());

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
