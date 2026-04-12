//! Find / replace window plumbing.
//!
//! Phase 2.D scope:
//!
//! - Lazy-create the `find` webview (route `#/find`) on first
//!   `find_show` invocation. Close button hides instead of destroying,
//!   matching Electron's `e.preventDefault(); win.hide()` behaviour
//!   so the search state and history persist for the next reopen.
//! - Search content of every local/remote node in the manifest. Group
//!   and folder nodes are skipped (Electron does the same — they have
//!   no own content, only references).
//! - Persist find / replace history to
//!   `internal/histories/find.json` and `internal/histories/replace.json`,
//!   capped at 20 entries (matches Electron's `MAX_LENGTH = 20`).
//!
//! On-disk shape mirrors the renderer's `IFindHistoryData` /
//! `string[]` types one-for-one so the existing `pages/find.tsx`
//! consumers work without changes.

use std::path::{Path, PathBuf};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WindowEvent};

use crate::storage::{
    atomic::atomic_write, entries, error::StorageError, manifest::Manifest, AppState,
};

pub const FIND_WINDOW_LABEL: &str = "find";

const FIND_WINDOW_WIDTH: f64 = 480.0;
const FIND_WINDOW_HEIGHT: f64 = 400.0;
const FIND_WINDOW_MIN_WIDTH: f64 = 400.0;
const FIND_WINDOW_MIN_HEIGHT: f64 = 400.0;

const FIND_HISTORY_FILE: &str = "find.json";
const REPLACE_HISTORY_FILE: &str = "replace.json";
const HISTORY_MAX: usize = 20;

// ---- search options + result types ----------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FindOptions {
    #[serde(default)]
    pub is_regexp: bool,
    #[serde(default)]
    pub is_ignore_case: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindPosition {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub line_pos: usize,
    pub end_line: usize,
    pub end_line_pos: usize,
    pub before: String,
    #[serde(rename = "match")]
    pub match_text: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindSplitter {
    pub before: String,
    #[serde(rename = "match")]
    pub match_text: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindItem {
    pub item_id: String,
    pub item_title: String,
    pub item_type: String,
    pub positions: Vec<FindPosition>,
    pub splitters: Vec<FindSplitter>,
}

// ---- search engine --------------------------------------------------------

/// Run a find pass against every local/remote node in the manifest
/// and return one `FindItem` per node that matched.
pub fn find_in_manifest(
    state: &AppState,
    keyword: &str,
    options: &FindOptions,
) -> Result<Vec<FindItem>, String> {
    if keyword.is_empty() {
        return Ok(Vec::new());
    }
    let regex = build_regex(keyword, options)?;

    let manifest = Manifest::load(&state.paths).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    walk_searchable(&manifest.root, &mut |id, title, kind| {
        let content = match entries::read_entry(&state.paths.entries_dir, id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[v5 find] read {id}: {e}");
                return;
            }
        };
        let positions = find_positions_in_content(&content, &regex);
        if positions.is_empty() {
            return;
        }
        let splitters = split_content(&content, &positions);
        out.push(FindItem {
            item_id: id.to_string(),
            item_title: title.to_string(),
            item_type: kind.to_string(),
            positions,
            splitters,
        });
    });
    Ok(out)
}

fn build_regex(keyword: &str, options: &FindOptions) -> Result<Regex, String> {
    let pattern = if options.is_regexp {
        keyword.to_string()
    } else {
        // Escape regex metacharacters so the user gets a literal
        // string search by default. Mirrors the
        // `keyword.replace(/([.^$([?*+])/gi, '\\$1')` pass in
        // `src/main/actions/find/findBy.ts` — `regex::escape` is
        // a strict superset (also escapes `]`, `}`, `|`, `\\`, etc.)
        // so any string the user types becomes a valid literal.
        regex::escape(keyword)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(options.is_ignore_case)
        .build()
        .map_err(|e| format!("invalid pattern: {e}"))
}

/// Mirror of `src/main/actions/find/findPositionsInContent.ts`. For
/// each match in `content` we record byte offsets, line numbers, and
/// the surrounding line slices the renderer needs to render the
/// result list and jump back to the source view.
fn find_positions_in_content(content: &str, regex: &Regex) -> Vec<FindPosition> {
    let mut out = Vec::new();
    for mat in regex.find_iter(content) {
        let start = mat.start();
        let end = mat.end();
        let match_text = mat.as_str().to_string();

        // line + line_pos at the match start
        let prefix = &content[..start];
        let line = prefix.matches('\n').count() + 1;
        let last_nl_before = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_pos = start - last_nl_before;
        let before = content[last_nl_before..start].to_string();

        // line + line_pos at the match end (handle multi-line matches)
        let match_lines = match_text.split('\n').count();
        let end_line = line + match_lines - 1;
        let end_line_pos = if match_lines > 1 {
            match_text.rsplit('\n').next().unwrap_or("").len()
        } else {
            line_pos + match_text.len()
        };

        // `after`: rest of the line that contains the match end
        let after_start = end;
        let next_nl = content[after_start..]
            .find('\n')
            .map(|i| after_start + i)
            .unwrap_or(content.len());
        let after = content[after_start..next_nl].to_string();

        out.push(FindPosition {
            start,
            end,
            line,
            line_pos,
            end_line,
            end_line_pos,
            before,
            match_text,
            after,
        });
    }
    out
}

/// Mirror of `src/main/actions/find/splitContent.ts`. Slices the
/// content into `[before-of-match-1] [match-1] [before-of-match-2]
/// [match-2] ... [last-after]` so the renderer can render the
/// result with the matched substrings highlighted.
fn split_content(content: &str, positions: &[FindPosition]) -> Vec<FindSplitter> {
    let mut splitters = Vec::with_capacity(positions.len());
    let mut last_end = 0;
    for (idx, pos) in positions.iter().enumerate() {
        let before = content[last_end..pos.start].to_string();
        last_end = pos.start + pos.match_text.len();
        let after = if idx == positions.len() - 1 {
            content[last_end..].to_string()
        } else {
            String::new()
        };
        splitters.push(FindSplitter {
            before,
            match_text: pos.match_text.clone(),
            after,
        });
    }
    splitters
}

fn walk_searchable<F: FnMut(&str, &str, &str)>(nodes: &[Value], visit: &mut F) {
    for node in nodes {
        let kind = node
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("local");
        if kind != "group" && kind != "folder" {
            if let Some(id) = node.get("id").and_then(Value::as_str) {
                let title = node
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                visit(id, title, kind);
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            walk_searchable(children, visit);
        }
    }
}

// ---- find / replace history persistence -----------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindHistoryEntry {
    pub value: String,
    #[serde(default)]
    pub is_regexp: bool,
    #[serde(default)]
    pub is_ignore_case: bool,
}

fn find_history_path(state: &AppState) -> PathBuf {
    state.paths.histories_dir.join(FIND_HISTORY_FILE)
}

fn replace_history_path(state: &AppState) -> PathBuf {
    state.paths.histories_dir.join(REPLACE_HISTORY_FILE)
}

pub fn get_find_history(state: &AppState) -> Result<Vec<FindHistoryEntry>, StorageError> {
    load_json_array(&find_history_path(state))
}

pub fn set_find_history(
    state: &AppState,
    items: &[FindHistoryEntry],
) -> Result<(), StorageError> {
    save_json_array(&find_history_path(state), items)
}

pub fn add_find_history(
    state: &AppState,
    entry: FindHistoryEntry,
) -> Result<Vec<FindHistoryEntry>, StorageError> {
    let mut all = get_find_history(state).unwrap_or_default();
    all.retain(|i| i.value != entry.value);
    all.push(entry);
    while all.len() > HISTORY_MAX {
        all.remove(0);
    }
    set_find_history(state, &all)?;
    Ok(all)
}

pub fn get_replace_history(state: &AppState) -> Result<Vec<String>, StorageError> {
    load_json_array(&replace_history_path(state))
}

pub fn set_replace_history(state: &AppState, items: &[String]) -> Result<(), StorageError> {
    save_json_array(&replace_history_path(state), items)
}

pub fn add_replace_history(
    state: &AppState,
    value: String,
) -> Result<Vec<String>, StorageError> {
    let mut all = get_replace_history(state).unwrap_or_default();
    all.retain(|v| v != &value);
    all.push(value);
    while all.len() > HISTORY_MAX {
        all.remove(0);
    }
    set_replace_history(state, &all)?;
    Ok(all)
}

fn load_json_array<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, StorageError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path)
        .map_err(|e| StorageError::io(path.display().to_string(), e))?;
    serde_json::from_slice::<Vec<T>>(&bytes).or_else(|_| {
        // Tolerate slight schema drift: anything that doesn't decode
        // as the expected list shape resets to empty rather than
        // crashing the find window.
        eprintln!(
            "[v5 find-history] {} could not be parsed; treating as empty.",
            path.display()
        );
        Ok(Vec::new())
    })
}

