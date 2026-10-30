//! hudsucker handlers that record traffic into the [`Store`].
//!
//! Port of httpmon's `internal/proxy/interceptor.go`, minus scripting.
//! Bodies are captured with a tee so streaming responses (SSE,
//! downloads) keep flowing to the client. Bodies are stored as received;
//! decoding happens when the UI asks (see `decode.rs`).
//!
//! For recorded flows the rules in `rules.rs` apply in this order:
//! block → request rewrite → Map Local (skips the upstream) → Map Remote →
//! upstream → response rewrite → throttle. Only flows a rewrite touches are
//! buffered; everything else streams.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hudsucker::hyper::body::{Body as HttpBody, Frame, SizeHint};
use hudsucker::hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hudsucker::hyper::{Method, Request, Response, StatusCode, Uri, Version, header};
use hudsucker::tokio_tungstenite::tungstenite::Message;
use hudsucker::{
    Body, HttpContext, HttpHandler, RequestOrResponse, WebSocketContext, WebSocketHandler,
};
use parking_lot::{Mutex, RwLock};

use super::decode;
use super::rules::{
    self, CompiledRules, MapLocal, RequestParts, ResponseParts, normalize_url, rewrite_request,
    rewrite_response,
};
use super::settings::Settings;
use super::store::{
    FlowData, FlowId, FlowMeta, FlowState, Store, WS_MESSAGES_CAP, WS_PAYLOAD_CAP, WsDirection,
    WsMessage,
};
use super::throttle::ThrottleBody;
use super::timing::{self, ConnTiming};

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
    pub settings: Arc<RwLock<Settings>>,
    pub rules: Arc<RwLock<Arc<CompiledRules>>>,
    pub processes: Arc<super::procinfo::Resolver>,
    pub local: LocalPages,
    /// Upgraded WebSocket connections, keyed by the client's socket address.
    websockets: Mutex<HashMap<SocketAddr, FlowId>>,
}

/// What the proxy serves about itself: a setup page, its CA certificate
/// and a PAC file (see `local_page`).
#[derive(Clone, Default)]
pub struct LocalPages {
    pub ca_pem: String,
    pub ca_der: Vec<u8>,
    pub port: u16,
}

impl Recorder {
    pub fn new(
        store: Arc<Store>,
        body_cap: usize,
        settings: Arc<RwLock<Settings>>,
        rules: Arc<RwLock<Arc<CompiledRules>>>,
        processes: Arc<super::procinfo::Resolver>,
        local: LocalPages,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            body_cap,
            settings,
            rules,
            processes,
            local,
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
    host: String,
    method: String,
    /// The request URL after rules, default ports removed.
    url: String,
    /// Rules as they were when the request arrived.
    rules: Option<Arc<CompiledRules>>,
}

impl FlowHandler {
    pub fn new(rec: Arc<Recorder>) -> Self {
        Self {
            rec,
            flow: None,
            host: String::new(),
            method: String::new(),
            url: String::new(),
            rules: None,
        }
    }

    /// Notes on the flow that a rule changed it ("Map Local", …).
    fn tag(&self, tag: impl Into<String>) {
        if let Some((id, _)) = self.flow {
            let tag = tag.into();
            self.rec.store.update(id, |m, _| {
                if !m.tags.contains(&tag) {
                    m.tags.push(tag);
                }
            });
        }
    }

    fn fail(&self, error: &str) {
        if let Some((id, started)) = self.flow {
            self.rec.store.update(id, |m, _| {
                m.state = FlowState::Failed;
                m.duration_ms = Some(elapsed_ms(started));
                m.error = Some(error.to_string());
            });
        }
    }

