// Syntax highlighting and find-in-body for the inspector, without a
// library: JSON and markup (HTML/XML) tokenizers produce segments, and
// find matches are cut into them as numbered marks.

export type Lang = "json" | "markup" | null;

export interface Segment {
  text: string;
  /** Token class: key, string, number, literal, tag, attr, comment. */
  token?: string;
  /** Index of the find match this piece belongs to. */
  match?: number;
}

/** Bodies larger than this are shown plain; tokenizing them would stall the UI. */
export const HIGHLIGHT_LIMIT = 200_000;
/** Find stops counting here. */
export const MATCH_LIMIT = 1000;

export function langFor(contentType: string | null, json: boolean): Lang {
  if (json) return "json";
  const ct = (contentType ?? "").toLowerCase();
  if (/html|xml|svg/.test(ct)) return "markup";
  return null;
}

export function tokenize(text: string, lang: Lang): Segment[] {
  if (!lang || text.length > HIGHLIGHT_LIMIT) return [{ text }];
  return lang === "json" ? tokenizeJson(text) : tokenizeMarkup(text);
}

const JSON_TOKEN = /("(?:\\.|[^"\\])*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g;

function tokenizeJson(text: string): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  for (const m of text.matchAll(JSON_TOKEN)) {
    const at = m.index!;
    if (at > last) out.push({ text: text.slice(last, at) });
    if (m[1] !== undefined) {
      out.push({ text: m[1], token: m[2] ? "key" : "string" });
      if (m[2]) out.push({ text: m[2] });
    } else {
      out.push({ text: m[0], token: m[3] !== undefined ? "number" : "literal" });
    }
    last = at + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

const MARKUP_TOKEN = /(<!--[\s\S]*?-->)|(<\/?[A-Za-z!?][\w:.-]*)|(\/?\??>)|([\w:.-]+)(?==)|("[^"]*"|'[^']*')/g;

/** Tags, attribute names and values; text between tags stays plain. */
function tokenizeMarkup(text: string): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  let inTag = false;
  for (const m of text.matchAll(MARKUP_TOKEN)) {
    const at = m.index!;
    let token: string | undefined;
    if (m[1]) token = "comment";
    else if (m[2]) {
      token = "tag";
      inTag = true;
    } else if (m[3] && inTag) {
      token = "tag";
      inTag = false;
    } else if (inTag) token = m[4] ? "attr" : m[5] ? "string" : undefined;
    if (!token) continue;
    if (at > last) out.push({ text: text.slice(last, at) });
    out.push({ text: m[0], token });
    last = at + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

/** Case-insensitive [start, end) ranges of `query` in `text`. */
export function findMatches(text: string, query: string): [number, number][] {
  if (!query) return [];
  const hay = text.toLowerCase();
  const needle = query.toLowerCase();
  const out: [number, number][] = [];
  for (let i = hay.indexOf(needle); i >= 0 && out.length < MATCH_LIMIT; i = hay.indexOf(needle, i + needle.length)) {
    out.push([i, i + needle.length]);
  }
  return out;
}

/** Splits segments at match boundaries, numbering the pieces inside matches. */
export function applyMatches(segments: Segment[], ranges: [number, number][]): Segment[] {
  if (!ranges.length) return segments;
  const out: Segment[] = [];
  let pos = 0;
  let r = 0;
  for (const seg of segments) {
    const end = pos + seg.text.length;
    let cut = pos;
    while (cut < end) {
      while (r < ranges.length && ranges[r][1] <= cut) r++;
      const range = ranges[r];
      if (!range || range[0] >= end) {
        out.push({ ...seg, text: seg.text.slice(cut - pos) });
        break;
      }
      if (range[0] > cut) {
        out.push({ ...seg, text: seg.text.slice(cut - pos, range[0] - pos) });
        cut = range[0];
      }
      const stop = Math.min(range[1], end);
      out.push({ ...seg, text: seg.text.slice(cut - pos, stop - pos), match: r });
      cut = stop;
    }
    pos = end;
  }
  return out;
}
