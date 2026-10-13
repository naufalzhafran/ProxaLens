import { describe, expect, it } from "vitest";
import { applyMatches, findMatches, HIGHLIGHT_LIMIT, langFor, tokenize } from "./highlight";

const joined = (segs: { text: string }[]) => segs.map((s) => s.text).join("");
const tokens = (segs: { text: string; token?: string }[]) => segs.filter((s) => s.token).map((s) => [s.token, s.text]);

describe("tokenize", () => {
  it("marks JSON keys, strings, numbers and literals, keeping every character", () => {
    const text = '{\n  "name": "a \\"b\\"",\n  "n": -1.5e3,\n  "ok": true,\n  "x": null\n}';
    const segs = tokenize(text, "json");
    expect(joined(segs)).toBe(text);
    expect(tokens(segs)).toEqual([
      ["key", '"name"'],
      ["string", '"a \\"b\\""'],
      ["key", '"n"'],
      ["number", "-1.5e3"],
      ["key", '"ok"'],
      ["literal", "true"],
      ["key", '"x"'],
      ["literal", "null"],
    ]);
  });

  it("marks tags, attributes and values in markup but not text", () => {
    const text = '<!-- c --><a href="/x" id=\'y\'>a=b "q"</a>';
    const segs = tokenize(text, "markup");
    expect(joined(segs)).toBe(text);
    expect(tokens(segs)).toEqual([
      ["comment", "<!-- c -->"],
      ["tag", "<a"],
      ["attr", "href"],
      ["string", '"/x"'],
      ["attr", "id"],
      ["string", "'y'"],
      ["tag", ">"],
      ["tag", "</a"],
      ["tag", ">"],
    ]);
  });

  it("leaves unknown languages and huge bodies plain", () => {
    expect(tokenize("plain", null)).toEqual([{ text: "plain" }]);
    const big = "1".repeat(HIGHLIGHT_LIMIT + 1);
    expect(tokenize(big, "json")).toEqual([{ text: big }]);
  });

  it("picks a language from the content type", () => {
    expect(langFor("text/html; charset=utf-8", false)).toBe("markup");
    expect(langFor("application/xml", false)).toBe("markup");
    expect(langFor("text/plain", true)).toBe("json");
    expect(langFor("text/plain", false)).toBeNull();
  });
});

describe("find", () => {
  it("finds case-insensitive, non-overlapping matches", () => {
    expect(findMatches("aAa a", "aa")).toEqual([[0, 2]]);
    expect(findMatches("Abc abc", "ABC")).toEqual([[0, 3], [4, 7]]);
    expect(findMatches("abc", "")).toEqual([]);
  });

  it("cuts matches into segments, across token boundaries", () => {
    const segs = tokenize('{"key": "value"}', "json");
    const text = joined(segs);
    const marked = applyMatches(segs, findMatches(text, 'y": "v'));
    expect(joined(marked)).toBe(text);
    const pieces = marked.filter((s) => s.match === 0);
    expect(pieces.map((s) => [s.token, s.text])).toEqual([
      ["key", 'y"'],
      [undefined, ":"],
      [undefined, " "],
      ["string", '"v'],
    ]);
  });
});
