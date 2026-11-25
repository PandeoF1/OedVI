// Measures of the running game, one point a second, shared by every window.
import { untrack } from "svelte";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";

export interface LivePoint {
  t: number;
  values: Record<string, number>;
  phase?: string | null;
}

const MAX = 900;
// Replaced, never mutated in place: 900 points must not become reactive proxies.
let store = $state.raw<Record<string, LivePoint[]>>({});
let listening = false;

export function pointsOf(id: string): LivePoint[] {
  return store[id] ?? [];
}

// Called from effects too: reading the store here must not make them depend on it.
function put(id: string, points: LivePoint[]) {
  store = untrack(() => ({ ...store, [id]: points }));
}

export function resetLive(id: string) {
  put(id, []);
}

export async function loadLive(id: string) {
  try {
    put(id, await api.liveHistory(id));
  } catch {
    /* no run yet */
  }
}

export async function initLive() {
  if (listening) return;
  listening = true;
  await listen<{ id: string; point: LivePoint }>("metrics", (e) => {
    const { id, point } = e.payload;
    const next = [...untrack(() => store[id] ?? []), point];
    if (next.length > MAX) next.splice(0, next.length - MAX);
    put(id, next);
  });
}
