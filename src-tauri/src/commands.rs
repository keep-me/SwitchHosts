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
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::{AppHandle, Emitter, Runtime, State, WebviewWindow};

use crate::storage::{
    entries, manifest::{self, Manifest},
    AppState, StorageError, Trashcan,
};

type Args = Vec<Value>;

// ---- small helpers ---------------------------------------------------------

fn load_manifest(state: &AppState) -> Result<Manifest, StorageError> {
    Manifest::load(&state.paths.manifest_file)
}

fn save_manifest(state: &AppState, m: &Manifest) -> Result<(), StorageError> {
    m.save(&state.paths.manifest_file)
}

fn load_trashcan(state: &AppState) -> Result<Trashcan, StorageError> {
    Trashcan::load(&state.paths.trashcan_file)
}

fn save_trashcan(state: &AppState, t: &Trashcan) -> Result<(), StorageError> {
    t.save(&state.paths.trashcan_file)
}

fn arg_str<'a>(args: &'a Args, index: usize, field: &'static str) -> Result<&'a str, StorageError> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::InvalidConfigValue {
            key: field.into(),
            reason: format!("expected a string at args[{index}]"),
        })
}

// ---- startup critical ------------------------------------------------------

#[tauri::command]
pub async fn ping(_args: Args) -> Value {
    json!("pong")
}

