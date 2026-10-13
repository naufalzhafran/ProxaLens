//! Protobuf and gRPC body decoding, a port of httpmon's `bodydecoder`.
//!
//! Without a schema, the wire format is shown with field numbers as keys
//! (nested messages, strings, floats and base64 guessed per field). With
//! `.proto` files configured for a host, gRPC calls are decoded by name:
//! the request path `/package.Service/Method` picks the input or output
//! message. gRPC and gRPC-Web bodies are unframed first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use parking_lot::Mutex;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

const RAW_TYPES: &[&str] = &[
    "application/protobuf",
    "application/x-protobuf",
    "application/x-google-protobuf",
    "application/vnd.google.protobuf",
];
const GRPC_TYPES: &[&str] = &["application/grpc", "application/grpc+proto"];
const GRPC_WEB_TYPES: &[&str] = &["application/grpc-web", "application/grpc-web+proto"];
const GRPC_WEB_TEXT_TYPES: &[&str] = &[
    "application/grpc-web-text",
    "application/grpc-web-text+proto",
];

/// `.proto` files to use for hosts matching `hosts` (`api.x.com`,
/// `*.x.com` or `*`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProtoSource {
    pub hosts: String,
    /// `.proto` files, or directories searched for them (also used as
    /// import roots).
    pub paths: Vec<String>,
}

/// A decoded protobuf body for display.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtoView {
    /// Pretty-printed JSON.
    pub json: String,
    /// The message type when decoded with a schema.
    pub message: Option<String>,
    pub notes: Vec<String>,
}

