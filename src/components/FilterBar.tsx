import { forwardRef, useState } from "react";
import { METHODS, STATUS_CLASSES } from "../lib/filter";
import { useFlows } from "../state/flows";
import { button, ContextMenu, type MenuEntry } from "./ui";

const HELP =
  "Terms are ANDed; prefix with ! to negate.\n" +
  "s:200  s:4xx   status\n" +
  "m:post         method\n" +
  "ct:json        content type\n" +
  "re:/v[12]/     regex on host+path\n" +
  "app:chrome     app that made the request\n" +
  "text           host+path contains";

const statusLabel = (c: string) => (c === "error" ? "Errors" : c);

export const FilterBar = forwardRef<HTMLInputElement>(function FilterBar(_, ref) {
  const filter = useFlows((s) => s.filter);
  const setFilter = useFlows((s) => s.setFilter);
  const methods = useFlows((s) => s.methods);
  const toggleMethod = useFlows((s) => s.toggleMethod);
  const statuses = useFlows((s) => s.statuses);
  const toggleStatus = useFlows((s) => s.toggleStatus);
  const scope = useFlows((s) => s.scope);
  const setScope = useFlows((s) => s.setScope);
  const [menu, setMenu] = useState<{ x: number; y: number; kind: "method" | "status" } | null>(null);

  function showMenu(e: React.MouseEvent, kind: "method" | "status") {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ x: r.left, y: r.bottom + 4, kind });
  }

  // Built on every render so the checks follow the toggles while the menu stays open.
  const items: MenuEntry[] =
    menu?.kind === "method"
      ? toggles(METHODS, methods, toggleMethod, (m) => m)
      : toggles(STATUS_CLASSES, statuses, toggleStatus, statusLabel);

  return (
    <div className="flex items-center gap-1 border-b border-neutral-200 px-2 py-1.5 dark:border-neutral-800">
      <input
        ref={ref}
        className="min-w-0 max-w-80 flex-1 rounded-md border border-neutral-300 bg-transparent px-2 py-0.5 font-mono text-xs dark:border-neutral-700"
        placeholder="Filter (⌘F)  e.g. m:post s:4xx api"
        title={HELP}
        spellCheck={false}
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && setFilter("")}
      />
      <MenuButton label="Method" active={methods} onClick={(e) => showMenu(e, "method")} />
      <MenuButton label="Status" active={statuses.map(statusLabel)} onClick={(e) => showMenu(e, "status")} />
      {scope && (
        <span className="ml-auto flex min-w-0 shrink items-center gap-1 rounded-full bg-neutral-200 px-2 py-0.5 text-[11px] dark:bg-neutral-800">
          <span className="truncate" title={scope.value}>
            {{ domain: "Domain", host: "Host", app: "App" }[scope.kind]}: {scope.value}
          </span>
          <button className="opacity-60 hover:opacity-100" aria-label="Clear scope" onClick={() => setScope(null)}>
            ✕
          </button>
        </span>
      )}
      {menu && <ContextMenu x={menu.x} y={menu.y} items={items} onClose={() => setMenu(null)} />}
    </div>
  );
});

/** Check items that toggle in place, plus a "Show All" reset when any is on. */
function toggles<T extends string>(all: readonly T[], on: T[], toggle: (v: T) => void, label: (v: T) => string): MenuEntry[] {
  return [
    ...all.map((v) => ({ label: label(v), checked: on.includes(v), keepOpen: true, onSelect: () => toggle(v) })),
    "separator" as const,
    { label: "Show All", disabled: on.length === 0, onSelect: () => on.forEach(toggle) },
  ];
}

/** "Method", "Method: GET" or "Method · 3", highlighted while it narrows the list. */
function MenuButton({ label, active, onClick }: { label: string; active: string[]; onClick: (e: React.MouseEvent) => void }) {
  const text = active.length === 0 ? label : active.length === 1 ? `${label}: ${active[0]}` : `${label} · ${active.length}`;
  return (
    <button
      className={`${button} shrink-0 ${active.length ? "border-sky-500 text-sky-700 dark:text-sky-300" : ""}`}
      aria-haspopup="menu"
      title={active.length ? active.join(", ") : undefined}
      onClick={onClick}
    >
      {text} ▾
    </button>
  );
}
