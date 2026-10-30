//! Shows TLS handshake failures between a client and the proxy as flows.
//!
//! When a client refuses our certificate (it pins its own, or the CA isn't
//! trusted) hudsucker only logs an error and drops the connection. This
//! `tracing` layer picks those log events up and records them in the store,
//! so the user sees which hosts fail and can stop decrypting them.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber, span};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use super::proxy::now_ms;
use super::store::{FlowData, FlowId, FlowMeta, FlowState, Store};

/// hudsucker's message for a failed handshake with the client.
const HANDSHAKE_FAILED: &str = "Failed to establish TLS connection";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeFailure {
    /// "host:port" from the CONNECT request.
    pub authority: String,
    pub client_addr: String,
    pub error: String,
}

type Sink = Box<dyn Fn(HandshakeFailure) + Send + Sync>;

/// Lets the app connect the layer to the store once the engine exists
/// (logging is set up before that).
#[derive(Clone, Default)]
pub struct Failures(Arc<Mutex<Option<Sink>>>);

impl Failures {
    pub fn connect(&self, sink: impl Fn(HandshakeFailure) + Send + Sync + 'static) {
        *self.0.lock() = Some(Box::new(sink));
    }
}

pub fn layer() -> (TlsFailureLayer, Failures) {
    let failures = Failures::default();
    (
        TlsFailureLayer {
            failures: failures.clone(),
        },
        failures,
    )
}

pub struct TlsFailureLayer {
    failures: Failures,
}

/// The CONNECT span's fields, kept in the span's extensions.
struct ConnectFields {
    uri: String,
    client_addr: String,
}

#[derive(Default)]
struct Fields {
    message: String,
    uri: String,
    client_addr: String,
    error: String,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let v = format!("{value:?}");
        match field.name() {
            "message" => self.message = v,
            "uri" => self.uri = v,
            "client_addr" => self.client_addr = v,
            "error" => self.error = v,
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_debug(field, &format_args!("{value}"));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if field.name() == "error" {
            self.error = value.to_string();
        }
    }
}

impl<S> Layer<S> for TlsFailureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let meta = attrs.metadata();
        if meta.name() != "proxy" || !meta.target().starts_with("hudsucker") {
            return;
        }
        let mut f = Fields::default();
        attrs.record(&mut f);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(ConnectFields {
                uri: f.uri,
                client_addr: f.client_addr,
            });
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() != Level::ERROR || !meta.target().starts_with("hudsucker") {
            return;
        }
        let mut f = Fields::default();
        event.record(&mut f);
        if f.message != HANDSHAKE_FAILED {
            return;
        }
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        for span in scope {
            if let Some(c) = span.extensions().get::<ConnectFields>() {
                if let Some(sink) = self.failures.0.lock().as_ref() {
                    sink(HandshakeFailure {
                        authority: c.uri.clone(),
                        client_addr: c.client_addr.clone(),
                        error: f.error.clone(),
                    });
                }
                return;
            }
        }
    }
}

/// A short explanation of why the client hung up.
pub fn describe(error: &str) -> String {
    let e = error.to_ascii_lowercase();
    let hint = "It may pin its certificate, or the ProxaLens certificate isn't trusted. \
                Right-click → Don't Decrypt This Host to let it through untouched.";
    if e.contains("eof") || e.contains("reset") {
        format!("The client closed the connection during the TLS handshake. {hint}")
    } else if e.contains("alert") {
        format!("The client rejected the proxy's certificate ({error}). {hint}")
    } else {
        format!("TLS handshake with the client failed: {error}")
    }
}

/// Records failures as flows, folding repeats for the same host into the
/// most recent row so a retrying client doesn't flood the list.
pub fn recorder(store: Arc<Store>) -> impl Fn(HandshakeFailure) + Send + Sync + 'static {
    let last: Mutex<HashMap<String, (FlowId, u32)>> = Mutex::new(HashMap::new());
    move |f: HandshakeFailure| {
        let error = describe(&f.error);
        let mut last = last.lock();
        if let Some((id, count)) = last.get_mut(&f.authority)
            && store.get(*id).is_some()
        {
            *count += 1;
            let n = *count;
            store.update(*id, |m, _| {
                m.error = Some(format!("{error} (failed {n} times)"))
            });
            return;
        }
        let host = f
            .authority
            .rsplit_once(':')
            .map_or(f.authority.as_str(), |(h, _)| h)
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let meta = FlowMeta {
            id: 0,
            method: "CONNECT".into(),
            scheme: "https".into(),
            host,
            path: String::new(),
            url: f.authority.clone(),
            http_version: "HTTP/1.1".into(),
            status: None,
            state: FlowState::Failed,
            content_type: None,
            started_at: now_ms(),
            ttfb_ms: None,
            duration_ms: Some(0.0),
            request_size: 0,
            response_size: 0,
            client_addr: f.client_addr,
            error: Some(error),
            ws_message_count: 0,
            timings: None,
            tags: Vec::new(),
            process: None,
            pid: None,
        };
        let id = store.add(meta, FlowData::default());
        last.insert(f.authority, (id, 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    fn emit_failure(uri: &str, error: &str) {
        let span = tracing::info_span!(target: "hudsucker::proxy::internal", "proxy", uri = %uri, client_addr = %"127.0.0.1:5000");
        let _g = span.enter();
        let inner = tracing::info_span!(target: "hudsucker::proxy::internal", "process_connect");
        let _g2 = inner.enter();
        let err = std::io::Error::other(error.to_string());
        tracing::error!(target: "hudsucker::proxy::internal", error = &err as &dyn std::error::Error, "Failed to establish TLS connection");
        tracing::error!(target: "hudsucker::proxy::internal", "Some other error");
        tracing::error!(target: "my_app", "Failed to establish TLS connection");
    }

    #[test]
    fn layer_reports_handshake_failures_with_their_connect_target() {
        let (layer, failures) = layer();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        failures.connect(move |f| s.lock().push(f));
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            emit_failure("gateway.icloud.com:443", "tls handshake eof")
        });
        assert_eq!(
            *seen.lock(),
            vec![HandshakeFailure {
                authority: "gateway.icloud.com:443".into(),
                client_addr: "127.0.0.1:5000".into(),
                error: "tls handshake eof".into(),
            }]
        );
    }

    #[test]
    fn recorder_folds_repeats_into_one_flow() {
        let store = Arc::new(Store::new(100));
        let record = recorder(store.clone());
        let fail = |a: &str| HandshakeFailure {
            authority: a.into(),
            client_addr: "127.0.0.1:1".into(),
            error: "received fatal alert: CertificateUnknown".into(),
        };
        record(fail("ssl.gstatic.com:443"));
        record(fail("ssl.gstatic.com:443"));
        record(fail("other.com:443"));
        let (flows, total) = store.list(0, 10);
        assert_eq!(total, 2);
        assert_eq!(
            (flows[0].host.as_str(), flows[0].state),
            ("ssl.gstatic.com", FlowState::Failed)
        );
        let err = flows[0].error.as_deref().unwrap();
        assert!(
            err.contains("rejected the proxy's certificate") && err.ends_with("(failed 2 times)"),
            "{err}"
        );

        // A cleared list starts counting afresh.
        store.clear();
        record(fail("ssl.gstatic.com:443"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn describes_common_errors() {
        assert!(describe("tls handshake eof").contains("closed the connection"));
        assert!(describe("received fatal alert: UnknownCA").contains("rejected"));
        assert!(describe("weird").starts_with("TLS handshake with the client failed"));
    }
}
