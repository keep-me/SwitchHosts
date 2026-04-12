//! Application menu (the macOS menu bar / Windows-Linux window menu).
//!
//! Phase 2.D ships a deliberately *minimal* version: take Tauri's
//! built-in default menu (which provides the standard system items
//! like Cut/Copy/Paste, Hide, Quit, etc.) and append a single
//! `Edit > Find` item bound to `Cmd+F` so the find window has a
//! discoverable entry point.
//!
//! Phase 2.C will replace this with a richer menu that mirrors the
//! Electron build's full structure (New Hosts, Preferences, View
//! menu, etc.) and add accelerators for the rest of the renderer
//! actions. Until then this file is intentionally tiny — its only
//! job is to put Find on the menu bar.

use tauri::menu::{Menu, MenuId, MenuItemBuilder};
use tauri::{AppHandle, Runtime};

/// Stable id we route through the global `on_menu_event` handler in
/// `lib.rs`.
pub const MENU_ID_FIND: &str = "app-find";

/// Build the application menu. Currently the default menu plus a
/// `Find...` item appended to the end of the `Edit` submenu.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), tauri::Error> {
    let menu = Menu::default(app)?;
    append_find_to_edit_submenu(app, &menu)?;
    app.set_menu(menu)?;
    Ok(())
}

fn append_find_to_edit_submenu<R: Runtime>(
    app: &AppHandle<R>,
    menu: &Menu<R>,
) -> Result<(), tauri::Error> {
    let find_item = MenuItemBuilder::with_id(MenuId::new(MENU_ID_FIND), "Find\u{2026}")
        .accelerator("CmdOrCtrl+F")
        .build(app)?;

    for item in menu.items()? {
        let Some(submenu) = item.as_submenu() else {
            continue;
        };
        let text = submenu.text().unwrap_or_default();
        // Tauri's `Menu::default` localises submenu titles based on
        // the OS, but the title for the standard Edit submenu is
        // always the literal "Edit" in en-US — and we currently ship
        // the app in English regardless. If a future i18n pass moves
        // the menu titles, this lookup will need to grow.
        if text == "Edit" {
            submenu.append(&find_item)?;
            return Ok(());
        }
    }
    // Default menu didn't include an Edit submenu (unlikely on
    // desktop, but possible on platforms we don't ship for). Fall
    // back to appending Find at the top level so the accelerator
    // still resolves.
    menu.append(&find_item)?;
    Ok(())
}