    /// Records a CONNECT that won't be decrypted (blocked or tunnelled).
    fn record_connect(&self, ctx: &HttpContext, req: &Request<Body>, state: FlowState) {
        let authority = req
            .uri()
            .authority()
            .map(|a| a.to_string())
            .unwrap_or_default();
        let meta = FlowMeta {
            id: 0,
            method: "CONNECT".into(),
            scheme: "https".into(),
            host: req.uri().host().unwrap_or_default().to_string(),
            path: String::new(),
            url: authority,
            http_version: version_str(req.version()).into(),
            status: (state == FlowState::Blocked).then_some(403),
            state,
            content_type: None,
            started_at: now_ms(),
            ttfb_ms: None,
            duration_ms: Some(0.0),
            request_size: 0,
            response_size: 0,
            client_addr: ctx.client_addr.to_string(),
            error: (state == FlowState::Blocked).then(|| BLOCKED.to_string()),
            ws_message_count: 0,
            timings: None,
            tags: Vec::new(),
            process: None,
            pid: None,
        };
        let data = FlowData {
            request_headers: headers_vec(req.headers()),
            ..Default::default()
        };
        let id = self.rec.store.add(meta, data);
        self.rec.processes.resolve(id, ctx.client_addr);
    }
}

const BLOCKED: &str = "Blocked by the block list";

fn blocked_response() -> Response<Body> {
    text_response(StatusCode::FORBIDDEN, &format!("{BLOCKED}."))
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

/// Reads a whole body; used only for flows a rewrite edits.
async fn collect(body: Body) -> Result<Vec<u8>, String> {
    use http_body_util::BodyExt;
    body.collect()
        .await
        .map(|c| c.to_bytes().to_vec())
        .map_err(|e| e.to_string())
}

/// Removes the Content-Encoding so the body can be read and edited; the
/// edited body is sent uncompressed.
fn decode_for_edit(headers: &mut Vec<(String, String)>, body: &mut Vec<u8>) {
    let encoding = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
        .map(|(_, v)| v.clone());
    if let Ok(Some(d)) = decode::decode(body, encoding.as_deref())
        && !d.partial
    {
        *body = d.bytes;
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-encoding"));
    }
}

/// Builds a header map for a rebuilt message, fixing its framing headers.
fn build_headers(headers: &[(String, String)], body_len: Option<usize>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("content-length") || k.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        if let (Ok(k), Ok(v)) = (
            HeaderName::from_bytes(k.trim().as_bytes()),
            HeaderValue::from_str(v.trim()),
        ) {
            map.append(k, v);
        }
    }
    if let Some(len) = body_len {
        map.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    }
    map
}

fn text_response(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(format!("ProxaLens: {msg}")))
        .expect("static response")
}

fn build_response(status: u16, headers: &[(String, String)], body: Vec<u8>) -> Response<Body> {
    let mut res = Response::new(Body::empty());
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    *res.headers_mut() = build_headers(headers, Some(body.len()));
    *res.body_mut() = Body::from(body);
    res
}

async fn map_local_response(rule: &MapLocal) -> Result<Response<Body>, String> {
    let body = if rule.file.trim().is_empty() {
        rule.body.clone().into_bytes()
    } else {
        tokio::fs::read(rule.file.trim())
            .await
            .map_err(|e| format!("Map Local could not read {}: {e}", rule.file))?
    };
    let mut headers = rule.headers.clone();
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        && let Some(ct) = rules::content_type_for(&rule.file)
    {
        headers.push(("Content-Type".into(), ct.into()));
    }
    Ok(build_response(rule.status, &headers, body))
}

/// Hosts that always mean "the proxy itself" when requested through it.
const LOCAL_HOSTS: &[&str] = &["proxy.local", "project.proxy"];

