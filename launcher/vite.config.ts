import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(({ command }) => ({
  // In dev, components inject their CSS from JS: Vite 8 can request the
  // virtual CSS modules before the component is compiled and drop them.
  plugins: [svelte({ emitCss: command === "build" })],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
  },
}));
