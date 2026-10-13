import { useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { formatBytes, formatDuration, formatTime, statusColor } from "../lib/format";
import { matchesFilter, useFlows } from "../state/flows";

const ROW_HEIGHT = 24;
const COLUMNS = "grid-cols-[56px_64px_56px_minmax(120px,1fr)_minmax(160px,2fr)_72px_72px_96px]";

export function FlowTable() {
  const order = useFlows((s) => s.order);
  const byId = useFlows((s) => s.byId);
  const filter = useFlows((s) => s.filter);
  const selectedId = useFlows((s) => s.selectedId);
  const select = useFlows((s) => s.select);

  const rows = useMemo(
    () => order.filter((id) => matchesFilter(byId[id], filter)),
    [order, byId, filter],
  );

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
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const i = selectedId === null ? -1 : rows.indexOf(selectedId);
    const next = Math.min(rows.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)));
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
            No traffic yet. Point an app at the proxy, or turn on “System proxy”.
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
                  style={{ position: "absolute", top: 0, left: 0, right: 0, height: ROW_HEIGHT, transform: `translateY(${v.start}px)` }}
                  className={`grid ${COLUMNS} cursor-default items-center gap-2 px-3 font-mono text-xs ${
                    selected
                      ? "bg-sky-600 text-white"
                      : v.index % 2
                        ? "bg-neutral-50 dark:bg-neutral-900"
                        : ""
                  }`}
                >
                  <span className={selected ? "" : "text-neutral-400"}>{f.id}</span>
                  <span className="font-semibold">{f.method}</span>
                  <span className={selected ? "" : statusColor(f.status)}>
                    {f.state === "failed" ? "ERR" : f.state === "inProgress" ? "…" : (f.status ?? "")}
                  </span>
                  <span className="truncate">{f.host}</span>
                  <span className="truncate" title={f.url}>
                    {f.path}
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
    </div>
  );
}
