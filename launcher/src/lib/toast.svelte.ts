export type ToastKind = "info" | "success" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

let next = 1;
export const toasts = $state<Toast[]>([]);

export function toast(text: string, kind: ToastKind = "info", ms = kind === "error" ? 8000 : 4000) {
  const id = next++;
  toasts.push({ id, kind, text });
  setTimeout(() => dismiss(id), ms);
}

export function dismiss(id: number) {
  const i = toasts.findIndex((t) => t.id === id);
  if (i >= 0) toasts.splice(i, 1);
}

/** Runs `fn`, showing its error as a toast. */
export async function attempt<T>(fn: () => Promise<T>, success?: string): Promise<T | undefined> {
  try {
    const r = await fn();
    if (success) toast(success, "success");
    return r;
  } catch (e) {
    toast(String(e), "error");
    return undefined;
  }
}
