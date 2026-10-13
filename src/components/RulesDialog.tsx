// Editor for Map Local, Map Remote, Rewrite, Block and Breakpoint rules.
import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  api,
  errorMessage,
  type BreakpointRule,
  type MapLocalRule,
  type MapRemoteRule,
  type Matcher,
  type RewriteRule,
  type Rules,
} from "../api";
import { parseHeaders } from "../lib/actions";
import { useToast } from "../lib/clipboard";
import { guardedClose } from "../lib/confirm";
import { actionsFor, newRule, RULE_KINDS, ruleSummary, type RuleKind } from "../lib/rules";
import { useTools, useUi, type RulesView } from "../state/ui";
import { button, input, Modal, primaryButton, Tabs } from "./ui";

type AnyRule = Rules[RuleKind][number];
const METHODS = ["", "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function RulesDialog({ view }: { view: RulesView }) {
  const close = useUi((s) => s.close);
  const [rules, setRules] = useState<Rules | null>(null);
  // What's on disk, to tell whether closing would lose edits.
  const [saved, setSaved] = useState<string | null>(null);
  const [tab, setTab] = useState<RuleKind>(view.tab);
  const [selected, setSelected] = useState<string | null>(view.add?.id ?? null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .getRules()
      .then((r) => {
        setSaved(JSON.stringify(r));
        if (view.add) r = { ...r, [view.tab]: [...r[view.tab], view.add] };
        setRules(r);
        setSelected((s) => s ?? r[view.tab][0]?.id ?? null);
      })
      .catch((e) => setError(errorMessage(e)));
  }, [view]);

  const dirty = useMemo(() => rules !== null && JSON.stringify(rules) !== saved, [rules, saved]);
  // A rule moved or picked from the keyboard keeps focus after re-rendering.
  const focusRule = useRef<string | null>(null);
  useEffect(() => {
    if (!focusRule.current) return;
    document.querySelector<HTMLElement>(`[data-rule-id="${focusRule.current}"]`)?.focus();
    focusRule.current = null;
  });

  if (!rules) return null;
  const cancel = guardedClose(dirty, close, "rule changes");
  const list = rules[tab] as AnyRule[];
  const current = list.find((r) => r.id === selected) ?? null;

  const setList = (next: AnyRule[]) => setRules({ ...rules, [tab]: next });
  const update = (r: AnyRule) => setList(list.map((x) => (x.id === r.id ? r : x)));
  function add() {
    const r = newRule(tab);
    setList([...list, r]);
    setSelected(r.id);
  }
  function remove(id: string) {
    const next = list.filter((r) => r.id !== id);
    setList(next);
    if (selected === id) setSelected(next[0]?.id ?? null);
  }
  function move(id: string, by: number) {
    const i = list.findIndex((r) => r.id === id);
    const j = i + by;
    if (j < 0 || j >= list.length) return;
    const next = [...list];
    [next[i], next[j]] = [next[j], next[i]];
    setList(next);
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      useTools.getState().setRules(await api.setRules(rules!));
      useToast.getState().show("Rules saved");
      close();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Rules"
      wide
      onClose={cancel}
      footer={
        <>
          {error && <span className="mr-auto text-red-500">{error}</span>}
          <span className="mr-2 text-neutral-500">The first matching Map Local / Map Remote rule wins; all matching rewrites apply in order.</span>
          <button className={button} onClick={cancel}>
            Cancel
          </button>
          <button className={primaryButton} disabled={busy} onClick={save}>
            Save
          </button>
        </>
      }
    >
      <Tabs<RuleKind>
        value={tab}
        onChange={(t) => {
          setTab(t);
          setSelected(rules[t][0]?.id ?? null);
        }}
        tabs={RULE_KINDS.map((k) => ({ id: k.id, label: k.label, badge: rules[k.id].length || undefined }))}
      />
      <div className="flex min-h-[380px] gap-3 pt-3">
        <div className="flex w-64 shrink-0 flex-col gap-1" role="listbox" aria-label="Rules" aria-description="⌥↑ and ⌥↓ reorder">
          {list.map((r, i) => (
            <div
              key={r.id}
              data-rule-id={r.id}
              role="option"
              aria-selected={r.id === selected}
              tabIndex={r.id === selected || (selected === null && i === 0) ? 0 : -1}
              onClick={() => setSelected(r.id)}
              onKeyDown={(e) => {
                if (e.target !== e.currentTarget) return;
                const by = { ArrowUp: -1, ArrowDown: 1 }[e.key];
                if (!by) return;
                e.preventDefault();
                if (e.altKey) {
                  move(r.id, by);
                  focusRule.current = r.id;
                } else if (list[i + by]) {
                  setSelected(list[i + by].id);
                  focusRule.current = list[i + by].id;
                }
              }}
              className={`group flex cursor-default items-start gap-2 rounded-md px-2 py-1 ${
                r.id === selected ? "bg-sky-600 text-white" : "hover:bg-neutral-100 dark:hover:bg-neutral-800"
              }`}
            >
              <input
                type="checkbox"
                aria-label="Enabled"
                className="mt-0.5"
                checked={r.enabled}
                onClick={(e) => e.stopPropagation()}
                onChange={(e) => update({ ...r, enabled: e.target.checked })}
              />
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium">{r.name || r.url || "Untitled rule"}</div>
                <div className={`truncate text-[11px] ${r.id === selected ? "opacity-80" : "text-neutral-500"}`}>
                  {r.method && `${r.method} · `}
                  {ruleSummary(tab, r)}
                </div>
              </div>
              <div className="hidden gap-0.5 group-focus-within:flex group-hover:flex">
                <button aria-label="Move up" disabled={i === 0} onClick={(e) => (e.stopPropagation(), move(r.id, -1))}>
                  ↑
                </button>
                <button
                  aria-label="Move down"
                  disabled={i === list.length - 1}
                  onClick={(e) => (e.stopPropagation(), move(r.id, 1))}
                >
                  ↓
                </button>
                <button aria-label="Delete" onClick={(e) => (e.stopPropagation(), remove(r.id))}>
                  ✕
                </button>
              </div>
            </div>
          ))}
          <button className={`${button} mt-1 self-start`} onClick={add}>
            + Add rule
          </button>
        </div>
        <div className="min-w-0 flex-1 border-l border-neutral-200 pl-3 dark:border-neutral-800">
          {current ? (
            <RuleForm key={current.id} kind={tab} rule={current} onChange={update} />
          ) : (
            <p className="text-neutral-500">{help[tab]}</p>
          )}
        </div>
      </div>
    </Modal>
  );
}

