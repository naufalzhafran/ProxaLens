// SSL Proxying, Block List, Allow List and Throttle settings.
import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type HostList, type McpInfo, type Settings } from "../api";
import { copy, useToast } from "../lib/clipboard";
import { guardedClose } from "../lib/confirm";
import { useSettings, useUi, type SettingsTab } from "../state/ui";
import { button, input, Modal, primaryButton, Tabs } from "./ui";

/** Mirrors `settings::preset` in the engine: bytes/s and latency (ms). */
export const THROTTLE_PRESETS: Record<string, { label: string; bytesPerSec: number; latencyMs: number }> = {
  "3g": { label: "3G", bytesPerSec: 93_750, latencyMs: 300 },
  "4g": { label: "4G", bytesPerSec: 500_000, latencyMs: 100 },
  wifi: { label: "Wi-Fi", bytesPerSec: 3_750_000, latencyMs: 20 },
};

type Tab = SettingsTab;

const lines = (text: string) =>
  text
    .split(/[\n,]/)
    .map((l) => l.trim())
    .filter(Boolean);

export function SettingsDialog({ initialTab = "ssl" }: { initialTab?: Tab }) {
  const close = useUi((s) => s.close);
  const [settings, setSettings] = useState<Settings | null>(null);
  // What's saved, to tell whether closing would lose edits.
  const [saved, setSaved] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>(initialTab);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setSaved(JSON.stringify(s));
      })
      .catch((e) => setError(errorMessage(e)));
  }, []);

  const dirty = useMemo(() => settings !== null && JSON.stringify(settings) !== saved, [settings, saved]);
  const cancel = guardedClose(dirty, close, "settings");

  async function save() {
    if (!settings) return;
    setBusy(true);
    try {
      await useSettings.getState().save(settings);
      useToast.getState().show("Settings saved");
      close();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Settings"
      onClose={cancel}
      footer={
        <>
          {error && <span className="mr-auto text-red-500">{error}</span>}
          <button className={button} onClick={cancel}>
            Cancel
          </button>
          <button className={primaryButton} disabled={!settings || busy} onClick={save}>
            Save
          </button>
        </>
      }
    >
      <Tabs<Tab>
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "general", label: "General" },
          { id: "ssl", label: "SSL Proxying" },
          { id: "block", label: "Block List" },
          { id: "allow", label: "Allow List" },
          { id: "throttle", label: "Throttle" },
          { id: "protobuf", label: "Protobuf" },
        ]}
      />
      {settings && (
        <div className="pt-3">
          {tab === "general" && (
            <div className="flex flex-col gap-3">
              <Help>How “Proxy On” points this Mac at the proxy.</Help>
              {(
                [
                  ["manual", "HTTP and HTTPS proxy settings", "Works with nearly every app. If the app is killed, your settings are restored by the watchdog."],
                  ["pac", "Automatic configuration (PAC)", "If the app dies, most apps go direct by themselves. Some apps ignore PAC files, so their traffic may not be captured."],
                ] as const
              ).map(([id, title, help]) => (
                <label key={id} className="flex items-start gap-2">
                  <input
                    type="radio"
                    className="mt-1"
                    checked={settings.systemProxyMode === id}
                    onChange={() => setSettings({ ...settings, systemProxyMode: id })}
                  />
                  <span>
                    <span className="font-medium">{title}</span>
                    <span className="block text-neutral-500">{help}</span>
                  </span>
                </label>
              ))}
              <McpSection settings={settings} setSettings={setSettings} />
            </div>
          )}
          {tab === "ssl" && (
            <div className="flex flex-col gap-3">
              <Help>
                Which HTTPS hosts are decrypted. Hosts that aren’t are passed through untouched and listed as a single
                CONNECT row, which is what you want for apps that pin their certificates. Patterns: <Code>example.com</Code>,{" "}
                <Code>*.example.com</Code> (subdomains), <Code>*</Code>.
              </Help>
              <HostsField
                label="Only decrypt these hosts (leave empty to decrypt everything)"
                hosts={settings.sslProxying.include}
                onChange={(include) => setSettings({ ...settings, sslProxying: { ...settings.sslProxying, include } })}
              />
              <HostsField
                label="Never decrypt these hosts"
                hosts={settings.sslProxying.exclude}
                onChange={(exclude) => setSettings({ ...settings, sslProxying: { ...settings.sslProxying, exclude } })}
              />
            </div>
          )}
          {tab === "block" && (
            <ListEditor
              help="Requests to these hosts are refused with 403 Forbidden and never reach the server."
              list={settings.blockList}
              onChange={(blockList) => setSettings({ ...settings, blockList })}
            />
          )}
          {tab === "allow" && (
            <ListEditor
              help="Only traffic to these hosts is recorded (and decrypted). Everything else still works but isn’t shown."
              list={settings.allowList}
              onChange={(allowList) => setSettings({ ...settings, allowList })}
            />
          )}
          {tab === "throttle" && <ThrottleEditor settings={settings} setSettings={setSettings} />}
          {tab === "protobuf" && <ProtobufEditor settings={settings} setSettings={setSettings} />}
        </div>
      )}
    </Modal>
  );
}

