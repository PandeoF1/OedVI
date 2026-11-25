<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, type BuilderDef, type BuilderLoadError, type RawKind } from "../lib/api";
  import { app, isRemote, refreshBuilders } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import { attempt, toast } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";

  let kind = $state<RawKind>("builders");
  let ids = $state<string[]>([]);
  let current = $state<string | null>(null);
  let isNew = $state(false);
  let newId = $state("");
  let text = $state("");
  let original = $state("");
  let checkError = $state<string | null>(null);
  let checkOk = $state<BuilderDef | null>(null);
  let otherErrors = $state<BuilderLoadError[]>([]);
  let saving = $state(false);
  let filter = $state("");

  let dirty = $derived(text !== original);
  let errorIds = $derived(new Set((app.builders?.errors ?? []).map((e) => e.id)));

  const BUILDER_TEMPLATE = `# Inherits from a template (the templates/ folder) and only changes what is needed.
extends = "victory"
name = "New builder"
description = ""
tags = []
# Builders that share a clone also share its build.
checkout = "victory"

[vars]
profile = "release"

# Environment variables when the game runs only.
[run_env]
`;

  const TEMPLATE_TEMPLATE = `stack = "rust"
repo = "https://github.com/owner/repo"
ref = "main"
build = [["cargo", "build", "--profile", "{profile}"]]
run = ["{repo_dir}/target/{profile}/app{exe_suffix}"]

[vars]
profile = "release"
`;

  async function loadIds() {
    const r = await attempt(() => api.listIds(kind));
    if (r) ids = r;
  }

  $effect(() => {
    kind;
    current = null;
    isNew = false;
    text = original = "";
    loadIds();
  });

  async function open(id: string) {
    if (dirty && !(await ask(t("Unsaved changes will be lost."), { title: t("Switch file?"), kind: "warning" }))) return;
    const content = await attempt(() => api.readRaw(kind, id));
    if (content == null) return;
    current = id;
    isNew = false;
    text = original = content;
    resetCheck();
  }

  function create(copyFrom?: string) {
    isNew = true;
    current = null;
    newId = copyFrom ? `${copyFrom}-copy` : "";
    text = copyFrom ? text : kind === "builders" ? BUILDER_TEMPLATE : TEMPLATE_TEMPLATE;
    original = "";
    resetCheck();
  }

  function resetCheck() {
    checkError = null;
    checkOk = null;
    otherErrors = [];
  }

  let targetId = $derived(isNew ? newId.trim() : current);

  async function check(): Promise<boolean> {
    if (!targetId) {
      checkError = t("Give an identifier (letters, digits, - _ .)");
      return false;
    }
    try {
      const list = await api.checkRaw(kind, targetId, text);
      checkError = null;
      checkOk = kind === "builders" ? (list.builders.find((b) => b.id === targetId) ?? null) : null;
      otherErrors = list.errors.filter((e) => e.id !== targetId);
      return true;
    } catch (e) {
      checkError = String(e);
      checkOk = null;
      return false;
    }
  }

  async function save() {
    if (!(await check())) return;
    saving = true;
    try {
      await api.writeRaw(kind, targetId!, text);
      toast(t("{id} saved", { id: targetId ?? "" }), "success");
      original = text;
      if (isNew) {
        current = targetId;
        isNew = false;
      }
      await Promise.all([loadIds(), refreshBuilders(true)]);
    } catch (e) {
      checkError = String(e);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!current) return;
    const ok = await ask(
      t(kind === "builders" ? "Delete builder “{id}”? A copy is kept in .trash." : "Delete template “{id}”? A copy is kept in .trash.", { id: current }),
      { title: t("Delete?"), kind: "warning" },
    );
    if (!ok) return;
    if ((await attempt(() => api.deleteRaw(kind, current!), t("Deleted"))) !== undefined) {
      current = null;
      text = original = "";
      await Promise.all([loadIds(), refreshBuilders(true)]);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    const ta = e.currentTarget as HTMLTextAreaElement;
    if (e.key === "Tab") {
      e.preventDefault();
      const { selectionStart: s, selectionEnd: en } = ta;
      text = text.slice(0, s) + "  " + text.slice(en);
      queueMicrotask(() => ta.setSelectionRange(s + 2, s + 2));
    } else if ((e.ctrlKey || e.metaKey) && e.key === "s") {
      e.preventDefault();
      save();
    }
  }

  let shownIds = $derived(ids.filter((i) => i.toLowerCase().includes(filter.trim().toLowerCase())));
  let lineCount = $derived(text.split("\n").length);
</script>

<div class="page">
  <div class="side">
    <div class="tabs">
      <button class:on={kind === "builders"} onclick={() => (kind = "builders")}>{t("Builders")}</button>
      <button class:on={kind === "templates"} onclick={() => (kind = "templates")}>{t("Templates")}</button>
    </div>
    <div class="row">
      <input class="flt" placeholder={t("Filter…")} bind:value={filter} />
      <button class="sm" onclick={() => create()} title={t("New")}><Icon name="plus" size={14} /></button>
    </div>
    <div class="ids">
      {#each shownIds as i}
        <button class="id" class:active={current === i && !isNew} onclick={() => open(i)}>
          <Icon name="file" size={13} />
          <span class="ellipsis mono">{i}</span>
          {#if kind === "builders" && errorIds.has(i)}<span class="dot err" title={t("invalid")}></span>{/if}
        </button>
      {:else}
        <div class="dim pad">{t("No file")}</div>
      {/each}
    </div>
    <div class="dim hint">
      {isRemote() ? t("The files are in {kind}/ on the server.", { kind }) : t("The files are in {kind}/ of the data folder.", { kind })}
      {t("Templates are not listed as builders: a builder uses them with")} <code>extends</code>.
    </div>
  </div>

  <div class="main">
    {#if current || isNew}
      <div class="bar">
        {#if isNew}
          <input class="mono idin" placeholder={t("identifier (e.g. amd-diag)")} bind:value={newId} />
        {:else}
          <h2 class="mono">{current}</h2>
          {#if dirty}<span class="badge warn">{t("modified")}</span>{/if}
        {/if}
        <div class="spacer"></div>
        <button onclick={check}><Icon name="check" /> {t("Check")}</button>
        {#if current && !isNew}
          <button onclick={() => create(current!)} title={t("Duplicate")}><Icon name="layers" /> {t("Duplicate")}</button>
          <button class="danger" onclick={remove}><Icon name="trash" /></button>
        {/if}
        <button class="primary" onclick={save} disabled={saving || (!dirty && !isNew)}><Icon name="save" /> {t("Save")}</button>
      </div>

      <div class="editor">
        <div class="gutter mono" aria-hidden="true">
          {#each { length: lineCount } as _, i}<div>{i + 1}</div>{/each}
        </div>
        <textarea class="mono" spellcheck="false" bind:value={text} {onkeydown} oninput={resetCheck}></textarea>
      </div>

      {#if checkError}
        <div class="result bad"><Icon name="alert" /> <span class="mono">{checkError}</span></div>
      {:else if checkOk}
        <div class="result good">
          <div class="row"><Icon name="check" /> <b>{t("Valid.")}</b> {t("After resolving")} <code>extends</code>:</div>
          <div class="kv mono small">
            <span>repo</span><span>{checkOk.repo} @ {checkOk.ref}</span>
            <span>stack</span><span>{checkOk.stack}</span>
            <span>{t("clone")}</span><span>{checkOk.checkout ?? checkOk.id}</span>
            {#each checkOk.build as b, i}<span>build {i + 1}</span><span>{b.join(" ")}</span>{/each}
            <span>run</span><span>{checkOk.run.join(" ")}</span>
            {#each Object.entries(checkOk.vars) as [k, v]}<span>{"{" + k + "}"}</span><span>{v}</span>{/each}
            {#each Object.entries({ ...checkOk.env, ...checkOk.run_env }) as [k, v]}<span>env</span><span>{k}={v}</span>{/each}
          </div>
        </div>
      {:else if kind === "templates" && checkError === null && otherErrors.length === 0 && dirty === false && original}
        <div></div>
      {/if}
      {#if otherErrors.length}
        <div class="result warnbox">
          <div class="row"><Icon name="alert" /> {t("With this file, these builders would be invalid:")}</div>
          {#each otherErrors as e}<div class="mono small"><b>{e.id}</b>: {e.message}</div>{/each}
        </div>
      {/if}
    {:else}
      <div class="empty full">
        <Icon name="code" size={40} />
        <h2>{t("Builder editor")}</h2>
        <div class="muted help">
          {t("Pick a file on the left or create one. Available placeholders:")}
          <code>{"{game_dir}"}</code> <code>{"{repo_dir}"}</code> <code>{"{work_dir}"}</code> <code>{"{cache_dir}"}</code>
          <code>{"{data_dir}"}</code> <code>{"{root_dir}"}</code> <code>{"{builder_id}"}</code> <code>{"{exe_suffix}"}</code>
          <code>{"{os}"}</code>, {t("your")} <code>[vars]</code>, {t("and")} <code>{"{x...}"}</code> {t("to spread a variable into several arguments.")}
        </div>
        <button class="primary" onclick={() => create()}><Icon name="plus" /> {kind === "builders" ? t("New builder") : t("New template")}</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .page {
    display: grid;
    grid-template-columns: 270px 1fr;
    height: 100%;
  }
  .side {
    border-right: 1px solid var(--line);
    padding: 22px 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
  }
  .tabs {
    display: flex;
    background: var(--bg-2);
    border: 1px solid var(--line-2);
    border-radius: 10px;
    padding: 3px;
  }
  .tabs button {
    flex: 1;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--muted);
    padding: 6px;
  }
  .tabs button.on {
    background: var(--panel-2);
    color: var(--text);
  }
  .flt {
    flex: 1;
    padding: 6px 9px;
  }
  .ids {
    flex: 1;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .id {
    background: transparent;
    border-color: transparent;
    width: 100%;
    padding: 6px 9px;
    font-size: 13px;
    color: var(--muted);
  }
  .id.active {
    background: var(--panel-2);
    border-color: var(--line-2);
    color: var(--text);
  }
  .pad {
    padding: 8px;
  }
  .hint {
    font-size: 11.5px;
    line-height: 1.5;
  }
  .main {
    display: flex;
    flex-direction: column;
    padding: 22px 26px;
    gap: 12px;
    min-height: 0;
    overflow: auto;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .idin {
    width: 300px;
  }
  .editor {
    display: flex;
    border: 1px solid var(--line-2);
    border-radius: var(--radius);
    background: #07080c;
    min-height: 380px;
    flex: 1;
    overflow: auto;
  }
  .gutter {
    padding: 12px 8px 12px 12px;
    color: var(--dim);
    text-align: right;
    user-select: none;
    font-size: 12.5px;
    line-height: 1.6;
    border-right: 1px solid var(--line);
    min-width: 44px;
  }
  .editor textarea {
    flex: 1;
    border: none;
    border-radius: 0;
    background: transparent;
    resize: none;
    padding: 12px 14px;
    font-size: 12.5px;
    line-height: 1.6;
    box-shadow: none;
    white-space: pre;
    overflow: hidden;
    field-sizing: content;
    min-height: 100%;
  }
  .result {
    border-radius: 10px;
    padding: 10px 13px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
  }
  .result.bad {
    flex-direction: row;
    align-items: flex-start;
    border: 1px solid rgba(248, 113, 113, 0.4);
    background: rgba(248, 113, 113, 0.07);
    color: #fecaca;
  }
  .result.good {
    border: 1px solid rgba(52, 211, 153, 0.35);
    background: rgba(52, 211, 153, 0.06);
  }
  .result.good :global(svg) {
    color: var(--ok);
  }
  .result.warnbox {
    border: 1px solid rgba(251, 191, 36, 0.35);
    background: rgba(251, 191, 36, 0.06);
    color: #fde68a;
  }
  .small {
    font-size: 12px;
  }
  .kv span {
    word-break: break-all;
  }
  .full {
    height: 100%;
  }
  .help {
    max-width: 620px;
    line-height: 1.8;
  }
</style>
