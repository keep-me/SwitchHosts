//! Manual import / export of v3, v4, and v5 backup JSON files.
//!
//! Two independent code paths per the v5 migration plan:
//!
//! - **First-startup auto-migration** reads a live PotDb layout. Lives
//!   in `crate::migration` and is not touched from here.
//! - **Manual import** (this module) reads a user-supplied backup JSON,
//!   regardless of whether the original data directory still exists.
//!   Accepts v3 (`version[0] === 3`), v4 (`version[0] === 4`) and v5
//!   (`format === "switchhosts-backup"`).
//!
//! The renderer contract for importData / importDataFromUrl / exportData
//! is preserved exactly: commands return `Value::Bool(true)` on success,
//! `Value::Null` on user cancel, and `Value::String(error_code)` on soft
//! failures. Hard failures (filesystem errors) bubble up as Err so the
//! invoke promise rejects.

use std::path::Path;

use serde_json::{json, Value};

use crate::storage::{
    atomic::atomic_write,
    entries,
    manifest::{self, Manifest},
    trashcan::Trashcan,
    StorageError, V5Paths,
};

// ---- error codes (string values returned to the renderer) ------------------

pub const ERR_PARSE: &str = "parse_error";
pub const ERR_INVALID_DATA: &str = "invalid_data";
pub const ERR_NEW_VERSION: &str = "new_version";
pub const ERR_INVALID_DATA_KEY: &str = "invalid_data_key";
pub const ERR_INVALID_V3_DATA: &str = "invalid_v3_data";

/// Outcome of a backup import. `Ok(Value)` mirrors the renderer-facing
/// return shape of `actions.importData()` / `actions.importDataFromUrl()`:
///
/// - `Value::Bool(true)` — success
/// - `Value::String(error_code)` — soft error the renderer displays
/// - (cancellation is handled in the command shell, not here)
pub fn import_backup_bytes(bytes: &[u8], paths: &V5Paths) -> Result<Value, StorageError> {
    let data: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json!(ERR_PARSE)),
    };

    if !data.is_object() {
        return Ok(json!(ERR_INVALID_DATA));
    }

    // v5 backup is distinguished by the `format` discriminator, not by
    // a `version` array, so check it first.
    if data.get("format").and_then(Value::as_str) == Some("switchhosts-backup") {
        return import_v5(&data, paths);
    }

    let version = data.get("version").and_then(Value::as_array);
    let Some(version) = version else {
        return Ok(json!(ERR_INVALID_DATA));
    };
    let major = version.first().and_then(Value::as_u64).unwrap_or(0);

    match major {
        3 => import_v3(&data, paths),
        4 => import_v4(&data, paths),
        n if n > 4 => Ok(json!(ERR_NEW_VERSION)),
        _ => Ok(json!(ERR_INVALID_DATA)),
    }
}

// ---- v3 import -------------------------------------------------------------
//
// v3 shape:
//   {
//     "version": [3, ...],
//     "list": [
//       { id, title, where: "local"|"remote"|"group"|"folder",
//         content?, on?, url?, refresh_interval?, include?, children?, ... }
//     ]
//   }
//
// We walk the tree recursively: at each local/remote node we extract
// `content` into `entries/<id>.hosts`, rename `where` → `type`, and
// convert `refresh_interval` from hours to seconds. Folder nodes
// recurse into `children`. System node id "0" keeps no content file.

fn import_v3(data: &Value, paths: &V5Paths) -> Result<Value, StorageError> {
    let Some(list) = data.get("list").and_then(Value::as_array) else {
        return Ok(json!(ERR_INVALID_V3_DATA));
    };

    let mut converted = Vec::with_capacity(list.len());
    for node in list {
        converted.push(convert_v3_node(node, paths)?);
    }

    let manifest = Manifest {
        root: converted,
        ..Default::default()
    };
    manifest.save(&paths.manifest_file)?;

    // v3 backups had no trashcan — reset to empty so the user isn't
    // looking at stale entries from a previous import.
    Trashcan::default().save(&paths.trashcan_file)?;

    Ok(json!(true))
}

