//! User settings that change how traffic is handled: which hosts get TLS
//! interception, block and allow lists, and network throttling. Stored as
//! `settings.json` in the data directory and applied without a restart.
//!
//! Host matching is a port of httpmon's `internal/hostfilter`.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub ssl_proxying: SslProxying,
    pub block_list: HostList,
    pub allow_list: HostList,
    pub throttle: Throttle,
    /// `.proto` files for decoding protobuf / gRPC bodies by name.
    pub protobuf: Vec<super::proto::ProtoSource>,
    /// Accept connections from other devices on the network (phones,
    /// tablets). Off by default: anyone who can reach the Mac could use it.
    pub allow_lan: bool,
    /// Point the system at the proxy with HTTP/HTTPS proxy settings or an
    /// automatic configuration (PAC) URL.
    pub system_proxy_mode: super::sysproxy::Mode,
}

/// Which HTTPS hosts are decrypted. Hosts that aren't are tunnelled as-is
/// and show up as a single CONNECT entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SslProxying {
    /// When non-empty, only these hosts are decrypted.
    pub include: Vec<String>,
    /// Never decrypted, even when they also match `include`.
    pub exclude: Vec<String>,
}

/// Apple services pin their certificates and fail (and retry constantly)
/// when decrypted, so they are passed through by default.
pub const DEFAULT_SSL_EXCLUDE: &[&str] = &[
    "apple.com",
    "*.apple.com",
    "icloud.com",
    "*.icloud.com",
    "*.icloud-content.com",
    "*.apple-cloudkit.com",
    "*.mzstatic.com",
    "*.cdn-apple.com",
];

impl Default for SslProxying {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: DEFAULT_SSL_EXCLUDE.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostList {
    pub enabled: bool,
    pub hosts: Vec<String>,
}

impl HostList {
    fn active(&self) -> bool {
        self.enabled && !self.hosts.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Throttle {
    pub enabled: bool,
    /// "3g" | "4g" | "wifi" | "custom"; informational, the numbers rule.
    pub preset: String,
    /// Response bandwidth in bytes per second (0 = unlimited).
    pub bytes_per_sec: u64,
    /// Delay added before each response is returned.
    pub latency_ms: u64,
    /// Hosts to throttle; empty means every host.
    pub hosts: Vec<String>,
}

impl Default for Throttle {
    fn default() -> Self {
        let (bytes_per_sec, latency_ms) = preset("3g").unwrap();
        Self {
            enabled: false,
            preset: "3g".into(),
            bytes_per_sec,
            latency_ms,
            hosts: Vec::new(),
        }
    }
}

/// Bandwidth (bytes/s, from httpmon's `throttle.PresetBandwidth`) and a
/// typical round-trip latency for each preset.
pub fn preset(name: &str) -> Option<(u64, u64)> {
    match name.to_ascii_lowercase().as_str() {
        "3g" => Some((93_750, 300)),
        "4g" => Some((500_000, 100)),
        "wifi" => Some((3_750_000, 20)),
        _ => None,
    }
}

/// What to do with a request to some host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPolicy {
    /// Refuse the request (block list).
    pub block: bool,
    /// Show it in the flow list (allow list).
    pub record: bool,
}

impl Settings {
    pub fn policy(&self, host: &str) -> HostPolicy {
        let host = normalize(host);
        HostPolicy {
            block: self.block_list.active() && matches_any(&self.block_list.hosts, &host),
            record: !self.allow_list.active() || matches_any(&self.allow_list.hosts, &host),
        }
    }

    /// Whether a CONNECT to `host` should be decrypted.
    pub fn should_intercept(&self, host: &str) -> bool {
        let host = normalize(host);
        let ssl = &self.ssl_proxying;
        if matches_any(&ssl.exclude, &host) {
            return false;
        }
        // Hosts that aren't recorded aren't worth decrypting either.
        (ssl.include.is_empty() || matches_any(&ssl.include, &host)) && self.policy(&host).record
    }

    /// Bandwidth and latency to apply to responses from `host`, if any.
    pub fn throttle_for(&self, host: &str) -> Option<(u64, u64)> {
        let t = &self.throttle;
        if !t.enabled || (t.bytes_per_sec == 0 && t.latency_ms == 0) {
            return None;
        }
        if !t.hosts.is_empty() && !matches_any(&t.hosts, &normalize(host)) {
            return None;
        }
        Some((t.bytes_per_sec, t.latency_ms))
    }

    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        f.sync_all()?;
        fs::rename(tmp, path)
    }
}

/// Lowercases and strips a `:port` suffix (but not from bare IPv6).
fn normalize(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    let h = h
        .strip_prefix('[')
        .map_or(h.as_str(), |s| s.split(']').next().unwrap_or(s));
    match h.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            name.to_string()
        }
        _ => h.to_string(),
    }
}

