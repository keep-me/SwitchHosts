mod commands;
mod storage;

use storage::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::bootstrap()
        .expect("failed to bootstrap SwitchHosts v5 storage layer");

    tauri::Builder::default()
        .manage(state)
        .setup(|_app| Ok(()))
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