/// Answers requests addressed to the proxy itself: sent straight to it
/// (`http://<mac-ip>:9090/`) or through it to `http://proxy.local/`.
pub fn local_page(local: &LocalPages, req: &Request<Body>) -> Option<Response<Body>> {
    let uri = req.uri();
    let direct = uri.authority().is_none();
    if !direct && !uri.host().is_some_and(|h| LOCAL_HOSTS.contains(&h)) {
        return None;
    }
    let host_header = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|_| direct)
        .map(str::to_string)
        .unwrap_or_else(|| format!("127.0.0.1:{}", local.port));
    let respond = |status: StatusCode, ct: &str, body: Vec<u8>, extra: Option<(&str, &str)>| {
        let mut b = Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, ct)
            .header(header::CACHE_CONTROL, "no-store");
        if let Some((k, v)) = extra {
            b = b.header(k, v);
        }
        b.body(Body::from(body)).expect("static response")
    };
    Some(match uri.path() {
        "/" | "/setup" => respond(
            StatusCode::OK,
            "text/html; charset=utf-8",
            setup_html(&host_header).into_bytes(),
            None,
        ),
        "/cert" | "/cert.pem" => respond(
            StatusCode::OK,
            "application/x-x509-ca-cert",
            local.ca_pem.clone().into_bytes(),
            Some((
                "content-disposition",
                "attachment; filename=\"ProxaLensCA.pem\"",
            )),
        ),
        "/cert.cer" => respond(
            StatusCode::OK,
            "application/x-x509-ca-cert",
            local.ca_der.clone(),
            Some((
                "content-disposition",
                "attachment; filename=\"ProxaLensCA.cer\"",
            )),
        ),
        "/proxy.pac" => respond(
            StatusCode::OK,
            "application/x-ns-proxy-autoconfig",
            pac_file(&host_header).into_bytes(),
            None,
        ),
        _ => respond(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            b"Not found".to_vec(),
            None,
        ),
    })
}

/// A PAC file sending everything but local names through `proxy`.
pub fn pac_file(proxy: &str) -> String {
    format!(
        "function FindProxyForURL(url, host) {{\n  if (isPlainHostName(host) || host === \"localhost\" || shExpMatch(host, \"127.*\") || shExpMatch(host, \"*.local\")) return \"DIRECT\";\n  return \"PROXY {proxy}; DIRECT\";\n}}\n"
    )
}

fn setup_html(proxy: &str) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ProxaLens setup</title>
<style>body{{font:16px -apple-system,system-ui,sans-serif;max-width:34rem;margin:2rem auto;padding:0 1rem;line-height:1.5}}a.b{{display:block;margin:.5rem 0;padding:.75rem 1rem;border-radius:.5rem;background:#0284c7;color:#fff;text-decoration:none;text-align:center}}code{{background:#eee;padding:.1rem .3rem;border-radius:.25rem}}</style></head>
<body><h1>ProxaLens</h1>
<p>Proxy address: <code>{proxy}</code></p>
<h2>1. Install the certificate</h2>
<a class="b" href="/cert.pem">Download certificate</a>
<p><b>iPhone / iPad:</b> after downloading, open Settings → Profile Downloaded → Install. Then turn it on under Settings → General → About → Certificate Trust Settings.</p>
<p><b>Android:</b> Settings → Security → Encryption &amp; credentials → Install a certificate → CA certificate. Apps only trust it if they opt in to user certificates.</p>
<h2>2. Use the proxy</h2>
<p>Wi-Fi settings → your network → Configure Proxy → Manual: server and port from the address above. Or Automatic, with URL <code>http://{proxy}/proxy.pac</code>.</p>
</body></html>"#
    )
}

fn split_url(url: &str) -> (String, String, String) {
    match url.parse::<Uri>() {
        Ok(u) => (
            u.scheme_str().unwrap_or("http").to_string(),
            u.host().unwrap_or_default().to_string(),
            u.path_and_query()
                .map_or("/".into(), |p| p.as_str().to_string()),
        ),
        Err(_) => ("http".into(), String::new(), url.to_string()),
    }
}