fn save_json_array<T: Serialize>(path: &Path, items: &[T]) -> Result<(), StorageError> {
    let bytes = serde_json::to_vec_pretty(items)
        .map_err(|e| StorageError::serialize(path.display().to_string(), e))?;
    atomic_write(path, &bytes)
}

// ---- window create + show -------------------------------------------------

/// Bring the find webview to the front, lazy-creating it the first
/// time. Subsequent calls reuse the existing webview so the in-window
/// search state survives close-and-reopen cycles.
pub fn show_find_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), tauri::Error> {
    let window = match app.get_webview_window(FIND_WINDOW_LABEL) {
        Some(w) => w,
        None => create_find_window(app)?,
    };
    window.unminimize().ok();
    window.show()?;
    window.set_focus()?;
    Ok(())
}

fn create_find_window<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<tauri::WebviewWindow<R>, tauri::Error> {
    let url = WebviewUrl::App("#/find".into());
    let window = WebviewWindowBuilder::new(app, FIND_WINDOW_LABEL, url)
        .title("Find")
        .inner_size(FIND_WINDOW_WIDTH, FIND_WINDOW_HEIGHT)
        .min_inner_size(FIND_WINDOW_MIN_WIDTH, FIND_WINDOW_MIN_HEIGHT)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .visible(false)
        .build()?;

    let window_for_handler = window.clone();
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        // The Electron find window intercepted Close to hide instead
        // of destroy: search history is in-process, recreating the
        // window means losing any in-progress query and result list.
        // We mirror that here by preventing default and hiding,
        // unless `is_will_quit` is set (real quit path).
        if let WindowEvent::CloseRequested { api, .. } = event {
            let app_state = app_handle.state::<AppState>();
            if app_state
                .is_will_quit
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return;
            }
            api.prevent_close();
            let _ = window_for_handler.hide();
            let _ = app_handle.emit("close_find", serde_json::json!({ "_args": [] }));
        }
    });

    Ok(window)
}
