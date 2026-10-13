// Collapsible JSON viewer. Children render only when expanded, so large
// documents stay cheap.
import { ChevronRight, Copy as CopyIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import { copy } from "../lib/clipboard";
import { ContextMenu } from "./ui";

const PAGE = 200;

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

/** What "Copy Value" puts on the clipboard: strings unquoted, the rest as JSON. */
export function copyText(value: Json): string {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

/** A JavaScript-style path to a child, e.g. `data.items[0]["content-type"]`. */
export function childPath(
  parent: string,
  key: string,
  isIndex: boolean,
): string {
  if (isIndex) return `${parent}[${key}]`;
  if (/^[A-Za-z_$][\w$]*$/.test(key)) return parent ? `${parent}.${key}` : key;
  return `${parent}[${JSON.stringify(key)}]`;
}

export function JsonTree({ value }: { value: Json }) {
  return (
    <div className="font-mono text-xs leading-5">
      <Node name={null} path="" value={value} depth={0} last />
    </div>
  );
}

/** One line of the tree, with a copy button on hover and a copy context menu. */
function Row({
  name,
  path,
  value,
  children,
}: {
  name: string | null;
  path: string;
  value: Json;
  children: ReactNode;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const what = name ?? (path || "value");
  return (
    <div
      className="group flex items-start hover:bg-ink/3"
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY });
      }}
    >
      <div className="min-w-0 flex-1">{children}</div>
      <button
        type="button"
        aria-label={`Copy ${what}`}
        title="Copy value"
        className="invisible mr-1 flex-none rounded p-0.5 text-muted group-hover:visible hover:text-ink focus-visible:visible"
        onClick={() => copy(copyText(value), what)}
      >
        <CopyIcon size={12} />
      </button>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            {
              label: "Copy Value",
              onSelect: () => copy(copyText(value), what),
            },
            ...(name !== null
              ? [{ label: "Copy Key", onSelect: () => copy(name, "key") }]
              : []),
            ...(path
              ? [{ label: "Copy Path", onSelect: () => copy(path, "path") }]
              : []),
          ]}
        />
      )}
    </div>
  );
}

function Node({
  name,
  path,
  value,
  depth,
  last,
}: {
  name: string | null;
  path: string;
  value: Json;
  depth: number;
  last: boolean;
}) {
  const [open, setOpen] = useState(depth < 2);
  const [shown, setShown] = useState(PAGE);
  const comma = last ? "" : ",";
  const label = name !== null && (
    <>
      <span className="text-syn-key">{JSON.stringify(name)}</span>
      <span className="text-muted">: </span>
    </>
  );

  if (value === null || typeof value !== "object") {
    return (
      <Row name={name} path={path} value={value}>
        <div style={{ paddingLeft: depth * 14 + 12 }}>
          {label}
          <Scalar value={value} />
          {comma}
        </div>
      </Row>
    );
  }

  const isArray = Array.isArray(value);
  const entries: [string, Json][] = isArray
    ? (value as Json[]).map((v, i) => [String(i), v])
    : Object.entries(value);
  const [openB, closeB] = isArray ? ["[", "]"] : ["{", "}"];

  if (!entries.length) {
    return (
      <Row name={name} path={path} value={value}>
        <div style={{ paddingLeft: depth * 14 + 12 }}>
          {label}
          {openB}
          {closeB}
          {comma}
        </div>
      </Row>
    );
  }

  return (
    <div>
      <Row name={name} path={path} value={value}>
        <button
          type="button"
          aria-expanded={open}
          style={{ paddingLeft: depth * 14 }}
          className="block w-full cursor-default text-left"
          onClick={() => setOpen(!open)}
        >
          <ChevronRight
            size={11}
            strokeWidth={2}
            className={`mr-0.5 inline-block align-[-1px] text-muted transition-transform duration-150 motion-reduce:transition-none ${open ? "rotate-90" : ""}`}
          />
          {label}
          {openB}
          {!open && (
            <>
              <span className="text-muted">
                {" "}
                {entries.length}{" "}
                {isArray
                  ? entries.length === 1
                    ? "item"
                    : "items"
                  : entries.length === 1
                    ? "key"
                    : "keys"}{" "}
              </span>
              {closeB}
              {comma}
            </>
          )}
        </button>
      </Row>
      {open && (
        <>
          {entries.slice(0, shown).map(([k, v], i) => (
            <Node
              key={k}
              name={isArray ? null : k}
              path={childPath(path, k, isArray)}
              value={v}
              depth={depth + 1}
              last={i === entries.length - 1}
            />
          ))}
          {shown < entries.length && (
            <button
              style={{ marginLeft: (depth + 1) * 14 + 12 }}
              className="text-accent hover:underline"
              onClick={() => setShown(shown + PAGE)}
            >
              Show {Math.min(PAGE, entries.length - shown)} more of{" "}
              {entries.length - shown}…
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
    return (
      <span className="break-all text-syn-string">{JSON.stringify(value)}</span>
    );
  if (typeof value === "number")
    return <span className="text-syn-number">{value}</span>;
  return <span className="text-syn-literal">{String(value)}</span>;
}
