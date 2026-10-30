import { describe, expect, it } from "vitest";
import type { BodyView } from "../api";
import {
  decodeGrpcWebText,
  decodeProto,
  graphqlOperation,
  isGraphqlRequest,
  isProtobuf,
  multipartBoundary,
  parseForm,
  parseGraphql,
  parseGrpcFrames,
  parseMultipart,
  textOf,
} from "./decoders";
import { bodyKind } from "./http";

const enc = (s: string) => new TextEncoder().encode(s);
const text = (t: string): BodyView => ({ size: t.length, truncated: false, text: t, base64: null });
const bin = (bytes: number[]): BodyView => ({
  size: bytes.length,
  truncated: false,
  text: null,
  base64: btoa(String.fromCharCode(...bytes)),
});

describe("forms", () => {
  it("parses url-encoded fields in order, keeping duplicates", () => {
    expect(parseForm("a=1&b=x+y%21&a=2&empty=")).toEqual([
      ["a", "1"],
      ["b", "x y!"],
      ["a", "2"],
      ["empty", ""],
    ]);
    expect(parseForm("")).toEqual([]);
  });
});

describe("multipart", () => {
  const body =
    "--xyz\r\n" +
    'Content-Disposition: form-data; name="title"\r\n\r\n' +
    "Hello\r\n" +
    "--xyz\r\n" +
    'Content-Disposition: form-data; name="file"; filename="a.bin"\r\n' +
    "Content-Type: application/octet-stream\r\n\r\n" +
    "\u0000\u0001\u0002\r\n" +
    "--xyz--\r\n";

  it("finds the boundary", () => {
    expect(multipartBoundary('multipart/form-data; boundary=xyz')).toBe("xyz");
    expect(multipartBoundary('multipart/form-data; boundary="a b"; charset=utf-8')).toBe("a b");
    expect(multipartBoundary("application/json")).toBeNull();
  });

  it("splits parts and reads their headers", () => {
    const bytes = Uint8Array.from(body, (c) => c.charCodeAt(0));
    const parts = parseMultipart(bytes, "xyz");
    expect(parts).toHaveLength(2);
    expect(parts[0]).toMatchObject({ name: "title", filename: null, contentType: null, text: "Hello" });
    expect(parts[1]).toMatchObject({ name: "file", filename: "a.bin", contentType: "application/octet-stream", text: null });
    expect([...parts[1].body]).toEqual([0, 1, 2]);
  });

  it("copes with a truncated body", () => {
    const bytes = enc('--xyz\r\nContent-Disposition: form-data; name="t"\r\n\r\nHel');
    const parts = parseMultipart(bytes, "xyz");
    expect(parts).toHaveLength(1);
    expect(parts[0].text).toBe("H"); // the trailing two bytes are taken for the CRLF that never came
    expect(parseMultipart(enc("nothing here"), "xyz")).toEqual([]);
  });

  it("recognises text", () => {
    expect(textOf(enc("plain\ttext\n"))).toBe("plain\ttext\n");
    expect(textOf(new Uint8Array([0x48, 0x00]))).toBeNull();
    expect(textOf(new Uint8Array([0xff, 0xfe]))).toBeNull();
  });
});

describe("GraphQL", () => {
  it("recognises requests, including batches", () => {
    expect(isGraphqlRequest({ query: "query A { a }" })).toBe(true);
    expect(isGraphqlRequest({ query: "{ a }", variables: {} })).toBe(true);
    expect(isGraphqlRequest([{ query: "mutation { x }" }, { query: "{ y }" }])).toBe(true);
    expect(isGraphqlRequest({ query: "SELECT * FROM t" })).toBe(false);
    expect(isGraphqlRequest({ data: { a: 1 } })).toBe(false);
    expect(isGraphqlRequest([])).toBe(false);
  });

  it("parses the request and names the operation", () => {
    const [r] = parseGraphql('{"operationName":"GetUser","query":"query GetUser($id: ID!) { user(id: $id) { name } }","variables":{"id":"1"}}')!;
    expect(r.operationName).toBe("GetUser");
    expect(r.variables).toEqual({ id: "1" });
    expect(graphqlOperation(r.query)).toEqual({ type: "query", name: "GetUser" });
    expect(graphqlOperation("mutation { like }")).toEqual({ type: "mutation", name: null });
    expect(graphqlOperation("# comment\n subscription OnX { x }")).toEqual({ type: "subscription", name: "OnX" });
    expect(graphqlOperation("{ shorthand }")).toEqual({ type: "query", name: null });
    expect(parseGraphql("not json")).toBeNull();
    expect(parseGraphql('{"a":1}')).toBeNull();
  });
});

