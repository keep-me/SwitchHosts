//! Tauri commands.
//!
//! Phase 1A stubs landed everything as `_args: Vec<serde_json::Value>`
//! returning fixtures. Phase 1B steps progressively replace stubs with
//! real implementations backed by the `storage` module.
//!
//! Convention: every command accepts `args: Vec<serde_json::Value>` to
//! match the positional-argument marshalling the front-end adapter uses
//! (`src/renderer/core/agent.ts` sends `invoke(cmd, { args: params })`).
//! Commands also take a `State<'_, AppState>` when they need shared
//! storage access.

use serde_json::{json, Value};
use tauri::State;

use crate::storage::{AppState, StorageError};

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
pub async fn config_all(state: State<'_, AppState>, _args: Args) -> Result<Value, StorageError> {
    let cfg = state.config.lock().expect("config mutex poisoned");
    Ok(cfg.to_flat_value())
}

#[tauri::command]
pub async fn config_get(state: State<'_, AppState>, args: Args) -> Result<Value, StorageError> {
    let key = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::InvalidConfigValue {
            key: "<arg0>".into(),
            reason: "config_get requires a string key as the first argument".into(),
        })?;
    let cfg = state.config.lock().expect("config mutex poisoned");
    Ok(cfg.get_key(key).unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn config_set(state: State<'_, AppState>, args: Args) -> Result<Value, StorageError> {
    let key = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::InvalidConfigValue {
            key: "<arg0>".into(),
            reason: "config_set requires a string key as the first argument".into(),
        })?
        .to_string();
    let value = args.get(1).cloned().unwrap_or(Value::Null);

    {
        let mut cfg = state.config.lock().expect("config mutex poisoned");
        cfg.set_key(&key, value)?;
    }
    state.persist_config()?;
    Ok(Value::Null)
}

#[tauri::command]
pub async fn config_update(state: State<'_, AppState>, args: Args) -> Result<Value, StorageError> {
    let patch = args.first().cloned().unwrap_or(Value::Null);
    if patch.is_null() {
        return Err(StorageError::InvalidConfigValue {
            key: "<arg0>".into(),
            reason: "config_update requires a partial object as the first argument".into(),
        });
    }
    {
        let mut cfg = state.config.lock().expect("config mutex poisoned");
        cfg.apply_partial(&patch)?;
    }
    state.persist_config()?;
    Ok(Value::Null)
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
pub async fn get_data_dir(state: State<'_, AppState>, _args: Args) -> Result<Value, StorageError> {
    Ok(json!(state.paths.root.display().to_string()))
}