function Help({ children }: { children: React.ReactNode }) {
  return <p className="text-neutral-600 dark:text-neutral-400">{children}</p>;
}

function Code({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-neutral-100 px-1 font-mono dark:bg-neutral-800">{children}</code>;
}

function HostsField({ label, hosts, onChange }: { label: string; hosts: string[]; onChange: (h: string[]) => void }) {
  // Keep the raw text while typing; parse it into hosts on every change.
  const [text, setText] = useState(hosts.join("\n"));
  // Follow changes made outside the textarea (e.g. "Add files…").
  useEffect(() => {
    if (lines(text).join("\n") !== hosts.join("\n")) setText(hosts.join("\n"));
  }, [hosts]);
  return (
    <label className="flex flex-col gap-1">
      <span className="text-neutral-500">{label}</span>
      <textarea
        rows={6}
        spellCheck={false}
        placeholder={"api.example.com\n*.example.org"}
        className={`${input} resize-y`}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          onChange(lines(e.target.value));
        }}
      />
    </label>
  );
}

function ListEditor({ help, list, onChange }: { help: string; list: HostList; onChange: (l: HostList) => void }) {
  return (
    <div className="flex flex-col gap-3">
      <Help>{help}</Help>
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={list.enabled} onChange={(e) => onChange({ ...list, enabled: e.target.checked })} />
        Enabled
      </label>
      <HostsField label="Hosts, one per line" hosts={list.hosts} onChange={(hosts) => onChange({ ...list, hosts })} />
    </div>
  );
}