describe("protobuf", () => {
  it("decodes the protobuf guide's examples", () => {
    // message Test1 { int32 a = 1; } with a = 150
    expect(decodeProto(new Uint8Array([0x08, 0x96, 0x01]))).toEqual({ "1": 150 });
    // message Test2 { string b = 2; } with b = "testing"
    expect(decodeProto(new Uint8Array([0x12, 0x07, 0x74, 0x65, 0x73, 0x74, 0x69, 0x6e, 0x67]))).toEqual({ "2": "testing" });
    // message Test3 { Test1 c = 3; } with c.a = 150
    expect(decodeProto(new Uint8Array([0x1a, 0x03, 0x08, 0x96, 0x01]))).toEqual({ "3": { "1": 150 } });
  });

  it("collects repeated fields and fixed-width values", () => {
    const msg = decodeProto(new Uint8Array([0x08, 0x01, 0x08, 0x02, 0x08, 0x03, 0x15, 0x00, 0x00, 0xc0, 0x3f]));
    expect(msg).toEqual({ "1": [1, 2, 3], "2": 1.5 }); // fixed32 shown as a float when it isn't a whole number
    // A whole-number float is indistinguishable from an integer, so it shows unsigned.
    expect(decodeProto(new Uint8Array([0x15, 0x00, 0x00, 0x80, 0x3f]))).toEqual({ "2": 1065353216 });
    const big = decodeProto(new Uint8Array([0x09, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]));
    expect(big).toEqual({ "1": "18446744073709551615" });
  });

  it("shows unprintable bytes as hex and rejects garbage", () => {
    expect(decodeProto(new Uint8Array([0x0a, 0x02, 0xff, 0xfe]))).toEqual({ "1": "0xfffe" });
    expect(() => decodeProto(new Uint8Array([0x08]))).toThrow();
    expect(() => decodeProto(new Uint8Array([0x0b]))).toThrow(); // wire type 3 (group)
    expect(() => decodeProto(new Uint8Array([0x00, 0x01]))).toThrow(); // field 0
  });

  it("knows protobuf content types", () => {
    for (const ct of ["application/x-protobuf", "application/protobuf; charset=x", "application/grpc", "application/grpc-web+proto", "application/vnd.foo+proto"]) {
      expect(isProtobuf(ct), ct).toBe(true);
    }
    expect(isProtobuf("application/json")).toBe(false);
    expect(isProtobuf(null)).toBe(false);
  });
});

describe("gRPC", () => {
  it("splits frames and reads trailers", () => {
    const frames = parseGrpcFrames(
      new Uint8Array([
        0x00, 0x00, 0x00, 0x00, 0x03, 0x08, 0x96, 0x01, // message {1: 150}
        0x80, 0x00, 0x00, 0x00, 0x12, ...enc("grpc-status: 0\r\n\r\n"), // trailers
      ]),
    );
    expect(frames).toHaveLength(2);
    expect(frames[0]).toMatchObject({ kind: "message", compressed: false, size: 3, message: { "1": 150 } });
    expect(frames[1]).toEqual({ kind: "trailers", headers: [["grpc-status", "0"]] });
  });

  it("marks compressed frames and rejects truncated ones", () => {
    const [f] = parseGrpcFrames(new Uint8Array([0x01, 0x00, 0x00, 0x00, 0x01, 0xaa]));
    expect(f).toMatchObject({ kind: "message", compressed: true, message: null });
    expect(() => parseGrpcFrames(new Uint8Array([0x00, 0x00, 0x00, 0x00, 0x09, 0x01]))).toThrow();
  });

  it("decodes grpc-web-text base64 chunks", () => {
    const b64 = (bytes: number[]) => btoa(String.fromCharCode(...bytes));
    // Servers send each frame as its own base64 chunk, concatenated.
    const bytes = decodeGrpcWebText(
      b64([0x00, 0x00, 0x00, 0x00, 0x03, 0x08, 0x96, 0x01]) + b64([0x80, 0x00, 0x00, 0x00, 0x12, ...enc("grpc-status: 0\r\n\r\n")]),
    );
    const frames = parseGrpcFrames(bytes);
    expect(frames[0]).toMatchObject({ kind: "message", message: { "1": 150 } });
    expect(frames[1]).toMatchObject({ kind: "trailers" });
  });
});

describe("bodyKind", () => {
  it("picks the new kinds from the content type or shape", () => {
    expect(bodyKind(text("a=1"), "application/x-www-form-urlencoded")).toBe("form");
    expect(bodyKind(text("--x\r\n"), "multipart/form-data; boundary=x")).toBe("multipart");
    expect(bodyKind(text("--x\r\n"), "multipart/form-data")).toBe("text"); // no boundary
    expect(bodyKind(bin([8, 1]), "application/x-protobuf")).toBe("protobuf");
    expect(bodyKind(text("AAAA"), "application/grpc-web-text")).toBe("protobuf");
    expect(bodyKind(text('{"query":"{ a }"}'), "application/json")).toBe("graphql");
    expect(bodyKind(text('{"data":{"a":1}}'), "application/json")).toBe("json");
    expect(bodyKind(bin([0, 1]), "application/octet-stream")).toBe("binary");
  });
});
