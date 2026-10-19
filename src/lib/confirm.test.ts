import { afterEach, describe, expect, it, vi } from "vitest";
import { confirmAction, guardedClose } from "./confirm";

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

describe("confirmAction", () => {
  afterEach(() => ask.mockReset());

  it("opens one dialog at a time", async () => {
    let answer!: (yes: boolean) => void;
    ask.mockReturnValueOnce(new Promise<boolean>((r) => (answer = r)));
    const first = confirmAction("Clear 5 flows?", "Clear");
    expect(await confirmAction("Clear 5 flows?", "Clear")).toBe(false);
    expect(ask).toHaveBeenCalledOnce();

    answer(true);
    expect(await first).toBe(true);

    ask.mockResolvedValueOnce(true);
    expect(await confirmAction("Clear 1 flow?", "Clear")).toBe(true);
    expect(ask).toHaveBeenCalledTimes(2);
  });
});
