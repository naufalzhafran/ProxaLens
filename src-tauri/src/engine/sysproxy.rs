//! macOS system proxy control with crash-safe restore.
//!
//! Crash safety comes in layers:
//! 1. Snapshot every network service's HTTP/HTTPS proxy and write a marker
//!    file *before* changing anything.
//! 2. Restore from the marker on every exit the app can observe.
//! 3. A watchdog process (see `guard.rs`) restores if the app dies hard.
//! 4. On launch, a marker left by a dead process is restored.
//!
//! A restore only touches a service whose proxy still points at us, so
//! changes the user made by hand in the meantime are kept.
//!
//! A setting that points at our own address is never treated as the user's:
//! it can only be left over from a run that lost its marker. Snapshots and
//! markers record it as "off", and `clear_dangling` switches it off when no
//! marker exists at all. Without this, one lost marker would make every
//! later restore put our own proxy back, and the system would stay routed
//! through a proxy that is about to go away.

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

/// Automatic proxy configuration (a PAC URL) for one service.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoProxySetting {
    pub enabled: bool,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub name: String,
    pub http: ProxySetting,
    pub https: ProxySetting,
    #[serde(default)]
    pub pac: AutoProxySetting,
}

/// How the system is pointed at the proxy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// HTTP and HTTPS proxy settings (the default).
    #[default]
    Manual,
    /// An automatic configuration (PAC) URL served by the proxy. If the app
    /// dies, the URL stops answering and most clients go direct by themselves.
    Pac,
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
    #[serde(default)]
    pub mode: Mode,
    /// The PAC URL we installed (PAC mode).
    #[serde(default)]
    pub pac_url: String,
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

pub fn parse_auto_proxy(out: &str) -> AutoProxySetting {
    let mut s = AutoProxySetting::default();
    for line in out.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("URL:") {
            let v = v.trim();
            s.url = if v == "(null)" {
                String::new()
            } else {
                v.to_string()
            };
        } else if let Some(v) = line.strip_prefix("Enabled:") {
            s.enabled = v.trim() == "Yes";
        }
    }
    s
}

fn read_auto_proxy(ns: &dyn NetworkSetup, svc: &str) -> Result<AutoProxySetting, SysProxyError> {
    Ok(parse_auto_proxy(
        &ns.run(&["-getautoproxyurl".into(), svc.into()])?,
    ))
}

pub fn snapshot(ns: &dyn NetworkSetup) -> Result<Vec<ServiceSnapshot>, SysProxyError> {
    list_services(ns)?
        .into_iter()
        .map(|name| {
            Ok(ServiceSnapshot {
                http: read_setting(ns, &name, Kind::Http)?,
                https: read_setting(ns, &name, Kind::Https)?,
                pac: read_auto_proxy(ns, &name)?,
                name,
            })
        })
        .collect()
}

/// The PAC URL the proxy serves for itself.
pub fn pac_url(host: &str, port: u16) -> String {
    format!("http://{host}:{port}/proxy.pac")
}

/// Records every setting that already points at `host:port` as "off". Such a
/// setting is ours, left behind by a run that lost its marker, so restoring
/// it would keep the system stuck on a proxy that is gone.
fn disown(services: &mut [ServiceSnapshot], host: &str, port: u16) {
    let pac = pac_url(host, port);
    for svc in services {
        for setting in [&mut svc.http, &mut svc.https] {
            if setting.points_to(host, port) {
                *setting = ProxySetting::default();
            }
        }
        if svc.pac.enabled && svc.pac.url == pac {
            svc.pac = AutoProxySetting::default();
        }
    }
}

