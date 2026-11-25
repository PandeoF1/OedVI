<script lang="ts">
  import { t as tr } from "../lib/i18n.svelte";
  import { dismiss, toasts } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";
</script>

<div class="toasts">
  {#each toasts as t (t.id)}
    <div class="toast {t.kind}">
      <Icon name={t.kind === "error" ? "alert" : t.kind === "success" ? "check" : "zap"} />
      <span>{t.text}</span>
      <button class="ghost sm" onclick={() => dismiss(t.id)} aria-label={tr("Close")}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 18px;
    bottom: 18px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 100;
    max-width: 440px;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 11px 12px;
    border-radius: 11px;
    background: rgba(27, 32, 48, 0.92);
    backdrop-filter: blur(10px);
    border: 1px solid var(--line-2);
    box-shadow: 0 12px 30px -12px rgba(0, 0, 0, 0.7);
    animation: in 0.2s ease-out;
    word-break: break-word;
  }
  .toast span {
    flex: 1;
    white-space: pre-wrap;
  }
  .toast.error {
    border-color: rgba(248, 113, 113, 0.5);
    color: #fecaca;
  }
  .toast.success {
    border-color: rgba(52, 211, 153, 0.45);
  }
  .toast.success :global(svg) {
    color: var(--ok);
  }
  @keyframes in {
    from {
      transform: translateY(8px);
      opacity: 0;
    }
  }
</style>
