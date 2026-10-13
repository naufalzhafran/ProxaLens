// Small shared UI primitives (Tailwind only).
import { useEffect, useLayoutEffect, useRef, useState } from "react";

export const button =
  "rounded-md border border-neutral-300 px-2.5 py-1 text-xs font-medium hover:bg-neutral-100 disabled:opacity-50 dark:border-neutral-700 dark:hover:bg-neutral-800";

export const primaryButton =
  "rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-500 disabled:opacity-50";

export function Chip({
  active,
  onClick,
  children,
  title,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
  title?: string;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      aria-pressed={active}
      className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${
        active
          ? "bg-sky-600 text-white"
          : "text-neutral-600 hover:bg-neutral-200 dark:text-neutral-400 dark:hover:bg-neutral-800"
      }`}
    >
      {children}
    </button>
  );
}

export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
  right,
}: {
  tabs: { id: T; label: string; badge?: number }[];
  value: T;
  onChange: (t: T) => void;
  right?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-0.5 border-b border-neutral-200 px-2 dark:border-neutral-800" role="tablist">
      {tabs.map((t) => (
        <button
          key={t.id}
          role="tab"
          aria-selected={t.id === value}
          onClick={() => onChange(t.id)}
          className={`-mb-px border-b-2 px-2 py-1 text-xs ${
            t.id === value
              ? "border-sky-600 font-medium text-neutral-900 dark:text-neutral-100"
              : "border-transparent text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
          }`}
        >
          {t.label}
          {t.badge ? <span className="ml-1 text-[10px] text-neutral-400">{t.badge}</span> : null}
        </button>
      ))}
      {right && <div className="ml-auto flex items-center gap-1">{right}</div>}
    </div>
  );
}

export interface MenuItem {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
}
export type MenuEntry = MenuItem | "separator";

/** A context menu at a fixed screen position, closed by click-away or Escape. */
export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: MenuEntry[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.min(x, window.innerWidth - r.width - 4),
      y: Math.min(y, window.innerHeight - r.height - 4),
    });
  }, [x, y]);

  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", down);
    window.addEventListener("keydown", key);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      style={{ left: pos.x, top: pos.y }}
      className="fixed z-50 min-w-48 rounded-md border border-neutral-200 bg-white py-1 text-xs shadow-lg dark:border-neutral-700 dark:bg-neutral-900"
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="my-1 h-px bg-neutral-200 dark:bg-neutral-700" />
        ) : (
          <button
            key={i}
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
            className="block w-full px-3 py-1 text-left hover:bg-sky-600 hover:text-white disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-inherit"
          >
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}

/** A draggable divider that resizes the element before it. */
export function useResizable(initial: number, min: number, max: number, axis: "x" | "y", storageKey: string) {
  const [size, setSize] = useState(() => {
    try {
      const v = Number(localStorage.getItem(storageKey));
      return v >= min && v <= max ? v : initial;
    } catch {
      return initial;
    }
  });

  function onMouseDown(e: React.MouseEvent) {
    e.preventDefault();
    const start = axis === "x" ? e.clientX : e.clientY;
    const startSize = size;
    let latest = startSize;
    const move = (ev: MouseEvent) => {
      const delta = (axis === "x" ? ev.clientX : ev.clientY) - start;
      latest = Math.min(max, Math.max(min, startSize + delta));
      setSize(latest);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      try {
        localStorage.setItem(storageKey, String(latest));
      } catch {
        /* not persisted */
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  const handle = (
    <div
      onMouseDown={onMouseDown}
      className={`shrink-0 bg-neutral-200 hover:bg-sky-500 dark:bg-neutral-800 ${
        axis === "x" ? "w-px cursor-col-resize px-px" : "h-px cursor-row-resize py-px"
      }`}
    />
  );
  return { size, handle };
}

/** A centered dialog; Escape or the backdrop closes it. */
export function Modal({
  title,
  onClose,
  children,
  footer,
  wide,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  footer?: React.ReactNode;
  wide?: boolean;
}) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/30 p-6"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal
        aria-label={title}
        className={`flex max-h-full w-full flex-col rounded-xl border border-neutral-200 bg-white text-xs shadow-2xl dark:border-neutral-700 dark:bg-neutral-900 ${
          wide ? "max-w-5xl" : "max-w-2xl"
        }`}
      >
        <div className="flex items-center border-b border-neutral-200 px-4 py-2.5 dark:border-neutral-800">
          <h2 className="flex-1 text-sm font-semibold">{title}</h2>
          <button className="opacity-60 hover:opacity-100" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-4">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t border-neutral-200 px-4 py-2.5 dark:border-neutral-800">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}

export const input =
  "rounded-md border border-neutral-300 bg-transparent px-2 py-1 font-mono text-xs dark:border-neutral-700";
