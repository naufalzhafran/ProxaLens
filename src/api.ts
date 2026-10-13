// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type FlowState = "inProgress" | "completed" | "failed" | "webSocket" | "tunnel" | "blocked" | "paused";

export interface FlowMeta {
  id: number;
  method: string;
  scheme: string;
  host: string;
  path: string;
  url: string;
  httpVersion: string;
  status: number | null;
  state: FlowState;
  contentType: string | null;
  startedAt: number;
  ttfbMs: number | null;
  durationMs: number | null;
  requestSize: number;
  responseSize: number;
  clientAddr: string;
  error: string | null;
  wsMessageCount: number;
  /** Rules that changed this flow ("Map Local", "Rewrite", …). */
  tags: string[];
  /** The app that made the request, when known. */
  process: string | null;
  pid: number | null;
}

export interface BodyView {
  size: number;
  truncated: boolean;
  text: string | null;
  base64: string | null;
  /** Content-Encoding removed for display, e.g. "gzip". */
  encoding?: string | null;
  /** Size on the wire before decoding. */
  encodedSize?: number | null;
  decodeError?: string | null;
  /** Protobuf / gRPC body decoded to JSON. */
  protobuf?: { json: string; message: string | null; notes: string[] } | null;
}

export interface WsMessageView {
  direction: "clientToServer" | "serverToClient";
  at: number;
  kind: "text" | "binary" | "ping" | "pong" | "close";
  size: number;
  payload: BodyView;
}

export interface FlowDetail {
  meta: FlowMeta;
  requestHeaders: [string, string][];
  requestBody: BodyView;
  responseHeaders: [string, string][];
  responseBody: BodyView;
  wsMessages: WsMessageView[];
}

export interface Status {
  running: boolean;
  addr: string | null;
  port: number;
  error: string | null;
  caTrusted: boolean;
  caCertPath: string;
  caFingerprint: string;
  systemProxyEnabled: boolean;
  dataDir: string;
  /** False while recording is paused. */
  recording: boolean;
}

export interface RestoreReport {
  restored: string[];
  skipped: string[];
}

export interface HostList {
  enabled: boolean;
  hosts: string[];
}

export interface Settings {
  port: number;
  sslProxying: { include: string[]; exclude: string[] };
  blockList: HostList;
  allowList: HostList;
  throttle: {
    enabled: boolean;
    preset: string;
    bytesPerSec: number;
    latencyMs: number;
    hosts: string[];
  };
  protobuf: { hosts: string; paths: string[] }[];
  allowLan: boolean;
  systemProxyMode: "manual" | "pac";
  mcp: { enabled: boolean; port: number };
}

export interface McpInfo {
  running: boolean;
  url: string | null;
  token: string;
  claudeCommand: string | null;
}

export interface DeviceInfo {
  allowLan: boolean;
  port: number;
  /** [interface, setup URL] */
  setupUrls: [string, string][];
}

export interface OutgoingRequest {
  method: string;
  url: string;
  headers: [string, string][];
  /** Raw bytes. */
  body: number[];
}

export interface Matcher {
  id: string;
  name: string;
  enabled: boolean;
  /** "" for any method. */
  method: string;
  url: string;
  regex: boolean;
}

export interface MapLocalRule extends Matcher {
  status: number;
  headers: [string, string][];
  body: string;
  file: string;
}

export interface MapRemoteRule extends Matcher {
  to: string;
}

export type RewriteTarget = "requestHeader" | "requestBody" | "responseHeader" | "responseBody" | "responseStatus";
export type RewriteAction = "set" | "add" | "remove" | "replace";

export interface RewriteRule extends Matcher {
  target: RewriteTarget;
  action: RewriteAction;
  header: string;
  find: string;
  value: string;
}

export type BlockRule = Matcher;

export interface BreakpointRule extends Matcher {
  request: boolean;
  response: boolean;
}

export interface Rules {
  mapLocal: MapLocalRule[];
  mapRemote: MapRemoteRule[];
  rewrite: RewriteRule[];
  block: BlockRule[];
  breakpoints: BreakpointRule[];
}

export type Phase = "request" | "response";

export interface BreakpointHit {
  flowId: number;
  phase: Phase;
  method: string;
  url: string;
  status: number | null;
  headers: [string, string][];
  body: BodyView;
  /** Epoch ms when the flow continues unchanged on its own. */
  deadline: number;
}

