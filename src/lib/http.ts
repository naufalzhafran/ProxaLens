// Helpers for presenting captured HTTP messages in the inspector.
import type { BodyView } from "../api";
import { isGraphqlRequest, isProtobuf, multipartBoundary } from "./decoders";

export function headerValue(headers: [string, string][], name: string): string | null {
  const n = name.toLowerCase();
  return headers.find(([k]) => k.toLowerCase() === n)?.[1] ?? null;
}

/** Query parameters of a URL, in order, keeping duplicates. */
export function queryParams(url: string): [string, string][] {
  const q = url.indexOf("?");
  if (q < 0) return [];
  const end = url.indexOf("#", q);
  const search = url.slice(q + 1, end < 0 ? undefined : end);
  return [...new URLSearchParams(search)];
}

export interface Cookie {
  name: string;
  value: string;
  /** Set-Cookie attributes (Path, Expires, HttpOnly…); empty for request cookies. */
  attrs: [string, string][];
}

/** Parses every `Cookie` header of a request. */
export function requestCookies(headers: [string, string][]): Cookie[] {
  return headers
    .filter(([k]) => k.toLowerCase() === "cookie")
    .flatMap(([, v]) => v.split(";"))
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p) => {
      const [name, value] = splitPair(p);
      return { name, value, attrs: [] };
    });
}

/** Parses every `Set-Cookie` header of a response. */
export function responseCookies(headers: [string, string][]): Cookie[] {
  return headers
    .filter(([k]) => k.toLowerCase() === "set-cookie")
    .map(([, v]) => {
      const [first, ...rest] = v.split(";").map((p) => p.trim());
      const [name, value] = splitPair(first);
      return { name, value, attrs: rest.filter(Boolean).map(splitPair) };
    });
}

function splitPair(s: string): [string, string] {
  const eq = s.indexOf("=");
  return eq < 0 ? [s, ""] : [s.slice(0, eq).trim(), s.slice(eq + 1).trim()];
}

export function bodyBytes(body: BodyView): Uint8Array {
  if (body.base64 !== null) {
    const bin = atob(body.base64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  return new TextEncoder().encode(body.text ?? "");
}

/** Classic 16-bytes-per-line hex dump with an ASCII gutter. */
export function hexDump(bytes: Uint8Array, limit = bytes.length): string {
  const lines: string[] = [];
  const n = Math.min(limit, bytes.length);
  for (let off = 0; off < n; off += 16) {
    const row = bytes.subarray(off, Math.min(off + 16, n));
    let hex = "";
    let ascii = "";
    for (let i = 0; i < 16; i++) {
      if (i < row.length) {
        hex += row[i].toString(16).padStart(2, "0") + " ";
        ascii += row[i] >= 0x20 && row[i] < 0x7f ? String.fromCharCode(row[i]) : ".";
      } else {
        hex += "   ";
      }
      if (i === 7) hex += " ";
    }
    lines.push(`${off.toString(16).padStart(8, "0")}  ${hex} ${ascii}`);
  }
  return lines.join("\n");
}

export type BodyKind = "json" | "graphql" | "form" | "multipart" | "protobuf" | "image" | "text" | "binary" | "empty";

export function bodyKind(body: BodyView, contentType: string | null): BodyKind {
  if (body.size === 0) return "empty";
  const ct = (contentType ?? "").toLowerCase();
  if (ct.startsWith("image/")) return "image";
  if (isProtobuf(ct)) return "protobuf";
  if (ct.startsWith("multipart/") && multipartBoundary(ct)) return "multipart";
  if (body.text === null) return "binary";
  if (ct.startsWith("application/x-www-form-urlencoded")) return "form";
  if (ct.includes("json") || /^\s*[[{]/.test(body.text)) {
    try {
      return isGraphqlRequest(JSON.parse(body.text)) ? "graphql" : "json";
    } catch {
      /* fall through */
    }
  }
  return "text";
}

/** Data URL for rendering an image body in an <img>. */
export function imageDataUrl(body: BodyView, contentType: string): string {
  const mime = contentType.split(";")[0].trim();
  if (body.base64 !== null) return `data:${mime};base64,${body.base64}`;
  return `data:${mime};charset=utf-8,${encodeURIComponent(body.text ?? "")}`;
}

/** The message as it would look on the wire (HTTP/1.1 framing). */
export function rawRequest(
  method: string,
  path: string,
  version: string,
  headers: [string, string][],
  body: BodyView,
): string {
  return raw(`${method} ${path} ${version}`, headers, body);
}

export function rawResponse(
  status: number | null,
  version: string,
  headers: [string, string][],
  body: BodyView,
): string {
  const reason = status !== null ? (REASONS[status] ?? "") : "";
  return raw(`${version} ${status ?? "—"} ${reason}`.trimEnd(), headers, body);
}

function raw(start: string, headers: [string, string][], body: BodyView): string {
  const head = [start, ...headers.map(([k, v]) => `${k}: ${v}`)].join("\n");
  let tail = "";
  if (body.size > 0) {
    tail = body.text ?? `<${body.size} bytes of binary data>`;
    if (body.truncated) tail += "\n<truncated>";
  }
  return `${head}\n\n${tail}`;
}

const REASONS: Record<number, string> = {
  100: "Continue", 101: "Switching Protocols", 200: "OK", 201: "Created", 202: "Accepted",
  204: "No Content", 206: "Partial Content", 301: "Moved Permanently", 302: "Found",
  303: "See Other", 304: "Not Modified", 307: "Temporary Redirect", 308: "Permanent Redirect",
  400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found",
  405: "Method Not Allowed", 409: "Conflict", 410: "Gone", 413: "Payload Too Large",
  415: "Unsupported Media Type", 422: "Unprocessable Entity", 429: "Too Many Requests",
  500: "Internal Server Error", 501: "Not Implemented", 502: "Bad Gateway",
  503: "Service Unavailable", 504: "Gateway Timeout",
};

// Common second-level labels under country-code TLDs (co.uk, com.au, co.id…).
const SECOND_LEVEL = new Set(["co", "com", "net", "org", "gov", "edu", "ac", "or", "go", "ne", "web", "my"]);

/**
 * The registrable domain used to group hosts in the sidebar. This is a
 * heuristic, not the Public Suffix List; it only needs to group sensibly.
 */
export function baseDomain(host: string): string {
  if (/^[\d.]+$/.test(host) || host.includes(":") || host.startsWith("[")) return host;
  const labels = host.split(".");
  if (labels.length <= 2) return host;
  const tld = labels[labels.length - 1];
  const sld = labels[labels.length - 2];
  const n = tld.length === 2 && SECOND_LEVEL.has(sld) ? 3 : 2;
  return labels.slice(-n).join(".");
}
