import { afterEach, describe, expect, it, vi } from "vitest";
import { guardedClose } from "./confirm";

const ask = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask }));

describe("guardedClose", () => {
  afterEach(() => ask.mockReset());

  it("closes at once when nothing changed", async () => {
    const close = vi.fn();
    await guardedClose(false, close)();
    expect(ask).not.toHaveBeenCalled();
    expect(close).toHaveBeenCalledOnce();
  });

  it("asks before discarding edits and respects the answer", async () => {
    const close = vi.fn();
    ask.mockResolvedValueOnce(false);
    await guardedClose(true, close, "rule changes")();
    expect(ask).toHaveBeenCalledWith("Discard your unsaved rule changes?", expect.objectContaining({ okLabel: "Discard" }));
    expect(close).not.toHaveBeenCalled();

    ask.mockResolvedValueOnce(true);
    await guardedClose(true, close)();
    expect(close).toHaveBeenCalledOnce();
  });
});
