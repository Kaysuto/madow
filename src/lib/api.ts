// The only door between the interface and the Rust core. Types mirror `src-tauri/src/items.rs`.
import { Channel, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'

export type Kind =
  | 'mail'
  | 'ticket'
  | 'merge_request'
  | 'notification'
  | 'event'
  | 'task'
  | 'note'
  | 'message'
  | 'alert'
  | 'article'

/** Where the user stands with an item. Kept across syncs. */
export type Triage = 'inbox' | 'done' | 'later'

/** What a mail is about, as the fast model sorted it. Kept across syncs. */
export const LABELS = ['action', 'read', 'finance', 'notification', 'promo'] as const
export type Label = (typeof LABELS)[number]

export interface Item {
  id: number
  source: string
  externalId: string
  kind: Kind
  title: string
  body: string
  author: string | null
  url: string | null
  /** State in the source's own words. */
  state: string
  triage: Triage
  /** Unix seconds: received, created or starting. */
  occurredAt: number
  endsAt: number | null
  meta: Record<string, unknown>
  /** Null until the mail has been sorted. */
  label: Label | null
}

export interface ItemQuery {
  kinds?: Kind[]
  triage?: Triage
  search?: string
  /** Unix seconds: keeps what is still going on at `since` and what starts before `until`. */
  since?: number
  until?: number
  /** Keeps the notes written about the item with this id. */
  about?: number
  limit?: number
}

/** A source that could not be synced, and why. */
export interface SyncFailure {
  source: string
  message: string
}

export const listItems = (query: ItemQuery) => invoke<Item[]>('list_items', { query })
export const setTriage = (id: number, triage: Triage) => invoke<void>('set_triage', { id, triage })
export const getSetting = <T>(key: string) => invoke<T | null>('get_setting', { key })
export const setSetting = (key: string, value: unknown) => invoke<void>('set_setting', { key, value })
export const syncNow = () => invoke<SyncFailure[]>('sync_now')

/** What arrives from outside and waits for a decision. Tasks, notes, events and articles have their own spaces. */
export const INBOX_KINDS: Kind[] = ['mail', 'merge_request', 'notification', 'ticket', 'message', 'alert']

export const createTask = (title: string) => invoke<void>('create_task', { title })
export const setTaskDone = (id: number, done: boolean) => invoke<void>('set_task_done', { id, done })
/** What a note is about (its `meta.about`), kept in the note: the item itself may be gone. */
export interface NoteSubject {
  id: number
  kind: Kind
  title: string
  url: string | null
}
/** Starts a note, about the item with the id `about` when given, and gives the note's id. */
export const createNote = (title: string, about?: number) => invoke<number>('create_note', { title, about })
export const saveNote = (id: number, title: string, body: string) => invoke<void>('save_note', { id, title, body })
/** Deletes something created in Madow: a task, an event or a note. */
export const deleteLocal = (id: number) => invoke<void>('delete_local', { id })

/**
 * An event about to be added. Dates are local: `2026-11-26T14:00`, or `2026-11-26` for a
 * whole-day event, whose `end` is its last day.
 */
export interface EventDraft {
  title: string
  start: string
  end: string
  allDay: boolean
  location: string
}

/**
 * Reads an event out of a sentence such as "lunch with Sam on Thursday at 12:30", with the
 * fast model. Nothing is created. `requestId` stops it through `cancel`.
 */
export const draftEvent = (requestId: string, text: string) =>
  invoke<EventDraft>('draft_event', { requestId, text })
/** Adds the event to the calendar kept in Madow. */
export const createEvent = (event: EventDraft) => invoke<void>('create_event', { event })
/** The page of the user's online calendar that creates this event, when it has one. */
export const eventPage = (event: EventDraft) => invoke<string | null>('event_page', { event })

/** A focus session that just ended. Times are Unix seconds; `seconds` leaves the pauses out. */
export interface FocusSession {
  itemId: number | null
  title: string
  startedAt: number
  endedAt: number
  seconds: number
}

export const logFocus = (session: FocusSession) => invoke<void>('log_focus', { session })
/** Seconds of focus in the sessions that ended at `since` (Unix seconds) or later. */
export const focusTotal = (since: number) => invoke<number>('focus_total', { since })

/** What a review looks back on: today, or the last seven days, today included. */
export type Period = 'day' | 'week'

/** The focus time one item got over a period. */
export interface FocusTime {
  itemId: number | null
  title: string
  /** The project or repository of the item, when it still exists and has one. */
  project: string | null
  seconds: number
}

/** The figures of a review, counted by the app itself. */
export interface ReviewNumbers {
  focus: {
    /** Most time first. */
    items: FocusTime[]
    /** One entry per day of the period, oldest first. `start` is Unix seconds. */
    days: { start: number; seconds: number }[]
  }
  /** Titles of the user's own tasks ticked off over the period. */
  done: string[]
  /** What the user committed over the period: the first line of each commit, by repository. */
  commits: { repository: string; commits: string[] }[]
}

export const reviewNumbers = (period: Period) => invoke<ReviewNumbers>('review_numbers', { period })

/**
 * Click handler for text written by a model: its links open in the browser, never inside the
 * app's own window.
 */
export function openLinks(event: MouseEvent) {
  const link = (event.target as HTMLElement).closest('a')
  if (!link) return
  event.preventDefault()
  openPage(link.href)
}

/** Opens a web page in the browser. Anything that is not an https address is ignored. */
export function openPage(url: string | null) {
  if (url && /^https:\/\/\S+$/.test(url)) void openUrl(url)
}

/** What a source can do to one of its items. Each item lists its own in `meta.actions`. */
export const ITEM_ACTIONS = ['mark_read', 'archive'] as const
export type ItemAction = (typeof ITEM_ACTIONS)[number]

export const actOnItem = (id: number, action: ItemAction) => invoke<void>('act_on_item', { id, action })

// ---------- code (types mirror `src-tauri/src/code.rs`) ----------

/** A git repository on this machine. */
export interface Repository {
  /** Empty for a sample repository, which is nowhere on disk. */
  path: string
  name: string
  /** The folder it sits in, when it is not itself one of the chosen folders. */
  group: string | null
  /** Empty when no branch is checked out. */
  branch: string
  /** Files changed or new since the last commit. */
  changes: number
  ahead: number
  behind: number
  /** Unix seconds. */
  lastCommit: number | null
  /** `owner/name` of the remote it is pushed to. */
  remote: string | null
}

export type Tool = 'editor' | 'terminal'

/** The repositories of the folders kept in the `code.folders` setting. */
export const listRepositories = () => invoke<Repository[]>('list_repositories')
/** Asks the user for a folder with the system's dialog. Null when they close it. */
export const pickFolder = () => invoke<string | null>('pick_folder')
export const openRepository = (path: string, tool: Tool) => invoke<void>('open_repository', { path, tool })

/** A coding agent Madow knows how to read and start (mirrors `src-tauri/src/agents.rs`). */
export type Engine = 'claude_code' | 'codex'

/** The agents installed on this machine. */
export const listAgents = () => invoke<Engine[]>('list_agents')
/** Starts an agent on a ticket, in a repository of the chosen folders, in a terminal window. */
export const launchAgent = (id: number, path: string, engine: Engine) =>
  invoke<void>('launch_agent', { id, path, engine })

/** A coding-agent session on this machine. */
export interface AgentSession {
  engine: Engine
  id: string
  /** Name of the folder the agent works in. */
  project: string
  title: string
  /** The last thing the agent wrote. */
  last: string
  /** The tool it called last, when that came after its last words. */
  tool: string | null
  /** Unix seconds. */
  updatedAt: number
  /** `idle`: it just stopped, finished or waiting for an answer. */
  state: 'working' | 'idle' | 'done'
}

/** The agent sessions of the last hours, the latest first. */
export const listAgentSessions = () => invoke<AgentSession[]>('list_agent_sessions')

/** What the machine is doing (mirrors `src-tauri/src/system.rs`). Sizes are bytes. */
export interface SystemReading {
  /** Processor use since the reading before, all cores together: 0 to 100. */
  cpu: number
  memoryUsed: number
  memoryTotal: number
  /** The disk the user's folder is on. */
  disk: { free: number; total: number } | null
}

/** Null on a system that cannot be read. */
export const systemReading = () => invoke<SystemReading | null>('system_reading')

/** What is playing on the machine (mirrors `src-tauri/src/media.rs`). */
export interface Track {
  title: string
  /** Empty when the player does not tell. */
  artist: string
  playing: boolean
}
export type MediaAction = 'toggle' | 'next' | 'previous'

/** Null when nothing is playing, or on a system that does not tell. */
export const nowPlaying = () => invoke<Track | null>('now_playing')
export const mediaControl = (action: MediaAction) => invoke<void>('media_control', { action })

/** The shortcut that calls the launcher from anywhere (mirrors `src-tauri/src/launcher.rs`). */
export interface LauncherShortcut {
  /** Written as `Alt+Space`. Empty when the user wants none. */
  shortcut: string
  /** False when another application holds it, or when this system has no such shortcuts. */
  active: boolean
}

export const launcherShortcut = () => invoke<LauncherShortcut>('launcher_shortcut')
/** Rejects when the shortcut cannot be taken; the one before then stays. */
export const setLauncherShortcut = (shortcut: string) => invoke<void>('set_launcher_shortcut', { shortcut })
/** Calls `handler` when that shortcut is pressed. Resolves to the function that stops it. */
export const onLauncherCalled = (handler: () => void): Promise<UnlistenFn> => listen('launcher-called', handler)

// ---------- sources (types mirror `src-tauri/src/connectors/mod.rs`) ----------

export type SourceKind = 'github' | 'gitlab' | 'linear' | 'jira' | 'ical' | 'imap' | 'feed' | 'server'

export interface Source {
  id: string
  kind: SourceKind
  name: string
  /** What this kind of source needs besides its secret (a mail server, a user name…). */
  options: Record<string, unknown>
}

/** A source as stored. The token itself never leaves the core. */
export interface SourceView extends Source {
  hasToken: boolean
  /** Why its last sync failed, if it did. */
  error: string | null
}

export const listSources = () => invoke<SourceView[]>('list_sources')
/** `token` replaces the stored one; leaving it out keeps the current one. */
export const saveSource = (source: Source, token?: string) => invoke<void>('save_source', { source, token })
export const deleteSource = (id: string) => invoke<void>('delete_source', { id })

// ---------- automations (types mirror `src-tauri/src/rules.rs`) ----------

/** "When an item like this arrives, do that." A rule only acts inside Madow. */
export interface Rule {
  id: string
  name: string
  enabled: boolean
  /** The kind of item it is about; null for every kind. */
  kind: Kind | null
  /** Words to find in the title, the text or the author, read like a search. May be empty. */
  search: string
  /** Makes a task of the item. */
  task: boolean
  /** Tells the user with a notification of the system. */
  notify: boolean
  /** Where the item goes; null leaves it where it arrived. */
  triage: 'done' | 'later' | null
  /** How many items it has acted on, and when it last did (Unix seconds). Kept by the core. */
  runs: number
  lastRun: number | null
}

/** The kinds a sync can bring in. Tasks and notes are written here: they never arrive. */
export const RULE_KINDS: Kind[] = [...INBOX_KINDS, 'event', 'article']

export const listRules = () => invoke<Rule[]>('list_rules')
/** Adds the rule, or replaces the one with the same id. */
export const saveRule = (rule: Rule) => invoke<void>('save_rule', { rule })
export const deleteRule = (id: string) => invoke<void>('delete_rule', { id })

// ---------- models (types mirror `src-tauri/src/llm/mod.rs`) ----------

export type ProviderKind = 'anthropic' | 'openai_compat' | 'claude_code' | 'codex'

export interface Provider {
  id: string
  kind: ProviderKind
  name: string
  /** Required for `openai_compat`; empty otherwise. */
  baseUrl: string
}

/** A provider as stored. The key itself never leaves the core. */
export interface ProviderView extends Provider {
  hasKey: boolean
  /** Subscription providers: whether their command-line tool is installed. */
  installed: boolean | null
}

export interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
}

