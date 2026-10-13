//! Flow filter expressions, a port of httpmon's `internal/filter` (the same
//! syntax as the UI's `src/lib/filter.ts`, which is what MCP tools accept).
//! Whitespace-separated terms are ANDed; a leading `!` negates:
//! `s:200` / `s:4xx` (`status:`), `m:post` (`method:`), `ct:json`
//! (`type:`), `re:/v[12]/` (regex on host+path), `app:chrome` (`p:`), and
//! anything else is a case-insensitive substring of host+path.

use regex::Regex;

use super::store::FlowMeta;

enum Term {
    Status(Option<(u16, u16)>),
    Method(String),
    ContentType(String),
    Regex(Option<Regex>),
    App(String),
    Text(String),
}

pub struct Filter {
    terms: Vec<(bool, Term)>,
}

impl Filter {
    /// Compiles an expression; `None` when it is empty (matches everything).
    pub fn compile(input: &str) -> Option<Self> {
        let terms: Vec<(bool, Term)> = input.split_whitespace().map(parse_term).collect();
        (!terms.is_empty()).then_some(Self { terms })
    }

    pub fn matches(&self, f: &FlowMeta) -> bool {
        self.terms.iter().all(|(negate, t)| t.matches(f) != *negate)
    }
}

/// Convenience for optional filters.
pub fn matches(filter: Option<&Filter>, f: &FlowMeta) -> bool {
    filter.is_none_or(|x| x.matches(f))
}

fn parse_term(raw: &str) -> (bool, Term) {
    let (negate, s) = match raw.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let term = match s.split_once(':') {
        Some((key, val)) if !key.is_empty() => match key.to_ascii_lowercase().as_str() {
            "s" | "status" => Term::Status(parse_status(val)),
            "m" | "method" => Term::Method(val.to_ascii_uppercase()),
            "ct" | "type" => Term::ContentType(val.to_lowercase()),
            "re" => Term::Regex(Regex::new(val.trim_start_matches('/').trim_end_matches('/')).ok()),
            "app" | "p" => Term::App(val.to_lowercase()),
            _ => Term::Text(s.to_lowercase()),
        },
        _ => Term::Text(s.to_lowercase()),
    };
    (negate, term)
}

/// `2xx` → 200..300, `404` → 404..405; anything else matches nothing.
fn parse_status(val: &str) -> Option<(u16, u16)> {
    let v = val.to_ascii_lowercase();
    if let Some(d) = v.strip_suffix("xx")
        && let Ok(d) = d.parse::<u16>()
        && (1..=5).contains(&d)
    {
        return Some((d * 100, d * 100 + 100));
    }
    v.parse::<u16>().ok().map(|c| (c, c + 1))
}

impl Term {
    fn matches(&self, f: &FlowMeta) -> bool {
        match self {
            Term::Status(range) => {
                matches!((range, f.status), (Some((lo, hi)), Some(s)) if s >= *lo && s < *hi)
            }
            Term::Method(m) => f.method.eq_ignore_ascii_case(m),
            Term::ContentType(ct) => f
                .content_type
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(ct),
            Term::Regex(re) => re
                .as_ref()
                .is_some_and(|r| r.is_match(&format!("{}{}", f.host, f.path))),
            Term::App(a) => f
                .process
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(a),
            Term::Text(q) => format!("{}{}", f.host, f.path).to_lowercase().contains(q),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::store::test_meta;

    fn flow(method: &str, status: Option<u16>, host: &str, path: &str, ct: &str) -> FlowMeta {
        let mut m = test_meta(path);
        m.method = method.into();
        m.status = status;
        m.host = host.into();
        m.content_type = (!ct.is_empty()).then(|| ct.into());
        m
    }

    fn check(q: &str, f: &FlowMeta) -> bool {
        Filter::compile(q).unwrap().matches(f)
    }

    // Ported from httpmon internal/filter/advanced_test.go and quick_test.go.
    #[test]
    fn matches_like_httpmon() {
        assert!(Filter::compile("   ").is_none());
        let f = flow("GET", Some(200), "api.stripe.com", "/v1/charges", "");
        assert!(check("STRIPE", &f) && check("com/v1", &f) && !check("github", &f));
        assert!(
            check("s:200", &f) && !check("s:201", &f) && check("s:2xx", &f) && !check("s:4xx", &f)
        );
        assert!(!check("s:abc", &f));
        assert!(!check("s:2xx", &flow("GET", None, "x", "/", "")));
        assert!(check("m:get", &f) && !check("m:post", &f));
        let json = flow("GET", Some(200), "x.com", "/", "application/json");
        assert!(check("ct:json", &json) && !check("!ct:json", &json));
        assert!(check("re:/v[12]/charges", &f) && !check("re:/v3/", &f) && !check("re:(", &f));
        assert!(!check("!m:GET", &f) && check("!s:4xx", &f));
        let q = "m:POST s:2xx api.example";
        assert!(check(
            q,
            &flow("POST", Some(201), "api.example.com", "/create", "")
        ));
        assert!(!check(
            q,
            &flow("POST", Some(404), "api.example.com", "/create", "")
        ));
        let mut app = f.clone();
        app.process = Some("Google Chrome".into());
        assert!(check("app:chrome", &app) && !check("p:safari", &app));
    }
}
