// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The same binary doubles as the system proxy watchdog (see engine/guard.rs).
    if project_proxy_lib::engine::guard::run_if_requested() {
        return;
    }
    project_proxy_lib::run()
}