export interface AskRequest {
  model: string
  system?: string
  messages: ChatMessage[]
}

export interface Reply {
  text: string
  inputTokens: number | null
  outputTokens: number | null
}

export const listProviders = () => invoke<ProviderView[]>('list_providers')
/** `key` replaces the stored API key; leaving it out keeps the current one. */
export const saveProvider = (provider: Provider, key?: string) =>
  invoke<void>('save_provider', { provider, key })
export const deleteProvider = (id: string) => invoke<void>('delete_provider', { id })
export const listModels = (providerId: string) => invoke<string[]>('list_models', { providerId })
/** `requestId` is any unique string; pass it to `cancel` to stop the call. */
export const ask = (requestId: string, providerId: string, request: AskRequest) =>
  invoke<Reply>('ask', { requestId, providerId, request })
export const cancel = (requestId: string) => invoke<void>('cancel', { requestId })
/** The channel the core hands an answer over on, piece by piece. */
function pieces(onDelta: (piece: string) => void) {
  const channel = new Channel<string>()
  channel.onmessage = onDelta
  return channel
}

/**
 * One turn of the chat, with the strong model. `onDelta` receives the answer piece by piece
 * while it is being written; the promise resolves with the whole answer.
 */
export function chat(
  requestId: string,
  lang: 'en' | 'fr',
  messages: ChatMessage[],
  onDelta: (piece: string) => void,
) {
  return invoke<Reply>('chat', { requestId, lang, messages, onDelta: pieces(onDelta) })
}