const help: Record<RuleKind, string> = {
  mapLocal: "Answer matching requests with a local file or an inline body, without contacting the server.",
  mapRemote: "Send matching requests to another host, for example a local or staging server.",
  rewrite: "Change headers, bodies or the status code of matching requests and responses.",
  block: "Refuse matching requests with 403 Forbidden.",
  breakpoints: "Pause matching requests or responses so you can edit them before they continue.",
};

function Field({ label, children, hint }: { label: string; children: React.ReactNode; hint?: string }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-neutral-500">{label}</span>
      {children}
      {hint && <span className="text-[11px] text-neutral-400">{hint}</span>}
    </label>
  );
}

function MatcherFields({ rule, onChange }: { rule: Matcher; onChange: (m: Matcher) => void }) {
  return (
    <>
      <Field label="Name (optional)">
        <input className={input} value={rule.name} onChange={(e) => onChange({ ...rule, name: e.target.value })} />
      </Field>
      <div className="flex gap-2">
        <Field label="Method">
          <select className={input} value={rule.method} onChange={(e) => onChange({ ...rule, method: e.target.value })}>
            {METHODS.map((m) => (
              <option key={m} value={m}>
                {m || "Any"}
              </option>
            ))}
          </select>
        </Field>
        <div className="flex-1">
          <Field
            label="URL"
            hint={
              rule.regex
                ? "Regular expression, searched anywhere in the URL."
                : "* matches anything, ? one character. A bare host like *.example.com matches every path on it."
            }
          >
            <input
              className={input}
              spellCheck={false}
              placeholder="https://api.example.com/v1/*"
              value={rule.url}
              onChange={(e) => onChange({ ...rule, url: e.target.value })}
            />
          </Field>
        </div>
      </div>
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={rule.regex} onChange={(e) => onChange({ ...rule, regex: e.target.checked })} />
        Use a regular expression
      </label>
    </>
  );
}

function RuleForm({ kind, rule, onChange }: { kind: RuleKind; rule: AnyRule; onChange: (r: AnyRule) => void }) {
  return (
    <div className="flex flex-col gap-3">
      <MatcherFields rule={rule} onChange={(m) => onChange({ ...rule, ...m })} />
      <div className="h-px bg-neutral-200 dark:bg-neutral-800" />
      {kind === "mapLocal" && <MapLocalFields rule={rule as MapLocalRule} onChange={onChange} />}
      {kind === "mapRemote" && (
        <Field label="Send to" hint="Scheme and host are replaced; the path and query are kept unless you give new ones.">
          <input
            className={input}
            spellCheck={false}
            placeholder="http://localhost:3000"
            value={(rule as MapRemoteRule).to}
            onChange={(e) => onChange({ ...(rule as MapRemoteRule), to: e.target.value })}
          />
        </Field>
      )}
      {kind === "rewrite" && <RewriteFields rule={rule as RewriteRule} onChange={onChange} />}
      {kind === "breakpoints" && (
        <div className="flex gap-4">
          {(["request", "response"] as const).map((p) => (
            <label key={p} className="flex items-center gap-2 capitalize">
              <input
                type="checkbox"
                checked={(rule as BreakpointRule)[p]}
                onChange={(e) => onChange({ ...(rule as BreakpointRule), [p]: e.target.checked })}
              />
              Pause on {p}
            </label>
          ))}
        </div>
      )}
      {kind === "block" && <p className="text-neutral-500">Matching requests get 403 Forbidden.</p>}
    </div>
  );
}

