//! Mocks: APIs picked from captured traffic, kept with a fake response.
//!
//! A mock matches requests by method and URL like a rule (see `rules.rs`).
//! Every matching flow is marked with the mock's id so the UI can list the
//! calls an API received; when the mock is enabled the proxy answers with
//! its response instead of contacting the server. Mocks are stored as
//! `mocks.json` in the data directory and checked before Map Local rules.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::rules::{CompiledMatcher, Matcher};

/// The request as it was captured, so the API can be sent again from the
/// Mocks page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MockRequest {
    /// The URL as it was captured, for sending the API again.
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Body text; an undecodable body is kept in `body_base64` instead.
    pub body: String,
    pub body_base64: String,
}

/// The response served while the mock is enabled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Added latency before the response is sent.
    pub delay_ms: u64,
}

impl Default for MockResponse {
    fn default() -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: String::new(),
            delay_ms: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mock {
    /// `matcher.enabled` is whether the fake response is served; a disabled
    /// mock still collects the calls its API receives.
    #[serde(flatten)]
    pub matcher: Matcher,
    pub request: MockRequest,
    pub response: MockResponse,
    /// Unix epoch milliseconds.
    pub created_at: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mocks {
    pub mocks: Vec<Mock>,
}

impl Mocks {
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

    /// Checks every id, pattern and status, naming the first mock that is wrong.
    pub fn validate(&self) -> Result<(), String> {
        for m in &self.mocks {
            let name = if m.matcher.name.is_empty() {
                format!("mock \"{}\"", m.matcher.url)
            } else {
                format!("mock \"{}\"", m.matcher.name)
            };
            if m.matcher.id.trim().is_empty() {
                return Err(format!("{name}: missing id"));
            }
            if m.matcher.url.trim().is_empty() {
                return Err(format!("{name}: missing URL pattern"));
            }
            CompiledMatcher::compile(&m.matcher).map_err(|e| format!("{name}: {e}"))?;
            if !(100..=999).contains(&m.response.status) {
                return Err(format!("{name}: status must be 100–999"));
            }
        }
        let mut ids: Vec<&str> = self.mocks.iter().map(|m| m.matcher.id.as_str()).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("two mocks share an id".into());
        }
        Ok(())
    }
}

/// Mocks with their patterns compiled. Cheap to share as an `Arc`.
pub struct CompiledMocks {
    pub mocks: Mocks,
    matchers: Vec<CompiledMatcher>,
}

impl CompiledMocks {
    pub fn new(mocks: Mocks) -> Self {
        Self {
            matchers: mocks
                .mocks
                .iter()
                // Disabled mocks still match, to collect calls.
                .map(|m| CompiledMatcher::new(&m.matcher).always())
                .collect(),
            mocks,
        }
    }

    /// The first mock whose API this request calls, enabled or not.
    pub fn find(&self, method: &str, url: &str) -> Option<&Mock> {
        let i = self.matchers.iter().position(|m| m.matches(method, url))?;
        Some(&self.mocks.mocks[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock(id: &str, url: &str) -> Mock {
        Mock {
            matcher: Matcher {
                id: id.into(),
                name: "Users".into(),
                enabled: true,
                method: "GET".into(),
                url: url.into(),
                regex: false,
            },
            response: MockResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: "[]".into(),
                delay_ms: 0,
            },
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("mocks.json");
        let mocks = Mocks {
            mocks: vec![mock("a", "https://api.example.com/users")],
        };
        mocks.save(&path).unwrap();
        assert_eq!(Mocks::load(&path), mocks);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_or_broken_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mocks.json");
        assert_eq!(Mocks::load(&path), Mocks::default());
        fs::write(&path, b"{not json").unwrap();
        assert_eq!(Mocks::load(&path), Mocks::default());
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let json =
            r#"{"mocks":[{"id":"x","url":"https://a/b","future":1,"response":{"status":204}}]}"#;
        let mocks: Mocks = serde_json::from_str(json).unwrap();
        let m = &mocks.mocks[0];
        assert_eq!(m.matcher.id, "x");
        assert!(m.matcher.enabled);
        assert_eq!(m.response.status, 204);
        assert_eq!(m.response.body, "");
        assert_eq!(m.response.delay_ms, 0);
    }

    #[test]
    fn validation_names_the_bad_mock() {
        let ok = Mocks {
            mocks: vec![mock("a", "https://a/*"), mock("b", "https://b/*")],
        };
        assert_eq!(ok.validate(), Ok(()));

        let mut bad = ok.clone();
        bad.mocks[1].matcher.id = "a".into();
        assert!(bad.validate().unwrap_err().contains("share an id"));

        let mut bad = ok.clone();
        bad.mocks[0].matcher.id = String::new();
        assert!(bad.validate().unwrap_err().contains("missing id"));

        let mut bad = ok.clone();
        bad.mocks[0].matcher.url = " ".into();
        assert!(bad.validate().unwrap_err().contains("missing URL"));

        let mut bad = ok.clone();
        bad.mocks[0].matcher.regex = true;
        bad.mocks[0].matcher.url = "(".into();
        assert!(bad.validate().unwrap_err().starts_with("mock \"Users\": "));

        let mut bad = ok;
        bad.mocks[0].response.status = 42;
        assert!(bad.validate().unwrap_err().contains("status"));
    }

    #[test]
    fn finds_by_method_and_pattern_even_when_disabled() {
        let mut off = mock("off", "https://api.example.com/users/*");
        off.matcher.enabled = false;
        let on = mock("on", "https://api.example.com/users");
        let compiled = CompiledMocks::new(Mocks {
            mocks: vec![off, on],
        });

        let hit = compiled
            .find("get", "https://api.example.com/users/42")
            .unwrap();
        assert_eq!(hit.matcher.id, "off");
        assert!(!hit.matcher.enabled);

        assert_eq!(
            compiled
                .find("GET", "https://api.example.com/users")
                .map(|m| m.matcher.id.as_str()),
            Some("on")
        );
        assert!(
            compiled
                .find("POST", "https://api.example.com/users")
                .is_none()
        );
        assert!(
            compiled
                .find("GET", "https://other.example.com/users")
                .is_none()
        );
    }
}