export interface BreakpointEdits {
  method: string;
  url: string;
  status: number;
  headers: [string, string][];
  body: number[];
}

export interface ScriptInfo {
  fileName: string;
  path: string;
  id: string;
  name: string;
  matches: string[];
  enabled: boolean;
  error: string | null;
  categories: string[];
}

export interface LogEntry {
  id: number;
  at: number;
  script: string;
  level: "log" | "info" | "warn" | "error";
  message: string;
}

export type FlowBatch =
  | { type: "upserts"; flows: FlowMeta[] }
  | { type: "cleared" }
  | { type: "removed"; ids: number[] }
  | { type: "resync" };

export const api = {
  getStatus: () => invoke<Status>("get_status"),
  startProxy: (port?: number) => invoke<Status>("start_proxy", { port }),
  stopProxy: () => invoke<Status>("stop_proxy"),
  setSystemProxy: (enabled: boolean) => invoke<Status>("set_system_proxy", { enabled }),
  installCa: () => invoke<Status>("install_ca"),
  takeRestoreNotice: () => invoke<RestoreReport | null>("take_restore_notice"),
  listFlows: (offset: number, limit: number) =>
    invoke<{ flows: FlowMeta[]; total: number }>("list_flows", { offset, limit }),
  getFlow: (id: number) => invoke<FlowDetail | null>("get_flow", { id }),
  clearFlows: () => invoke<void>("clear_flows"),
  removeFlows: (ids: number[]) => invoke<number>("remove_flows", { ids }),
  setRecording: (enabled: boolean) => invoke<Status>("set_recording", { enabled }),
  subscribeFlows: (onBatch: (b: FlowBatch) => void) => {
    const channel = new Channel<FlowBatch>();
    channel.onmessage = onBatch;
    return invoke<void>("subscribe_flows", { onEvent: channel });
  },
  getSettings: () => invoke<Settings>("get_settings"),
  deviceInfo: () => invoke<DeviceInfo>("device_info"),
  mcpInfo: () => invoke<McpInfo>("mcp_info"),
  installSimulatorCert: () => invoke<string>("install_simulator_cert"),
  setSettings: (settings: Settings) => invoke<Settings>("set_settings", { settings }),
  exportHar: (path: string, ids?: number[]) => invoke<number>("export_har", { path, ids }),
  saveSession: (path: string) => invoke<number>("save_session", { path }),
  importFlows: (path: string, replace: boolean) => invoke<number>("import_flows", { path, replace }),
  repeatFlow: (id: number) => invoke<number | null>("repeat_flow", { id }),
  sendRequest: (request: OutgoingRequest) => invoke<number | null>("send_request", { request }),
  getRules: () => invoke<Rules>("get_rules"),
  setRules: (rules: Rules) => invoke<Rules>("set_rules", { rules }),
  listBreakpoints: () => invoke<BreakpointHit[]>("list_breakpoints"),
  resumeBreakpoint: (flowId: number, phase: Phase, abort: boolean, edits?: BreakpointEdits) =>
    invoke<boolean>("resume_breakpoint", { flowId, phase, abort, edits }),
  resumeAllBreakpoints: () => invoke<void>("resume_all_breakpoints"),
  onBreakpointsChanged: (cb: () => void) => listen("breakpoints-changed", cb),
  listScripts: () => invoke<{ dir: string; scripts: ScriptInfo[] }>("list_scripts"),
  reloadScripts: () => invoke<void>("reload_scripts"),
  readScript: (fileName: string) => invoke<string>("read_script", { fileName }),
  saveScript: (fileName: string, source: string) => invoke<void>("save_script", { fileName, source }),
  createScript: (name: string, matches: string, body: string) =>
    invoke<string>("create_script", { name, matches, body }),
  deleteScript: (fileName: string) => invoke<void>("delete_script", { fileName }),
  setScriptEnabled: (fileName: string, enabled: boolean) =>
    invoke<void>("set_script_enabled", { fileName, enabled }),
  scriptConsole: () => invoke<LogEntry[]>("script_console"),
  clearScriptConsole: () => invoke<void>("clear_script_console"),
  onScriptsChanged: (cb: () => void) => listen("scripts-changed", cb),
  /** Fires when the tray (or another command) changes proxy state. */
  onStatusChanged: (cb: () => void) => listen("status-changed", cb),
};

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
