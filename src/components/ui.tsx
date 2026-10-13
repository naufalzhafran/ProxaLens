// Small shared UI primitives (Tailwind only).
import { useEffect, useLayoutEffect, useRef, useState } from "react";

export const button =
  "rounded-lg bg-ink/6 px-2.5 py-1 text-xs font-medium hover:bg-ink/10 disabled:opacity-50";

/** A square toolbar button showing only an icon; `label` is its name and tooltip. */
export function IconButton({
  label,
  onClick,
  children,
  className = "",
}: {
  label: string;
  onClick: (e: React.MouseEvent) => void;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`flex h-[26px] w-[26px] items-center justify-center rounded-lg text-muted hover:bg-ink/8 hover:text-ink ${className}`}
    >
      {children}
    </button>
  );
}

export const primaryButton =
  "rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-on-accent hover:bg-accent/90 disabled:opacity-50";

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
          ? "bg-accent text-on-accent"
          : "text-muted hover:bg-ink/6 hover:text-ink"
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
    <div
      className="flex items-center gap-1 overflow-x-auto border-b border-line px-2 py-1.5 [scrollbar-width:none]"
      role="tablist"
      onKeyDown={(e) => {
        const by = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
        if (!by) return;
        e.preventDefault();
        const i = tabs.findIndex((t) => t.id === value);
        const next = tabs[(i + by + tabs.length) % tabs.length];
        onChange(next.id);
        e.currentTarget.querySelector<HTMLElement>(`[data-tab="${next.id}"]`)?.focus();
      }}
    >
      {tabs.map((t) => (
        <button
          data-tab={t.id}
          tabIndex={t.id === value ? 0 : -1}
          key={t.id}
          role="tab"
          aria-selected={t.id === value}
          onClick={() => onChange(t.id)}
          className={`shrink-0 whitespace-nowrap rounded-md px-2.5 py-0.5 text-xs font-medium ${
            t.id === value ? "bg-panel text-ink ring-1 ring-line" : "text-muted hover:text-ink"
          }`}
        >
          {t.label}
          {t.badge ? <span className="ml-1 text-[10px] font-normal text-muted">{t.badge}</span> : null}
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
  /** Shows a check column; true draws the check. */
  checked?: boolean;
  /** Leaves the menu open after selecting, for toggles. */
  keepOpen?: boolean;
}
export type MenuEntry = MenuItem | "separator";

/**
 * Positions a floating panel at (x, y), kept inside the window, and closes
 * it on click-away, window blur or Escape. Escape is caught first, so a menu
 * inside a dialog closes without closing the dialog too.
 */
function useFloating(x: number, y: number, onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)),
    });
  }, [x, y]);

  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      onClose();
    };
    window.addEventListener("mousedown", down);
    window.addEventListener("keydown", key, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return { ref, style: { left: pos.x, top: pos.y } };
}

/** A panel anchored below a button, closed by click-away or Escape. */
export function Popover({
  x,
  y,
  onClose,
  label,
  children,
}: {
  x: number;
  y: number;
  onClose: () => void;
  label: string;
  children: React.ReactNode;
}) {
  const { ref, style } = useFloating(x, y, onClose);
  useRestoreFocus();
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
  }, [ref]);
  return (
    <div
      ref={ref}
      role="dialog"
      aria-label={label}
      style={style}
      className="fixed z-50 w-80 rounded-ds border border-line bg-panel text-xs"
    >
      {children}
    </div>
  );
}

/** Opens a floating panel below the clicked element. */
export function useAnchor() {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  return {
    at,
    open: (e: React.MouseEvent) => {
      const r = e.currentTarget.getBoundingClientRect();
      setAt({ x: r.left, y: r.bottom + 4 });
    },
    close: () => setAt(null),
  };
}

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), textarea, select, [href], [tabindex]:not([tabindex="-1"])';

/** Puts focus back where it was when the component unmounts. */
function useRestoreFocus() {
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    return () => {
      if (before?.isConnected) before.focus();
    };
  }, []);
}

/**
 * The context-menu key (or ⇧F10) opens the element's context menu, as a
 * right-click at its bottom-left corner would.
 */
