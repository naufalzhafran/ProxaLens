//! MCP server so LLM agents can inspect and drive the proxy. A port of
//! httpmon's `internal/mcpserver`: Streamable HTTP on 127.0.0.1, protected by
//! a bearer token stored in the data directory.
//!
//! Tools: list/search/count/get requests, export HAR, replay, throttle,
//! mock responses (Map Local rules) and script management.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hudsucker::hyper::body::Incoming;
use hudsucker::hyper::server::conn::http1;
use hudsucker::hyper::service::service_fn;
use hudsucker::hyper::{Request, Response, StatusCode};
use hudsucker::hyper_util::rt::TokioIo;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_service::Service as _;

use crate::engine::filter::{self, Filter};
use crate::engine::repeat::OutgoingRequest;
use crate::engine::rules::{MapLocal, Matcher};
use crate::engine::script;
use crate::engine::store::{FlowData, FlowMeta};
use crate::engine::{Engine, har, settings};

pub const DEFAULT_PORT: u16 = 9091;
const TOKEN_FILE: &str = "mcp-token";
const MAX_LIMIT: usize = 500;

/// Reads the bearer token, creating a random one on first use (port of
/// httpmon's `LoadOrCreateToken`).
pub fn load_or_create_token(data_dir: &Path) -> std::io::Result<String> {
    let path = data_dir.join(TOKEN_FILE);
    if let Ok(t) = std::fs::read_to_string(&path)
        && !t.trim().is_empty()
    {
        return Ok(t.trim().to_string());
    }
    let mut buf = [0u8; 32];
    getrandom(&mut buf)?;
    let token: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(data_dir)?;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    f.write_all(format!("{token}\n").as_bytes())?;
    Ok(token)
}

fn getrandom(buf: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(buf)
}

fn summary(m: &FlowMeta) -> Json {
    json!({
        "id": m.id,
        "method": m.method,
        "url": m.url,
        "status": m.status,
        "state": m.state,
        "contentType": m.content_type,
        "durationMs": m.duration_ms,
        "responseSize": m.response_size,
        "app": m.process,
        "startedAt": m.started_at,
        "tags": m.tags,
        "error": m.error,
    })
}

fn body_json(
    bytes: &[u8],
    headers: &[(String, String)],
    decoded: bool,
    truncated: bool,
    max: usize,
) -> Json {
    let body = har::display_body(bytes, headers, decoded);
    let size = body.len();
    let shown = &body[..size.min(max)];
    match std::str::from_utf8(shown) {
        Ok(text) => json!({ "size": size, "truncated": truncated || size > max, "text": text }),
        Err(_) => json!({ "size": size, "truncated": truncated, "binary": true }),
    }
}

fn header_list(h: &[(String, String)]) -> Json {
    Json::Array(h.iter().map(|(k, v)| json!([k, v])).collect())
}

