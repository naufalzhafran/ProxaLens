// Flow actions shared by the context menu, inspector and toolbar.
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type FlowDetail, type OutgoingRequest } from "../api";
import { useFlows } from "../state/flows";
import { useSettings, useUi, type Draft, type RulesView } from "../state/ui";
import { copy, useToast } from "./clipboard";
import { confirmAction } from "./confirm";
import { formatCurl } from "./curl";
import { buildPredicate } from "./filter";
import { bodyBytes } from "./http";
import { mapLocalFrom, newRule, patternFor, type RuleKind } from "./rules";

const toast = (m: string) => useToast.getState().show(m);

/** Headers a composed request shouldn't carry over (reqwest sets them). */
const DROP = new Set(["content-length", "host", "connection", "proxy-connection", "keep-alive", "transfer-encoding"]);

export function draftFromDetail(d: FlowDetail): Draft {
  const body = d.requestBody;
  const headers = d.requestHeaders
    .filter(([k]) => !DROP.has(k.toLowerCase()))
    // The body is shown decoded, so it is sent decoded too.
    .filter(([k]) => !(body.encoding && k.toLowerCase() === "content-encoding"))
    .map(([k, v]) => `${k}: ${v}`)
    .join("\n");
  return {
    method: d.meta.method,
    url: d.meta.url,
    headers,
    body: body.text ?? "",
    binaryBody: body.text === null && body.size > 0 ? Array.from(bodyBytes(body)) : null,
  };
}

/** Clears every flow, after asking (there is no undo). */
export async function clearFlows() {
  const n = useFlows.getState().order.length;
  const what = n === 1 ? "1 flow" : `${n} flows`;
  if (n && !(await confirmAction(`Clear ${what}? This can’t be undone; save a session (⌘S) first to keep them.`, "Clear"))) {
    return;
  }
  await api.clearFlows();
}

export function parseHeaders(text: string): [string, string][] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => {
      const i = l.indexOf(":");
      return (i <= 0 ? [l, ""] : [l.slice(0, i).trim(), l.slice(i + 1).trim()]) as [string, string];
    });
}

export function draftToRequest(d: Draft): OutgoingRequest {
  return {
    method: d.method.trim().toUpperCase() || "GET",
    url: d.url.trim(),
    headers: parseHeaders(d.headers),
    body: d.binaryBody ?? Array.from(new TextEncoder().encode(d.body)),
  };
}

function selectNew(id: number | null) {
  if (id === null) return;
  // The batch carrying the new flow may still be in flight.
  const trySelect = (left: number) => {
    if (id in useFlows.getState().byId) useFlows.getState().select(id);
    else if (left > 0) setTimeout(() => trySelect(left - 1), 100);
  };
  trySelect(10);
}

export async function repeat(id: number) {
  try {
    selectNew(await api.repeatFlow(id));
    toast("Request repeated");
  } catch (e) {
    toast(`Repeat failed: ${errorMessage(e)}`);
  }
}

export async function editAndRepeat(id: number) {
  const d = await api.getFlow(id);
  if (d) useUi.getState().openComposer(draftFromDetail(d));
}

/** Sends a composed request; throws so the composer can show the error. */
export async function sendDraft(d: Draft) {
  selectNew(await api.sendRequest(draftToRequest(d)));
}

export function compareWith(id: number) {
  const { compareMark, markForCompare, openDiff } = useUi.getState();
  if (compareMark === null || compareMark === id) {
    markForCompare(id);
    toast(`Marked #${id} for comparison`);
  } else {
    openDiff(compareMark, id);
  }
}

const HAR = [{ name: "HTTP Archive", extensions: ["har"] }];
const SESSION = [{ name: "Project Proxy Session", extensions: ["ppsession"] }];

async function run(fn: () => Promise<string | null>) {
  try {
    const msg = await fn();
    if (msg) toast(msg);
  } catch (e) {
    toast(errorMessage(e));
  }
}

