import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp } from "lucide-react";
import { api, type FlowMeta } from "../api";
import { addRuleFor, batch, dontDecrypt, editAndRepeat, files, removeFlows, repeat } from "../lib/actions";
import { copy } from "../lib/clipboard";
import { formatCurl } from "../lib/curl";
import { buildPredicate } from "../lib/filter";
import { baseDomain } from "../lib/http";
import { formatBytes, formatDuration, formatTime, methodHue, shortContentType, statusColor } from "../lib/format";
import { useColumns, type ColumnKey, type Sort } from "../state/columns";
import { useFlows } from "../state/flows";
import { ContextMenu, contextMenuKey, HUE_BADGE, separatorLine, startDrag, type MenuEntry } from "./ui";

const ROW_HEIGHT = 24;
const HEADER_HEIGHT = 24;
const GAP = 6;
const PAD_X = 12;
/** Rows sit this far in from the table edges, so the selection reads as a pill. */
const INSET = 4;

interface Column {
  key: ColumnKey;
  label: string;
  /** Grid track when the user hasn't set a width. */
  track: string;
  /** Narrowest width, also the floor when resizing. */
  min: number;
  right?: boolean;
  /** Sort key; strings sort case-insensitively. */
  value: (f: FlowMeta) => string | number;
  cell: (f: FlowMeta, selected: boolean) => React.ReactNode;
}

const muted = (selected: boolean) => (selected ? "" : "text-muted");

export const COLUMNS: Column[] = [
  {
    key: "id", label: "#", track: "44px", min: 36, value: (f) => f.id,
    cell: (f, sel) => <span className={sel ? "" : "text-muted"}>{f.id}</span>,
  },
  {
    key: "app", label: "App", track: "minmax(60px,0.7fr)", min: 40, value: (f) => f.process ?? "",
    cell: (f) => (
      <span className="truncate font-sans" title={f.pid ? `${f.process} (pid ${f.pid})` : undefined}>
        {f.process ?? ""}
      </span>
    ),
  },
  {
    key: "method", label: "Method", track: "60px", min: 44, value: (f) => f.method,
    cell: (f) => (
      <span className="flex">
        <span data-badge className={`truncate rounded px-1 text-[11px] leading-4 font-semibold ${HUE_BADGE[methodHue(f.method)]}`}>
          {f.method}
        </span>
      </span>
    ),
  },
  {
    key: "status", label: "Status", track: "60px", min: 48, value: (f) => f.status ?? -1,
    cell: (f) => <span className={statusColor(f.status)}>{statusLabel(f)}</span>,
  },
  {
    key: "host", label: "Host", track: "minmax(100px,1.2fr)", min: 60, value: (f) => f.host,
    cell: (f) => <span className="truncate">{f.host}</span>,
  },
  {
    key: "path", label: "Path", track: "minmax(120px,2fr)", min: 60, value: (f) => f.path,
    cell: (f) => (
      <span className="truncate" title={f.tags.length ? `${f.url}\n${f.tags.join(", ")}` : f.url}>
        {f.tags.length > 0 && (
          <span className="mr-1 rounded bg-ink/8 px-1 text-[10px] text-ink">
            {f.tags.map((t) => t.split(" ")[0]).join(" · ")}
          </span>
        )}
        {f.path}
      </span>
    ),
  },
  {
    key: "type", label: "Type", track: "52px", min: 36, value: typeLabel,
    cell: (f, sel) => <span className={`truncate ${muted(sel)}`}>{typeLabel(f)}</span>,
  },
  {
    key: "size", label: "Size", track: "56px", min: 40, right: true, value: (f) => f.responseSize,
    cell: (f) => <span className="truncate text-right">{f.state === "completed" ? formatBytes(f.responseSize) : ""}</span>,
  },
  {
    key: "time", label: "Time", track: "56px", min: 40, right: true, value: (f) => f.durationMs ?? -1,
    cell: (f) => <span className="truncate text-right">{formatDuration(f.durationMs)}</span>,
  },
  {
    key: "started", label: "Started", track: "64px", min: 48, value: (f) => f.startedAt,
    cell: (f, sel) => (
      <span className={`truncate ${muted(sel)}`} title={formatTime(f.startedAt)}>
        {formatTime(f.startedAt, false)}
      </span>
    ),
  },
];

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/** Sorts ids by a column, keeping arrival order for ties; exported for tests. */
export function sortRows(rows: number[], byId: Record<number, FlowMeta>, sort: Sort | null): number[] {
  if (!sort) return rows;
  const col = COLUMNS.find((c) => c.key === sort.key);
  if (!col) return rows;
  const dir = sort.desc ? -1 : 1;
  const keyed = rows.map((id) => [id, col.value(byId[id])] as const);
  keyed.sort(([ia, a], [ib, b]) => {
    const c = typeof a === "number" && typeof b === "number" ? a - b : collator.compare(String(a), String(b));
    return c * dir || ia - ib;
  });
  return keyed.map(([id]) => id);
}

