// Writes src-tauri/tauri.release.local.conf.json: the config overlay for a
// signed release build.
//
// The Tauri CLI checks `plugins.updater.pubkey` of the *config* while it
// signs the installer, and tauri.conf.json keeps that empty (the key is
// yours, not the repository's). So the public key is injected here, from a
// file (`node scripts/release-config.mjs ../oedvi-updater.key.pub`) or from
// the OEDVI_UPDATER_PUBKEY variable (CI).
import { readFileSync, writeFileSync } from "node:fs";

const fromFile = process.argv[2];
const pubkey = (fromFile ? readFileSync(fromFile, "utf8") : (process.env.OEDVI_UPDATER_PUBKEY ?? "")).trim();
if (!pubkey) {
  console.error("Public key missing: pass the .key.pub file as an argument, or set OEDVI_UPDATER_PUBKEY.");
  process.exit(1);
}
const out = new URL("../src-tauri/tauri.release.local.conf.json", import.meta.url);
writeFileSync(
  out,
  JSON.stringify({ bundle: { createUpdaterArtifacts: true }, plugins: { updater: { pubkey } } }, null, 2),
);
console.log("written:", out.pathname.replace(/^\/([A-Za-z]:)/, "$1"));
