// Right pane: request on top, response below, each with tabs.
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronUp, Copy as CopyIcon } from "lucide-react";
import { api, errorMessage, type BodyView, type FlowDetail, type WsMessageView } from "../api";
import { editAndRepeat, repeat } from "../lib/actions";
import { copy } from "../lib/clipboard";
import { formatCurl } from "../lib/curl";
import { formatBytes, formatDuration, formatTime, methodHue, statusColor } from "../lib/format";
import { applyMatches, findMatches, langFor, MATCH_LIMIT, tokenize, type Lang } from "../lib/highlight";
import {
  bodyBytes,
  bodyKind,
  headerValue,
  hexDump,
  imageDataUrl,
  queryParams,
  rawRequest,
  rawResponse,
  requestCookies,
  responseCookies,
  type Cookie,
} from "../lib/http";
import { useFlows } from "../state/flows";
import { JsonTree } from "./JsonTree";
import { button, Chip, ContextMenu, HUE_BADGE, input, Tabs, useResizable } from "./ui";

const pill = `${button} rounded-full! px-3`;

/** `sideBySide` puts request and response next to each other, for a wide, short pane. */
export const Inspector = memo(function Inspector({ sideBySide = false }: { sideBySide?: boolean }) {
  const selectedId = useFlows((s) => s.selectedId);
  // Refetch when the selected flow's summary changes (e.g. response arrives).
  const meta = useFlows((s) => (s.selectedId === null ? null : s.byId[s.selectedId]));
  const [detail, setDetail] = useState<FlowDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const split = useResizable(320, 120, 1200, "y", "inspectorSplit");
  const splitX = useResizable(480, 200, 2000, "x", "inspectorSplitX");

  useEffect(() => {
    if (selectedId === null) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    api
      .getFlow(selectedId)
      .then((d) => !cancelled && (setDetail(d), setError(null)))
      .catch((e) => !cancelled && setError(errorMessage(e)));
    return () => {
      cancelled = true;
    };
  }, [selectedId, meta]);

  if (selectedId === null) {
    return <EmptyState />;
  }
  if (error) return <div className="p-4 text-st-server">{error}</div>;
  if (!detail || detail.meta.id !== selectedId) return null;

  const m = detail.meta;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-line bg-panel px-3 py-2.5">
        <div className="selectable font-mono text-xs">
          <div className="line-clamp-3 break-all" title={m.url}>
            <span className={`mr-1 rounded px-1 py-px text-[11px] font-semibold ${HUE_BADGE[methodHue(m.method)]}`}>{m.method}</span>{" "}
            <span className={statusColor(m.status)}>{m.status ?? (m.state === "failed" ? "ERR" : m.state === "paused" ? "paused" : "…")}</span> {m.url}
          </div>
          {m.error && <div className="mt-1 text-st-server">{m.error}</div>}
          {m.tags.length > 0 && (
            <div className="mt-1 flex flex-wrap gap-1">
              {m.tags.map((t) => (
                <span key={t} className="rounded-full bg-accent/14 px-2 text-[11px] font-medium text-accent">
                  {t}
                </span>
              ))}
            </div>
          )}
        </div>
        <div className="mt-2.5 flex flex-wrap gap-1.5">
          {m.state !== "tunnel" && m.state !== "webSocket" && (
            <>
              <button className={pill} title="Send this request again (⌘R)" onClick={() => repeat(m.id)}>
                Repeat
              </button>
              <button className={pill} onClick={() => editAndRepeat(m.id)}>
                Edit and Repeat…
              </button>
            </>
          )}
          <button className={pill} onClick={() => copy(m.url, "URL")}>
            Copy URL
          </button>
          <button
            className={pill}
            onClick={() => copy(formatCurl(m.method, m.url, detail.requestHeaders, detail.requestBody), "cURL command")}
          >
            Copy cURL
          </button>
        </div>
      </div>
      {sideBySide ? (
        <div className="flex min-h-0 flex-1">
          <div style={{ width: splitX.size }} className="flex min-w-0 shrink-0 flex-col">
            <RequestPane detail={detail} />
          </div>
          {splitX.handle}
          <div style={{ minWidth: 200 }} className="flex min-w-0 flex-1 flex-col">
            <ResponsePane detail={detail} />
          </div>
        </div>
      ) : (
        <>
          <div style={{ height: split.size }} className="flex min-h-0 shrink-0 flex-col">
            <RequestPane detail={detail} />
          </div>
          {split.handle}
          <div className="flex min-h-0 flex-1 flex-col">
            <ResponsePane detail={detail} />
          </div>
        </>
      )}
    </div>
  );
});

