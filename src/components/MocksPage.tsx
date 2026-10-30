// The Mocks page: APIs picked from traffic, each with a fake response the
// proxy serves while the mock is on, and the calls the API received.
import { memo, useEffect, useMemo, useState } from "react";
import { ExternalLink, Trash2 } from "lucide-react";
import { api, errorMessage, type FlowDetail, type FlowMeta, type Mock } from "../api";
import { draftToRequest, parseHeaders } from "../lib/actions";
import { useToast } from "../lib/clipboard";
import { confirmAction } from "../lib/confirm";
import { formatDuration, formatTime, methodHue, statusColor } from "../lib/format";
import { callsByMock, draftOf, prettyJson, wasMocked } from "../lib/mocks";
import { useFlows } from "../state/flows";
import { useMocks } from "../state/mocks";
import { useLayout, type Draft } from "../state/ui";
import { ResponsePane } from "./Inspector";
import { RequestForm } from "./RequestForm";
import { button, HUE_BADGE, input, primaryButton, Tabs } from "./ui";

const toast = (m: string) => useToast.getState().show(m);

export function MocksPage() {
  const mocks = useMocks((s) => s.mocks);
  const selectedId = useMocks((s) => s.selectedId);
  const seen = useMocks((s) => s.seen);
  const order = useFlows((s) => s.order);
  const byId = useFlows((s) => s.byId);
  const calls = useMemo(() => callsByMock(order, byId), [order, byId]);
  const selected = mocks?.find((m) => m.id === selectedId) ?? null;

  useEffect(() => {
    if (useMocks.getState().mocks === null) useMocks.getState().load().catch((e) => toast(errorMessage(e)));
  }, []);

  // Calls of the mock on screen count as seen.
  const newest = selected ? calls.get(selected.id)?.[0] : undefined;
  useEffect(() => {
    if (selected && newest !== undefined) useMocks.getState().markSeen(selected.id, newest);
  }, [selected, newest]);

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex w-72 shrink-0 flex-col border-r border-line bg-surface">
        <div className="flex items-center justify-between px-3 py-2 text-xs font-semibold text-muted">
          <span>Mocked APIs</span>
          {mocks && mocks.length > 0 && <span className="font-normal">{mocks.length}</span>}
        </div>
        {mocks === null ? (
          <p className="px-3 text-xs text-muted">Loading…</p>
        ) : mocks.length === 0 ? (
          <NoMocks />
        ) : (
          <div className="min-h-0 flex-1 overflow-auto pb-2">
            {mocks.map((m) => (
              <MockRow
                key={m.id}
                mock={m}
                calls={calls.get(m.id)?.length ?? 0}
                unseen={(calls.get(m.id) ?? []).filter((id) => id > (seen[m.id] ?? 0)).length}
                selected={m.id === selectedId}
              />
            ))}
          </div>
        )}
      </div>
      {selected ? (
        <>
          <div className="flex min-w-0 flex-1 flex-col bg-panel">
            <Editor key={selected.id} mock={selected} />
          </div>
          <div className="flex w-[440px] shrink-0 flex-col border-l border-line bg-panel">
            <Calls mock={selected} ids={calls.get(selected.id) ?? []} byId={byId} />
          </div>
        </>
      ) : (
        <div className="flex flex-1 items-center justify-center bg-panel p-6 text-center text-xs text-muted">
          {mocks?.length ? "Select an API to edit its mock response." : ""}
        </div>
      )}
    </div>
  );
}

export function NoMocks() {
  const setPage = useLayout((s) => s.setPage);
  return (
    <div className="flex flex-col gap-2 px-3 text-xs text-muted">
      <p>No APIs yet.</p>
      <p>
        In{" "}
        <button className="underline" onClick={() => setPage("traffic")}>
          Traffic
        </button>
        , right-click a request and choose “Mock This API…”. It appears here with its real response, ready to
        change.
      </p>
      <p>While a mock is on, the proxy answers that API with your response instead of the server.</p>
    </div>
  );
}