fn matches_any(patterns: &[String], host: &str) -> bool {
    patterns.iter().any(|p| match_host(p, host))
}

/// `example.com` matches exactly; `*.example.com` matches subdomains only
/// (as in httpmon); `*` matches everything.
pub fn match_host(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    if p == "*" {
        return true;
    }
    match p.strip_prefix('*') {
        Some(suffix) if suffix.starts_with('.') => host.ends_with(suffix),
        _ => p == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(hosts: &[&str]) -> HostList {
        HostList {
            enabled: true,
            hosts: hosts.iter().map(|s| s.to_string()).collect(),
        }
    }

    // Ported from httpmon internal/hostfilter/hostfilter_test.go.
    #[test]
    fn wildcard_matching() {
        assert!(match_host("*.example.com", "api.example.com"));
        assert!(match_host("*.example.com", "a.b.example.com"));
        assert!(!match_host("*.example.com", "example.com"));
        assert!(!match_host("*.example.com", "notexample.com"));
        assert!(match_host("Example.com", "example.com"));
        assert!(!match_host("example.com", "api.example.com"));
        assert!(match_host("*", "anything.io"));
    }

    #[test]
    fn block_list() {
        let s = Settings {
            block_list: list(&["*.ads.com", "tracker.io"]),
            ..Default::default()
        };
        assert!(s.policy("x.ads.com").block);
        assert!(s.policy("tracker.io:443").block);
        assert!(!s.policy("example.com").block);
        let off = Settings {
            block_list: HostList {
                enabled: false,
                ..list(&["tracker.io"])
            },
            ..Default::default()
        };
        assert!(!off.policy("tracker.io").block);
    }

    #[test]
    fn allow_list_limits_recording_and_interception() {
        let s = Settings {
            allow_list: list(&["*.mine.dev"]),
            ..Default::default()
        };
        assert!(s.policy("api.mine.dev").record);
        assert!(!s.policy("google.com").record);
        assert!(s.should_intercept("api.mine.dev:443"));
        assert!(!s.should_intercept("google.com:443"));
    }

    #[test]
    fn no_rules_intercepts_and_records_everything() {
        let s = Settings::default();
        assert_eq!(
            s.policy("a.com"),
            HostPolicy {
                block: false,
                record: true
            }
        );
        assert!(s.should_intercept("a.com:443"));
        assert!(s.throttle_for("a.com").is_none());
    }

    #[test]
    fn apple_services_are_tunnelled_by_default() {
        let s = Settings::default();
        assert!(!s.should_intercept("gateway.icloud.com:443"));
        assert!(!s.should_intercept("apple.com"));
        assert!(s.should_intercept("ssl.gstatic.com:443"));
    }

    #[test]
    fn ssl_include_and_exclude() {
        let mut s = Settings::default();
        s.ssl_proxying.exclude = vec!["*.apple.com".into()];
        assert!(!s.should_intercept("api.apple.com:443"));
        assert!(s.should_intercept("example.com:443"));
        s.ssl_proxying.include = vec!["*.example.com".into(), "api.apple.com".into()];
        assert!(s.should_intercept("www.example.com"));
        assert!(!s.should_intercept("other.com"));
        assert!(
            !s.should_intercept("api.apple.com"),
            "exclude wins over include"
        );
    }

    #[test]
    fn throttle_scope() {
        let mut s = Settings::default();
        s.throttle.enabled = true;
        assert_eq!(s.throttle_for("a.com"), Some((93_750, 300)));
        s.throttle.hosts = vec!["slow.com".into()];
        assert_eq!(s.throttle_for("a.com"), None);
        assert!(s.throttle_for("slow.com:8080").is_some());
    }

    #[test]
    fn presets_match_httpmon() {
        assert_eq!(preset("3G").map(|p| p.0), Some(93_750));
        assert_eq!(preset("4g").map(|p| p.0), Some(500_000));
        assert_eq!(preset("wifi").map(|p| p.0), Some(3_750_000));
        assert_eq!(preset("dialup"), None);
    }

    #[test]
    fn normalizes_hosts() {
        assert_eq!(normalize("Example.COM:8080"), "example.com");
        assert_eq!(normalize("[::1]:443"), "::1");
        assert_eq!(normalize("::1"), "::1");
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let s = Settings {
            block_list: list(&["x.com"]),
            ..Default::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }
}
