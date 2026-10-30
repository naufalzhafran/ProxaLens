//! In-memory flow store: a bounded ring buffer of captured HTTP flows.
//!
//! Port of httpmon's `internal/store`. The list view only needs [`FlowMeta`];
//! headers and bodies live in [`FlowData`] and are fetched lazily.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub type FlowId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowState {
    InProgress,
    Completed,
    Failed,
    WebSocket,
    /// HTTPS passed through without decryption (SSL Proxying settings).
    Tunnel,
    /// Refused by the block list or a block rule.
    Blocked,
}

/// Summary fields shown in the flow list.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowMeta {
    pub id: FlowId,
    pub method: String,
    pub scheme: String,
    pub host: String,
    pub path: String,
    pub url: String,
    pub http_version: String,
    pub status: Option<u16>,
    pub state: FlowState,
    pub content_type: Option<String>,
    /// Unix epoch milliseconds.
    pub started_at: f64,
    /// Time from request start until response headers arrived.
    pub ttfb_ms: Option<f64>,
    /// Time from request start until the response body finished.
    pub duration_ms: Option<f64>,
    pub request_size: u64,
    pub response_size: u64,
    pub client_addr: String,
    pub error: Option<String>,
    pub ws_message_count: u32,
    /// Rules that changed this flow ("Map Local", "Rewrite", …).
    pub tags: Vec<String>,
    /// The app that made the request, when it could be identified.
    pub process: Option<String>,
    pub pid: Option<u32>,
    /// The mock whose API this request called (see `mocks.rs`), served or not.
    pub mock_id: Option<String>,
    /// Where the time went; `None` until response headers arrive, and for
    /// blocked, tunnelled and WebSocket flows.
    pub timings: Option<Timings>,
}

/// Where a flow's time went, in milliseconds, in this order. A phase that
/// didn't happen (a reused connection, plain HTTP, an IP address, a locally
/// answered request) is `None`; `receive` is `None` until the body is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Timings {
    /// From the request arriving until a connection was being opened.
    pub blocked: Option<f64>,
    pub dns: Option<f64>,
    pub connect: Option<f64>,
    pub tls: Option<f64>,
    /// Sending the request headers and body.
    pub send: Option<f64>,
    /// From the request being sent until the first response byte.
    pub wait: Option<f64>,
    /// Receiving the response body.
    pub receive: Option<f64>,
    /// The request went over a connection an earlier flow opened.
    pub reused_connection: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WsDirection {
    ClientToServer,
    ServerToClient,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsMessage {
    pub direction: WsDirection,
    pub at: f64,
    /// "text" | "binary" | "ping" | "pong" | "close"
    pub kind: &'static str,
    pub size: usize,
    /// Payload, truncated to [`WS_PAYLOAD_CAP`] bytes.
    #[serde(skip)]
    pub payload: Vec<u8>,
}

pub const WS_PAYLOAD_CAP: usize = 64 * 1024;
pub const WS_MESSAGES_CAP: usize = 2_000;

/// Full request/response detail for one flow.
#[derive(Clone, Debug, Default)]
pub struct FlowData {
    pub request_headers: Vec<(String, String)>,
    pub request_body: Vec<u8>,
    pub request_body_truncated: bool,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Vec<u8>,
    pub response_body_truncated: bool,
    pub ws_messages: Vec<WsMessage>,
    /// Bodies are already decoded (imported from HAR), so Content-Encoding
    /// headers must not be applied again.
    pub bodies_decoded: bool,
    /// When the request body finished going upstream, in ms from the
    /// flow's start; an input to the `send` phase of [`Timings`].
    pub request_sent_ms: Option<f64>,
}

/// Change notifications for subscribers (the UI bridge, tests).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum FlowEvent {
    Upsert(FlowMeta),
    Cleared,
    Removed(Vec<FlowId>),
}

struct Entry {
    meta: FlowMeta,
    data: FlowData,
}

struct Inner {
    order: VecDeque<FlowId>,
    flows: HashMap<FlowId, Entry>,
}

