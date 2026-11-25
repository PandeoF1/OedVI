<script lang="ts">
  // The small window next to the game: live charts of one builder.
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { initLive } from "../lib/live.svelte";
  import LiveCharts from "./LiveCharts.svelte";

  let id = $state(new URLSearchParams(location.search).get("overlay") ?? "");

  onMount(() => {
    initLive();
    const un = listen<string>("overlay-select", (e) => (id = e.payload));
    return () => void un.then((f) => f());
  });
</script>

<div class="overlay">
  <div class="title mono">{id}</div>
  {#key id}
    <LiveCharts {id} compact />
  {/key}
</div>

<style>
  .overlay {
    padding: 10px 12px 14px;
    height: 100vh;
    overflow: auto;
    background: var(--bg, #0b0d12);
  }
  .title {
    color: var(--muted);
    font-size: 11px;
    margin-bottom: 6px;
  }
</style>
