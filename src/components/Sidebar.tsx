// Left pane: all traffic, pinned domains, and a domain → host tree.
import { memo, useMemo, useState } from "react";
import { ChevronRight, Pin } from "lucide-react";
import type { FlowMeta } from "../api";
import type { Scope } from "../lib/filter";
import { baseDomain } from "../lib/http";
import { useFlows } from "../state/flows";
import { ContextMenu, contextMenuKey, type MenuEntry } from "./ui";

interface DomainGroup {
  domain: string;
  count: number;
  hosts: [string, number][];
}

export function groupByDomain(order: number[], byId: Record<number, FlowMeta>): DomainGroup[] {
  const groups = new Map<string, Map<string, number>>();
  for (const id of order) {
    const host = byId[id].host;
    const d = baseDomain(host);
    let hosts = groups.get(d);
    if (!hosts) groups.set(d, (hosts = new Map()));
    hosts.set(host, (hosts.get(host) ?? 0) + 1);
  }
  return [...groups]
    .map(([domain, hosts]) => ({
      domain,
      count: [...hosts.values()].reduce((a, b) => a + b, 0),
      hosts: [...hosts].sort(([a], [b]) => a.localeCompare(b)),
    }))
    .sort((a, b) => a.domain.localeCompare(b.domain));
}

/** Apps that made requests, by number of flows. */
export function groupByApp(order: number[], byId: Record<number, FlowMeta>): [string, number][] {
  const counts = new Map<string, number>();
  for (const id of order) {
    const p = byId[id].process;
    if (p) counts.set(p, (counts.get(p) ?? 0) + 1);
  }
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

export const Sidebar = memo(function Sidebar() {
  const order = useFlows((s) => s.order);
  const byId = useFlows((s) => s.byId);
  const scope = useFlows((s) => s.scope);
  const setScope = useFlows((s) => s.setScope);
  const pinned = useFlows((s) => s.pinned);
  const togglePin = useFlows((s) => s.togglePin);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<{ x: number; y: number; value: string } | null>(null);

  const groups = useMemo(() => groupByDomain(order, byId), [order, byId]);
  const apps = useMemo(() => groupByApp(order, byId), [order, byId]);
  const counts = useMemo(() => new Map(groups.map((g) => [g.domain, g.count])), [groups]);

  const isSelected = (s: Scope) =>
    s === null ? scope === null : scope?.kind === s.kind && scope.value === s.value;

  function onContextMenu(e: React.MouseEvent, value: string) {
    e.preventDefault();
    setMenu({ x: e.clientX, y: e.clientY, value });
  }

  const menuItems: MenuEntry[] = menu
    ? [
        {
          label: pinned.includes(menu.value) ? `Unpin ${menu.value}` : `Pin ${menu.value}`,
          onSelect: () => togglePin(menu.value),
        },
        { label: "Show only this domain", onSelect: () => setScope({ kind: "domain", value: menu.value }) },
      ]
    : [];

  return (
    <nav aria-label="Traffic" onKeyDown={onNavKeyDown} className="flex min-h-0 flex-1 flex-col overflow-auto py-2 text-xs">
      <Row selected={isSelected(null)} onClick={() => setScope(null)} label="All traffic" count={order.length} />

      {pinned.length > 0 && (
        <>
          <Heading>Pinned</Heading>
          {pinned.map((p) => (
            <Row
              key={p}
              icon={<Pin size={11} className="shrink-0" aria-label="Pinned" />}
              label={p}
              count={counts.get(p)}
              selected={isSelected({ kind: "domain", value: p })}
              onClick={() => setScope({ kind: "domain", value: p })}
              onContextMenu={(e) => onContextMenu(e, p)}
            />
          ))}
        </>
      )}

      {apps.length > 0 && (
        <>
          <Heading>Apps</Heading>
          {apps.map(([app, count]) => (
            <Row
              key={app}
              label={app}
              count={count}
              selected={isSelected({ kind: "app", value: app })}
              onClick={() => setScope({ kind: "app", value: app })}
            />
          ))}
        </>
      )}

      <Heading>Domains</Heading>
      {groups.length === 0 && <p className="px-3 py-1 text-muted">None yet</p>}
      {groups.map((g) => {
        const single = g.hosts.length === 1 && g.hosts[0][0] === g.domain;
        const open = expanded[g.domain] ?? false;
        return (
          <div key={g.domain}>
            <Row
              label={g.domain}
              count={g.count}
              caret={single ? undefined : open}
              onCaret={() => setExpanded((e) => ({ ...e, [g.domain]: !open }))}
              selected={isSelected({ kind: "domain", value: g.domain })}
              onClick={() => setScope({ kind: "domain", value: g.domain })}
              onContextMenu={(e) => onContextMenu(e, g.domain)}
            />
            {open &&
              !single &&
              g.hosts.map(([host, count]) => (
                <Row
                  key={host}
                  indent
                  label={host}
                  count={count}
                  selected={isSelected({ kind: "host", value: host })}
                  onClick={() => setScope({ kind: "host", value: host })}
                  onContextMenu={(e) => onContextMenu(e, host)}
                />
              ))}
          </div>
        );
      })}
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems} onClose={() => setMenu(null)} />}
    </nav>
  );
});