type ReqTab = "headers" | "query" | "cookies" | "body" | "raw";
type ResTab = "headers" | "cookies" | "body" | "raw" | "messages" | "timing";

const RequestPane = memo(function RequestPane({ detail }: { detail: FlowDetail }) {
  const [tab, setTab] = useState<ReqTab>("headers");
  const m = detail.meta;
  const query = useMemo(() => queryParams(m.url), [m.url]);
  const cookies = useMemo(() => requestCookies(detail.requestHeaders), [detail.requestHeaders]);
  const ct = headerValue(detail.requestHeaders, "content-type");

  return (
    <>
      <Tabs<ReqTab>
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "headers", label: "Request", badge: detail.requestHeaders.length },
          { id: "query", label: "Query", badge: query.length },
          { id: "cookies", label: "Cookies", badge: cookies.length },
          { id: "body", label: "Body", badge: detail.requestBody.size ? undefined : 0 },
          { id: "raw", label: "Raw" },
        ]}
      />
      <div data-find-scope tabIndex={-1} className="selectable min-h-0 flex-1 overflow-auto p-2 text-xs outline-none">
        {tab === "headers" && <KeyValues rows={detail.requestHeaders} empty="No headers" />}
        {tab === "query" && <KeyValues rows={query} empty="No query parameters" />}
        {tab === "cookies" && <Cookies cookies={cookies} />}
        {tab === "body" && <BodyViewer body={detail.requestBody} contentType={ct} />}
        {tab === "raw" && (
          <Pre text={rawRequest(m.method, pathOf(m.url), m.httpVersion, detail.requestHeaders, detail.requestBody)} />
        )}
      </div>
    </>
  );
});

const ResponsePane = memo(function ResponsePane({ detail }: { detail: FlowDetail }) {
  const [tab, setTab] = useState<ResTab>("body");
  const m = detail.meta;
  const cookies = useMemo(() => responseCookies(detail.responseHeaders), [detail.responseHeaders]);
  const ws = detail.wsMessages.length > 0 || m.state === "webSocket";
  const shown: ResTab = tab === "messages" && !ws ? "body" : tab;

  return (
    <>
      <Tabs<ResTab>
        value={shown}
        onChange={setTab}
        tabs={[
          { id: "headers", label: "Response", badge: detail.responseHeaders.length },
          { id: "cookies", label: "Cookies", badge: cookies.length },
          { id: "body", label: "Body" },
          { id: "raw", label: "Raw" },
          ...(ws ? [{ id: "messages" as const, label: "Messages", badge: m.wsMessageCount }] : []),
          { id: "timing", label: "Timing" },
        ]}
      />
      <div data-find-scope tabIndex={-1} className="selectable min-h-0 flex-1 overflow-auto p-2 text-xs outline-none">
        {m.state === "inProgress" && shown !== "timing" && (
          <p className="mb-2 text-muted">Waiting for the response…</p>
        )}
        {m.state === "tunnel" && (
          <p className="mb-2 text-muted">
            This connection was passed through without decryption, so its contents can’t be shown. Change this under
            Settings → SSL Proxying.
          </p>
        )}
        {shown === "headers" && <KeyValues rows={detail.responseHeaders} empty="No headers" />}
        {shown === "cookies" && <Cookies cookies={cookies} />}
        {shown === "body" && <BodyViewer body={detail.responseBody} contentType={m.contentType} />}
        {shown === "raw" && (
          <Pre text={rawResponse(m.status, m.httpVersion, detail.responseHeaders, detail.responseBody)} />
        )}
        {shown === "messages" && <WsMessages messages={detail.wsMessages} total={m.wsMessageCount} />}
        {shown === "timing" && <Timing detail={detail} />}
      </div>
    </>
  );
});

