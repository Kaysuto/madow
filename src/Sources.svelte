<script lang="ts">
  import { onMount } from 'svelte'
  import { type SourceKind, type SourceView, deleteSource, listSources, saveSource, syncNow } from './lib/api'
  import { i18n } from './lib/i18n.svelte'

  const KINDS: SourceKind[] = ['github', 'gitlab', 'linear', 'jira', 'ical', 'imap', 'feed', 'server', 'telegram']
  /** Servers of common mail services, offered while typing. Any other IMAP server works too. */
  const MAIL_SERVERS = ['imap.gmail.com', 'imap.mail.me.com', 'imap.fastmail.com', 'imap.mail.yahoo.com', 'imap.gmx.com']
  /** Calendars have no brand: any service gives an iCal address. */
  const label = (kind: SourceKind) =>
    ({
      github: 'GitHub',
      gitlab: 'GitLab',
      linear: 'Linear',
      jira: 'Jira',
      ical: `${i18n.t.spaces.calendar} (iCal)`,
      imap: `${i18n.t.kinds.mail} (IMAP)`,
      feed: `${i18n.t.spaces.news} (RSS, Atom)`,
      server: i18n.t.sources.serverKind,
      telegram: 'Telegram',
    })[kind]

  let sources = $state<SourceView[]>([])
  let kind = $state<SourceKind>('github')
  let name = $state('GitHub')
  let token = $state('')
  let host = $state('')
  let port = $state(993)
  let username = $state('')
  let smtpHost = $state('')
  let smtpPort = $state(465)
  let instance = $state('') // address of a GitLab that is not gitlab.com, or of a Jira site

  /** Most services name the server that sends after the one that reads. */
  function suggestSmtp() {
    if (host.trim().startsWith('imap.')) smtpHost = `smtp.${host.trim().slice(5)}`
  }
  let busy = $state(false)
  let error = $state('')
  let armed = $state('') // id of the source whose removal waits for a second click

  $effect(() => {
    // A source that has no brand is named after what it brings; the user renames it.
    name = { ical: i18n.t.spaces.calendar, imap: i18n.t.kinds.mail, feed: i18n.t.spaces.news, server: i18n.t.sources.serverKind }[kind as string] ?? label(kind)
    token = ''
  })

  async function guard(action: () => Promise<void>) {
    busy = true
    try {
      await action()
      error = ''
    } catch (cause) {
      error = String(cause)
    } finally {
      busy = false
    }
  }

  /** Syncs, then reads the sources again so each shows whether it worked. */
  const sync = () =>
    guard(async () => {
      await syncNow()
      sources = await listSources()
    })

  onMount(() => guard(async () => void (sources = await listSources())))

  const add = () =>
    guard(async () => {
      const options =
        kind === 'imap'
          ? { host: host.trim(), port, username: username.trim(), smtpHost: smtpHost.trim(), smtpPort }
          : kind === 'gitlab'
            ? { url: instance.trim() }
            : kind === 'jira'
              ? { url: instance.trim(), email: username.trim() }
              : kind === 'server'
                ? { address: instance.trim() }
                : {}
      await saveSource({ id: crypto.randomUUID(), kind, name: name.trim(), options }, token)
      token = ''
      await syncNow()
      sources = await listSources()
    })

  function remove(id: string) {
    if (armed !== id) {
      armed = id
      setTimeout(() => armed === id && (armed = ''), 4000)
      return
    }
    armed = ''
    void guard(async () => {
      await deleteSource(id)
      await syncNow()
      sources = await listSources()
    })
  }
</script>

<h2>{i18n.t.sources.title}</h2>
<p class="hint">{i18n.t.sources.intro}</p>

{#if sources.length === 0}
  <p class="hint">{i18n.t.sources.none}</p>
{/if}
<ul>
  {#each sources as source (source.id)}
    <li>
      <span class="name">{source.name}</span>
      <span class="hint">{label(source.kind)}</span>
      {#if source.error}
        <span class="error" role="alert">{source.error}</span>
      {/if}
      <button class="btn" onclick={() => remove(source.id)}>
        {armed === source.id ? i18n.t.sources.confirmRemove : i18n.t.sources.remove}
      </button>
    </li>
  {/each}
</ul>

<form
  onsubmit={event => {
    event.preventDefault()
    void add()
  }}
>
  <select class="field" aria-label={i18n.t.sources.service} bind:value={kind}>
    {#each KINDS as option (option)}
      <option value={option}>{label(option)}</option>
    {/each}
  </select>
  <input class="field" placeholder={i18n.t.sources.name} aria-label={i18n.t.sources.name} required bind:value={name} />
  {#if kind === 'imap'}
    <input
      class="field wide"
      placeholder={i18n.t.sources.server}
      aria-label={i18n.t.sources.server}
      list="mail-servers"
      required
      bind:value={host}
      onchange={suggestSmtp}
    />
    <datalist id="mail-servers">
      {#each MAIL_SERVERS as server (server)}
        <option value={server}></option>
      {/each}
    </datalist>
    <input
      class="field port"
      type="number"
      min="1"
      max="65535"
      aria-label={i18n.t.sources.port}
      required
      bind:value={port}
    />
    <input
      class="field wide"
      type="email"
      autocomplete="off"
      placeholder={i18n.t.sources.user}
      aria-label={i18n.t.sources.user}
      required
      bind:value={username}
    />
    <input
      class="field wide"
      placeholder={i18n.t.sources.smtp}
      aria-label={i18n.t.sources.smtp}
      bind:value={smtpHost}
    />
    <input
      class="field port"
      type="number"
      min="1"
      max="65535"
      aria-label={i18n.t.sources.smtpPort}
      required
      bind:value={smtpPort}
    />
  {/if}
  {#if kind === 'gitlab' || kind === 'jira' || kind === 'server'}
    <input
      class="field wide"
      type={kind === 'server' ? 'text' : 'url'}
      placeholder={i18n.t.sources.instance[kind]}
      aria-label={i18n.t.sources.instance[kind]}
      required={kind !== 'gitlab'}
      bind:value={instance}
    />
  {/if}
  {#if kind === 'jira'}
    <input
      class="field wide"
      type="email"
      autocomplete="off"
      placeholder={i18n.t.sources.user}
      aria-label={i18n.t.sources.user}
      required
      bind:value={username}
    />
  {/if}
  <!-- A server to watch has no secret: its address is all there is. -->
  <input
    class="field wide"
    hidden={kind === 'server'}
    type={kind === 'feed' ? 'url' : 'password'}
    autocomplete="off"
    placeholder={i18n.t.sources.token[kind]}
    aria-label={i18n.t.sources.token[kind]}
    required={kind !== 'github' && kind !== 'server'}
    bind:value={token}
  />
  <button class="btn" type="submit" disabled={busy}>{i18n.t.sources.add}</button>
  <button class="btn" type="button" disabled={busy} onclick={sync}>{i18n.t.sources.sync}</button>
</form>
<p class="hint">{i18n.t.sources.hints[kind]}</p>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

<style>
  h2 {
    margin: 32px 0 4px;
    font-size: 18px;
    font-weight: 600;
  }
  p {
    max-width: 760px;
    margin: 6px 0;
  }
  .hint {
    color: var(--muted);
    font-size: 13px;
  }
  .name {
    font-weight: 600;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li,
  form {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    max-width: 760px;
  }
  li {
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
  }
  li .btn {
    margin-left: auto;
  }
  form {
    margin-top: 12px;
  }
  .wide {
    flex: 1 1 260px;
  }
  .port {
    width: 84px;
  }
  .error {
    color: var(--danger);
    font-size: 13px;
  }
</style>
