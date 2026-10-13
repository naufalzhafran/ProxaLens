//! Which app made a request: client port → PID → app name. A port of
//! httpmon's `internal/procinfo`, using libproc instead of gopsutil.
//!
//! One scan of every process's sockets resolves all connections waiting at
//! that moment, so a burst of new connections costs one scan, not one each.
//! Results are cached per client port for a short while (ports get reused).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::store::{FlowId, Store};

const CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    /// The app's name ("Google Chrome"), or the executable's.
    pub name: String,
}

/// OS access, behind a trait so tests can fake it.
pub trait Lookup: Send + Sync + 'static {
    /// Local port → PID for TCP sockets connected to `remote_port`.
    fn connections_to(&self, remote_port: u16) -> HashMap<u16, u32>;
    fn describe(&self, pid: u32) -> Option<Process>;
}

/// The app name for an executable path: the outermost `.app` bundle
/// (so Chrome's helper processes count as "Google Chrome").
pub fn app_name(path: &str) -> Option<String> {
    path.split('/')
        .find_map(|c| c.strip_suffix(".app"))
        .map(str::to_string)
        .or_else(|| {
            path.rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

pub struct Darwin;

impl Lookup for Darwin {
    fn connections_to(&self, remote_port: u16) -> HashMap<u16, u32> {
        use libproc::libproc::bsd_info::BSDInfo;
        use libproc::libproc::file_info::{ListFDs, ProcFDType, pidfdinfo};
        use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind};
        use libproc::libproc::proc_pid::{listpidinfo, pidinfo};
        use libproc::processes::{ProcFilter, pids_by_type};

        let me = std::process::id();
        let mut found = HashMap::new();
        let Ok(pids) = pids_by_type(ProcFilter::All) else {
            return found;
        };
        for pid in pids.into_iter().filter(|p| *p != 0 && *p != me) {
            let pid = pid as i32;
            let Ok(info) = pidinfo::<BSDInfo>(pid, 0) else {
                continue;
            };
            let Ok(fds) = listpidinfo::<ListFDs>(pid, info.pbi_nfiles as usize) else {
                continue;
            };
            for fd in fds {
                if !matches!(ProcFDType::from(fd.proc_fdtype), ProcFDType::Socket) {
                    continue;
                }
                let Ok(sock) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else {
                    continue;
                };
                if !matches!(SocketInfoKind::from(sock.psi.soi_kind), SocketInfoKind::Tcp) {
                    continue;
                }
                // SAFETY: soi_kind says this union holds TCP info.
                let tcp = unsafe { sock.psi.soi_proto.pri_tcp };
                // Ports are stored in network byte order in an int.
                let port = |p: i32| u16::from_be(p as u16);
                if port(tcp.tcpsi_ini.insi_fport) == remote_port {
                    found.insert(port(tcp.tcpsi_ini.insi_lport), pid as u32);
                }
            }
        }
        found
    }

    fn describe(&self, pid: u32) -> Option<Process> {
        use libproc::libproc::proc_pid;
        let path = proc_pid::pidpath(pid as i32).ok();
        let name = path
            .as_deref()
            .and_then(app_name)
            .or_else(|| proc_pid::name(pid as i32).ok())?;
        Some(Process { pid, name })
    }
}

#[derive(Default)]
struct State {
    known: HashMap<u16, (Option<Process>, Instant)>,
    waiting: HashMap<u16, Vec<FlowId>>,
    scanning: bool,
}

pub struct Resolver {
    lookup: Arc<dyn Lookup>,
    store: Arc<Store>,
    proxy_port: AtomicU16,
    state: Arc<Mutex<State>>,
}

impl Resolver {
    pub fn new(lookup: Arc<dyn Lookup>, store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            lookup,
            store,
            proxy_port: AtomicU16::new(0),
            state: Arc::default(),
        })
    }

    pub fn set_proxy_port(&self, port: u16) {
        self.proxy_port.store(port, Ordering::Relaxed);
        self.state.lock().known.clear();
    }

    fn apply(&self, id: FlowId, process: &Process) {
        let (name, pid) = (process.name.clone(), process.pid);
        self.store.update(id, |m, _| {
            m.process = Some(name);
            m.pid = Some(pid);
        });
    }

    /// Fills in the flow's process once known. Only loopback clients are
    /// looked up (devices on the network are other machines).
    pub fn resolve(self: &Arc<Self>, id: FlowId, client: std::net::SocketAddr) {
        if !client.ip().is_loopback() {
            return;
        }
        let port = client.port();
        let mut st = self.state.lock();
        if let Some((known, at)) = st.known.get(&port)
            && at.elapsed() < CACHE_TTL
        {
            if let Some(p) = known.clone() {
                drop(st);
                self.apply(id, &p);
            }
            return;
        }
        st.waiting.entry(port).or_default().push(id);
        if st.scanning {
            return;
        }
        st.scanning = true;
        drop(st);
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.scan_loop());
    }

    /// Scans until nothing is waiting; requests that arrive during a scan
    /// are served by the next one.
    fn scan_loop(&self) {
        loop {
            let ports: Vec<u16> = {
                let mut st = self.state.lock();
                if st.waiting.is_empty() {
                    st.scanning = false;
                    return;
                }
                st.waiting.keys().copied().collect()
            };
            let owners = self
                .lookup
                .connections_to(self.proxy_port.load(Ordering::Relaxed));
            let mut names: HashMap<u32, Option<Process>> = HashMap::new();
            let mut done = Vec::new();
            {
                let mut st = self.state.lock();
                if st.known.len() > 8192 {
                    st.known.retain(|_, (_, at)| at.elapsed() < CACHE_TTL);
                }
                for port in ports {
                    let process = owners.get(&port).and_then(|pid| {
                        names
                            .entry(*pid)
                            .or_insert_with(|| self.lookup.describe(*pid))
                            .clone()
                    });
                    st.known.insert(port, (process.clone(), Instant::now()));
                    if let Some(ids) = st.waiting.remove(&port) {
                        done.push((ids, process));
                    }
                }
            }
            for (ids, process) in done {
                if let Some(p) = process {
                    for id in ids {
                        self.apply(id, &p);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::store::{FlowData, test_meta};
    use std::sync::atomic::AtomicUsize;

    struct Fake {
        scans: AtomicUsize,
    }

    impl Lookup for Fake {
        fn connections_to(&self, remote_port: u16) -> HashMap<u16, u32> {
            self.scans.fetch_add(1, Ordering::SeqCst);
            assert_eq!(remote_port, 9090);
            HashMap::from([(5000, 42), (5001, 42)])
        }
        fn describe(&self, pid: u32) -> Option<Process> {
            Some(Process {
                pid,
                name: "Safari".into(),
            })
        }
    }

    async fn settle(store: &Store, id: FlowId) -> Option<String> {
        for _ in 0..100 {
            if let Some(p) = store.get(id).and_then(|(m, _)| m.process) {
                return Some(p);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    }

    // After httpmon internal/procinfo/resolver_test.go.
    #[tokio::test]
    async fn resolves_and_caches_by_client_port() {
        let store = Arc::new(Store::new(10));
        let fake = Arc::new(Fake {
            scans: AtomicUsize::new(0),
        });
        let r = Resolver::new(fake.clone(), store.clone());
        r.set_proxy_port(9090);
        let a = store.add(test_meta("/a"), FlowData::default());
        let b = store.add(test_meta("/b"), FlowData::default());
        let c = store.add(test_meta("/c"), FlowData::default());
        r.resolve(a, "127.0.0.1:5000".parse().unwrap());
        assert_eq!(settle(&store, a).await.as_deref(), Some("Safari"));
        r.resolve(b, "127.0.0.1:5000".parse().unwrap());
        assert_eq!(settle(&store, b).await.as_deref(), Some("Safari"));
        assert_eq!(fake.scans.load(Ordering::SeqCst), 1, "cached");
        assert_eq!(store.get(b).unwrap().0.pid, Some(42));

        r.resolve(c, "127.0.0.1:7777".parse().unwrap());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(store.get(c).unwrap().0.process, None, "unknown stays empty");

        let remote = store.add(test_meta("/d"), FlowData::default());
        r.resolve(remote, "192.168.1.20:5000".parse().unwrap());
        assert_eq!(
            fake.scans.load(Ordering::SeqCst),
            2,
            "devices aren't looked up"
        );
    }

    #[test]
    fn names_apps_by_their_outermost_bundle() {
        assert_eq!(
            app_name("/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Helper.app/Contents/MacOS/Helper").as_deref(),
            Some("Google Chrome")
        );
        assert_eq!(app_name("/usr/bin/curl").as_deref(), Some("curl"));
    }

    #[test]
    fn finds_the_owner_of_a_real_connection() {
        // Our own process is skipped, so connect from a child: `nc`.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut child = match std::process::Command::new("/usr/bin/nc")
            .args(["127.0.0.1", &port.to_string()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return, // no nc on this machine
        };
        let (_conn, peer) = listener.accept().unwrap();
        let owners = Darwin.connections_to(port);
        let described = Darwin.describe(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(owners.get(&peer.port()), Some(&child.id()));
        assert_eq!(described.map(|p| p.name), Some("nc".to_string()));
    }
}
