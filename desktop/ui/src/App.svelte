<svelte:options runes={true} />

<script>
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import { onMount } from 'svelte';

  let tab = $state('transcribe');
  let input = $state('');
  let progress = $state('');
  let result = $state('');
  let error = $state('');
  let dataRoot = $state('');
  let tools = $state([]);
  let showWizard = $state(false);
  let wizardPath = $state('');
  let enableTransparency = $state(true);
  let licenses = $state([]);
  let installing = $state('');
  let ready = $state(false);
  let busy = $state(false);

  onMount(() => {
    let disposed = false;
    const cleanup = [];
    const subscribe = async (name, callback) => {
      const unlisten = await listen(name, callback);
      if (disposed) unlisten(); else cleanup.push(unlisten);
    };
    const initialize = async () => {
    await subscribe('job-progress', (e) => {
      progress = JSON.stringify(e.payload);
    });
    await subscribe('job-done', (e) => {
      busy = false;
      progress = '';
      result = typeof e.payload === 'string' ? e.payload : JSON.stringify(e.payload, null, 2);
      error = '';
    });
    await subscribe('job-error', (e) => {
      busy = false;
      progress = '';
      error = e.payload?.display ?? String(e.payload);
    });
    const unlisten = await getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'drop' && !busy) {
        input = event.payload.paths[0] ?? '';
        tab = 'transcribe';
      }
    });
    if (disposed) unlisten(); else cleanup.push(unlisten);
    showWizard = await invoke('needs_setup');
    dataRoot = await invoke('get_data_root');
    wizardPath = dataRoot;
    tools = await invoke('list_tools');
    const cfg = await invoke('get_config');
    enableTransparency = cfg.ui?.enable_transparency ?? true;
    licenses = await invoke('list_licenses');
    if (!disposed) ready = true;
    };
    initialize().catch((e) => { error = e?.display ?? String(e); });
    return () => { disposed = true; cleanup.forEach((unlisten) => unlisten()); };
  });

  async function run() {
    if (!ready || busy) return;
    busy = true;
    error = '';
    result = '';
    progress = '处理中…';
    try {
      if (tab === 'transcribe') {
        await invoke('start_transcribe', { path: input });
      } else {
        await invoke('start_analyze', { url: input });
      }
    } catch (e) {
      busy = false;
      progress = '';
      error = e?.display ?? String(e);
    }
  }

  async function perform(action) {
    error = '';
    try { await action(); } catch (e) { error = e?.display ?? String(e); }
  }

  async function confirmWizard() {
    dataRoot = await invoke('set_data_root', { path: wizardPath });
    showWizard = false;
    tools = await invoke('list_tools');
  }

  async function pickTool(toolId) {
    const selected = await open({ multiple: false });
    if (selected) {
      await invoke('set_tool_path', { toolId, path: selected });
      tools = await invoke('list_tools');
    }
  }

  async function installTool(toolId) {
    installing = toolId;
    error = '';
    try {
      await invoke('install_tool', { toolId });
      tools = await invoke('list_tools');
    } catch (e) {
      error = e?.display ?? String(e);
    } finally {
      installing = '';
    }
  }

  async function toggleTransparency() {
    const cfg = await invoke('get_config');
    cfg.ui.enable_transparency = enableTransparency;
    await invoke('save_app_config', { cfg });
  }
</script>

