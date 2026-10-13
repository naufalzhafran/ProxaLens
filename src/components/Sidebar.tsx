// Left pane: all traffic, pinned domains, and a domain → host tree.
import { useMemo, useState } from "react";
import type { FlowMeta } from "../api";
import type { Scope } from "../lib/filter";
import { baseDomain } from "../lib/http";
import { useFlows } from "../state/flows";
import { ContextMenu, type MenuEntry } from "./ui";

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

export function Sidebar() {
  const order = useFlows((s) => s.order);
  const byId = useFlows((s) => s.byId);
  const scope = useFlows((s) => s.scope);
  const setScope = useFlows((s) => s.setScope);
  const pinned = useFlows((s) => s.pinned);
  const togglePin = useFlows((s) => s.togglePin);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<{ x: number; y: number; value: string } | null>(null);

  const groups = useMemo(() => groupByDomain(order, byId), [order, byId]);
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
    <nav className="flex min-h-0 flex-1 flex-col overflow-auto py-2 text-xs">
      <Row selected={isSelected(null)} onClick={() => setScope(null)} label="All traffic" count={order.length} />

      {pinned.length > 0 && (
        <>
          <Heading>Pinned</Heading>
          {pinned.map((p) => (
            <Row
              key={p}
              icon="📌"
              label={p}
              count={counts.get(p)}
              selected={isSelected({ kind: "domain", value: p })}
              onClick={() => setScope({ kind: "domain", value: p })}
              onContextMenu={(e) => onContextMenu(e, p)}
            />
          ))}
        </>
      )}

      <Heading>Domains</Heading>
      {groups.length === 0 && <p className="px-3 py-1 text-neutral-500">None yet</p>}
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
}

function Heading({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mt-3 mb-0.5 px-3 text-[10px] font-semibold uppercase tracking-wide text-neutral-500">
      {children}
    </h3>
  );
}

function Row({
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
  icon?: string;
  indent?: boolean;
  caret?: boolean;
  onCaret?: () => void;
}) {
  return (
    <div
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={label}
      className={`mx-1 flex cursor-default items-center gap-1 rounded px-2 py-0.5 ${indent ? "pl-7" : ""} ${
        selected ? "bg-sky-600 text-white" : "hover:bg-neutral-200/60 dark:hover:bg-neutral-800"
      }`}
    >
      {caret !== undefined ? (
        <button
          className="w-3 text-[9px] opacity-60"
          aria-label={caret ? "Collapse" : "Expand"}
          onClick={(e) => {
            e.stopPropagation();
            onCaret?.();
          }}
        >
          {caret ? "▼" : "▶"}
        </button>
      ) : (
        !indent && <span className="w-3" />
      )}
      {icon && <span className="text-[10px]">{icon}</span>}
      <span className="flex-1 truncate">{label}</span>
      {count !== undefined && <span className={selected ? "opacity-80" : "text-neutral-400"}>{count}</span>}
    </div>
  );
}
