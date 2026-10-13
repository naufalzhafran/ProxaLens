// Right pane: request on top, response below, each with tabs.
import { useEffect, useMemo, useState } from "react";
import { api, errorMessage, type BodyView, type FlowDetail, type WsMessageView } from "../api";
import { editAndRepeat, repeat } from "../lib/actions";
import { copy } from "../lib/clipboard";
import { formatCurl } from "../lib/curl";
import { formatBytes, formatDuration, formatTime, statusColor } from "../lib/format";
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
import { button, Tabs, useResizable } from "./ui";

export function Inspector() {
  const selectedId = useFlows((s) => s.selectedId);
  // Refetch when the selected flow's summary changes (e.g. response arrives).
  const meta = useFlows((s) => (s.selectedId === null ? null : s.byId[s.selectedId]));
  const [detail, setDetail] = useState<FlowDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const split = useResizable(320, 120, 1200, "y", "inspectorSplit");

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
    return <div className="flex flex-1 items-center justify-center text-neutral-500">Select a flow</div>;
  }
  if (error) return <div className="p-4 text-red-500">{error}</div>;
  if (!detail || detail.meta.id !== selectedId) return null;

  const m = detail.meta;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="selectable flex items-start gap-2 border-b border-neutral-200 px-3 py-2 dark:border-neutral-800">
        <div className="min-w-0 flex-1 break-all font-mono text-xs">
          <span className="font-semibold">{m.method}</span>{" "}
          <span className={statusColor(m.status)}>{m.status ?? (m.state === "failed" ? "ERR" : "…")}</span> {m.url}
          {m.error && <div className="mt-1 text-red-500">{m.error}</div>}
        </div>
        {m.state !== "tunnel" && m.state !== "webSocket" && (
          <>
            <button className={button} title="Send this request again" onClick={() => repeat(m.id)}>
              Repeat
            </button>
            <button className={button} onClick={() => editAndRepeat(m.id)}>
              Edit…
            </button>
          </>
        )}
        <button className={button} onClick={() => copy(m.url, "URL")}>
          Copy URL
        </button>
        <button
          className={button}
          onClick={() => copy(formatCurl(m.method, m.url, detail.requestHeaders, detail.requestBody), "cURL command")}
        >
          Copy cURL
        </button>
      </div>
      <div style={{ height: split.size }} className="flex min-h-0 shrink-0 flex-col">
        <RequestPane detail={detail} />
      </div>
      {split.handle}
      <div className="flex min-h-0 flex-1 flex-col">
        <ResponsePane detail={detail} />
      </div>
    </div>
  );
}

type ReqTab = "headers" | "query" | "cookies" | "body" | "raw";
type ResTab = "headers" | "cookies" | "body" | "raw" | "messages" | "timing";

function RequestPane({ detail }: { detail: FlowDetail }) {
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
      <div className="selectable min-h-0 flex-1 overflow-auto p-2 text-xs">
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
}

