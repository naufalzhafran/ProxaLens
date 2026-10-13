//! In-memory flow store: a bounded ring buffer of captured HTTP flows.
//!
//! Port of httpmon's `internal/store`. The list view only needs [`FlowMeta`];
//! headers and bodies live in [`FlowData`] and are fetched lazily.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// Refused by the block list.
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
}

/// Change notifications for subscribers (the UI bridge, tests).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum FlowEvent {
    Upsert(FlowMeta),
    Cleared,
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
    inner: RwLock<Inner>,
    events: broadcast::Sender<FlowEvent>,
}

impl Store {
    pub fn new(capacity: usize) -> Self {
        let (events, _) = broadcast::channel(4096);
        Self {
            capacity: capacity.max(1),
            next_id: AtomicU64::new(1),
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
    /// of `meta` is overwritten with a freshly allocated id.
    pub fn add(&self, mut meta: FlowMeta, data: FlowData) -> FlowId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
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
}
