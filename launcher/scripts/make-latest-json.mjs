// Writes `latest.json` next to the signed installer: the file Tauri's updater
// reads on GitHub releases.
//
//   node scripts/make-latest-json.mjs <bundle dir> <download base URL>
//
// The version comes from the installer's name (`<product>_<version>_x64-setup.exe`).
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, base] = process.argv.slice(2);
if (!dir || !base) {
  console.error("usage: make-latest-json.mjs <bundle dir> <download base URL>");
  process.exit(1);
}
const setup = readdirSync(dir).find((f) => f.endsWith("_x64-setup.exe"));
if (!setup) {
  console.error(`no *_x64-setup.exe in ${dir}`);
  process.exit(1);
}
const version = setup.slice(0, -"_x64-setup.exe".length).split("_").pop();
const signature = readFileSync(join(dir, `${setup}.sig`), "utf8").trim();
const latest = {
  version,
  notes: `OedVI ${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": { signature, url: `${base.replace(/\/$/, "")}/${encodeURIComponent(setup)}` },
  },
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(latest, null, 2));
console.log(`latest.json for ${version}`);
