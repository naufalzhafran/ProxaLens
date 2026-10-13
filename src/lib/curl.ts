// Copy-as-cURL, ported from httpmon's internal/tui/curl.go. Unlike the
// original, values are shell-escaped and Content-Length is dropped (curl
// computes it, and a truncated body would make it wrong).
import type { BodyView } from "../api";

const SKIP = new Set(["content-length"]);

/** Single-quotes `s` for a POSIX shell. */
export function shellQuote(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

export function formatCurl(
  method: string,
  url: string,
  headers: [string, string][],
  body: BodyView | null,
): string {
  const parts = ["curl"];
  if (method !== "GET") parts.push("-X", method);
  const sorted = headers
    .filter(([k]) => !SKIP.has(k.toLowerCase()))
    .sort(([a], [b]) => (a.toLowerCase() < b.toLowerCase() ? -1 : a.toLowerCase() > b.toLowerCase() ? 1 : 0));
  for (const [k, v] of sorted) parts.push("-H", shellQuote(`${k}: ${v}`));
  if (body && body.size > 0) {
    if (body.text !== null) parts.push("--data-raw", shellQuote(body.text));
    else if (body.base64 !== null) parts.push("--data-binary", `@<(echo ${shellQuote(body.base64)} | base64 -d)`);
  }
  parts.push(shellQuote(url));
  return parts.join(" ");
}
