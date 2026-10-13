// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type FlowState = "inProgress" | "completed" | "failed" | "webSocket";

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
}

export interface BodyView {
  size: number;
  truncated: boolean;
  text: string | null;
  base64: string | null;
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
}

export interface RestoreReport {
  restored: string[];
  skipped: string[];
}

export type FlowBatch =
  | { type: "upserts"; flows: FlowMeta[] }
  | { type: "cleared" }
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
  subscribeFlows: (onBatch: (b: FlowBatch) => void) => {
    const channel = new Channel<FlowBatch>();
    channel.onmessage = onBatch;
    return invoke<void>("subscribe_flows", { onEvent: channel });
  },
  /** Fires when the tray (or another command) changes proxy state. */
  onStatusChanged: (cb: () => void) => listen("status-changed", cb),
};

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
