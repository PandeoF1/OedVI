// Checks that every string passed to t() has a French translation, and that
// the French file has no string nobody uses. Run by `npm run check:i18n`.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const root = new URL("../src/", import.meta.url).pathname;

function files(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) return p.includes("locales") ? [] : files(p);
    return /\.(svelte|ts)$/.test(f) ? [p] : [];
  });
}

/** The text between the parentheses of a call starting at `i`, and where it ends. */
function balanced(s, i) {
  let depth = 1;
  let str = null;
  let j = i;
  for (; j < s.length && depth; j++) {
    const c = s[j];
    if (str) {
      if (c === "\\") j++;
      else if (c === str) str = null;
    } else if ("\"'`".includes(c)) str = c;
    else if ("([{".includes(c)) depth++;
    else if (")]}".includes(c)) depth--;
  }
  return s.slice(i, j - 1);
}

function firstArg(args) {
  let depth = 0;
  let str = null;
  for (let k = 0; k < args.length; k++) {
    const c = args[k];
    if (str) {
      if (c === "\\") k++;
      else if (c === str) str = null;
    } else if ("\"'`".includes(c)) str = c;
    else if ("([{".includes(c)) depth++;
    else if (")]}".includes(c)) depth--;
    else if (c === "," && depth === 0) return args.slice(0, k);
  }
  return args;
}

const literal = /"((?:[^"\\]|\\.)*)"/g;
const used = new Set();
for (const f of files(root)) {
  const s = readFileSync(f, "utf8");
  for (const m of s.matchAll(/(?<![\w.$])(?:t|tr)\(/g)) {
    for (const l of firstArg(balanced(s, m.index + m[0].length)).matchAll(literal)) used.add(l[1].replaceAll('\\"', '"'));
  }
  for (const m of s.matchAll(/(?<![\w.$])plural\(/g)) {
    for (const l of balanced(s, m.index + m[0].length).matchAll(literal)) used.add(l[1]);
  }
}
for (const l of readFileSync(join(root, "lib/metrics.ts"), "utf8").matchAll(/label: "([^"]+)"/g)) used.add(l[1]);

const fr = new Set([...readFileSync(join(root, "lib/locales/fr.ts"), "utf8").matchAll(/^\s*("(?:[^"\\]|\\.)*"):/gm)].map((m) => JSON.parse(m[1])));

const missing = [...used].filter((k) => !fr.has(k)).sort();
const unused = [...fr].filter((k) => !used.has(k)).sort();
for (const k of missing) console.error(`missing French translation: ${k}`);
for (const k of unused) console.error(`unused French entry: ${k}`);
console.log(`${used.size} strings, ${fr.size} French entries`);
process.exit(missing.length || unused.length ? 1 : 0);