function ResponsePane({ detail }: { detail: FlowDetail }) {
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
      <div className="selectable min-h-0 flex-1 overflow-auto p-2 text-xs">
        {m.state === "inProgress" && shown !== "timing" && (
          <p className="mb-2 text-neutral-500">Waiting for the response…</p>
        )}
        {m.state === "tunnel" && (
          <p className="mb-2 text-neutral-500">
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
        {shown === "messages" && <WsMessages messages={detail.wsMessages} />}
        {shown === "timing" && <Timing detail={detail} />}
      </div>
    </>
  );
}

function pathOf(url: string): string {
  try {
    const u = new URL(url);
    return u.pathname + u.search;
  } catch {
    return url;
  }
}

function KeyValues({ rows, empty }: { rows: [string, string][]; empty: string }) {
  if (!rows.length) return <p className="text-neutral-500">{empty}</p>;
  return (
    <table className="w-full font-mono">
      <tbody>
        {rows.map(([k, v], i) => (
          <tr key={i} className="align-top odd:bg-neutral-50 dark:odd:bg-neutral-900">
            <td className="w-1/3 py-0.5 pr-3 pl-1 text-neutral-500">{k}</td>
            <td className="break-all py-0.5">{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Cookies({ cookies }: { cookies: Cookie[] }) {
  if (!cookies.length) return <p className="text-neutral-500">No cookies</p>;
  return (
    <table className="w-full font-mono">
      <tbody>
        {cookies.map((c, i) => (
          <tr key={i} className="align-top odd:bg-neutral-50 dark:odd:bg-neutral-900">
            <td className="w-1/4 py-0.5 pr-3 pl-1 text-neutral-500">{c.name}</td>
            <td className="break-all py-0.5">
              {c.value}
              {c.attrs.length > 0 && (
                <div className="text-neutral-500">
                  {c.attrs.map(([k, v]) => (v ? `${k}=${v}` : k)).join("; ")}
                </div>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

type BodyMode = "tree" | "text" | "hex" | "preview";
const HEX_PAGE = 64 * 1024;

export function BodyViewer({ body, contentType }: { body: BodyView; contentType: string | null }) {
  const kind = useMemo(() => bodyKind(body, contentType), [body, contentType]);
  const modes: BodyMode[] =
    kind === "json" ? ["tree", "text", "hex"] : kind === "image" ? ["preview", "hex"] : kind === "text" ? ["text", "hex"] : ["hex"];
  const [preferred, setPreferred] = useState<BodyMode | null>(null);
  const [hexLimit, setHexLimit] = useState(HEX_PAGE);
  const mode = preferred && modes.includes(preferred) ? preferred : modes[0];
  useEffect(() => setHexLimit(HEX_PAGE), [body]);

  const parsed = useMemo(() => (kind === "json" ? JSON.parse(body.text!) : null), [kind, body.text]);
  const bytes = useMemo(() => (mode === "hex" ? bodyBytes(body) : null), [mode, body]);

  if (kind === "empty") return <p className="text-neutral-500">No body</p>;

  return (
    <div>
      <div className="mb-2 flex items-center gap-2 text-neutral-500">
        <span>
          {formatBytes(body.size)}
          {body.encoding && body.encodedSize != null && ` · ${body.encoding}, ${formatBytes(body.encodedSize)} on the wire`}
          {body.truncated && " (truncated at capture limit)"}
          {kind === "binary" && " · binary"}
          {body.decodeError && <span className="text-amber-600"> · {body.decodeError}; showing raw bytes</span>}
        </span>
        {modes.length > 1 && (
          <div className="ml-auto flex overflow-hidden rounded-md border border-neutral-300 dark:border-neutral-700">
            {modes.map((md) => (
              <button
                key={md}
                onClick={() => setPreferred(md)}
                className={`px-2 py-0.5 capitalize ${md === mode ? "bg-neutral-200 text-neutral-900 dark:bg-neutral-700 dark:text-neutral-100" : ""}`}
              >
                {md}
              </button>
            ))}
          </div>
        )}
      </div>
      {mode === "tree" && <JsonTree value={parsed} />}
      {mode === "text" && <Pre text={kind === "json" ? JSON.stringify(parsed, null, 2) : (body.text ?? "")} />}
      {mode === "preview" && contentType && (
        <img
          alt="Response body"
          src={imageDataUrl(body, contentType)}
          className="max-w-full rounded border border-neutral-200 bg-[repeating-conic-gradient(#e5e5e5_0_25%,transparent_0_50%)] bg-[length:16px_16px] dark:border-neutral-800"
        />
      )}
      {mode === "hex" && bytes && (
        <>
          <Pre text={hexDump(bytes, hexLimit)} />
          {bytes.length > hexLimit && (
            <button className="mt-1 text-sky-600 hover:underline" onClick={() => setHexLimit(hexLimit + HEX_PAGE)}>
              Show more ({formatBytes(bytes.length - hexLimit)} left)
            </button>
          )}
        </>
      )}
    </div>
  );
}

function Pre({ text }: { text: string }) {
  return <pre className="whitespace-pre-wrap break-all font-mono text-xs">{text}</pre>;
}

function WsMessages({ messages }: { messages: WsMessageView[] }) {
  const [open, setOpen] = useState<number | null>(null);
  if (!messages.length) return <p className="text-neutral-500">No messages yet</p>;
  return (
    <ul className="font-mono">
      {messages.map((w, i) => (
        <li key={i} className="border-b border-neutral-100 dark:border-neutral-900">
          <div className="flex cursor-default gap-2 py-0.5" onClick={() => setOpen(open === i ? null : i)}>
            <span className={w.direction === "clientToServer" ? "text-sky-600" : "text-emerald-600"}>
              {w.direction === "clientToServer" ? "↑" : "↓"}
            </span>
            <span className="w-28 shrink-0 text-neutral-500">{formatTime(w.at)}</span>
            <span className="w-12 shrink-0 text-neutral-500">{w.kind}</span>
            <span className="min-w-0 flex-1 truncate">{w.payload.text ?? `${formatBytes(w.size)} binary`}</span>
          </div>
          {open === i && (
            <div className="py-1 pl-6">
              <BodyViewer body={w.payload} contentType={null} />
            </div>
          )}
        </li>
      ))}
    </ul>
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
            <dt className="text-neutral-500">{k}</dt>
            <dd className="font-mono">{v}</dd>
          </div>
        ))}
      </dl>
      {total > 0 && (
        <div className="mt-3 flex h-2 w-full overflow-hidden rounded bg-neutral-200 dark:bg-neutral-800" title="Waiting · Download">
          <div className="bg-amber-400" style={{ width: `${(ttfb / total) * 100}%` }} />
          <div className="bg-sky-500" style={{ width: `${((total - ttfb) / total) * 100}%` }} />
        </div>
      )}
      <p className="mt-2 text-neutral-500">DNS, connect and TLS timings aren’t captured yet.</p>
    </div>
  );
}