fn pretty(v: &Json) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Filter expression, e.g. `m:post s:4xx api.example.com` (see the server instructions).
    #[serde(default)]
    pub filter: Option<String>,
    /// Max results (default 50, max 500).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Skip this many of the newest matches.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IdParams {
    /// The request id from list_requests.
    pub id: u64,
    /// Truncate each body to this many bytes (default 65536).
    #[serde(default)]
    pub max_body_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Substring of host + path.
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CountParams {
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExportParams {
    /// Ids to export; all matching `filter` when omitted.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    #[serde(default)]
    pub filter: Option<String>,
    /// Max entries (default 100).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReplayParams {
    /// Replay this captured request unchanged…
    #[serde(default)]
    pub id: Option<u64>,
    /// …or send a new one (method and url required).
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<Vec<(String, String)>>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ThrottleParams {
    /// false switches throttling off.
    pub enabled: bool,
    /// "3g", "4g" or "wifi"; overrides the numbers below.
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub bytes_per_sec: Option<u64>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MockParams {
    /// URL wildcard, e.g. `https://api.example.com/v1/users*`.
    pub url_pattern: String,
    /// Only this method (default any).
    #[serde(default)]
    pub method: Option<String>,
    /// Default 200.
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub headers: Option<Vec<(String, String)>>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RuleIdParams {
    pub rule_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScriptParams {
    pub file_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateScriptParams {
    pub name: String,
    /// Glob, e.g. `*://api.example.com/*`.
    pub r#match: String,
    /// JavaScript defining onRequest(ctx) and/or onResponse(ctx).
    pub source: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ToggleScriptParams {
    pub file_name: String,
    pub enabled: bool,
}

#[derive(Clone)]
pub struct Tools {
    engine: Arc<Engine>,
    tool_router: ToolRouter<Self>,
}

fn compile(filter: &Option<String>) -> Option<Filter> {
    filter.as_deref().and_then(Filter::compile)
}

#[tool_router]
impl Tools {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            tool_router: Self::tool_router(),
        }
    }

    /// Newest-first metas matching a filter.
    fn matching(&self, filter: Option<&Filter>) -> Vec<FlowMeta> {
        let (mut flows, _) = self.engine.store.list(0, usize::MAX);
        flows.reverse();
        flows.retain(|m| filter::matches(filter, m));
        flows
    }

    #[tool(
        description = "List captured HTTP requests, newest first, with optional filtering and pagination."
    )]
    fn list_requests(&self, Parameters(p): Parameters<ListParams>) -> String {
        let limit = p.limit.unwrap_or(50).clamp(1, MAX_LIMIT);
        let flows = self.matching(compile(&p.filter).as_ref());
        let page: Vec<Json> = flows
            .iter()
            .skip(p.offset.unwrap_or(0))
            .take(limit)
            .map(summary)
            .collect();
        pretty(&json!({ "total": flows.len(), "requests": page }))
    }

    #[tool(description = "Get full details of a captured request: headers and decoded bodies.")]
    fn get_request(&self, Parameters(p): Parameters<IdParams>) -> Result<String, String> {
        let (m, d): (FlowMeta, FlowData) = self
            .engine
            .store
            .get(p.id)
            .ok_or("no request with that id")?;
        let max = p.max_body_bytes.unwrap_or(64 * 1024);
        Ok(pretty(&json!({
            "request": summary(&m),
            "requestHeaders": header_list(&d.request_headers),
            "requestBody": body_json(&d.request_body, &d.request_headers, d.bodies_decoded, d.request_body_truncated, max),
            "responseHeaders": header_list(&d.response_headers),
            "responseBody": body_json(&d.response_body, &d.response_headers, d.bodies_decoded, d.response_body_truncated, max),
            "websocketMessages": d.ws_messages.len(),
        })))
    }

    #[tool(description = "Search captured requests by substring of host + path.")]
    fn search_requests(&self, Parameters(p): Parameters<SearchParams>) -> String {
        let q = p.query.to_lowercase();
        let limit = p.limit.unwrap_or(50).clamp(1, MAX_LIMIT);
        let hits: Vec<Json> = self
            .matching(None)
            .iter()
            .filter(|m| format!("{}{}", m.host, m.path).to_lowercase().contains(&q))
            .take(limit)
            .map(summary)
            .collect();
        pretty(&json!({ "requests": hits }))
    }

    #[tool(description = "Count captured requests, optionally filtered.")]
    fn get_request_count(&self, Parameters(p): Parameters<CountParams>) -> String {
        self.matching(compile(&p.filter).as_ref()).len().to_string()
    }

    #[tool(description = "Export captured requests as a HAR 1.2 JSON document.")]
    fn export_har(&self, Parameters(p): Parameters<ExportParams>) -> String {
        let limit = p.limit.unwrap_or(100).clamp(1, MAX_LIMIT);
        let ids: Vec<u64> = match p.ids {
            Some(ids) => ids.into_iter().take(limit).collect(),
            None => {
                let mut ids: Vec<u64> = self
                    .matching(compile(&p.filter).as_ref())
                    .iter()
                    .take(limit)
                    .map(|m| m.id)
                    .collect();
                ids.reverse();
                ids
            }
        };
        let flows = self.engine.snapshot(Some(&ids));
        String::from_utf8(har::to_json(&har::export(&flows))).unwrap_or_default()
    }

    #[tool(
        description = "Replay a captured request by id, or send a new request (method, url, headers, body) through the proxy. Returns the new request's id and status."
    )]
    async fn replay_request(
        &self,
        Parameters(p): Parameters<ReplayParams>,
    ) -> Result<String, String> {
        let req = match p.id {
            Some(id) => {
                let (m, d) = self.engine.store.get(id).ok_or("no request with that id")?;
                OutgoingRequest::from_flow(&m, &d)
            }
            None => OutgoingRequest {
                method: p.method.ok_or("method is required without an id")?,
                url: p.url.ok_or("url is required without an id")?,
                headers: p.headers.unwrap_or_default(),
                body: p.body.unwrap_or_default().into_bytes(),
            },
        };
        let id = self.engine.send(req).await.map_err(|e| e.to_string())?;
        let status = id
            .and_then(|i| self.engine.store.get(i))
            .and_then(|(m, _)| m.status);
        Ok(pretty(&json!({ "id": id, "status": status })))
    }

    #[tool(
        description = "Set network throttling: a preset (3g/4g/wifi) or custom bytes per second and added latency."
    )]
    fn set_throttle(&self, Parameters(p): Parameters<ThrottleParams>) -> Result<String, String> {
        let mut s = self.engine.settings();
        s.throttle.enabled = p.enabled;
        if let Some(name) = p.preset {
            let (bps, lat) = settings::preset(&name).ok_or("preset must be 3g, 4g or wifi")?;
            s.throttle.preset = name.to_lowercase();
            s.throttle.bytes_per_sec = bps;
            s.throttle.latency_ms = lat;
        } else if p.bytes_per_sec.is_some() || p.latency_ms.is_some() {
            s.throttle.preset = "custom".into();
            s.throttle.bytes_per_sec = p.bytes_per_sec.unwrap_or(s.throttle.bytes_per_sec);
            s.throttle.latency_ms = p.latency_ms.unwrap_or(s.throttle.latency_ms);
        }
        self.engine.set_settings(s).map_err(|e| e.to_string())?;
        Ok(self.get_throttle())
    }

    #[tool(description = "Get the current throttle settings.")]
    fn get_throttle(&self) -> String {
        pretty(&serde_json::to_value(self.engine.settings().throttle).unwrap_or_default())
    }

    #[tool(
        description = "Create a mock: matching requests get this response without contacting the server (a Map Local rule). Returns the rule id."
    )]
    fn mock_response(&self, Parameters(p): Parameters<MockParams>) -> Result<String, String> {
        let mut rules = self.engine.rules();
        let id = format!("mcp-{:x}", crate::engine::proxy::now_ms() as u64);
        rules.map_local.push(MapLocal {
            matcher: Matcher {
                id: id.clone(),
                name: "Mock (MCP)".into(),
                method: p.method.unwrap_or_default(),
                url: p.url_pattern,
                ..Default::default()
            },
            status: p.status.unwrap_or(200),
            headers: p.headers.unwrap_or_default(),
            body: p.body.unwrap_or_default(),
            file: String::new(),
        });
        self.engine.set_rules(rules).map_err(|e| e.to_string())?;
        Ok(id)
    }

    #[tool(description = "Remove a mock created with mock_response.")]
    fn remove_mock(&self, Parameters(p): Parameters<RuleIdParams>) -> Result<String, String> {
        let mut rules = self.engine.rules();
        let before = rules.map_local.len();
        rules.map_local.retain(|r| r.matcher.id != p.rule_id);
        if rules.map_local.len() == before {
            return Err("no mock with that id".into());
        }
        self.engine.set_rules(rules).map_err(|e| e.to_string())?;
        Ok("removed".into())
    }

    #[tool(description = "List JavaScript scripts (onRequest/onResponse hooks).")]
    fn list_scripts(&self) -> String {
        pretty(&serde_json::to_value(self.engine.scripts.list()).unwrap_or_default())
    }

    #[tool(description = "Get a script's source.")]
    fn get_script(&self, Parameters(p): Parameters<ScriptParams>) -> Result<String, String> {
        self.engine
            .scripts
            .read(&p.file_name)
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create a script. `source` defines onRequest(ctx) and/or onResponse(ctx); ctx has method, url, headers, body, status, blocked, respondWith({status, headers, body}), readFile(path), breakpoint(). Returns the file name."
    )]
    fn create_script(
        &self,
        Parameters(p): Parameters<CreateScriptParams>,
    ) -> Result<String, String> {
        let source = script::template(&p.name, &p.r#match, &p.source);
        self.engine
            .scripts
            .create(&source)
            .map_err(|e| e.to_string())
    }

    #[tool(description = "Enable or disable a script.")]
    fn toggle_script(
        &self,
        Parameters(p): Parameters<ToggleScriptParams>,
    ) -> Result<String, String> {
        self.engine
            .scripts
            .set_enabled(&p.file_name, p.enabled)
            .map(|_| "ok".into())
            .map_err(|e| e.to_string())
    }

    #[tool(description = "Delete a script.")]
    fn delete_script(&self, Parameters(p): Parameters<ScriptParams>) -> Result<String, String> {
        self.engine
            .scripts
            .delete(&p.file_name)
            .map(|_| "deleted".into())
            .map_err(|e| e.to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ProxaLens", env!("CARGO_PKG_VERSION")))
            .with_instructions(
            "ProxaLens captures HTTP/HTTPS traffic on this Mac. Filters: whitespace-separated terms, ANDed; \
             prefix ! to negate. s:200 / s:4xx (status), m:post (method), ct:json (content type), \
             re:/v[12]/ (regex on host+path), app:chrome (app), any other text = host+path substring.",
        )
    }
}

/// The running server.
pub struct McpServer {
    engine: Arc<Engine>,
    token: String,
    running: tokio::sync::Mutex<Option<(SocketAddr, CancellationToken)>>,
}

impl McpServer {
    pub fn new(engine: Arc<Engine>, token: String) -> Self {
        Self {
            engine,
            token,
            running: tokio::sync::Mutex::new(None),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub async fn addr(&self) -> Option<SocketAddr> {
        self.running.lock().await.as_ref().map(|(a, _)| *a)
    }

    /// Starts on 127.0.0.1:`port` (0 = any); a no-op when already running.
    pub async fn start(&self, port: u16) -> Result<SocketAddr, String> {
        let mut running = self.running.lock().await;
        if let Some((addr, _)) = running.as_ref() {
            return Ok(*addr);
        }
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("could not listen on 127.0.0.1:{port}: {e}"))?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let cancel = CancellationToken::new();
        let mut config = StreamableHttpServerConfig::default();
        config.cancellation_token = cancel.child_token();
        let engine = self.engine.clone();
        let service: StreamableHttpService<Tools, LocalSessionManager> = StreamableHttpService::new(
            move || Ok(Tools::new(engine.clone())),
            Default::default(),
            config,
        );
        let expected = format!("Bearer {}", self.token);
        let stop = cancel.clone();
        tokio::spawn(async move {
            loop {
                let (tcp, _) = tokio::select! {
                    _ = stop.cancelled() => return,
                    accepted = listener.accept() => match accepted {
                        Ok(a) => a,
                        Err(_) => continue,
                    },
                };
                let service = service.clone();
                let expected = expected.clone();
                tokio::spawn(async move {
                    let handler = service_fn(move |req: Request<Incoming>| {
                        let mut service = service.clone();
                        let authorized = req
                            .headers()
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            == Some(expected.as_str());
                        let path_ok = req.uri().path().starts_with("/mcp");
                        async move {
                            if !authorized || !path_ok {
                                let status = if authorized {
                                    StatusCode::NOT_FOUND
                                } else {
                                    StatusCode::UNAUTHORIZED
                                };
                                let body = Full::new(Bytes::from_static(b"unauthorized"))
                                    .map_err(|e: Infallible| match e {})
                                    .boxed();
                                let mut res = Response::new(body);
                                *res.status_mut() = status;
                                return Ok::<_, Infallible>(res);
                            }
                            service.call(req).await
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(tcp), handler)
                        .await;
                });
            }
        });
        *running = Some((addr, cancel));
        Ok(addr)
    }

    pub async fn stop(&self) {
        if let Some((_, cancel)) = self.running.lock().await.take() {
            cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineConfig;
    use crate::engine::store::test_meta;

    #[test]
    fn token_is_created_once_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create_token(dir.path()).unwrap();
        let b = load_or_create_token(dir.path()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(TOKEN_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    async fn rpc(
        client: &reqwest::Client,
        url: &str,
        token: &str,
        session: Option<&str>,
        body: Json,
    ) -> (reqwest::StatusCode, Option<String>, String) {
        let mut req = client
            .post(url)
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(body.to_string());
        if let Some(s) = session {
            req = req
                .header("mcp-session-id", s)
                .header("mcp-protocol-version", "2025-03-26");
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        let sid = res
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        (status, sid, res.text().await.unwrap())
    }

    #[tokio::test]
    async fn serves_tools_over_streamable_http_with_auth() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(Engine::new(EngineConfig::new(dir.path())).unwrap());
        let mut m = test_meta("/v1/users");
        m.method = "POST".into();
        m.status = Some(201);
        engine.store.add(m, FlowData::default());
        engine.store.add(test_meta("/health"), FlowData::default());

        let server = McpServer::new(engine.clone(), "secret".into());
        let addr = server.start(0).await.unwrap();
        let url = format!("http://{addr}/mcp");
        let client = reqwest::Client::new();

        let (status, _, _) = rpc(&client, &url, "wrong", None, json!({})).await;
        assert_eq!(status, 401);

        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
        let (status, sid, body) = rpc(&client, &url, "secret", None, init).await;
        assert_eq!(status, 200, "{body}");
        assert!(
            body.contains(&format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION"))),
            "serverInfo should report the app version: {body}"
        );
        let sid = sid.expect("session id");
        rpc(
            &client,
            &url,
            "secret",
            Some(&sid),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;

        let (_, _, tools) = rpc(
            &client,
            &url,
            "secret",
            Some(&sid),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .await;
        for name in [
            "list_requests",
            "get_request",
            "export_har",
            "replay_request",
            "mock_response",
            "create_script",
        ] {
            assert!(tools.contains(name), "missing {name}: {tools}");
        }
        let call = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_requests","arguments":{"filter":"m:post"}}});
        let (_, _, out) = rpc(&client, &url, "secret", Some(&sid), call).await;
        assert!(
            out.contains("\\\"total\\\": 1") && out.contains("/v1/users"),
            "{out}"
        );

        let mock = json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mock_response","arguments":{"url_pattern":"*/mocked","body":"hi"}}});
        rpc(&client, &url, "secret", Some(&sid), mock).await;
        assert_eq!(engine.rules().map_local.len(), 1);

        server.stop().await;
    }
}