/** A conversation with the assistant, as the history lists it. `updatedAt` is Unix seconds. */
export interface ConversationSummary {
  id: number
  /** The start of the first question. */
  title: string
  updatedAt: number
}

/** The conversations kept, the one touched last first. */
export const listConversations = () => invoke<ConversationSummary[]>('list_conversations')
/** The messages of a conversation, or null when it no longer exists. */
export const conversation = (id: number) => invoke<ChatMessage[] | null>('conversation', { id })
/** Keeps a conversation as it stands and resolves to its id; `id` is null for a new one. */
export const saveConversation = (id: number | null, messages: ChatMessage[]) =>
  invoke<number>('save_conversation', { id, messages })
export const deleteConversation = (id: number) => invoke<void>('delete_conversation', { id })

/** A reply being written to a mail, kept so it survives a restart. */
export interface SavedReply {
  id: number
  intent: string
  text: string
}

/**
 * Drafts the answer to a mail with the strong model. `intent` is what the user wants to say; it
 * may be empty. `onDelta` receives the draft piece by piece. Nothing is sent.
 */
export const draftReply = (requestId: string, id: number, intent: string, onDelta: (piece: string) => void) =>
  invoke<Reply>('draft_reply', { requestId, id, intent, onDelta: pieces(onDelta) })

/** Sends the user's answer to a mail, through the mailbox it arrived in. */
export const sendReply = (id: number, text: string) => invoke<void>('send_reply', { id, text })

/**
 * Sorts the mail waiting without a label, with the fast model. Resolves to how many mails
 * got one. `requestId` stops it through `cancel`.
 */
export const sortMail = (requestId: string) => invoke<number>('sort_mail', { requestId })

/** Writes today's briefing with the strong model. `done` is what was already ticked off today. */
export const briefing = (requestId: string, lang: 'en' | 'fr', done: string[]) =>
  invoke<Reply>('briefing', { requestId, lang, done })

/** Writes the review of the day or of the last seven days with the strong model. */
export const review = (requestId: string, lang: 'en' | 'fr', period: Period) =>
  invoke<Reply>('review', { requestId, lang, period })

/** Calls `handler` whenever stored items may have changed. Resolves to the function that stops it. */
export const onItemsChanged = (handler: () => void): Promise<UnlistenFn> =>
  listen('items-changed', handler)
