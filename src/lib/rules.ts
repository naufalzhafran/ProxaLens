// Helpers for building rules in the UI. Matching itself happens in the
// engine (src-tauri/src/engine/rules.rs).
import type {
  BreakpointRule,
  FlowDetail,
  MapLocalRule,
  MapRemoteRule,
  Matcher,
  RewriteRule,
  Rules,
} from "../api";

export type RuleKind = keyof Rules;

export const RULE_KINDS: { id: RuleKind; label: string }[] = [
  { id: "mapLocal", label: "Map Local" },
  { id: "mapRemote", label: "Map Remote" },
  { id: "rewrite", label: "Rewrite" },
  { id: "block", label: "Block" },
  { id: "breakpoints", label: "Breakpoints" },
];

/** Enabled rules per kind, skipping kinds with none: [["Map Local", 2], …]. */
export function activeRules(rules: Rules): [string, number][] {
  return RULE_KINDS.map(({ id, label }) => [label, rules[id].filter((r) => r.enabled).length] as [string, number]).filter(
    ([, n]) => n > 0,
  );
}


function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function matcher(url = ""): Matcher {
  return { id: newId(), name: "", enabled: true, method: "", url, regex: false };
}

/** A wildcard pattern for a captured URL: the same path, any query. */
export function patternFor(url: string): string {
  const q = url.indexOf("?");
  return `${q < 0 ? url : url.slice(0, q)}*`;
}

export function newRule(kind: RuleKind, url = ""): Rules[RuleKind][number] {
  const m = matcher(url);
  switch (kind) {
    case "mapLocal":
      return { ...m, status: 200, headers: [["Content-Type", "application/json"]], body: "", file: "" } satisfies MapLocalRule;
    case "mapRemote":
      return { ...m, to: "" } satisfies MapRemoteRule;
    case "rewrite":
      return { ...m, target: "responseHeader", action: "set", header: "", find: "", value: "" } satisfies RewriteRule;
    case "breakpoints":
      return { ...m, request: true, response: true } satisfies BreakpointRule;
    case "block":
      return m;
  }
}

/** A Map Local rule that replays a captured response. */
export function mapLocalFrom(d: FlowDetail): MapLocalRule {
  const skip = new Set(["content-length", "content-encoding", "transfer-encoding", "connection"]);
  return {
    ...(newRule("mapLocal", patternFor(d.meta.url)) as MapLocalRule),
    method: d.meta.method,
    status: d.meta.status ?? 200,
    headers: d.responseHeaders.filter(([k]) => !skip.has(k.toLowerCase())),
    body: d.responseBody.text ?? "",
  };
}

export function ruleSummary(kind: RuleKind, r: Rules[RuleKind][number]): string {
  switch (kind) {
    case "mapLocal": {
      const ml = r as MapLocalRule;
      return ml.file ? `→ ${ml.file}` : `→ ${ml.status} inline`;
    }
    case "mapRemote":
      return `→ ${(r as MapRemoteRule).to || "?"}`;
    case "rewrite": {
      const rw = r as RewriteRule;
      return `${rw.action} ${rw.target.replace(/([A-Z])/g, " $1").toLowerCase()}${rw.header ? ` ${rw.header}` : ""}`;
    }
    case "breakpoints": {
      const b = r as BreakpointRule;
      return [b.request && "request", b.response && "response"].filter(Boolean).join(" + ") || "off";
    }
    case "block":
      return "→ 403";
  }
}

/** Which rewrite actions make sense for a target. */
export function actionsFor(target: RewriteRule["target"]): RewriteRule["action"][] {
  if (target === "responseStatus") return ["set"];
  if (target.endsWith("Body")) return ["set", "replace", "add", "remove"];
  return ["set", "add", "remove", "replace"];
}
