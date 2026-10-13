//! macOS system proxy control with crash-safe restore.
//!
//! See "System proxy crash safety" in docs/PLAN.md. In short:
//! 1. Snapshot every network service's HTTP/HTTPS proxy and write a marker
//!    file *before* changing anything.
//! 2. Restore from the marker on every exit the app can observe.
//! 3. A watchdog process (see `guard.rs`) restores if the app dies hard.
//! 4. On launch, a marker left by a dead process is restored.
//!
//! A restore only touches a service whose proxy still points at us, so
//! changes the user made by hand in the meantime are kept.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::ca::{run_as_admin, shell_quote};

const NETWORKSETUP: &str = "/usr/sbin/networksetup";

#[derive(Debug, thiserror::Error)]
pub enum SysProxyError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("marker: {0}")]
    Marker(#[from] serde_json::Error),
    #[error("networksetup: {0}")]
    Command(String),
    #[error("no enabled network services found")]
    NoServices,
}

/// Abstraction over `networksetup` so the logic can be tested without
/// touching real network settings.
pub trait NetworkSetup: Send + Sync {
    /// Runs `networksetup` with `args`; returns combined stdout+stderr.
    fn run(&self, args: &[String]) -> Result<String, SysProxyError>;
    /// Runs several `networksetup` invocations with administrator rights,
    /// behind a single password prompt.
    fn run_admin(&self, cmds: &[Vec<String>]) -> Result<(), SysProxyError>;
}

pub struct RealNetworkSetup;

impl NetworkSetup for RealNetworkSetup {
    fn run(&self, args: &[String]) -> Result<String, SysProxyError> {
        let out = Command::new(NETWORKSETUP).args(args).output()?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            return Err(SysProxyError::Command(text.trim().to_string()));
        }
        Ok(text)
    }

    fn run_admin(&self, cmds: &[Vec<String>]) -> Result<(), SysProxyError> {
        let script = cmds
            .iter()
            .map(|args| {
                std::iter::once(NETWORKSETUP.to_string())
                    .chain(args.iter().map(|a| shell_quote(a)))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" && ");
        run_as_admin(&script).map_err(|e| SysProxyError::Command(e.to_string()))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxySetting {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
}

impl ProxySetting {
    fn points_to(&self, host: &str, port: u16) -> bool {
        self.enabled && self.server == host && self.port == port
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub name: String,
    pub http: ProxySetting,
    pub https: ProxySetting,
}

/// Written to disk before the system proxy is changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub version: u32,
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub created_at: f64,
    pub services: Vec<ServiceSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Services whose settings were put back.
    pub restored: Vec<String>,
    /// Services left alone because they no longer point at us.
    pub skipped: Vec<String>,
}

#[derive(Clone, Copy)]
enum Kind {
    Http,
    Https,
}

impl Kind {
    fn get(self) -> &'static str {
        match self {
            Kind::Http => "-getwebproxy",
            Kind::Https => "-getsecurewebproxy",
        }
    }
    fn set(self) -> &'static str {
        match self {
            Kind::Http => "-setwebproxy",
            Kind::Https => "-setsecurewebproxy",
        }
    }
    fn set_state(self) -> &'static str {
        match self {
            Kind::Http => "-setwebproxystate",
            Kind::Https => "-setsecurewebproxystate",
        }
    }
}

/// Lists enabled network services (disabled ones are prefixed with `*`).
pub fn list_services(ns: &dyn NetworkSetup) -> Result<Vec<String>, SysProxyError> {
    let out = ns.run(&["-listallnetworkservices".into()])?;
    Ok(out
        .lines()
        .skip(1) // "An asterisk (*) denotes that a network service is disabled."
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('*'))
        .map(str::to_string)
        .collect())
}

pub fn parse_proxy_setting(out: &str) -> ProxySetting {
    let mut s = ProxySetting::default();
    for line in out.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("Enabled:") {
            s.enabled = v.trim() == "Yes";
        } else if let Some(v) = line.strip_prefix("Server:") {
            s.server = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Port:") {
            s.port = v.trim().parse().unwrap_or(0);
        }
    }
    s
}

