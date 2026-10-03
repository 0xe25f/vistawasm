import { describe, expect, it } from "vitest";
import { historyForKey, isTypingTarget, overlayForKey } from "../src/shortcuts.js";

const body = { tagName: "BODY" };
const key = (value, extra = {}) => ({ key: value, target: body, ...extra });

describe("overlay shortcuts", () => {
  it("toggle the stats, minimap, hint, status and panel with 1, 2, 3, 4 and 0", () => {
    expect(["1", "2", "3", "4", "0"].map((value) => overlayForKey(key(value)))).toEqual(["stats", "minimap", "hint", "status", "panel"]);
    expect(overlayForKey(key("5"))).toBeNull();
  });

  it("are ignored while an input, select or textarea has focus", () => {
    for (const tagName of ["INPUT", "SELECT", "TEXTAREA"]) {
      expect(overlayForKey(key("1", { target: { tagName } }))).toBeNull();
    }

    expect(overlayForKey(key("2", { target: { tagName: "DIV", isContentEditable: true } }))).toBeNull();
  });

  it("are ignored with modifiers and on key repeat", () => {
    expect(overlayForKey(key("1", { ctrlKey: true }))).toBeNull();
    expect(overlayForKey(key("1", { metaKey: true }))).toBeNull();
    expect(overlayForKey(key("1", { repeat: true }))).toBeNull();
  });

  it("never use the camera keys", () => {
    for (const value of ["w", "a", "s", "d", " ", "Shift"]) {
      expect(overlayForKey(key(value))).toBeNull();
    }
  });
});

describe("history shortcuts", () => {
  it("undo with Ctrl or Cmd + Z, and redo with Shift or Ctrl + Y", () => {
    expect(historyForKey(key("z", { ctrlKey: true }))).toBe("undo");
    expect(historyForKey(key("z", { metaKey: true }))).toBe("undo");
    expect(historyForKey(key("Z", { ctrlKey: true, shiftKey: true }))).toBe("redo");
    expect(historyForKey(key("Z", { metaKey: true, shiftKey: true }))).toBe("redo");
    expect(historyForKey(key("y", { ctrlKey: true }))).toBe("redo");
    expect(historyForKey(key("z"))).toBeNull();
  });

  it("leave text fields their own undo", () => {
    expect(historyForKey(key("z", { ctrlKey: true, target: { tagName: "INPUT" } }))).toBeNull();
    expect(isTypingTarget(null)).toBe(false);
  });
});