function MapLocalFields({ rule, onChange }: { rule: MapLocalRule; onChange: (r: MapLocalRule) => void }) {
  const [headers, setHeaders] = useState(rule.headers.map(([k, v]) => `${k}: ${v}`).join("\n"));
  const [source, setSource] = useState<"inline" | "file">(rule.file ? "file" : "inline");
  return (
    <>
      <Field label="Status">
        <input
          type="number"
          className={`${input} w-24`}
          value={rule.status}
          onChange={(e) => onChange({ ...rule, status: Number(e.target.value) || 200 })}
        />
      </Field>
      <Field label="Headers (one “Name: value” per line)">
        <textarea
          rows={4}
          spellCheck={false}
          className={`${input} resize-y`}
          value={headers}
          onChange={(e) => {
            setHeaders(e.target.value);
            onChange({ ...rule, headers: parseHeaders(e.target.value) });
          }}
        />
      </Field>
      <div className="flex gap-4">
        {(["inline", "file"] as const).map((s) => (
          <label key={s} className="flex items-center gap-2">
            <input
              type="radio"
              checked={source === s}
              onChange={() => {
                setSource(s);
                if (s === "inline") onChange({ ...rule, file: "" });
              }}
            />
            {s === "inline" ? "Inline body" : "Local file"}
          </label>
        ))}
      </div>
      {source === "file" ? (
        <div className="flex gap-2">
          <input
            className={`${input} flex-1`}
            spellCheck={false}
            placeholder="/Users/me/mocks/users.json"
            value={rule.file}
            onChange={(e) => onChange({ ...rule, file: e.target.value })}
          />
          <button
            className={button}
            onClick={async () => {
              const path = await open({ multiple: false, directory: false });
              if (path) onChange({ ...rule, file: path });
            }}
          >
            Choose…
          </button>
        </div>
      ) : (
        <textarea
          rows={10}
          spellCheck={false}
          className={`${input} resize-y`}
          value={rule.body}
          onChange={(e) => onChange({ ...rule, body: e.target.value })}
        />
      )}
    </>
  );
}

function RewriteFields({ rule, onChange }: { rule: RewriteRule; onChange: (r: RewriteRule) => void }) {
  const actions = actionsFor(rule.target);
  const isHeader = rule.target.endsWith("Header");
  const needsFind = rule.action === "replace" || (rule.action === "remove" && isHeader);
  const needsValue = rule.action !== "remove";
  return (
    <>
      <div className="flex gap-2">
        <Field label="What">
          <select
            className={input}
            value={rule.target}
            onChange={(e) => {
              const target = e.target.value as RewriteRule["target"];
              const action = actionsFor(target).includes(rule.action) ? rule.action : "set";
              onChange({ ...rule, target, action });
            }}
          >
            <option value="requestHeader">Request header</option>
            <option value="requestBody">Request body</option>
            <option value="responseHeader">Response header</option>
            <option value="responseBody">Response body</option>
            <option value="responseStatus">Response status</option>
          </select>
        </Field>
        <Field label="Action">
          <select
            className={input}
            value={rule.action}
            onChange={(e) => onChange({ ...rule, action: e.target.value as RewriteRule["action"] })}
          >
            {actions.map((a) => (
              <option key={a} value={a}>
                {{ set: "Set", add: "Add", remove: "Remove", replace: "Find and replace" }[a]}
              </option>
            ))}
          </select>
        </Field>
        {isHeader && (
          <div className="flex-1">
            <Field label="Header name">
              <input className={input} value={rule.header} onChange={(e) => onChange({ ...rule, header: e.target.value })} />
            </Field>
          </div>
        )}
      </div>
      {needsFind && (
        <Field label={rule.action === "remove" ? "Only values matching (regex, optional)" : "Find (regex)"}>
          <input
            className={input}
            spellCheck={false}
            value={rule.find}
            onChange={(e) => onChange({ ...rule, find: e.target.value })}
          />
        </Field>
      )}
      {needsValue && (
        <Field
          label={rule.action === "replace" ? "Replace with ($1 inserts a group)" : rule.target === "responseStatus" ? "Status code" : "Value"}
        >
          {rule.target.endsWith("Body") ? (
            <textarea
              rows={6}
              spellCheck={false}
              className={`${input} resize-y`}
              value={rule.value}
              onChange={(e) => onChange({ ...rule, value: e.target.value })}
            />
          ) : (
            <input className={input} spellCheck={false} value={rule.value} onChange={(e) => onChange({ ...rule, value: e.target.value })} />
          )}
        </Field>
      )}
    </>
  );
}