fn read_setting(
    ns: &dyn NetworkSetup,
    svc: &str,
    kind: Kind,
) -> Result<ProxySetting, SysProxyError> {
    Ok(parse_proxy_setting(
        &ns.run(&[kind.get().into(), svc.into()])?,
    ))
}

pub fn snapshot(ns: &dyn NetworkSetup) -> Result<Vec<ServiceSnapshot>, SysProxyError> {
    list_services(ns)?
        .into_iter()
        .map(|name| {
            Ok(ServiceSnapshot {
                http: read_setting(ns, &name, Kind::Http)?,
                https: read_setting(ns, &name, Kind::Https)?,
                name,
            })
        })
        .collect()
}

/// Runs `networksetup` set commands. They print nothing on success (and
/// sometimes exit 0 even on failure), so any output counts as a failure.
/// Whatever fails is retried once with administrator rights.
fn apply(ns: &dyn NetworkSetup, cmds: Vec<Vec<String>>) -> Result<(), SysProxyError> {
    let failed: Vec<_> = cmds
        .into_iter()
        .filter(|c| !matches!(ns.run(c), Ok(out) if out.trim().is_empty()))
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        ns.run_admin(&failed)
    }
}

pub fn read_marker(path: &Path) -> Result<Option<Marker>, SysProxyError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes the marker atomically: temp file, fsync, rename.
pub fn write_marker(path: &Path, marker: &Marker) -> Result<(), SysProxyError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(marker)?)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent()
        && let Ok(d) = fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

fn remove_marker(path: &Path) -> Result<(), SysProxyError> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Snapshots current settings, writes the marker, then points every enabled
/// service's HTTP and HTTPS proxy at `host:port`.
pub fn enable(
    ns: &dyn NetworkSetup,
    marker_path: &Path,
    host: &str,
    port: u16,
    created_at: f64,
) -> Result<Marker, SysProxyError> {
    // A marker from an earlier run holds the user's original settings;
    // never overwrite it with a snapshot that already points at us.
    if let Some(old) = read_marker(marker_path)? {
        restore(ns, marker_path, &old)?;
    }

    let services = snapshot(ns)?;
    if services.is_empty() {
        return Err(SysProxyError::NoServices);
    }
    let marker = Marker {
        version: 1,
        pid: std::process::id(),
        host: host.to_string(),
        port,
        created_at,
        services,
    };
    write_marker(marker_path, &marker)?;

    let port_s = port.to_string();
    let cmds = marker
        .services
        .iter()
        .flat_map(|s| {
            [Kind::Http, Kind::Https]
                .map(|k| vec![k.set().into(), s.name.clone(), host.into(), port_s.clone()])
        })
        .collect();
    if let Err(e) = apply(ns, cmds) {
        // Couldn't switch (e.g. password prompt cancelled): undo partial changes.
        let _ = restore(ns, marker_path, &marker);
        return Err(e);
    }
    Ok(marker)
}

/// Puts back the settings recorded in `marker` for every service that still
/// points at the marker's `host:port`, then deletes the marker file.
pub fn restore(
    ns: &dyn NetworkSetup,
    marker_path: &Path,
    marker: &Marker,
) -> Result<RestoreReport, SysProxyError> {
    let mut report = RestoreReport::default();
    let mut cmds = Vec::new();
    for svc in &marker.services {
        let mut touched = false;
        for (kind, saved) in [(Kind::Http, &svc.http), (Kind::Https, &svc.https)] {
            // A service that disappeared (or can't be read) is skipped.
            let Ok(current) = read_setting(ns, &svc.name, kind) else {
                continue;
            };
            if !current.points_to(&marker.host, marker.port) {
                continue;
            }
            touched = true;
            if !saved.server.is_empty() {
                cmds.push(vec![
                    kind.set().into(),
                    svc.name.clone(),
                    saved.server.clone(),
                    saved.port.to_string(),
                ]);
            }
            if !saved.enabled || saved.server.is_empty() {
                cmds.push(vec![
                    kind.set_state().into(),
                    svc.name.clone(),
                    "off".into(),
                ]);
            }
        }
        if touched {
            report.restored.push(svc.name.clone());
        } else {
            report.skipped.push(svc.name.clone());
        }
    }
    if !cmds.is_empty() {
        apply(ns, cmds)?;
    }
    remove_marker(marker_path)?;
    Ok(report)
}

