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

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::hosts_apply::{self, ApplyHistoryItem, HostsApplyError};
use crate::import_export;
use crate::lifecycle::{self, MAIN_WINDOW_LABEL};
use crate::storage::{
    entries, manifest::{self, Manifest},
    AppState, StorageError, Trashcan,
};

/// Per-process counter so apply-history ids generated within the
/// same nanosecond are still unique. Cheap, opaque, never compared
/// across machines or runs — adequate for journal entries.
static APPLY_HISTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn make_history_id(now_ms: i64) -> String {
    let seq = APPLY_HISTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("apply_{now_ms}_{seq}")
}

type Args = Vec<Value>;

// ---- small helpers ---------------------------------------------------------

fn load_manifest(state: &AppState) -> Result<Manifest, StorageError> {
    Manifest::load(&state.paths)
}

fn save_manifest(state: &AppState, m: &Manifest) -> Result<(), StorageError> {
    m.save(&state.paths)
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
pub async fn get_content_of_list(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    // The renderer hands us its current in-memory list as args[0]; we
    // intentionally do NOT re-read manifest.json here. The Apply button
    // is supposed to write whatever the user is looking at, including
    // edits that haven't yet been persisted via set_list.
    let list_value = args.into_iter().next().unwrap_or(Value::Null);
    let list: Vec<Value> = match list_value {
        Value::Array(arr) => arr,
        Value::Null => Vec::new(),
        _ => {
            return Err(StorageError::InvalidConfigValue {
                key: "get_content_of_list.args[0]".into(),
                reason: "expected an array of host nodes".into(),
            });
        }
    };

    let remove_duplicate = {
        let cfg = state.config.lock().expect("config mutex poisoned");
        cfg.remove_duplicate_records
    };

    let content = hosts_apply::aggregate_selected_content(
        &list,
        &state.paths,
        remove_duplicate,
    )?;
    Ok(json!(content))
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
pub async fn apply_hosts_selection<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, String> {
    // args[0] = content (string, already aggregated by get_content_of_list)
    // args[1] = options ({ sudo_pswd? }) — ignored under v5/Tauri because
    //          OS-native elevation handles credentials.
    let content = match args.first().and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
            return Ok(json!({
                "success": false,
                "code": "fail",
                "message": "apply_hosts_selection: args[0] must be a string",
            }));
        }
    };

    let (write_mode, history_limit) = {
        let cfg = state.config.lock().expect("config mutex poisoned");
        (cfg.write_mode.clone(), cfg.history_limit as i32)
    };

    // The privileged write is potentially long-running (waits for the
    // user at the OS auth prompt) and *must not* hold the store_lock —
    // see implementation-notes A5. We do all the work outside the lock
    // and only retake it for the history journal write below.
    let outcome = match hosts_apply::apply_to_system_hosts(&content, &write_mode) {
        Ok(o) => o,
        Err(HostsApplyError::Cancelled) => {
            return Ok(HostsApplyError::Cancelled.into_renderer_value());
        }
        Err(e) => {
            eprintln!("[v5 apply] {e}");
            return Ok(e.into_renderer_value());
        }
    };

    // Persist apply history (mirrors Electron behaviour: insert
    // previous content first if it differs from the last entry, then
    // insert the new content). Skip the journal updates entirely when
    // the file was already up-to-date — we don't want a noop apply
    // to spam the history.
    if !outcome.unchanged {
        let history_path = state.paths.histories_dir.join("system-hosts.json");
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Step 1: previous content, only if not redundant.
        let existing = hosts_apply::history::load(&history_path).unwrap_or_default();
        let last_content = existing.last().map(|i| i.content.as_str());
        if last_content != Some(outcome.previous_content.as_str()) {
            let item = ApplyHistoryItem {
                id: make_history_id(now_ms),
                content: outcome.previous_content.clone(),
                add_time_ms: now_ms,
                label: None,
            };
            if let Err(e) = hosts_apply::history::insert(&history_path, item, history_limit) {
                eprintln!("[v5 apply] failed to write previous content history: {e}");
            }
        }

        // Step 2: new content.
        let new_item = ApplyHistoryItem {
            id: make_history_id(now_ms),
            content: outcome.new_content.clone(),
            add_time_ms: now_ms,
            label: None,
        };
        if let Err(e) = hosts_apply::history::insert(&history_path, new_item, history_limit) {
            eprintln!("[v5 apply] failed to write new content history: {e}");
        }
    }

    // Notify any listening windows that the system file has changed.
    // Editor.HostsEditor refreshes the system view; tray refreshes its
    // selection display. Wrapped in the standard `_args` envelope.
    let _ = app.emit("system_hosts_updated", json!({ "_args": [] }));

    Ok(json!({
        "success": true,
        "old_content": outcome.previous_content,
        "new_content": outcome.new_content,
    }))
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
pub async fn get_apply_history(
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, StorageError> {
    let path = state.paths.histories_dir.join("system-hosts.json");
    let items = hosts_apply::history::load(&path)?;
    let value = serde_json::to_value(items).map_err(|e| {
        StorageError::serialize(path.display().to_string(), e)
    })?;
    Ok(value)
}

#[tauri::command]
pub async fn delete_apply_history_item(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, StorageError> {
    let id = arg_str(&args, 0, "id")?;
    let path = state.paths.histories_dir.join("system-hosts.json");
    let removed = hosts_apply::history::delete_by_id(&path, id)?;
    Ok(json!(removed))
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
pub async fn hide_main_window<R: Runtime>(app: AppHandle<R>, _args: Args) -> Value {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.hide();
    }
    Value::Null
}

#[tauri::command]
pub async fn focus_main_window<R: Runtime>(app: AppHandle<R>, _args: Args) -> Value {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    Value::Null
}

#[tauri::command]
pub async fn quit_app<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, String> {
    // Persist window geometry while the window is still around. The
    // ExitRequested run-event hook also covers Cmd+Q / system
    // shutdown paths; this branch covers the renderer-driven Quit.
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        lifecycle::persist_window_geometry(&window, state.inner());
    }

    // Flip the flag so the close handler stops intercepting close
    // events as "hide", then ask Tauri to exit cleanly.
    state.is_will_quit.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(Value::Null)
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
//
// All three commands preserve the Electron-era return shape so the
// existing renderer error handling in TopBar/ConfigMenu and
// TopBar/ImportFromUrl keeps working without changes:
//
//   exportData()            -> null (cancelled) | false (failed) | string (path)
//   importData()            -> null (cancelled) | true (ok)       | string (error_code)
//   importDataFromUrl(url)  -> null (error?)    | true (ok)       | string (error_code or msg)
//
// Hard filesystem / Tauri errors bubble up as Err(String) so the
// invoke promise rejects; soft errors (parse failure, invalid shape)
// come back as Ok(Value::String("error_code")) the renderer can
// display.

#[tauri::command]
pub async fn export_data<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name("swh_data.json")
        .blocking_save_file();

    let Some(dest) = picked else {
        return Ok(Value::Null);
    };

    let dest_path = match dest.into_path() {
        Ok(p) => p,
        Err(e) => return Err(format!("invalid save path: {e}")),
    };

    let _guard = state.store_lock.lock().expect("store lock poisoned");
    if let Err(e) = import_export::export_to_file(&dest_path, &state.paths) {
        eprintln!("[v5 export] failed: {e}");
        return Ok(Value::Bool(false));
    }
    Ok(Value::String(dest_path.display().to_string()))
}

#[tauri::command]
pub async fn import_data<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    _args: Args,
) -> Result<Value, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .blocking_pick_file();

    let Some(src) = picked else {
        return Ok(Value::Null);
    };

    let src_path = match src.into_path() {
        Ok(p) => p,
        Err(e) => return Err(format!("invalid pick path: {e}")),
    };

    let bytes = match std::fs::read(&src_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[v5 import] read failed: {e}");
            return Ok(Value::String(import_export::ERR_PARSE.into()));
        }
    };

    let _guard = state.store_lock.lock().expect("store lock poisoned");
    match import_export::import_backup_bytes(&bytes, &state.paths) {
        Ok(result) => Ok(result),
        Err(e) => Err(format!("import failed: {e}")),
    }
}

#[tauri::command]
pub async fn import_data_from_url(
    state: State<'_, AppState>,
    args: Args,
) -> Result<Value, String> {
    let url = arg_str(&args, 0, "url").map_err(|e| format!("{e:?}"))?;

    let body = match fetch_url(url).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[v5 import-url] fetch failed: {e}");
            return Ok(Value::String(e));
        }
    };

    let _guard = state.store_lock.lock().expect("store lock poisoned");
    match import_export::import_backup_bytes(body.as_bytes(), &state.paths) {
        Ok(result) => Ok(result),
        Err(e) => Err(format!("import failed: {e}")),
    }
}

async fn fetch_url(url: &str) -> Result<String, String> {
    let response = reqwest::get(url).await.map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("error_{}", status.as_u16()));
    }
    response.text().await.map_err(|e| e.to_string())
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