const plural = (n: number) => `${n} flow${n === 1 ? "" : "s"}`;

export const files = {
  exportHar: (ids?: number[]) =>
    run(async () => {
      const path = await save({ filters: HAR, defaultPath: "flows.har" });
      return path ? `Exported ${plural(await api.exportHar(path, ids))}` : null;
    }),
  saveSession: () =>
    run(async () => {
      const path = await save({ filters: SESSION, defaultPath: "session.ppsession" });
      return path ? `Saved ${plural(await api.saveSession(path))}` : null;
    }),
  /** Opens a session or HAR, replacing the current flows. */
  open: () =>
    run(async () => {
      const path = await open({ filters: [...SESSION, ...HAR], multiple: false, directory: false });
      return path ? `Opened ${plural(await api.importFlows(path, true))}` : null;
    }),
  importHar: () =>
    run(async () => {
      const path = await open({ filters: [...HAR, ...SESSION], multiple: false, directory: false });
      return path ? `Imported ${plural(await api.importFlows(path, false))}` : null;
    }),
};

/** Removes flows from the list; asks first when there are several. */
export async function removeFlows(ids: number[]) {
  if (!ids.length) return;
  if (ids.length > 1 && !(await confirmAction(`Remove ${plural(ids.length)}? This can’t be undone.`, "Remove"))) return;
  try {
    await api.removeFlows(ids);
  } catch (e) {
    toast(errorMessage(e));
  }
}

/** Actions for several selected flows at once. */
export const batch = {
  copyUrls: (ids: number[]) => {
    const { byId } = useFlows.getState();
    return copy(ids.map((id) => byId[id]?.url).filter(Boolean).join("\n"), `${ids.length} URLs`);
  },
  copyCurls: async (ids: number[]) => {
    const details = await Promise.all(ids.map((id) => api.getFlow(id)));
    const commands = details
      .filter((d): d is FlowDetail => d !== null)
      .map((d) => formatCurl(d.meta.method, d.meta.url, d.requestHeaders, d.requestBody));
    return copy(commands.join("\n\n"), `${commands.length} cURL commands`);
  },
  /** Repeats one after another, so the copies keep the original order. */
  repeat: async (ids: number[]) => {
    let sent = 0;
    for (const id of ids) {
      try {
        await api.repeatFlow(id);
        sent++;
      } catch (e) {
        toast(`Repeat of #${id} failed: ${errorMessage(e)}`);
        return;
      }
    }
    toast(`Repeated ${plural(sent)}`);
  },
};

/** Ids of the flows the table currently shows (filters applied). */
export function visibleIds(): number[] {
  const s = useFlows.getState();
  const p = buildPredicate({ text: s.filter, methods: s.methods, statuses: s.statuses, scope: s.scope });
  return s.order.filter((id) => p(s.byId[id]));
}

/** Adds a host to SSL Proxying → "Never decrypt", so it is tunnelled. */
export async function dontDecrypt(host: string) {
  try {
    const store = useSettings.getState();
    const settings = store.settings ?? (await api.getSettings());
    const exclude = settings.sslProxying.exclude;
    if (!exclude.includes(host)) {
      await store.save({ ...settings, sslProxying: { ...settings.sslProxying, exclude: [...exclude, host] } });
    }
    toast(`${host} will no longer be decrypted`);
  } catch (e) {
    toast(errorMessage(e));
  }
}

/** Opens the Rules dialog with a new rule for this flow's URL. */
export async function addRuleFor(id: number, kind: RuleKind) {
  const d = await api.getFlow(id);
  if (!d) return;
  const rule = kind === "mapLocal" ? mapLocalFrom(d) : { ...newRule(kind, patternFor(d.meta.url)), method: d.meta.method };
  useUi.getState().openRules({ tab: kind, add: rule as unknown as RulesView["add"] });
}