/// Restores a marker left behind by a process that is no longer running.
/// Returns `None` when there is nothing to recover.
pub fn recover_stale(
    ns: &dyn NetworkSetup,
    marker_path: &Path,
    is_alive: impl Fn(u32) -> bool,
) -> Result<Option<RestoreReport>, SysProxyError> {
    let Some(marker) = read_marker(marker_path)? else {
        return Ok(None);
    };
    if marker.pid != std::process::id() && is_alive(marker.pid) {
        return Ok(None);
    }
    restore(ns, marker_path, &marker).map(Some)
}

pub fn pid_alive(pid: u32) -> bool {
    // Signal 0 checks for existence without delivering anything.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::BTreeMap;

    /// In-memory fake of `networksetup`.
    #[derive(Default)]
    pub struct FakeNs {
        pub services: Vec<(String, bool)>, // (name, enabled)
        pub state: Mutex<BTreeMap<(String, &'static str), ProxySetting>>,
        /// When set, non-admin `set` calls print an error (like a standard user).
        pub needs_admin: bool,
        pub admin_calls: Mutex<usize>,
        pub log: Mutex<Vec<Vec<String>>>,
    }

    impl FakeNs {
        pub fn new(names: &[&str]) -> Self {
            Self {
                services: names.iter().map(|n| (n.to_string(), true)).collect(),
                ..Default::default()
            }
        }
        pub fn get(&self, svc: &str, kind: &'static str) -> ProxySetting {
            self.state
                .lock()
                .get(&(svc.to_string(), kind))
                .cloned()
                .unwrap_or_default()
        }
        pub fn set(&self, svc: &str, kind: &'static str, s: ProxySetting) {
            self.state.lock().insert((svc.to_string(), kind), s);
        }
        fn exec(&self, args: &[String]) -> Result<String, SysProxyError> {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            let kind = |verb: &str| {
                if verb.contains("secure") {
                    "https"
                } else {
                    "http"
                }
            };
            match a.as_slice() {
                ["-listallnetworkservices"] => {
                    let mut out =
                        "An asterisk (*) denotes that a network service is disabled.\n".to_string();
                    for (n, en) in &self.services {
                        out.push_str(&format!("{}{n}\n", if *en { "" } else { "*" }));
                    }
                    Ok(out)
                }
                [verb @ ("-getwebproxy" | "-getsecurewebproxy"), svc] => {
                    if !self.services.iter().any(|(n, _)| n == svc) {
                        return Err(SysProxyError::Command(format!(
                            "{svc} is not a recognized network service."
                        )));
                    }
                    let s = self.get(svc, kind(verb));
                    Ok(format!(
                        "Enabled: {}\nServer: {}\nPort: {}\nAuthenticated Proxy Enabled: 0\n",
                        if s.enabled { "Yes" } else { "No" },
                        s.server,
                        s.port
                    ))
                }
                [
                    verb @ ("-setwebproxy" | "-setsecurewebproxy"),
                    svc,
                    host,
                    port,
                ] => {
                    self.set(
                        svc,
                        kind(verb),
                        ProxySetting {
                            enabled: true,
                            server: host.to_string(),
                            port: port.parse().unwrap(),
                        },
                    );
                    Ok(String::new())
                }
                [
                    verb @ ("-setwebproxystate" | "-setsecurewebproxystate"),
                    svc,
                    st,
                ] => {
                    let mut s = self.get(svc, kind(verb));
                    s.enabled = *st == "on";
                    self.set(svc, kind(verb), s);
                    Ok(String::new())
                }
                other => panic!("unexpected networksetup call: {other:?}"),
            }
        }
    }

    impl NetworkSetup for FakeNs {
        fn run(&self, args: &[String]) -> Result<String, SysProxyError> {
            self.log.lock().push(args.to_vec());
            if self.needs_admin && args[0].starts_with("-set") {
                return Ok(
                    "You cannot set network settings unless you are an administrator.".into(),
                );
            }
            self.exec(args)
        }
        fn run_admin(&self, cmds: &[Vec<String>]) -> Result<(), SysProxyError> {
            *self.admin_calls.lock() += 1;
            for c in cmds {
                self.exec(c)?;
            }
            Ok(())
        }
    }

    fn ours() -> ProxySetting {
        ProxySetting {
            enabled: true,
            server: "127.0.0.1".into(),
            port: 9090,
        }
    }

    #[test]
    fn parses_networksetup_output() {
        let s = parse_proxy_setting(
            "Enabled: Yes\nServer: proxy.corp\nPort: 3128\nAuthenticated Proxy Enabled: 0\n",
        );
        assert_eq!(
            s,
            ProxySetting {
                enabled: true,
                server: "proxy.corp".into(),
                port: 3128
            }
        );
        assert_eq!(
            parse_proxy_setting("Enabled: No\nServer: \nPort: 0\n"),
            ProxySetting::default()
        );
    }

    #[test]
    fn lists_only_enabled_services() {
        let mut ns = FakeNs::new(&["Wi-Fi", "Thunderbolt Bridge"]);
        ns.services[1].1 = false;
        assert_eq!(list_services(&ns).unwrap(), vec!["Wi-Fi".to_string()]);
    }

    #[test]
    fn enable_then_restore_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("sysproxy.marker.json");
        let ns = FakeNs::new(&["Wi-Fi", "Ethernet"]);
        let corp = ProxySetting {
            enabled: true,
            server: "proxy.corp".into(),
            port: 3128,
        };
        ns.set("Ethernet", "http", corp.clone());

        let m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();
        assert!(marker.exists());
        assert_eq!(read_marker(&marker).unwrap().unwrap(), m);
        for svc in ["Wi-Fi", "Ethernet"] {
            assert_eq!(ns.get(svc, "http"), ours());
            assert_eq!(ns.get(svc, "https"), ours());
        }

        let report = restore(&ns, &marker, &m).unwrap();
        assert_eq!(report.restored.len(), 2);
        assert!(!marker.exists());
        assert_eq!(ns.get("Ethernet", "http"), corp);
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(!ns.get("Wi-Fi", "https").enabled);
    }

    #[test]
    fn restore_skips_services_changed_by_hand() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi", "Ethernet"]);
        let m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();

        let manual = ProxySetting {
            enabled: true,
            server: "10.0.0.5".into(),
            port: 8888,
        };
        ns.set("Wi-Fi", "http", manual.clone());
        ns.set("Wi-Fi", "https", manual.clone());

        let report = restore(&ns, &marker, &m).unwrap();
        assert_eq!(report.skipped, vec!["Wi-Fi".to_string()]);
        assert_eq!(report.restored, vec!["Ethernet".to_string()]);
        assert_eq!(ns.get("Wi-Fi", "http"), manual);
    }

    #[test]
    fn recover_stale_only_for_dead_pids() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        let mut m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();
        m.pid = 999_999;
        write_marker(&marker, &m).unwrap();

        assert!(recover_stale(&ns, &marker, |_| true).unwrap().is_none());
        assert!(marker.exists());

        let report = recover_stale(&ns, &marker, |_| false).unwrap().unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!marker.exists());
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(recover_stale(&ns, &marker, |_| false).unwrap().is_none());
    }

    #[test]
    fn falls_back_to_one_admin_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs {
            needs_admin: true,
            ..FakeNs::new(&["Wi-Fi", "Ethernet"])
        };
        let m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();
        assert_eq!(*ns.admin_calls.lock(), 1);
        assert_eq!(ns.get("Ethernet", "https"), ours());
        restore(&ns, &marker, &m).unwrap();
        assert_eq!(*ns.admin_calls.lock(), 2);
        assert!(!ns.get("Ethernet", "https").enabled);
    }

    #[test]
    fn enable_reuses_original_snapshot_from_leftover_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        let corp = ProxySetting {
            enabled: true,
            server: "proxy.corp".into(),
            port: 3128,
        };
        ns.set("Wi-Fi", "http", corp.clone());
        enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();
        // Enabling again must not record our own proxy as the "original".
        let m2 = enable(&ns, &marker, "127.0.0.1", 9090, 2.0).unwrap();
        assert_eq!(m2.services[0].http, corp);
        restore(&ns, &marker, &m2).unwrap();
        assert_eq!(ns.get("Wi-Fi", "http"), corp);
    }

    #[test]
    fn pid_alive_detects_self() {
        assert!(pid_alive(std::process::id()));
    }
}
