// Name/value tables with copy affordances, shared by the inspector tabs and
// the body views.
import { useState } from "react";
import { Copy as CopyIcon } from "lucide-react";
import { copy } from "../lib/clipboard";
import { ContextMenu } from "./ui";

export function KeyValues({ rows, empty }: { rows: [string, string][]; empty: string }) {
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

export interface CopyRow {
  name: string;
  value: string;
  line: string;
  cell: React.ReactNode;
}

/** Name/value rows with a copy button on hover and a copy context menu. */
export function CopyRows({ rows, lineLabel }: { rows: CopyRow[]; lineLabel: string }) {
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

export function Pre({ text }: { text: string }) {
  return <pre className="whitespace-pre-wrap break-all font-mono text-xs">{text}</pre>;
}
