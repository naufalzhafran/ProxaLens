//! Bandwidth limiting for response bodies. Port of httpmon's
//! `internal/throttle` reader as an async body: data is released in chunks
//! of about 100 ms worth, each held back until the rate allows it.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hudsucker::Body;
use hudsucker::hyper::body::{Body as HttpBody, Frame, SizeHint};
use tokio::time::{Instant, Sleep};

pub struct ThrottleBody {
    inner: Body,
    bytes_per_sec: u64,
    chunk: usize,
    started: Option<Instant>,
    sent: u64,
    /// Rest of a frame larger than one chunk.
    pending: Bytes,
    /// A chunk waiting for `sleep` before it is released.
    held: Option<Bytes>,
    sleep: Option<Pin<Box<Sleep>>>,
}

impl ThrottleBody {
    /// Wraps `inner`; a rate of 0 returns it unchanged.
    pub fn wrap(inner: Body, bytes_per_sec: u64) -> Body {
        if bytes_per_sec == 0 {
            return inner;
        }
        Body::from(BoxBody::new(Self {
            inner,
            bytes_per_sec,
            chunk: (bytes_per_sec / 10).max(1) as usize,
            started: None,
            sent: 0,
            pending: Bytes::new(),
            held: None,
            sleep: None,
        }))
    }

    fn due(&self, started: Instant) -> Instant {
        started + Duration::from_secs_f64(self.sent as f64 / self.bytes_per_sec as f64)
    }
}

impl HttpBody for ThrottleBody {
    type Data = Bytes;
    type Error = hudsucker::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if let Some(sleep) = this.sleep.as_mut() {
                ready!(sleep.as_mut().poll(cx));
                this.sleep = None;
                if let Some(chunk) = this.held.take() {
                    return Poll::Ready(Some(Ok(Frame::data(chunk))));
                }
            }

            let mut data = if this.pending.is_empty() {
                match ready!(Pin::new(&mut this.inner).poll_frame(cx)) {
                    Some(Ok(frame)) => match frame.into_data() {
                        Ok(data) => data,
                        // Trailers and other frames pass straight through.
                        Err(frame) => return Poll::Ready(Some(Ok(frame))),
                    },
                    other => return Poll::Ready(other),
                }
            } else {
                std::mem::take(&mut this.pending)
            };
            if data.is_empty() {
                continue;
            }
            if data.len() > this.chunk {
                this.pending = data.split_off(this.chunk);
            }

            let started = *this.started.get_or_insert_with(Instant::now);
            this.sent += data.len() as u64;
            let due = this.due(started);
            if due <= Instant::now() {
                return Poll::Ready(Some(Ok(Frame::data(data))));
            }
            this.held = Some(data);
            this.sleep = Some(Box::pin(tokio::time::sleep_until(due)));
        }
    }

    fn is_end_stream(&self) -> bool {
        self.held.is_none() && self.pending.is_empty() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn time_to_read(body: Body) -> (usize, Duration) {
        let start = std::time::Instant::now();
        let bytes = body.collect().await.unwrap().to_bytes();
        (bytes.len(), start.elapsed())
    }

    // Ported from httpmon internal/throttle/throttle_test.go.
    #[tokio::test]
    async fn slows_the_transfer() {
        // 10 KB at 20 KB/s: about 0.5 s (the first chunk goes out at once).
        let body = ThrottleBody::wrap(Body::from(vec![1u8; 10_000]), 20_000);
        let (n, took) = time_to_read(body).await;
        assert_eq!(n, 10_000);
        assert!(took >= Duration::from_millis(400), "took {took:?}");
        assert!(took < Duration::from_secs(2), "took {took:?}");
    }

    #[tokio::test]
    async fn zero_rate_is_passthrough() {
        let (n, took) = time_to_read(ThrottleBody::wrap(Body::from(vec![1u8; 1_000_000]), 0)).await;
        assert_eq!(n, 1_000_000);
        assert!(took < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn preserves_content_across_chunks() {
        let data: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let body = ThrottleBody::wrap(Body::from(data.clone()), 50_000);
        assert_eq!(body.collect().await.unwrap().to_bytes(), data);
    }
}