export const MockRow = memo(function MockRow({
  mock,
  calls,
  unseen,
  selected,
}: {
  mock: Mock;
  calls: number;
  unseen: number;
  selected: boolean;
}) {
  const select = useMocks((s) => s.select);
  const updateNow = useMocks((s) => s.updateNow);
  const target = mock.request.url || mock.url;
  return (
    <div
      role="button"
      tabIndex={0}
      aria-current={selected || undefined}
      onClick={() => select(mock.id)}
      onKeyDown={(e) => e.key === "Enter" && select(mock.id)}
      className={`mx-2 flex cursor-default items-center gap-2 rounded-lg px-2 py-1.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        selected ? "bg-accent text-on-accent" : "hover:bg-ink/6"
      }`}
    >
      <span
        className={`shrink-0 rounded px-1 py-px text-[10px] font-semibold ${selected ? "bg-on-accent/20" : HUE_BADGE[methodHue(mock.method)]}`}
      >
        {mock.method || "ANY"}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium" title={mock.name}>
          {mock.name || mock.url}
        </span>
        <span className={`block truncate font-mono text-[10px] ${selected ? "text-on-accent/70" : "text-muted"}`} title={target}>
          {target}
        </span>
      </span>
      {unseen > 0 && (
        <span
          title={`${unseen} new call${unseen === 1 ? "" : "s"}`}
          className={`shrink-0 rounded-full px-1.5 text-[10px] font-semibold ${selected ? "bg-on-accent text-accent" : "bg-accent text-on-accent"}`}
        >
          {unseen}
        </span>
      )}
      {calls > 0 && unseen === 0 && (
        <span className={`shrink-0 text-[10px] ${selected ? "text-on-accent/70" : "text-muted"}`}>{calls}</span>
      )}
      <Switch
        on={mock.enabled}
        label={mock.enabled ? "Mock is on: the proxy serves this response" : "Mock is off: the server answers"}
        onChange={(enabled) => updateNow(mock.id, { enabled })}
        inverted={selected}
      />
    </div>
  );
});

function Switch({
  on,
  label,
  onChange,
  inverted = false,
}: {
  on: boolean;
  label: string;
  onChange: (on: boolean) => void;
  inverted?: boolean;
}) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!on);
      }}
      className={`relative h-4 w-7 shrink-0 rounded-full transition-colors ${
        on ? (inverted ? "bg-on-accent" : "bg-accent") : inverted ? "bg-on-accent/30" : "bg-ink/20"
      }`}
    >
      <span
        className={`absolute top-0.5 left-0.5 h-3 w-3 rounded-full shadow-thumb transition-transform ${
          on ? "translate-x-3" : ""
        } ${on && inverted ? "bg-accent" : "bg-panel"}`}
      />
    </button>
  );
}

type EditorTab = "response" | "request";

export function Editor({ mock }: { mock: Mock }) {
  const update = useMocks((s) => s.update);
  const updateNow = useMocks((s) => s.updateNow);
  const remove = useMocks((s) => s.remove);
  const [tab, setTab] = useState<EditorTab>("response");

  async function del() {
    if (await confirmAction(`Delete the mock for ${mock.name || mock.url}? Its calls stay in Traffic.`, "Delete")) {
      await remove(mock.id);
    }
  }

  return (
    <>
      <div className="flex flex-col gap-2 border-b border-line px-3 py-2.5 text-xs">
        <div className="flex items-center gap-2">
          <input
            aria-label="Name"
            className={`${input} min-w-0 flex-1 font-sans font-medium`}
            value={mock.name}
            placeholder="Name"
            onChange={(e) => update(mock.id, { name: e.target.value })}
          />
          <label className="flex items-center gap-1.5 text-muted">
            <Switch
              on={mock.enabled}
              label={mock.enabled ? "Mock is on" : "Mock is off"}
              onChange={(enabled) => updateNow(mock.id, { enabled })}
            />
            {mock.enabled ? <span className="text-ink">Serving mock</span> : "Off: server answers"}
          </label>
          <button className={`${button} flex items-center gap-1`} title="Delete this mock" onClick={del}>
            <Trash2 size={13} />
            Delete
          </button>
        </div>
        <div className="flex items-center gap-2">
          <input
            aria-label="Method to match"
            className={`${input} w-20 uppercase`}
            value={mock.method}
            placeholder="ANY"
            onChange={(e) => update(mock.id, { method: e.target.value.toUpperCase() })}
          />
          <input
            aria-label="URL pattern"
            spellCheck={false}
            className={`${input} min-w-0 flex-1`}
            value={mock.url}
            placeholder="https://api.example.com/users/*"
            onChange={(e) => update(mock.id, { url: e.target.value })}
          />
          <label className="flex items-center gap-1 text-muted">
            <input type="checkbox" checked={mock.regex} onChange={(e) => update(mock.id, { regex: e.target.checked })} />
            regex
          </label>
        </div>
        <p className="text-muted">
          Requests matching this method and URL pattern (<code>*</code> any text, <code>?</code> one character) are
          calls of this API.
        </p>
      </div>
      <Tabs<EditorTab>
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "response", label: "Mock Response" },
          { id: "request", label: "Request" },
        ]}
      />
      <div className="min-h-0 flex-1 overflow-auto p-3 text-xs">
        {tab === "response" ? <ResponseEditor mock={mock} /> : <RequestEditor mock={mock} />}
      </div>
    </>
  );
}

