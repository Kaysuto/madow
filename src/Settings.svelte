<script lang="ts">
  import { onMount } from 'svelte'
  import Models from './Models.svelte'
  import Sources from './Sources.svelte'
  import {
    type SyncFailure,
    getSetting,
    launcherShortcut,
    setLauncherShortcut,
    setSetting,
    setUsageJournal,
    syncNow,
  } from './lib/api'
  import { i18n } from './lib/i18n.svelte'
  import { LOCALES } from './lib/locale'

  let demo = $state(false)
  let failures = $state<SyncFailure[]>([])
  let error = $state('')
  let shortcut = $state('')
  let active = $state(true)
  let habits = $state(true)

  const readShortcut = async () => void ({ shortcut, active } = await launcherShortcut())

  onMount(async () => {
    demo = (await getSetting<boolean>('demo')) === true
    habits = (await getSetting<boolean>('usage.off')) !== true
    await readShortcut()
  })

  async function toggleHabits() {
    try {
      await setUsageJournal(habits)
      error = ''
    } catch (cause) {
      error = String(cause)
    }
  }

  async function saveShortcut() {
    try {
      await setLauncherShortcut(shortcut)
      error = ''
    } catch (cause) {
      error = String(cause)
    }
    // Shows the shortcut that is really kept: the one before, when the new one was refused.
    await readShortcut()
  }

  async function toggleDemo() {
    try {
      await setSetting('demo', demo)
      failures = await syncNow()
      error = ''
    } catch (cause) {
      error = String(cause)
    }
  }
</script>

<h1>{i18n.t.settings}</h1>

<label class="row">
  <span class="name">{i18n.t.prefs.language}</span>
  <select class="field" bind:value={i18n.locale}>
    {#each LOCALES as locale (locale)}
      <option value={locale}>{locale === 'fr' ? 'Français' : 'English'}</option>
    {/each}
  </select>
</label>

<label class="row">
  <span>
    <span class="name">{i18n.t.prefs.demo}</span>
    <span class="hint">{i18n.t.prefs.demoHint}</span>
  </span>
  <input type="checkbox" bind:checked={demo} onchange={toggleDemo} />
</label>

<label class="row">
  <span>
    <span class="name">{i18n.t.prefs.shortcut}</span>
    <span class="hint">{i18n.t.prefs.shortcutHint}</span>
    {#if shortcut && !active}
      <span class="hint error">{i18n.t.prefs.shortcutTaken}</span>
    {/if}
  </span>
  <input class="field shortcut" spellcheck="false" bind:value={shortcut} onchange={saveShortcut} />
</label>

<label class="row">
  <span>
    <span class="name">{i18n.t.prefs.habits}</span>
    <span class="hint">{i18n.t.prefs.habitsHint}</span>
  </span>
  <input type="checkbox" bind:checked={habits} onchange={toggleHabits} />
</label>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}
{#each failures as failure (failure.source)}
  <p class="error" role="alert">{i18n.t.prefs.syncFailed} ({failure.source}) : {failure.message}</p>
{/each}

<Sources />
<Models />

<style>
  h1 {
    margin: 0 0 16px;
    font-size: 24px;
    font-weight: 600;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    max-width: 640px;
    padding: 14px 0;
    border-bottom: 1px solid var(--border);
  }
  .name {
    display: block;
    font-weight: 600;
  }
  .hint {
    display: block;
    color: var(--muted);
    font-size: 13px;
  }
  input[type='checkbox'] {
    flex: none;
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }
  .shortcut {
    flex: none;
    width: 160px;
  }
  .error {
    color: var(--danger);
  }
</style>
