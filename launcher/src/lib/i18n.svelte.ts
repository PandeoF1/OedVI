// Interface language. The source strings are English and double as keys: a
// language other than English maps each of them to its translation, and a
// string without a translation shows as written.
import fr from "./locales/fr";

export type Lang = "en" | "fr";

export const LANGS: { id: Lang; label: string }[] = [
  { id: "en", label: "English" },
  { id: "fr", label: "Français" },
];

const STORAGE_KEY = "oedvi.lang";

const TABLES: Record<Lang, Record<string, string>> = { en: {}, fr };

function load(): Lang {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "fr" || v === "en") return v;
  } catch {
    /* storage blocked: English */
  }
  return "en";
}

let current = $state<Lang>(load());

export function lang(): Lang {
  return current;
}

export function setLang(l: Lang) {
  current = l;
  try {
    localStorage.setItem(STORAGE_KEY, l);
  } catch {
    /* kept for this session only */
  }
  document.documentElement.lang = l;
}

/** `t("Delete {name}?", { name })`: translated when the language has it. */
export function t(text: string, vars?: Record<string, string | number>): string {
  const s = TABLES[current][text] ?? text;
  return vars ? s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : s;
}
