//! hudsucker handlers that record traffic into the [`Store`].
//!
//! Port of httpmon's `internal/proxy/interceptor.go`, minus scripting,
//! throttling and decoding (later phases). Bodies are captured with a tee so
//! streaming responses (SSE, downloads) keep flowing to the client.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hudsucker::hyper::body::{Body as HttpBody, Frame, SizeHint};
use hudsucker::hyper::{Method, Request, Response, StatusCode, Version, header};
use hudsucker::tokio_tungstenite::tungstenite::Message;
use hudsucker::{
    Body, HttpContext, HttpHandler, RequestOrResponse, WebSocketContext, WebSocketHandler,
};
use parking_lot::Mutex;

use super::store::{
    FlowData, FlowId, FlowMeta, FlowState, Store, WS_MESSAGES_CAP, WS_PAYLOAD_CAP, WsDirection,
    WsMessage,
};

/// Bodies larger than this are recorded truncated (httpmon uses 5 MB too).
pub const DEFAULT_BODY_CAP: usize = 5 * 1024 * 1024;

pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

/// Shared state for all handler clones.
pub struct Recorder {
    pub store: Arc<Store>,
    pub body_cap: usize,
    /// Upgraded WebSocket connections, keyed by the client's socket address.
    websockets: Mutex<HashMap<SocketAddr, FlowId>>,
}

impl Recorder {
    pub fn new(store: Arc<Store>, body_cap: usize) -> Arc<Self> {
        Arc::new(Self {
            store,
            body_cap,
            websockets: Mutex::new(HashMap::new()),
        })
    }
}

/// hudsucker clones the handler for every request, so per-flow state lives
/// directly on the struct between `handle_request` and `handle_response`.
#[derive(Clone)]
pub struct FlowHandler {
    rec: Arc<Recorder>,
    flow: Option<(FlowId, Instant)>,
}

impl FlowHandler {
    pub fn new(rec: Arc<Recorder>) -> Self {
        Self { rec, flow: None }
    }
}

fn headers_vec(h: &header::HeaderMap) -> Vec<(String, String)> {
    h.iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn version_str(v: Version) -> &'static str {
    match v {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_3 => "HTTP/3",
        _ => "HTTP",
    }
}

fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn is_websocket_upgrade(req: &Request<Body>) -> bool {
    req.headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

impl HttpHandler for FlowHandler {
    async fn handle_request(&mut self, ctx: &HttpContext, req: Request<Body>) -> RequestOrResponse {
        // CONNECT only opens a tunnel; the requests inside it are recorded.
        if req.method() == Method::CONNECT {
            return req.into();
        }

        let uri = req.uri();
        let scheme = uri.scheme_str().unwrap_or("http").to_string();
        let host = uri
            .host()
            .map(str::to_string)
            .or_else(|| {
                req.headers()
                    .get(header::HOST)
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let path = uri
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".into());
        let websocket = is_websocket_upgrade(&req);

        let meta = FlowMeta {
            id: 0,
            method: req.method().to_string(),
            url: uri.to_string(),
            scheme,
            host,
            path,
            http_version: version_str(req.version()).into(),
            status: None,
            state: FlowState::InProgress,
            content_type: req
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            started_at: now_ms(),
            ttfb_ms: None,
            duration_ms: None,
            request_size: 0,
            response_size: 0,
            client_addr: ctx.client_addr.to_string(),
            error: None,
            ws_message_count: 0,
        };
        let data = FlowData {
            request_headers: headers_vec(req.headers()),
            ..Default::default()
        };
        let id = self.rec.store.add(meta, data);
        let started = Instant::now();
        self.flow = Some((id, started));

        if websocket {
            // hudsucker answers the upgrade itself and never calls
            // handle_response, so mark the flow now.
            self.rec.websockets.lock().insert(ctx.client_addr, id);
            self.rec.store.update(id, |m, _| {
                m.state = FlowState::WebSocket;
                m.status = Some(101);
                m.ttfb_ms = Some(elapsed_ms(started));
            });
            return req.into();
        }

        let store = self.rec.store.clone();
        let (parts, body) = req.into_parts();
        let body = CaptureBody::wrap(body, self.rec.body_cap, move |cap| {
            store.update(id, |m, d| {
                m.request_size = cap.total;
                d.request_body = cap.bytes;
                d.request_body_truncated = cap.truncated;
            });
        });
        Request::from_parts(parts, body).into()
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        let Some((id, started)) = self.flow else {
            return res;
        };
        let ttfb = elapsed_ms(started);
        let headers = headers_vec(res.headers());
        let content_type = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let status = res.status().as_u16();
        let version = version_str(res.version());
        self.rec.store.update(id, |m, d| {
            m.status = Some(status);
            m.ttfb_ms = Some(ttfb);
            m.content_type = content_type;
            m.http_version = version.into();
            d.response_headers = headers;
        });

        let store = self.rec.store.clone();
        let (parts, body) = res.into_parts();
        let body = CaptureBody::wrap(body, self.rec.body_cap, move |cap| {
            store.update(id, |m, d| {
                m.response_size = cap.total;
                m.duration_ms = Some(elapsed_ms(started));
                match cap.error {
                    None => m.state = FlowState::Completed,
                    Some(e) => {
                        m.state = FlowState::Failed;
                        m.error = Some(e);
                    }
                }
                d.response_body = cap.bytes;
                d.response_body_truncated = cap.truncated;
            });
        });
        Response::from_parts(parts, body)
    }

    async fn handle_error(
        &mut self,
        _ctx: &HttpContext,
        err: hudsucker::hyper_util::client::legacy::Error,
    ) -> Response<Body> {
        let msg = error_chain(&err);
        if let Some((id, started)) = self.flow {
            let msg = msg.clone();
            self.rec.store.update(id, |m, _| {
                m.state = FlowState::Failed;
                m.duration_ms = Some(elapsed_ms(started));
                m.error = Some(msg);
            });
        }
        Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(format!(
                "Project Proxy: upstream request failed: {msg}"
            )))
            .expect("static response")
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

impl WebSocketHandler for FlowHandler {
    async fn handle_message(
        &mut self,
        ctx: &WebSocketContext,
        message: Message,
    ) -> Option<Message> {
        let (addr, direction) = match ctx {
            WebSocketContext::ClientToServer { src, .. } => (*src, WsDirection::ClientToServer),
            WebSocketContext::ServerToClient { dst, .. } => (*dst, WsDirection::ServerToClient),
        };
        let id = self.rec.websockets.lock().get(&addr).copied();
        if let Some(id) = id {
            let (kind, payload): (&'static str, &[u8]) = match &message {
                Message::Text(t) => ("text", t.as_bytes()),
                Message::Binary(b) => ("binary", b),
                Message::Ping(b) => ("ping", b),
                Message::Pong(b) => ("pong", b),
                Message::Close(_) => ("close", &[]),
                Message::Frame(f) => ("binary", f.payload()),
            };
            let msg = WsMessage {
                direction,
                at: now_ms(),
                kind,
                size: payload.len(),
                payload: payload[..payload.len().min(WS_PAYLOAD_CAP)].to_vec(),
            };
            self.rec.store.update(id, |m, d| {
                m.ws_message_count += 1;
                if d.ws_messages.len() < WS_MESSAGES_CAP {
                    d.ws_messages.push(msg);
                }
            });
            if kind == "close" {
                self.rec.websockets.lock().remove(&addr);
            }
        }
        Some(message)
    }
}

/// Result of teeing a body.
pub struct Captured {
    pub bytes: Vec<u8>,
    pub total: u64,
    pub truncated: bool,
    /// Set when the stream errored or was dropped before finishing.
    pub error: Option<String>,
}

type OnDone = Box<dyn FnOnce(Captured) + Send + Sync>;

/// A pass-through body that copies up to `cap` bytes and reports them once
/// the stream ends, errors, or is dropped.
pub struct CaptureBody {
    inner: Body,
    buf: Vec<u8>,
    cap: usize,
    total: u64,
    truncated: bool,
    on_done: Option<OnDone>,
}

impl CaptureBody {
    pub fn wrap(
        inner: Body,
        cap: usize,
        on_done: impl FnOnce(Captured) + Send + Sync + 'static,
    ) -> Body {
        let body = Self {
            inner,
            buf: Vec::new(),
            cap,
            total: 0,
            truncated: false,
            on_done: Some(Box::new(on_done)),
        };
        if body.inner.is_end_stream() {
            // Nothing will ever be polled; report immediately.
            let mut body = body;
            body.finish(None);
            return Body::from(BoxBody::new(body));
        }
        Body::from(BoxBody::new(body))
    }

    fn finish(&mut self, error: Option<String>) {
        if let Some(cb) = self.on_done.take() {
            cb(Captured {
                bytes: std::mem::take(&mut self.buf),
                total: self.total,
                truncated: self.truncated,
                error,
            });
        }
    }
}

impl HttpBody for CaptureBody {
    type Data = Bytes;
    type Error = hudsucker::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_frame(cx);
        match &res {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.total += data.len() as u64;
                    let room = this.cap.saturating_sub(this.buf.len());
                    if data.len() > room {
                        this.truncated = true;
                    }
                    this.buf.extend_from_slice(&data[..data.len().min(room)]);
                }
                if this.inner.is_end_stream() {
                    this.finish(None);
                }
            }
            Poll::Ready(Some(Err(e))) => this.finish(Some(e.to_string())),
            Poll::Ready(None) => this.finish(None),
            Poll::Pending => {}
        }
        res
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for CaptureBody {
    fn drop(&mut self) {
        if self.on_done.is_some() {
            self.finish(Some("connection closed before body finished".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn capture_body_truncates_but_forwards_everything() {
        let got = Arc::new(Mutex::new(None));
        let g = got.clone();
        let body = CaptureBody::wrap(Body::from(vec![7u8; 100]), 10, move |c| {
            *g.lock() = Some((c.bytes, c.total, c.truncated, c.error));
        });
        let forwarded = body.collect().await.unwrap().to_bytes();
        assert_eq!(forwarded.len(), 100);
        let (bytes, total, truncated, error) = got.lock().take().unwrap();
        assert_eq!(bytes.len(), 10);
        assert_eq!(total, 100);
        assert!(truncated);
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn capture_body_reports_on_drop() {
        let got = Arc::new(Mutex::new(None));
        let g = got.clone();
        let stream = hudsucker::futures::stream::pending::<Result<Bytes, hudsucker::Error>>();
        let body = CaptureBody::wrap(Body::from_stream(stream), 10, move |c| {
            *g.lock() = Some(c.error);
        });
        drop(body);
        assert!(got.lock().take().unwrap().is_some());
    }

    #[test]
    fn empty_body_reports_immediately() {
        let got = Arc::new(Mutex::new(false));
        let g = got.clone();
        let _body = CaptureBody::wrap(Body::empty(), 10, move |c| {
            assert_eq!(c.total, 0);
            *g.lock() = true;
        });
        assert!(*got.lock());
    }
}
