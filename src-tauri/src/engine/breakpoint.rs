//! Breakpoints: pause a flow until the user edits and resumes it, aborts it,
//! or the timeout passes. Port of httpmon's `internal/breakpoint` controller,
//! using a oneshot per paused flow.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};

use super::proxy::now_ms;
use super::store::FlowId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Request,
    Response,
}

/// A paused flow, as shown to the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub flow_id: FlowId,
    pub phase: Phase,
    pub method: String,
    pub url: String,
    /// Set in the response phase.
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
    #[serde(skip)]
    pub body: Vec<u8>,
    /// When the flow continues unchanged on its own (epoch ms).
    pub deadline: u64,
}

/// The user's edits. Fields that don't apply to the phase are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Edits {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resume {
    /// Continue, with edits (`None` = unchanged, e.g. after a timeout).
    Continue(Option<Edits>),
    Abort,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

type Key = (FlowId, Phase);

pub struct Breakpoints {
    pending: Mutex<HashMap<Key, (Hit, oneshot::Sender<Resume>)>>,
    changed: broadcast::Sender<()>,
    timeout: Duration,
}

impl Breakpoints {
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            changed: broadcast::channel(64).0,
            timeout,
        }
    }

    /// Fires whenever a flow is paused or resumed.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    /// Pauses until resumed, aborted or timed out. If the caller is dropped
    /// (the client went away), the hit is withdrawn.
    pub async fn pause(&self, mut hit: Hit) -> Resume {
        let key = (hit.flow_id, hit.phase);
        hit.deadline = (now_ms() + self.timeout.as_millis() as f64) as u64;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(key, (hit, tx));
        let _ = self.changed.send(());

        struct Withdraw<'a>(&'a Breakpoints, Key);
        impl Drop for Withdraw<'_> {
            fn drop(&mut self) {
                if self.0.pending.lock().remove(&self.1).is_some() {
                    let _ = self.0.changed.send(());
                }
            }
        }
        let _withdraw = Withdraw(self, key);

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(resume)) => resume,
            // Timed out, or the controller dropped the sender.
            _ => Resume::Continue(None),
        }
    }

    /// Resumes a paused flow; returns false if it wasn't paused.
    pub fn resume(&self, flow_id: FlowId, phase: Phase, resume: Resume) -> bool {
        let Some((_, tx)) = self.pending.lock().remove(&(flow_id, phase)) else {
            return false;
        };
        let _ = tx.send(resume);
        let _ = self.changed.send(());
        true
    }

    /// Paused flows, oldest first.
    pub fn pending(&self) -> Vec<Hit> {
        let mut hits: Vec<Hit> = self
            .pending
            .lock()
            .values()
            .map(|(h, _)| h.clone())
            .collect();
        hits.sort_by_key(|h| (h.deadline, h.flow_id));
        hits
    }

    /// Lets every paused flow continue unchanged.
    pub fn resume_all(&self) {
        let all: Vec<_> = self.pending.lock().drain().collect();
        for (_, (_, tx)) in all {
            let _ = tx.send(Resume::Continue(None));
        }
        let _ = self.changed.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn hit(id: FlowId, phase: Phase) -> Hit {
        Hit {
            flow_id: id,
            phase,
            method: "GET".into(),
            url: format!("https://a.com/{id}"),
            status: None,
            headers: vec![],
            body: vec![],
            deadline: 0,
        }
    }

    async fn wait_pending(bp: &Breakpoints, n: usize) {
        for _ in 0..100 {
            if bp.pending().len() == n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("expected {n} pending hits");
    }

    // Ported from httpmon internal/breakpoint/breakpoint_test.go.
    #[tokio::test]
    async fn pause_blocks_until_resume_with_edits() {
        let bp = Arc::new(Breakpoints::new(DEFAULT_TIMEOUT));
        let b = bp.clone();
        let task = tokio::spawn(async move { b.pause(hit(1, Phase::Request)).await });
        wait_pending(&bp, 1).await;
        assert!(!task.is_finished());
        let edits = Edits {
            body: b"edited".to_vec(),
            ..Default::default()
        };
        assert!(bp.resume(1, Phase::Request, Resume::Continue(Some(edits.clone()))));
        assert_eq!(task.await.unwrap(), Resume::Continue(Some(edits)));
        assert!(bp.pending().is_empty());
    }

    #[tokio::test]
    async fn abort_and_unknown_resume() {
        let bp = Arc::new(Breakpoints::new(DEFAULT_TIMEOUT));
        let b = bp.clone();
        let task = tokio::spawn(async move { b.pause(hit(2, Phase::Response)).await });
        wait_pending(&bp, 1).await;
        assert!(!bp.resume(2, Phase::Request, Resume::Abort), "wrong phase");
        assert!(!bp.resume(99, Phase::Response, Resume::Abort));
        assert!(bp.resume(2, Phase::Response, Resume::Abort));
        assert_eq!(task.await.unwrap(), Resume::Abort);
    }

    #[tokio::test]
    async fn concurrent_pauses_resume_independently_and_all() {
        let bp = Arc::new(Breakpoints::new(DEFAULT_TIMEOUT));
        let mut rx = bp.subscribe();
        let tasks: Vec<_> = (1..=3)
            .map(|i| {
                let b = bp.clone();
                tokio::spawn(async move { b.pause(hit(i, Phase::Request)).await })
            })
            .collect();
        wait_pending(&bp, 3).await;
        assert!(rx.try_recv().is_ok(), "subscribers hear about hits");
        bp.resume(2, Phase::Request, Resume::Abort);
        wait_pending(&bp, 2).await;
        bp.resume_all();
        let results: Vec<_> = futures_join(tasks).await;
        assert_eq!(
            results,
            vec![
                Resume::Continue(None),
                Resume::Abort,
                Resume::Continue(None)
            ]
        );
    }

    async fn futures_join(tasks: Vec<tokio::task::JoinHandle<Resume>>) -> Vec<Resume> {
        let mut out = Vec::new();
        for t in tasks {
            out.push(t.await.unwrap());
        }
        out
    }

    #[tokio::test]
    async fn times_out_unchanged_and_withdraws_when_dropped() {
        let bp = Arc::new(Breakpoints::new(Duration::from_millis(30)));
        assert_eq!(
            bp.pause(hit(1, Phase::Request)).await,
            Resume::Continue(None)
        );
        assert!(bp.pending().is_empty());

        let slow = Arc::new(Breakpoints::new(DEFAULT_TIMEOUT));
        let b = slow.clone();
        let task = tokio::spawn(async move { b.pause(hit(5, Phase::Request)).await });
        wait_pending(&slow, 1).await;
        task.abort();
        let _ = task.await;
        assert!(slow.pending().is_empty(), "client went away");
    }
}
