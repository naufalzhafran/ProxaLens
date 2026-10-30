// Body decoders beyond JSON and text: URL-encoded forms, multipart bodies,
// GraphQL requests, protobuf messages (without a schema) and gRPC framing.
// All pure, so they can be tested without the UI.

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

// ---- application/x-www-form-urlencoded ------------------------------------

/** Fields of a form body, in order, keeping duplicates. */
export function parseForm(text: string): [string, string][] {
  try {
    return [...new URLSearchParams(text.replace(/^\s+|\s+$/g, ""))];
  } catch {
    return [];
  }
}

// ---- multipart/form-data ---------------------------------------------------

export interface Part {
  /** From Content-Disposition. */
  name: string | null;
  filename: string | null;
  contentType: string | null;
  headers: [string, string][];
  body: Uint8Array;
  /** The body as text when it is valid UTF-8 without control bytes. */
  text: string | null;
}

/** The boundary from a multipart Content-Type, or null. */
export function multipartBoundary(contentType: string | null): string | null {
  const m = /;\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType ?? "");
  return m ? (m[1] ?? m[2]) : null;
}

const CRLF = new Uint8Array([13, 10]);

function indexOf(hay: Uint8Array, needle: Uint8Array, from = 0): number {
  outer: for (let i = from; i <= hay.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
    return i;
  }
  return -1;
}

function startsWith(hay: Uint8Array, needle: Uint8Array, at: number): boolean {
  if (at + needle.length > hay.length) return false;
  for (let j = 0; j < needle.length; j++) if (hay[at + j] !== needle[j]) return false;
  return true;
}

const utf8 = new TextDecoder("utf-8", { fatal: true });
const latin1 = new TextDecoder("latin1");

/** UTF-8 text without control bytes (tabs and newlines allowed), or null. */
export function textOf(bytes: Uint8Array): string | null {
  for (const b of bytes) if (b < 0x20 && b !== 9 && b !== 10 && b !== 13) return null;
  try {
    return utf8.decode(bytes);
  } catch {
    return null;
  }
}

/** Splits a multipart body into its parts (RFC 2046). */
export function parseMultipart(bytes: Uint8Array, boundary: string): Part[] {
  const delim = new TextEncoder().encode(`--${boundary}`);
  const parts: Part[] = [];
  let at = indexOf(bytes, delim);
  while (at >= 0) {
    let p = at + delim.length;
    if (startsWith(bytes, new Uint8Array([45, 45]), p)) break; // closing "--"
    const eol = indexOf(bytes, CRLF, p);
    if (eol < 0) break;
    p = eol + 2;
    const next = indexOf(bytes, delim, p);
    const end = next < 0 ? bytes.length : next;
    const content = bytes.subarray(p, Math.max(p, end - 2)); // drop the CRLF before the delimiter
    parts.push(parsePart(content));
    at = next;
  }
  return parts;
}

function parsePart(content: Uint8Array): Part {
  const sep = indexOf(content, new Uint8Array([13, 10, 13, 10]));
  const headText = latin1.decode(sep < 0 ? content : content.subarray(0, sep));
  const body = sep < 0 ? new Uint8Array() : content.subarray(sep + 4);
  const headers: [string, string][] = headText
    .split("\r\n")
    .filter(Boolean)
    .map((l) => {
      const i = l.indexOf(":");
      return i < 0 ? [l.trim(), ""] : [l.slice(0, i).trim(), l.slice(i + 1).trim()];
    });
  const disposition = headers.find(([k]) => k.toLowerCase() === "content-disposition")?.[1] ?? "";
  const param = (name: string) => {
    const m = new RegExp(`;\\s*${name}=(?:"([^"]*)"|([^;]+))`, "i").exec(disposition);
    return m ? (m[1] ?? m[2]).trim() : null;
  };
  const contentType = headers.find(([k]) => k.toLowerCase() === "content-type")?.[1] ?? null;
  return { name: param("name"), filename: param("filename"), contentType, headers, body, text: textOf(body) };
}