/** Memoised so dragging a pane divider doesn't re-render the table. */
export const FlowTable = memo(function FlowTable() {
  const order = useFlows((s) => s.order);
  const byId = useFlows((s) => s.byId);
  const filter = useFlows((s) => s.filter);
  const methods = useFlows((s) => s.methods);
  const statuses = useFlows((s) => s.statuses);
  const scope = useFlows((s) => s.scope);
  const selectedId = useFlows((s) => s.selectedId);
  const selection = useFlows((s) => s.selection);
  const hidden = useColumns((s) => s.hidden);
  const widths = useColumns((s) => s.widths);
  const sort = useColumns((s) => s.sort);
  // Items are built at render time so toggles stay current while the menu is open.
  const [menu, setMenu] = useState<{ x: number; y: number; items: () => MenuEntry[] } | null>(null);

  const predicate = useMemo(
    () => buildPredicate({ text: filter, methods, statuses, scope }),
    [filter, methods, statuses, scope],
  );
  const rows = useMemo(
    () => sortRows(order.filter((id) => predicate(byId[id])), byId, sort),
    [order, byId, predicate, sort],
  );
  const selected = useMemo(() => new Set(selection), [selection]);
  const tally = useMemo(() => tallyStatuses(rows, byId), [rows, byId]);
  const rowsRef = useRef(rows);
  rowsRef.current = rows;

  const columns = COLUMNS.filter((c) => !hidden.includes(c.key));
  const grid: React.CSSProperties = {
    gridTemplateColumns: columns.map((c) => (widths[c.key] ? `${widths[c.key]}px` : c.track)).join(" "),
    columnGap: GAP,
    paddingInline: PAD_X,
  };
  const minWidth = columns.reduce((n, c) => n + (widths[c.key] ?? c.min), 0) + GAP * (columns.length - 1) + PAD_X * 2;

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
    // Rows start below the sticky header, which mustn't cover a row scrolled to.
    scrollMargin: HEADER_HEIGHT,
    scrollPaddingStart: HEADER_HEIGHT,
  });

  // Follow new traffic when scrolled to the bottom of a list in arrival order.
  const atBottom = useRef(true);
  const follows = !sort || (sort.key === "id" && !sort.desc);
  useEffect(() => {
    if (follows && atBottom.current && rows.length) virtualizer.scrollToIndex(rows.length - 1);
  }, [rows.length, virtualizer, follows]);

  // A new sort order: keep the selected flow in view, or start at the top.
  const lastSort = useRef(sort);
  useEffect(() => {
    if (lastSort.current === sort) return;
    lastSort.current = sort;
    const i = useFlows.getState().selectedId;
    const at = i === null ? -1 : rowsRef.current.indexOf(i);
    virtualizer.scrollToIndex(Math.max(0, at), { align: at < 0 ? "start" : "center" });
  }, [sort, virtualizer]);

  function onRowClick(e: React.MouseEvent, id: number) {
    const s = useFlows.getState();
    if (e.metaKey) s.toggleSelected(id);
    else if (e.shiftKey) s.selectRange(id, rows);
    else s.select(id);
  }

  function onKeyDown(e: React.KeyboardEvent) {
    const s = useFlows.getState();
    if (contextMenuKey(e, e.currentTarget.querySelector(`[data-id="${selectedId}"]`))) return;
    if (e.metaKey && e.key === "a") {
      e.preventDefault();
      s.selectAll(rows);
      return;
    }
    if (e.key === "Backspace" || e.key === "Delete") {
      e.preventDefault();
      removeFlows(rows.filter((id) => selected.has(id)));
      return;
    }
    if (e.key === "Escape" && selection.length > 1) {
      s.select(selectedId);
      return;
    }
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
    if (rows[next] === undefined) return;
    if (e.shiftKey) s.selectRange(rows[next], rows);
    else s.select(rows[next]);
    virtualizer.scrollToIndex(next);
  }

  function onRowMenu(e: React.MouseEvent, f: FlowMeta) {
    e.preventDefault();
    const several = selected.has(f.id) && selection.length > 1;
    if (!several) useFlows.getState().select(f.id);
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: () => (several ? batchMenu(rows.filter((id) => selected.has(id))) : flowMenu(f)),
    });
  }

  function onHeaderMenu(e: React.MouseEvent) {
    e.preventDefault();
    setMenu({ x: e.clientX, y: e.clientY, items: columnMenu });
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        ref={parentRef}
        tabIndex={0}
        aria-label="Flows"
        onKeyDown={onKeyDown}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < ROW_HEIGHT * 2;
        }}
        className="min-h-0 flex-1 overflow-auto outline-none"
      >
        <div style={{ minWidth }}>
          <div
            style={{ ...grid, height: HEADER_HEIGHT }}
            onContextMenu={onHeaderMenu}
            title="Click to sort; drag an edge to resize; right-click to choose columns"
            className="sticky top-0 z-10 grid items-center border-b border-line bg-panel text-[11px] font-medium text-muted"
          >
            {columns.map((c) => (
              <HeaderCell key={c.key} column={c} sort={sort?.key === c.key ? sort : null} />
            ))}
          </div>
          {rows.length > 0 && (
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((v) => {
                const f = byId[rows[v.index]];
                const sel = selected.has(f.id);
                return (
                  <div
                    key={f.id}
                    data-id={f.id}
                    aria-selected={sel}
                    onClick={(e) => onRowClick(e, f.id)}
                    onContextMenu={(e) => onRowMenu(e, f)}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: INSET,
                      right: INSET,
                      height: ROW_HEIGHT,
                      transform: `translateY(${v.start - HEADER_HEIGHT}px)`,
                      ...grid,
                      paddingInline: PAD_X - INSET,
                    }}
                    className={`grid cursor-default items-center rounded-md font-mono text-xs ${rowClass(f, sel, v.index)}`}
                  >
                    {columns.map((c) => (
                      <Cell key={c.key}>{c.cell(f, sel)}</Cell>
                    ))}
                  </div>
                );
              })}
            </div>
          )}
        </div>
        {rows.length === 0 && (
          <p className="p-6 text-center text-muted">
            {order.length
              ? "No flows match the current filter."
              : "No traffic yet. Point an app at the proxy, or switch the proxy on."}
          </p>
        )}
      </div>
      <div className="flex items-center gap-3 border-t border-line px-3 py-1 text-[11px] text-muted">
        <span>
          {rows.length === order.length ? `${order.length} flows` : `${rows.length} of ${order.length} flows`}
          {selection.length > 1 && ` · ${selection.length} selected`}
        </span>
        <span className="ml-auto flex items-center gap-3 tabular-nums">
          <Tally dot="bg-st-ok" label="2xx" n={tally.ok} />
          <Tally dot="bg-st-redirect" label="3xx" n={tally.redirect} />
          <Tally dot="bg-st-client" label="4xx" n={tally.client} />
          <Tally dot="bg-st-server" label="5xx & errors" n={tally.server} />
        </span>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items()} onClose={() => setMenu(null)} />}
    </div>
  );
});