/// Switches off every setting that still points at `host:port` although no
/// marker claims it. Returns the services that were changed.
pub fn clear_dangling(
    ns: &dyn NetworkSetup,
    host: &str,
    port: u16,
) -> Result<Vec<String>, SysProxyError> {
    let pac = pac_url(host, port);
    let mut cmds = Vec::new();
    let mut touched = Vec::new();
    for name in list_services(ns)? {
        let mut hit = false;
        for kind in [Kind::Http, Kind::Https] {
            if read_setting(ns, &name, kind).is_ok_and(|s| s.points_to(host, port)) {
                cmds.push(vec![kind.set_state().into(), name.clone(), "off".into()]);
                hit = true;
            }
        }
        if read_auto_proxy(ns, &name).is_ok_and(|p| p.enabled && p.url == pac) {
            cmds.push(vec![
                "-setautoproxystate".into(),
                name.clone(),
                "off".into(),
            ]);
            hit = true;
        }
        if hit {
            touched.push(name);
        }
    }
    if !cmds.is_empty() {
        apply(ns, cmds)?;
    }
    Ok(touched)
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
/// service at `host:port`: its HTTP and HTTPS proxy (manual mode) or its
/// automatic proxy URL (PAC mode).
pub fn enable(
    ns: &dyn NetworkSetup,
    marker_path: &Path,
    host: &str,
    port: u16,
    created_at: f64,
) -> Result<Marker, SysProxyError> {
    enable_mode(ns, marker_path, host, port, created_at, Mode::Manual)
}

pub fn enable_mode(
    ns: &dyn NetworkSetup,
    marker_path: &Path,
    host: &str,
    port: u16,
    created_at: f64,
    mode: Mode,
) -> Result<Marker, SysProxyError> {
    // A marker from an earlier run holds the user's original settings;
    // never overwrite it with a snapshot that already points at us.
    if let Some(old) = read_marker(marker_path)? {
        restore(ns, marker_path, &old)?;
    }

    let mut services = snapshot(ns)?;
    if services.is_empty() {
        return Err(SysProxyError::NoServices);
    }
    disown(&mut services, host, port);
    let marker = Marker {
        version: 1,
        pid: std::process::id(),
        host: host.to_string(),
        port,
        created_at,
        services,
        mode,
        pac_url: if mode == Mode::Pac {
            pac_url(host, port)
        } else {
            String::new()
        },
    };
    write_marker(marker_path, &marker)?;

    let port_s = port.to_string();
    let cmds = match mode {
        Mode::Manual => marker
            .services
            .iter()
            .flat_map(|s| {
                [Kind::Http, Kind::Https]
                    .map(|k| vec![k.set().into(), s.name.clone(), host.into(), port_s.clone()])
            })
            .collect(),
        // -setautoproxyurl also switches automatic configuration on.
        Mode::Pac => marker
            .services
            .iter()
            .map(|s| {
                vec![
                    "-setautoproxyurl".into(),
                    s.name.clone(),
                    marker.pac_url.clone(),
                ]
            })
            .collect(),
    };
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
    // Markers written before `disown` existed may hold our own address.
    let mut services = marker.services.clone();
    disown(&mut services, &marker.host, marker.port);
    for svc in &services {
        let mut touched = false;
        if marker.mode == Mode::Pac {
            if let Ok(current) = read_auto_proxy(ns, &svc.name)
                && current.enabled
                && current.url == marker.pac_url
            {
                touched = true;
                let saved = &svc.pac;
                if !saved.url.is_empty() {
                    cmds.push(vec![
                        "-setautoproxyurl".into(),
                        svc.name.clone(),
                        saved.url.clone(),
                    ]);
                }
                if !saved.enabled || saved.url.is_empty() {
                    cmds.push(vec![
                        "-setautoproxystate".into(),
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
            continue;
        }
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
/// Launch-time recovery: restores a marker left by a dead process, or, when
/// there is no marker at all, switches off anything still pointing at
/// `host:port`. A marker owned by a live process is left alone.
pub fn recover(
    ns: &dyn NetworkSetup,
    marker_path: &Path,
    host: &str,
    port: u16,
    is_alive: impl Fn(u32) -> bool,
) -> Result<Option<RestoreReport>, SysProxyError> {
    match read_marker(marker_path)? {
        Some(m) if m.pid != std::process::id() && is_alive(m.pid) => Ok(None),
        Some(m) => restore(ns, marker_path, &m).map(Some),
        None => {
            let restored = clear_dangling(ns, host, port)?;
            Ok((!restored.is_empty()).then(|| RestoreReport {
                restored,
                skipped: Vec::new(),
            }))
        }
    }
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
        pub pac: Mutex<BTreeMap<String, AutoProxySetting>>,
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
                ["-getautoproxyurl", svc] => {
                    let p = self.pac.lock().get(*svc).cloned().unwrap_or_default();
                    Ok(format!(
                        "URL: {}\nEnabled: {}\n",
                        if p.url.is_empty() { "(null)" } else { &p.url },
                        if p.enabled { "Yes" } else { "No" }
                    ))
                }
                ["-setautoproxyurl", svc, url] => {
                    self.pac.lock().insert(
                        svc.to_string(),
                        AutoProxySetting {
                            enabled: true,
                            url: url.to_string(),
                        },
                    );
                    Ok(String::new())
                }
                ["-setautoproxystate", svc, st] => {
                    let mut pac = self.pac.lock();
                    let e = pac.entry(svc.to_string()).or_default();
                    e.enabled = *st == "on";
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
    fn recover_restores_only_for_dead_pids() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        let mut m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();
        m.pid = 999_999;
        write_marker(&marker, &m).unwrap();

        assert!(
            recover(&ns, &marker, "127.0.0.1", 9090, |_| true)
                .unwrap()
                .is_none()
        );
        assert!(marker.exists());

        let report = recover(&ns, &marker, "127.0.0.1", 9090, |_| false)
            .unwrap()
            .unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!marker.exists());
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(
            recover(&ns, &marker, "127.0.0.1", 9090, |_| false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn recover_without_marker_switches_off_dangling_proxy() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi", "Ethernet"]);
        let corp = ProxySetting {
            enabled: true,
            server: "proxy.corp".into(),
            port: 3128,
        };
        // A previous run died and its marker is gone, but Wi-Fi still
        // points at us. Ethernet belongs to the user.
        ns.set("Wi-Fi", "http", ours());
        ns.set("Wi-Fi", "https", ours());
        ns.set("Ethernet", "http", corp.clone());

        let report = recover(&ns, &marker, "127.0.0.1", 9090, |_| false)
            .unwrap()
            .unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(!ns.get("Wi-Fi", "https").enabled);
        assert_eq!(ns.get("Ethernet", "http"), corp);
        assert!(
            recover(&ns, &marker, "127.0.0.1", 9090, |_| false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn clear_dangling_ignores_other_ports_and_handles_pac() {
        let ns = FakeNs::new(&["Wi-Fi", "Ethernet", "USB"]);
        ns.set("Wi-Fi", "http", ours());
        ns.set(
            "Ethernet",
            "http",
            ProxySetting {
                enabled: true,
                server: "127.0.0.1".into(),
                port: 8080,
            },
        );
        ns.pac.lock().insert(
            "USB".into(),
            AutoProxySetting {
                enabled: true,
                url: pac_url("127.0.0.1", 9090),
            },
        );
        let touched = clear_dangling(&ns, "127.0.0.1", 9090).unwrap();
        assert_eq!(touched, vec!["Wi-Fi".to_string(), "USB".to_string()]);
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(
            ns.get("Ethernet", "http").enabled,
            "another port is not ours"
        );
        assert!(!ns.pac.lock().get("USB").unwrap().enabled);
        assert!(clear_dangling(&ns, "127.0.0.1", 9090).unwrap().is_empty());
    }

    #[test]
    fn enable_never_records_our_own_proxy_as_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        // Left behind by a run whose marker was lost.
        ns.set("Wi-Fi", "http", ours());
        ns.set("Wi-Fi", "https", ours());

        let m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();
        assert_eq!(m.services[0].http, ProxySetting::default());
        assert_eq!(m.services[0].https, ProxySetting::default());

        restore(&ns, &marker, &m).unwrap();
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(!ns.get("Wi-Fi", "https").enabled);
        assert!(!marker.exists());
    }

    #[test]
    fn restore_of_an_old_poisoned_marker_switches_us_off() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        ns.set("Wi-Fi", "http", ours());
        ns.set("Wi-Fi", "https", ours());
        // A marker from before `disown`: the "original" is our own address.
        let m = Marker {
            version: 1,
            pid: std::process::id(),
            host: "127.0.0.1".into(),
            port: 9090,
            created_at: 1.0,
            services: vec![ServiceSnapshot {
                name: "Wi-Fi".into(),
                http: ours(),
                https: ours(),
                pac: AutoProxySetting::default(),
            }],
            mode: Mode::Manual,
            pac_url: String::new(),
        };
        write_marker(&marker, &m).unwrap();

        let report = restore(&ns, &marker, &m).unwrap();
        assert_eq!(report.restored, vec!["Wi-Fi".to_string()]);
        assert!(!ns.get("Wi-Fi", "http").enabled);
        assert!(!ns.get("Wi-Fi", "https").enabled);
        assert!(!marker.exists());
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
    fn parses_auto_proxy_output() {
        assert_eq!(
            parse_auto_proxy("URL: http://wpad/proxy.pac\nEnabled: Yes\n"),
            AutoProxySetting {
                enabled: true,
                url: "http://wpad/proxy.pac".into()
            }
        );
        assert_eq!(
            parse_auto_proxy("URL: (null)\nEnabled: No\n"),
            AutoProxySetting::default()
        );
    }

    #[test]
    fn pac_mode_round_trips_and_leaves_manual_proxies_alone() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi", "Ethernet"]);
        let corp_pac = AutoProxySetting {
            enabled: true,
            url: "http://wpad.corp/proxy.pac".into(),
        };
        ns.pac.lock().insert("Ethernet".into(), corp_pac.clone());

        let m = enable_mode(&ns, &marker, "127.0.0.1", 9090, 1.0, Mode::Pac).unwrap();
        assert_eq!(m.pac_url, "http://127.0.0.1:9090/proxy.pac");
        assert_eq!(read_marker(&marker).unwrap().unwrap().mode, Mode::Pac);
        for svc in ["Wi-Fi", "Ethernet"] {
            let p = ns.pac.lock().get(svc).cloned().unwrap();
            assert!(p.enabled && p.url == m.pac_url);
            assert!(!ns.get(svc, "http").enabled, "manual proxies untouched");
        }

        let report = restore(&ns, &marker, &m).unwrap();
        assert_eq!(report.restored.len(), 2);
        assert_eq!(ns.pac.lock().get("Ethernet").cloned().unwrap(), corp_pac);
        assert!(!ns.pac.lock().get("Wi-Fi").unwrap().enabled);
        assert!(!marker.exists());
    }

    #[test]
    fn pac_restore_skips_services_changed_by_hand() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        let m = enable_mode(&ns, &marker, "127.0.0.1", 9090, 1.0, Mode::Pac).unwrap();
        let mine = AutoProxySetting {
            enabled: true,
            url: "http://other/x.pac".into(),
        };
        ns.pac.lock().insert("Wi-Fi".into(), mine.clone());
        let report = restore(&ns, &marker, &m).unwrap();
        assert_eq!(report.skipped, vec!["Wi-Fi".to_string()]);
        assert_eq!(ns.pac.lock().get("Wi-Fi").cloned().unwrap(), mine);
    }

    #[test]
    fn old_markers_without_mode_still_restore() {
        let json = r#"{"version":1,"pid":1,"host":"127.0.0.1","port":9090,"created_at":0,
            "services":[{"name":"Wi-Fi","http":{"enabled":false,"server":"","port":0},"https":{"enabled":false,"server":"","port":0}}]}"#;
        let m: Marker = serde_json::from_str(json).unwrap();
        assert_eq!(m.mode, Mode::Manual);
    }

    #[test]
    fn pid_alive_detects_self() {
        assert!(pid_alive(std::process::id()));
    }
}
