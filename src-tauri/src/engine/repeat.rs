//! Repeat and Compose: send a request through our own proxy so it is
//! captured like any other flow (as httpmon's `tui/repeat.go` does).

use std::net::SocketAddr;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::broadcast::error::TryRecvError;

use super::store::{FlowData, FlowEvent, FlowId, FlowMeta, Store};

/// Headers that describe one connection, not the request; reqwest sets its
/// own.
const SKIP_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "proxy-connection",
    "keep-alive",
    "transfer-encoding",
    "upgrade",
    "te",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum RepeatError {
    #[error("start the proxy first")]
    NotRunning,
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("request failed: {0}")]
    Send(String),
}

impl OutgoingRequest {
    /// Rebuilds a captured request exactly as it was sent.
    pub fn from_flow(meta: &FlowMeta, data: &FlowData) -> Self {
        let headers = data
            .request_headers
            .iter()
            // An imported body is already decoded, so its encoding no longer
            // applies.
            .filter(|(k, _)| !(data.bodies_decoded && k.eq_ignore_ascii_case("content-encoding")))
            .cloned()
            .collect();
        Self {
            method: meta.method.clone(),
            url: meta.url.clone(),
            headers,
            body: data.request_body.clone(),
        }
    }
}

/// Sends `req` through the proxy at `proxy_addr`, trusting our CA, and
/// returns the id of the flow it produced (when it could be identified).
pub async fn send(
    store: &Store,
    proxy_addr: SocketAddr,
    ca_der: &[u8],
    req: OutgoingRequest,
) -> Result<Option<FlowId>, RepeatError> {
    let method = reqwest::Method::from_bytes(req.method.trim().as_bytes())
        .map_err(|_| RepeatError::Invalid(format!("bad method \"{}\"", req.method)))?;
    let url =
        reqwest::Url::parse(req.url.trim()).map_err(|e| RepeatError::Invalid(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(RepeatError::Invalid(
            "only http and https URLs can be sent".into(),
        ));
    }

    let proxy = reqwest::Proxy::all(format!("http://{proxy_addr}"))
        .map_err(|e| RepeatError::Send(e.to_string()))?;
    let ca =
        reqwest::Certificate::from_der(ca_der).map_err(|e| RepeatError::Send(e.to_string()))?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .add_root_certificate(ca)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| RepeatError::Send(e.to_string()))?;

    let mut builder = client.request(method.clone(), url.clone());
    for (k, v) in &req.headers {
        let k = k.trim();
        if k.is_empty()
            || k.starts_with(':')
            || SKIP_HEADERS.iter().any(|s| k.eq_ignore_ascii_case(s))
        {
            continue;
        }
        builder = builder.header(k, v.trim());
    }
    if !req.body.is_empty() {
        builder = builder.body(req.body);
    }

    let mut events = store.subscribe();
    let result = async {
        let res = builder.send().await?;
        res.bytes().await
    }
    .await;

    // The proxy records the flow; find it among the store's new flows.
    let host = url.host_str().unwrap_or_default().to_string();
    let path = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let mut found = None;
    loop {
        match events.try_recv() {
            Ok(FlowEvent::Upsert(m)) => {
                if found.is_none()
                    && m.method == method.as_str()
                    && m.host == host
                    && m.path == path
                {
                    found = Some(m.id);
                }
            }
            Ok(FlowEvent::Cleared | FlowEvent::Removed(_)) | Err(TryRecvError::Lagged(_)) => {}
            Err(_) => break,
        }
    }

    match result {
        Ok(_) => Ok(found),
        // The proxy recorded the failure too; select it if we found it.
        Err(e) if found.is_some() => {
            tracing::info!("repeated request failed: {e}");
            Ok(found)
        }
        Err(e) => Err(RepeatError::Send(error_chain(&e))),
    }
}

fn error_chain(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut src = err.source();
    while let Some(e) = src {
        msg.push_str(": ");
        msg.push_str(&e.to_string());
        src = e.source();
    }
    msg
}