// ---- GraphQL ---------------------------------------------------------------

export interface GraphqlRequest {
  operationName: string | null;
  query: string;
  variables: Json | undefined;
  /** Persisted query hashes and the like. */
  extensions: Json | undefined;
}

export interface Operation {
  type: "query" | "mutation" | "subscription";
  name: string | null;
}

/** True for a JSON object (or array of objects) shaped like a GraphQL request. */
export function isGraphqlRequest(value: unknown): boolean {
  const items = Array.isArray(value) ? value : [value];
  return (
    items.length > 0 &&
    items.every(
      (v) => v && typeof v === "object" && !Array.isArray(v) && looksLikeGraphql((v as Record<string, unknown>).query),
    )
  );
}

function looksLikeGraphql(query: unknown): boolean {
  return typeof query === "string" && /^\s*(query|mutation|subscription|fragment|\{)/.test(query);
}

/** The requests in a GraphQL body (one, or a batch); null if it isn't one. */
export function parseGraphql(text: string): GraphqlRequest[] | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isGraphqlRequest(value)) return null;
  return (Array.isArray(value) ? value : [value]).map((v) => {
    const o = v as Record<string, Json>;
    return {
      operationName: typeof o.operationName === "string" ? o.operationName : null,
      query: o.query as string,
      variables: o.variables,
      extensions: o.extensions,
    };
  });
}