function ThrottleEditor({ settings, setSettings }: { settings: Settings; setSettings: (s: Settings) => void }) {
  const t = settings.throttle;
  const update = (patch: Partial<Settings["throttle"]>) =>
    setSettings({ ...settings, throttle: { ...t, ...patch } });
  return (
    <div className="flex flex-col gap-3">
      <Help>Slows responses down to simulate a slower network. Applies to every host unless you list some.</Help>
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={t.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
        Enabled
      </label>
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Preset</span>
          <select
            className={input}
            value={t.preset in THROTTLE_PRESETS ? t.preset : "custom"}
            onChange={(e) => {
              const p = THROTTLE_PRESETS[e.target.value];
              update(p ? { preset: e.target.value, bytesPerSec: p.bytesPerSec, latencyMs: p.latencyMs } : { preset: "custom" });
            }}
          >
            {Object.entries(THROTTLE_PRESETS).map(([id, p]) => (
              <option key={id} value={id}>
                {p.label}
              </option>
            ))}
            <option value="custom">Custom</option>
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Bandwidth (kbit/s, 0 = unlimited)</span>
          <input
            type="number"
            min={0}
            className={`${input} w-40`}
            value={Math.round((t.bytesPerSec * 8) / 1000)}
            onChange={(e) => update({ preset: "custom", bytesPerSec: Math.max(0, Math.round((Number(e.target.value) * 1000) / 8)) })}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Latency (ms)</span>
          <input
            type="number"
            min={0}
            className={`${input} w-28`}
            value={t.latencyMs}
            onChange={(e) => update({ preset: "custom", latencyMs: Math.max(0, Math.round(Number(e.target.value))) })}
          />
        </label>
      </div>
      <HostsField label="Only throttle these hosts (leave empty for all)" hosts={t.hosts} onChange={(hosts) => update({ hosts })} />
    </div>
  );
}

function ProtobufEditor({ settings, setSettings }: { settings: Settings; setSettings: (s: Settings) => void }) {
  const list = settings.protobuf;
  const set = (protobuf: Settings["protobuf"]) => setSettings({ ...settings, protobuf });
  return (
    <div className="flex flex-col gap-3">
      <Help>
        Protobuf and gRPC bodies are always shown with field numbers. Add your <Code>.proto</Code> files (or folders of
        them) to decode gRPC calls by name. The first entry whose host pattern matches is used.
      </Help>
      {list.map((entry, i) => (
        <div key={i} className="flex flex-col gap-2 rounded-md border border-neutral-200 p-2 dark:border-neutral-800">
          <div className="flex items-end gap-2">
            <label className="flex flex-1 flex-col gap-1">
              <span className="text-neutral-500">Hosts</span>
              <input
                className={input}
                spellCheck={false}
                placeholder="*.example.com or *"
                value={entry.hosts}
                onChange={(e) => set(list.map((x, j) => (j === i ? { ...x, hosts: e.target.value } : x)))}
              />
            </label>
            <button className={button} onClick={() => set(list.filter((_, j) => j !== i))}>
              Remove
            </button>
          </div>
          <HostsField
            label=".proto files or folders, one per line"
            hosts={entry.paths}
            onChange={(paths) => set(list.map((x, j) => (j === i ? { ...x, paths } : x)))}
          />
          <button
            className={`${button} self-start`}
            onClick={async () => {
              const picked = await open({ multiple: true, directory: false, filters: [{ name: "Protocol Buffers", extensions: ["proto"] }] });
              const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
              if (paths.length) set(list.map((x, j) => (j === i ? { ...x, paths: [...x.paths, ...paths] } : x)));
            }}
          >
            Add files…
          </button>
        </div>
      ))}
      <button className={`${button} self-start`} onClick={() => set([...list, { hosts: "*", paths: [] }])}>
        + Add .proto source
      </button>
    </div>
  );
}

function McpSection({ settings, setSettings }: { settings: Settings; setSettings: (s: Settings) => void }) {
  const [info, setInfo] = useState<McpInfo | null>(null);
  useEffect(() => {
    api.mcpInfo().then(setInfo).catch(() => {});
  }, []);
  const mcp = settings.mcp;
  return (
    <div className="mt-2 flex flex-col gap-2 border-t border-neutral-200 pt-3 dark:border-neutral-800">
      <h3 className="font-medium">MCP server for AI agents</h3>
      <Help>
        Lets agents such as Claude Code list, search and replay captured requests, add mocks and manage scripts. It listens
        on 127.0.0.1 only and needs the token below.
      </Help>
      <div className="flex items-center gap-4">
        <label className="flex items-center gap-2">
          <input type="checkbox" checked={mcp.enabled} onChange={(e) => setSettings({ ...settings, mcp: { ...mcp, enabled: e.target.checked } })} />
          Enabled
        </label>
        <label className="flex items-center gap-2">
          Port
          <input
            type="number"
            className={`${input} w-24`}
            value={mcp.port}
            onChange={(e) => setSettings({ ...settings, mcp: { ...mcp, port: Number(e.target.value) || 9091 } })}
          />
        </label>
      </div>
      {info?.claudeCommand ? (
        <div className="flex items-center gap-2">
          <code className="selectable min-w-0 flex-1 truncate rounded bg-neutral-100 px-2 py-1 font-mono text-[11px] dark:bg-neutral-800" title={info.claudeCommand}>
            {info.claudeCommand}
          </code>
          <button className={button} onClick={() => copy(info.claudeCommand!, "command")}>
            Copy
          </button>
        </div>
      ) : (
        <p className="text-neutral-500">Save with “Enabled” on to start it and get the connection command.</p>
      )}
    </div>
  );
}