function ResponseEditor({ mock }: { mock: Mock }) {
  const update = useMocks((s) => s.update);
  const r = mock.response;
  const set = (patch: Partial<Mock["response"]>) => update(mock.id, { response: { ...r, ...patch } });
  // Typed as text so a half-written line isn't reshaped under the cursor.
  const [headers, setHeaders] = useState(() => r.headers.map(([k, v]) => `${k}: ${v}`).join("\n"));
  const isJson = r.headers.some(([k, v]) => k.toLowerCase() === "content-type" && v.includes("json"));

  return (
    <div className="flex flex-col gap-3">
      <div className="flex gap-4">
        <label className="flex items-center gap-2">
          <span className="text-muted">Status</span>
          <input
            type="number"
            min={100}
            max={999}
            className={`${input} w-20`}
            value={r.status}
            onChange={(e) => set({ status: Number(e.target.value) || 200 })}
          />
        </label>
        <label className="flex items-center gap-2">
          <span className="text-muted">Delay (ms)</span>
          <input
            type="number"
            min={0}
            step={100}
            className={`${input} w-24`}
            value={r.delayMs}
            onChange={(e) => set({ delayMs: Math.max(0, Number(e.target.value) || 0) })}
          />
        </label>
      </div>
      <label className="flex flex-col gap-1">
        <span className="text-muted">Headers (one “Name: value” per line)</span>
        <textarea
          spellCheck={false}
          rows={5}
          className={`${input} resize-y`}
          value={headers}
          onChange={(e) => {
            setHeaders(e.target.value);
            set({ headers: parseHeaders(e.target.value) });
          }}
        />
      </label>
      <label className="flex flex-col gap-1">
        <span className="flex items-center text-muted">
          Body
          {isJson && (
            <button className={`${button} ml-auto`} onClick={() => set({ body: prettyJson(r.body) })}>
              Format JSON
            </button>
          )}
        </span>
        <textarea
          spellCheck={false}
          rows={18}
          className={`${input} resize-y`}
          value={r.body}
          onChange={(e) => set({ body: e.target.value })}
        />
      </label>
    </div>
  );
}