fn mime(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Whether the content type is one this module decodes.
pub fn is_protobuf(content_type: &str) -> bool {
    let m = mime(content_type);
    [RAW_TYPES, GRPC_TYPES, GRPC_WEB_TYPES, GRPC_WEB_TEXT_TYPES]
        .iter()
        .any(|types| types.contains(&m.as_str()))
}

fn read_varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for (i, byte) in b.iter().enumerate().take(10) {
        v |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

fn reasonable_float(f: f64) -> bool {
    f.is_finite() && (1e-10..1e15).contains(&f.abs())
}

fn readable(b: &[u8]) -> Option<&str> {
    if b.is_empty()
        || b.iter()
            .any(|&c| c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r'))
    {
        return None;
    }
    std::str::from_utf8(b).ok()
}

fn interpret_bytes(b: &[u8]) -> Json {
    if let Ok(nested) = decode_wire(b)
        && nested.as_object().is_some_and(|o| !o.is_empty())
    {
        return nested;
    }
    match readable(b) {
        Some(s) => Json::String(s.to_string()),
        None => Json::String(B64.encode(b)),
    }
}

/// Adds a field; a repeated field number turns into an array. (Decoded
/// values are never arrays themselves, so any array is a repeated field.)
fn push(fields: &mut Map<String, Json>, num: u64, val: Json) {
    let key = num.to_string();
    match fields.get_mut(&key) {
        Some(Json::Array(arr)) => arr.push(val),
        Some(existing) => {
            let prev = existing.take();
            *existing = Json::Array(vec![prev, val]);
        }
        None => {
            fields.insert(key, val);
        }
    }
}

/// Decodes protobuf wire format into JSON with field numbers as keys
/// (port of httpmon's `decodeWireFields`).
pub fn decode_wire(mut b: &[u8]) -> Result<Json, String> {
    let mut fields = Map::new();
    while !b.is_empty() {
        let (tag, n) = read_varint(b).ok_or("invalid tag")?;
        b = &b[n..];
        let (num, wire) = (tag >> 3, tag & 7);
        if num == 0 {
            return Err("field number 0".into());
        }
        let val = match wire {
            0 => {
                let (v, n) =
                    read_varint(b).ok_or_else(|| format!("invalid varint for field {num}"))?;
                b = &b[n..];
                Json::from(v)
            }
            1 => {
                let raw: [u8; 8] = b
                    .get(..8)
                    .ok_or_else(|| format!("short fixed64 for field {num}"))?
                    .try_into()
                    .unwrap();
                b = &b[8..];
                let v = u64::from_le_bytes(raw);
                let f = f64::from_bits(v);
                if reasonable_float(f) {
                    Json::from(f)
                } else {
                    Json::from(v)
                }
            }
            2 => {
                let (len, n) =
                    read_varint(b).ok_or_else(|| format!("invalid length for field {num}"))?;
                b = &b[n..];
                let len = usize::try_from(len).map_err(|_| "length too large")?;
                let data = b
                    .get(..len)
                    .ok_or_else(|| format!("short bytes for field {num}"))?;
                b = &b[len..];
                interpret_bytes(data)
            }
            5 => {
                let raw: [u8; 4] = b
                    .get(..4)
                    .ok_or_else(|| format!("short fixed32 for field {num}"))?
                    .try_into()
                    .unwrap();
                b = &b[4..];
                let v = u32::from_le_bytes(raw);
                let f = f32::from_bits(v);
                if reasonable_float(f64::from(f)) {
                    Json::from(f64::from(f))
                } else {
                    Json::from(v)
                }
            }
            w => return Err(format!("unknown wire type {w} for field {num}")),
        };
        push(&mut fields, num, val);
    }
    Ok(Json::Object(fields))
}

/// Concatenated data frames of a gRPC / gRPC-Web body, with notes about
/// compressed, unknown or truncated frames (port of `extractDataFrames`).
pub fn grpc_frames(body: &[u8]) -> (Vec<u8>, Vec<String>) {
    let mut payload = Vec::new();
    let mut notes = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        if rest.len() < 5 {
            notes.push(format!("truncated frame header: {} bytes", rest.len()));
            break;
        }
        let flag = rest[0];
        let len = u32::from_be_bytes([rest[1], rest[2], rest[3], rest[4]]) as usize;
        rest = &rest[5..];
        let (data, truncated) = if rest.len() < len {
            notes.push(format!(
                "truncated frame: expected {len} bytes, got {}",
                rest.len()
            ));
            (rest, true)
        } else {
            (&rest[..len], false)
        };
        match flag {
            0x00 => payload.extend_from_slice(data),
            0x01 => notes.push(format!("compressed gRPC payload ({len} bytes) not shown")),
            f if f & 0x80 != 0 => {
                if let Some(t) = readable(data) {
                    notes.push(format!("trailers: {}", t.trim().replace("\r\n", ", ")));
                }
            }
            f => notes.push(format!("unknown frame flag 0x{f:02x} ({len} bytes)")),
        }
        if truncated {
            break;
        }
        rest = &rest[len..];
    }
    (payload, notes)
}

/// Compiled `.proto` files.
pub struct ProtoRegistry {
    pool: DescriptorPool,
}

impl ProtoRegistry {
    /// Compiles every `.proto` file in `paths` (files or directories).
    pub fn load(paths: &[String]) -> Result<Self, String> {
        let mut files: Vec<PathBuf> = Vec::new();
        let mut includes: Vec<PathBuf> = Vec::new();
        for p in paths {
            let p = PathBuf::from(p.trim());
            if p.is_dir() {
                includes.push(p.clone());
                collect_protos(&p, &mut files);
            } else if p.is_file() {
                if let Some(dir) = p.parent() {
                    includes.push(dir.to_path_buf());
                }
                files.push(p);
            } else {
                return Err(format!("{} does not exist", p.display()));
            }
        }
        if files.is_empty() {
            return Err("no .proto files found".into());
        }
        includes.sort();
        includes.dedup();
        // protox wants file names relative to an include directory.
        let relative: Vec<PathBuf> = files
            .iter()
            .map(|f| {
                includes
                    .iter()
                    .filter_map(|i| f.strip_prefix(i).ok())
                    .min_by_key(|r| r.components().count())
                    .map_or_else(|| f.clone(), Path::to_path_buf)
            })
            .collect();
        let fds = protox::compile(&relative, &includes).map_err(|e| e.to_string())?;
        let pool = DescriptorPool::from_file_descriptor_set(fds).map_err(|e| e.to_string())?;
        Ok(Self { pool })
    }

    /// The input or output message for a gRPC path's last two segments
    /// (`/[prefix/]package.Service/Method`).
    pub fn message_for(&self, path: &str, request: bool) -> Option<MessageDescriptor> {
        let path = path.split('?').next().unwrap_or(path);
        let mut parts = path.trim_matches('/').rsplit('/');
        let method = parts.next()?;
        let service = parts.next()?;
        let svc = self.pool.get_service_by_name(service)?;
        let m = svc.methods().find(|m| m.name() == method)?;
        Some(if request { m.input() } else { m.output() })
    }

    pub fn decode(&self, desc: MessageDescriptor, payload: &[u8]) -> Result<Json, String> {
        let msg = DynamicMessage::decode(desc, payload).map_err(|e| e.to_string())?;
        serde_json::to_value(&msg).map_err(|e| e.to_string())
    }
}

fn collect_protos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_protos(&p, out);
        } else if p.extension().is_some_and(|x| x == "proto") {
            out.push(p);
        }
    }
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    p == "*"
        || p == host
        || p.strip_prefix('*')
            .is_some_and(|s| s.starts_with('.') && host.ends_with(s))
}

