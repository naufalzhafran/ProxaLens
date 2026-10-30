//! Traffic rules: Map Local, Map Remote, Rewrite and Block.
//!
//! Each rule matches requests by method and URL (a `*`/`?` wildcard or a
//! regex). Rules are stored as `rules.json` in the data directory, compiled
//! once when saved, and applied by the proxy handler in this order:
//! block → request rewrite → Map Local → Map Remote → upstream → response
//! rewrite.
//! The edits themselves are pure functions on [`RequestParts`] and
//! [`ResponseParts`] so they can be tested without a proxy.

use std::fs;
use std::io::Write;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Which requests a rule applies to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Matcher {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Empty or "*" for any method.
    pub method: String,
    /// Wildcard pattern (`https://api.example.com/v1/*`) or, with `regex`, a
    /// regular expression searched in the URL. A pattern without "://" and
    /// "/" is a host pattern (`*.example.com`).
    pub url: String,
    pub regex: bool,
}

impl Default for Matcher {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            method: String::new(),
            url: String::new(),
            regex: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MapLocal {
    #[serde(flatten)]
    pub matcher: Matcher,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Inline body, used when `file` is empty.
    pub body: String,
    /// Path of a file whose contents are the body.
    pub file: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MapRemote {
    #[serde(flatten)]
    pub matcher: Matcher,
    /// Destination: scheme and host are always taken from it; its path and
    /// query replace the original ones only when present.
    pub to: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RewriteTarget {
    #[default]
    RequestHeader,
    RequestBody,
    ResponseHeader,
    ResponseBody,
    ResponseStatus,
}

impl RewriteTarget {
    pub fn is_request(self) -> bool {
        matches!(self, Self::RequestHeader | Self::RequestBody)
    }

    pub fn is_body(self) -> bool {
        matches!(self, Self::RequestBody | Self::ResponseBody)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RewriteAction {
    /// Header: replace all values with `value`. Body: replace it with `value`.
    /// Status: set it to `value`.
    #[default]
    Set,
    /// Header: add another `header: value`.
    Add,
    /// Header: remove it (only values matching `find`, when set). Body: empty it.
    Remove,
    /// Regex-replace `find` with `value` in the header's values or the body.
    Replace,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rewrite {
    #[serde(flatten)]
    pub matcher: Matcher,
    pub target: RewriteTarget,
    pub action: RewriteAction,
    /// Header name, for header targets.
    pub header: String,
    /// Regex for `Replace` (and optionally `Remove`).
    pub find: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BlockRule {
    #[serde(flatten)]
    pub matcher: Matcher,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rules {
    pub map_local: Vec<MapLocal>,
    pub map_remote: Vec<MapRemote>,
    pub rewrite: Vec<Rewrite>,
    pub block: Vec<BlockRule>,
}

impl Rules {
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

    /// Checks every pattern and regex, naming the first rule that is wrong.
    pub fn validate(&self) -> Result<(), String> {
        let matchers = self
            .map_local
            .iter()
            .map(|r| &r.matcher)
            .chain(self.map_remote.iter().map(|r| &r.matcher))
            .chain(self.rewrite.iter().map(|r| &r.matcher))
            .chain(self.block.iter().map(|r| &r.matcher));
        for m in matchers {
            compile_pattern(m).map_err(|e| format!("{}: {e}", label(m)))?;
        }
        for r in &self.rewrite {
            if !r.find.is_empty() {
                Regex::new(&r.find).map_err(|e| format!("{}: {e}", label(&r.matcher)))?;
            }
        }
        for r in &self.map_remote {
            parse_target(&r.to).map_err(|e| format!("{}: {e}", label(&r.matcher)))?;
        }
        Ok(())
    }
}

fn label(m: &Matcher) -> String {
    if m.name.is_empty() {
        format!("rule \"{}\"", m.url)
    } else {
        format!("rule \"{}\"", m.name)
    }
}

/// Turns a wildcard pattern into an anchored regex.
fn wildcard_regex(pattern: &str) -> String {
    let p = pattern.trim();
    // A bare host pattern matches every URL on that host.
    let p = if !p.contains("://") && !p.contains('/') {
        format!("*://{p}/*")
    } else if !p.contains("://") {
        format!("*://{p}")
    } else {
        p.to_string()
    };
    let mut re = String::from("^");
    for c in p.chars() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    re
}

fn compile_pattern(m: &Matcher) -> Result<Regex, regex::Error> {
    if m.regex {
        Regex::new(&m.url)
    } else {
        Regex::new(&format!("(?i){}", wildcard_regex(&m.url)))
    }
}

/// Removes the default port so `https://x.com:443/a` reads `https://x.com/a`.
pub fn normalize_url(url: &str) -> String {
    for (scheme, port) in [("https://", ":443"), ("http://", ":80")] {
        if let Some(rest) = url.strip_prefix(scheme) {
            let end = rest.find(['/', '?']).unwrap_or(rest.len());
            if let Some(host) = rest[..end].strip_suffix(port) {
                return format!("{scheme}{host}{}", &rest[end..]);
            }
        }
    }
    url.to_string()
}

pub(crate) struct CompiledMatcher {
    enabled: bool,
    method: Option<String>,
    pattern: Option<Regex>,
}

impl CompiledMatcher {
    pub(crate) fn new(m: &Matcher) -> Self {
        let method = m.method.trim();
        Self {
            enabled: m.enabled && !m.url.trim().is_empty(),
            method: (!method.is_empty() && method != "*").then(|| method.to_ascii_uppercase()),
            pattern: compile_pattern(m).ok(),
        }
    }

    /// Matches whether or not the matcher is enabled.
    pub(crate) fn always(mut self) -> Self {
        self.enabled = self.pattern.is_some();
        self
    }

    /// Checks the pattern without building a matcher.
    pub(crate) fn compile(m: &Matcher) -> Result<(), regex::Error> {
        compile_pattern(m).map(drop)
    }

    pub(crate) fn matches(&self, method: &str, url: &str) -> bool {
        self.enabled
            && self
                .method
                .as_ref()
                .is_none_or(|m| m.eq_ignore_ascii_case(method))
            && self.pattern.as_ref().is_some_and(|p| p.is_match(url))
    }
}

/// Rules with their patterns compiled. Cheap to share as an `Arc`.
pub struct CompiledRules {
    pub rules: Rules,
    map_local: Vec<CompiledMatcher>,
    map_remote: Vec<CompiledMatcher>,
    rewrite: Vec<(CompiledMatcher, Option<Regex>)>,
    block: Vec<CompiledMatcher>,
}

impl CompiledRules {
    pub fn new(rules: Rules) -> Self {
        Self {
            map_local: rules
                .map_local
                .iter()
                .map(|r| CompiledMatcher::new(&r.matcher))
                .collect(),
            map_remote: rules
                .map_remote
                .iter()
                .map(|r| CompiledMatcher::new(&r.matcher))
                .collect(),
            rewrite: rules
                .rewrite
                .iter()
                .map(|r| {
                    let find = (!r.find.is_empty())
                        .then(|| Regex::new(&r.find).ok())
                        .flatten();
                    (CompiledMatcher::new(&r.matcher), find)
                })
                .collect(),
            block: rules
                .block
                .iter()
                .map(|r| CompiledMatcher::new(&r.matcher))
                .collect(),
            rules,
        }
    }

    pub fn is_blocked(&self, method: &str, url: &str) -> bool {
        self.block.iter().any(|m| m.matches(method, url))
    }

    /// The first matching Map Local rule.
    pub fn map_local(&self, method: &str, url: &str) -> Option<&MapLocal> {
        let i = self.map_local.iter().position(|m| m.matches(method, url))?;
        Some(&self.rules.map_local[i])
    }

    /// The first matching Map Remote rule.
    pub fn map_remote(&self, method: &str, url: &str) -> Option<&MapRemote> {
        let i = self
            .map_remote
            .iter()
            .position(|m| m.matches(method, url))?;
        Some(&self.rules.map_remote[i])
    }

    /// Matching rewrite rules for one side, in order, with their `find` regex.
    pub fn rewrites(
        &self,
        method: &str,
        url: &str,
        request: bool,
    ) -> Vec<(&Rewrite, Option<&Regex>)> {
        self.rewrite
            .iter()
            .zip(&self.rules.rewrite)
            .filter(|((m, _), r)| r.target.is_request() == request && m.matches(method, url))
            .map(|((_, find), r)| (r, find.as_ref()))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestParts {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseParts {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

fn edit_headers(headers: &mut Vec<(String, String)>, r: &Rewrite, find: Option<&Regex>) {
    let name = r.header.trim();
    if name.is_empty() {
        return;
    }
    let is = |k: &str| k.eq_ignore_ascii_case(name);
    match r.action {
        RewriteAction::Set => {
            headers.retain(|(k, _)| !is(k));
            headers.push((name.to_string(), r.value.clone()));
        }
        RewriteAction::Add => headers.push((name.to_string(), r.value.clone())),
        RewriteAction::Remove => {
            headers.retain(|(k, v)| !(is(k) && find.is_none_or(|f| f.is_match(v))))
        }
        RewriteAction::Replace => {
            if let Some(find) = find {
                for (_, v) in headers.iter_mut().filter(|(k, _)| is(k)) {
                    *v = find.replace_all(v, r.value.as_str()).into_owned();
                }
            }
        }
    }
}

fn edit_body(body: &mut Vec<u8>, r: &Rewrite, find: Option<&Regex>) {
    match r.action {
        RewriteAction::Set => *body = r.value.as_bytes().to_vec(),
        RewriteAction::Remove => body.clear(),
        RewriteAction::Replace => {
            // Only text bodies can be searched.
            if let (Some(find), Ok(text)) = (find, std::str::from_utf8(body)) {
                *body = find
                    .replace_all(text, r.value.as_str())
                    .into_owned()
                    .into_bytes();
            }
        }
        RewriteAction::Add => body.extend_from_slice(r.value.as_bytes()),
    }
}

pub fn rewrite_request(req: &mut RequestParts, rules: &[(&Rewrite, Option<&Regex>)]) {
    for (r, find) in rules {
        match r.target {
            RewriteTarget::RequestHeader => edit_headers(&mut req.headers, r, *find),
            RewriteTarget::RequestBody => edit_body(&mut req.body, r, *find),
            _ => {}
        }
    }
}

pub fn rewrite_response(res: &mut ResponseParts, rules: &[(&Rewrite, Option<&Regex>)]) {
    for (r, find) in rules {
        match r.target {
            RewriteTarget::ResponseHeader => edit_headers(&mut res.headers, r, *find),
            RewriteTarget::ResponseBody => edit_body(&mut res.body, r, *find),
            RewriteTarget::ResponseStatus => {
                if let Ok(s) = r.value.trim().parse::<u16>()
                    && (100..=999).contains(&s)
                {
                    res.status = s;
                }
            }
            _ => {}
        }
    }
}

/// Parses a Map Remote destination into (scheme, authority, path, query).
fn parse_target(to: &str) -> Result<(String, String, Option<String>, Option<String>), String> {
    let to = to.trim();
    let (scheme, rest) = to
        .split_once("://")
        .ok_or_else(|| "the destination needs a scheme, like https://".to_string())?;
    if !matches!(scheme, "http" | "https") {
        return Err(format!("unsupported scheme \"{scheme}\""));
    }
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() {
        return Err("the destination has no host".into());
    }
    let tail = &rest[end..];
    let (path, query) = match tail.split_once('?') {
        Some((p, q)) => (p, Some(q.to_string())),
        None => (tail, None),
    };
    let path = (!path.is_empty() && path != "/").then(|| path.to_string());
    Ok((scheme.to_string(), authority.to_string(), path, query))
}

/// Applies a Map Remote rule to a URL.
pub fn map_remote_url(url: &str, to: &str) -> Option<String> {
    let (scheme, authority, path, query) = parse_target(to).ok()?;
    let rest = url.split_once("://")?.1;
    let start = rest.find(['/', '?']).unwrap_or(rest.len());
    let tail = &rest[start..];
    let (orig_path, orig_query) = match tail.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (tail, None),
    };
    let path = path
        .as_deref()
        .unwrap_or(if orig_path.is_empty() { "/" } else { orig_path });
    let query = query.as_deref().or(orig_query);
    Some(match query {
        Some(q) => format!("{scheme}://{authority}{path}?{q}"),
        None => format!("{scheme}://{authority}{path}"),
    })
}

/// A Content-Type guess from a file extension, for Map Local files.
pub fn content_type_for(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(url: &str) -> Matcher {
        Matcher {
            url: url.into(),
            ..Default::default()
        }
    }

    fn matches(pattern: Matcher, method: &str, url: &str) -> bool {
        CompiledMatcher::new(&pattern).matches(method, url)
    }

    #[test]
    fn wildcard_and_host_patterns() {
        assert!(matches(
            m("https://api.x.com/v1/*"),
            "GET",
            "https://api.x.com/v1/users?a=1"
        ));
        assert!(!matches(
            m("https://api.x.com/v1/*"),
            "GET",
            "https://api.x.com/v2/users"
        ));
        assert!(matches(
            m("api.x.com/v?/users"),
            "GET",
            "http://api.x.com/v2/users"
        ));
        assert!(matches(m("*.x.com"), "GET", "https://cdn.x.com/a.png"));
        assert!(!matches(m("*.x.com"), "GET", "https://x.com/a.png"));
        assert!(matches(
            m("HTTPS://API.X.COM/*"),
            "GET",
            "https://api.x.com/"
        ));
        assert!(
            !matches(m("a.com/x.json"), "GET", "https://a.com/xxjson"),
            "dots are literal"
        );
    }

    #[test]
    fn regex_method_and_enabled() {
        let re = Matcher {
            regex: true,
            url: r"/users/\d+$".into(),
            ..Default::default()
        };
        assert!(matches(re.clone(), "GET", "https://a.com/users/42"));
        assert!(!matches(re.clone(), "GET", "https://a.com/users/me"));
        let post = Matcher {
            method: "post".into(),
            ..m("*")
        };
        assert!(matches(post.clone(), "POST", "https://a.com/"));
        assert!(!matches(post, "GET", "https://a.com/"));
        let off = Matcher {
            enabled: false,
            ..m("*")
        };
        assert!(!matches(off, "GET", "https://a.com/"));
        assert!(
            !matches(m(""), "GET", "https://a.com/"),
            "an empty pattern matches nothing"
        );
    }

    #[test]
    fn normalizes_default_ports() {
        assert_eq!(normalize_url("https://x.com:443/a?b"), "https://x.com/a?b");
        assert_eq!(normalize_url("http://x.com:80"), "http://x.com");
        assert_eq!(
            normalize_url("https://x.com:8443/a"),
            "https://x.com:8443/a"
        );
        assert_eq!(normalize_url("http://x.com:443/a"), "http://x.com:443/a");
    }

    #[test]
    fn map_remote_keeps_what_the_destination_leaves_out() {
        assert_eq!(
            map_remote_url("https://api.x.com/v1/u?id=1", "http://localhost:3000").as_deref(),
            Some("http://localhost:3000/v1/u?id=1")
        );
        assert_eq!(
            map_remote_url("https://api.x.com/v1/u?id=1", "https://staging.x.com/v2/u").as_deref(),
            Some("https://staging.x.com/v2/u?id=1")
        );
        assert_eq!(
            map_remote_url("https://api.x.com/v1/u?id=1", "https://s.x.com/?mock=1").as_deref(),
            Some("https://s.x.com/v1/u?mock=1")
        );
        assert_eq!(map_remote_url("https://a.com/", "ftp://b").as_deref(), None);
    }

    fn rw(
        target: RewriteTarget,
        action: RewriteAction,
        header: &str,
        find: &str,
        value: &str,
    ) -> Rewrite {
        Rewrite {
            matcher: m("*"),
            target,
            action,
            header: header.into(),
            find: find.into(),
            value: value.into(),
        }
    }

    fn apply_res(res: &mut ResponseParts, rules: &[Rewrite]) {
        let compiled = CompiledRules::new(Rules {
            rewrite: rules.to_vec(),
            ..Default::default()
        });
        rewrite_response(res, &compiled.rewrites("GET", "https://a.com/", false));
    }

    #[test]
    fn rewrites_response_headers_body_and_status() {
        use RewriteAction::*;
        use RewriteTarget::*;
        let mut res = ResponseParts {
            status: 200,
            headers: vec![
                ("Cache-Control".into(), "max-age=60".into()),
                ("Set-Cookie".into(), "a=1".into()),
                ("Set-Cookie".into(), "b=2".into()),
            ],
            body: br#"{"plan":"free","n":1}"#.to_vec(),
        };
        apply_res(
            &mut res,
            &[
                rw(ResponseHeader, Set, "cache-control", "", "no-store"),
                rw(ResponseHeader, Add, "X-Debug", "", "1"),
                rw(ResponseHeader, Remove, "set-cookie", "^b=", ""),
                rw(ResponseBody, Replace, "", r#""free""#, r#""pro""#),
                rw(ResponseStatus, Set, "", "", "503"),
                rw(ResponseStatus, Set, "", "", "nope"),
            ],
        );
        assert_eq!(res.status, 503);
        assert_eq!(
            res.headers,
            vec![
                ("Set-Cookie".into(), "a=1".into()),
                ("cache-control".into(), "no-store".into()),
                ("X-Debug".into(), "1".into()),
            ]
        );
        assert_eq!(res.body, br#"{"plan":"pro","n":1}"#);
    }

    #[test]
    fn rewrites_requests_only_with_request_rules() {
        use RewriteAction::*;
        use RewriteTarget::*;
        let compiled = CompiledRules::new(Rules {
            rewrite: vec![
                rw(
                    RequestHeader,
                    Replace,
                    "authorization",
                    "Bearer .*",
                    "Bearer test",
                ),
                rw(RequestBody, Set, "", "", "{}"),
                rw(ResponseBody, Set, "", "", "ignored"),
            ],
            ..Default::default()
        });
        let mut req = RequestParts {
            method: "POST".into(),
            url: "https://a.com/".into(),
            headers: vec![("Authorization".into(), "Bearer real".into())],
            body: b"{\"x\":1}".to_vec(),
        };
        let rules = compiled.rewrites("POST", "https://a.com/", true);
        assert_eq!(rules.len(), 2);
        rewrite_request(&mut req, &rules);
        assert_eq!(req.headers[0].1, "Bearer test");
        assert_eq!(req.body, b"{}");
    }

    #[test]
    fn binary_bodies_are_not_regex_replaced() {
        let mut res = ResponseParts {
            status: 200,
            headers: vec![],
            body: vec![0xff, b'a'],
        };
        apply_res(
            &mut res,
            &[rw(
                RewriteTarget::ResponseBody,
                RewriteAction::Replace,
                "",
                "a",
                "b",
            )],
        );
        assert_eq!(res.body, vec![0xff, b'a']);
    }

    #[test]
    fn first_match_wins() {
        let rules = CompiledRules::new(Rules {
            map_local: vec![
                MapLocal {
                    matcher: m("https://a.com/x"),
                    status: 201,
                    ..Default::default()
                },
                MapLocal {
                    matcher: m("*"),
                    status: 202,
                    ..Default::default()
                },
            ],
            block: vec![BlockRule {
                matcher: m("*/ads/*"),
            }],
            ..Default::default()
        });
        assert_eq!(
            rules.map_local("GET", "https://a.com/x").unwrap().status,
            201
        );
        assert_eq!(
            rules.map_local("GET", "https://b.com/").unwrap().status,
            202
        );
        assert!(rules.is_blocked("GET", "https://b.com/ads/1.js"));
        assert!(!rules.is_blocked("GET", "https://b.com/app.js"));
    }

    #[test]
    fn validate_names_the_bad_rule() {
        let bad = Rules {
            rewrite: vec![Rewrite {
                matcher: Matcher {
                    name: "Fix plan".into(),
                    ..m("*")
                },
                find: "(".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(bad.validate().unwrap_err().contains("Fix plan"));
        let remote = Rules {
            map_remote: vec![MapRemote {
                matcher: m("*"),
                to: "localhost:3000".into(),
            }],
            ..Default::default()
        };
        assert!(remote.validate().is_err());
        assert!(Rules::default().validate().is_ok());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        let rules = Rules {
            map_remote: vec![MapRemote {
                matcher: m("api.x.com"),
                to: "http://localhost:3000".into(),
            }],
            ..Default::default()
        };
        rules.save(&path).unwrap();
        assert_eq!(Rules::load(&path), rules);
        let json = fs::read_to_string(&path).unwrap();
        assert!(
            json.contains("\"url\": \"api.x.com\""),
            "matcher fields are flattened: {json}"
        );
    }

    #[test]
    fn guesses_content_types() {
        assert_eq!(content_type_for("/tmp/mock.JSON"), Some("application/json"));
        assert_eq!(content_type_for("/tmp/file"), None);
    }
}
