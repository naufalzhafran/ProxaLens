//! JavaScript scripting, a port of httpmon's `internal/scripting` on QuickJS.
//!
//! Scripts are `.js` files in the scripts directory with a YAML-ish header:
//!
//! ```js
//! // ---
//! // id: 3f2a…
//! // name: Add auth header
//! // match:
//! //   - "*://api.example.com/*"
//! // enabled: true
//! // ---
//! function onRequest(ctx) { ctx.headers["Authorization"] = "Bearer test"; }
//! function onResponse(ctx) { ctx.body = ctx.body.replace("free", "pro"); }
//! ```
//!
//! `ctx` in `onRequest` has `method`, `url`, `headers`, `body` and `blocked`;
//! in `onResponse` it has `status`, `headers` and `body` (plus read-only
//! `method` and `url`). Both have `respondWith({status, headers, body} |
//! {file})`, `readFile(path)` and `breakpoint()`, and `console.log` writes to
//! the script console. Each call gets a fresh QuickJS runtime with a memory
//! limit and a CPU-time limit (time spent paused at a breakpoint excluded).

use std::collections::VecDeque;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use rquickjs::{Context, Ctx, Function, Runtime, Value};
use serde::Serialize;
use serde_json::{Map, Value as Json};
use tokio::sync::broadcast;

use super::breakpoint::{Breakpoints, Edits, Hit, Phase, Resume};
use super::proxy::now_ms;
use super::rules::{RequestParts, ResponseParts};
use super::store::FlowId;

const DELIMITER: &str = "// ---";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const CONSOLE_CAP: usize = 500;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptMeta {
    pub id: String,
    pub name: String,
    pub matches: Vec<String>,
    pub enabled: bool,
}

/// Parses the `// ---` header. Returns the metadata; the header lines are
/// comments, so the whole file is still valid JavaScript.
pub fn parse_header(source: &str) -> Result<ScriptMeta, String> {
    let lines: Vec<&str> = source.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim() == DELIMITER)
        .ok_or("missing header: no opening // --- line")?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim() == DELIMITER)
        .map(|i| i + start + 1)
        .ok_or("missing header: no closing // --- line")?;

    let mut meta = ScriptMeta {
        enabled: true,
        ..Default::default()
    };
    let mut in_match = false;
    for raw in &lines[start + 1..end] {
        let line = raw.trim_start();
        let line = line.strip_prefix("//").unwrap_or(line);
        let line = line.strip_prefix(' ').unwrap_or(line);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or(trimmed.strip_prefix('-').filter(|s| s.is_empty()))
        {
            if !in_match {
                return Err(format!("invalid header line: {trimmed}"));
            }
            if !item.trim().is_empty() {
                meta.matches.push(unquote(item.trim()));
            }
            continue;
        }
        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("invalid header line: {trimmed}"))?;
        let value = value.trim();
        in_match = false;
        match key.trim() {
            "id" => meta.id = unquote(value),
            "name" => meta.name = unquote(value),
            "enabled" => {
                meta.enabled = match value {
                    "true" | "yes" | "" => true,
                    "false" | "no" => false,
                    v => return Err(format!("enabled must be true or false, not {v}")),
                }
            }
            "match" => {
                if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                    meta.matches.extend(
                        list.split(',')
                            .map(|s| unquote(s.trim()))
                            .filter(|s| !s.is_empty()),
                    );
                } else if value.is_empty() {
                    in_match = true;
                } else {
                    meta.matches.push(unquote(value));
                }
            }
            _ => {}
        }
    }
    if meta.name.is_empty() {
        return Err("header missing required field: name".into());
    }
    if meta.matches.is_empty() {
        return Err("header missing required field: match".into());
    }
    Ok(meta)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    s.to_string()
}