function pathOf(url: string): string {
  try {
    const u = new URL(url);
    return u.pathname + u.search;
  } catch {
    return url;
  }
}

function KeyValues({ rows, empty }: { rows: [string, string][]; empty: string }) {
  if (!rows.length) return <p className="text-muted">{empty}</p>;
  return (
    <CopyRows
      rows={rows.map(([k, v]) => ({
        name: k,
        value: v,
        line: `${k}: ${v}`,
        cell: <span className="break-all">{v}</span>,
      }))}
      lineLabel="Copy as “Name: value”"
    />
  );
}

function Cookies({ cookies }: { cookies: Cookie[] }) {
  if (!cookies.length) return <p className="text-muted">No cookies</p>;
  return (
    <CopyRows
      rows={cookies.map((c) => ({
        name: c.name,
        value: c.value,
        line: `${c.name}=${c.value}`,
        cell: (
          <span className="break-all">
            {c.value}
            {c.attrs.length > 0 && (
              <span className="block text-muted">{c.attrs.map(([k, v]) => (v ? `${k}=${v}` : k)).join("; ")}</span>
            )}
          </span>
        ),
      }))}
      lineLabel="Copy as “name=value”"
    />
  );
}

interface CopyRow {
  name: string;
  value: string;
  line: string;
  cell: React.ReactNode;
}

/** Name/value rows with a copy button on hover and a copy context menu. */
function CopyRows({ rows, lineLabel }: { rows: CopyRow[]; lineLabel: string }) {
  const [menu, setMenu] = useState<{ x: number; y: number; row: CopyRow } | null>(null);
  return (
    <>
      <table className="w-full font-mono">
        <tbody>
          {rows.map((r, i) => (
            <tr
              key={i}
              className="group border-b border-line/70 align-top last:border-0"
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ x: e.clientX, y: e.clientY, row: r });
              }}
            >
              <td className="w-1/3 py-1 pr-3 pl-1 break-all text-muted">{r.name}</td>
              <td className="py-1">{r.cell}</td>
              <td className="w-6 py-1 text-right">
                <button
                  aria-label={`Copy ${r.name}`}
                  title="Copy value"
                  className="invisible rounded p-0.5 text-muted group-hover:visible hover:text-ink focus-visible:visible"
                  onClick={() => copy(r.value, r.name)}
                >
                  <CopyIcon size={12} />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "Copy Value", onSelect: () => copy(menu.row.value, menu.row.name) },
            { label: "Copy Name", onSelect: () => copy(menu.row.name, "name") },
            { label: lineLabel, onSelect: () => copy(menu.row.line, menu.row.name) },
          ]}
        />
      )}
    </>
  );
}

const SHORTCUTS: [string, string][] = [
  ["⌘F", "Filter flows, or find in the body you clicked"],
  ["↑ ↓", "Move through flows; ⇧ extends the selection"],
  ["⌘-click, ⇧-click", "Select several flows"],
  ["⌘R / ⌘N", "Repeat the selected flow / compose a request"],
  ["⌫", "Remove the selected flows"],
  ["⌘K", "Clear all flows"],
  ["⌘L / ⌘J", "Rules / scripts"],
  ["⌘O / ⌘S", "Open / save a session"],
  ["⌘,", "Settings"],
];

function EmptyState() {
  return (
    <div className="flex flex-1 items-center justify-center p-6 text-muted">
      <div className="max-w-xs">
        <p className="mb-3 text-sm text-ink">Select a flow to see its request and response.</p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
          {SHORTCUTS.map(([keys, what]) => (
            <div key={keys} className="contents">
              <dt className="font-mono text-ink">{keys}</dt>
              <dd>{what}</dd>
            </div>
          ))}
        </dl>
        <p className="mt-3">Right-click a flow for copy, repeat, rules and more.</p>
      </div>
    </div>
  );
}

type BodyMode = "protobuf" | "tree" | "text" | "hex" | "preview";
const HEX_PAGE = 64 * 1024;

