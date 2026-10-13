import { forwardRef } from "react";
import { METHODS, STATUS_CLASSES } from "../lib/filter";
import { useFlows } from "../state/flows";
import { Chip } from "./ui";

const HELP =
  "Terms are ANDed; prefix with ! to negate.\n" +
  "s:200  s:4xx   status\n" +
  "m:post         method\n" +
  "ct:json        content type\n" +
  "re:/v[12]/     regex on host+path\n" +
  "text           host+path contains";

export const FilterBar = forwardRef<HTMLInputElement>(function FilterBar(_, ref) {
  const filter = useFlows((s) => s.filter);
  const setFilter = useFlows((s) => s.setFilter);
  const methods = useFlows((s) => s.methods);
  const toggleMethod = useFlows((s) => s.toggleMethod);
  const statuses = useFlows((s) => s.statuses);
  const toggleStatus = useFlows((s) => s.toggleStatus);
  const scope = useFlows((s) => s.scope);
  const setScope = useFlows((s) => s.setScope);

  return (
    <div className="flex flex-wrap items-center gap-1 border-b border-neutral-200 px-2 py-1.5 dark:border-neutral-800">
      <input
        ref={ref}
        className="w-64 rounded-md border border-neutral-300 bg-transparent px-2 py-0.5 font-mono text-xs dark:border-neutral-700"
        placeholder="Filter (⌘F)  e.g. m:post s:4xx api"
        title={HELP}
        spellCheck={false}
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && setFilter("")}
      />
      <div className="mx-1 h-4 w-px bg-neutral-200 dark:bg-neutral-800" />
      {METHODS.map((m) => (
        <Chip key={m} active={methods.includes(m)} onClick={() => toggleMethod(m)}>
          {m}
        </Chip>
      ))}
      <div className="mx-1 h-4 w-px bg-neutral-200 dark:bg-neutral-800" />
      {STATUS_CLASSES.map((c) => (
        <Chip key={c} active={statuses.includes(c)} onClick={() => toggleStatus(c)}>
          {c === "error" ? "Errors" : c}
        </Chip>
      ))}
      {scope && (
        <span className="ml-auto flex items-center gap-1 rounded-full bg-neutral-200 px-2 py-0.5 text-[11px] dark:bg-neutral-800">
          {scope.kind === "domain" ? "Domain" : "Host"}: {scope.value}
          <button className="opacity-60 hover:opacity-100" aria-label="Clear scope" onClick={() => setScope(null)}>
            ✕
          </button>
        </span>
      )}
    </div>
  );
});
