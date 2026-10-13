//! End-to-end: real client → engine → local upstream (HTTP and HTTPS).
//! Mirrors httpmon's `internal/proxy/*_e2e_test.go`.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hudsucker::hyper::{
    Request, Response, body::Incoming, server::conn::http1, service::service_fn,
};
use hudsucker::hyper_util::rt::TokioIo;
use hudsucker::rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use hudsucker::rustls::ServerConfig;
use hudsucker::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use project_proxy_lib::engine::breakpoint::{Edits, Phase, Resume};
use project_proxy_lib::engine::repeat::OutgoingRequest;
use project_proxy_lib::engine::rules::{
    BlockRule, BreakpointRule, MapLocal, MapRemote, Matcher, Rewrite, RewriteAction, RewriteTarget,
    Rules,
};
use project_proxy_lib::engine::settings::{HostList, Settings};
use project_proxy_lib::engine::store::{FlowMeta, FlowState};
use project_proxy_lib::engine::{Engine, EngineConfig, decode};
use tokio::net::TcpListener;

async fn route(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let auth = req
        .headers()
        .get("authorization")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = req.into_body().collect().await.unwrap().to_bytes();
    let res = match path.as_str() {
        "/hello" => Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(br#"{"hello":"world"}"#))),
        "/echo" => Response::builder().status(201).body(Full::new(body)),
        "/auth" => Response::builder().body(Full::new(Bytes::from(auth))),
        "/big" => Response::builder().body(Full::new(Bytes::from(vec![b'x'; 10_000]))),
        "/gzip" => {
            use std::io::Write;
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(b"decoded text").unwrap();
            Response::builder()
                .header("content-encoding", "gzip")
                .header("content-type", "text/plain")
                .body(Full::new(Bytes::from(e.finish().unwrap())))
        }
        _ => Response::builder()
            .status(404)
            .body(Full::new(Bytes::new())),
    };
    Ok(res.unwrap())
}

async fn http_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(
                http1::Builder::new().serve_connection(TokioIo::new(tcp), service_fn(route)),
            );
        }
    });
    addr
}

/// An HTTPS upstream for "localhost", signed by a throwaway test root.
async fn https_upstream() -> (SocketAddr, CertificateDer<'static>) {
    let root_key = KeyPair::generate().unwrap();
    let mut root = CertificateParams::default();
    root.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let root_cert = root.self_signed(&root_key).unwrap();
    let root_der = root_cert.der().clone();
    let issuer = Issuer::from_ca_cert_der(&root_der, root_key).unwrap();

    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["localhost".into()]).unwrap();
    let leaf_cert = leaf.signed_by(&leaf_key, &issuer).unwrap();

    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf_cert.der().clone()],
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        )
        .unwrap();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service_fn(route))
                        .await;
                }
            });
        }
    });
    (addr, root_der)
}

