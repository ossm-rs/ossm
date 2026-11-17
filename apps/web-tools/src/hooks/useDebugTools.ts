import { useSyncExternalStore } from "react";

// "?" anywhere (outside a text field) toggles the debugging tools: the view
// cube and readouts. Remembered for the tab.
const KEY = "debug-tools";

let enabled = (() => {
  try {
    return sessionStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
})();
const listeners = new Set<() => void>();

function onKey(e: KeyboardEvent) {
  if (e.key !== "?" || e.ctrlKey || e.metaKey || e.altKey) return;
  const t = e.target as HTMLElement | null;
  if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
  enabled = !enabled;
  try {
    sessionStorage.setItem(KEY, enabled ? "1" : "0");
  } catch {}
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  if (listeners.size === 0) window.addEventListener("keydown", onKey);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) window.removeEventListener("keydown", onKey);
  };
}

/** Whether the debugging tools are on (toggled with "?"). */
export function useDebugTools(): boolean {
  return useSyncExternalStore(subscribe, () => enabled);
}