function RequestEditor({ mock }: { mock: Mock }) {
  const update = useMocks((s) => s.update);
  const [draft, setDraft] = useState<Draft>(() => draftOf(mock));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function change(d: Draft) {
    setDraft(d);
    update(mock.id, {
      method: d.method.trim().toUpperCase(),
      request: {
        url: d.url,
        headers: parseHeaders(d.headers),
        body: d.binaryBody ? "" : d.body,
        bodyBase64: d.binaryBody ? mock.request.bodyBase64 : "",
      },
    });
  }

  async function send() {
    setBusy(true);
    setError(null);
    try {
      await api.sendRequest(draftToRequest(draft));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <RequestForm compact draft={draft} onChange={change} onSubmit={send} />
      <div className="flex items-center gap-2">
        <span className="text-muted">
          {mock.enabled ? "The mock answers this call." : "The server answers this call."} It appears under Calls.
        </span>
        {error && <span className="text-st-server">{error}</span>}
        <button className={`${primaryButton} ml-auto`} disabled={busy || !draft.url.trim()} onClick={send}>
          {busy ? "Sending…" : "Send (⌘↩)"}
        </button>
      </div>
    </div>
  );
}

export function Calls({ mock, ids, byId }: { mock: Mock; ids: number[]; byId: Record<number, FlowMeta> }) {
  const [picked, setPicked] = useState<number | null>(null);
  const shown = picked !== null && ids.includes(picked) ? picked : (ids[0] ?? null);
  return (
    <>
      <div className="flex items-center justify-between border-b border-line px-3 py-2 text-xs">
        <span className="font-semibold text-muted">Calls</span>
        <span className="text-muted">{ids.length}</span>
      </div>
      <div className="max-h-[40%] min-h-24 shrink-0 overflow-auto text-xs">
        {ids.length === 0 ? (
          <p className="px-3 py-2 text-muted">
            No calls yet. When an app calls {mock.method || "this"} {mock.url}, it shows up here.
          </p>
        ) : (
          ids.map((id) => <CallRow key={id} f={byId[id]} selected={id === shown} onClick={() => setPicked(id)} />)
        )}
      </div>
      <div className="flex min-h-0 flex-1 flex-col border-t border-line">
        {shown !== null && <CallDetail id={shown} />}
      </div>
    </>
  );
}

const CallRow = memo(function CallRow({ f, selected, onClick }: { f: FlowMeta; selected: boolean; onClick: () => void }) {
  const mocked = wasMocked(f);
  const label = f.state === "failed" ? "ERR" : f.state === "blocked" ? "BLK" : (f.status?.toString() ?? "…");
  return (
    <button
      aria-current={selected || undefined}
      onClick={onClick}
      className={`flex w-full items-center gap-2 px-3 py-1 text-left font-mono ${
        selected ? "bg-accent text-on-accent" : "hover:bg-ink/6"
      }`}
    >
      <span className={`w-16 shrink-0 ${selected ? "" : "text-muted"}`}>{formatTime(f.startedAt, false)}</span>
      <span className={`w-8 shrink-0 ${selected ? "" : statusColor(f.status)}`}>{label}</span>
      <span className={`w-14 shrink-0 text-right ${selected ? "" : "text-muted"}`}>{formatDuration(f.durationMs)}</span>
      <span className={`min-w-0 flex-1 truncate ${selected ? "text-on-accent/80" : "text-muted"}`} title={f.url}>
        {f.process ?? f.clientAddr}
      </span>
      <span
        className={`shrink-0 rounded-full px-1.5 text-[10px] font-sans font-medium ${
          selected ? "bg-on-accent/20" : mocked ? "bg-accent/14 text-accent" : "bg-ink/8 text-muted"
        }`}
      >
        {mocked ? "Mocked" : "Real"}
      </span>
    </button>
  );
});

function CallDetail({ id }: { id: number }) {
  const meta = useFlows((s) => s.byId[id]);
  const setPage = useLayout((s) => s.setPage);
  const [detail, setDetail] = useState<FlowDetail | null>(null);
  useEffect(() => {
    let cancelled = false;
    api
      .getFlow(id)
      .then((d) => !cancelled && setDetail(d))
      .catch((e) => !cancelled && toast(errorMessage(e)));
    return () => {
      cancelled = true;
    };
  }, [id, meta]);

  function openInTraffic() {
    useFlows.getState().select(id);
    setPage("traffic");
  }

  if (!detail || detail.meta.id !== id) return null;
  return (
    <>
      <div className="flex items-center gap-2 px-3 py-1.5 font-mono text-xs">
        <span className={`rounded px-1 py-px text-[11px] font-semibold ${HUE_BADGE[methodHue(detail.meta.method)]}`}>
          {detail.meta.method}
        </span>
        <span className="min-w-0 flex-1 truncate" title={detail.meta.url}>
          {detail.meta.url}
        </span>
        <button className={`${button} flex items-center gap-1`} onClick={openInTraffic}>
          <ExternalLink size={12} />
          Open in Traffic
        </button>
      </div>
      <ResponsePane detail={detail} />
    </>
  );
}