/** Shown flows by status class, for the footer; failed and blocked count as errors. */
export function tallyStatuses(rows: number[], byId: Record<number, FlowMeta>) {
  const t = { ok: 0, redirect: 0, client: 0, server: 0 };
  for (const id of rows) {
    const f = byId[id];
    if (f.state === "failed" || f.state === "blocked") t.server++;
    else if (f.status === null) continue;
    else if (f.status >= 500) t.server++;
    else if (f.status >= 400) t.client++;
    else if (f.status >= 300) t.redirect++;
    else t.ok++;
  }
  return t;
}

function Tally({ dot, label, n }: { dot: string; label: string; n: number }) {
  return (
    <span className="flex items-center gap-1.5" title={`${n} ${label}`}>
      <span aria-hidden className={`h-1.5 w-1.5 rounded-full ${dot}`} />
      <span className="text-ink">{n}</span>
      <span className="sr-only">{label}</span>
    </span>
  );
}

/** Lets a cell's content truncate inside its grid track. */
function Cell({ children }: { children: React.ReactNode }) {
  return <div className="flex min-w-0 [&>*]:min-w-0 [&>*]:flex-1">{children}</div>;
}

function HeaderCell({ column, sort }: { column: Column; sort: Sort | null }) {
  const [active, setActive] = useState(false);

  function startResize(e: React.PointerEvent) {
    const startWidth = e.currentTarget.parentElement!.getBoundingClientRect().width;
    const started = startDrag(
      e,
      "x",
      (delta) => useColumns.getState().setWidth(column.key, Math.max(column.min, startWidth + delta)),
      () => setActive(false),
    );
    if (started) setActive(true);
  }

  return (
    <div className="relative flex h-full min-w-0 items-center">
      <button
        className={`flex h-full min-w-0 flex-1 items-center gap-0.5 ${column.right ? "justify-end" : ""} ${sort ? "text-ink" : ""}`}
        aria-sort={sort ? (sort.desc ? "descending" : "ascending") : undefined}
        onClick={() => useColumns.getState().cycleSort(column.key)}
      >
        <span className="truncate">{column.label}</span>
        {sort && (sort.desc ? <ArrowDown size={10} className="shrink-0" /> : <ArrowUp size={10} className="shrink-0" />)}
      </button>
      {/* A 10px grab area centred on a 1px line in the middle of the column gap. */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={`Resize ${column.label}`}
        onPointerDown={startResize}
        onDoubleClick={() => useColumns.getState().setWidth(column.key, null)}
        className="group absolute top-0 bottom-0 z-10 w-[10px] cursor-col-resize"
        style={{ right: -(GAP / 2 + 5) }}
      >
        <span
          data-active={active || undefined}
          className={`${separatorLine} absolute top-1 bottom-1 left-[5px] w-px bg-line group-hover:bg-accent group-hover:delay-150`}
        />
      </div>
    </div>
  );
}

/** Show/hide columns; the last visible one can't be hidden. */
function columnMenu(): MenuEntry[] {
  const { hidden, toggleHidden, reset } = useColumns.getState();
  const visible = COLUMNS.length - hidden.length;
  return [
    ...COLUMNS.map((c) => {
      const shown = !hidden.includes(c.key);
      return {
        label: c.label === "#" ? "# (number)" : c.label,
        checked: shown,
        keepOpen: true,
        disabled: shown && visible === 1,
        onSelect: () => toggleHidden(c.key),
      };
    }),
    "separator" as const,
    { label: "Reset Columns", onSelect: reset },
  ];
}

function typeLabel(f: FlowMeta): string {
  return f.state === "webSocket" ? "ws" : f.state === "tunnel" ? "tunnel" : shortContentType(f.contentType);
}

function statusLabel(f: FlowMeta): string {
  if (f.state === "tunnel") return "TUN";
  if (f.state === "blocked") return "BLK";
  if (f.state === "failed") return "ERR";
  if (f.state === "inProgress") return "…";
  return f.status?.toString() ?? "";
}

function rowClass(f: FlowMeta, selected: boolean, index: number): string {
  if (selected) return "row-sel bg-accent text-on-accent";
  if (f.state === "failed" || f.state === "blocked")
    return "bg-st-server/8 text-st-server";
  if (f.state === "tunnel") return "text-muted italic";
  if (f.state === "inProgress") return "text-muted";
  return index % 2 ? "bg-ink/[0.03] hover:bg-ink/6" : "hover:bg-ink/5";
}

/** Context-menu entries for one flow. Bodies are fetched on demand. */
export function flowMenu(f: FlowMeta): MenuEntry[] {
  const { pinned, togglePin, setScope } = useFlows.getState();
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
    { label: "Export as HAR…", onSelect: () => files.exportHar([f.id]) },
    "separator",
    { label: "Map Local…", disabled: !replayable, onSelect: () => addRuleFor(f.id, "mapLocal") },
    { label: "Map Remote…", disabled: !replayable, onSelect: () => addRuleFor(f.id, "mapRemote") },
    { label: "Rewrite…", disabled: !replayable, onSelect: () => addRuleFor(f.id, "rewrite") },
    { label: "Block URL…", disabled: !replayable, onSelect: () => addRuleFor(f.id, "block") },
    "separator",
    ...(f.scheme === "https" && f.state !== "tunnel"
      ? [{ label: `Don’t Decrypt ${f.host}`, onSelect: () => dontDecrypt(f.host) }]
      : []),
    {
      label: pinned.includes(domain) ? `Unpin ${domain}` : `Pin ${domain}`,
      onSelect: () => togglePin(domain),
    },
    { label: `Show Only ${f.host}`, onSelect: () => setScope({ kind: "host", value: f.host }) },
    ...(f.process ? [{ label: `Show Only ${f.process}`, onSelect: () => setScope({ kind: "app", value: f.process! }) }] : []),
    "separator",
    { label: "Remove Flow (⌫)", onSelect: () => removeFlows([f.id]) },
  ];
}

/** Context-menu entries for several selected flows, in table order. */
export function batchMenu(ids: number[]): MenuEntry[] {
  const { byId } = useFlows.getState();
  const replayable = ids.filter((id) => byId[id] && byId[id].state !== "tunnel" && byId[id].state !== "webSocket");
  return [
    { label: `${ids.length} flows selected`, disabled: true, onSelect: () => {} },
    "separator",
    { label: "Copy URLs", onSelect: () => batch.copyUrls(ids) },
    { label: "Copy as cURL", onSelect: () => batch.copyCurls(replayable) },
    { label: "Export as HAR…", onSelect: () => files.exportHar(ids) },
    "separator",
    { label: `Repeat ${replayable.length} in Order`, disabled: !replayable.length, onSelect: () => batch.repeat(replayable) },
    "separator",
    { label: `Remove ${ids.length} Flows (⌫)`, onSelect: () => removeFlows(ids) },
  ];
}
