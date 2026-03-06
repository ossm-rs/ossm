import { useRef, useCallback, useSyncExternalStore } from "react";

type PlaybackState = "stopped" | "homing" | "playing" | "paused";

const ENGINE_STATE_MAP: Record<number, PlaybackState> = {
  0: "stopped",
  1: "homing",
  2: "playing",
  3: "paused",
};

export function useEngineState(
  simulator: import("sim-wasm").Simulator,
): PlaybackState {
  const stateRef = useRef<PlaybackState>("stopped");

  const subscribe = useCallback(
    (onStoreChange: () => void) => {
      let raf: number;
      const poll = () => {
        const raw = simulator.get_engine_state();
        const next = ENGINE_STATE_MAP[raw] ?? "stopped";
        if (next !== stateRef.current) {
          stateRef.current = next;
          onStoreChange();
        }
        raf = requestAnimationFrame(poll);
      };
      raf = requestAnimationFrame(poll);
      return () => cancelAnimationFrame(raf);
    },
    [simulator],
  );

  return useSyncExternalStore(subscribe, () => stateRef.current);
}
