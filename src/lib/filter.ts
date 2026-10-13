// Flow filter syntax, ported from httpmon's internal/filter (quick.go and
// advanced.go). Whitespace-separated terms are ANDed; a leading `!` negates:
//
//   s:200  s:4xx   status (alias `status:`)
//   m:post         method, case-insensitive (alias `method:`)
//   ct:json        content-type substring (alias `type:`)
//   re:/v[12]/     regex on host+path
//   anything else  case-insensitive substring on host+path
import type { FlowMeta } from "../api";

type Matcher = (f: FlowMeta) => boolean;

export type FlowPredicate = Matcher;

const never: Matcher = () => false;

/** Compiles a filter expression; returns null when it matches everything. */
export function compileFilter(input: string): FlowPredicate | null {
  const terms = input.trim().split(/\s+/).filter(Boolean).map(parseTerm);
  if (!terms.length) return null;
  return (f) => terms.every((t) => t(f));
}

function parseTerm(raw: string): Matcher {
  let s = raw;
  const negate = s.startsWith("!");
  if (negate) s = s.slice(1);
  const m = compileTerm(s);
  return negate ? (f) => !m(f) : m;
}

function compileTerm(s: string): Matcher {
  const colon = s.indexOf(":");
  if (colon > 0) {
    const key = s.slice(0, colon).toLowerCase();
    const val = s.slice(colon + 1);
    switch (key) {
      case "s":
      case "status":
        return compileStatus(val);
      case "m":
      case "method":
        return (f) => f.method.toLowerCase() === val.toLowerCase();
      case "ct":
      case "type":
        return (f) => (f.contentType ?? "").toLowerCase().includes(val.toLowerCase());
      case "re":
        return compileRegex(val);
    }
  }
  const q = s.toLowerCase();
  return (f) => (f.host + f.path).toLowerCase().includes(q);
}

function compileStatus(val: string): Matcher {
  const range = /^([1-5])xx$/i.exec(val);
  if (range) {
    const base = Number(range[1]) * 100;
    return (f) => f.status !== null && f.status >= base && f.status < base + 100;
  }
  if (!/^\d+$/.test(val)) return never;
  const code = Number(val);
  return (f) => f.status === code;
}

function compileRegex(val: string): Matcher {
  const src = val.replace(/^\//, "").replace(/\/$/, "");
  try {
    const re = new RegExp(src);
    return (f) => re.test(f.host + f.path);
  } catch {
    return never;
  }
}

/** Status classes offered as chips in the filter bar. */
export const STATUS_CLASSES = ["2xx", "3xx", "4xx", "5xx", "error"] as const;
export type StatusClass = (typeof STATUS_CLASSES)[number];

export const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] as const;

export function statusClass(f: FlowMeta): StatusClass | null {
  if (f.state === "failed") return "error";
  if (f.status === null) return null;
  const c = `${Math.floor(f.status / 100)}xx`;
  return (STATUS_CLASSES as readonly string[]).includes(c) ? (c as StatusClass) : null;
}

/** What the sidebar has selected: one host, or a domain and its subdomains. */
export type Scope = { kind: "host" | "domain"; value: string } | null;

export function inScope(f: FlowMeta, scope: Scope): boolean {
  if (!scope) return true;
  if (scope.kind === "host") return f.host === scope.value;
  return f.host === scope.value || f.host.endsWith(`.${scope.value}`);
}

export interface FilterState {
  text: string;
  methods: string[];
  statuses: StatusClass[];
  scope: Scope;
}

/** Combines the text filter, chips and sidebar scope into one predicate. */
export function buildPredicate(s: FilterState): FlowPredicate {
  const text = compileFilter(s.text);
  const methods = new Set(s.methods);
  const statuses = new Set<StatusClass>(s.statuses);
  return (f) => {
    if (!inScope(f, s.scope)) return false;
    if (methods.size && !methods.has(f.method.toUpperCase())) return false;
    if (statuses.size) {
      const c = statusClass(f);
      if (!c || !statuses.has(c)) return false;
    }
    return text ? text(f) : true;
  };
}
