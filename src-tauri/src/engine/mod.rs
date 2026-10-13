//! The proxy engine. Independent of Tauri so it can be tested on its own.

pub mod breakpoint;
pub mod ca;
pub mod decode;
pub mod guard;
pub mod har;
pub mod proto;
pub mod proxy;
pub mod repeat;
pub mod rules;
pub mod script;
pub mod settings;
pub mod store;
pub mod sysproxy;
pub mod throttle;
pub mod tlsfail;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Arc;

use hudsucker::Proxy;
use hudsucker::rustls::crypto::aws_lc_rs;
use hudsucker::rustls::pki_types::CertificateDer;
use hudsucker::rustls::{ClientConfig, RootCertStore};
use hudsucker::tokio_tungstenite::Connector;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use breakpoint::Breakpoints;
use ca::CaBundle;
use proxy::{FlowHandler, Recorder};
use repeat::{OutgoingRequest, RepeatError};
use rules::{CompiledRules, Rules};
use script::Scripts;
use settings::Settings;
use store::{FlowData, FlowId, FlowMeta, Store};
use sysproxy::{NetworkSetup, RestoreReport, SysProxyError};

pub const DEFAULT_PORT: u16 = 9090;
pub const LOCALHOST: &str = "127.0.0.1";

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Ca(#[from] ca::CaError),
    #[error(transparent)]
    SysProxy(#[from] SysProxyError),
    #[error("could not listen on {addr}: {source}")]
    Bind {
        addr: String,
        source: std::io::Error,
    },
    #[error("proxy: {0}")]
    Proxy(#[from] hudsucker::Error),
    #[error("TLS setup: {0}")]
    Tls(String),
    #[error("the proxy is not running")]
    NotRunning,
    #[error(transparent)]
    Repeat(#[from] RepeatError),
    #[error(transparent)]
    Har(#[from] har::HarError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Rules(String),
}

pub struct EngineConfig {
    pub data_dir: PathBuf,
    pub buffer_size: usize,
    pub body_cap: usize,
    pub listen_host: String,
    /// Extra trusted roots for upstream TLS (used by tests).
    pub extra_upstream_roots: Vec<CertificateDer<'static>>,
}

impl EngineConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            buffer_size: 10_000,
            body_cap: proxy::DEFAULT_BODY_CAP,
            listen_host: LOCALHOST.into(),
            extra_upstream_roots: Vec::new(),
        }
    }
}

struct Running {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), hudsucker::Error>>,
}

pub struct Engine {
    cfg: EngineConfig,
    pub store: Arc<Store>,
    pub ca: CaBundle,
    settings: Arc<parking_lot::RwLock<Settings>>,
    rules: Arc<parking_lot::RwLock<Arc<CompiledRules>>>,
    pub breakpoints: Arc<Breakpoints>,
    pub scripts: Arc<Scripts>,
    pub proto: proto::Registries,
    running: Mutex<Option<Running>>,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Result<Self, EngineError> {
        let ca = CaBundle::load_or_create(&cfg.data_dir.join("ca"))?;
        let settings = Settings::load(&cfg.data_dir.join("settings.json"));
        let rules = Rules::load(&cfg.data_dir.join("rules.json"));
        Ok(Self {
            store: Arc::new(Store::new(cfg.buffer_size)),
            ca,
            settings: Arc::new(parking_lot::RwLock::new(settings)),
            rules: Arc::new(parking_lot::RwLock::new(Arc::new(CompiledRules::new(
                rules,
            )))),
            breakpoints: Arc::new(Breakpoints::new(breakpoint::DEFAULT_TIMEOUT)),
            scripts: Arc::new(Scripts::new(
                cfg.data_dir.join("scripts"),
                script::DEFAULT_TIMEOUT,
            )),
            proto: proto::Registries::default(),
            cfg,
            running: Mutex::new(None),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.cfg.data_dir
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    /// Applies new settings to the running proxy at once and saves them.
    pub fn set_settings(&self, settings: Settings) -> Result<(), EngineError> {
        settings.save(&self.cfg.data_dir.join("settings.json"))?;
        *self.settings.write() = settings;
        Ok(())
    }

    pub fn rules(&self) -> Rules {
        self.rules.read().rules.clone()
    }

    /// Checks, saves and applies new rules (to requests that start after).
    pub fn set_rules(&self, rules: Rules) -> Result<(), EngineError> {
        rules.validate().map_err(EngineError::Rules)?;
        rules.save(&self.cfg.data_dir.join("rules.json"))?;
        *self.rules.write() = Arc::new(CompiledRules::new(rules));
        Ok(())
    }

    /// Full copies of the given flows (all flows when `ids` is `None`).
    pub fn snapshot(&self, ids: Option<&[FlowId]>) -> Vec<(FlowMeta, FlowData)> {
        let ids: Vec<FlowId> = match ids {
            Some(ids) => ids.to_vec(),
            None => self
                .store
                .list(0, usize::MAX)
                .0
                .iter()
                .map(|m| m.id)
                .collect(),
        };
        ids.into_iter()
            .filter_map(|id| self.store.get(id))
            .collect()
    }

    /// Adds flows from a HAR or session file and returns how many. With
    /// `replace`, existing flows are cleared first, but only once the file
    /// has parsed, so a bad file doesn't wipe the list.
    pub fn import(&self, bytes: &[u8], replace: bool) -> Result<usize, EngineError> {
        let flows = har::import(har::parse(bytes)?);
        if replace {
            self.store.clear();
        }
        let n = flows.len();
        for (meta, data) in flows {
            self.store.add(meta, data);
        }
        Ok(n)
    }

    /// Sends a request through the running proxy so it is captured.
    pub async fn send(&self, req: OutgoingRequest) -> Result<Option<FlowId>, EngineError> {
        let addr = self.addr().await.ok_or(RepeatError::NotRunning)?;
        Ok(repeat::send(&self.store, addr, &self.ca.cert_der, req).await?)
    }

    /// Starts listening on `listen_host:port` (port 0 picks a free one).
    /// Returns the bound address; a no-op if already running.
    pub async fn start(&self, port: u16) -> Result<SocketAddr, EngineError> {
        let mut running = self.running.lock().await;
        if let Some(r) = running.as_ref()
            && !r.task.is_finished()
        {
            return Ok(r.addr);
        }

        let bind = format!("{}:{port}", self.cfg.listen_host);
        let listener = TcpListener::bind(&bind)
            .await
            .map_err(|source| EngineError::Bind {
                addr: bind.clone(),
                source,
            })?;
        let addr = listener
            .local_addr()
            .map_err(|source| EngineError::Bind { addr: bind, source })?;

        let tls = upstream_tls_config(&self.cfg.extra_upstream_roots)?;
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls.clone())
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();

        let handler = FlowHandler::new(Recorder::new(
            self.store.clone(),
            self.cfg.body_cap,
            self.settings.clone(),
            self.rules.clone(),
            self.breakpoints.clone(),
            self.scripts.clone(),
        ));
        let (tx, rx) = oneshot::channel::<()>();
        let proxy = Proxy::builder()
            .with_listener(listener)
            .with_ca(self.ca.authority()?)
            .with_http_connector(https)
            .with_http_handler(handler.clone())
            .with_websocket_handler(handler)
            .with_websocket_connector(Connector::Rustls(Arc::new(tls)))
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .build()?;

        let task = tokio::spawn(proxy.start());
        *running = Some(Running {
            addr,
            shutdown: tx,
            task,
        });
        Ok(addr)
    }

    pub async fn stop(&self) {
        // Paused flows would otherwise hold their connections open.
        self.breakpoints.resume_all();
        let running = self.running.lock().await.take();
        if let Some(r) = running {
            let _ = r.shutdown.send(());
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), r.task).await;
        }
    }

