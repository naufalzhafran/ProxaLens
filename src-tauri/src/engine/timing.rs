//! Where a flow's time goes: DNS, TCP connect, TLS handshake, sending the
//! request, waiting for the server and downloading the response.
//!
//! The upstream connector is wrapped at three levels so each phase can be
//! timed: a resolver (DNS), the TCP connector (DNS + connect) and the TLS
//! connector (everything). The inner levels report into a task-local slot the
//! outer level opens around the whole connection attempt, and the result rides
//! on the connection as a hyper-util "extra", which the client copies into
//! every response's extensions. The first response to see it belongs to the
//! flow that opened the connection; later ones reused it.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use hudsucker::hyper::Uri;
use hudsucker::hyper::rt::{Read, ReadBufCursor, Write};
use hudsucker::hyper_util::client::legacy::connect::dns::{GaiAddrs, GaiResolver, Name};
use hudsucker::hyper_util::client::legacy::connect::{Connected, Connection, HttpConnector};
use hudsucker::rustls::ClientConfig;
use hyper_rustls::HttpsConnector;
use parking_lot::Mutex;
use tower_service::Service;

use super::store::Timings;

/// How long opening one upstream connection took, by phase.
#[derive(Clone, Debug)]
pub struct ConnTiming {
    /// When the connection attempt began.
    pub started: Instant,
    /// `None` when the host was an IP address.
    pub dns: Option<Duration>,
    pub connect: Duration,
    /// `None` for plain HTTP.
    pub tls: Option<Duration>,
    claimed: Arc<AtomicBool>,
}

impl ConnTiming {
    /// True the first time only: the flow that opened the connection owns
    /// its timings. Later flows on the same connection reused it.
    pub fn claim(&self) -> bool {
        !self.claimed.swap(true, Ordering::AcqRel)
    }

    /// The connection phases as milliseconds from `flow_started`.
    pub fn phases(&self, flow_started: Instant) -> ConnPhases {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        ConnPhases {
            blocked_ms: self
                .started
                .checked_duration_since(flow_started)
                .map_or(0.0, ms),
            dns_ms: self.dns.map(ms),
            connect_ms: ms(self.connect),
            tls_ms: self.tls.map(ms),
        }
    }
}

/// Connection phases in milliseconds, relative to the flow's start.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnPhases {
    pub blocked_ms: f64,
    pub dns_ms: Option<f64>,
    pub connect_ms: f64,
    pub tls_ms: Option<f64>,
}

/// Splits the time to the first response byte into phases. `conn` is the
/// connection this flow opened, if any; `request_sent_ms` is when its request
/// body finished going out. Phases are clamped so they never add up to more
/// than `ttfb_ms`. `receive` is filled in once the body has arrived.
pub fn split(
    ttfb_ms: f64,
    request_sent_ms: Option<f64>,
    conn: Option<ConnPhases>,
    reused: bool,
) -> Timings {
    let mut t = Timings {
        reused_connection: reused,
        ..Timings::default()
    };
    let mut at = 0.0_f64;
    if let Some(c) = conn {
        let mut phase = |v: f64| {
            let v = v.max(0.0).min((ttfb_ms - at).max(0.0));
            at += v;
            v
        };
        t.blocked = Some(phase(c.blocked_ms));
        t.dns = c.dns_ms.map(&mut phase);
        t.connect = Some(phase(c.connect_ms));
        t.tls = c.tls_ms.map(&mut phase);
    }
    let send = request_sent_ms
        .map(|s| (s - at).max(0.0))
        .unwrap_or(0.0)
        .min((ttfb_ms - at).max(0.0));
    at += send;
    t.send = Some(send);
    t.wait = Some((ttfb_ms - at).max(0.0));
    t
}

#[derive(Default)]
struct Partial {
    dns: Option<Duration>,
    /// DNS and TCP connect together.
    tcp: Option<Duration>,
}

tokio::task_local! {
    static PARTIAL: Arc<Mutex<Partial>>;
}

fn report(f: impl FnOnce(&mut Partial)) {
    let _ = PARTIAL.try_with(|p| f(&mut p.lock()));
}

type BoxFuture<T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send>>;

/// The system resolver, timed.
#[derive(Clone)]
pub struct TimedResolver(GaiResolver);

impl Default for TimedResolver {
    fn default() -> Self {
        Self(GaiResolver::new())
    }
}

impl Service<Name> for TimedResolver {
    type Response = GaiAddrs;
    type Error = io::Error;
    type Future = BoxFuture<GaiAddrs, io::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let fut = self.0.call(name);
        Box::pin(async move {
            let t = Instant::now();
            let r = fut.await;
            report(|p| p.dns = Some(t.elapsed()));
            r
        })
    }
}

/// Times the TCP connector (which resolves the name first).
#[derive(Clone)]
pub struct TimedTcp<C>(pub C);

impl<C> Service<Uri> for TimedTcp<C>
where
    C: Service<Uri>,
    C::Future: Send + 'static,
    C::Response: Send + 'static,
    C::Error: Send + 'static,
{
    type Response = C::Response;
    type Error = C::Error;
    type Future = BoxFuture<C::Response, C::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let fut = self.0.call(uri);
        Box::pin(async move {
            let t = Instant::now();
            let r = fut.await;
            report(|p| p.tcp = Some(t.elapsed()));
            r
        })
    }
}

/// Times the whole connection attempt and attaches a [`ConnTiming`] to the
/// connection it produces.
#[derive(Clone)]
pub struct TimedConnector<C>(pub C);