export function BodyViewer({ body, contentType }: { body: BodyView; contentType: string | null }) {
  const kind = useMemo(() => bodyKind(body, contentType), [body, contentType]);
  const base: BodyMode[] =
    kind === "json" ? ["tree", "text", "hex"] : kind === "image" ? ["preview", "hex"] : kind === "text" ? ["text", "hex"] : ["hex"];
  const modes: BodyMode[] = body.protobuf ? ["protobuf", ...base.filter((m) => m !== "tree")] : base;
  const [preferred, setPreferred] = useState<BodyMode | null>(null);
  const [hexLimit, setHexLimit] = useState(HEX_PAGE);
  const mode = preferred && modes.includes(preferred) ? preferred : modes[0];
  useEffect(() => setHexLimit(HEX_PAGE), [body]);

  const parsed = useMemo(() => (kind === "json" ? JSON.parse(body.text!) : null), [kind, body.text]);
  const proto = useMemo(() => (body.protobuf ? JSON.parse(body.protobuf.json) : null), [body.protobuf]);
  const bytes = useMemo(() => (mode === "hex" ? bodyBytes(body) : null), [mode, body]);

  // Find works on text; searching a tree shows it as text so every match is visible.
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const searchable = kind === "json" || kind === "text" || !!body.protobuf;
  const shown: BodyMode = query && (mode === "tree" || mode === "protobuf") ? "text" : mode;
  const text = useMemo(() => {
    if (shown !== "text") return "";
    if (mode === "protobuf") return JSON.stringify(proto, null, 2);
    return kind === "json" ? JSON.stringify(parsed, null, 2) : (body.text ?? "");
  }, [shown, mode, kind, parsed, proto, body.text]);
  const lang: Lang = langFor(contentType, kind === "json" || mode === "protobuf");
  const ranges = useMemo(() => findMatches(text, query), [text, query]);
  useEffect(() => setCurrent(0), [query, body]);
  const step = (by: number) => {
    if (ranges.length) setCurrent((c) => (c + by + ranges.length) % ranges.length);
  };

  if (kind === "empty") return <p className="text-muted">No body</p>;

  return (
    <div>
      <div className="sticky -top-2 z-[1] -mx-2 mb-1 flex flex-wrap items-center gap-2 bg-surface px-2 py-1 text-muted">
        <span className="min-w-0 flex-1">
          {formatBytes(body.size)}
          {body.encoding && body.encodedSize != null && ` · ${body.encoding}, ${formatBytes(body.encodedSize)} on the wire`}
          {body.truncated && " (truncated at capture limit)"}
          {kind === "binary" && " · binary"}
          {body.decodeError && <span className="text-st-client"> · {body.decodeError}; showing raw bytes</span>}
        </span>
        {searchable && (
          <div className="flex items-center gap-0.5">
            <input
              data-find
              aria-label="Find in body"
              placeholder="Find"
              spellCheck={false}
              className={`${input} w-32 font-sans`}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") step(e.shiftKey ? -1 : 1);
                if (e.key === "Escape") setQuery("");
              }}
            />
            {query && (
              <span className="w-12 text-right tabular-nums" aria-live="polite">
                {ranges.length ? `${current + 1}/${ranges.length}${ranges.length >= MATCH_LIMIT ? "+" : ""}` : "0/0"}
              </span>
            )}
            <button
              aria-label="Previous match (⇧↩)"
              title="Previous match (⇧↩)"
              disabled={!ranges.length}
              className="rounded p-0.5 disabled:opacity-40"
              onClick={() => step(-1)}
            >
              <ChevronUp size={14} />
            </button>
            <button
              aria-label="Next match (↩)"
              title="Next match (↩)"
              disabled={!ranges.length}
              className="rounded p-0.5 disabled:opacity-40"
              onClick={() => step(1)}
            >
              <ChevronDown size={14} />
            </button>
          </div>
        )}
        {modes.length > 1 && (
          <div className="flex rounded-full bg-ink/6 p-0.5">
            {modes.map((md) => (
              <button
                key={md}
                onClick={() => setPreferred(md)}
                className={`rounded-full px-2.5 py-0.5 capitalize transition-colors ${md === shown ? "bg-panel text-ink shadow-thumb" : "hover:text-ink"}`}
              >
                {md}
              </button>
            ))}
          </div>
        )}
      </div>
      {shown === "protobuf" && body.protobuf && (
        <>
          <p className="mb-1 text-muted">
            {body.protobuf.message
              ? `Decoded as ${body.protobuf.message}`
              : "Decoded without a schema (field numbers as keys). Add .proto files under Settings → Protobuf for names."}
          </p>
          {body.protobuf.notes.map((n, i) => (
            <p key={i} className="mb-1 text-st-client">
              {n}
            </p>
          ))}
          <JsonTree value={proto} />
        </>
      )}
      {shown === "tree" && <JsonTree value={parsed} />}
      {shown === "text" && <Code text={text} lang={lang} ranges={ranges} current={current} />}
      {mode === "preview" && contentType && (
        <img
          alt="Response body"
          src={imageDataUrl(body, contentType)}
          className="max-w-full rounded border border-line bg-[repeating-conic-gradient(#e5e5e5_0_25%,transparent_0_50%)] bg-[length:16px_16px]"
        />
      )}
      {mode === "hex" && bytes && (
        <>
          <Pre text={hexDump(bytes, hexLimit)} />
          {bytes.length > hexLimit && (
            <button className="mt-1 text-accent hover:underline" onClick={() => setHexLimit(hexLimit + HEX_PAGE)}>
              Show more ({formatBytes(bytes.length - hexLimit)} left)
            </button>
          )}
        </>
      )}
    </div>
  );
}

