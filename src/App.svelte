<script lang="ts">
  import { onMount } from 'svelte'
  import Assistant from './Assistant.svelte'
  import Focus from './Focus.svelte'
  import Launcher from './Launcher.svelte'
  import Settings from './Settings.svelte'
  import StatusBar from './StatusBar.svelte'
  import { type Item, getSetting, setSetting } from './lib/api'
  import { asked } from './lib/assistant.svelte'
  import { i18n, restoreLocale } from './lib/i18n.svelte'
  import { takeNote } from './lib/notes.svelte'
  import { SPACES, type Space } from './lib/spaces'
  import Automations from './spaces/Automations.svelte'
  import Calendar from './spaces/Calendar.svelte'
  import Code from './spaces/Code.svelte'
  import Inbox from './spaces/Inbox.svelte'
  import News from './spaces/News.svelte'
  import Notes from './spaces/Notes.svelte'
  import Review from './spaces/Review.svelte'
  import Tasks from './spaces/Tasks.svelte'
  import Today from './spaces/Today.svelte'

  type View = Space | 'settings'
  const VIEWS: readonly View[] = [...SPACES, 'settings']

  let view = $state<View>('today')
  let assistant = $state(false)
  let launcher = $state<Launcher>()

  /** Opens or closes the assistant, and remembers it. */
  function toggleAssistant() {
    assistant = !assistant
    void setSetting('assistant', assistant)
  }

  /** Opens the assistant, on `question` when there is one. */
  function ask(question: string) {
    if (!assistant) toggleAssistant()
    asked.question = question
  }

  /** Opens a view and remembers it, so the app reopens where it was left. */
  function show(next: View) {
    view = next
    void setSetting('view', next)
  }

  /** Opens the note about an item in the Notes space, starting it when there is none. */
  const note = (item: Item) => takeNote(item).then(() => show('notes'))

  function onkeydown(event: KeyboardEvent) {
    const space = SPACES[Number(event.key) - 1]
    if (event.altKey && space) {
      event.preventDefault()
      show(space)
    } else if (event.altKey && event.key.toLowerCase() === 'a') {
      event.preventDefault()
      toggleAssistant()
    }
  }

  onMount(async () => {
    await restoreLocale()
    const saved = await getSetting<string>('view')
    view = VIEWS.find(v => v === saved) ?? view
    assistant = (await getSetting<boolean>('assistant')) === true
  })

  $effect(() => {
    document.documentElement.lang = i18n.locale
  })
</script>

<svelte:window {onkeydown} />

<div class="shell" class:with-assistant={assistant}>
  <nav class="sidebar" aria-label="Madow">
    <div class="brand">
      <span class="logo" aria-hidden="true">窓</span>
      <span>Madow</span>
    </div>
    <button class="nav" onclick={() => launcher?.open()}>
      <span>{i18n.t.launcher.open}</span>
      <kbd>Ctrl+K</kbd>
    </button>
    <ul>
      {#each SPACES as space, index (space)}
        <li>
          <button
            class="nav"
            aria-current={view === space ? 'page' : undefined}
            onclick={() => show(space)}
          >
            <span>{i18n.t.spaces[space]}</span>
            <kbd>Alt+{index + 1}</kbd>
          </button>
        </li>
      {/each}
    </ul>
    <Focus />
    <button class="nav" aria-pressed={assistant} onclick={toggleAssistant}>
      <span>{i18n.t.assistant.title}</span>
      <kbd>Alt+A</kbd>
    </button>
    <button
      class="nav"
      aria-current={view === 'settings' ? 'page' : undefined}
      onclick={() => show('settings')}
    >
      {i18n.t.settings}
    </button>
  </nav>

  <main>
    {#if view === 'today'}
      <Today onshow={show} />
    {:else if view === 'inbox'}
      <Inbox onsettings={() => show('settings')} onnote={note} />
    {:else if view === 'calendar'}
      <Calendar onnote={note} />
    {:else if view === 'tasks'}
      <Tasks onnote={note} />
    {:else if view === 'code'}
      <Code />
    {:else if view === 'news'}
      <News onsettings={() => show('settings')} />
    {:else if view === 'notes'}
      <Notes />
    {:else if view === 'review'}
      <Review onsettings={() => show('settings')} />
    {:else if view === 'automations'}
      <Automations />
    {:else if view === 'settings'}
      <Settings />
    {/if}
  </main>

  {#if assistant}
    <Assistant onclose={toggleAssistant} />
  {/if}
  <StatusBar onshow={show} />
  <Launcher bind:this={launcher} onshow={show} onask={ask} />
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
    height: 100%;
  }
  .shell.with-assistant {
    grid-template-columns: 220px minmax(0, 1fr) minmax(300px, 380px);
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 12px;
    border-right: 1px solid var(--border);
    background: var(--surface);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px;
    font-size: 16px;
    font-weight: 600;
  }
  .logo {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 15px;
  }
  ul {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 8px 10px;
    border-radius: 8px;
    color: var(--muted);
    font-size: 13px;
    font-weight: 500;
    text-align: left;
  }
  .nav:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav[aria-pressed='true'] {
    color: var(--accent);
  }
  .nav[aria-current='page'] {
    background: var(--accent-soft);
    color: var(--accent);
  }
  kbd {
    font: inherit;
    font-size: 10px;
    opacity: 0.6;
  }
  main {
    padding: 32px 40px;
    overflow: auto;
  }
</style>
