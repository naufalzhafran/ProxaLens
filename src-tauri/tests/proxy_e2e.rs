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
use project_proxy_lib::engine::store::{FlowMeta, FlowState};
use project_proxy_lib::engine::{Engine, EngineConfig};
use tokio::net::TcpListener;

async fn route(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let body = req.into_body().collect().await.unwrap().to_bytes();
    let res = match path.as_str() {
        "/hello" => Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(br#"{"hello":"world"}"#))),
        "/echo" => Response::builder().status(201).body(Full::new(body)),
        "/big" => Response::builder().body(Full::new(Bytes::from(vec![b'x'; 10_000]))),
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
    tokio::spawn(async move { e.start(0).await.unwrap() }).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let addr = engine.addr().await.expect("proxy stopped by itself");
    tokio::net::TcpStream::connect(addr).await.unwrap();
}
