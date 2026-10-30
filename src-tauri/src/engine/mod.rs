//! The proxy engine. Independent of Tauri so it can be tested on its own.

pub mod ca;
pub mod decode;
pub mod guard;
pub mod har;
pub mod procinfo;
pub mod proxy;
pub mod repeat;
pub mod rules;
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

use ca::CaBundle;
use proxy::{FlowHandler, Recorder};
use repeat::{OutgoingRequest, RepeatError};
use rules::{CompiledRules, Rules};
use settings::Settings;
use store::{FlowData, FlowId, FlowMeta, Store};
use sysproxy::{NetworkSetup, RestoreReport, SysProxyError};

pub const DEFAULT_PORT: u16 = 9090;

/// The data folder's name before the app was renamed to ProxaLens.
pub const OLD_DATA_DIR: &str = "dev.naufalzhafran.projectproxy";

/// Moves the data folder from before the rename (CA, settings, rules,
/// scripts and the crash-recovery marker) to `data_dir`, so the trusted CA
/// and a pending proxy restore carry over. Does nothing once `data_dir`
/// exists. Returns whether it moved.
pub fn migrate_data_dir(data_dir: &Path) -> std::io::Result<bool> {
    let Some(old) = data_dir.parent().map(|p| p.join(OLD_DATA_DIR)) else {
        return Ok(false);
    };
    if data_dir.exists() || !old.is_dir() {
        return Ok(false);
    }
    std::fs::rename(&old, data_dir)?;
    Ok(true)
}
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
    pub processes: Arc<procinfo::Resolver>,
    running: Mutex<Option<Running>>,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Result<Self, EngineError> {
        let ca = CaBundle::load_or_create(&cfg.data_dir.join("ca"))?;
        let settings = Settings::load(&cfg.data_dir.join("settings.json"));
        let rules = Rules::load(&cfg.data_dir.join("rules.json"));
        let store = Arc::new(Store::new(cfg.buffer_size));
        Ok(Self {
            store: store.clone(),
            ca,
            settings: Arc::new(parking_lot::RwLock::new(settings)),
            rules: Arc::new(parking_lot::RwLock::new(Arc::new(CompiledRules::new(
                rules,
            )))),
            processes: procinfo::Resolver::new(Arc::new(procinfo::Darwin), store.clone()),
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

        // Devices on the network need the proxy on every interface.
        let host = if self.settings.read().allow_lan {
            "0.0.0.0"
        } else {
            self.cfg.listen_host.as_str()
        };
        let bind = format!("{host}:{port}");
        let listener = TcpListener::bind(&bind)
            .await
            .map_err(|source| EngineError::Bind {
                addr: bind.clone(),
                source,
            })?;
        let addr = listener
            .local_addr()
            .map_err(|source| EngineError::Bind { addr: bind, source })?;
        self.processes.set_proxy_port(addr.port());

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
            self.processes.clone(),
            proxy::LocalPages {
                ca_pem: self.ca.cert_pem.clone(),
                ca_der: self.ca.cert_der.clone(),
                port: addr.port(),
            },
        ));
        let (tx, rx) = oneshot::channel::<()>();
        let proxy = Proxy::builder()
            .with_listener(listener)
            .with_ca(self.ca.authority()?)
            .with_http_connector(https)
            .with_client(upstream_client())
            .with_server(proxy_server())
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

/// Largest HTTP/2 header block accepted, matching Chrome. hyper's default
/// is 16 KiB, and some sites (Gemini sends about 26 KiB of response headers)
/// go over it, which h2 reports as "unspecific protocol error".
const MAX_H2_HEADER_LIST: u32 = 256 * 1024;

/// hudsucker's default upstream client, with room for large HTTP/2 headers.
fn upstream_client() -> hudsucker::hyper_util::client::legacy::Builder {
    use hudsucker::hyper_util::client::legacy::Client;
    use hudsucker::hyper_util::rt::TokioExecutor;
    let mut b = Client::builder(TokioExecutor::new());
    b.http1_title_case_headers(true)
        .http1_preserve_header_case(true)
        .http2_max_header_list_size(MAX_H2_HEADER_LIST);
    b
}

/// hudsucker's default server for clients, with room for large HTTP/2 headers.
fn proxy_server()
-> hudsucker::hyper_util::server::conn::auto::Builder<hudsucker::hyper_util::rt::TokioExecutor> {
    use hudsucker::hyper_util::rt::TokioExecutor;
    use hudsucker::hyper_util::server::conn::auto::Builder;
    let mut b = Builder::new(TokioExecutor::new());
    b.http1()
        .title_case_headers(true)
        .preserve_header_case(true);
    b.http2().max_header_list_size(MAX_H2_HEADER_LIST);
    b
}

/// IPv4 addresses of this Mac on its networks (for device setup).
pub fn lan_addresses() -> Vec<(String, std::net::Ipv4Addr)> {
    let mut out = Vec::new();
    let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list we walk read-only and free.
    unsafe {
        if libc::getifaddrs(&mut addrs) != 0 {
            return out;
        }
        let mut cur = addrs;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null()
                && i32::from((*ifa.ifa_addr).sa_family) == libc::AF_INET
                && ifa.ifa_flags & (libc::IFF_UP as u32) != 0
                && ifa.ifa_flags & (libc::IFF_LOOPBACK as u32) == 0
            {
                let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                    .to_string_lossy()
                    .into_owned();
                if !ip.is_link_local() {
                    out.push((name, ip));
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(addrs);
    }
    out
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

    /// Layer 4: restore what a dead previous run left behind, including a
    /// proxy still pointing at `port` when the marker itself was lost.
    pub fn recover_stale(&self, port: u16) -> Result<Option<RestoreReport>, SysProxyError> {
        sysproxy::recover(
            self.ns.as_ref(),
            &self.marker_path,
            LOCALHOST,
            port,
            sysproxy::pid_alive,
        )
    }

    pub fn enable(&self, host: &str, port: u16, mode: sysproxy::Mode) -> Result<(), SysProxyError> {
        sysproxy::enable_mode(
            self.ns.as_ref(),
            &self.marker_path,
            host,
            port,
            proxy::now_ms(),
            mode,
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
    ///
    /// Without a marker, anything still pointing at `port` is switched off:
    /// the system must never be left routed through a proxy that is gone.
    /// A marker owned by another process is left to that process.
    pub fn disable(&self, port: u16) -> Result<Option<RestoreReport>, SysProxyError> {
        let report = match sysproxy::read_marker(&self.marker_path)? {
            Some(m) if m.pid == std::process::id() => {
                Some(sysproxy::restore(self.ns.as_ref(), &self.marker_path, &m)?)
            }
            Some(_) => None,
            None => {
                let restored = sysproxy::clear_dangling(self.ns.as_ref(), LOCALHOST, port)?;
                (!restored.is_empty()).then(|| RestoreReport {
                    restored,
                    skipped: Vec::new(),
                })
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sysproxy::ProxySetting;
    use sysproxy::tests::FakeNs;

    /// Lets a test keep a handle on the fake the controller owns.
    struct Shared(Arc<FakeNs>);
    impl NetworkSetup for Shared {
        fn run(&self, args: &[String]) -> Result<String, SysProxyError> {
            self.0.run(args)
        }
        fn run_admin(&self, cmds: &[Vec<String>]) -> Result<(), SysProxyError> {
            self.0.run_admin(cmds)
        }
    }

    fn ours() -> ProxySetting {
        ProxySetting {
            enabled: true,
            server: LOCALHOST.into(),
            port: 9090,
        }
    }

    #[test]
    fn controller_round_trips_and_clears_a_dangling_proxy() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let fake = Arc::new(FakeNs::new(&["Wi-Fi"]));
        let ctl = SysProxyController::new(Box::new(Shared(fake.clone())), marker.clone());

        ctl.enable(LOCALHOST, 9090, sysproxy::Mode::Manual).unwrap();
        assert!(ctl.is_enabled());
        assert_eq!(fake.get("Wi-Fi", "http"), ours());
        let report = ctl.disable(9090).unwrap().unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!ctl.is_enabled());
        assert!(!fake.get("Wi-Fi", "http").enabled);

        // No marker, but the system still points at us (a lost marker).
        fake.set("Wi-Fi", "https", ours());
        assert!(!ctl.is_enabled());
        let report = ctl.disable(9090).unwrap().unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!fake.get("Wi-Fi", "https").enabled);
        assert!(ctl.disable(9090).unwrap().is_none());

        // Same at launch.
        fake.set("Wi-Fi", "http", ours());
        let report = ctl.recover_stale(9090).unwrap().unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!fake.get("Wi-Fi", "http").enabled);
    }

    #[test]
    fn moves_the_old_data_folder_once() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join(OLD_DATA_DIR);
        let new = root.path().join("dev.naufalzhafran.proxalens");
        std::fs::create_dir_all(old.join("ca")).unwrap();
        std::fs::write(old.join("ca/ca-cert.pem"), "cert").unwrap();

        assert!(migrate_data_dir(&new).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.join("ca/ca-cert.pem")).unwrap(),
            "cert"
        );
        assert!(!old.exists());

        // Nothing to move any more, and an existing folder is never replaced.
        assert!(!migrate_data_dir(&new).unwrap());
        std::fs::create_dir_all(&old).unwrap();
        assert!(!migrate_data_dir(&new).unwrap());
        assert!(old.exists());
    }

    #[test]
    fn leaves_a_fresh_install_alone() {
        let root = tempfile::tempdir().unwrap();
        let new = root.path().join("dev.naufalzhafran.proxalens");
        assert!(!migrate_data_dir(&new).unwrap());
        assert!(!new.exists());
    }
}
