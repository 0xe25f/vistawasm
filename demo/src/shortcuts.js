// Keyboard shortcuts, as pure functions of a key event, so they can be
// tested without a browser. None of them fire while typing in a field,
// and none use the camera keys (WASD, Space and Shift).

/** The overlay each digit key toggles. */
export const OVERLAY_KEYS = { 1: "stats", 2: "minimap", 3: "hint", 4: "status", 0: "panel" };

/** Whether key presses on `element` belong to it: a field, select or editable text. */
export function isTypingTarget(element) {
  if (!element) {
    return false;
  }

  const tag = element.tagName;
  return tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA" || element.isContentEditable === true;
}

function modified(event) {
  return event.ctrlKey || event.metaKey || event.altKey;
}

/** The overlay a key press toggles, or `null`. */
export function overlayForKey(event) {
  if (modified(event) || event.repeat || isTypingTarget(event.target)) {
    return null;
  }

  return OVERLAY_KEYS[event.key] ?? null;
}

/** `"undo"`, `"redo"` or `null`: Ctrl or Cmd + Z, Ctrl or Cmd + Shift + Z, and Ctrl + Y. */
export function historyForKey(event) {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || isTypingTarget(event.target)) {
    return null;
  }

  const key = String(event.key).toLowerCase();

  if (key === "z") {
    return event.shiftKey ? "redo" : "undo";
  }

  return key === "y" && event.ctrlKey ? "redo" : null;
}