{#if showWizard}
  <div class="wizard-overlay">
    <div class="wizard card">
      <h1>欢迎使用 ScenePacket</h1>
      <p class="hint">选择数据根目录。ffmpeg、模型与缓存将存放在此，删除文件夹即可完全卸载。</p>
      <input bind:value={wizardPath} placeholder="D:\AIVideoReader" />
      <button class="primary" onclick={() => perform(confirmWizard)}>继续</button>
      {#if error}<pre class="error" role="alert">{error}</pre>{/if}
    </div>
  </div>
{/if}

<div class="titlebar" data-tauri-drag-region>
  <span class="title">ScenePacket</span>
</div>

<nav class="sidebar">
  <button class:active={tab === 'transcribe'} onclick={() => (tab = 'transcribe')}>转写</button>
  <button class:active={tab === 'url'} onclick={() => (tab = 'url')}>链接</button>
  <button class:active={tab === 'tools'} onclick={() => (tab = 'tools')}>工具</button>
  <button class:active={tab === 'settings'} onclick={() => (tab = 'settings')}>设置</button>
  <button class:active={tab === 'about'} onclick={() => (tab = 'about')}>关于</button>
</nav>

<main class="content">
  {#if error}<pre class="error" role="alert">{error}</pre>{/if}
  {#if tab === 'transcribe' || tab === 'url'}
    <section class="card">
      <h1>{tab === 'transcribe' ? '拖入音视频文件' : '粘贴 B站/抖音链接'}</h1>
      <p class="hint">总结由 Cursor/Codex 完成；本程序只提取文本。</p>
      <input
        bind:value={input}
        placeholder={tab === 'transcribe' ? 'D:\\video.mp4' : 'https://www.bilibili.com/video/BV…'}
      />
      <button class="primary" onclick={run} disabled={!input.trim() || !ready || busy}>开始</button>
      {#if progress}<p class="status">{progress}</p>{/if}
      {#if result}<pre class="result">{result}</pre>{/if}
    </section>
  {:else if tab === 'tools'}
    <section class="card">
      <h1>工具就绪检查</h1>
      <ul class="tool-list">
        {#each tools as t}
          <li>
            <span class={t.ready ? 'ok' : 'bad'}>{t.ready ? '✓' : '○'}</span>
            <strong>{t.id}</strong>
            <span class="ver">{t.version || ''}</span>
            {#if !t.ready}
              {#if t.download_url}
                <button class="link-btn" disabled={installing === t.id} onclick={() => installTool(t.id)}>
                  {installing === t.id ? '安装中…' : '安装'}
                </button>
              {/if}
              <button class="link-btn" onclick={() => perform(() => pickTool(t.id))}>选择已有</button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {:else if tab === 'settings'}
    <section class="card">
      <h1>设置</h1>
      <p>数据根：<code>{dataRoot}</code></p>
      <label class="toggle">
        <input type="checkbox" bind:checked={enableTransparency} onchange={() => perform(toggleTransparency)} />
        启用透明效果（Mica，Win11）
      </label>
      <p class="hint">模型放入 <code>{dataRoot}\models\</code>，默认 ggml-large-v3-turbo.bin</p>
    </section>
  {:else}
    <section class="card">
      <h1>关于</h1>
      <p>ScenePacket — 视频上下文包，单 exe，无 Python 依赖。</p>
      <h2>第三方组件</h2>
      <ul class="tool-list">
        {#each licenses as l}
          <li><strong>{l.name}</strong> — {l.license} <a href={l.url} target="_blank" rel="noreferrer">来源</a></li>
        {/each}
      </ul>
    </section>
  {/if}
</main>

<style>
  .wizard-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .wizard {
    width: min(480px, 90vw);
  }
  .titlebar {
    height: 36px;
    display: flex;
    align-items: center;
    padding: 0 var(--space-4);
    border-bottom: 1px solid var(--separator);
    -webkit-app-region: drag;
  }
  .title {
    font-weight: 600;
    font-size: 13px;
  }
  :global(body) {
    display: flex;
    flex-direction: column;
  }
  :global(#app) {
    display: grid;
    grid-template-columns: 140px 1fr;
    grid-template-rows: 36px 1fr;
    height: 100vh;
  }
  .titlebar {
    grid-column: 1 / -1;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border-right: 1px solid var(--separator);
    background: var(--bg-elevated);
  }
  .sidebar button {
    text-align: left;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-btn);
    background: transparent;
    color: var(--label-secondary);
    cursor: pointer;
    transition: background var(--duration) var(--ease);
  }
  .sidebar button.active,
  .sidebar button:hover {
    background: rgba(10, 132, 255, 0.15);
    color: var(--label);
  }
  .content {
    padding: var(--space-5);
    overflow: auto;
  }
  .card {
    max-width: var(--max-ch);
    background: var(--bg-elevated);
    border-radius: var(--radius-card);
    padding: var(--space-5);
    border: 1px solid var(--separator);
  }
  h1 {
    font-size: 20px;
    margin: 0 0 var(--space-3);
  }
  h2 {
    font-size: 16px;
    margin: var(--space-4) 0 var(--space-2);
  }
  .hint {
    color: var(--label-secondary);
    margin-bottom: var(--space-4);
  }
  input:not([type]) {
    width: 100%;
    padding: var(--space-3);
    border-radius: var(--radius-btn);
    border: 1px solid var(--separator);
    background: var(--bg);
    color: var(--label);
    margin-bottom: var(--space-3);
  }
  .toggle {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    margin: var(--space-3) 0;
  }
  .primary {
    background: var(--tint);
    color: white;
    border: none;
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-btn);
    cursor: pointer;
    font-weight: 600;
  }
  .primary:disabled {
    opacity: 0.5;
  }
  .link-btn {
    background: transparent;
    border: none;
    color: var(--tint);
    cursor: pointer;
    font-size: 13px;
  }
  .status {
    color: var(--label-secondary);
    margin-top: var(--space-3);
  }
  .error {
    color: #ff453a;
    white-space: pre-wrap;
    font-size: 13px;
  }
  .result {
    margin-top: var(--space-4);
    font-size: var(--font-transcript);
    line-height: var(--line-transcript);
    white-space: pre-wrap;
    max-height: 320px;
    overflow: auto;
    background: var(--bg);
    padding: var(--space-3);
    border-radius: var(--radius-btn);
  }
  .tool-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .tool-list li {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--separator);
    flex-wrap: wrap;
  }
  .ok {
    color: #30d158;
  }
  .bad {
    color: var(--label-tertiary);
  }
  .ver {
    color: var(--label-tertiary);
    font-size: 12px;
    flex: 1;
  }
  a {
    color: var(--tint);
    font-size: 13px;
  }
  code {
    font-size: 12px;
    word-break: break-all;
  }
</style>