/// `*` and `?` wildcard match, case-insensitive.
fn wildcard(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = s.to_lowercase().chars().collect();
    let (mut pi, mut si) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// Glob match on `scheme://host/path` (port of httpmon's `GlobMatch`): the
/// scheme may be `*`; `*` in the host never crosses into the path; `*` in
/// the path matches anything. A pattern without a scheme matches any scheme,
/// and `*` alone matches every URL.
pub fn glob_match(pattern: &str, url: &str) -> bool {
    let pattern = pattern.trim();
    if pattern == "*" {
        return true;
    }
    let split = |s: &str| -> (Option<String>, String, String) {
        let (scheme, rest) = match s.split_once("://") {
            Some((a, b)) => (Some(a.to_lowercase()), b.to_string()),
            None => (None, s.to_string()),
        };
        match rest.find('/') {
            Some(i) => (scheme, rest[..i].to_string(), rest[i..].to_string()),
            None => (scheme, rest, String::new()),
        }
    };
    let (ps, ph, pp) = split(pattern);
    let (us, uh, up) = split(url);
    if let Some(ps) = ps
        && ps != "*"
        && Some(ps) != us
    {
        return false;
    }
    wildcard(&ph, &uh) && wildcard(&pp, &up)
}

/// What the script uses, for the UI (port of `DetectCategories`).
pub fn categories(source: &str) -> Vec<&'static str> {
    let mut cats = Vec::new();
    if source.contains("ctx.breakpoint(") {
        cats.push("Breakpoint");
    }
    if source.contains("ctx.respondWith(") {
        cats.push("Map Local");
    }
    if cats.is_empty() {
        cats.push("Script");
    }
    cats
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seed = format!(
        "{}-{}-{}",
        now_ms(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    sha1_smol::Sha1::from(seed).digest().to_string()[..16].to_string()
}

/// Adds `// id: …` after the opening delimiter when a script has none.
fn with_id(source: &str, id: &str) -> String {
    let mut out = Vec::new();
    let mut done = false;
    for line in source.split('\n') {
        out.push(line.to_string());
        if !done && line.trim() == DELIMITER {
            out.push(format!("// id: {id}"));
            done = true;
        }
    }
    out.join("\n")
}

/// Flips (or inserts) the `// enabled:` line (port of `ToggleEnabled`).
pub fn set_enabled(source: &str, enabled: bool) -> String {
    let mut lines: Vec<String> = source.split('\n').map(str::to_string).collect();
    let mut delimiters = 0;
    let mut closing = None;
    for (i, line) in lines.iter_mut().enumerate() {
        let t = line.trim();
        if t == DELIMITER {
            delimiters += 1;
            if delimiters == 2 {
                closing = Some(i);
                break;
            }
        } else if delimiters == 1 && t.starts_with("// enabled:") {
            *line = format!("// enabled: {enabled}");
            return lines.join("\n");
        }
    }
    if let Some(i) = closing {
        lines.insert(i, format!("// enabled: {enabled}"));
    }
    lines.join("\n")
}

pub fn template(name: &str, matches: &str, body: &str) -> String {
    format!(
        "// ---\n// id: {}\n// name: {name}\n// match:\n//   - \"{matches}\"\n// enabled: true\n// ---\n\n{body}",
        new_id()
    )
}

#[derive(Clone, Debug)]
struct Loaded {
    file_name: String,
    path: PathBuf,
    source: String,
    meta: Result<ScriptMeta, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptInfo {
    pub file_name: String,
    pub path: String,
    pub id: String,
    pub name: String,
    pub matches: Vec<String>,
    pub enabled: bool,
    pub error: Option<String>,
    pub categories: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: u64,
    pub at: f64,
    pub script: String,
    /// "log" | "info" | "warn" | "error"
    pub level: String,
    pub message: String,
}

/// A script that will run for a flow.
#[derive(Clone)]
pub struct Runnable {
    name: String,
    source: String,
    dir: PathBuf,
}

/// A response produced by `ctx.respondWith`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Responded(pub ResponseParts);

#[derive(Debug, thiserror::Error)]
pub enum ScriptFileError {
    #[error("invalid script file name \"{0}\"")]
    BadName(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Scripts {
    dir: PathBuf,
    loaded: RwLock<Arc<Vec<Loaded>>>,
    console: Mutex<VecDeque<LogEntry>>,
    next_log: AtomicU64,
    changed: broadcast::Sender<()>,
    timeout: Duration,
}

impl Scripts {
    pub fn new(dir: impl Into<PathBuf>, timeout: Duration) -> Self {
        let s = Self {
            dir: dir.into(),
            loaded: RwLock::new(Arc::new(Vec::new())),
            console: Mutex::new(VecDeque::new()),
            next_log: AtomicU64::new(1),
            changed: broadcast::channel(64).0,
            timeout,
        };
        s.reload();
        s
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Fires when scripts or the console change.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    /// Re-reads every `.js` file, giving scripts without an id one.
    pub fn reload(&self) {
        let mut loaded = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".js"))
                .collect();
            names.sort();
            for file_name in names {
                let path = self.dir.join(&file_name);
                let mut source = match fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        loaded.push(Loaded {
                            file_name,
                            path,
                            source: String::new(),
                            meta: Err(e.to_string()),
                        });
                        continue;
                    }
                };
                let mut meta = parse_header(&source);
                if let Ok(m) = &mut meta
                    && m.id.is_empty()
                {
                    m.id = new_id();
                    source = with_id(&source, &m.id);
                    let _ = fs::write(&path, &source);
                }
                loaded.push(Loaded {
                    file_name,
                    path,
                    source,
                    meta,
                });
            }
        }
        *self.loaded.write() = Arc::new(loaded);
        let _ = self.changed.send(());
    }

    pub fn list(&self) -> Vec<ScriptInfo> {
        self.loaded
            .read()
            .iter()
            .map(|l| {
                let (meta, error) = match &l.meta {
                    Ok(m) => (m.clone(), None),
                    Err(e) => (
                        ScriptMeta {
                            name: l.file_name.trim_end_matches(".js").to_string(),
                            ..Default::default()
                        },
                        Some(e.clone()),
                    ),
                };
                ScriptInfo {
                    file_name: l.file_name.clone(),
                    path: l.path.to_string_lossy().into_owned(),
                    id: meta.id,
                    name: meta.name,
                    matches: meta.matches,
                    enabled: meta.enabled && error.is_none(),
                    error,
                    categories: categories(&l.source),
                }
            })
            .collect()
    }

    fn path_for(&self, file_name: &str) -> Result<PathBuf, ScriptFileError> {
        let ok = file_name.ends_with(".js")
            && !file_name.starts_with('.')
            && file_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '));
        if !ok {
            return Err(ScriptFileError::BadName(file_name.into()));
        }
        Ok(self.dir.join(file_name))
    }

    pub fn read(&self, file_name: &str) -> Result<String, ScriptFileError> {
        Ok(fs::read_to_string(self.path_for(file_name)?)?)
    }

    /// Writes a script (creating it if needed) and reloads.
    pub fn save(&self, file_name: &str, source: &str) -> Result<(), ScriptFileError> {
        let path = self.path_for(file_name)?;
        fs::create_dir_all(&self.dir)?;
        fs::write(path, source)?;
        self.reload();
        Ok(())
    }

    /// Creates a new script file and returns its name.
    pub fn create(&self, source: &str) -> Result<String, ScriptFileError> {
        fs::create_dir_all(&self.dir)?;
        let id = parse_header(source)
            .map(|m| m.id)
            .ok()
            .filter(|i| !i.is_empty())
            .unwrap_or_else(new_id);
        let mut name = format!("script-{}.js", &id[..id.len().min(8)]);
        let mut n = 1;
        while self.dir.join(&name).exists() {
            n += 1;
            name = format!("script-{}-{n}.js", &id[..id.len().min(8)]);
        }
        self.save(&name, source)?;
        Ok(name)
    }

    pub fn delete(&self, file_name: &str) -> Result<(), ScriptFileError> {
        fs::remove_file(self.path_for(file_name)?)?;
        self.reload();
        Ok(())
    }

    pub fn set_enabled(&self, file_name: &str, enabled: bool) -> Result<(), ScriptFileError> {
        let source = self.read(file_name)?;
        self.save(file_name, &set_enabled(&source, enabled))
    }

    /// Enabled scripts matching `url` that define the phase's handler.
    pub fn matching(&self, url: &str, phase: Phase) -> Vec<Runnable> {
        let handler = match phase {
            Phase::Request => "onRequest",
            Phase::Response => "onResponse",
        };
        self.loaded
            .read()
            .iter()
            .filter_map(|l| {
                let m = l.meta.as_ref().ok()?;
                (m.enabled
                    && l.source.contains(handler)
                    && m.matches.iter().any(|p| glob_match(p, url)))
                .then(|| Runnable {
                    name: m.name.clone(),
                    source: l.source.clone(),
                    dir: self.dir.clone(),
                })
            })
            .collect()
    }

    pub fn console(&self) -> Vec<LogEntry> {
        self.console.lock().iter().cloned().collect()
    }

    pub fn clear_console(&self) {
        self.console.lock().clear();
        let _ = self.changed.send(());
    }

    pub fn log(&self, script: &str, level: &str, message: impl Into<String>) {
        let entry = LogEntry {
            id: self.next_log.fetch_add(1, Ordering::Relaxed),
            at: now_ms(),
            script: script.to_string(),
            level: level.to_string(),
            message: message.into(),
        };
        {
            let mut c = self.console.lock();
            if c.len() >= CONSOLE_CAP {
                c.pop_front();
            }
            c.push_back(entry);
        }
        let _ = self.changed.send(());
    }

    /// Runs `onRequest` of each script in order. Blocking: call it from
    /// `spawn_blocking`. Stops early when a script blocks the request or
    /// answers it with `respondWith`.
    pub fn run_request(
        &self,
        scripts: &[Runnable],
        req: &mut RequestParts,
        bp: Option<&PauseWith>,
    ) -> RequestOutcome {
        for s in scripts {
            let input = serde_json::json!({
                "method": req.method,
                "url": req.url,
                "headers": headers_to_js(&req.headers),
                "body": body_to_js(&req.body),
                "blocked": false,
            });
            let Some(out) = self.call(s, Phase::Request, &input, bp) else {
                continue;
            };
            if let Some(r) = responded(&out, &s.dir) {
                return RequestOutcome::Respond(r);
            }
            if let Some(m) = out.get("method").and_then(Json::as_str) {
                req.method = m.to_ascii_uppercase();
            }
            if let Some(u) = out.get("url").and_then(Json::as_str) {
                req.url = u.to_string();
            }
            apply_headers(&mut req.headers, &input["headers"], &out["headers"]);
            apply_body(&mut req.body, &input["body"], &out["body"]);
            if out.get("blocked").and_then(Json::as_bool) == Some(true) {
                return RequestOutcome::Block;
            }
        }
        RequestOutcome::Continue
    }

    /// Runs `onResponse` of each script in order (blocking).
    pub fn run_response(
        &self,
        scripts: &[Runnable],
        method: &str,
        url: &str,
        res: &mut ResponseParts,
        bp: Option<&PauseWith>,
    ) -> Option<Responded> {
        for s in scripts {
            let input = serde_json::json!({
                "method": method,
                "url": url,
                "status": res.status,
                "headers": headers_to_js(&res.headers),
                "body": body_to_js(&res.body),
            });
            let Some(out) = self.call(s, Phase::Response, &input, bp) else {
                continue;
            };
            if let Some(r) = responded(&out, &s.dir) {
                return Some(r);
            }
            if let Some(st) = out.get("status").and_then(Json::as_u64)
                && (100..=999).contains(&st)
            {
                res.status = st as u16;
            }
            apply_headers(&mut res.headers, &input["headers"], &out["headers"]);
            apply_body(&mut res.body, &input["body"], &out["body"]);
        }
        None
    }

    /// Runs one handler; logs and returns `None` on errors or when the
    /// script doesn't define it.
    fn call(
        &self,
        s: &Runnable,
        phase: Phase,
        input: &Json,
        bp: Option<&PauseWith>,
    ) -> Option<Json> {
        match run_js(self, s, phase, input, bp, self.timeout) {
            Ok(Some(out)) => serde_json::from_str(&out).ok(),
            Ok(None) => None,
            Err(e) => {
                self.log(&s.name, "error", e);
                None
            }
        }
    }
}

/// What `ctx.breakpoint()` needs to pause a flow from the script's thread.
#[derive(Clone)]
pub struct PauseWith {
    pub breakpoints: Arc<Breakpoints>,
    pub flow_id: FlowId,
    pub handle: tokio::runtime::Handle,
}

pub enum RequestOutcome {
    Continue,
    Block,
    Respond(Responded),
}

/// Single value per header name, first one wins (as httpmon does).
fn headers_to_js(headers: &[(String, String)]) -> Json {
    let mut map = Map::new();
    for (k, v) in headers {
        if !map.keys().any(|existing| existing.eq_ignore_ascii_case(k)) {
            map.insert(k.clone(), Json::String(v.clone()));
        }
    }
    Json::Object(map)
}

/// Applies the script's header object, keeping repeated headers (like
/// Set-Cookie) intact unless the script changed or removed them.
fn apply_headers(headers: &mut Vec<(String, String)>, before: &Json, after: &Json) {
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else {
        return;
    };
    if before == after {
        return;
    }
    let value = |o: &Map<String, Json>, k: &str| -> Option<String> {
        o.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str().map_or_else(|| v.to_string(), str::to_string))
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (k, v) in headers.iter() {
        let lk = k.to_ascii_lowercase();
        match (value(before, k), value(after, k)) {
            (_, None) => {}
            (Some(b), Some(a)) if a == b => out.push((k.clone(), v.clone())),
            (_, Some(a)) => {
                if !seen.contains(&lk) {
                    out.push((k.clone(), a));
                }
            }
        }
        seen.push(lk);
    }
    for (k, v) in after {
        if !seen.contains(&k.to_ascii_lowercase()) {
            out.push((
                k.clone(),
                v.as_str().map_or_else(|| v.to_string(), str::to_string),
            ));
        }
    }
    *headers = out;
}

fn body_to_js(body: &[u8]) -> Json {
    match std::str::from_utf8(body) {
        Ok(s) => Json::String(s.to_string()),
        Err(_) => Json::Null,
    }
}

fn apply_body(body: &mut Vec<u8>, before: &Json, after: &Json) {
    if after != before
        && let Some(s) = after.as_str()
    {
        *body = s.as_bytes().to_vec();
    }
}

fn responded(out: &Json, dir: &Path) -> Option<Responded> {
    let r = out.get("responded")?.as_object()?;
    let mut headers: Vec<(String, String)> = r
        .get("headers")
        .and_then(Json::as_object)
        .map(|h| {
            h.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map_or_else(|| v.to_string(), str::to_string),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let body = match r.get("file").and_then(Json::as_str) {
        Some(file) => {
            let path = resolve(dir, file)?;
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            {
                let ct = super::rules::content_type_for(file).unwrap_or("application/octet-stream");
                headers.push(("Content-Type".into(), ct.into()));
            }
            fs::read(path).unwrap_or_default()
        }
        None => r
            .get("body")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };
    let status = r
        .get("status")
        .and_then(Json::as_u64)
        .filter(|s| (100..=999).contains(s))
        .unwrap_or(200) as u16;
    Some(Responded(ResponseParts {
        status,
        headers,
        body,
    }))
}

/// Resolves a path for `readFile`/`respondWith({file})`: relative paths stay
/// inside the scripts directory; absolute paths are allowed (as in httpmon,
/// whose Map Local scripts point at files anywhere).
fn resolve(dir: &Path, path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    if p.components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(dir.join(p))
}

const PRELUDE: &str = r#"
(function () {
  const fmt = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Error) return String(v);
    try { return JSON.stringify(v); } catch (e) { return String(v); }
  };
  const log = (level) => (...args) => __log(level, args.map(fmt).join(" "));
  globalThis.console = { log: log("log"), info: log("info"), debug: log("log"), warn: log("warn"), error: log("error") };
  const RESPOND = {};
  globalThis.__run = function (phase, input) {
    const ctx = JSON.parse(input);
    let responded = null;
    ctx.respondWith = function (opts) {
      opts = opts || {};
      const r = { status: opts.status === undefined ? 200 : Number(opts.status) };
      if (opts.file !== undefined) {
        r.file = String(opts.file);
      } else {
        r.body = opts.body === undefined ? "" : typeof opts.body === "string" ? opts.body : JSON.stringify(opts.body);
      }
      if (opts.headers) {
        r.headers = {};
        for (const k of Object.keys(opts.headers)) r.headers[k] = String(opts.headers[k]);
      }
      responded = r;
      if (phase === "request") throw RESPOND;
    };
    ctx.readFile = (path) => { const v = __readFile(String(path)); return v === undefined ? null : v; };
    ctx.breakpoint = function () {
      const out = __breakpoint(JSON.stringify({
        method: ctx.method, url: ctx.url, status: ctx.status === undefined ? null : ctx.status,
        headers: ctx.headers, body: typeof ctx.body === "string" ? ctx.body : null,
      }));
      if (out == null) return;
      const e = JSON.parse(out);
      if (phase === "request") { ctx.method = e.method; ctx.url = e.url; } else { ctx.status = e.status; }
      ctx.headers = e.headers;
      if (e.body !== null) ctx.body = e.body;
    };
    const fn = phase === "request" ? globalThis.onRequest : globalThis.onResponse;
    if (typeof fn !== "function") return null;
    try { fn(ctx); } catch (e) { if (e !== RESPOND) throw e; }
    return JSON.stringify({
      responded, method: ctx.method, url: ctx.url, status: ctx.status,
      headers: ctx.headers, body: ctx.body === undefined ? null : ctx.body, blocked: !!ctx.blocked,
    });
  };
})();
"#;

fn error_text(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if !matches!(e, rquickjs::Error::Exception) {
        return e.to_string();
    }
    let v: Value = ctx.catch();
    if let Some(ex) = v.as_exception() {
        let msg = ex.message().unwrap_or_default();
        return match ex.stack() {
            Some(stack) if !stack.trim().is_empty() => format!("{msg}\n{}", stack.trim_end()),
            _ => msg,
        };
    }
    if let Some(s) = v.as_string().and_then(|s| s.to_string().ok()) {
        return s;
    }
    if v.is_null() || v.is_undefined() {
        return "the script was interrupted (it ran longer than the time limit)".into();
    }
    format!("{v:?}")
}

/// Evaluates the script in a fresh runtime and calls its handler. Returns
/// the JSON result, `None` when the handler is missing, or an error.
fn run_js(
    scripts: &Scripts,
    s: &Runnable,
    phase: Phase,
    input: &Json,
    bp: Option<&PauseWith>,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(MEMORY_LIMIT);
    rt.set_max_stack_size(1024 * 1024);
    let deadline = Arc::new(Mutex::new(Instant::now() + timeout));
    let d = deadline.clone();
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > *d.lock())));
    let context = Context::full(&rt).map_err(|e| e.to_string())?;

    let logs: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let result = context.with(|ctx| {
        let globals = ctx.globals();
        let sink = logs.clone();
        let log = Function::new(ctx.clone(), move |level: String, msg: String| {
            sink.lock().push((level, msg));
        });
        let dir = s.dir.clone();
        let read_file = Function::new(ctx.clone(), move |path: String| -> Option<String> {
            fs::read_to_string(resolve(&dir, &path)?).ok()
        });
        let method = input["method"].as_str().unwrap_or_default().to_string();
        let url = input["url"].as_str().unwrap_or_default().to_string();
        let bp = bp.cloned();
        let deadline = deadline.clone();
        let breakpoint = Function::new(ctx.clone(), move |state: String| -> Option<String> {
            let PauseWith {
                breakpoints,
                flow_id,
                handle,
            } = bp.clone()?;
            let state: Json = serde_json::from_str(&state).ok()?;
            let headers: Vec<(String, String)> = state["headers"]
                .as_object()
                .map(|h| {
                    h.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_string)))
                        .collect()
                })
                .unwrap_or_default();
            let body = state["body"].as_str().unwrap_or_default().as_bytes().to_vec();
            let hit = Hit {
                flow_id,
                phase,
                method: state["method"].as_str().unwrap_or(&method).to_string(),
                url: state["url"].as_str().unwrap_or(&url).to_string(),
                status: state["status"].as_u64().map(|s| s as u16),
                headers: headers.clone(),
                body: body.clone(),
                deadline: 0,
            };
            let resume = handle.block_on(breakpoints.pause(hit));
            // Time spent paused doesn't count against the script.
            *deadline.lock() = Instant::now() + timeout;
            let edits = match resume {
                Resume::Abort | Resume::Continue(None) => return None,
                Resume::Continue(Some(e)) => e,
            };
            let Edits {
                method: m,
                url: u,
                status,
                headers,
                body,
            } = edits;
            Some(
                serde_json::json!({
                    "method": if m.is_empty() { state["method"].clone() } else { Json::String(m) },
                    "url": if u.is_empty() { state["url"].clone() } else { Json::String(u) },
                    "status": if status == 0 { state["status"].clone() } else { Json::from(status) },
                    "headers": headers_to_js(&headers),
                    "body": body_to_js(&body),
                })
                .to_string(),
            )
        });
        let setup = (|| -> rquickjs::Result<()> {
            globals.set("__log", log?)?;
            globals.set("__readFile", read_file?)?;
            globals.set("__breakpoint", breakpoint?)?;
            ctx.eval::<(), _>(PRELUDE)
        })();
        setup.map_err(|e| error_text(&ctx, e))?;
        ctx.eval::<(), _>(s.source.as_str()).map_err(|e| error_text(&ctx, e))?;
        let run: Function = globals.get("__run").map_err(|e| error_text(&ctx, e))?;
        let phase_name = match phase {
            Phase::Request => "request",
            Phase::Response => "response",
        };
        let out: Option<String> = run
            .call((phase_name, input.to_string()))
            .map_err(|e| error_text(&ctx, e))?;
        Ok(out)
    });
    for (level, msg) in logs.lock().drain(..) {
        scripts.log(&s.name, &level, msg);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(body: &str) -> String {
        template("Test", "*", body)
    }

    fn engine(scripts: &[(&str, String)]) -> (Scripts, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        for (name, src) in scripts {
            fs::write(dir.path().join(name), src).unwrap();
        }
        (Scripts::new(dir.path(), Duration::from_secs(2)), dir)
    }

    fn request() -> RequestParts {
        RequestParts {
            method: "GET".into(),
            url: "https://api.example.com/v1/users".into(),
            headers: vec![
                ("Accept".into(), "*/*".into()),
                ("Cookie".into(), "a=1".into()),
                ("Cookie".into(), "b=2".into()),
            ],
            body: Vec::new(),
        }
    }

    fn run_req(s: &Scripts, req: &mut RequestParts) -> RequestOutcome {
        let r = s.matching(&req.url.clone(), Phase::Request);
        s.run_request(&r, req, None)
    }

    // Ported from httpmon internal/scripting/header_test.go.
    #[test]
    fn parses_headers() {
        let m = parse_header("// ---\n// id: abc\n// name: Mock\n// match:\n//   - \"*://a.com/*\"\n//   - '*://b.com/*'\n// enabled: false\n// ---\nfunction onRequest(){}").unwrap();
        assert_eq!(
            m,
            ScriptMeta {
                id: "abc".into(),
                name: "Mock".into(),
                matches: vec!["*://a.com/*".into(), "*://b.com/*".into()],
                enabled: false
            }
        );
        let inline =
            parse_header("// ---\n// name: X\n// match: [\"a\", \"b\"]\n// ---\n").unwrap();
        assert_eq!(
            (inline.matches.len(), inline.enabled),
            (2, true),
            "enabled defaults to true"
        );
        assert!(
            parse_header("function onRequest(){}")
                .unwrap_err()
                .contains("opening")
        );
        assert!(
            parse_header("// ---\n// match: [\"*\"]\n// ---\n")
                .unwrap_err()
                .contains("name")
        );
        assert!(
            parse_header("// ---\n// name: X\n// ---\n")
                .unwrap_err()
                .contains("match")
        );
    }

    // Ported from httpmon internal/scripting/glob_test.go.
    #[test]
    fn glob_matching() {
        let cases = [
            ("http://example.com/api", "http://example.com/api", true),
            ("*://example.com/api", "https://example.com/api", true),
            ("*://example.com/*", "https://example.com/foo/bar", true),
            ("*://example.com/*", "https://example.com/", true),
            ("*://*.example.com/*", "https://api.example.com/v1", true),
            ("*://*.example.com/*", "https://example.com/v1", false),
            ("*://api.example.com/*", "https://other.com/api", false),
            ("*://API.Example.COM/*", "https://api.example.com/foo", true),
            (
                "*://*.example.com/*/users/*",
                "https://api.example.com/v1/users/123",
                true,
            ),
            (
                "*://*.example.com/*/users/*",
                "https://api.example.com/v1/posts/123",
                false,
            ),
            ("*://example.com", "https://example.com", true),
            ("*://example.com", "https://example.com/path", false),
            ("https://example.com/*", "http://example.com/x", false),
            ("example.com/*", "http://example.com/x", true),
            ("*", "https://anything.io/x", true),
        ];
        for (p, u, want) in cases {
            assert_eq!(glob_match(p, u), want, "{p} vs {u}");
        }
    }

    #[test]
    fn detects_categories() {
        assert_eq!(
            categories("ctx.breakpoint(); ctx.respondWith({})"),
            vec!["Breakpoint", "Map Local"]
        );
        assert_eq!(categories(""), vec!["Script"]);
    }

    #[test]
    fn toggles_enabled_lines() {
        let src = "// ---\n// name: X\n// match: [\"*\"]\n// enabled: true\n// ---\nx";
        assert!(set_enabled(src, false).contains("// enabled: false"));
        let without = "// ---\n// name: X\n// match: [\"*\"]\n// ---\nx";
        let toggled = set_enabled(without, false);
        assert!(!parse_header(&toggled).unwrap().enabled);
        assert!(toggled.ends_with("// ---\nx"));
    }

    // Ported from httpmon internal/scripting/scripting_test.go.
    #[test]
    fn modifies_requests() {
        let (s, _d) = engine(&[(
            "a.js",
            script(
                r#"function onRequest(ctx) {
                     ctx.headers["X-Added"] = "yes";
                     delete ctx.headers["Accept"];
                     ctx.method = "post";
                     ctx.body = JSON.stringify({ from: ctx.url });
                   }"#,
            ),
        )]);
        let mut req = request();
        assert!(matches!(run_req(&s, &mut req), RequestOutcome::Continue));
        assert_eq!(req.method, "POST");
        assert_eq!(req.body, br#"{"from":"https://api.example.com/v1/users"}"#);
        assert_eq!(
            req.headers,
            vec![
                ("Cookie".into(), "a=1".into()),
                ("Cookie".into(), "b=2".into()),
                ("X-Added".into(), "yes".into())
            ],
            "untouched repeated headers survive"
        );
    }

    #[test]
    fn modifies_responses() {
        let (s, _d) = engine(&[(
            "a.js",
            script(
                r#"function onResponse(ctx) {
                     const data = JSON.parse(ctx.body);
                     data.plan = "pro";
                     ctx.body = JSON.stringify(data);
                     ctx.status = 201;
                     ctx.headers["X-Url"] = ctx.url;
                   }"#,
            ),
        )]);
        let mut res = ResponseParts {
            status: 200,
            headers: vec![],
            body: br#"{"plan":"free"}"#.to_vec(),
        };
        let r = s.matching("https://a.com/x", Phase::Response);
        assert!(
            s.run_response(&r, "GET", "https://a.com/x", &mut res, None)
                .is_none()
        );
        assert_eq!(
            (res.status, res.body.as_slice()),
            (201, br#"{"plan":"pro"}"#.as_slice())
        );
        assert_eq!(
            res.headers,
            vec![("X-Url".into(), "https://a.com/x".into())]
        );
    }

    #[test]
    fn blocks_requests() {
        let (s, _d) = engine(&[(
            "a.js",
            script("function onRequest(ctx) { ctx.blocked = true; }"),
        )]);
        assert!(matches!(run_req(&s, &mut request()), RequestOutcome::Block));
    }

    #[test]
    fn script_errors_go_to_the_console_and_change_nothing() {
        let (s, _d) = engine(&[(
            "a.js",
            script("function onRequest(ctx) { ctx.method = 'PUT'; undefinedFn(); }"),
        )]);
        let mut req = request();
        run_req(&s, &mut req);
        assert_eq!(req.method, "GET");
        let console = s.console();
        assert_eq!(console.len(), 1);
        assert_eq!(
            (console[0].script.as_str(), console[0].level.as_str()),
            ("Test", "error")
        );
        assert!(
            console[0].message.contains("undefinedFn"),
            "{}",
            console[0].message
        );
    }

    #[test]
    fn console_log_and_runaway_scripts() {
        let (s, _d) = engine(&[
            (
                "a.js",
                script(
                    "function onRequest(ctx) { console.log('hi', {n: 1}); console.warn('careful'); }",
                ),
            ),
            (
                "b.js",
                script("function onRequest(ctx) { while (true) {} }"),
            ),
        ]);
        let mut s = s;
        s.timeout = Duration::from_millis(100);
        let start = Instant::now();
        run_req(&s, &mut request());
        assert!(start.elapsed() < Duration::from_secs(2));
        let msgs: Vec<_> = s
            .console()
            .into_iter()
            .map(|e| (e.level, e.message))
            .collect();
        assert_eq!(msgs[0], ("log".to_string(), r#"hi {"n":1}"#.to_string()));
        assert_eq!(msgs[1].0, "warn");
        assert_eq!(msgs[2].0, "error");
        assert!(
            msgs[2].1.contains("interrupted") || msgs[2].1.contains("time"),
            "{}",
            msgs[2].1
        );
    }

    // Ported from httpmon internal/scripting/respondwith_test.go.
    #[test]
    fn respond_with_body_status_headers_and_file() {
        let (s, d) = engine(&[
            (
                "a.js",
                template(
                    "Inline",
                    "*://a.com/*",
                    r#"function onRequest(ctx) {
                         ctx.respondWith({ status: 418, headers: { "X-Mock": 1 }, body: { ok: true } });
                         ctx.method = "SHOULD NOT RUN";
                       }"#,
                ),
            ),
            (
                "b.js",
                template(
                    "File",
                    "*://b.com/*",
                    r#"function onRequest(ctx) { ctx.respondWith({ file: "mock.json" }); }"#,
                ),
            ),
        ]);
        fs::write(d.path().join("mock.json"), r#"{"from":"file"}"#).unwrap();

        let mut req = request();
        req.url = "https://a.com/x".into();
        let RequestOutcome::Respond(Responded(r)) = run_req(&s, &mut req) else {
            panic!("expected a response");
        };
        assert_eq!(
            (r.status, r.body.as_slice()),
            (418, br#"{"ok":true}"#.as_slice())
        );
        assert_eq!(r.headers, vec![("X-Mock".into(), "1".into())]);
        assert_eq!(req.method, "GET", "respondWith stops the script");

        req.url = "https://b.com/x".into();
        let RequestOutcome::Respond(Responded(r)) = run_req(&s, &mut req) else {
            panic!("expected a response");
        };
        assert_eq!(
            (r.status, r.body.as_slice()),
            (200, br#"{"from":"file"}"#.as_slice())
        );
        assert_eq!(
            r.headers,
            vec![("Content-Type".into(), "application/json".into())]
        );
    }

    #[test]
    fn read_file_is_confined_to_relative_paths_in_the_scripts_dir() {
        let (s, d) = engine(&[(
            "a.js",
            script(
                r#"function onRequest(ctx) {
                     ctx.headers["X-A"] = String(ctx.readFile("data.txt"));
                     ctx.headers["X-B"] = String(ctx.readFile("../secret.txt"));
                     ctx.headers["X-C"] = String(ctx.readFile("missing.txt"));
                   }"#,
            ),
        )]);
        fs::write(d.path().join("data.txt"), "hello").unwrap();
        let mut req = request();
        run_req(&s, &mut req);
        let get = |k: &str| req.headers.iter().find(|(h, _)| h == k).unwrap().1.clone();
        assert_eq!(
            (get("X-A"), get("X-B"), get("X-C")),
            ("hello".into(), "null".into(), "null".into())
        );
    }

    #[test]
    fn disabled_non_matching_and_broken_scripts_are_skipped() {
        let (s, _d) = engine(&[
            (
                "a.js",
                set_enabled(
                    &script("function onRequest(ctx) { ctx.blocked = true; }"),
                    false,
                ),
            ),
            (
                "b.js",
                template(
                    "Other",
                    "*://other.com/*",
                    "function onRequest(ctx) { ctx.blocked = true; }",
                ),
            ),
            (
                "c.js",
                "function onRequest(ctx) { ctx.blocked = true; }".to_string(),
            ),
        ]);
        assert!(matches!(
            run_req(&s, &mut request()),
            RequestOutcome::Continue
        ));
        let list = s.list();
        assert_eq!(list.len(), 3);
        assert!(!list[0].enabled && list[0].error.is_none());
        assert!(list[2].error.as_deref().unwrap().contains("header"));
    }

    // Ported from httpmon internal/scripting/loader_test.go and manager_test.go.
    #[test]
    fn manages_files_and_backfills_ids() {
        let (s, d) = engine(&[(
            "old.js",
            "// ---\n// name: Old\n// match: [\"*\"]\n// ---\n".to_string(),
        )]);
        let on_disk = fs::read_to_string(d.path().join("old.js")).unwrap();
        let id = parse_header(&on_disk).unwrap().id;
        assert_eq!(id.len(), 16, "an id was written back");

        let name = s.create(&template("New", "*", "")).unwrap();
        assert!(name.starts_with("script-") && name.ends_with(".js"));
        assert_eq!(s.list().len(), 2);
        s.set_enabled(&name, false).unwrap();
        assert!(
            !s.list()
                .iter()
                .find(|i| i.file_name == name)
                .unwrap()
                .enabled
        );
        s.delete(&name).unwrap();
        assert_eq!(s.list().len(), 1);
        assert!(s.read("../etc/passwd").is_err());
        assert!(s.save("x/../../y.js", "").is_err());
    }

    // Ported from httpmon internal/scripting/breakpoint_test.go.
    #[tokio::test(flavor = "multi_thread")]
    async fn breakpoint_pauses_and_applies_edits() {
        let (s, _d) = engine(&[(
            "a.js",
            script(
                r#"function onRequest(ctx) {
                     ctx.body = "before";
                     ctx.breakpoint();
                     ctx.headers["X-After"] = ctx.body;
                   }"#,
            ),
        )]);
        let s = Arc::new(s);
        let bps = Arc::new(Breakpoints::new(Duration::from_secs(10)));
        let (s2, b2) = (s.clone(), bps.clone());
        let handle = tokio::runtime::Handle::current();
        let task = tokio::task::spawn_blocking(move || {
            let mut req = request();
            let r = s2.matching(&req.url.clone(), Phase::Request);
            let pause = PauseWith {
                breakpoints: b2,
                flow_id: 7,
                handle,
            };
            s2.run_request(&r, &mut req, Some(&pause));
            req
        });
        let hit = loop {
            if let Some(h) = bps.pending().pop() {
                break h;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(
            (hit.flow_id, hit.body.as_slice()),
            (7, b"before".as_slice())
        );
        bps.resume(
            7,
            Phase::Request,
            Resume::Continue(Some(Edits {
                headers: hit.headers.clone(),
                body: b"edited".to_vec(),
                ..Default::default()
            })),
        );
        let req = task.await.unwrap();
        assert_eq!(req.body, b"edited");
        assert!(req.headers.contains(&("X-After".into(), "edited".into())));
    }
}
