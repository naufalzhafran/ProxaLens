pub mod commands;
pub mod engine;
mod tray;

use std::sync::Arc;

use tauri::{Manager, RunEvent};

use commands::AppState;
use engine::sysproxy::RealNetworkSetup;
use engine::{Engine, EngineConfig, SysProxyController};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // hudsucker logs every refused handshake as an error; those are shown in
    // the flow list instead (see tlsfail.rs), so keep them out of the
    // terminal unless PROXALENS_DEBUG is set.
    use tracing_subscriber::prelude::*;
    let debug = std::env::var_os("PROXALENS_DEBUG").is_some();
    let (tls_failures, failures) = engine::tlsfail::layer();
    tracing_subscriber::registry()
        .with(tls_failures)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::filter::filter_fn(move |m| {
                    *m.level() <= tracing::Level::INFO
                        && (debug || !m.target().starts_with("hudsucker"))
                })),
        )
        .init();
    let _ = hudsucker::rustls::crypto::aws_lc_rs::default_provider().install_default();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            match engine::migrate_data_dir(&data_dir) {
                Ok(true) => tracing::info!(
                    "moved data from {} to {}",
                    engine::OLD_DATA_DIR,
                    data_dir.display()
                ),
                Ok(false) => {}
                Err(e) => tracing::warn!("could not move data from {}: {e}", engine::OLD_DATA_DIR),
            }
            let engine = Arc::new(Engine::new(EngineConfig::new(&data_dir))?);
            failures.connect(engine::tlsfail::recorder(engine.store.clone()));
            let marker = data_dir.join("sysproxy.marker.json");
            let sysproxy = Arc::new(SysProxyController::new(
                Box::new(RealNetworkSetup),
                marker.clone(),
            ));
            install_exit_hooks(marker);

            let state = AppState::new(engine, sysproxy.clone());
            // Layer 4: a previous run died with the system proxy still on.
            match sysproxy.recover_stale() {
                Ok(Some(report)) if !report.restored.is_empty() => {
                    *state.notice.lock() = Some(report)
                }
                Ok(_) => {}
                Err(e) => tracing::error!("could not restore previous proxy settings: {e}"),
            }
            app.manage(state);
            if let Err(e) = tray::install(app.handle()) {
                tracing::warn!("could not create the menu bar icon: {e}");
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if let Err(e) = commands::start(&state, state.engine.settings().port).await {
                    tracing::error!("proxy did not start: {e}");
                }
                commands::notify_status(&handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::start_proxy,
            commands::stop_proxy,
            commands::set_system_proxy,
            commands::install_ca,
            commands::take_restore_notice,
            commands::restart_app,
            commands::list_flows,
            commands::get_flow,
            commands::clear_flows,
            commands::remove_flows,
            commands::set_recording,
            commands::subscribe_flows,
            commands::get_settings,
            commands::device_info,
            commands::install_simulator_cert,
            commands::set_settings,
            commands::export_har,
            commands::save_session,
            commands::import_flows,
            commands::repeat_flow,
            commands::send_request,
            commands::get_rules,
            commands::set_rules,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        // Layer 2: normal quit, window close, logout/shutdown.
        if let RunEvent::Exit = event
            && let Err(e) = handle.state::<AppState>().sysproxy.disable()
        {
            tracing::error!("could not restore system proxy on exit: {e}");
        }
    });
}

/// Layer 2: restore on panics and termination signals as well.
fn install_exit_hooks(marker: std::path::PathBuf) {
    let panic_marker = marker.clone();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        engine::emergency_restore(&panic_marker);
        prev(info);
    }));

    tauri::async_runtime::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
            signal(SignalKind::hangup()),
        ) else {
            tracing::warn!("could not install signal handlers");
            return;
        };
        tokio::select! {
            _ = term.recv() => {},
            _ = int.recv() => {},
            _ = hup.recv() => {},
        }
        let m = marker.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || engine::emergency_restore(&m)).await;
        std::process::exit(0);
    });
}