/** The operation a GraphQL document defines (its first one). */
export function graphqlOperation(query: string): Operation {
  const m = /^\s*(?:#[^\n]*\n\s*)*(query|mutation|subscription)\b\s*([A-Za-z_]\w*)?/.exec(query);
  if (!m) return { type: "query", name: null };
  return { type: m[1] as Operation["type"], name: m[2] ?? null };
}

// ---- Protocol Buffers ------------------------------------------------------

/** A field decoded without a schema; a plain JSON shape so the tree can show it. */
export type ProtoMessage = { [field: string]: Json };

const MAX_FIELD = 536_870_911;

class Reader {
  pos = 0;
  constructor(readonly buf: Uint8Array) {}
  get done() {
    return this.pos >= this.buf.length;
  }
  varint(): bigint {
    let result = 0n;
    let shift = 0n;
    for (;;) {
      if (this.pos >= this.buf.length || shift > 63n) throw new Error("truncated varint");
      const b = this.buf[this.pos++];
      result |= BigInt(b & 0x7f) << shift;
      if ((b & 0x80) === 0) return result;
      shift += 7n;
    }
  }
  bytes(n: number): Uint8Array {
    if (this.pos + n > this.buf.length) throw new Error("truncated field");
    const out = this.buf.subarray(this.pos, this.pos + n);
    this.pos += n;
    return out;
  }
}

function numberOf(v: bigint): Json {
  return v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v.toString();
}

function hex(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}

/**
 * Decodes a protobuf message without its schema. Field numbers become keys,
 * repeated fields become arrays. Varints are shown unsigned; length-delimited
 * fields are shown as text when printable, as a nested message when they
 * decode as one, and as hex otherwise. Throws on malformed input.
 */
export function decodeProto(bytes: Uint8Array, depth = 0): ProtoMessage {
  const r = new Reader(bytes);
  const out: ProtoMessage = {};
  const repeated = new Set<string>();
  while (!r.done) {
    const tag = r.varint();
    const field = Number(tag >> 3n);
    const wire = Number(tag & 7n);
    if (field < 1 || field > MAX_FIELD) throw new Error("bad field number");
    let value: Json;
    switch (wire) {
      case 0:
        value = numberOf(r.varint());
        break;
      case 1: {
        const b = r.bytes(8);
        value = fixed(b, 64);
        break;
      }
      case 5: {
        const b = r.bytes(4);
        value = fixed(b, 32);
        break;
      }
      case 2: {
        const n = Number(r.varint());
        value = lengthDelimited(r.bytes(n), depth);
        break;
      }
      default:
        throw new Error(`unsupported wire type ${wire}`);
    }
    const key = String(field);
    if (!(key in out)) out[key] = value;
    else if (repeated.has(key)) (out[key] as Json[]).push(value);
    else {
      out[key] = [out[key], value];
      repeated.add(key);
    }
  }
  return out;
}

function fixed(b: Uint8Array, bits: 32 | 64): Json {
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  if (bits === 32) {
    const f = view.getFloat32(0, true);
    const i = view.getUint32(0, true);
    return Number.isFinite(f) && Math.abs(f) > 1e-6 && Math.abs(f) < 1e12 && !Number.isInteger(f) ? f : i;
  }
  const f = view.getFloat64(0, true);
  const i = view.getBigUint64(0, true);
  return Number.isFinite(f) && Math.abs(f) > 1e-6 && Math.abs(f) < 1e15 && !Number.isInteger(f) ? f : numberOf(i);
}

function lengthDelimited(b: Uint8Array, depth: number): Json {
  if (b.length === 0) return "";
  const text = textOf(b);
  if (text !== null) return text;
  if (depth < 16) {
    try {
      return decodeProto(b, depth + 1);
    } catch {
      /* not a message */
    }
  }
  return `0x${hex(b)}`;
}

// ---- gRPC ------------------------------------------------------------------

export type GrpcFrame =
  | { kind: "message"; compressed: boolean; size: number; message: ProtoMessage | null; raw: Uint8Array }
  | { kind: "trailers"; headers: [string, string][] };

export function isGrpc(contentType: string | null): boolean {
  return /^application\/grpc(-web)?(-text)?(\+|$|;)/i.test((contentType ?? "").trim());
}

export function isProtobuf(contentType: string | null): boolean {
  const ct = (contentType ?? "").toLowerCase();
  return isGrpc(ct) || /^application\/(x-)?protobuf|^application\/vnd\.google\.protobuf|\+proto(buf)?(;|$)/.test(ct);
}

/** grpc-web-text bodies are base64, possibly as several concatenated chunks. */
export function decodeGrpcWebText(text: string): Uint8Array {
  const chunks = text.match(/[A-Za-z0-9+/]+=*/g) ?? [];
  const parts = chunks.map((c) => Uint8Array.from(atob(c), (ch) => ch.charCodeAt(0)));
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/** Splits a gRPC body into its length-prefixed frames. Throws when malformed. */
export function parseGrpcFrames(bytes: Uint8Array): GrpcFrame[] {
  const frames: GrpcFrame[] = [];
  let at = 0;
  while (at < bytes.length) {
    if (at + 5 > bytes.length) throw new Error("truncated gRPC frame header");
    const flags = bytes[at];
    const len = ((bytes[at + 1] << 24) | (bytes[at + 2] << 16) | (bytes[at + 3] << 8) | bytes[at + 4]) >>> 0;
    at += 5;
    if (at + len > bytes.length) throw new Error("truncated gRPC frame");
    const payload = bytes.subarray(at, at + len);
    at += len;
    if (flags & 0x80) {
      const headers = latin1
        .decode(payload)
        .split(/\r?\n/)
        .filter(Boolean)
        .map((l): [string, string] => {
          const i = l.indexOf(":");
          return i < 0 ? [l.trim(), ""] : [l.slice(0, i).trim(), l.slice(i + 1).trim()];
        });
      frames.push({ kind: "trailers", headers });
      continue;
    }
    const compressed = (flags & 1) === 1;
    let message: ProtoMessage | null = null;
    if (!compressed) {
      try {
        message = decodeProto(payload);
      } catch {
        message = null;
      }
    }
    frames.push({ kind: "message", compressed, size: len, message, raw: payload });
  }
  return frames;
}