impl HttpHandler for FlowHandler {
    async fn handle_request(&mut self, ctx: &HttpContext, req: Request<Body>) -> RequestOrResponse {
        // CONNECT only opens a tunnel; the requests inside it are recorded,
        // unless the tunnel is refused here or not decrypted (see
        // `should_intercept_connect`).
        if req.method() == Method::CONNECT {
            let host = req.uri().host().unwrap_or_default();
            let policy = self.rec.settings.read().policy(host);
            if policy.block {
                if policy.record {
                    self.record_connect(ctx, &req, FlowState::Blocked);
                }
                return blocked_response().into();
            }
            return req.into();
        }

        if let Some(res) = local_page(&self.rec.local, &req) {
            return res.into();
        }

        let uri = req.uri();
        let url = normalize_url(&uri.to_string());
        let (scheme, _, path) = split_url(&url);
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
        let method = req.method().to_string();
        let websocket = is_websocket_upgrade(&req);
        self.host = host.clone();
        let policy = self.rec.settings.read().policy(&host);
        // Rules apply to recorded traffic only, so that what they change is
        // always visible.
        if !policy.record {
            return if policy.block {
                blocked_response().into()
            } else {
                req.into()
            };
        }
        let rules = self.rec.rules.read().clone();
        let rule_blocked = rules.is_blocked(&method, &url);

        let meta = FlowMeta {
            id: 0,
            method: method.clone(),
            url: url.clone(),
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
            timings: None,
            tags: Vec::new(),
            process: None,
            pid: None,
        };
        let data = FlowData {
            request_headers: headers_vec(req.headers()),
            ..Default::default()
        };
        let id = self.rec.store.add(meta, data);
        self.rec.processes.resolve(id, ctx.client_addr);
        let started = Instant::now();
        self.flow = Some((id, started));
        self.method = method;
        self.url = url;

        if policy.block || rule_blocked {
            if rule_blocked {
                self.tag("Block");
            }
            self.rec.store.update(id, |m, _| {
                m.state = FlowState::Blocked;
                m.status = Some(403);
                m.duration_ms = Some(elapsed_ms(started));
                m.error = Some(BLOCKED.into());
            });
            return blocked_response().into();
        }

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

        let (mut parts, mut body) = req.into_parts();
        let rewrites = rules.rewrites(&self.method, &self.url, true);

        if !rewrites.is_empty() {
            let bytes = match collect(body).await {
                Ok(b) => b,
                Err(e) => {
                    self.fail(&e);
                    return text_response(StatusCode::BAD_GATEWAY, &e).into();
                }
            };
            let mut req = RequestParts {
                method: self.method.clone(),
                url: self.url.clone(),
                headers: headers_vec(&parts.headers),
                body: bytes,
            };
            if rewrites.iter().any(|(r, _)| r.target.is_body()) {
                decode_for_edit(&mut req.headers, &mut req.body);
            }
            let before = req.clone();
            rewrite_request(&mut req, &rewrites);
            if req != before {
                self.tag("Rewrite");
            }

            if let Ok(m) = Method::from_bytes(req.method.as_bytes()) {
                parts.method = m;
            }
            if let Ok(u) = req.url.parse::<Uri>()
                && u.scheme().is_some()
                && u.authority().is_some()
            {
                parts.uri = u;
            }
            self.method = parts.method.to_string();
            self.url = normalize_url(&parts.uri.to_string());
            let has_body =
                !req.body.is_empty() || parts.headers.contains_key(header::CONTENT_LENGTH);
            parts.headers = build_headers(&req.headers, has_body.then_some(req.body.len()));
            body = Body::from(req.body);
            let (method, url, headers) = (
                self.method.clone(),
                self.url.clone(),
                headers_vec(&parts.headers),
            );
            self.rec.store.update(id, |m, d| {
                let (scheme, host, path) = split_url(&url);
                (m.method, m.url, m.scheme, m.host, m.path) = (method, url, scheme, host, path);
                d.request_headers = headers;
            });
        }

        // Map Local answers without contacting the server.
        if let Some(rule) = rules.map_local(&self.method, &self.url) {
            self.tag("Map Local");
            self.rules = Some(rules.clone());
            let res = match map_local_response(rule).await {
                Ok(res) => res,
                Err(e) => text_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
            };
            return self.finish_response(res).await.into();
        }

        if let Some(rule) = rules.map_remote(&self.method, &self.url)
            && let Some(to) = rules::map_remote_url(&self.url, &rule.to)
            && let Ok(uri) = to.parse::<Uri>()
        {
            parts.uri = uri;
            self.tag(format!("Map Remote → {to}"));
        }
        self.rules = Some(rules);

        let store = self.rec.store.clone();
        let body = CaptureBody::wrap(body, self.rec.body_cap, move |cap| {
            let sent = elapsed_ms(started);
            store.update(id, |m, d| {
                m.request_size = cap.total;
                d.request_body = cap.bytes;
                d.request_body_truncated = cap.truncated;
                d.request_sent_ms = Some(sent);
            });
        });
        Request::from_parts(parts, body).into()
    }