pub struct Store {
    capacity: usize,
    next_id: AtomicU64,
    /// Off while recording is paused: traffic still flows, rules still
    /// apply, but nothing new is kept.
    recording: AtomicBool,
    inner: RwLock<Inner>,
    events: broadcast::Sender<FlowEvent>,
}

impl Store {
    pub fn new(capacity: usize) -> Self {
        let (events, _) = broadcast::channel(4096);
        Self {
            capacity: capacity.max(1),
            next_id: AtomicU64::new(1),
            recording: AtomicBool::new(true),
            inner: RwLock::new(Inner {
                order: VecDeque::new(),
                flows: HashMap::new(),
            }),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FlowEvent> {
        self.events.subscribe()
    }

    /// Inserts a new flow, evicting the oldest one when full. The `id` field
    /// of `meta` is overwritten with a freshly allocated id. While recording
    /// is paused the id is allocated but nothing is kept, so later
    /// `update`s for it do nothing, as for an evicted flow.
    pub fn add(&self, mut meta: FlowMeta, data: FlowData) -> FlowId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if !self.is_recording() {
            return id;
        }
        meta.id = id;
        {
            let mut inner = self.inner.write();
            while inner.order.len() >= self.capacity {
                if let Some(old) = inner.order.pop_front() {
                    inner.flows.remove(&old);
                }
            }
            inner.order.push_back(id);
            inner.flows.insert(
                id,
                Entry {
                    meta: meta.clone(),
                    data,
                },
            );
        }
        let _ = self.events.send(FlowEvent::Upsert(meta));
        id
    }

    /// Mutates a flow's meta and data together, then notifies subscribers.
    /// Does nothing if the flow was evicted or cleared in the meantime.
    pub fn update(&self, id: FlowId, f: impl FnOnce(&mut FlowMeta, &mut FlowData)) {
        let meta = {
            let mut inner = self.inner.write();
            let Some(entry) = inner.flows.get_mut(&id) else {
                return;
            };
            f(&mut entry.meta, &mut entry.data);
            entry.meta.clone()
        };
        let _ = self.events.send(FlowEvent::Upsert(meta));
    }

    pub fn get(&self, id: FlowId) -> Option<(FlowMeta, FlowData)> {
        let inner = self.inner.read();
        inner
            .flows
            .get(&id)
            .map(|e| (e.meta.clone(), e.data.clone()))
    }

    /// Returns flows oldest-first, skipping `offset` and returning at most
    /// `limit`, plus the total number of stored flows.
    pub fn list(&self, offset: usize, limit: usize) -> (Vec<FlowMeta>, usize) {
        let inner = self.inner.read();
        let items = inner
            .order
            .iter()
            .skip(offset)
            .take(limit)
            .filter_map(|id| inner.flows.get(id).map(|e| e.meta.clone()))
            .collect();
        (items, inner.order.len())
    }

    pub fn len(&self) -> usize {
        self.inner.read().order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    pub fn set_recording(&self, on: bool) {
        self.recording.store(on, Ordering::Relaxed);
    }

    /// Removes the given flows; returns how many were stored.
    pub fn remove(&self, ids: &[FlowId]) -> usize {
        let removed: Vec<FlowId> = {
            let mut inner = self.inner.write();
            let removed: Vec<FlowId> = ids
                .iter()
                .copied()
                .filter(|id| inner.flows.remove(id).is_some())
                .collect();
            if !removed.is_empty() {
                let gone: HashSet<FlowId> = removed.iter().copied().collect();
                inner.order.retain(|id| !gone.contains(id));
            }
            removed
        };
        let n = removed.len();
        if n > 0 {
            let _ = self.events.send(FlowEvent::Removed(removed));
        }
        n
    }

    pub fn clear(&self) {
        {
            let mut inner = self.inner.write();
            inner.order.clear();
            inner.flows.clear();
        }
        let _ = self.events.send(FlowEvent::Cleared);
    }
}

#[cfg(test)]
pub(crate) fn test_meta(path: &str) -> FlowMeta {
    FlowMeta {
        id: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "example.com".into(),
        path: path.into(),
        url: format!("http://example.com{path}"),
        http_version: "HTTP/1.1".into(),
        status: None,
        state: FlowState::InProgress,
        content_type: None,
        started_at: 0.0,
        ttfb_ms: None,
        duration_ms: None,
        request_size: 0,
        response_size: 0,
        client_addr: "127.0.0.1:1".into(),
        error: None,
        ws_message_count: 0,
        timings: None,
        tags: Vec::new(),
        mock_id: None,
        process: None,
        pid: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_when_full() {
        let s = Store::new(2);
        let a = s.add(test_meta("/a"), FlowData::default());
        let b = s.add(test_meta("/b"), FlowData::default());
        let c = s.add(test_meta("/c"), FlowData::default());
        assert!(s.get(a).is_none());
        assert!(s.get(b).is_some());
        let (items, total) = s.list(0, 10);
        assert_eq!(total, 2);
        assert_eq!(items.iter().map(|m| m.id).collect::<Vec<_>>(), vec![b, c]);
    }

    #[test]
    fn update_and_paging() {
        let s = Store::new(10);
        let ids: Vec<_> = (0..5)
            .map(|i| s.add(test_meta(&format!("/{i}")), FlowData::default()))
            .collect();
        s.update(ids[1], |m, d| {
            m.status = Some(204);
            d.response_body = b"hi".to_vec();
        });
        let (m, d) = s.get(ids[1]).unwrap();
        assert_eq!(m.status, Some(204));
        assert_eq!(d.response_body, b"hi");
        let (page, total) = s.list(2, 2);
        assert_eq!(total, 5);
        assert_eq!(page[0].path, "/2");
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn update_after_clear_is_noop_and_events_fire() {
        let s = Store::new(10);
        let mut rx = s.subscribe();
        let id = s.add(test_meta("/x"), FlowData::default());
        s.clear();
        s.update(id, |m, _| m.status = Some(200));
        assert!(matches!(rx.try_recv().unwrap(), FlowEvent::Upsert(_)));
        assert!(matches!(rx.try_recv().unwrap(), FlowEvent::Cleared));
        assert!(rx.try_recv().is_err());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn paused_recording_keeps_nothing_new() {
        let s = Store::new(10);
        let mut rx = s.subscribe();
        let a = s.add(test_meta("/a"), FlowData::default());
        s.set_recording(false);
        let b = s.add(test_meta("/b"), FlowData::default());
        s.update(b, |m, _| m.status = Some(200));
        assert!(b > a, "ids keep increasing while paused");
        assert!(s.get(b).is_none());
        assert_eq!(s.len(), 1);
        // Flows from before the pause still update.
        s.update(a, |m, _| m.status = Some(204));
        assert_eq!(s.get(a).unwrap().0.status, Some(204));
        s.set_recording(true);
        let c = s.add(test_meta("/c"), FlowData::default());
        assert!(s.get(c).is_some());
        let mut seen = Vec::new();
        while let Ok(FlowEvent::Upsert(m)) = rx.try_recv() {
            seen.push(m.id);
        }
        assert_eq!(seen, vec![a, a, c]);
    }

    #[test]
    fn removes_flows_and_reports_them() {
        let s = Store::new(10);
        let ids: Vec<FlowId> = ["/a", "/b", "/c"]
            .iter()
            .map(|p| s.add(test_meta(p), FlowData::default()))
            .collect();
        let mut rx = s.subscribe();
        assert_eq!(s.remove(&[ids[0], ids[2], 999]), 2);
        assert_eq!(
            s.list(0, 10).0.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![ids[1]]
        );
        assert!(matches!(rx.try_recv(), Ok(FlowEvent::Removed(r)) if r == vec![ids[0], ids[2]]));
        // Removing nothing sends nothing.
        assert_eq!(s.remove(&[ids[0]]), 0);
        assert!(rx.try_recv().is_err());
    }
}
