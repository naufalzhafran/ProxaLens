//! `#[tauri::command]` API surface used by the React UI.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::broadcast::error::TryRecvError;

use crate::engine::store::{FlowEvent, FlowId, FlowMeta, WsDirection};
use crate::engine::sysproxy::RestoreReport;
use crate::engine::{DEFAULT_PORT, Engine, LOCALHOST, SysProxyController};

pub struct AppState {
    pub engine: Arc<Engine>,
    pub sysproxy: Arc<SysProxyController>,
    pub port: Mutex<u16>,
    pub last_error: Mutex<Option<String>>,
    pub notice: Mutex<Option<RestoreReport>>,
    pub bridge: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl AppState {
    pub fn new(engine: Arc<Engine>, sysproxy: Arc<SysProxyController>) -> Self {
        Self {
            engine,
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

#[tauri::command]
pub async fn start_proxy(state: State<'_, AppState>, port: Option<u16>) -> CmdResult<Status> {
    let port = port.unwrap_or_else(|| *state.port.lock());
    if state.engine.addr().await.is_some_and(|a| a.port() != port) {
        stop(&state).await?;
    }
    start(&state, port).await?;
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
pub async fn stop_proxy(state: State<'_, AppState>) -> CmdResult<Status> {
    stop(&state).await?;
    Ok(status(&state).await)
}

#[tauri::command]
pub async fn set_system_proxy(state: State<'_, AppState>, enabled: bool) -> CmdResult<Status> {
    let sp = state.sysproxy.clone();
    if enabled {
        let Some(addr) = state.engine.addr().await else {
            return Err("Start the proxy before routing system traffic through it.".into());
        };
        tauri::async_runtime::spawn_blocking(move || sp.enable(LOCALHOST, addr.port()))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    } else {
        tauri::async_runtime::spawn_blocking(move || sp.disable())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }
    Ok(status(&state).await)
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
    /// Set when the captured bytes are valid UTF-8.
    text: Option<String>,
    /// Set otherwise.
    base64: Option<String>,
}

impl BodyView {
    fn new(bytes: Vec<u8>, truncated: bool) -> Self {
        let size = bytes.len();
        match String::from_utf8(bytes) {
            Ok(text) => Self {
                size,
                truncated,
                text: Some(text),
                base64: None,
            },
            Err(e) => Self {
                size,
                truncated,
                text: None,
                base64: Some(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
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

#[tauri::command]
pub fn get_flow(state: State<'_, AppState>, id: FlowId) -> Option<FlowDetail> {
    let (meta, d) = state.engine.store.get(id)?;
    Some(FlowDetail {
        meta,
        request_headers: d.request_headers,
        request_body: BodyView::new(d.request_body, d.request_body_truncated),
        response_headers: d.response_headers,
        response_body: BodyView::new(d.response_body, d.response_body_truncated),
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

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FlowBatch {
    Upserts {
        flows: Vec<FlowMeta>,
    },
    Cleared,
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
                        cleared = true;
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
                if !order.is_empty() {
                    let flows = order.iter().filter_map(|id| latest.remove(id)).collect();
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