    async fn should_intercept_connect(&mut self, ctx: &HttpContext, req: &Request<Body>) -> bool {
        let host = req.uri().host().unwrap_or_default();
        let (intercept, record) = {
            let s = self.rec.settings.read();
            (s.should_intercept(host), s.policy(host).record)
        };
        if !intercept && record {
            self.record_connect(ctx, req, FlowState::Tunnel);
        }
        intercept
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        self.finish_response(res).await
    }

    async fn handle_error(
        &mut self,
        _ctx: &HttpContext,
        err: hudsucker::hyper_util::client::legacy::Error,
    ) -> Response<Body> {
        let msg = error_chain(&err);
        self.fail(&msg);
        text_response(
            StatusCode::BAD_GATEWAY,
            &format!("upstream request failed: {msg}"),
        )
    }
}

impl FlowHandler {
    /// Response side: rewrite, throttle, then record.
    async fn finish_response(&mut self, res: Response<Body>) -> Response<Body> {
        let ttfb = self.flow.map(|(_, started)| elapsed_ms(started));
        // The connection's timings belong to the first flow that sees them;
        // every later flow on that connection reused it.
        let conn = res.extensions().get::<ConnTiming>().cloned();
        let reused = conn.as_ref().is_some_and(|c| !c.claim());
        let phases = self
            .flow
            .zip(conn.filter(|_| !reused))
            .map(|((_, started), c)| c.phases(started));
        let mut res = res;
        if let Some(rules) = self.rules.clone()
            && self.flow.is_some()
        {
            let rewrites = rules.rewrites(&self.method, &self.url, false);
            if !rewrites.is_empty() {
                let (mut parts, body) = res.into_parts();
                let bytes = match collect(body).await {
                    Ok(b) => b,
                    Err(e) => {
                        self.fail(&e);
                        return text_response(StatusCode::BAD_GATEWAY, &e);
                    }
                };
                let mut r = ResponseParts {
                    status: parts.status.as_u16(),
                    headers: headers_vec(&parts.headers),
                    body: bytes,
                };
                if rewrites.iter().any(|(rw, _)| rw.target.is_body()) {
                    decode_for_edit(&mut r.headers, &mut r.body);
                }
                let before = r.clone();
                rewrite_response(&mut r, &rewrites);
                if r != before {
                    self.tag("Rewrite");
                }
                parts.status = StatusCode::from_u16(r.status).unwrap_or(parts.status);
                let body_len = (self.method != "HEAD").then_some(r.body.len());
                parts.headers = build_headers(&r.headers, body_len);
                res = Response::from_parts(parts, Body::from(r.body));
            }
        }

        let throttle = self.rec.settings.read().throttle_for(&self.host);
        let res = match throttle {
            Some((bytes_per_sec, latency_ms)) => {
                if latency_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(latency_ms)).await;
                }
                let (parts, body) = res.into_parts();
                Response::from_parts(parts, ThrottleBody::wrap(body, bytes_per_sec))
            }
            None => res,
        };
        let Some((id, started)) = self.flow else {
            return res;
        };
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
            m.ttfb_ms = ttfb;
            m.content_type = content_type;
            m.http_version = version.into();
            d.response_headers = headers;
            m.timings = ttfb.map(|t| timing::split(t, d.request_sent_ms, phases, reused));
        });

        let store = self.rec.store.clone();
        let (parts, body) = res.into_parts();
        let body = CaptureBody::wrap(body, self.rec.body_cap, move |cap| {
            store.update(id, |m, d| {
                m.response_size = cap.total;
                let duration = elapsed_ms(started);
                m.duration_ms = Some(duration);
                if let Some(t) = m.timings.as_mut() {
                    t.receive = Some((duration - m.ttfb_ms.unwrap_or(duration)).max(0.0));
                }
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
