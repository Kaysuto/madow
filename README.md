# Madow

> The all-in-one workspace for developers, with the AI model of your choice.

**Madow** (from 窓, *mado*, "window" in Japanese) brings mail, calendar, tasks, pull requests, tickets, coding agents and news into one local-first desktop app. You pick the model: a subscription you already pay for, an API key, a free tier, or a model running on your own machine.

Early preview: Madow is usable day to day but still young, and things will move between versions.

## What works today

- **Today**: the day at a glance, with a briefing written by your model.
- **Inbox**: mail, review requests, notifications and alerts in one list, sorted by your fast model. Answer a mail with a draft you edit before sending.
- **Tasks**: your own tasks and the tickets of your trackers together.
- **Calendar**: your calendars by month, and new events written in plain words.
- **Code**: your git repositories with their state, their merge requests, tickets and security alerts, and the coding-agent sessions running on your machine. Start an agent on a ticket, in a terminal where you see it work.
- **News**: the articles of the feeds you follow, to read now, later or never.
- **Notes**: notes in Markdown, on their own or about a task, a ticket, a mail or an event, found by the search.
- **Review**: focus time by project, your commits, and a written review of the day or the week.
- **Assistant**: a chat that knows your day, with its history.
- **Focus**: a stopwatch started from any item.
- **Automations**: rules of your own on what arrives ("a mail containing invoice", "an alert"): make a task of it, get a notification, or take it out of the inbox. Madow suggests a rule when you keep doing the same thing to what a sender brings. Rules only act inside Madow.
- **Launcher**: Ctrl+K, or Alt+Space from anywhere on the machine, finds any item, jumps to a space, opens a project in your editor, captures a task or a note, or asks the assistant. The global shortcut is yours to change.
- **Status bar**: processor, memory and disk, what is playing with its buttons (Windows and Linux), the agents at work, and the servers that stopped answering.

Sources so far: any mailbox (IMAP/SMTP), calendars by iCal link, GitHub and GitLab, Linear and Jira, Telegram groups and channels through a bot, RSS and Atom feeds, and servers to watch. Models: Claude Code or Codex with your subscription, an API key (Anthropic or any OpenAI-compatible service), or a local model. Sample data is built in, to look around before connecting anything.

Your data stays on your machine. The model never gets a tool on it: Madow reads your sources itself and hands over text, and nothing is sent or created without your click.

## Install

Download the installer for your system from the [latest release](https://github.com/Kaysuto/madow/releases).

The installers are not signed yet, so your system will warn you:

- **Windows**: on the SmartScreen warning, choose "More info", then "Run anyway".
- **macOS**: open the app once, then allow it in System Settings → Privacy & Security → "Open Anyway". Pick `aarch64` for Apple Silicon, `x64` for Intel.
- **Linux**: install the `.deb` or `.rpm`, or make the `.AppImage` executable and run it.

[Version française](README.fr.md)

## Development

Requirements: [Rust](https://rustup.rs), Node.js 24, [pnpm](https://pnpm.io) and the [Tauri system dependencies](https://tauri.app/start/prerequisites/) for your platform.

```sh
pnpm install
pnpm tauri dev    # run the app with hot reload
pnpm check        # type-check the interface
pnpm test         # unit tests
```

The interface lives in `src/` (Svelte 5 + TypeScript) and the core in `src-tauri/` (Rust).

## License

[MIT](LICENSE)