impl<C> Service<Uri> for TimedConnector<C>
where
    C: Service<Uri>,
    C::Future: Send + 'static,
    C::Response: Send + 'static,
    C::Error: Send + 'static,
{
    type Response = TimedIo<C::Response>;
    type Error = C::Error;
    type Future = BoxFuture<TimedIo<C::Response>, C::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let https = uri.scheme_str() == Some("https");
        let fut = self.0.call(uri);
        Box::pin(async move {
            let partial = Arc::new(Mutex::new(Partial::default()));
            let started = Instant::now();
            let inner = PARTIAL.scope(partial.clone(), fut).await?;
            let total = started.elapsed();
            let p = partial.lock();
            let tcp = p.tcp.unwrap_or(total);
            let timing = ConnTiming {
                started,
                dns: p.dns,
                connect: tcp.saturating_sub(p.dns.unwrap_or_default()),
                tls: https.then(|| total.saturating_sub(tcp)),
                claimed: Arc::default(),
            };
            Ok(TimedIo { inner, timing })
        })
    }
}

/// A connection that carries its [`ConnTiming`].
pub struct TimedIo<T> {
    inner: T,
    timing: ConnTiming,
}

impl<T: Connection + Unpin> Connection for TimedIo<T> {
    fn connected(&self) -> Connected {
        self.inner.connected().extra(self.timing.clone())
    }
}

impl<T: Read + Unpin> Read for TimedIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: Write + Unpin> Write for TimedIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }
}

pub type TimedHttps = TimedConnector<HttpsConnector<TimedTcp<HttpConnector<TimedResolver>>>>;

/// The upstream connector: HTTP/1.1 and HTTP/2 over TLS or plain TCP, timed.
pub fn connector(tls: ClientConfig) -> TimedHttps {
    let mut http = HttpConnector::new_with_resolver(TimedResolver::default());
    // The TLS layer handles https URIs; the TCP layer must accept them.
    http.enforce_http(false);
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(TimedTcp(http));
    TimedConnector(https)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> ConnPhases {
        ConnPhases {
            blocked_ms: 2.0,
            dns_ms: Some(10.0),
            connect_ms: 20.0,
            tls_ms: Some(30.0),
        }
    }

    #[test]
    fn splits_a_fresh_connection_into_phases() {
        let t = split(100.0, Some(65.0), Some(conn()), false);
        assert_eq!(t.blocked, Some(2.0));
        assert_eq!(t.dns, Some(10.0));
        assert_eq!(t.connect, Some(20.0));
        assert_eq!(t.tls, Some(30.0));
        // The body finished 3 ms after the connection was ready.
        assert_eq!(t.send, Some(3.0));
        assert_eq!(t.wait, Some(35.0));
        assert_eq!(t.receive, None);
        assert!(!t.reused_connection);
    }

    #[test]
    fn a_reused_connection_has_no_connection_phases() {
        let t = split(50.0, Some(0.1), None, true);
        assert_eq!(
            (t.blocked, t.dns, t.connect, t.tls),
            (None, None, None, None)
        );
        assert_eq!(t.send, Some(0.1));
        assert_eq!(t.wait, Some(49.9));
        assert!(t.reused_connection);
    }

    #[test]
    fn plain_http_to_an_ip_has_neither_dns_nor_tls() {
        let c = ConnPhases {
            dns_ms: None,
            tls_ms: None,
            ..conn()
        };
        let t = split(40.0, None, Some(c), false);
        assert_eq!((t.dns, t.tls), (None, None));
        assert_eq!(t.send, Some(0.0));
        assert_eq!(t.wait, Some(18.0));
    }

    #[test]
    fn phases_never_exceed_the_time_to_first_byte() {
        // Clock skew between the connector and the handler can't produce
        // negative waits or phases longer than the whole request.
        let t = split(25.0, Some(100.0), Some(conn()), false);
        let sum: f64 = [t.blocked, t.dns, t.connect, t.tls, t.send, t.wait]
            .iter()
            .flatten()
            .sum();
        assert!((sum - 25.0).abs() < 1e-9, "{t:?}");
        assert_eq!(t.wait, Some(0.0));
        // A body that finished before the connection was ready counts as 0.
        let t = split(100.0, Some(10.0), Some(conn()), false);
        assert_eq!(t.send, Some(0.0));
    }

    #[test]
    fn a_timing_is_claimed_once() {
        let c = ConnTiming {
            started: Instant::now(),
            dns: None,
            connect: Duration::from_millis(5),
            tls: None,
            claimed: Arc::default(),
        };
        let copy = c.clone();
        assert!(c.claim());
        assert!(!copy.claim(), "clones share the claim");
        assert!(!c.claim());
    }

    #[test]
    fn connection_phases_are_relative_to_the_flow() {
        let flow_started = Instant::now();
        let c = ConnTiming {
            started: flow_started + Duration::from_millis(4),
            dns: Some(Duration::from_millis(6)),
            connect: Duration::from_millis(8),
            tls: Some(Duration::from_millis(10)),
            claimed: Arc::default(),
        };
        let p = c.phases(flow_started);
        assert_eq!(p.blocked_ms, 4.0);
        assert_eq!(p.dns_ms, Some(6.0));
        assert_eq!(p.connect_ms, 8.0);
        assert_eq!(p.tls_ms, Some(10.0));
        // A connection opened before the flow started isn't "blocked" time.
        let early = ConnTiming {
            started: flow_started - Duration::from_millis(1),
            ..c
        };
        assert_eq!(early.phases(flow_started).blocked_ms, 0.0);
    }
}