#[tauri::command]
pub async fn get_basic_data(
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, StorageError> {
    let manifest = load_manifest(&state)?;
    let trashcan = load_trashcan(&state)?;
    Ok(json!({
        "list": manifest.root,
        "trashcan": trashcan.items,
        "version": [4, 3, 0, 6140],
    }))
}

#[tauri::command]
pub async fn migration_status(_args: Args) -> Value {
    // In v5, PotDb → v5 migration runs once inside `AppState::bootstrap`
    // before the renderer is ever served. By the time this command is
    // reachable from the renderer, migration has already been attempted
    // (and either applied or skipped). Returning `false` tells the old
    // Electron-era `actions.migrateCheck()` caller in index.tsx that it
    // should not prompt the user — which is what we want in v5.
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
pub async fn get_list(state: State<'_, AppState>, _args: Args) -> Result<Value, StorageError> {
    let m = load_manifest(&state)?;
    Ok(Value::Array(m.root))
}

#[tauri::command]
pub async fn get_item_from_list(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?;
    let m = load_manifest(&state)?;
    Ok(manifest::find_node(&m.root, id).unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn get_content_of_list(_args: Args) -> Value {
    // Content aggregation (group.include resolution + dedup + apply
    // pipeline) lands with `hosts_apply` in Phase 2. Phase 1B keeps
    // this inert so the renderer's apply button no-ops cleanly.
    json!("")
}

#[tauri::command]
pub async fn set_list(state: State<'_, AppState>, args: Args) -> Result<Value, StorageError> {
    let list = args.into_iter().next().unwrap_or(Value::Null);
    let root = match list {
        Value::Array(arr) => arr,
        Value::Null => Vec::new(),
        _ => {
            return Err(StorageError::InvalidConfigValue {
                key: "set_list.args[0]".into(),
                reason: "expected an array of host nodes".into(),
            });
        }
    };
    let _guard = state.store_lock.lock().expect("store lock poisoned");
    let mut m = load_manifest(&state).unwrap_or_default();
    m.root = root;
    save_manifest(&state, &m)?;
    Ok(Value::Null)
}

#[tauri::command]
pub async fn move_to_trashcan(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?.to_string();
    let _guard = state.store_lock.lock().expect("store lock poisoned");
    move_ids_to_trashcan(&state, &[id])?;
    Ok(Value::Null)
}

#[tauri::command]
pub async fn move_many_to_trashcan(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let ids_value = args.into_iter().next().unwrap_or(Value::Null);
    let ids: Vec<String> = match ids_value {
        Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => {
            return Err(StorageError::InvalidConfigValue {
                key: "move_many_to_trashcan.args[0]".into(),
                reason: "expected an array of ids".into(),
            });
        }
    };
    let _guard = state.store_lock.lock().expect("store lock poisoned");
    move_ids_to_trashcan(&state, &ids)?;
    Ok(Value::Null)
}

fn move_ids_to_trashcan(state: &AppState, ids: &[String]) -> Result<(), StorageError> {
    let mut m = load_manifest(state).unwrap_or_default();
    let mut t = load_trashcan(state).unwrap_or_default();
    for id in ids {
        if let Some((node, parent_id)) = manifest::remove_node(&mut m.root, id) {
            t.add_item(node, parent_id);
        }
    }
    save_manifest(state, &m)?;
    save_trashcan(state, &t)?;
    Ok(())
}

#[tauri::command]
pub async fn get_trashcan_list(
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, StorageError> {
    let t = load_trashcan(&state)?;
    Ok(Value::Array(t.items))
}

#[tauri::command]
pub async fn clear_trashcan(
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, StorageError> {
    let _guard = state.store_lock.lock().expect("store lock poisoned");
    let mut t = load_trashcan(&state).unwrap_or_default();
    t.items.clear();
    save_trashcan(&state, &t)?;
    // Note: we deliberately do NOT delete orphaned entries/*.hosts
    // files here. Garbage collection of orphan content files lands
    // alongside the "permanent delete" flow in Phase 2.
    Ok(Value::Null)
}

#[tauri::command]
pub async fn delete_item_from_trashcan(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?.to_string();
    let _guard = state.store_lock.lock().expect("store lock poisoned");
    let mut t = load_trashcan(&state).unwrap_or_default();
    let removed = t.remove_item(&id).is_some();
    save_trashcan(&state, &t)?;
    Ok(json!(removed))
}

#[tauri::command]
pub async fn restore_item_from_trashcan(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?.to_string();
    let _guard = state.store_lock.lock().expect("store lock poisoned");
    let mut t = load_trashcan(&state).unwrap_or_default();
    let item = match t.remove_item(&id) {
        Some(item) => item,
        None => return Ok(json!(false)),
    };

    let parent_id = item
        .get("parent_id")
        .and_then(Value::as_str)
        .map(String::from);
    let node = item.get("data").cloned().unwrap_or(Value::Null);
    if node.is_null() {
        // Trashcan entry was malformed — save the (now-shorter)
        // trashcan and report failure so the UI shows an error.
        save_trashcan(&state, &t)?;
        return Ok(json!(false));
    }

    let mut m = load_manifest(&state).unwrap_or_default();
    manifest::insert_node(&mut m.root, node, parent_id.as_deref());
    save_manifest(&state, &m)?;
    save_trashcan(&state, &t)?;
    Ok(json!(true))
}

// ---- hosts content ---------------------------------------------------------

#[tauri::command]
pub async fn get_hosts_content(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?;
    let content = entries::read_entry(&state.paths.entries_dir, id)?;
    Ok(json!(content))
}

#[tauri::command]
pub async fn set_hosts_content(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?.to_string();
    let content = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::InvalidConfigValue {
            key: "set_hosts_content.args[1]".into(),
            reason: "expected a string content at args[1]".into(),
        })?
        .to_string();
    entries::write_entry(&state.paths.entries_dir, &id, &content)?;
    Ok(Value::Null)
}

#[tauri::command]
pub async fn get_system_hosts(_args: Args) -> Result<Value, StorageError> {
    let path = system_hosts_path();
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(json!(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!("")),
        Err(e) => Err(StorageError::io(path.to_string(), e)),
    }
}

#[tauri::command]
pub async fn get_path_of_system_hosts(_args: Args) -> Value {
    json!(system_hosts_path())
}

fn system_hosts_path() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        r"C:\Windows\System32\drivers\etc\hosts"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "/etc/hosts"
    }
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

// ---- popup menu ------------------------------------------------------------
//
// The renderer's PopupMenu helper stays unchanged: for each menu item with
// a click handler it generates a unique `_click_evt` event name, registers
// an `agent.once(_click_evt, handler)`, then calls `agent.popupMenu({menu_id,
// items})`. We build a Tauri context menu using the same `_click_evt` strings
// as menu item ids, show it at the cursor, then emit a close signal. The
// matching click event is fan-out by the `.on_menu_event(...)` handler
// installed in `lib.rs` which forwards any menu id starting with
// `popup_menu_item_` as a same-named Tauri event.

#[tauri::command]
pub fn popup_menu<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    args: Args,
) -> Result<Value, String> {
    let spec = args.into_iter().next().unwrap_or(Value::Null);
    let menu_id = spec
        .get("menu_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let items: Vec<Value> = spec
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut builder = MenuBuilder::new(&app);
    let mut fallback_counter = 0u32;
    for item in &items {
        let item_type = item.get("type").and_then(Value::as_str);
        if item_type == Some("separator") {
            builder = builder.separator();
            continue;
        }

        let label = item.get("label").and_then(Value::as_str).unwrap_or("");
        let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let id = match item.get("_click_evt").and_then(Value::as_str) {
            Some(evt) if !evt.is_empty() => evt.to_string(),
            _ => {
                fallback_counter += 1;
                format!("__swh_popup_noop_{fallback_counter}")
            }
        };

        let mi = MenuItemBuilder::with_id(&id, label)
            .enabled(enabled)
            .build(&app)
            .map_err(|e| e.to_string())?;
        builder = builder.item(&mi);
    }

    let menu = builder.build().map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())?;

    // The popup call is synchronous on all three desktop platforms (NSMenu
    // modal on macOS, TrackPopupMenu with TPM_RETURNCMD on Windows, GTK main
    // iteration loop on Linux). By the time it returns, any click event has
    // already been routed through the on_menu_event handler, so emitting the
    // close signal now is safe.
    let _ = app.emit(
        &format!("popup_menu_close:{menu_id}"),
        json!({ "_args": [] }),
    );

    Ok(Value::Null)
}

// ---- data dir --------------------------------------------------------------

#[tauri::command]
pub async fn get_data_dir(state: State<'_, AppState>, _args: Args) -> Result<Value, StorageError> {
    Ok(json!(state.paths.root.display().to_string()))
}