/// A compiled registry, or why it failed to compile.
type Loaded = Arc<Result<ProtoRegistry, String>>;

/// Per-host registries, compiled on first use and cached until the
/// configuration changes.
#[derive(Default)]
pub struct Registries {
    cache: Mutex<(Vec<ProtoSource>, HashMap<usize, Loaded>)>,
}

impl Registries {
    /// The registry for `host`, or the compile error of the matching entry.
    pub fn resolve(&self, sources: &[ProtoSource], host: &str) -> Option<Loaded> {
        let i = sources.iter().position(|s| host_matches(&s.hosts, host))?;
        let mut cache = self.cache.lock();
        if cache.0 != sources {
            *cache = (sources.to_vec(), HashMap::new());
        }
        Some(
            cache
                .1
                .entry(i)
                .or_insert_with(|| Arc::new(ProtoRegistry::load(&sources[i].paths)))
                .clone(),
        )
    }
}

/// Decodes a protobuf / gRPC body for display, or `None` when the content
/// type isn't one of ours.
pub fn decode_body(
    content_type: &str,
    body: &[u8],
    host: &str,
    path: &str,
    request: bool,
    sources: &[ProtoSource],
    registries: &Registries,
) -> Option<ProtoView> {
    let m = mime(content_type);
    let mut notes = Vec::new();
    let payload: Vec<u8> = if RAW_TYPES.contains(&m.as_str()) {
        body.to_vec()
    } else if GRPC_TYPES.contains(&m.as_str()) || GRPC_WEB_TYPES.contains(&m.as_str()) {
        let (p, n) = grpc_frames(body);
        notes = n;
        p
    } else if GRPC_WEB_TEXT_TYPES.contains(&m.as_str()) {
        let raw = B64
            .decode(
                body.iter()
                    .copied()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect::<Vec<_>>(),
            )
            .ok()?;
        let (p, n) = grpc_frames(&raw);
        notes = n;
        p
    } else {
        return None;
    };

    let mut message = None;
    let mut value = None;
    if let Some(reg) = registries.resolve(sources, host) {
        match reg.as_ref() {
            Ok(reg) => {
                if let Some(desc) = reg.message_for(path, request) {
                    match reg.decode(desc.clone(), &payload) {
                        Ok(v) => {
                            message = Some(desc.full_name().to_string());
                            value = Some(v);
                        }
                        Err(e) => notes.push(format!(
                            "{} did not decode ({e}); showing raw fields",
                            desc.full_name()
                        )),
                    }
                }
            }
            Err(e) => notes.push(format!("could not load .proto files: {e}")),
        }
    }
    let value = match value {
        Some(v) => v,
        None if payload.is_empty() => Json::Object(Map::new()),
        None => match decode_wire(&payload) {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!("not valid protobuf: {e}"));
                return Some(ProtoView {
                    json: "{}".into(),
                    message: None,
                    notes,
                });
            }
        },
    };
    Some(ProtoView {
        json: serde_json::to_string_pretty(&value).unwrap_or_default(),
        message,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes HelloRequest { name: "Ann", age: 42 } by hand.
    fn hello() -> Vec<u8> {
        let mut b = vec![0x0a, 3];
        b.extend_from_slice(b"Ann");
        b.extend_from_slice(&[0x10, 42]);
        b
    }

    fn frame(flag: u8, data: &[u8]) -> Vec<u8> {
        let mut f = vec![flag];
        f.extend_from_slice(&(data.len() as u32).to_be_bytes());
        f.extend_from_slice(data);
        f
    }

    // Ported from httpmon internal/bodydecoder/protobuf_test.go.
    #[test]
    fn decodes_raw_wire_format() {
        let mut b = hello();
        // field 3: nested { 1: 7 }, field 4 twice (repeated), field 5: float 1.5
        b.extend_from_slice(&[0x1a, 2, 0x08, 7]);
        b.extend_from_slice(&[0x20, 1, 0x20, 2]);
        b.push(0x2d);
        b.extend_from_slice(&1.5f32.to_le_bytes());
        let v = decode_wire(&b).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"1": "Ann", "2": 42, "3": {"1": 7}, "4": [1, 2], "5": 1.5})
        );
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"1":"Ann","2":42,"3":{"1":7},"4":[1,2],"5":1.5}"#,
            "field order kept"
        );
        assert!(decode_wire(&[0x0a, 10, b'x']).is_err());
        assert!(
            decode_wire(&[0x0b]).is_err(),
            "wire type 3 (groups) is not supported"
        );
    }

    // Ported from httpmon internal/bodydecoder/grpcweb_test.go.
    #[test]
    fn unframes_grpc_web() {
        let mut body = frame(0, &hello());
        body.extend(frame(0x80, b"grpc-status: 0\r\ngrpc-message: OK\r\n"));
        let (payload, notes) = grpc_frames(&body);
        assert_eq!(payload, hello());
        assert_eq!(notes, vec!["trailers: grpc-status: 0, grpc-message: OK"]);

        let (_, notes) = grpc_frames(&frame(1, b"zz"));
        assert!(notes[0].contains("compressed"));
        let mut cut = frame(0, &hello());
        cut.truncate(7);
        let (payload, notes) = grpc_frames(&cut);
        assert_eq!(payload.len(), 2);
        assert!(notes[0].contains("truncated"));
    }

    fn write_proto(dir: &Path) {
        std::fs::write(
            dir.join("test.proto"),
            r#"syntax = "proto3";
package testpkg;
service Greeter { rpc SayHello (HelloRequest) returns (HelloReply) {} }
message HelloRequest { string name = 1; int32 age = 2; }
message HelloReply { string message = 1; bool success = 2; }
"#,
        )
        .unwrap();
    }

    // Ported from httpmon internal/bodydecoder/protoregistry_test.go and
    // host_registry_test.go.
    #[test]
    fn decodes_grpc_by_name_with_per_host_protos() {
        let dir = tempfile::tempdir().unwrap();
        write_proto(dir.path());
        let sources = vec![ProtoSource {
            hosts: "*.example.com".into(),
            paths: vec![dir.path().to_string_lossy().into_owned()],
        }];
        let regs = Registries::default();
        let body = frame(0, &hello());

        let named = decode_body(
            "application/grpc-web+proto",
            &body,
            "api.example.com",
            "/testpkg.Greeter/SayHello",
            true,
            &sources,
            &regs,
        )
        .unwrap();
        assert_eq!(named.message.as_deref(), Some("testpkg.HelloRequest"));
        assert_eq!(
            serde_json::from_str::<Json>(&named.json).unwrap(),
            serde_json::json!({"name": "Ann", "age": 42})
        );

        // Another host has no schema: raw fields.
        let raw = decode_body(
            "application/grpc",
            &body,
            "other.com",
            "/testpkg.Greeter/SayHello",
            true,
            &sources,
            &regs,
        )
        .unwrap();
        assert_eq!(raw.message, None);
        assert_eq!(
            serde_json::from_str::<Json>(&raw.json).unwrap(),
            serde_json::json!({"1": "Ann", "2": 42})
        );

        // grpc-web-text is base64.
        let text = B64.encode(&body);
        let t = decode_body(
            "application/grpc-web-text",
            text.as_bytes(),
            "api.example.com",
            "/testpkg.Greeter/SayHello",
            true,
            &sources,
            &regs,
        )
        .unwrap();
        assert_eq!(t.message.as_deref(), Some("testpkg.HelloRequest"));

        assert!(decode_body("application/json", b"{}", "x", "/", true, &sources, &regs).is_none());
    }

    #[test]
    fn reports_bad_proto_configuration() {
        let sources = vec![ProtoSource {
            hosts: "*".into(),
            paths: vec!["/definitely/missing".into()],
        }];
        let v = decode_body(
            "application/x-protobuf",
            &hello(),
            "a.com",
            "/",
            false,
            &sources,
            &Registries::default(),
        )
        .unwrap();
        assert!(v.notes[0].contains("could not load"), "{:?}", v.notes);
        assert_eq!(serde_json::from_str::<Json>(&v.json).unwrap()["1"], "Ann");
    }

    #[test]
    fn host_patterns() {
        assert!(host_matches("*", "a.com"));
        assert!(host_matches("*.a.com", "x.a.com"));
        assert!(!host_matches("*.a.com", "a.com"));
        assert!(host_matches("A.com", "a.com"));
    }
}
