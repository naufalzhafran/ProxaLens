import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, type FlowMeta } from "../api";
import { compareWith, editAndRepeat, files, repeat } from "../lib/actions";
import { copy } from "../lib/clipboard";
import { formatCurl } from "../lib/curl";
import { buildPredicate } from "../lib/filter";
import { baseDomain } from "../lib/http";
import { formatBytes, formatDuration, formatTime, shortContentType, statusColor } from "../lib/format";
import { useFlows } from "../state/flows";
import { useUi } from "../state/ui";
import { ContextMenu, type MenuEntry } from "./ui";

const ROW_HEIGHT = 24;
const COLUMNS =
  "grid-cols-[48px_60px_48px_minmax(110px,1fr)_minmax(150px,2fr)_64px_64px_64px_92px]";

export function FlowTable() {
  const order = useFlows((s) => s.order);
  const byId = useFlows((s) => s.byId);
  const filter = useFlows((s) => s.filter);
  const methods = useFlows((s) => s.methods);
  const statuses = useFlows((s) => s.statuses);
  const scope = useFlows((s) => s.scope);
  const selectedId = useFlows((s) => s.selectedId);
  const select = useFlows((s) => s.select);
  const [menu, setMenu] = useState<{ x: number; y: number; flow: FlowMeta } | null>(null);

  const predicate = useMemo(
    () => buildPredicate({ text: filter, methods, statuses, scope }),
    [filter, methods, statuses, scope],
  );
  const rows = useMemo(() => order.filter((id) => predicate(byId[id])), [order, byId, predicate]);

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  // Follow new traffic when scrolled to the bottom.
  const atBottom = useRef(true);
  useEffect(() => {
    if (atBottom.current && rows.length) virtualizer.scrollToIndex(rows.length - 1);
  }, [rows.length, virtualizer]);

  function onKeyDown(e: React.KeyboardEvent) {
    const moves: Record<string, (i: number) => number> = {
      ArrowDown: (i) => i + 1,
      ArrowUp: (i) => i - 1,
      Home: () => 0,
      End: () => rows.length - 1,
    };
    const move = moves[e.key];
    if (!move) return;
    e.preventDefault();
    const i = selectedId === null ? -1 : rows.indexOf(selectedId);
    const next = Math.min(rows.length - 1, Math.max(0, move(i)));
    if (rows[next] !== undefined) {
      select(rows[next]);
      virtualizer.scrollToIndex(next);
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        className={`grid ${COLUMNS} gap-2 border-b border-neutral-200 px-3 py-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-500 dark:border-neutral-800`}
      >
        <span>#</span>
        <span>Method</span>
        <span>Status</span>
        <span>Host</span>
        <span>Path</span>
        <span>Type</span>
        <span className="text-right">Size</span>
        <span className="text-right">Time</span>
        <span>Started</span>
      </div>
      <div
        ref={parentRef}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < ROW_HEIGHT * 2;
        }}
        className="min-h-0 flex-1 overflow-auto outline-none"
      >
        {rows.length === 0 ? (
          <p className="p-6 text-center text-neutral-500">
            {order.length
              ? "No flows match the current filter."
              : "No traffic yet. Point an app at the proxy, or switch the proxy on."}
          </p>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((v) => {
              const f = byId[rows[v.index]];
              const selected = f.id === selectedId;
              return (
                <div
                  key={f.id}
                  onClick={() => select(f.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    select(f.id);
                    setMenu({ x: e.clientX, y: e.clientY, flow: f });
                  }}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    right: 0,
                    height: ROW_HEIGHT,
                    transform: `translateY(${v.start}px)`,
                  }}
                  className={`grid ${COLUMNS} cursor-default items-center gap-2 px-3 font-mono text-xs ${rowClass(
                    f,
                    selected,
                    v.index,
                  )}`}
                >
                  <span className={selected ? "" : "text-neutral-400"}>{f.id}</span>
                  <span className="font-semibold">{f.method}</span>
                  <span className={selected ? "" : statusColor(f.status)}>{statusLabel(f)}</span>
                  <span className="truncate">{f.host}</span>
                  <span className="truncate" title={f.url}>
                    {f.path}
                  </span>
                  <span className={`truncate ${selected ? "" : "text-neutral-500"}`}>
                    {f.state === "webSocket" ? "ws" : f.state === "tunnel" ? "tunnel" : shortContentType(f.contentType)}
                  </span>
                  <span className="text-right">{f.state === "completed" ? formatBytes(f.responseSize) : ""}</span>
                  <span className="text-right">{formatDuration(f.durationMs)}</span>
                  <span className={selected ? "" : "text-neutral-500"}>{formatTime(f.startedAt)}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
      <div className="border-t border-neutral-200 px-3 py-1 text-[11px] text-neutral-500 dark:border-neutral-800">
        {rows.length === order.length ? `${order.length} flows` : `${rows.length} of ${order.length} flows`}
      </div>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={flowMenu(menu.flow)} onClose={() => setMenu(null)} />
      )}
    </div>
  );
}