const TOKEN_CLASS: Record<string, string> = {
  key: "text-syn-key",
  string: "text-syn-string",
  number: "text-syn-number",
  literal: "text-syn-literal",
  tag: "text-syn-key",
  attr: "text-syn-number",
  comment: "italic text-muted",
};

/** Highlighted text with find matches; the current match is scrolled into view. */
function Code({ text, lang, ranges, current }: { text: string; lang: Lang; ranges: [number, number][]; current: number }) {
  const ref = useRef<HTMLPreElement>(null);
  const tokens = useMemo(() => tokenize(text, lang), [text, lang]);
  const segments = useMemo(() => applyMatches(tokens, ranges), [tokens, ranges]);
  useEffect(() => {
    ref.current?.querySelector(`[data-match="${current}"]`)?.scrollIntoView({ block: "nearest" });
  }, [current, segments]);
  return (
    <pre ref={ref} className="whitespace-pre-wrap break-all font-mono text-xs">
      {segments.map((seg, i) => {
        const cls = seg.token ? TOKEN_CLASS[seg.token] : undefined;
        if (seg.match === undefined) {
          return cls ? (
            <span key={i} className={cls}>
              {seg.text}
            </span>
          ) : (
            seg.text
          );
        }
        return (
          <mark
            key={i}
            data-match={seg.match}
            className={`rounded-sm ${seg.match === current ? "bg-accent text-on-accent" : "bg-select"} ${cls ?? ""}`}
          >
            {seg.text}
          </mark>
        );
      })}
    </pre>
  );
}

function Pre({ text }: { text: string }) {
  return <pre className="whitespace-pre-wrap break-all font-mono text-xs">{text}</pre>;
}

type WsFilter = "all" | "clientToServer" | "serverToClient";

/** Messages after the direction filter and search; exported for tests. */
export function filterWs(messages: WsMessageView[], dir: WsFilter, query: string, controlFrames: boolean): number[] {
  const q = query.trim().toLowerCase();
  const out: number[] = [];
  messages.forEach((m, i) => {
    if (dir !== "all" && m.direction !== dir) return;
    if (!controlFrames && (m.kind === "ping" || m.kind === "pong")) return;
    if (q && !(m.payload.text ?? "").toLowerCase().includes(q)) return;
    out.push(i);
  });
  return out;
}

