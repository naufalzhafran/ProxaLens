//! Menu bar (tray) icon: proxy status, a system-proxy toggle, and quit.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::commands::{self, AppState};

const SYSTEM_PROXY: &str = "system-proxy";
const SHOW: &str = "show";
const CLEAR: &str = "clear";
const QUIT: &str = "quit";

/// Menu items whose text or check state follows the engine.
pub struct TrayItems {
    status: MenuItem<Wry>,
    system_proxy: CheckMenuItem<Wry>,
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Proxy starting…", false, None::<&str>)?;
    let system_proxy = CheckMenuItem::with_id(
        app,
        SYSTEM_PROXY,
        "Route System Traffic Through Proxy",
        false,
        false,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &system_proxy,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, SHOW, "Show Project Proxy", true, None::<&str>)?,
            &MenuItem::with_id(app, CLEAR, "Clear Flows", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "Quit Project Proxy", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("Project Proxy")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SYSTEM_PROXY => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    let enable = !state.sysproxy.is_enabled();
                    if let Err(e) = commands::apply_system_proxy(&state, enable).await {
                        tracing::error!("tray: could not switch the system proxy: {e}");
                        *state.last_error.lock() = Some(e);
                    }
                    commands::notify_status(&app).await;
                });
            }
            SHOW => show_main_window(app),
            CLEAR => app.state::<AppState>().engine.store.clear(),
            QUIT => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    app.manage(TrayItems {
        status,
        system_proxy,
    });
    Ok(())
}

/// Brings the tray menu in line with the engine.
pub fn refresh(app: &AppHandle, addr: Option<std::net::SocketAddr>, system_proxy: bool) {
    let Some(items) = app.try_state::<TrayItems>() else {
        return;
    };
    let text = match addr {
        Some(a) => format!("Proxy running on {a}"),
        None => "Proxy stopped".to_string(),
    };
    let _ = items.status.set_text(text);
    let _ = items.system_proxy.set_checked(system_proxy);
    let _ = items.system_proxy.set_enabled(addr.is_some() || system_proxy);
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
