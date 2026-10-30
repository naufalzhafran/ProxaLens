// Body views for forms, multipart uploads, GraphQL requests and protobuf
// or gRPC messages. Each takes the decoded body the inspector shows.
import { useMemo } from "react";
import type { BodyView } from "../api";
import {
  decodeGrpcWebText,
  decodeProto,
  graphqlOperation,
  isGrpc,
  multipartBoundary,
  parseForm,
  parseGraphql,
  parseGrpcFrames,
  parseMultipart,
  type GraphqlRequest,
  type GrpcFrame,
  type Json,
  type Part,
} from "../lib/decoders";
import { formatBytes } from "../lib/format";
import { bodyBytes, hexDump } from "../lib/http";
import { JsonTree } from "./JsonTree";
import { KeyValues, Pre } from "./KeyValues";
import { HUE_BADGE } from "./ui";

const PART_TEXT_LIMIT = 20_000;

export function FormView({ text }: { text: string }) {
  const rows = useMemo(() => parseForm(text), [text]);
  return <KeyValues rows={rows} empty="No form fields" />;
}

export function MultipartView({ body, contentType }: { body: BodyView; contentType: string | null }) {
  const parts = useMemo(() => {
    const boundary = multipartBoundary(contentType);
    return boundary ? parseMultipart(bodyBytes(body), boundary) : [];
  }, [body, contentType]);
  if (!parts.length) return <p className="text-muted">No parts found; the boundary may be wrong or the body truncated.</p>;
  return (
    <div className="flex flex-col gap-2">
      {parts.map((p, i) => (
        <PartView key={i} part={p} />
      ))}
    </div>
  );
}

function PartView({ part }: { part: Part }) {
  const isImage = part.contentType?.toLowerCase().startsWith("image/");
  const imageUrl = useMemo(() => {
    if (!isImage) return null;
    let bin = "";
    for (const b of part.body) bin += String.fromCharCode(b);
    return `data:${part.contentType!.split(";")[0].trim()};base64,${btoa(bin)}`;
  }, [isImage, part]);
  return (
    <div className="rounded-ds border border-line bg-panel">
      <div className="flex flex-wrap items-baseline gap-x-2 border-b border-line px-2 py-1">
        <span className="font-mono font-semibold">{part.name ?? "(unnamed)"}</span>
        {part.filename && <span className="font-mono text-muted">{part.filename}</span>}
        {part.contentType && <span className="text-muted">{part.contentType}</span>}
        <span className="ml-auto text-muted">{formatBytes(part.body.length)}</span>
      </div>
      <div className="p-2">
        {imageUrl ? (
          <img alt={part.filename ?? part.name ?? "part"} src={imageUrl} className="max-h-64 max-w-full rounded border border-line" />
        ) : part.text !== null ? (
          <Pre text={part.text.length > PART_TEXT_LIMIT ? `${part.text.slice(0, PART_TEXT_LIMIT)}\n… (${formatBytes(part.body.length)} in all)` : part.text} />
        ) : part.body.length ? (
          <Pre text={hexDump(part.body, 512) + (part.body.length > 512 ? "\n…" : "")} />
        ) : (
          <p className="text-muted">Empty</p>
        )}
      </div>
    </div>
  );
}

export function GraphqlView({ text }: { text: string }) {
  const requests = useMemo(() => parseGraphql(text), [text]);
  if (!requests) return <p className="text-muted">Not a GraphQL request</p>;
  return (
    <div className="flex flex-col gap-3">
      {requests.map((r, i) => (
        <GraphqlRequestView key={i} request={r} count={requests.length} index={i} />
      ))}
    </div>
  );
}

const OP_HUE = { query: "blue", mutation: "green", subscription: "violet" } as const;

