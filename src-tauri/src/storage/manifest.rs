//! `~/.SwitchHosts/manifest.json` reader, writer, and tree operations.
//!
//! Phase 1B step 2 uses the renderer-facing `IHostsListObject` shape
//! verbatim as the on-disk node shape. Each node is persisted as a raw
//! `serde_json::Value` under the `root` array so we don't have to
//! maintain a parallel Rust type hierarchy while the renderer still
//! drives the contract. A later sub-step will migrate the field names
//! to the camelCase + nested shape from the storage plan.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::atomic::atomic_write;
use super::error::StorageError;

pub const MANIFEST_FORMAT: &str = "switchhosts-data";
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default = "default_format")]
    #[allow(dead_code)]
    pub format: String,
    #[serde(default = "default_schema_version", rename = "schemaVersion")]
    #[allow(dead_code)]
    pub schema_version: u32,
    #[serde(default)]
    pub root: Vec<Value>,
}

fn default_format() -> String {
    MANIFEST_FORMAT.to_string()
}

fn default_schema_version() -> u32 {
    MANIFEST_SCHEMA_VERSION
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            format: default_format(),
            schema_version: default_schema_version(),
            root: Vec::new(),
        }
    }
}

impl Manifest {
    /// Read `manifest.json`.
    ///
    /// - Missing file → empty in-memory manifest, no write. Phase 1B
    ///   starts every user off with an empty tree until the PotDb
    ///   migration step runs.
    /// - Unreadable file → `StorageError::Io`
    /// - Unparsable file → `StorageError::Parse` (left on disk for the
    ///   user to inspect; the in-memory fallback is *not* persisted).
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = std::fs::read(path).map_err(|e| {
            StorageError::io(path.display().to_string(), e)
        })?;
        serde_json::from_slice::<Manifest>(&bytes).map_err(|e| {
            StorageError::parse(path.display().to_string(), e)
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let mut value = json!({
            "format": MANIFEST_FORMAT,
            "schemaVersion": MANIFEST_SCHEMA_VERSION,
            "root": self.root.clone(),
        });
        // Ensure the top-level object has stable key ordering for human
        // readability: format, schemaVersion, root.
        let obj = value.as_object_mut().expect("manifest value is object");
        let json = serde_json::to_vec_pretty(obj).map_err(|e| {
            StorageError::serialize(path.display().to_string(), e)
        })?;
        atomic_write(path, &json)
    }
}

// ---- tree operations -------------------------------------------------------
//
// All operations work against a `Vec<Value>` slice of the root forest.
// Nodes may have a `children: Vec<Value>` field when they are folders;
// these helpers walk into children recursively.

/// Find a node anywhere in the tree by id, returning a cloned copy.
pub fn find_node(nodes: &[Value], id: &str) -> Option<Value> {
    for node in nodes {
        if node_id(node) == Some(id) {
            return Some(node.clone());
        }
        if let Some(children) = node_children(node) {
            if let Some(found) = find_node(children, id) {
                return Some(found);
            }
        }
    }
    None
}

/// Remove a node by id. Returns the removed node plus the id of its
/// parent folder (`None` if it lived at the top level).
pub fn remove_node(nodes: &mut Vec<Value>, id: &str) -> Option<(Value, Option<String>)> {
    remove_node_inner(nodes, id, None)
}

fn remove_node_inner(
    nodes: &mut Vec<Value>,
    id: &str,
    parent_id: Option<&str>,
) -> Option<(Value, Option<String>)> {
    if let Some(pos) = nodes.iter().position(|n| node_id(n) == Some(id)) {
        let removed = nodes.remove(pos);
        return Some((removed, parent_id.map(String::from)));
    }
    for node in nodes.iter_mut() {
        let this_id = node_id(node).map(String::from);
        if let Some(children) = node_children_mut(node) {
            if let Some(result) = remove_node_inner(children, id, this_id.as_deref()) {
                return Some(result);
            }
        }
    }
    None
}

/// Insert `node` at the top level or inside the folder with `parent_id`.
/// If `parent_id` is supplied but no matching folder exists, the node
/// is appended to the top level.
pub fn insert_node(nodes: &mut Vec<Value>, node: Value, parent_id: Option<&str>) {
    if let Some(pid) = parent_id {
        if append_into_folder(nodes, &node, pid) {
            return;
        }
    }
    nodes.push(node);
}

fn append_into_folder(nodes: &mut Vec<Value>, node: &Value, parent_id: &str) -> bool {
    for current in nodes.iter_mut() {
        if node_id(current) == Some(parent_id) {
            if let Some(children) = node_children_mut(current) {
                children.push(node.clone());
                return true;
            }
            // Parent matched but isn't a folder — fall back to top
            // level by returning false from the enclosing call.
            return false;
        }
        if let Some(children) = node_children_mut(current) {
            if append_into_folder(children, node, parent_id) {
                return true;
            }
        }
    }
    false
}

fn node_id(node: &Value) -> Option<&str> {
    node.get("id").and_then(Value::as_str)
}

fn node_children(node: &Value) -> Option<&Vec<Value>> {
    node.get("children").and_then(Value::as_array)
}

fn node_children_mut(node: &mut Value) -> Option<&mut Vec<Value>> {
    node.get_mut("children").and_then(Value::as_array_mut)
}

/// Walk the tree and collect the ids of every `local`/`remote` node
/// reachable from the root. Used to garbage-collect orphaned
/// `entries/<id>.hosts` files during Phase 2 but harmless to expose
/// now.
#[allow(dead_code)]
pub fn collect_content_ids(nodes: &[Value], out: &mut Vec<String>) {
    for node in nodes {
        let kind = node.get("type").and_then(Value::as_str);
        if matches!(kind, Some("local") | Some("remote")) {
            if let Some(id) = node_id(node) {
                out.push(id.to_string());
            }
        }
        if let Some(children) = node_children(node) {
            collect_content_ids(children, out);
        }
    }
}