function Heading({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mt-3 mb-0.5 px-3 text-[11px] font-semibold uppercase tracking-[0.04em] text-muted">
      {children}
    </h3>
  );
}

export function Row({
  label,
  count,
  selected,
  onClick,
  onContextMenu,
  icon,
  indent,
  caret,
  onCaret,
}: {
  label: string;
  count?: number;
  selected: boolean;
  onClick: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  icon?: React.ReactNode;
  indent?: boolean;
  caret?: boolean;
  onCaret?: () => void;
}) {
  return (
    <div
      className={`mx-1 flex items-center rounded-md ${indent ? "pl-5" : ""} ${
        selected ? "bg-select font-semibold text-ink" : "hover:bg-ink/6"
      }`}
    >
      {caret !== undefined ? (
        <button
          tabIndex={-1}
          className="flex w-5 shrink-0 items-center justify-center self-stretch text-muted"
          aria-label={caret ? `Collapse ${label}` : `Expand ${label}`}
          onClick={onCaret}
        >
          <ChevronRight
            size={12}
            strokeWidth={2}
            className={`transition-transform duration-150 motion-reduce:transition-none ${caret ? "rotate-90" : ""}`}
          />
        </button>
      ) : (
        <span className="w-5 shrink-0" />
      )}
      <button
        data-nav-row
        aria-current={selected ? "true" : undefined}
        aria-expanded={caret}
        title={label}
        onClick={onClick}
        onContextMenu={onContextMenu}
        onKeyDown={(e) => {
          if (contextMenuKey(e)) return;
          // → expands and ← collapses a domain, as in a Finder sidebar.
          if ((e.key === "ArrowRight" && caret === false) || (e.key === "ArrowLeft" && caret === true)) {
            e.preventDefault();
            onCaret?.();
          }
        }}
        className="flex min-w-0 flex-1 cursor-default items-center gap-1 py-0.5 pr-2 text-left"
      >
        {icon}
        <span className="flex-1 truncate">{label}</span>
        {count !== undefined && <span className="font-normal text-muted tabular-nums">{count}</span>}
      </button>
    </div>
  );
}

/** ↑/↓ move focus between sidebar rows. */
function onNavKeyDown(e: React.KeyboardEvent) {
  const by = { ArrowDown: 1, ArrowUp: -1 }[e.key];
  if (!by) return;
  const rows = [...e.currentTarget.querySelectorAll<HTMLElement>("[data-nav-row]")];
  const i = rows.indexOf(document.activeElement as HTMLElement);
  if (i < 0) return;
  e.preventDefault();
  rows[Math.min(rows.length - 1, Math.max(0, i + by))].focus();
}
