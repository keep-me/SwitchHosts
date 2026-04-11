//! Phase 1A command stubs.
//!
//! Every command the renderer may call is registered here as a `#[tauri::command]`
//! returning an empty / fixture JSON value. This is enough to unblock the first
//! render of the main window. Phase 1B will replace the stubs with real domain
//! services that read/write `~/.SwitchHosts`.
//!
//! Convention: each command accepts `args: Vec<serde_json::Value>` to match the
//! positional-argument marshalling that the front-end adapter layer
//! (`src/renderer/core/agent.ts`) uses. Commands ignore `args` for now.

use serde_json::{json, Value};

type Args = Vec<Value>;

// ---- startup critical ------------------------------------------------------

#[tauri::command]
pub async fn ping(_args: Args) -> Value {
    json!("pong")
}

#[tauri::command]
pub async fn get_basic_data(_args: Args) -> Value {
    json!({
        "list": [],
        "trashcan": [],
        "version": [4, 3, 0, 6140],
    })
}

#[tauri::command]
pub async fn migration_status(_args: Args) -> Value {
    // Phase 1A has no real PotDb migration yet; report "no migration needed".
    json!(false)
}

#[tauri::command]
pub async fn dark_mode_toggle(_args: Args) -> Value {
    // Phase 1B will call Tauri window `set_theme`.
    Value::Null
}

// ---- config ----------------------------------------------------------------

#[tauri::command]
pub async fn config_all(_args: Args) -> Value {
    // Must match ConfigsType in src/common/default_configs.ts.
    json!({
        "left_panel_show": true,
        "left_panel_width": 270,
        "use_system_window_frame": false,
        "write_mode": "append",
        "history_limit": 50,
        "locale": null,
        "theme": "light",
        "choice_mode": 2,
        "show_title_on_tray": false,
        "hide_at_launch": false,
        "send_usage_data": false,
        "cmd_after_hosts_apply": "",
        "remove_duplicate_records": false,
        "hide_dock_icon": false,
        "use_proxy": false,
        "proxy_protocol": "http",
        "proxy_host": "",
        "proxy_port": 0,
        "http_api_on": false,
        "http_api_only_local": true,
        "tray_mini_window": true,
        "multi_chose_folder_switch_all": false,
        "auto_download_update": true,
        "env": "PROD",
    })
}

#[tauri::command]
pub async fn config_get(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn config_set(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn config_update(_args: Args) -> Value {
    Value::Null
}

// ---- list / tree -----------------------------------------------------------

#[tauri::command]
pub async fn get_list(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn get_item_from_list(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn get_content_of_list(_args: Args) -> Value {
    json!("")
}

#[tauri::command]
pub async fn set_list(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn move_to_trashcan(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn move_many_to_trashcan(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn get_trashcan_list(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn clear_trashcan(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn delete_item_from_trashcan(_args: Args) -> Value {
    json!(true)
}

#[tauri::command]
pub async fn restore_item_from_trashcan(_args: Args) -> Value {
    json!(true)
}

// ---- hosts content ---------------------------------------------------------

#[tauri::command]
pub async fn get_hosts_content(_args: Args) -> Value {
    json!("")
}

#[tauri::command]
pub async fn set_hosts_content(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn get_system_hosts(_args: Args) -> Value {
    json!("")
}

#[tauri::command]
pub async fn get_path_of_system_hosts(_args: Args) -> Value {
    #[cfg(target_os = "windows")]
    let path = r"C:\Windows\System32\drivers\etc\hosts";
    #[cfg(not(target_os = "windows"))]
    let path = "/etc/hosts";
    json!(path)
}

// ---- apply / refresh -------------------------------------------------------

#[tauri::command]
pub async fn apply_hosts_selection(_args: Args) -> Value {
    json!({ "success": true, "message": "[v5 stub]" })
}

#[tauri::command]
pub async fn toggle_hosts_item(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn refresh_remote_hosts(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn refresh_all_remote_hosts(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn get_apply_history(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn delete_apply_history_item(_args: Args) -> Value {
    Value::Null
}

// ---- cmd_after_hosts_apply history -----------------------------------------

#[tauri::command]
pub async fn cmd_get_history_list(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn cmd_delete_history_item(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn cmd_clear_history(_args: Args) -> Value {
    Value::Null
}

// ---- find window -----------------------------------------------------------

#[tauri::command]
pub async fn find_show(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn find_by(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn find_add_history(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn find_get_history(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn find_set_history(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn find_add_replace_history(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn find_get_replace_history(_args: Args) -> Value {
    json!([])
}

#[tauri::command]
pub async fn find_set_replace_history(_args: Args) -> Value {
    Value::Null
}

// ---- window / misc ---------------------------------------------------------

#[tauri::command]
pub async fn hide_main_window(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn focus_main_window(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn quit_app(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn update_tray_title(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn open_url(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn show_item_in_folder(_args: Args) -> Value {
    Value::Null
}

// ---- import / export -------------------------------------------------------

#[tauri::command]
pub async fn export_data(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn import_data(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn import_data_from_url(_args: Args) -> Value {
    Value::Null
}

// ---- updater ---------------------------------------------------------------

#[tauri::command]
pub async fn check_update(_args: Args) -> Value {
    json!({ "has_update": false })
}

#[tauri::command]
pub async fn download_update(_args: Args) -> Value {
    Value::Null
}

#[tauri::command]
pub async fn install_update(_args: Args) -> Value {
    Value::Null
}

// ---- data dir --------------------------------------------------------------

#[tauri::command]
pub async fn get_data_dir(_args: Args) -> Value {
    // Phase 1B will compute ~/.SwitchHosts here.
    let path = std::env::var("HOME")
        .map(|h| format!("{}/.SwitchHosts", h))
        .unwrap_or_else(|_| "~/.SwitchHosts".to_string());
    json!(path)
}
