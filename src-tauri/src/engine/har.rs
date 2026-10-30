//! HAR 1.2 export/import (port of httpmon's `internal/har`), plus sessions.
//!
//! A session file is a zstd-compressed HAR. Fields HAR has no room for
//! (flow state, errors, WebSocket messages…) go in a `_proxaLens`
//! extension object, which other tools ignore.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, SecondsFormat};
use hudsucker::hyper::Uri;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::decode;
use super::store::{FlowData, FlowMeta, FlowState, WsDirection, WsMessage};

const CREATOR: &str = "ProxaLens";
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

#[derive(Debug, thiserror::Error)]
pub enum HarError {
    #[error("not a HAR file: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Har {
    pub log: Log,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Log {
    pub version: String,
    pub creator: Creator,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Creator {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Entry {
    pub started_date_time: String,
    pub time: f64,
    pub request: Request,
    pub response: Response,
    pub cache: Value,
    pub timings: Timings,
    // `_projectProxy` is the name sessions used before the app was renamed.
    #[serde(
        rename = "_proxaLens",
        alias = "_projectProxy",
        skip_serializing_if = "Option::is_none"
    )]
    pub extra: Option<Extra>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub http_version: String,
    pub cookies: Vec<Value>,
    pub headers: Vec<NameValue>,
    pub query_string: Vec<NameValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_data: Option<PostData>,
    pub headers_size: i64,
    pub body_size: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Response {
    pub status: i64,
    pub status_text: String,
    pub http_version: String,
    pub cookies: Vec<Value>,
    pub headers: Vec<NameValue>,
    pub content: Content,
    #[serde(rename = "redirectURL")]
    pub redirect_url: String,
    pub headers_size: i64,
    pub body_size: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NameValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PostData {
    pub mime_type: String,
    pub text: String,
    /// Not in HAR 1.2, but common: "base64" for binary bodies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Content {
    pub size: i64,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

/// HAR timings. `blocked`, `dns`, `connect` and `ssl` are -1 when they
/// don't apply; per the spec, `connect` includes `ssl`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Timings {
    pub blocked: f64,
    pub dns: f64,
    pub connect: f64,
    pub send: f64,
    pub wait: f64,
    pub receive: f64,
    pub ssl: f64,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            blocked: -1.0,
            dns: -1.0,
            connect: -1.0,
            send: 0.0,
            wait: 0.0,
            receive: 0.0,
            ssl: -1.0,
        }
    }
}

impl Timings {
    fn from_flow(m: &FlowMeta) -> Self {
        let total = m.duration_ms.unwrap_or(0.0);
        let wait = m.ttfb_ms.unwrap_or(total);
        let t = m.timings.unwrap_or_default();
        let or = |v: Option<f64>| v.unwrap_or(-1.0);
        Self {
            blocked: or(t.blocked),
            dns: or(t.dns),
            connect: t.connect.map_or(-1.0, |c| c + t.tls.unwrap_or(0.0)),
            send: t.send.unwrap_or(0.0),
            wait: t.wait.unwrap_or(wait),
            receive: t.receive.unwrap_or((total - wait).max(0.0)),
            ssl: or(t.tls),
        }
    }

