<script lang="ts">
  import { onMount } from 'svelte'
  import { type Item, type Rule, RULE_KINDS, deleteRule, listItems, listRules, onItemsChanged, saveRule } from '../lib/api'
  import { i18n } from '../lib/i18n.svelte'
  import { relativeTime } from '../lib/time'

  /** How many matching items the preview asks for; that many found is shown as "50+". */
  const PREVIEW = 50

  const blank = (): Rule => ({
    id: crypto.randomUUID(),
    name: '',
    enabled: true,
    kind: 'mail',
    search: '',
    task: false,
    notify: true,
    triage: null,
    runs: 0,
    lastRun: null,
  })

  let rules = $state<Rule[]>([])
  let draft = $state<Rule>(blank())
  let editing = $state(false) // the draft is a rule that exists, not a new one
  let matching = $state<Item[] | null>(null) // null until the draft is about something
  let error = $state('')
  let armed = $state('') // id of the rule whose removal waits for a second click
  const now = Date.now() / 1000

  async function guard(action: () => Promise<void>) {
    try {
      await action()
      error = ''
    } catch (cause) {
      error = String(cause)
    }
  }

  const load = () => guard(async () => void (rules = await listRules()))

  onMount(() => {
    void load()
    // A sync may have run the rules: their counts are read again.
    const listening = onItemsChanged(load)
    return () => void listening.then(stop => stop())
  })

  /** What a rule is about and what it does, in a line. */
  const about = (rule: Rule) =>
    [rule.kind ? i18n.t.kinds[rule.kind] : i18n.t.rules.any, rule.search && `« ${rule.search} »`]
      .filter(Boolean)
      .join(' · ')
  const effects = (rule: Rule) =>
    [rule.task && i18n.t.rules.task, rule.notify && i18n.t.rules.notify, rule.triage && i18n.t.rules.triage[rule.triage]]
      .filter(Boolean)
      .join(', ')

  // What the draft would act on among the items already here, to check its wording.
  $effect(() => {
    const { kind, search } = draft
    if (!kind && !search.trim()) {
      matching = null
      return
    }
    let latest = true
    const waiting = setTimeout(async () => {
      try {
        const found = await listItems({ kinds: kind ? [kind] : RULE_KINDS, search, limit: PREVIEW })
        // What the user wrote here never arrives.
        if (latest) matching = found.filter(item => item.source !== 'local')
      } catch {
        if (latest) matching = null
      }
    }, 150)
    return () => {
      latest = false
      clearTimeout(waiting)
    }
  })

  const save = () =>
    guard(async () => {
      await saveRule($state.snapshot(draft))
      draft = blank()
      editing = false
      await load()
    })

  function edit(rule: Rule) {
    draft = { ...rule }
    editing = true
  }

  function cancel() {
    draft = blank()
    editing = false
  }

  const toggle = (rule: Rule, enabled: boolean) =>
    guard(async () => {
      await saveRule({ ...$state.snapshot(rule), enabled })
      await load()
    })

  function remove(id: string) {
    if (armed !== id) {
      armed = id
      setTimeout(() => armed === id && (armed = ''), 4000)
      return
    }
    armed = ''
    void guard(async () => {
      await deleteRule(id)
      if (draft.id === id) cancel()
      await load()
    })
  }
</script>

<h1>{i18n.t.spaces.automations}</h1>
<p class="hint">{i18n.t.rules.intro}</p>

{#if rules.length === 0}
  <p class="hint">{i18n.t.rules.none}</p>
{/if}
<ul class="rules">
  {#each rules as rule (rule.id)}
    <li class:off={!rule.enabled}>
      <input
        type="checkbox"
        checked={rule.enabled}
        aria-label={`${i18n.t.rules.enabled} : ${rule.name}`}
        onchange={event => toggle(rule, event.currentTarget.checked)}
      />
      <div class="what">
        <span class="name">{rule.name}</span>
        <span class="hint">{about(rule)} → {effects(rule)}</span>
        <span class="hint">
          {#if rule.lastRun}
            {i18n.t.rules.ran} {rule.runs} · {relativeTime(rule.lastRun, now, i18n.locale)}
          {:else}
            {i18n.t.rules.never}
          {/if}
        </span>
      </div>
      <button class="btn" onclick={() => edit(rule)}>{i18n.t.rules.edit}</button>
      <button class="btn" onclick={() => remove(rule.id)}>
        {armed === rule.id ? i18n.t.rules.confirmRemove : i18n.t.rules.remove}
      </button>
    </li>
  {/each}
</ul>

<form
  onsubmit={event => {
    event.preventDefault()
    void save()
  }}
>
  <input class="field" placeholder={i18n.t.rules.name} aria-label={i18n.t.rules.name} required bind:value={draft.name} />

  <fieldset>
    <legend>{i18n.t.rules.when}</legend>
    <select class="field" aria-label={i18n.t.rules.when} bind:value={draft.kind}>
      <option value={null}>{i18n.t.rules.any}</option>
      {#each RULE_KINDS as kind (kind)}
        <option value={kind}>{i18n.t.kinds[kind]}</option>
      {/each}
    </select>
    <input class="field wide" placeholder={i18n.t.rules.search} aria-label={i18n.t.rules.search} bind:value={draft.search} />
  </fieldset>

  <fieldset>
    <legend>{i18n.t.rules.then}</legend>
    <label><input type="checkbox" bind:checked={draft.task} /> {i18n.t.rules.task}</label>
    <label><input type="checkbox" bind:checked={draft.notify} /> {i18n.t.rules.notify}</label>
    <select class="field" aria-label={i18n.t.spaces.inbox} bind:value={draft.triage}>
      <option value={null}>{i18n.t.rules.triage.inbox}</option>
      <option value="done">{i18n.t.rules.triage.done}</option>
      <option value="later">{i18n.t.rules.triage.later}</option>
    </select>
  </fieldset>

  <div class="actions">
    <button class="btn primary" type="submit">{editing ? i18n.t.rules.save : i18n.t.rules.add}</button>
    {#if editing}
      <button class="btn" type="button" onclick={cancel}>{i18n.t.rules.cancel}</button>
    {/if}
  </div>
</form>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if matching}
  <section aria-live="polite">
    {#if matching.length === 0}
      <p class="hint">{i18n.t.rules.noMatch}</p>
    {:else}
      <p class="hint">{i18n.t.rules.preview} {matching.length === PREVIEW ? `${PREVIEW}+` : matching.length}</p>
      <ul class="preview">
        {#each matching.slice(0, 5) as item (item.id)}
          <li>
            <span class="kind">{i18n.t.kinds[item.kind]}</span>
            <span class="title">{item.title}</span>
            {#if item.author}<span class="hint">{item.author}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  h1 {
    margin: 0 0 8px;
    font-size: 24px;
    font-weight: 600;
  }
  p,
  ul,
  form {
    max-width: 760px;
  }
  p {
    margin: 6px 0;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .hint {
    color: var(--muted);
    font-size: 13px;
  }
  .rules li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 0;
    border-bottom: 1px solid var(--border);
  }
  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    font-weight: 600;
  }
  .off .name {
    color: var(--muted);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 24px;
  }
  fieldset {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 16px;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    padding: 0 0 6px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  fieldset .wide {
    flex: 1 1 260px;
  }
  .actions {
    display: flex;
    gap: 10px;
  }
  input[type='checkbox'] {
    flex: none;
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
  }
  section {
    margin-top: 16px;
  }
  .preview li {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  .kind {
    flex: none;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
  }
  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    color: var(--danger);
  }
</style>