struct Harness {
    engine: Engine,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn harness(extra_roots: Vec<CertificateDer<'static>>, body_cap: Option<usize>) -> Harness {
    let _ = hudsucker::rustls::crypto::aws_lc_rs::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = EngineConfig::new(dir.path());
    cfg.extra_upstream_roots = extra_roots;
    if let Some(cap) = body_cap {
        cfg.body_cap = cap;
    }
    let engine = Engine::new(cfg).unwrap();
    let addr = engine.start(0).await.unwrap();

    let ca = reqwest::Certificate::from_pem(engine.ca.cert_pem.as_bytes()).unwrap();
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://{addr}")).unwrap())
        .tls_certs_only([ca])
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    Harness {
        engine,
        client,
        _dir: dir,
    }
}

impl Harness {
    /// Waits until a flow for `path` reaches a final state.
    async fn wait_done(&self, path: &str) -> FlowMeta {
        for _ in 0..100 {
            let (flows, _) = self.engine.store.list(0, 1000);
            if let Some(f) = flows
                .into_iter()
                .find(|f| f.path == path && f.state != FlowState::InProgress)
            {
                return f;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("flow for {path} never finished");
    }
}

#[tokio::test]
async fn captures_plain_http_request_and_response() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;

    let res = h
        .client
        .get(format!("http://{up}/hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.text().await.unwrap(), r#"{"hello":"world"}"#);

    let f = h.wait_done("/hello").await;
    assert_eq!(f.method, "GET");
    assert_eq!(f.scheme, "http");
    assert_eq!(f.status, Some(200));
    assert_eq!(f.state, FlowState::Completed);
    assert_eq!(f.content_type.as_deref(), Some("application/json"));
    assert_eq!(f.response_size, 17);
    assert!(f.duration_ms.is_some() && f.ttfb_ms.is_some());

    let (_, data) = h.engine.store.get(f.id).unwrap();
    assert_eq!(data.response_body, br#"{"hello":"world"}"#);
    assert!(
        data.response_headers
            .iter()
            .any(|(k, _)| k == "content-type")
    );
}

#[tokio::test]
async fn captures_request_body() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;

    let res = h
        .client
        .post(format!("http://{up}/echo"))
        .body("ping-body")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);
    assert_eq!(res.text().await.unwrap(), "ping-body");

    let f = h.wait_done("/echo").await;
    assert_eq!(f.method, "POST");
    assert_eq!(f.request_size, 9);
    let (_, data) = h.engine.store.get(f.id).unwrap();
    assert_eq!(data.request_body, b"ping-body");
    assert_eq!(data.response_body, b"ping-body");
}

#[tokio::test]
async fn intercepts_https_with_generated_certificates() {
    let (up, root) = https_upstream().await;
    let h = harness(vec![root], None).await;

    let url = format!("https://localhost:{}/hello", up.port());
    let res = h.client.get(&url).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.text().await.unwrap(), r#"{"hello":"world"}"#);

    let f = h.wait_done("/hello").await;
    assert_eq!(f.scheme, "https");
    assert_eq!(f.host, "localhost");
    assert_eq!(f.status, Some(200));
    let (_, data) = h.engine.store.get(f.id).unwrap();
    assert_eq!(data.response_body, br#"{"hello":"world"}"#);
    // The CONNECT tunnel itself is not listed as a flow.
    assert!(
        h.engine
            .store
            .list(0, 100)
            .0
            .iter()
            .all(|f| f.method != "CONNECT")
    );
}

#[tokio::test]
async fn truncates_large_bodies_but_forwards_them_whole() {
    let up = http_upstream().await;
    let h = harness(vec![], Some(1024)).await;

    let body = h
        .client
        .get(format!("http://{up}/big"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body.len(), 10_000);

    let f = h.wait_done("/big").await;
    assert_eq!(f.response_size, 10_000);
    let (_, data) = h.engine.store.get(f.id).unwrap();
    assert_eq!(data.response_body.len(), 1024);
    assert!(data.response_body_truncated);
}

#[tokio::test]
async fn records_upstream_failures() {
    // Grab a free port, then close it so the connection is refused.
    let dead = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let h = harness(vec![], None).await;

    let res = h
        .client
        .get(format!("http://{dead}/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 502);

    let f = h.wait_done("/nope").await;
    assert_eq!(f.state, FlowState::Failed);
    assert!(f.error.is_some());
}

#[tokio::test]
async fn stop_releases_the_port() {
    let h = harness(vec![], None).await;
    let addr = h.engine.addr().await.unwrap();
    h.engine.stop().await;
    assert!(h.engine.addr().await.is_none());
    // Port can be bound again once stopped.
    TcpListener::bind(addr).await.unwrap();
}

#[tokio::test]
async fn keeps_running_when_started_from_a_spawned_task() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::new(EngineConfig::new(dir.path())).unwrap());
    let e = engine.clone();
    tokio::spawn(async move { e.start(0).await.unwrap() })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let addr = engine.addr().await.expect("proxy stopped by itself");
    tokio::net::TcpStream::connect(addr).await.unwrap();
}

fn host_list(hosts: &[&str]) -> HostList {
    HostList {
        enabled: true,
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
    }
}

#[tokio::test]
async fn stores_encoded_bodies_and_decodes_them_on_demand() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    let res = h
        .client
        .get(format!("http://{up}/gzip"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.headers()["content-encoding"], "gzip");

    let f = h.wait_done("/gzip").await;
    let (_, data) = h.engine.store.get(f.id).unwrap();
    assert_ne!(data.response_body, b"decoded text", "stored as sent");
    let d = decode::decode(&data.response_body, Some("gzip"))
        .unwrap()
        .unwrap();
    assert_eq!(d.bytes, b"decoded text");
}

#[tokio::test]
async fn block_list_refuses_http_and_https() {
    let up = http_upstream().await;
    let (https, root) = https_upstream().await;
    let h = harness(vec![root], None).await;
    h.engine
        .set_settings(Settings {
            block_list: host_list(&["127.0.0.1", "localhost"]),
            ..Default::default()
        })
        .unwrap();

    let res = h
        .client
        .get(format!("http://{up}/hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let f = h.wait_done("/hello").await;
    assert_eq!((f.state, f.status), (FlowState::Blocked, Some(403)));

    let url = format!("https://localhost:{}/hello", https.port());
    assert!(
        h.client.get(&url).send().await.is_err(),
        "CONNECT is refused"
    );
    let flows = h.engine.store.list(0, 100).0;
    let connect = flows.iter().find(|f| f.method == "CONNECT").unwrap();
    assert_eq!(
        (connect.state, connect.host.as_str()),
        (FlowState::Blocked, "localhost")
    );
}

#[tokio::test]
async fn allow_list_only_records_matching_hosts() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    h.engine
        .set_settings(Settings {
            allow_list: host_list(&["*.mine.dev"]),
            ..Default::default()
        })
        .unwrap();
    let res = h
        .client
        .get(format!("http://{up}/hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "traffic still flows");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.engine.store.is_empty());
}

#[tokio::test]
async fn ssl_proxying_exclude_tunnels_without_decrypting() {
    let (up, root) = https_upstream().await;
    let h = harness(vec![], None).await;
    let mut settings = Settings::default();
    settings.ssl_proxying.exclude = vec!["localhost".into()];
    h.engine.set_settings(settings).unwrap();

    // The client must now trust the upstream's own certificate.
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://{}", h.engine.addr().await.unwrap())).unwrap())
        .tls_certs_only([reqwest::Certificate::from_der(&root).unwrap()])
        .build()
        .unwrap();
    let url = format!("https://localhost:{}/hello", up.port());
    let res = client.get(&url).send().await.unwrap();
    assert_eq!(res.text().await.unwrap(), r#"{"hello":"world"}"#);

    let flows = h.engine.store.list(0, 100).0;
    assert_eq!(flows.len(), 1);
    assert_eq!(
        (flows[0].method.as_str(), flows[0].state),
        ("CONNECT", FlowState::Tunnel)
    );
}

#[tokio::test]
async fn throttle_slows_responses() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    let mut settings = Settings::default();
    settings.throttle.enabled = true;
    settings.throttle.bytes_per_sec = 20_000;
    settings.throttle.latency_ms = 100;
    h.engine.set_settings(settings).unwrap();

    let start = std::time::Instant::now();
    let body = h
        .client
        .get(format!("http://{up}/big"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let took = start.elapsed();
    assert_eq!(body.len(), 10_000);
    // 100 ms latency + 10 KB at 20 KB/s (the first 2 KB chunk is immediate).
    assert!(took >= Duration::from_millis(450), "took {took:?}");
}

#[tokio::test]
async fn repeat_sends_the_request_again_as_a_new_flow() {
    let (up, root) = https_upstream().await;
    let h = harness(vec![root], None).await;
    let url = format!("https://localhost:{}/echo", up.port());
    h.client
        .post(&url)
        .header("x-test", "1")
        .body("again")
        .send()
        .await
        .unwrap();
    let first = h.wait_done("/echo").await;

    let (meta, data) = h.engine.store.get(first.id).unwrap();
    let id = h
        .engine
        .send(OutgoingRequest::from_flow(&meta, &data))
        .await
        .unwrap()
        .expect("new flow identified");
    assert_ne!(id, first.id);
    let (m2, d2) = h.engine.store.get(id).unwrap();
    assert_eq!((m2.method.as_str(), m2.status), ("POST", Some(201)));
    assert_eq!(d2.request_body, b"again");
    assert!(
        d2.request_headers
            .iter()
            .any(|(k, v)| k == "x-test" && v == "1")
    );
}

#[tokio::test]
async fn send_requires_a_running_proxy_and_a_valid_url() {
    let h = harness(vec![], None).await;
    let bad = OutgoingRequest {
        method: "GET".into(),
        url: "ftp://x".into(),
        headers: vec![],
        body: vec![],
    };
    assert!(h.engine.send(bad.clone()).await.is_err());
    h.engine.stop().await;
    let err = h
        .engine
        .send(OutgoingRequest {
            url: "http://x.com".into(),
            ..bad
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("start the proxy"));
}

fn matching(url: &str) -> Matcher {
    Matcher {
        url: url.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn https_urls_drop_the_default_port() {
    let (up, root) = https_upstream().await;
    let h = harness(vec![root], None).await;
    h.client
        .get(format!("https://localhost:{}/hello", up.port()))
        .send()
        .await
        .unwrap();
    let f = h.wait_done("/hello").await;
    assert_eq!(f.url, format!("https://localhost:{}/hello", up.port()));
    assert!(!f.url.contains(":443"));
}

#[tokio::test]
async fn map_local_answers_without_the_server() {
    let h = harness(vec![], None).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mock.json");
    std::fs::write(&file, r#"{"mocked":true}"#).unwrap();
    h.engine
        .set_rules(Rules {
            map_local: vec![
                MapLocal {
                    matcher: matching("http://nowhere.invalid/inline"),
                    status: 418,
                    headers: vec![("X-Mock".into(), "yes".into())],
                    body: "short and stout".into(),
                    file: String::new(),
                },
                MapLocal {
                    matcher: matching("nowhere.invalid/file*"),
                    status: 200,
                    file: file.to_string_lossy().into_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    let res = h
        .client
        .get("http://nowhere.invalid/inline")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 418);
    assert_eq!(res.headers()["x-mock"], "yes");
    assert_eq!(res.text().await.unwrap(), "short and stout");

    let res = h
        .client
        .get("http://nowhere.invalid/file?x=1")
        .send()
        .await
        .unwrap();
    assert_eq!(res.headers()["content-type"], "application/json");
    assert_eq!(res.text().await.unwrap(), r#"{"mocked":true}"#);

    let f = h.wait_done("/inline").await;
    assert_eq!((f.status, f.state), (Some(418), FlowState::Completed));
    assert_eq!(f.tags, vec!["Map Local"]);
    let (_, d) = h.engine.store.get(f.id).unwrap();
    assert_eq!(d.response_body, b"short and stout");
}

#[tokio::test]
async fn map_remote_sends_the_request_elsewhere() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    h.engine
        .set_rules(Rules {
            map_remote: vec![MapRemote {
                matcher: matching("api.example.invalid"),
                to: format!("http://{up}"),
            }],
            ..Default::default()
        })
        .unwrap();
    let res = h
        .client
        .get("http://api.example.invalid/hello")
        .send()
        .await
        .unwrap();
    assert_eq!(res.text().await.unwrap(), r#"{"hello":"world"}"#);
    let f = h.wait_done("/hello").await;
    assert_eq!(
        f.host, "api.example.invalid",
        "the flow keeps the original URL"
    );
    assert_eq!(f.tags, vec![format!("Map Remote → http://{up}/hello")]);
}

#[tokio::test]
async fn rewrite_edits_requests_and_decoded_responses() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    let rw = |target, action, header: &str, find: &str, value: &str| Rewrite {
        matcher: matching("*"),
        target,
        action,
        header: header.into(),
        find: find.into(),
        value: value.into(),
    };
    h.engine
        .set_rules(Rules {
            rewrite: vec![
                rw(
                    RewriteTarget::RequestHeader,
                    RewriteAction::Set,
                    "Authorization",
                    "",
                    "Bearer test",
                ),
                rw(
                    RewriteTarget::ResponseBody,
                    RewriteAction::Replace,
                    "",
                    "decoded",
                    "rewritten",
                ),
                rw(
                    RewriteTarget::ResponseHeader,
                    RewriteAction::Add,
                    "X-Rewritten",
                    "",
                    "1",
                ),
                rw(
                    RewriteTarget::ResponseStatus,
                    RewriteAction::Set,
                    "",
                    "",
                    "299",
                ),
            ],
            ..Default::default()
        })
        .unwrap();

    let res = h
        .client
        .get(format!("http://{up}/auth"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.text().await.unwrap(), "Bearer test");

    // The gzip body is decoded, edited and sent uncompressed.
    let res = h
        .client
        .get(format!("http://{up}/gzip"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 299);
    assert!(res.headers().get("content-encoding").is_none());
    assert_eq!(res.headers()["x-rewritten"], "1");
    assert_eq!(res.text().await.unwrap(), "rewritten text");
    let f = h.wait_done("/gzip").await;
    assert_eq!(f.tags, vec!["Rewrite"]);
}

#[tokio::test]
async fn block_rule_matches_urls() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    h.engine
        .set_rules(Rules {
            block: vec![BlockRule {
                matcher: matching("*/big"),
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        h.client
            .get(format!("http://{up}/big"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        h.client
            .get(format!("http://{up}/hello"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(h.wait_done("/big").await.state, FlowState::Blocked);
}

async fn wait_for_hit(h: &Harness, phase: Phase) -> project_proxy_lib::engine::breakpoint::Hit {
    for _ in 0..200 {
        if let Some(hit) = h
            .engine
            .breakpoints
            .pending()
            .into_iter()
            .find(|x| x.phase == phase)
        {
            return hit;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no breakpoint hit");
}

#[tokio::test]
async fn breakpoints_edit_requests_and_abort_responses() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    h.engine
        .set_rules(Rules {
            breakpoints: vec![
                BreakpointRule {
                    matcher: matching("*/echo"),
                    request: true,
                    response: false,
                },
                BreakpointRule {
                    matcher: matching("*/hello"),
                    request: false,
                    response: true,
                },
            ],
            ..Default::default()
        })
        .unwrap();

    // Request phase: change the body before it reaches the server.
    let client = h.client.clone();
    let url = format!("http://{up}/echo");
    let sent = tokio::spawn(async move {
        client
            .post(url)
            .body("original")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });
    let hit = wait_for_hit(&h, Phase::Request).await;
    assert_eq!(hit.body, b"original");
    let paused = h.engine.store.get(hit.flow_id).unwrap().0;
    assert_eq!(paused.state, FlowState::Paused);
    let edits = Edits {
        headers: hit.headers.clone(),
        body: b"edited at breakpoint".to_vec(),
        ..Default::default()
    };
    assert!(h.engine.breakpoints.resume(
        hit.flow_id,
        Phase::Request,
        Resume::Continue(Some(edits))
    ));
    assert_eq!(sent.await.unwrap(), "edited at breakpoint");
    let f = h.wait_done("/echo").await;
    assert_eq!(f.tags, vec!["Breakpoint"]);

    // Response phase: abort.
    let client = h.client.clone();
    let url = format!("http://{up}/hello");
    let got = tokio::spawn(async move { client.get(url).send().await.unwrap().status() });
    let hit = wait_for_hit(&h, Phase::Response).await;
    assert_eq!(hit.status, Some(200));
    assert_eq!(hit.body, br#"{"hello":"world"}"#);
    h.engine
        .breakpoints
        .resume(hit.flow_id, Phase::Response, Resume::Abort);
    assert_eq!(got.await.unwrap(), 502);
    let f = h.wait_done("/hello").await;
    assert_eq!(f.state, FlowState::Failed);
}

#[tokio::test]
async fn invalid_rules_are_refused() {
    let h = harness(vec![], None).await;
    let bad = Rules {
        map_remote: vec![MapRemote {
            matcher: matching("*"),
            to: "no-scheme".into(),
        }],
        ..Default::default()
    };
    assert!(h.engine.set_rules(bad).is_err());
    assert_eq!(h.engine.rules(), Rules::default());
}

#[tokio::test]
async fn scripts_change_traffic_and_can_answer_requests() {
    let up = http_upstream().await;
    let h = harness(vec![], None).await;
    let scripts = &h.engine.scripts;
    scripts
        .create(&project_proxy_lib::engine::script::template(
            "Auth and plan",
            "*",
            r#"function onRequest(ctx) {
                 ctx.headers["Authorization"] = "Bearer from-script";
               }
               function onResponse(ctx) {
                 if (ctx.url.endsWith("/gzip")) ctx.body = ctx.body.toUpperCase();
               }"#,
        ))
        .unwrap();
    scripts
        .create(&project_proxy_lib::engine::script::template(
            "Mock",
            "*://mock.invalid/*",
            r#"function onRequest(ctx) { ctx.respondWith({ status: 202, body: { mocked: ctx.method } }); }"#,
        ))
        .unwrap();

    let res = h
        .client
        .get(format!("http://{up}/auth"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.text().await.unwrap(), "Bearer from-script");
    let res = h
        .client
        .get(format!("http://{up}/gzip"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.text().await.unwrap(),
        "DECODED TEXT",
        "scripts see decoded bodies"
    );
    let res = h.client.get("http://mock.invalid/x").send().await.unwrap();
    assert_eq!(res.status(), 202);
    assert_eq!(res.text().await.unwrap(), r#"{"mocked":"GET"}"#);
    assert_eq!(h.wait_done("/x").await.tags, vec!["Script"]);
    assert!(scripts.console().is_empty(), "{:?}", scripts.console());
}

#[tokio::test]
async fn captures_websocket_messages_both_ways() {
    use hudsucker::futures::{SinkExt, StreamExt};
    use hudsucker::tokio_tungstenite::{accept_async, client_async, tungstenite::Message};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Echo server.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(tcp).await.unwrap();
        while let Some(Ok(msg)) = ws.next().await {
            if msg.is_text() {
                let reply = format!("echo: {}", msg.to_text().unwrap());
                ws.send(Message::text(reply)).await.unwrap();
            }
        }
    });

    let h = harness(vec![], None).await;
    // Open a tunnel through the proxy, then speak WebSocket inside it.
    let mut tcp = tokio::net::TcpStream::connect(h.engine.addr().await.unwrap())
        .await
        .unwrap();
    tcp.write_all(format!("CONNECT {ws_addr} HTTP/1.1\r\nHost: {ws_addr}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = [0u8; 256];
    let n = tcp.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
    let (mut ws, _) = client_async(format!("ws://{ws_addr}/chat"), tcp)
        .await
        .unwrap();
    ws.send(Message::text("hello")).await.unwrap();
    let reply = ws.next().await.unwrap().unwrap();
    assert_eq!(reply.to_text().unwrap(), "echo: hello");
    ws.close(None).await.unwrap();

    let mut flow = None;
    for _ in 0..100 {
        let flows = h.engine.store.list(0, 10).0;
        if let Some(f) = flows
            .into_iter()
            .find(|f| f.path == "/chat" && f.ws_message_count >= 2)
        {
            flow = Some(f);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let f = flow.expect("websocket flow recorded");
    assert_eq!((f.state, f.status), (FlowState::WebSocket, Some(101)));
    let (_, d) = h.engine.store.get(f.id).unwrap();
    let texts: Vec<_> = d
        .ws_messages
        .iter()
        .filter(|m| m.kind == "text")
        .map(|m| {
            (
                m.direction,
                String::from_utf8_lossy(&m.payload).into_owned(),
            )
        })
        .collect();
    use project_proxy_lib::engine::store::WsDirection::*;
    assert_eq!(
        texts,
        vec![
            (ClientToServer, "hello".into()),
            (ServerToClient, "echo: hello".into())
        ]
    );
}

#[tokio::test]
async fn serves_setup_page_certificate_and_pac() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let h = harness(vec![], None).await;
    let addr = h.engine.addr().await.unwrap();

    // Straight to the proxy, as a phone on the network would.
    let direct = |path: &'static str| async move {
        let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        tcp.write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: 192.168.1.5:{}\r\nConnection: close\r\n\r\n",
                addr.port()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut out = String::new();
        tcp.read_to_string(&mut out).await.unwrap();
        out
    };
    let page = direct("/").await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("192.168.1.5:"));
    let pac = direct("/proxy.pac").await;
    assert!(
        pac.contains(&format!("PROXY 192.168.1.5:{}; DIRECT", addr.port())),
        "{pac}"
    );
    assert!(direct("/nope").await.starts_with("HTTP/1.1 404"));

    // Through the proxy, by name.
    let cert = h
        .client
        .get("http://proxy.local/cert.pem")
        .send()
        .await
        .unwrap();
    assert_eq!(cert.headers()["content-type"], "application/x-x509-ca-cert");
    assert_eq!(cert.text().await.unwrap(), h.engine.ca.cert_pem);
    assert!(
        h.engine.store.is_empty(),
        "the proxy's own pages aren't recorded"
    );
}

#[tokio::test]
async fn allow_lan_listens_on_every_interface() {
    let h = harness(vec![], None).await;
    assert!(h.engine.addr().await.unwrap().ip().is_loopback());
    let mut settings = h.engine.settings();
    settings.allow_lan = true;
    h.engine.set_settings(settings).unwrap();
    h.engine.stop().await;
    let addr = h.engine.start(0).await.unwrap();
    assert!(addr.ip().is_unspecified(), "{addr}");
}
