<script lang="ts">
  let {
    value,
    indeterminate = false,
    tone = "accent",
    height = 8,
  }: { value: number; indeterminate?: boolean; tone?: "accent" | "ok" | "info"; height?: number } = $props();
</script>

<div class="track" style:height="{height}px">
  <div class="fill {tone}" class:indeterminate style:width="{indeterminate ? 35 : value}%"></div>
</div>

<style>
  .track {
    width: 100%;
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: 999px;
    overflow: hidden;
    position: relative;
  }
  .fill {
    height: 100%;
    border-radius: 999px;
    transition: width 0.3s ease;
  }
  .accent {
    background: var(--grad);
    box-shadow: 0 0 16px rgba(124, 92, 255, 0.55);
  }
  .ok {
    background: var(--ok);
  }
  .info {
    background: var(--info);
  }
  .indeterminate {
    position: absolute;
    animation: slide 1.3s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      left: -35%;
    }
    to {
      left: 100%;
    }
  }
</style>
