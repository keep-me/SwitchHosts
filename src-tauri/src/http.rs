//! Shared `reqwest::Client` builder that honours the user's proxy +
//! User-Agent settings.
//!
//! Used by both the remote-hosts refresh path (`refresh::refresh_one`)
//! and the URL-import path (`commands::import_data_from_url`). Keeping
//! the construction in one place means a future tweak — adding TLS
//! options, request signing, retries — only has to land here once,
//! and clears implementation-notes D8 ("`import_data_from_url` does
//! not honour proxy config").

use std::time::Duration;

use crate::storage::AppState;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!(
    "SwitchHosts/4.3.0 (Tauri; ",
    env!("CARGO_PKG_NAME"),
    ")"
);

/// Build a fresh `reqwest::Client` configured with:
///
/// - 30s connect+read timeout (matches the Electron `axios` default)
/// - SwitchHosts user agent string
/// - HTTP proxy from `use_proxy` / `proxy_protocol` / `proxy_host` /
///   `proxy_port` config when `use_proxy == true`
///
/// Returns a `String` error so commands can convert it to whatever
/// renderer-facing shape they need.
pub fn build_client(state: &AppState) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .user_agent(USER_AGENT);

    let proxy_settings = {
        let cfg = state.config.lock().expect("config mutex poisoned");
        if cfg.use_proxy
            && !cfg.proxy_host.trim().is_empty()
            && cfg.proxy_port > 0
        {
            Some((
                cfg.proxy_protocol.clone(),
                cfg.proxy_host.clone(),
                cfg.proxy_port,
            ))
        } else {
            None
        }
    };

    if let Some((protocol, host, port)) = proxy_settings {
        let proxy_url = format!("{protocol}://{host}:{port}");
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| format!("invalid proxy {proxy_url}: {e}"))?;
        builder = builder.proxy(proxy);
    }

    builder.build().map_err(|e| e.to_string())
}