    pub async fn addr(&self) -> Option<SocketAddr> {
        self.running
            .lock()
            .await
            .as_ref()
            .filter(|r| !r.task.is_finished())
            .map(|r| r.addr)
    }
}

/// Upstream trust: Mozilla roots + the macOS keychain roots (so corporate
/// CAs work) + any extra roots from the config.
fn upstream_tls_config(extra: &[CertificateDer<'static>]) -> Result<ClientConfig, EngineError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let native = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(native.certs);
    roots.add_parsable_certificates(extra.iter().cloned());

    Ok(
        ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| EngineError::Tls(e.to_string()))?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// Owns the "system proxy is on" state, the marker file and the watchdog.
pub struct SysProxyController {
    ns: Box<dyn NetworkSetup>,
    marker_path: PathBuf,
    guard: parking_lot::Mutex<Option<Child>>,
}

impl SysProxyController {
    pub fn new(ns: Box<dyn NetworkSetup>, marker_path: PathBuf) -> Self {
        Self {
            ns,
            marker_path,
            guard: parking_lot::Mutex::new(None),
        }
    }

    pub fn marker_path(&self) -> &Path {
        &self.marker_path
    }

    /// True when the marker on disk belongs to this process.
    pub fn is_enabled(&self) -> bool {
        matches!(sysproxy::read_marker(&self.marker_path), Ok(Some(m)) if m.pid == std::process::id())
    }

    /// Layer 4: restore what a dead previous run left behind.
    pub fn recover_stale(&self) -> Result<Option<RestoreReport>, SysProxyError> {
        sysproxy::recover_stale(self.ns.as_ref(), &self.marker_path, sysproxy::pid_alive)
    }

    pub fn enable(&self, host: &str, port: u16) -> Result<(), SysProxyError> {
        sysproxy::enable(
            self.ns.as_ref(),
            &self.marker_path,
            host,
            port,
            proxy::now_ms(),
        )?;
        let mut guard = self.guard.lock();
        if guard.is_none() {
            match guard::spawn(&self.marker_path) {
                Ok(child) => *guard = Some(child),
                Err(e) => tracing::warn!("could not start proxy watchdog: {e}"),
            }
        }
        Ok(())
    }

    /// Restores the user's settings (if we changed them) and stops the
    /// watchdog. Safe to call repeatedly and from any exit path.
    pub fn disable(&self) -> Result<Option<RestoreReport>, SysProxyError> {
        let report = match sysproxy::read_marker(&self.marker_path)? {
            Some(m) if m.pid == std::process::id() => {
                Some(sysproxy::restore(self.ns.as_ref(), &self.marker_path, &m)?)
            }
            _ => None,
        };
        if let Some(mut child) = self.guard.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(report)
    }
}

/// Last-resort restore for panic hooks and signal handlers: reads the marker
/// straight from disk and avoids any locks that might be held.
pub fn emergency_restore(marker_path: &Path) {
    if let Ok(Some(m)) = sysproxy::read_marker(marker_path)
        && m.pid == std::process::id()
    {
        let _ = sysproxy::restore(&sysproxy::RealNetworkSetup, marker_path, &m);
    }
}