function GraphqlRequestView({ request, count, index }: { request: GraphqlRequest; count: number; index: number }) {
  const op = graphqlOperation(request.query);
  const name = request.operationName ?? op.name;
  const hasVariables = isNonEmpty(request.variables);
  return (
    <div>
      <div className="mb-1 flex flex-wrap items-center gap-1.5">
        {count > 1 && <span className="text-muted">#{index + 1}</span>}
        <span className={`rounded px-1 text-[11px] font-semibold ${HUE_BADGE[OP_HUE[op.type]]}`}>{op.type}</span>
        <span className="font-mono font-semibold">{name ?? <span className="font-normal text-muted">anonymous</span>}</span>
      </div>
      <Pre text={request.query.trim()} />
      {hasVariables && (
        <>
          <p className="mt-2 mb-1 text-muted">Variables</p>
          <JsonTree value={request.variables as Json} />
        </>
      )}
      {isNonEmpty(request.extensions) && (
        <>
          <p className="mt-2 mb-1 text-muted">Extensions</p>
          <JsonTree value={request.extensions as Json} />
        </>
      )}
    </div>
  );
}

function isNonEmpty(v: Json | undefined): boolean {
  if (v === undefined || v === null) return false;
  if (Array.isArray(v)) return v.length > 0;
  if (typeof v === "object") return Object.keys(v).length > 0;
  return true;
}

export function ProtoView({ body, contentType }: { body: BodyView; contentType: string | null }) {
  const decoded = useMemo(() => decodeBody(body, contentType), [body, contentType]);
  if ("error" in decoded) {
    return (
      <p className="text-muted">
        Could not decode as protobuf: {decoded.error}. {body.truncated ? "The body was truncated at the capture limit." : ""}
      </p>
    );
  }
  if ("message" in decoded) {
    return (
      <>
        <p className="mb-1 text-muted">Decoded without a schema: keys are field numbers, and numbers are shown unsigned.</p>
        <JsonTree value={decoded.message} />
      </>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      <p className="text-muted">
        {decoded.frames.length} gRPC frame{decoded.frames.length === 1 ? "" : "s"}. Decoded without a schema: keys are field
        numbers.
      </p>
      {decoded.frames.map((f, i) => (
        <FrameView key={i} frame={f} index={i} />
      ))}
    </div>
  );
}

function FrameView({ frame, index }: { frame: GrpcFrame; index: number }) {
  if (frame.kind === "trailers") {
    return (
      <div className="rounded-ds border border-line bg-panel">
        <div className="border-b border-line px-2 py-1 text-muted">Trailers</div>
        <div className="p-1">
          <KeyValues rows={frame.headers} empty="No trailers" />
        </div>
      </div>
    );
  }
  return (
    <div className="rounded-ds border border-line bg-panel">
      <div className="flex gap-2 border-b border-line px-2 py-1">
        <span>Message {index + 1}</span>
        <span className="ml-auto text-muted">
          {formatBytes(frame.size)}
          {frame.compressed && " · compressed"}
        </span>
      </div>
      <div className="p-2">
        {frame.compressed ? (
          <p className="text-muted">Compressed with the connection’s grpc-encoding; shown as bytes.</p>
        ) : frame.message ? (
          <JsonTree value={frame.message} />
        ) : (
          <p className="mb-1 text-muted">Not a valid protobuf message; raw bytes:</p>
        )}
        {(frame.compressed || !frame.message) && <Pre text={hexDump(frame.raw, 512)} />}
      </div>
    </div>
  );
}

type Decoded = { message: Json } | { frames: GrpcFrame[] } | { error: string };

function decodeBody(body: BodyView, contentType: string | null): Decoded {
  try {
    const ct = (contentType ?? "").toLowerCase();
    const isText = /^application\/grpc-web-text/.test(ct);
    const bytes = isText && body.text !== null ? decodeGrpcWebText(body.text) : bodyBytes(body);
    if (isGrpc(ct)) return { frames: parseGrpcFrames(bytes) };
    return { message: decodeProto(bytes) };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}
