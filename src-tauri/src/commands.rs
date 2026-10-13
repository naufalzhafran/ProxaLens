//! `#[tauri::command]` API surface used by the React UI.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast::error::TryRecvError;

use crate::engine::breakpoint::{Edits, Phase, Resume};
use crate::engine::repeat::OutgoingRequest;
use crate::engine::rules::Rules;
use crate::engine::script::{LogEntry, ScriptInfo};
use crate::engine::settings::Settings;
use crate::engine::store::{FlowEvent, FlowId, FlowMeta, WsDirection};
use crate::engine::sysproxy::RestoreReport;
use crate::engine::{DEFAULT_PORT, Engine, LOCALHOST, SysProxyController};
use crate::engine::{decode, har};

pub struct AppState {
    pub engine: Arc<Engine>,
    pub mcp: Arc<crate::mcp::McpServer>,
    pub sysproxy: Arc<SysProxyController>,
    pub port: Mutex<u16>,
    pub last_error: Mutex<Option<String>>,
    pub notice: Mutex<Option<RestoreReport>>,
    pub bridge: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl AppState {
    pub fn new(
        engine: Arc<Engine>,
        sysproxy: Arc<SysProxyController>,
        mcp: Arc<crate::mcp::McpServer>,
    ) -> Self {
        Self {
            engine,
            mcp,
            sysproxy,
            port: Mutex::new(DEFAULT_PORT),
            last_error: Mutex::new(None),
            notice: Mutex::new(None),
            bridge: Mutex::new(None),
        }
    }
}

type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    running: bool,
    addr: Option<String>,
    port: u16,
    error: Option<String>,
    ca_trusted: bool,
    ca_cert_path: String,
    ca_fingerprint: String,
    system_proxy_enabled: bool,
    data_dir: String,
    /// False while recording is paused.
    recording: bool,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> CmdResult<Status> {
    Ok(status(&state).await)
}

async fn status(state: &AppState) -> Status {
    let engine = state.engine.clone();
    let addr = engine.addr().await;
    let ca_trusted = tauri::async_runtime::spawn_blocking(move || engine.ca.is_trusted())
        .await
        .unwrap_or(false);
    Status {
        running: addr.is_some(),
        addr: addr.map(|a| a.to_string()),
        port: *state.port.lock(),
        error: state.last_error.lock().clone(),
        ca_trusted,
        ca_cert_path: state.engine.ca.cert_der_path.to_string_lossy().into_owned(),
        ca_fingerprint: state.engine.ca.sha1_fingerprint(),
        system_proxy_enabled: state.sysproxy.is_enabled(),
        data_dir: state.engine.data_dir().to_string_lossy().into_owned(),
        recording: state.engine.store.is_recording(),
    }
}

/// Starts the proxy; used at launch and by the UI. Errors are kept in
/// `last_error` so the UI can show them.
pub async fn start(state: &AppState, port: u16) -> CmdResult<()> {
    *state.port.lock() = port;
    match state.engine.start(port).await {
        Ok(_) => {
            *state.last_error.lock() = None;
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            *state.last_error.lock() = Some(msg.clone());
            Err(msg)
        }
    }
}

/// Tells the tray and the UI that the proxy or system proxy state changed.
pub async fn notify_status(app: &AppHandle) {
    let state = app.state::<AppState>();
    let addr = state.engine.addr().await;
    crate::tray::refresh(app, addr, state.sysproxy.is_enabled());
    let _ = app.emit("status-changed", ());
}

#[tauri::command]
pub async fn start_proxy(
    app: AppHandle,
    state: State<'_, AppState>,
    port: Option<u16>,
) -> CmdResult<Status> {
    let port = port.unwrap_or_else(|| *state.port.lock());
    if state.engine.addr().await.is_some_and(|a| a.port() != port) {
        stop(&state).await?;
    }
    let result = start(&state, port).await;
    notify_status(&app).await;
    result?;
    Ok(status(&state).await)
}

async fn stop(state: &AppState) -> CmdResult<()> {
    // Never leave the system pointing at a proxy that is gone.
    let sp = state.sysproxy.clone();
    tauri::async_runtime::spawn_blocking(move || sp.disable())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    state.engine.stop().await;
    Ok(())
}

#[tauri::command]
pub async fn stop_proxy(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Status> {
    let result = stop(&state).await;
    notify_status(&app).await;
    result?;
    Ok(status(&state).await)
}

#[tauri::command]
pub async fn set_system_proxy(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<Status> {
    let result = apply_system_proxy(&state, enabled).await;
    notify_status(&app).await;
    result?;
    Ok(status(&state).await)
}

/// Points every network service at the running proxy, or restores the
/// previous settings. Shared by the UI command and the tray menu.
pub async fn apply_system_proxy(state: &AppState, enabled: bool) -> CmdResult<()> {
    let sp = state.sysproxy.clone();
    if enabled {
        let Some(addr) = state.engine.addr().await else {
            return Err("Start the proxy before routing system traffic through it.".into());
        };
        let mode = state.engine.settings().system_proxy_mode;
        tauri::async_runtime::spawn_blocking(move || sp.enable(LOCALHOST, addr.port(), mode))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    } else {
        tauri::async_runtime::spawn_blocking(move || sp.disable())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn install_ca(state: State<'_, AppState>) -> CmdResult<Status> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.ca.install_trust())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(status(&state).await)
}

/// Returns (once) the report of a restore done at launch for a previous
/// session that did not shut down cleanly.
#[tauri::command]
pub fn take_restore_notice(state: State<'_, AppState>) -> Option<RestoreReport> {
    state.notice.lock().take()
}

#[derive(Serialize)]
pub struct FlowPage {
    flows: Vec<FlowMeta>,
    total: usize,
}

#[tauri::command]
pub fn list_flows(state: State<'_, AppState>, offset: usize, limit: usize) -> FlowPage {
    let (flows, total) = state.engine.store.list(offset, limit);
    FlowPage { flows, total }
}

#[tauri::command]
pub fn clear_flows(state: State<'_, AppState>) {
    state.engine.store.clear();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyView {
    size: usize,
    truncated: bool,
    /// Set when the (decoded) bytes are valid UTF-8.
    text: Option<String>,
    /// Set otherwise.
    base64: Option<String>,
    /// The Content-Encoding that was removed for display ("gzip").
    encoding: Option<String>,
    /// Size on the wire, before decoding.
    encoded_size: Option<usize>,
    /// Why the body could not be decoded (it is shown raw instead).
    decode_error: Option<String>,
    /// Protobuf / gRPC bodies decoded to JSON.
    protobuf: Option<crate::engine::proto::ProtoView>,
}

impl BodyView {
    fn new(bytes: Vec<u8>, truncated: bool) -> Self {
        let size = bytes.len();
        let (text, base64) = match String::from_utf8(bytes) {
            Ok(text) => (Some(text), None),
            Err(e) => (
                None,
                Some(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
            ),
        };
        Self {
            size,
            truncated,
            text,
            base64,
            encoding: None,
            encoded_size: None,
            decode_error: None,
            protobuf: None,
        }
    }

    /// Removes any Content-Encoding named in `headers`.
    fn decoded(
        bytes: Vec<u8>,
        truncated: bool,
        headers: &[(String, String)],
        already: bool,
    ) -> Self {
        let encoding = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
            .map(|(_, v)| v.as_str());
        if already {
            return Self::new(bytes, truncated);
        }
        match decode::decode(&bytes, encoding) {
            Ok(None) => Self::new(bytes, truncated),
            Ok(Some(d)) => {
                let encoded_size = bytes.len();
                Self {
                    encoding: Some(d.encoding),
                    encoded_size: Some(encoded_size),
                    ..Self::new(d.bytes, truncated || d.partial)
                }
            }
            Err(e) => Self {
                decode_error: Some(e.to_string()),
                ..Self::new(bytes, truncated)
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsMessageView {
    direction: WsDirection,
    at: f64,
    kind: &'static str,
    size: usize,
    payload: BodyView,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowDetail {
    meta: FlowMeta,
    request_headers: Vec<(String, String)>,
    request_body: BodyView,
    response_headers: Vec<(String, String)>,
    response_body: BodyView,
    ws_messages: Vec<WsMessageView>,
}

/// Pauses or resumes recording. The proxy keeps forwarding traffic (and
/// applying rules) while paused; it just doesn't keep it.
#[tauri::command]
pub async fn set_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<Status> {
    state.engine.store.set_recording(enabled);
    notify_status(&app).await;
    Ok(status(&state).await)
}

#[tauri::command]
pub fn remove_flows(state: State<'_, AppState>, ids: Vec<FlowId>) -> usize {
    state.engine.store.remove(&ids)
}

#[tauri::command]
pub fn get_flow(state: State<'_, AppState>, id: FlowId) -> Option<FlowDetail> {
    let (meta, d) = state.engine.store.get(id)?;
    let engine = &state.engine;
    let sources = engine.settings().protobuf;
    let proto = |headers: &[(String, String)], body: &BodyView, request: bool| {
        let ct = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.as_str())?;
        if !crate::engine::proto::is_protobuf(ct) || body.size == 0 {
            return None;
        }
        let bytes = match (&body.text, &body.base64) {
            (Some(t), _) => t.as_bytes().to_vec(),
            (_, Some(b)) => base64::engine::general_purpose::STANDARD.decode(b).ok()?,
            _ => return None,
        };
        crate::engine::proto::decode_body(
            ct,
            &bytes,
            &meta.host,
            &meta.path,
            request,
            &sources,
            &engine.proto,
        )
    };
    let request_body = BodyView::decoded(
        d.request_body,
        d.request_body_truncated,
        &d.request_headers,
        d.bodies_decoded,
    );
    let response_body = BodyView::decoded(
        d.response_body,
        d.response_body_truncated,
        &d.response_headers,
        d.bodies_decoded,
    );
    let mut request_body = request_body;
    let mut response_body = response_body;
    request_body.protobuf = proto(&d.request_headers, &request_body, true);
    response_body.protobuf = proto(&d.response_headers, &response_body, false);
    Some(FlowDetail {
        meta,
        request_headers: d.request_headers,
        request_body,
        response_headers: d.response_headers,
        response_body,
        ws_messages: d
            .ws_messages
            .into_iter()
            .map(|m| {
                let truncated = m.size > m.payload.len();
                WsMessageView {
                    direction: m.direction,
                    at: m.at,
                    kind: m.kind,
                    size: m.size,
                    payload: BodyView::new(m.payload, truncated),
                }
            })
            .collect(),
    })
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.engine.settings()
}

/// Saves settings. A new port or listening on the network takes a proxy
/// restart, and a new system proxy mode is applied straight away if it is on.
#[tauri::command]
pub async fn set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<Settings> {
    if settings.port == 0 {
        return Err("Choose a port from 1 to 65535.".into());
    }
    let before = state.engine.settings();
    state
        .engine
        .set_settings(settings.clone())
        .map_err(|e| e.to_string())?;
    let moved = before.port != settings.port;
    if (moved || before.allow_lan != settings.allow_lan) && state.engine.addr().await.is_some() {
        // The system proxy points at the old port; move it along.
        let routed = moved && state.sysproxy.is_enabled();
        if routed {
            apply_system_proxy(&state, false).await?;
        }
        state.engine.stop().await;
        start(&state, settings.port).await?;
        if routed {
            apply_system_proxy(&state, true).await?;
        }
    } else if moved {
        *state.port.lock() = settings.port;
    }
    if before.mcp != settings.mcp {
        sync_mcp(&state).await?;
    }
    if before.system_proxy_mode != settings.system_proxy_mode && state.sysproxy.is_enabled() {
        apply_system_proxy(&state, false).await?;
        apply_system_proxy(&state, true).await?;
    }
    notify_status(&app).await;
    Ok(state.engine.settings())
}

/// Starts, stops or moves the MCP server to match the settings.
pub async fn sync_mcp(state: &AppState) -> CmdResult<()> {
    let cfg = state.engine.settings().mcp;
    let running = state.mcp.addr().await;
    if running.is_some_and(|a| !cfg.enabled || a.port() != cfg.port) {
        state.mcp.stop().await;
    }
    if cfg.enabled {
        state.mcp.start(cfg.port).await?;
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    running: bool,
    url: Option<String>,
    token: String,
    /// Command that registers the server with Claude Code.
    claude_command: Option<String>,
}

#[tauri::command]
pub async fn mcp_info(state: State<'_, AppState>) -> CmdResult<McpInfo> {
    let addr = state.mcp.addr().await;
    let url = addr.map(|a| format!("http://{a}/mcp"));
    let token = state.mcp.token().to_string();
    Ok(McpInfo {
        running: addr.is_some(),
        claude_command: url.as_ref().map(|u| {
            format!("claude mcp add --transport http project-proxy {u} --header \"Authorization: Bearer {token}\"")
        }),
        url,
        token,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    allow_lan: bool,
    port: u16,
    /// `http://<ip>:<port>/` for each network interface.
    setup_urls: Vec<(String, String)>,
}

#[tauri::command]
pub async fn device_info(state: State<'_, AppState>) -> CmdResult<DeviceInfo> {
    let port = state
        .engine
        .addr()
        .await
        .map_or(*state.port.lock(), |a| a.port());
    Ok(DeviceInfo {
        allow_lan: state.engine.settings().allow_lan,
        port,
        setup_urls: crate::engine::lan_addresses()
            .into_iter()
            .map(|(iface, ip)| (iface, format!("http://{ip}:{port}/")))
            .collect(),
    })
}

/// Trusts the CA in every booted iOS Simulator.
#[tauri::command]
pub async fn install_simulator_cert(state: State<'_, AppState>) -> CmdResult<String> {
    let pem = state.engine.ca.cert_pem_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("/usr/bin/xcrun")
            .args(["simctl", "keychain", "booted", "add-root-cert"])
            .arg(&pem)
            .output()
            .map_err(|e| format!("Xcode command line tools are needed: {e}"))?;
        if out.status.success() {
            Ok("Installed in the booted Simulator(s).".to_string())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(if err.contains("No devices are booted") {
                "Start a Simulator first.".to_string()
            } else {
                err.trim().to_string()
            })
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Writes the given flows (all when `ids` is omitted) to a HAR file.
#[tauri::command]
pub async fn export_har(
    state: State<'_, AppState>,
    path: String,
    ids: Option<Vec<FlowId>>,
) -> CmdResult<usize> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let flows = engine.snapshot(ids.as_deref());
        std::fs::write(&path, har::to_json(&har::export(&flows))).map_err(|e| e.to_string())?;
        Ok(flows.len())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Saves every flow to a session file (zstd-compressed HAR).
#[tauri::command]
pub async fn save_session(state: State<'_, AppState>, path: String) -> CmdResult<usize> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let flows = engine.snapshot(None);
        let bytes = har::to_session(&har::export(&flows)).map_err(|e| e.to_string())?;
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(flows.len())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Adds the flows in a HAR or session file. With `replace`, the current
/// flows are cleared first (opening a session).
#[tauri::command]
pub async fn import_flows(
    state: State<'_, AppState>,
    path: String,
    replace: bool,
) -> CmdResult<usize> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        engine.import(&bytes, replace).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sends a captured request again, unchanged.
#[tauri::command]
pub async fn repeat_flow(state: State<'_, AppState>, id: FlowId) -> CmdResult<Option<FlowId>> {
    let (meta, data) = state
        .engine
        .store
        .get(id)
        .ok_or("That flow no longer exists.")?;
    state
        .engine
        .send(OutgoingRequest::from_flow(&meta, &data))
        .await
        .map_err(|e| e.to_string())
}

/// Sends an edited or newly composed request.
#[tauri::command]
pub async fn send_request(
    state: State<'_, AppState>,
    request: OutgoingRequest,
) -> CmdResult<Option<FlowId>> {
    state.engine.send(request).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_rules(state: State<'_, AppState>) -> Rules {
    state.engine.rules()
}

#[tauri::command]
pub fn set_rules(state: State<'_, AppState>, rules: Rules) -> CmdResult<Rules> {
    state.engine.set_rules(rules).map_err(|e| e.to_string())?;
    Ok(state.engine.rules())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitView {
    flow_id: FlowId,
    phase: Phase,
    method: String,
    url: String,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    body: BodyView,
    deadline: u64,
}

/// Flows paused at a breakpoint, oldest first.
#[tauri::command]
pub fn list_breakpoints(state: State<'_, AppState>) -> Vec<HitView> {
    state
        .engine
        .breakpoints
        .pending()
        .into_iter()
        .map(|h| HitView {
            flow_id: h.flow_id,
            phase: h.phase,
            method: h.method,
            url: h.url,
            status: h.status,
            headers: h.headers,
            body: BodyView::new(h.body, false),
            deadline: h.deadline,
        })
        .collect()
}

/// Continues a paused flow with `edits`, or aborts it when `abort` is set.
#[tauri::command]
pub fn resume_breakpoint(
    state: State<'_, AppState>,
    flow_id: FlowId,
    phase: Phase,
    abort: bool,
    edits: Option<Edits>,
) -> bool {
    let resume = if abort {
        Resume::Abort
    } else {
        Resume::Continue(edits)
    };
    state.engine.breakpoints.resume(flow_id, phase, resume)
}

#[tauri::command]
pub fn resume_all_breakpoints(state: State<'_, AppState>) {
    state.engine.breakpoints.resume_all();
}

/// Tells the UI when flows are paused or resumed, and brings the window
/// forward when a new one is paused.
pub fn bridge_breakpoints(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut rx = state.engine.breakpoints.subscribe();
    let breakpoints = state.engine.breakpoints.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = 0;
        loop {
            match rx.recv().await {
                Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
            let n = breakpoints.pending().len();
            if n > last
                && let Some(w) = app.get_webview_window("main")
            {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
            last = n;
            let _ = app.emit("breakpoints-changed", n);
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptList {
    dir: String,
    scripts: Vec<ScriptInfo>,
}

#[tauri::command]
pub fn list_scripts(state: State<'_, AppState>) -> ScriptList {
    let scripts = &state.engine.scripts;
    ScriptList {
        dir: scripts.dir().to_string_lossy().into_owned(),
        scripts: scripts.list(),
    }
}

/// Picks up scripts edited outside the app.
#[tauri::command]
pub fn reload_scripts(state: State<'_, AppState>) {
    state.engine.scripts.reload();
}

#[tauri::command]
pub fn read_script(state: State<'_, AppState>, file_name: String) -> CmdResult<String> {
    state
        .engine
        .scripts
        .read(&file_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_script(state: State<'_, AppState>, file_name: String, source: String) -> CmdResult<()> {
    state
        .engine
        .scripts
        .save(&file_name, &source)
        .map_err(|e| e.to_string())
}

/// Creates a script from a template; returns its file name.
#[tauri::command]
pub fn create_script(
    state: State<'_, AppState>,
    name: String,
    matches: String,
    body: String,
) -> CmdResult<String> {
    let source = crate::engine::script::template(&name, &matches, &body);
    state
        .engine
        .scripts
        .create(&source)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_script(state: State<'_, AppState>, file_name: String) -> CmdResult<()> {
    state
        .engine
        .scripts
        .delete(&file_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_script_enabled(
    state: State<'_, AppState>,
    file_name: String,
    enabled: bool,
) -> CmdResult<()> {
    state
        .engine
        .scripts
        .set_enabled(&file_name, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn script_console(state: State<'_, AppState>) -> Vec<LogEntry> {
    state.engine.scripts.console()
}

#[tauri::command]
pub fn clear_script_console(state: State<'_, AppState>) {
    state.engine.scripts.clear_console();
}

/// Tells the UI when scripts or their console change (debounced).
pub fn bridge_scripts(app: &AppHandle) {
    let mut rx = app.state::<AppState>().engine.scripts.subscribe();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            while rx.try_recv().is_ok() {}
            let _ = app.emit("scripts-changed", ());
        }
    });
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FlowBatch {
    Upserts {
        flows: Vec<FlowMeta>,
    },
    Cleared,
    Removed {
        ids: Vec<FlowId>,
    },
    /// The UI fell behind; it should reload with `list_flows`.
    Resync,
}

/// Streams flow changes to the UI in batches (about every 100 ms), keeping
/// only the latest version of each flow per batch.
#[tauri::command]
pub fn subscribe_flows(state: State<'_, AppState>, on_event: Channel<FlowBatch>) {
    let mut rx = state.engine.store.subscribe();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut order: Vec<FlowId> = Vec::new();
            let mut latest: HashMap<FlowId, FlowMeta> = HashMap::new();
            let (mut cleared, mut resync) = (false, false);
            let mut removed: Vec<FlowId> = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(FlowEvent::Upsert(m)) => {
                        if latest.insert(m.id, m.clone()).is_none() {
                            order.push(m.id);
                        }
                    }
                    Ok(FlowEvent::Cleared) => {
                        order.clear();
                        latest.clear();
                        removed.clear();
                        cleared = true;
                    }
                    Ok(FlowEvent::Removed(ids)) => {
                        // A removed flow is never updated again, so dropping
                        // its pending upsert is enough.
                        for id in &ids {
                            latest.remove(id);
                        }
                        removed.extend(ids);
                    }
                    Err(TryRecvError::Lagged(_)) => resync = true,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Closed) => return,
                }
            }
            let mut batches = Vec::new();
            if resync {
                batches.push(FlowBatch::Resync);
            } else {
                if cleared {
                    batches.push(FlowBatch::Cleared);
                }
                if !removed.is_empty() {
                    batches.push(FlowBatch::Removed { ids: removed });
                }
                let flows: Vec<FlowMeta> =
                    order.iter().filter_map(|id| latest.remove(id)).collect();
                if !flows.is_empty() {
                    batches.push(FlowBatch::Upserts { flows });
                }
            }
            for b in batches {
                if on_event.send(b).is_err() {
                    return;
                }
            }
        }
    });
    // Only one live subscription (the webview may reload).
    if let Some(old) = state.bridge.lock().replace(task) {
        old.abort();
    }
}
