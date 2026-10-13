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

/** Tailwind text color for an HTTP status code. */
export function statusColor(status: number | null): string {
  if (status === null) return "text-neutral-400";
  if (status >= 500) return "text-red-500";
  if (status >= 400) return "text-orange-500";
  if (status >= 300) return "text-sky-500";
  return "text-emerald-600 dark:text-emerald-400";
}

/** "application/json; charset=utf-8" → "json", "image/svg+xml" → "svg". */
export function shortContentType(ct: string | null): string {
  if (!ct) return "";
  const mime = ct.split(";")[0].trim().toLowerCase();
  const sub = (mime.split("/")[1] ?? mime).replace(/^x-/, "");
  return sub.endsWith("+json") ? "json" : sub.split("+")[0];
}
