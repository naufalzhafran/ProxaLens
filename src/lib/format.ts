export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatDuration(ms: number | null): string {
  if (ms === null) return "";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

/** "HH:MM:SS.mmm", or "HH:MM:SS" without milliseconds. */
export function formatTime(epochMs: number, millis = true): string {
  const d = new Date(epochMs);
  const pad = (n: number, w = 2) => n.toString().padStart(w, "0");
  const clock = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  return millis ? `${clock}.${pad(d.getMilliseconds(), 3)}` : clock;
}

/**
 * Classes for an HTTP status code: its color plus a shape (see `.st` in
 * index.css), so the class never rests on hue alone.
 */
export function statusColor(status: number | null): string {
  if (status === null) return "st st-none";
  if (status >= 500) return "st st-server";
  if (status >= 400) return "st st-client";
  if (status >= 300) return "st st-redirect";
  return "st st-ok";
}

/** "application/json; charset=utf-8" → "json", "image/svg+xml" → "svg". */
export function shortContentType(ct: string | null): string {
  if (!ct) return "";
  const mime = ct.split(";")[0].trim().toLowerCase();
  const sub = (mime.split("/")[1] ?? mime).replace(/^x-/, "");
  return sub.endsWith("+json") ? "json" : sub.split("+")[0];
}

export type Hue = "blue" | "green" | "pink" | "orange" | "yellow" | "teal" | "violet" | "red" | "gray";

const METHOD_HUES: Record<string, Hue> = {
  GET: "blue",
  POST: "green",
  PUT: "orange",
  PATCH: "yellow",
  DELETE: "red",
  CONNECT: "violet",
};

/** The badge colour for an HTTP method; anything unusual is gray. */
export function methodHue(method: string): Hue {
  return METHOD_HUES[method.toUpperCase()] ?? "gray";
}

/** Hues for names, gray left out so a tile never looks disabled. */
const NAME_HUES: Hue[] = ["blue", "green", "pink", "orange", "yellow", "teal", "violet", "red"];

/** A stable hue for a name (an app or domain), so its tile keeps its colour. */
export function hueFor(name: string): Hue {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0;
  return NAME_HUES[Math.abs(h) % NAME_HUES.length];
}