function WsMessages({ messages, total }: { messages: WsMessageView[]; total: number }) {
  const [dir, setDir] = useState<WsFilter>("all");
  const [query, setQuery] = useState("");
  const [control, setControl] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  const shown = useMemo(() => filterWs(messages, dir, query, control), [messages, dir, query, control]);
  if (!messages.length) return <p className="text-muted">No messages yet</p>;
  const first = messages[0].at;
  const current = selected !== null ? messages[selected] : null;

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-1">
        {(["all", "clientToServer", "serverToClient"] as const).map((d) => (
          <Chip key={d} active={dir === d} onClick={() => setDir(d)}>
            {{ all: "All", clientToServer: "↑ Sent", serverToClient: "↓ Received" }[d]}
          </Chip>
        ))}
        <label className="ml-1 flex items-center gap-1 text-muted">
          <input type="checkbox" checked={control} onChange={(e) => setControl(e.target.checked)} />
          ping/pong
        </label>
        <input
          className="ml-auto w-40 rounded-ds bg-ink/6 px-2 py-0.5 placeholder:text-muted"
          placeholder="Search messages"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="text-muted">
          {shown.length} of {messages.length}
          {total > messages.length && ` (first ${messages.length} of ${total} kept)`}
        </span>
      </div>
      <div className="min-h-[120px] flex-1 overflow-auto rounded-ds border border-line bg-panel font-mono">
        {shown.map((i) => {
          const w = messages[i];
          const sel = i === selected;
          return (
            <div
              key={i}
              onClick={() => setSelected(sel ? null : i)}
              className={`flex cursor-default gap-2 border-b border-line px-1 py-0.5 ${
                sel ? "bg-select text-ink" : ""
              }`}
            >
              <span className={sel ? "" : w.direction === "clientToServer" ? "text-accent" : "text-st-ok"}>
                {w.direction === "clientToServer" ? "↑" : "↓"}
              </span>
              <span className={`w-20 shrink-0 text-right ${sel ? "" : "text-muted"}`}>+{formatDuration(w.at - first) || "0 ms"}</span>
              <span className={`w-12 shrink-0 ${sel ? "" : "text-muted"}`}>{w.kind}</span>
              <span className={`w-16 shrink-0 text-right ${sel ? "" : "text-muted"}`}>{formatBytes(w.size)}</span>
              <span className="min-w-0 flex-1 truncate">{w.payload.text ?? "binary"}</span>
            </div>
          );
        })}
      </div>
      {current && (
        <div className="max-h-[50%] overflow-auto rounded-ds border border-line bg-panel p-2">
          <p className="mb-1 text-muted">
            {current.direction === "clientToServer" ? "Sent" : "Received"} at {formatTime(current.at)} · {current.kind}
          </p>
          <BodyViewer body={current.payload} contentType={null} />
        </div>
      )}
    </div>
  );
}

function Timing({ detail }: { detail: FlowDetail }) {
  const m = detail.meta;
  const rows: [string, React.ReactNode][] = [
    ["Started", `${new Date(m.startedAt).toLocaleDateString()} ${formatTime(m.startedAt)}`],
    ["Time to headers", formatDuration(m.ttfbMs) || "—"],
    ["Total time", formatDuration(m.durationMs) || "—"],
    ["Request size", formatBytes(m.requestSize)],
    ["Response size", formatBytes(m.responseSize)],
    ["Protocol", m.httpVersion],
    ["Client", m.clientAddr],
  ];
  const total = m.durationMs ?? 0;
  const ttfb = m.ttfbMs ?? 0;
  return (
    <div>
      <dl className="grid grid-cols-[120px_1fr] gap-y-0.5">
        {rows.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-muted">{k}</dt>
            <dd className="font-mono">{v}</dd>
          </div>
        ))}
      </dl>
      {total > 0 && (
        <div className="mt-3 flex h-2 w-full overflow-hidden rounded bg-line" title="Waiting · Download">
          <div className="bg-st-client" style={{ width: `${(ttfb / total) * 100}%` }} />
          <div className="bg-accent" style={{ width: `${((total - ttfb) / total) * 100}%` }} />
        </div>
      )}
      <p className="mt-2 text-muted">DNS, connect and TLS timings aren’t captured yet.</p>
    </div>
  );
}