export function contextMenuKey(e: React.KeyboardEvent, el: Element | null = e.currentTarget) {
  if (!el || !(e.key === "ContextMenu" || (e.shiftKey && e.key === "F10"))) return false;
  e.preventDefault();
  const r = el.getBoundingClientRect();
  el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 8, clientY: r.bottom }));
  return true;
}

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
  const { ref, style } = useFloating(x, y, onClose);
  const checks = items.some((i) => i !== "separator" && i.checked !== undefined);
  useRestoreFocus();

  // Focus the first item so the menu works from the keyboard at once.
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>("[role^=menuitem]:not(:disabled)")?.focus();
  }, [ref]);

  function onKeyDown(e: React.KeyboardEvent) {
    const enabled = [...e.currentTarget.querySelectorAll<HTMLElement>("[role^=menuitem]:not(:disabled)")];
    const i = enabled.indexOf(document.activeElement as HTMLElement);
    const to = { ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: enabled.length - 1 }[e.key];
    if (e.key === "Tab") onClose();
    if (to === undefined || !enabled.length) return;
    e.preventDefault();
    enabled[(to + enabled.length) % enabled.length].focus();
  }

  return (
    <div
      ref={ref}
      role="menu"
      onKeyDown={onKeyDown}
      style={style}
      className="fixed z-50 min-w-48 rounded-lg border border-line bg-panel p-1 text-xs"
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="my-1 h-px bg-line" />
        ) : (
          <button
            key={i}
            role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={item.checked}
            disabled={item.disabled}
            onClick={() => {
              if (!item.keepOpen) onClose();
              item.onSelect();
            }}
            className="flex w-full items-center rounded-md px-2 py-1 text-left outline-none hover:bg-accent hover:text-on-accent focus:bg-accent focus:text-on-accent disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-inherit"
          >
            {checks && <span className="w-4 shrink-0">{item.checked ? "✓" : ""}</span>}
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}

/**
 * A draggable divider that resizes the element before it. `fit` shrinks the
 * stored size to what currently fits; the returned size is the fitted one.
 */
export function useResizable(
  initial: number,
  min: number,
  max: number,
  axis: "x" | "y",
  storageKey: string,
  fit: (size: number) => number = (s) => s,
) {
  const [size, setSize] = useState(() => {
    try {
      const v = Number(localStorage.getItem(storageKey));
      return v >= min && v <= max ? v : initial;
    } catch {
      return initial;
    }
  });

  const shown = fit(size);

  function onMouseDown(e: React.MouseEvent) {
    e.preventDefault();
    const start = axis === "x" ? e.clientX : e.clientY;
    // Drag from what is on screen, not from a stored size that didn't fit.
    const startSize = shown;
    let latest = startSize;
    const move = (ev: MouseEvent) => {
      const delta = (axis === "x" ? ev.clientX : ev.clientY) - start;
      latest = fit(Math.min(max, Math.max(min, startSize + delta)));
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
      role="separator"
      aria-orientation={axis === "x" ? "vertical" : "horizontal"}
      onMouseDown={onMouseDown}
      className={`shrink-0 bg-line hover:bg-accent ${
        axis === "x" ? "w-px cursor-col-resize px-px" : "h-px cursor-row-resize py-px"
      }`}
    />
  );
  return { size: shown, handle };
}

/**
 * A centered dialog; Escape, the backdrop or ✕ call `onClose`. Without
 * `onClose` the dialog can only be left through its own buttons.
 */
export function Modal({
  title,
  onClose,
  children,
  footer,
  wide,
  narrow,
}: {
  title: string;
  onClose?: () => void;
  children: React.ReactNode;
  footer?: React.ReactNode;
  wide?: boolean;
  narrow?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!onClose) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  useRestoreFocus();
  // Start in the first field (unless a child took focus with autoFocus).
  useEffect(() => {
    const el = ref.current;
    if (!el || el.contains(document.activeElement)) return;
    (el.querySelector<HTMLElement>("input, textarea, select") ?? el).focus();
  }, []);

  /** Keeps Tab inside the dialog. */
  function trap(e: React.KeyboardEvent) {
    if (e.key !== "Tab" || e.defaultPrevented) return;
    const items = [...e.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((n) => n.offsetParent !== null);
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === e.currentTarget)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/30 p-6"
      onMouseDown={(e) => e.target === e.currentTarget && onClose?.()}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal
        aria-label={title}
        tabIndex={-1}
        onKeyDown={trap}
        className={`flex max-h-full w-full flex-col rounded-xl border border-line bg-panel text-xs outline-none ${
          wide ? "max-w-5xl" : narrow ? "max-w-lg" : "max-w-2xl"
        }`}
      >
        <div className="flex items-center border-b border-line px-4 py-2.5">
          <h2 className="flex-1 text-[15px] leading-5 font-semibold">{title}</h2>
          {onClose && (
            <button className="text-muted hover:text-ink" onClick={onClose} aria-label="Close">
              ✕
            </button>
          )}
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-4">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-2.5">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}

export const input =
  "rounded-ds bg-ink/6 px-2 py-1 font-mono text-xs placeholder:text-muted";
