// Collapsible JSON viewer. Children render only when expanded, so large
// documents stay cheap.
import { useState } from "react";

const PAGE = 200;

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

export function JsonTree({ value }: { value: Json }) {
  return (
    <div className="font-mono text-xs leading-5">
      <Node name={null} value={value} depth={0} last />
    </div>
  );
}

function Node({ name, value, depth, last }: { name: string | null; value: Json; depth: number; last: boolean }) {
  const [open, setOpen] = useState(depth < 2);
  const [shown, setShown] = useState(PAGE);
  const comma = last ? "" : ",";
  const label = name !== null && (
    <>
      <span className="text-purple-700 dark:text-purple-300">{JSON.stringify(name)}</span>
      <span className="text-neutral-500">: </span>
    </>
  );

  if (value === null || typeof value !== "object") {
    return (
      <div style={{ paddingLeft: depth * 14 + 12 }}>
        {label}
        <Scalar value={value} />
        {comma}
      </div>
    );
  }

  const isArray = Array.isArray(value);
  const entries: [string, Json][] = isArray
    ? (value as Json[]).map((v, i) => [String(i), v])
    : Object.entries(value);
  const [openB, closeB] = isArray ? ["[", "]"] : ["{", "}"];

  if (!entries.length) {
    return (
      <div style={{ paddingLeft: depth * 14 + 12 }}>
        {label}
        {openB}
        {closeB}
        {comma}
      </div>
    );
  }

  return (
    <div>
      <div style={{ paddingLeft: depth * 14 }} className="cursor-default" onClick={() => setOpen(!open)}>
        <span className="inline-block w-3 text-[9px] text-neutral-400">{open ? "▼" : "▶"}</span>
        {label}
        {openB}
        {!open && (
          <>
            <span className="text-neutral-400">
              {" "}
              {entries.length} {isArray ? (entries.length === 1 ? "item" : "items") : entries.length === 1 ? "key" : "keys"}{" "}
            </span>
            {closeB}
            {comma}
          </>
        )}
      </div>
      {open && (
        <>
          {entries.slice(0, shown).map(([k, v], i) => (
            <Node
              key={k}
              name={isArray ? null : k}
              value={v}
              depth={depth + 1}
              last={i === entries.length - 1}
            />
          ))}
          {shown < entries.length && (
            <button
              style={{ marginLeft: (depth + 1) * 14 + 12 }}
              className="text-sky-600 hover:underline"
              onClick={() => setShown(shown + PAGE)}
            >
              Show {Math.min(PAGE, entries.length - shown)} more of {entries.length - shown}…
            </button>
          )}
          <div style={{ paddingLeft: depth * 14 + 12 }}>
            {closeB}
            {comma}
          </div>
        </>
      )}
    </div>
  );
}

function Scalar({ value }: { value: null | boolean | number | string }) {
  if (typeof value === "string")
    return <span className="break-all text-emerald-700 dark:text-emerald-400">{JSON.stringify(value)}</span>;
  if (typeof value === "number") return <span className="text-sky-700 dark:text-sky-400">{value}</span>;
  return <span className="text-orange-600 dark:text-orange-400">{String(value)}</span>;
}