    /// Phases from a HAR another tool wrote; `None` when it recorded none.
    fn to_flow(&self) -> Option<super::store::Timings> {
        let opt = |v: f64| (v >= 0.0).then_some(v);
        if [self.blocked, self.dns, self.connect, self.ssl]
            .iter()
            .all(|v| *v < 0.0)
            && self.send <= 0.0
            && self.wait <= 0.0
            && self.receive <= 0.0
        {
            return None;
        }
        Some(super::store::Timings {
            blocked: opt(self.blocked),
            dns: opt(self.dns),
            connect: opt(self.connect).map(|c| (c - self.ssl.max(0.0)).max(0.0)),
            tls: opt(self.ssl),
            send: opt(self.send),
            wait: opt(self.wait),
            receive: opt(self.receive),
            reused_connection: self.connect < 0.0,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extra {
    pub state: FlowState,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub client_addr: String,
    #[serde(default)]
    pub ttfb_ms: Option<f64>,
    #[serde(default)]
    pub request_truncated: bool,
    #[serde(default)]
    pub response_truncated: bool,
    #[serde(default)]
    pub ws_messages: Vec<WsExtra>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Our own phases, exact; the HAR `timings` are derived from them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timings: Option<super::store::Timings>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsExtra {
    pub direction: WsDirection,
    pub at: f64,
    pub kind: String,
    pub size: usize,
    pub data: String,
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn name_values(h: &[(String, String)]) -> Vec<NameValue> {
    h.iter()
        .map(|(n, v)| NameValue {
            name: n.clone(),
            value: v.clone(),
        })
        .collect()
}

/// The body as the UI shows it: Content-Encoding removed when possible.
pub fn display_body(raw: &[u8], headers: &[(String, String)], already_decoded: bool) -> Vec<u8> {
    if already_decoded {
        return raw.to_vec();
    }
    match decode::decode(raw, header(headers, "content-encoding")) {
        Ok(Some(d)) => d.bytes,
        _ => raw.to_vec(),
    }
}

/// UTF-8 text as-is, anything else as base64.
fn text_or_base64(bytes: Vec<u8>) -> (String, Option<String>) {
    match String::from_utf8(bytes) {
        Ok(t) => (t, None),
        Err(e) => (B64.encode(e.into_bytes()), Some("base64".into())),
    }
}

fn format_time(epoch_ms: f64) -> String {
    DateTime::from_timestamp_millis(epoch_ms as i64)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn export(flows: &[(FlowMeta, FlowData)]) -> Har {
    Har {
        log: Log {
            version: "1.2".into(),
            creator: Creator {
                name: CREATOR.into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            entries: flows.iter().map(|(m, d)| entry(m, d)).collect(),
        },
    }
}

fn entry(m: &FlowMeta, d: &FlowData) -> Entry {
    let total = m.duration_ms.unwrap_or(0.0);
    let query_string = m
        .url
        .parse::<Uri>()
        .ok()
        .and_then(|u| u.query().map(str::to_string))
        .map(|q| {
            q.split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (n, v) = p.split_once('=').unwrap_or((p, ""));
                    NameValue {
                        name: n.into(),
                        value: v.into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let post_data = (!d.request_body.is_empty()).then(|| {
        let body = display_body(&d.request_body, &d.request_headers, d.bodies_decoded);
        let (text, encoding) = text_or_base64(body);
        PostData {
            mime_type: header(&d.request_headers, "content-type")
                .unwrap_or_default()
                .into(),
            text,
            encoding,
        }
    });

    let response_body = display_body(&d.response_body, &d.response_headers, d.bodies_decoded);
    let content_size = response_body.len() as i64;
    let (text, encoding) = if response_body.is_empty() {
        (None, None)
    } else {
        let (t, e) = text_or_base64(response_body);
        (Some(t), e)
    };

    Entry {
        started_date_time: format_time(m.started_at),
        time: total,
        request: Request {
            method: m.method.clone(),
            url: m.url.clone(),
            http_version: m.http_version.clone(),
            cookies: vec![],
            headers: name_values(&d.request_headers),
            query_string,
            post_data,
            headers_size: -1,
            body_size: m.request_size as i64,
        },
        response: Response {
            status: m.status.map_or(0, i64::from),
            status_text: String::new(),
            http_version: m.http_version.clone(),
            cookies: vec![],
            headers: name_values(&d.response_headers),
            content: Content {
                size: content_size,
                mime_type: m.content_type.clone().unwrap_or_default(),
                text,
                encoding,
            },
            redirect_url: header(&d.response_headers, "location")
                .unwrap_or_default()
                .into(),
            headers_size: -1,
            body_size: m.response_size as i64,
        },
        cache: Value::Object(Default::default()),
        timings: Timings::from_flow(m),
        extra: Some(Extra {
            state: m.state,
            error: m.error.clone(),
            client_addr: m.client_addr.clone(),
            ttfb_ms: m.ttfb_ms,
            request_truncated: d.request_body_truncated,
            response_truncated: d.response_body_truncated,
            ws_messages: d
                .ws_messages
                .iter()
                .map(|w| WsExtra {
                    direction: w.direction,
                    at: w.at,
                    kind: w.kind.into(),
                    size: w.size,
                    data: B64.encode(&w.payload),
                })
                .collect(),
            tags: m.tags.clone(),
            process: m.process.clone(),
            pid: m.pid,
            timings: m.timings,
        }),
    }
}

/// Converts HAR entries to flows. Bodies in HAR are already decoded.
pub fn import(har: Har) -> Vec<(FlowMeta, FlowData)> {
    har.log.entries.into_iter().map(from_entry).collect()
}

fn decode_text(text: &str, encoding: Option<&str>) -> Vec<u8> {
    match encoding {
        Some(e) if e.eq_ignore_ascii_case("base64") => B64
            .decode(text.trim())
            .unwrap_or_else(|_| text.as_bytes().to_vec()),
        _ => text.as_bytes().to_vec(),
    }
}

fn ws_kind(kind: &str) -> &'static str {
    match kind {
        "text" => "text",
        "ping" => "ping",
        "pong" => "pong",
        "close" => "close",
        _ => "binary",
    }
}

fn from_entry(e: Entry) -> (FlowMeta, FlowData) {
    let pairs = |h: Vec<NameValue>| -> Vec<(String, String)> {
        h.into_iter().map(|nv| (nv.name, nv.value)).collect()
    };
    let uri = e.request.url.parse::<Uri>().ok();
    let request_body = e
        .request
        .post_data
        .as_ref()
        .map(|p| decode_text(&p.text, p.encoding.as_deref()))
        .unwrap_or_default();
    let content = &e.response.content;
    let response_body = content
        .text
        .as_deref()
        .map(|t| decode_text(t, content.encoding.as_deref()))
        .unwrap_or_default();
    let status = u16::try_from(e.response.status).ok().filter(|s| *s > 0);
    let started_at = DateTime::parse_from_rfc3339(&e.started_date_time)
        .map(|t| t.timestamp_millis() as f64)
        .unwrap_or(0.0);
    let extra = e.extra;
    let state = extra
        .as_ref()
        .map(|x| x.state)
        .unwrap_or(if status.is_some() {
            FlowState::Completed
        } else {
            FlowState::Failed
        });
    let content_type = Some(content.mime_type.clone()).filter(|s| !s.is_empty());
    let response_size = if e.response.body_size >= 0 {
        e.response.body_size as u64
    } else {
        response_body.len() as u64
    };
    let request_size = if e.request.body_size >= 0 {
        e.request.body_size as u64
    } else {
        request_body.len() as u64
    };

    let meta = FlowMeta {
        id: 0,
        method: e.request.method,
        scheme: uri
            .as_ref()
            .and_then(|u| u.scheme_str())
            .unwrap_or("http")
            .to_string(),
        host: uri
            .as_ref()
            .and_then(|u| u.host())
            .unwrap_or_default()
            .to_string(),
        path: uri
            .as_ref()
            .and_then(|u| u.path_and_query())
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".into()),
        url: e.request.url,
        http_version: e.request.http_version,
        status,
        state,
        content_type,
        started_at,
        ttfb_ms: extra
            .as_ref()
            .and_then(|x| x.ttfb_ms)
            .or(Some(e.timings.wait)),
        duration_ms: Some(e.time),
        request_size,
        response_size,
        client_addr: extra
            .as_ref()
            .map(|x| x.client_addr.clone())
            .unwrap_or_default(),
        error: extra.as_ref().and_then(|x| x.error.clone()),
        ws_message_count: extra.as_ref().map_or(0, |x| x.ws_messages.len() as u32),
        tags: extra.as_ref().map(|x| x.tags.clone()).unwrap_or_default(),
        process: extra.as_ref().and_then(|x| x.process.clone()),
        pid: extra.as_ref().and_then(|x| x.pid),
        timings: match &extra {
            Some(x) => x.timings,
            None => e.timings.to_flow(),
        },
    };
    let data = FlowData {
        request_headers: pairs(e.request.headers),
        request_body,
        request_body_truncated: extra.as_ref().is_some_and(|x| x.request_truncated),
        response_headers: pairs(e.response.headers),
        response_body,
        response_body_truncated: extra.as_ref().is_some_and(|x| x.response_truncated),
        ws_messages: extra
            .map(|x| {
                x.ws_messages
                    .into_iter()
                    .map(|w| WsMessage {
                        direction: w.direction,
                        at: w.at,
                        kind: ws_kind(&w.kind),
                        size: w.size,
                        payload: B64.decode(w.data).unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        bodies_decoded: true,
        request_sent_ms: None,
    };
    (meta, data)
}

pub fn to_json(har: &Har) -> Vec<u8> {
    serde_json::to_vec_pretty(har).expect("HAR serializes")
}

/// Session files are zstd-compressed HAR.
pub fn to_session(har: &Har) -> std::io::Result<Vec<u8>> {
    zstd::encode_all(&serde_json::to_vec(har).expect("HAR serializes")[..], 3)
}

/// Reads a session file or a plain HAR file.
pub fn parse(bytes: &[u8]) -> Result<Har, HarError> {
    if bytes.starts_with(&ZSTD_MAGIC) {
        let json = zstd::decode_all(bytes)?;
        return Ok(serde_json::from_slice(&json)?);
    }
    Ok(serde_json::from_slice(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::store::test_meta;

    fn flow() -> (FlowMeta, FlowData) {
        let mut m = test_meta("/v1/users?limit=10&q=a");
        m.method = "POST".into();
        m.status = Some(201);
        m.state = FlowState::Completed;
        m.content_type = Some("application/json".into());
        m.started_at = 1_700_000_000_123.0;
        m.ttfb_ms = Some(40.0);
        m.duration_ms = Some(55.0);
        let d = FlowData {
            request_headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "*/*".into()),
            ],
            request_body: br#"{"name":"test"}"#.to_vec(),
            response_headers: vec![("content-type".into(), "application/json".into())],
            response_body: br#"{"id":1}"#.to_vec(),
            ..Default::default()
        };
        (m, d)
    }

    // Ported from httpmon internal/har/har_test.go.
    #[test]
    fn single_flow() {
        let har = export(&[flow()]);
        assert_eq!(har.log.version, "1.2");
        let e = &har.log.entries[0];
        assert_eq!(e.started_date_time, "2023-11-14T22:13:20.123Z");
        assert_eq!(e.request.method, "POST");
        assert_eq!(e.request.url, "http://example.com/v1/users?limit=10&q=a");
        assert_eq!(e.request.query_string.len(), 2);
        assert_eq!(
            e.request.post_data.as_ref().unwrap().text,
            r#"{"name":"test"}"#
        );
        assert_eq!(e.response.status, 201);
        assert_eq!(e.response.content.text.as_deref(), Some(r#"{"id":1}"#));
        assert_eq!(e.response.content.mime_type, "application/json");
        assert_eq!((e.timings.wait, e.timings.receive), (40.0, 15.0));
    }

    #[test]
    fn timings_round_trip_and_follow_the_har_spec() {
        use super::super::store::Timings as Phases;
        let (mut m, d) = flow();
        m.timings = Some(Phases {
            blocked: Some(1.0),
            dns: Some(2.0),
            connect: Some(3.0),
            tls: Some(4.0),
            send: Some(5.0),
            wait: Some(25.0),
            receive: Some(15.0),
            reused_connection: false,
        });
        let har = export(&[(m.clone(), d)]);
        let t = &har.log.entries[0].timings;
        // HAR's connect includes the TLS handshake.
        assert_eq!((t.blocked, t.dns, t.connect, t.ssl), (1.0, 2.0, 7.0, 4.0));
        assert_eq!((t.send, t.wait, t.receive), (5.0, 25.0, 15.0));

        let back = import(parse(&to_json(&har)).unwrap());
        assert_eq!(back[0].0.timings, m.timings);

        // Without phases of our own, n/a fields are -1 and the rest derive
        // from TTFB and duration.
        let (m, d) = flow();
        let t = &export(&[(m, d)]).log.entries[0].timings;
        assert_eq!(
            (t.blocked, t.dns, t.connect, t.ssl),
            (-1.0, -1.0, -1.0, -1.0)
        );
        assert_eq!((t.send, t.wait, t.receive), (0.0, 40.0, 15.0));
    }

    #[test]
    fn imports_phases_from_a_foreign_har() {
        let json = br#"{"log":{"entries":[{
            "startedDateTime":"2024-01-02T03:04:05Z","time":30,
            "request":{"method":"GET","url":"https://a.com/","headers":[]},
            "response":{"status":200,"headers":[],"content":{"size":0,"mimeType":""}},
            "timings":{"blocked":1,"dns":2,"connect":10,"ssl":6,"send":0.5,"wait":12,"receive":4.5}
        },{
            "startedDateTime":"2024-01-02T03:04:05Z","time":5,
            "request":{"method":"GET","url":"https://a.com/b","headers":[]},
            "response":{"status":200,"headers":[],"content":{"size":0,"mimeType":""}},
            "timings":{"blocked":-1,"dns":-1,"connect":-1,"ssl":-1,"send":0,"wait":4,"receive":1}
        }]}}"#;
        let flows = import(parse(json).unwrap());
        let t = flows[0].0.timings.unwrap();
        assert_eq!((t.blocked, t.dns), (Some(1.0), Some(2.0)));
        assert_eq!((t.connect, t.tls), (Some(4.0), Some(6.0)));
        assert_eq!(
            (t.send, t.wait, t.receive),
            (Some(0.5), Some(12.0), Some(4.5))
        );
        assert!(!t.reused_connection);
        let t = flows[1].0.timings.unwrap();
        assert!(t.reused_connection);
        assert_eq!((t.connect, t.wait), (None, Some(4.0)));
    }

    #[test]
    fn headers_and_empty_body() {
        let (m, mut d) = flow();
        d.request_body.clear();
        d.response_body.clear();
        let har = export(&[(m.clone(), d.clone()), (m, d)]);
        assert_eq!(har.log.entries.len(), 2);
        let e = &har.log.entries[0];
        assert!(e.request.post_data.is_none());
        assert!(e.response.content.text.is_none());
        assert_eq!(
            e.request.headers[0],
            NameValue {
                name: "Content-Type".into(),
                value: "application/json".into()
            }
        );
    }

    #[test]
    fn exports_decoded_and_binary_bodies() {
        let (m, mut d) = flow();
        d.response_headers
            .push(("Content-Encoding".into(), "gzip".into()));
        d.response_body = decode::tests::gzip(b"plain text");
        d.request_body = vec![0xff, 0x00, 0x01];
        let e = &export(&[(m, d)]).log.entries[0];
        assert_eq!(e.response.content.text.as_deref(), Some("plain text"));
        assert_eq!(e.response.content.size, 10);
        let post = e.request.post_data.as_ref().unwrap();
        assert_eq!(
            (post.text.as_str(), post.encoding.as_deref()),
            ("/wAB", Some("base64"))
        );
    }

    #[test]
    fn session_round_trip_keeps_everything() {
        let (m, mut d) = flow();
        d.response_body_truncated = true;
        d.ws_messages.push(WsMessage {
            direction: WsDirection::ServerToClient,
            at: 5.0,
            kind: "text",
            size: 2,
            payload: b"hi".to_vec(),
        });
        let bytes = to_session(&export(&[(m.clone(), d)])).unwrap();
        assert!(bytes.starts_with(&ZSTD_MAGIC));
        let flows = import(parse(&bytes).unwrap());
        let (m2, d2) = &flows[0];
        assert_eq!(
            (m2.method.as_str(), m2.host.as_str(), m2.path.as_str()),
            ("POST", "example.com", "/v1/users?limit=10&q=a")
        );
        assert_eq!(
            (m2.status, m2.state, m2.started_at),
            (Some(201), FlowState::Completed, m.started_at)
        );
        assert_eq!(m2.ttfb_ms, Some(40.0));
        assert_eq!(d2.response_body, br#"{"id":1}"#);
        assert!(d2.response_body_truncated && d2.bodies_decoded);
        assert_eq!(d2.ws_messages[0].payload, b"hi");
        assert_eq!(m2.ws_message_count, 1);
    }

    #[test]
    fn reads_sessions_saved_before_the_rename() {
        let (m, mut d) = flow();
        d.response_body_truncated = true;
        let json = serde_json::to_string(&export(&[(m, d)])).unwrap();
        assert!(json.contains("\"_proxaLens\""));
        let old = json.replace("\"_proxaLens\"", "\"_projectProxy\"");
        let flows = import(parse(old.as_bytes()).unwrap());
        let (m2, d2) = &flows[0];
        assert_eq!((m2.status, m2.state), (Some(201), FlowState::Completed));
        assert!(d2.response_body_truncated);
    }

    #[test]
    fn imports_foreign_har() {
        let json = br#"{"log":{"version":"1.2","creator":{"name":"Chrome","version":"1"},"entries":[{
            "startedDateTime":"2024-01-02T03:04:05.678+00:00","time":12.5,
            "request":{"method":"GET","url":"https://a.com/x","httpVersion":"h2","headers":[{"name":"accept","value":"*/*"}],"queryString":[],"cookies":[],"headersSize":-1,"bodySize":0},
            "response":{"status":0,"statusText":"","httpVersion":"","headers":[],"cookies":[],"content":{"size":3,"mimeType":"image/png","text":"AAEC","encoding":"base64"},"redirectURL":"","headersSize":-1,"bodySize":-1},
            "cache":{},"timings":{"send":0,"wait":10,"receive":2.5}}]}}"#;
        let flows = import(parse(json).unwrap());
        let (m, d) = &flows[0];
        assert_eq!(
            (m.scheme.as_str(), m.host.as_str(), m.path.as_str()),
            ("https", "a.com", "/x")
        );
        assert_eq!((m.status, m.state), (None, FlowState::Failed));
        assert_eq!(d.response_body, [0, 1, 2]);
        assert_eq!(m.response_size, 3);
        assert_eq!(m.content_type.as_deref(), Some("image/png"));
        assert!(parse(b"nope").is_err());
    }
}