fn convert_v3_node(node: &Value, paths: &V5Paths) -> Result<Value, StorageError> {
    let Some(obj) = node.as_object() else {
        return Ok(node.clone());
    };
    let mut out = serde_json::Map::with_capacity(obj.len());

    for (key, value) in obj {
        match key.as_str() {
            "where" => {
                // Skip here, re-insert under `type` below.
                continue;
            }
            "content" => {
                // Don't carry inline content into v5 tree — extract it
                // to entries/<id>.hosts instead.
                continue;
            }
            "refresh_interval" => {
                let hours = value.as_u64().unwrap_or(0);
                out.insert("refresh_interval".into(), json!(hours * 3600));
            }
            "children" => {
                if let Some(children) = value.as_array() {
                    let mut new_children = Vec::with_capacity(children.len());
                    for child in children {
                        new_children.push(convert_v3_node(child, paths)?);
                    }
                    out.insert("children".into(), Value::Array(new_children));
                } else {
                    out.insert("children".into(), value.clone());
                }
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }

    // Promote `where` → `type` once, using the original object.
    if let Some(where_val) = obj.get("where") {
        out.insert("type".into(), where_val.clone());
    }

    // If the original node had inline content, write it to entries.
    if let Some(id) = obj.get("id").and_then(Value::as_str) {
        if id != "0" {
            if let Some(content) = obj.get("content").and_then(Value::as_str) {
                entries::write_entry(&paths.entries_dir, id, content)?;
            }
        }
    }

    Ok(Value::Object(out))
}

// ---- v4 import -------------------------------------------------------------
//
// v4 shape produced by the Electron export flow is PotDb's toJSON():
//   {
//     "version": [4, ...],
//     "data": {
//       "dict": { "meta": {...} },
//       "list": { "tree": [...], "trashcan": [...] },
//       "set": {},
//       "collection": {
//         "hosts":   { "data": [{id, content, _id}, ...], "meta": {...} },
//         "history": { "data": [...], "meta": {...} }
//       }
//     }
//   }

fn import_v4(data: &Value, paths: &V5Paths) -> Result<Value, StorageError> {
    let Some(inner) = data.get("data").filter(|v| v.is_object()) else {
        return Ok(json!(ERR_INVALID_DATA_KEY));
    };

    let tree = inner
        .get("list")
        .and_then(|l| l.get("tree"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let trashcan_items = inner
        .get("list")
        .and_then(|l| l.get("trashcan"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let hosts_data = inner
        .get("collection")
        .and_then(|c| c.get("hosts"))
        .and_then(|h| h.get("data"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let history_data = inner
        .get("collection")
        .and_then(|c| c.get("history"))
        .and_then(|h| h.get("data"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Write entries first — if this fails, manifest hasn't been
    // overwritten yet, so the user still sees their pre-import state.
    for entry in &hosts_data {
        let id = entry.get("id").and_then(Value::as_str);
        let content = entry.get("content").and_then(Value::as_str);
        if let (Some(id), Some(content)) = (id, content) {
            if id == "0" {
                continue;
            }
            entries::write_entry(&paths.entries_dir, id, content)?;
        }
    }

    // Then trashcan, then manifest (commit marker).
    Trashcan {
        items: trashcan_items,
        ..Default::default()
    }
    .save(&paths.trashcan_file)?;

    Manifest {
        root: tree,
        ..Default::default()
    }
    .save(&paths.manifest_file)?;

    if !history_data.is_empty() {
        write_history(&paths.histories_dir.join("system-hosts.json"), &history_data)?;
    }

    Ok(json!(true))
}

// ---- v5 import -------------------------------------------------------------
//
// v5 backup shape produced by `export_to_file`:
//   {
//     "format": "switchhosts-backup",
//     "schemaVersion": 1,
//     "version": [5, 0, 0, 0],
//     "exportedAt": "2026-04-11T...",
//     "manifest": { "format": "switchhosts-data", "schemaVersion": 1, "root": [...] },
//     "entries": { "<node-id>": "<content>", ... },
//     "trashcan": { "format": "switchhosts-trashcan", "schemaVersion": 1, "items": [...] }
//   }

fn import_v5(data: &Value, paths: &V5Paths) -> Result<Value, StorageError> {
    let manifest_value = data.get("manifest");
    let entries_value = data.get("entries");
    let trashcan_value = data.get("trashcan");

    let Some(manifest_obj) = manifest_value.filter(|v| v.is_object()) else {
        return Ok(json!(ERR_INVALID_DATA));
    };

    let root = manifest_obj
        .get("root")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Entries written first (same ordering rationale as v4 / PotDb
    // migration: manifest.json is the commit marker).
    if let Some(entries_obj) = entries_value.and_then(Value::as_object) {
        for (id, content_val) in entries_obj {
            if id == "0" {
                continue;
            }
            let content = content_val.as_str().unwrap_or("");
            entries::write_entry(&paths.entries_dir, id, content)?;
        }
    }

    let trashcan_items = trashcan_value
        .and_then(|t| t.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Trashcan {
        items: trashcan_items,
        ..Default::default()
    }
    .save(&paths.trashcan_file)?;

    Manifest {
        root,
        ..Default::default()
    }
    .save(&paths.manifest_file)?;

    Ok(json!(true))
}

// ---- export ----------------------------------------------------------------

/// Serialize the current v5 state into a backup JSON and write it to
/// `dest`. Returns `Ok(())` on success. Hard I/O errors bubble up.
pub fn export_to_file(dest: &Path, paths: &V5Paths) -> Result<(), StorageError> {
    let manifest = Manifest::load(&paths.manifest_file).unwrap_or_default();
    let trashcan = Trashcan::load(&paths.trashcan_file).unwrap_or_default();

    // Walk the tree and collect every local/remote node id that owns a
    // content file. We read each file and embed it inline in the backup
    // JSON under `entries`, keyed by node id.
    let mut ids = Vec::new();
    manifest::collect_content_ids(&manifest.root, &mut ids);

    let mut entries_map = serde_json::Map::with_capacity(ids.len());
    for id in ids {
        if id == "0" {
            continue;
        }
        let content = entries::read_entry(&paths.entries_dir, &id)?;
        entries_map.insert(id, Value::String(content));
    }

    let backup = json!({
        "format": "switchhosts-backup",
        "schemaVersion": 1,
        // Legacy Electron import reads `version[0]` — flagging this as
        // v5 lets old clients fail with "new_version" rather than a
        // "parse_error" / "invalid_data" mystery.
        "version": [5, 0, 0, 0],
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "manifest": {
            "format": "switchhosts-data",
            "schemaVersion": 1,
            "root": manifest.root,
        },
        "entries": Value::Object(entries_map),
        "trashcan": {
            "format": "switchhosts-trashcan",
            "schemaVersion": 1,
            "items": trashcan.items,
        },
    });

    let bytes = serde_json::to_vec_pretty(&backup).map_err(|e| {
        StorageError::serialize(dest.display().to_string(), e)
    })?;
    atomic_write(dest, &bytes)
}

// ---- helpers ---------------------------------------------------------------

fn write_history(path: &Path, items: &[Value]) -> Result<(), StorageError> {
    let payload = serde_json::to_vec_pretty(items)
        .map_err(|e| StorageError::serialize(path.display().to_string(), e))?;
    atomic_write(path, &payload)
}