function statusLabel(f: FlowMeta): string {
  if (f.state === "tunnel") return "TUN";
  if (f.state === "blocked") return "BLK";
  if (f.state === "failed") return "ERR";
  if (f.state === "inProgress") return "…";
  return f.status?.toString() ?? "";
}

function rowClass(f: FlowMeta, selected: boolean, index: number): string {
  if (selected) return "bg-sky-600 text-white";
  if (f.state === "failed" || f.state === "blocked")
    return "bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300";
  if (f.state === "tunnel") return "text-neutral-400 italic";
  if (f.state === "inProgress") return "text-neutral-500";
  return index % 2 ? "bg-neutral-50 dark:bg-neutral-900" : "";
}

/** Context-menu entries for one flow. Bodies are fetched on demand. */
export function flowMenu(f: FlowMeta): MenuEntry[] {
  const { pinned, togglePin, setScope } = useFlows.getState();
  const mark = useUi.getState().compareMark;
  const replayable = f.state !== "tunnel" && f.state !== "webSocket";
  const domain = baseDomain(f.host);
  const withDetail = (fn: (d: NonNullable<Awaited<ReturnType<typeof api.getFlow>>>) => void) => () =>
    api.getFlow(f.id).then((d) => d && fn(d));
  return [
    { label: "Copy URL", onSelect: () => copy(f.url, "URL") },
    {
      label: "Copy as cURL",
      onSelect: withDetail((d) => copy(formatCurl(f.method, f.url, d.requestHeaders, d.requestBody), "cURL command")),
    },
    {
      label: "Copy Request Body",
      disabled: f.requestSize === 0,
      onSelect: withDetail((d) => copy(d.requestBody.text ?? d.requestBody.base64 ?? "", "request body")),
    },
    {
      label: "Copy Response Body",
      disabled: f.state !== "completed",
      onSelect: withDetail((d) => copy(d.responseBody.text ?? d.responseBody.base64 ?? "", "response body")),
    },
    "separator",
    { label: "Repeat", disabled: !replayable, onSelect: () => repeat(f.id) },
    { label: "Edit and Repeat…", disabled: !replayable, onSelect: () => editAndRepeat(f.id) },
    {
      label: mark !== null && mark !== f.id ? `Compare with #${mark}` : "Mark for Comparison",
      onSelect: () => compareWith(f.id),
    },
    { label: "Export as HAR…", onSelect: () => files.exportHar([f.id]) },
    "separator",
    {
      label: pinned.includes(domain) ? `Unpin ${domain}` : `Pin ${domain}`,
      onSelect: () => togglePin(domain),
    },
    { label: `Show Only ${f.host}`, onSelect: () => setScope({ kind: "host", value: f.host }) },
  ];
}
