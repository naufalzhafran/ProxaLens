// Scripts: list, Monaco editor, and the script console. Loaded lazily so
// Monaco stays out of the main bundle.
import { useCallback, useEffect, useRef, useState } from "react";
import Editor, { type OnMount } from "@monaco-editor/react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorMessage, type LogEntry, type ScriptInfo } from "../api";
import { ContextMenu, Modal, button, primaryButton, type MenuEntry } from "../components/ui";
import { formatTime } from "../lib/format";
import { useUi } from "../state/ui";
import { monaco } from "./monaco";
import { TEMPLATES } from "./templates";

export default function ScriptsWindow() {
  const close = useUi((s) => s.close);
  const [dir, setDir] = useState("");
  const [scripts, setScripts] = useState<ScriptInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [source, setSource] = useState("");
  const [saved, setSaved] = useState("");
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [dark, setDark] = useState(() => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false);
  const saveRef = useRef<() => void>(() => {});
  const dirty = source !== saved;

  const refresh = useCallback(async () => {
    const [list, console] = await Promise.all([api.listScripts(), api.scriptConsole()]);
    setDir(list.dir);
    setScripts(list.scripts);
    setLogs(console);
    return list.scripts;
  }, []);

  useEffect(() => {
    api
      .reloadScripts()
      .then(refresh)
      .then((list) => setSelected((s) => s ?? list[0]?.fileName ?? null))
      .catch((e) => setError(errorMessage(e)));
    const unlisten = api.onScriptsChanged(() => refresh().catch(() => {}));
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    const onScheme = (e: MediaQueryListEvent) => setDark(e.matches);
    mq?.addEventListener("change", onScheme);
    return () => {
      unlisten.then((f) => f()).catch(() => {});
      mq?.removeEventListener("change", onScheme);
    };
  }, [refresh]);

  useEffect(() => {
    if (!selected) {
      setSource("");
      setSaved("");
      return;
    }
    api
      .readScript(selected)
      .then((s) => {
        setSource(s);
        setSaved(s);
      })
      .catch((e) => setError(errorMessage(e)));
  }, [selected]);

  async function save() {
    if (!selected) return;
    try {
      await api.saveScript(selected, source);
      setSaved(source);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }
  saveRef.current = save;

  function select(fileName: string) {
    if (dirty && !window.confirm("Discard unsaved changes to this script?")) return;
    setSelected(fileName);
  }

  async function create(t: (typeof TEMPLATES)[number]) {
    try {
      const name = await api.createScript(t.name, t.matches, t.body);
      await refresh();
      setSelected(name);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function remove(s: ScriptInfo) {
    if (!window.confirm(`Delete “${s.name}”? This removes ${s.fileName} from the scripts folder.`)) return;
    try {
      await api.deleteScript(s.fileName);
      const list = await refresh();
      setSelected(list[0]?.fileName ?? null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const onMount: OnMount = (editor) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => saveRef.current());
  };

  const current = scripts.find((s) => s.fileName === selected) ?? null;
  const templateMenu: MenuEntry[] = TEMPLATES.map((t) => ({ label: t.label, onSelect: () => create(t) }));

  return (
    <Modal
      title="Scripts"
      wide
      onClose={() => {
        if (!dirty || window.confirm("Discard unsaved changes?")) close();
      }}
      footer={
        <>
          {error && <span className="mr-auto whitespace-pre-wrap text-red-500">{error}</span>}
          <span className="mr-auto truncate text-neutral-500" title={dir}>
            Scripts run in order of file name. Folder: {dir}
          </span>
          {current && (
            <button className={button} onClick={() => remove(current)}>
              Delete
            </button>
          )}
          <button className={primaryButton} disabled={!dirty} onClick={save}>
            Save (⌘S)
          </button>
        </>
      }
    >
      <div className="flex h-[65vh] min-h-[420px] gap-3">
        <div className="flex w-56 shrink-0 flex-col gap-1 overflow-auto">
          <div className="mb-1 flex gap-1">
            <button
              className={button}
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setMenu({ x: r.left, y: r.bottom + 4 });
              }}
            >
              + New ▾
            </button>
            <button className={button} title="Reload files changed outside the app" onClick={() => api.reloadScripts().catch(() => {})}>
              Reload
            </button>
            <button className={button} title="Show the scripts folder" onClick={() => dir && revealItemInDir(dir).catch(() => {})}>
              Folder
            </button>
          </div>
          {scripts.length === 0 && <p className="text-neutral-500">No scripts yet. Create one from a template.</p>}
          {scripts.map((s) => (
            <div
              key={s.fileName}
              onClick={() => select(s.fileName)}
              className={`flex cursor-default items-start gap-2 rounded-md px-2 py-1 ${
                s.fileName === selected ? "bg-sky-600 text-white" : "hover:bg-neutral-100 dark:hover:bg-neutral-800"
              }`}
            >
              <input
                type="checkbox"
                aria-label="Enabled"
                className="mt-0.5"
                disabled={!!s.error}
                checked={s.enabled}
                onClick={(e) => e.stopPropagation()}
                onChange={(e) => api.setScriptEnabled(s.fileName, e.target.checked).catch((err) => setError(errorMessage(err)))}
              />
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium">
                  {s.name}
                  {s.fileName === selected && dirty && " •"}
                </div>
                <div className={`truncate text-[11px] ${s.fileName === selected ? "opacity-80" : "text-neutral-500"}`}>
                  {s.error ? <span className="text-red-400">⚠ {s.error}</span> : s.matches.join(", ")}
                </div>
                <div className="mt-0.5 flex gap-1">
                  {s.categories.map((c) => (
                    <span key={c} className="rounded bg-black/10 px-1 text-[10px] dark:bg-white/10">
                      {c}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          ))}
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <div className="min-h-0 flex-[3] overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
            {selected ? (
              <Editor
                language="javascript"
                theme={dark ? "vs-dark" : "vs"}
                value={source}
                onChange={(v) => setSource(v ?? "")}
                onMount={onMount}
                options={{ fontSize: 12, minimap: { enabled: false }, scrollBeyondLastLine: false, tabSize: 2 }}
              />
            ) : (
              <div className="p-4 text-neutral-500">
                Scripts change traffic with JavaScript: <code>onRequest(ctx)</code> and <code>onResponse(ctx)</code>. The
                header’s <code>match</code> list picks which URLs they run for. Files use the same format as httpmon.
              </div>
            )}
          </div>
          <Console logs={logs} />
        </div>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={templateMenu} onClose={() => setMenu(null)} />}
    </Modal>
  );
}

function Console({ logs }: { logs: LogEntry[] }) {
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => end.current?.scrollIntoView({ block: "end" }), [logs.length]);
  const color = { log: "", info: "text-sky-600", warn: "text-amber-600", error: "text-red-500" };
  return (
    <div className="flex min-h-0 flex-1 flex-col rounded-md border border-neutral-200 dark:border-neutral-800">
      <div className="flex items-center border-b border-neutral-200 px-2 py-1 dark:border-neutral-800">
        <span className="flex-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-500">Console</span>
        <button className="text-[11px] text-neutral-500 hover:underline" onClick={() => api.clearScriptConsole().catch(() => {})}>
          Clear
        </button>
      </div>
      <div className="selectable min-h-0 flex-1 overflow-auto p-1 font-mono text-[11px]">
        {logs.length === 0 && <p className="p-1 text-neutral-500">console.log output and script errors appear here.</p>}
        {logs.map((l) => (
          <div key={l.id} className={`whitespace-pre-wrap break-all ${color[l.level] ?? ""}`}>
            <span className="text-neutral-400">{formatTime(l.at)}</span> <span className="text-neutral-500">[{l.script}]</span>{" "}
            {l.message}
          </div>
        ))}
        <div ref={end} />
      </div>
    </div>
  );
}
