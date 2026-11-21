mod agents;
mod assistant;
mod chats;
mod code;
mod connectors;
mod db;
mod error;
mod focus;
mod http;
mod items;
mod launcher;
mod llm;
mod media;
mod rules;
mod secrets;
mod settings;
mod sync;
mod system;

use error::{Error, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Mutex, MutexGuard},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

/// How long a model may take to answer before the call is given up.
const ANSWER_LIMIT: Duration = Duration::from_secs(180);
const MODEL_LIST_LIMIT: Duration = Duration::from_secs(20);

/// What every command can reach.
pub struct State {
    db: Mutex<Connection>,
    /// Model calls in progress, by the id the interface gave them, so they can be cancelled.
    running: Mutex<HashMap<String, oneshot::Sender<()>>>,
    /// Why each source failed at the last sync. A source that is not listed is fine.
    sync_errors: Mutex<HashMap<String, String>>,
    /// Kept between readings: the processor use is what happened since the reading before.
    system: Mutex<Option<sysinfo::System>>,
}

impl State {
    fn db(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock means a command panicked mid-query; the connection itself is still valid.
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn sync_errors(&self) -> MutexGuard<'_, HashMap<String, String>> {
        self.sync_errors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn running(&self) -> MutexGuard<'_, HashMap<String, oneshot::Sender<()>>> {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------- items ----------

#[tauri::command]
fn list_items(state: tauri::State<'_, State>, query: items::Query) -> Result<Vec<items::Item>> {
    items::list(&state.db(), &query)
}

#[tauri::command]
fn set_triage(app: AppHandle, id: i64, triage: items::Triage) -> Result<()> {
    items::set_triage(&app.state::<State>().db(), id, triage)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

#[tauri::command]
async fn sync_now(app: AppHandle) -> Result<Vec<sync::Failure>> {
    sync::run(&app).await
}

#[tauri::command]
fn create_task(app: AppHandle, title: &str) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Message("A task needs a title.".into()));
    }
    items::create_local(&app.state::<State>().db(), items::Kind::Task, title)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

#[tauri::command]
fn set_task_done(app: AppHandle, id: i64, done: bool) -> Result<()> {
    let state = if done { "done" } else { "todo" };
    items::set_local_state(&app.state::<State>().db(), id, state)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

/// Starts a note, about an item when `about` is given, and gives its id for the interface to
/// open it.
#[tauri::command]
fn create_note(app: AppHandle, title: &str, about: Option<i64>) -> Result<i64> {
    let id = items::create_note(&app.state::<State>().db(), title, about)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(id)
}

#[tauri::command]
fn save_note(app: AppHandle, id: i64, title: &str, body: &str) -> Result<()> {
    items::save_note(&app.state::<State>().db(), id, title, body)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

/// Deletes something the user created in Madow: a task, an event or a note.
#[tauri::command]
fn delete_local(app: AppHandle, id: i64) -> Result<()> {
    items::delete_local(&app.state::<State>().db(), id)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

/// Records a focus session the user just finished.
#[tauri::command]
fn log_focus(state: tauri::State<'_, State>, session: focus::Session) -> Result<()> {
    focus::record(&state.db(), &session)
}

/// Seconds of focus in the sessions that ended at `since` (Unix seconds) or later.
#[tauri::command]
fn focus_total(state: tauri::State<'_, State>, since: i64) -> Result<i64> {
    focus::total_since(&state.db(), since)
}

/// Adds an event of the user's own to the calendar kept in Madow.
#[tauri::command]
fn create_event(app: AppHandle, event: items::EventDraft) -> Result<()> {
    items::create_local_event(&app.state::<State>().db(), &event)?;
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

/// The page of the user's online calendar that creates this event, already filled in.
/// `None` when none of their calendars has such a page.
#[tauri::command]
fn event_page(state: tauri::State<'_, State>, event: items::EventDraft) -> Result<Option<String>> {
    event.span()?;
    Ok(connectors::configured(&state.db())?
        .iter()
        .find_map(|connector| connector.event_page(&event)))
}

/// An item and the connector of the source it comes from.
fn item_with_connector(
    state: &State,
    id: i64,
) -> Result<(items::Item, Box<dyn connectors::Connector>)> {
    let gone = || Error::Message("This item no longer exists.".into());
    let item = items::get(&state.db(), id)?.ok_or_else(gone)?;
    let connector = connectors::configured(&state.db())?
        .into_iter()
        .find(|connector| connector.source() == item.source)
        .ok_or_else(gone)?;
    Ok((item, connector))
}

/// Sends the user's answer to a mail, through the mailbox it arrived in. Nothing calls this
/// but the user's own click on "Send".
#[tauri::command]
async fn send_reply(app: AppHandle, id: i64, text: String) -> Result<()> {
    if text.trim().is_empty() {
        return Err(Error::Message("The reply is empty.".into()));
    }
    let (mail, connector) = item_with_connector(&app.state::<State>(), id)?;
    connector.reply(&mail, &text).await?;
    Ok(())
}

/// Does something to an item where it lives (mark a mail as read, archive it…), then
/// records the result locally.
#[tauri::command]
async fn act_on_item(app: AppHandle, id: i64, action: connectors::Action) -> Result<()> {
    let state = app.state::<State>();
    let (item, connector) = item_with_connector(&state, id)?;
    connector.act(&item.external_id, action).await?;
    match action {
        connectors::Action::MarkRead => items::set_state(&state.db(), id, "read")?,
        connectors::Action::Archive => items::set_triage(&state.db(), id, items::Triage::Done)?,
    }
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(())
}

// ---------- code ----------

/// The git repositories of the folders the user chose, with the sample ones when sample data
/// is shown.
#[tauri::command]
async fn list_repositories(app: AppHandle) -> Result<Vec<code::Repository>> {
    let (folders, samples) = {
        let state = app.state::<State>();
        let db = state.db();
        (code::folders(&db)?, settings::flag(&db, settings::DEMO)?)
    };
    let mut repositories = code::list(&folders).await?;
    if samples {
        repositories.extend(code::samples());
    }
    Ok(repositories)
}

/// The coding-agent sessions of the last hours on this machine, with the sample ones when
/// sample data is shown.
// Not on the main thread: session files are read from disk.
#[tauri::command(async)]
fn list_agent_sessions(app: AppHandle) -> Result<Vec<agents::Session>> {
    let now = items::now();
    let mut sessions = agents::list(&app.path().home_dir()?, now);
    if settings::flag(&app.state::<State>().db(), settings::DEMO)? {
        sessions.extend(agents::samples(now));
    }
    Ok(sessions)
}

/// The coding agents installed on this machine.
#[tauri::command]
fn list_agents() -> Vec<agents::Engine> {
    agents::installed()
}

/// Starts a coding agent on a ticket, in one of the user's repositories, in a terminal window
/// where the user sees it work. Nothing calls this but the user's own click on "Start".
#[tauri::command]
fn launch_agent(app: AppHandle, id: i64, path: &str, engine: agents::Engine) -> Result<()> {
    let state = app.state::<State>();
    let db = state.db();
    let ticket = items::get(&db, id)?
        .filter(|item| item.kind == items::Kind::Ticket)
        .ok_or_else(|| Error::Message("This ticket no longer exists.".into()))?;
    let repository = code::repository(&db, path)?;
    let terminal = code::Tool::Terminal.command(&db)?;
    let briefs = app.path().app_cache_dir()?.join("briefs");
    agents::launch(engine, &ticket, &repository, &briefs, &terminal)
}

/// Lets the user pick a folder with the system's own dialog. `None` when they close it.
// Not on the main thread, which the dialog needs free while this waits for it.
#[tauri::command(async)]
fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let folder = app
        .dialog()
        .file()
        .blocking_pick_folder()?
        .into_path()
        .ok()?;
    Some(folder.to_string_lossy().into_owned())
}

/// Opens a repository in the user's editor or terminal.
#[tauri::command]
fn open_repository(state: tauri::State<'_, State>, path: &str, tool: code::Tool) -> Result<()> {
    code::open(&state.db(), path, tool)
}

// ---------- system ----------

/// What the machine is doing right now, for the status bar. `None` on a system that cannot
/// be read.
#[tauri::command]
fn system_reading(app: AppHandle) -> Result<Option<system::Reading>> {
    let state = app.state::<State>();
    let mut system = state
        .system
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let home = app.path().home_dir()?;
    Ok(system.as_mut().map(|system| system::read(system, &home)))
}

// Not on the main thread: both wait for the system's answer.
#[tauri::command(async)]
fn now_playing() -> Option<media::Track> {
    media::track()
}

#[tauri::command(async)]
fn media_control(action: media::Action) {
    media::act(action);
}

// ---------- sources ----------

/// A source as the interface sees it: never its token, only whether it has one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceView {
    #[serde(flatten)]
    source: connectors::Source,
    has_token: bool,
    /// Why the last sync of this source failed, if it did.
    error: Option<String>,
}

#[tauri::command]
fn list_sources(state: tauri::State<'_, State>) -> Result<Vec<SourceView>> {
    let errors = state.sync_errors().clone();
    connectors::sources(&state.db())?
        .into_iter()
        .map(|source| {
            Ok(SourceView {
                has_token: secrets::get(&connectors::secret_name(&source.id))?.is_some(),
                error: errors.get(&source.id).cloned(),
                source,
            })
        })
        .collect()
}

/// Saves a source. `token` replaces its token when given; leaving it out keeps the stored one.
#[tauri::command]
fn save_source(
    state: tauri::State<'_, State>,
    source: connectors::Source,
    token: Option<String>,
) -> Result<()> {
    if source.id.trim().is_empty() || source.name.trim().is_empty() {
        return Err(Error::Message("A source needs a name.".into()));
    }
    if let Some(token) = token.filter(|token| !token.trim().is_empty()) {
        secrets::set(&connectors::secret_name(&source.id), token.trim())?;
    }
    connectors::save_source(&state.db(), source)
}

/// Forgets a source and its token. Its items go at the next sync.
#[tauri::command]
fn delete_source(state: tauri::State<'_, State>, id: &str) -> Result<()> {
    secrets::delete(&connectors::secret_name(id))?;
    connectors::delete_source(&state.db(), id)
}

// ---------- automations ----------

#[tauri::command]
fn list_rules(state: tauri::State<'_, State>) -> Result<Vec<rules::Rule>> {
    rules::list(&state.db())
}

/// Adds a rule, or replaces the one with the same id.
#[tauri::command]
fn save_rule(state: tauri::State<'_, State>, rule: rules::Rule) -> Result<()> {
    rules::save(&state.db(), rule)
}

#[tauri::command]
fn delete_rule(state: tauri::State<'_, State>, id: &str) -> Result<()> {
    rules::delete(&state.db(), id)
}

// ---------- settings ----------

#[tauri::command]
fn get_setting(state: tauri::State<'_, State>, key: &str) -> Result<Option<serde_json::Value>> {
    settings::get(&state.db(), key)
}

#[tauri::command]
fn set_setting(state: tauri::State<'_, State>, key: &str, value: serde_json::Value) -> Result<()> {
    settings::set(&state.db(), key, &value)
}

// ---------- launcher ----------

/// The shortcut that calls the launcher, as the settings show it.
#[derive(Serialize)]
struct LauncherShortcut {
    shortcut: String,
    /// False when another application holds it, or when this system has no such shortcuts.
    active: bool,
}

#[tauri::command]
fn launcher_shortcut(app: AppHandle) -> Result<LauncherShortcut> {
    let shortcut = launcher::chosen(&app.state::<State>().db())?;
    let active = launcher::bound(&app, &shortcut);
    Ok(LauncherShortcut { shortcut, active })
}

/// Changes the shortcut that calls the launcher; an empty one removes it. A shortcut that
/// cannot be taken is refused, and the one before stays.
#[tauri::command]
fn set_launcher_shortcut(app: AppHandle, shortcut: &str) -> Result<()> {
    let shortcut = shortcut.trim();
    let before = launcher::chosen(&app.state::<State>().db())?;
    if let Err(cause) = launcher::bind(&app, shortcut) {
        let _ = launcher::bind(&app, &before);
        return Err(cause);
    }
    settings::set(
        &app.state::<State>().db(),
        launcher::SHORTCUT,
        &shortcut.into(),
    )
}

// ---------- models ----------

/// A provider as the interface sees it: never its key, only whether it has one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderView {
    #[serde(flatten)]
    provider: llm::Provider,
    has_key: bool,
    /// For subscription providers: whether their command-line tool is installed.
    installed: Option<bool>,
}

#[tauri::command]
fn list_providers(state: tauri::State<'_, State>) -> Result<Vec<ProviderView>> {
    llm::providers(&state.db())?
        .into_iter()
        .map(|provider| {
            Ok(ProviderView {
                has_key: secrets::get(&provider.id)?.is_some(),
                installed: llm::installed(provider.kind),
                provider,
            })
        })
        .collect()
}

/// Saves a provider. `key` replaces its API key when given; leaving it out keeps the stored one.
#[tauri::command]
fn save_provider(
    state: tauri::State<'_, State>,
    provider: llm::Provider,
    key: Option<String>,
) -> Result<()> {
    if provider.id.trim().is_empty() || provider.name.trim().is_empty() {
        return Err(Error::Message("A provider needs a name.".into()));
    }
    if let Some(key) = key.filter(|key| !key.trim().is_empty()) {
        secrets::set(&provider.id, key.trim())?;
    }
    llm::save_provider(&state.db(), provider)
}

#[tauri::command]
fn delete_provider(state: tauri::State<'_, State>, id: &str) -> Result<()> {
    secrets::delete(id)?;
    llm::delete_provider(&state.db(), id)
}

fn provider_with_key(app: &AppHandle, id: &str) -> Result<(llm::Provider, Option<String>)> {
    let provider = llm::providers(&app.state::<State>().db())?
        .into_iter()
        .find(|provider| provider.id == id)
        .ok_or_else(|| Error::Message("This provider no longer exists.".into()))?;
    let key = secrets::get(&provider.id)?;
    Ok((provider, key))
}

#[tauri::command]
async fn list_models(app: AppHandle, provider_id: String) -> Result<Vec<String>> {
    let (provider, key) = provider_with_key(&app, &provider_id)?;
    let models = tokio::time::timeout(MODEL_LIST_LIMIT, llm::models(&provider, key.as_deref()))
        .await
        .map_err(|_| Error::Message("The provider took too long to answer.".into()))??;
    Ok(models)
}

/// Asks a model and waits for its whole answer. `request_id` is chosen by the interface so it
/// can cancel the call with `cancel`.
#[tauri::command]
async fn ask(
    app: AppHandle,
    request_id: String,
    provider_id: String,
    request: llm::Request,
) -> Result<llm::Reply> {
    let (provider, key) = provider_with_key(&app, &provider_id)?;
    let workdir = app.path().app_cache_dir()?.join("cli");
    let call = llm::complete(&provider, key.as_deref(), &request, &workdir);
    Ok(cancellable(&app.state::<State>(), request_id, ANSWER_LIMIT, call).await?)
}

/// Writes the morning briefing from what the app knows, with the user's strong model.
/// `done` is what the user already ticked off today. Cancellable like `ask`.
#[tauri::command]
async fn briefing(
    app: AppHandle,
    request_id: String,
    lang: assistant::Lang,
    done: Vec<String>,
) -> Result<llm::Reply> {
    written(&app, request_id, |db, model| {
        assistant::briefing_request(db, lang, &done, chrono::Local::now(), model)
    })
    .await
}

/// The figures of a review, counted by the app: focus time by item and by day, and the tasks
/// ticked off, over today or the last seven days.
#[tauri::command]
async fn review_numbers(app: AppHandle, period: assistant::Period) -> Result<assistant::Review> {
    let now = chrono::Local::now();
    // The database is read and released before git is waited on.
    let (mut review, folders) = {
        let state = app.state::<State>();
        let db = state.db();
        (assistant::review(&db, period, &now)?, code::folders(&db)?)
    };
    review.commits = code::commits(&folders, period.start(&now)).await;
    Ok(review)
}

/// Writes the review of the day or of the last seven days, with the user's strong model.
/// Cancellable like `ask`.
#[tauri::command]
async fn review(
    app: AppHandle,
    request_id: String,
    lang: assistant::Lang,
    period: assistant::Period,
) -> Result<llm::Reply> {
    let now = chrono::Local::now();
    let folders = code::folders(&app.state::<State>().db())?;
    let commits = code::commits(&folders, period.start(&now)).await;
    written(&app, request_id, |db, model| {
        assistant::review_request(db, lang, period, now, model, &commits)
    })
    .await
}

/// Asks the strong model what `build` makes of the database, and waits for its whole answer.
async fn written(
    app: &AppHandle,
    request_id: String,
    build: impl FnOnce(&Connection, String) -> Result<llm::Request>,
) -> Result<llm::Reply> {
    let state = app.state::<State>();
    // The database is read and released before the model is waited on.
    let (provider, request) = {
        let db = state.db();
        let (provider, model) = llm::chosen(&db, llm::Tier::Strong)?;
        (provider, build(&db, model)?)
    };
    let key = secrets::get(&provider.id)?;
    let workdir = app.path().app_cache_dir()?.join("cli");
    let call = llm::complete(&provider, key.as_deref(), &request, &workdir);
    Ok(cancellable(&state, request_id, ANSWER_LIMIT, call).await?)
}

/// One turn of the chat: sends the conversation and hands the answer over piece by piece on
/// `on_delta` while it is being written. Returns the whole answer. Cancellable like `ask`.
#[tauri::command]
async fn chat(
    app: AppHandle,
    request_id: String,
    lang: assistant::Lang,
    messages: Vec<llm::Message>,
    on_delta: tauri::ipc::Channel<String>,
) -> Result<llm::Reply> {
    let state = app.state::<State>();
    let (provider, request) = {
        let db = state.db();
        let (provider, model) = llm::chosen(&db, llm::Tier::Strong)?;
        let request = assistant::chat_request(&db, lang, messages, chrono::Local::now(), model)?;
        (provider, request)
    };
    let key = secrets::get(&provider.id)?;
    let workdir = app.path().app_cache_dir()?.join("cli");
    // A piece that cannot be delivered means the window is gone; the answer is still returned.
    let deliver = |piece: &str| {
        let _ = on_delta.send(piece.to_owned());
    };
    let call = llm::stream(&provider, key.as_deref(), &request, &workdir, &deliver);
    Ok(cancellable(&state, request_id, ANSWER_LIMIT, call).await?)
}

/// The conversations kept with the assistant, the one touched last first.
#[tauri::command]
fn list_conversations(state: tauri::State<'_, State>) -> Result<Vec<chats::Summary>> {
    chats::list(&state.db())
}

/// The messages of a conversation. `None` when it no longer exists.
#[tauri::command]
fn conversation(state: tauri::State<'_, State>, id: i64) -> Result<Option<Vec<llm::Message>>> {
    chats::messages(&state.db(), id)
}

/// Keeps a conversation as it stands and returns its id; `id` is left out for a new one.
#[tauri::command]
fn save_conversation(
    state: tauri::State<'_, State>,
    id: Option<i64>,
    messages: Vec<llm::Message>,
) -> Result<i64> {
    chats::save(&state.db(), id, &messages)
}

#[tauri::command]
fn delete_conversation(state: tauri::State<'_, State>, id: i64) -> Result<()> {
    chats::delete(&state.db(), id)
}

/// Drafts the user's answer to a mail with the strong model, handing it over piece by piece
/// on `on_delta`. `intent` is what the user wants to say; it may be empty. The draft is only
/// text for the user to edit: nothing is sent. Cancellable like `ask`.
#[tauri::command]
async fn draft_reply(
    app: AppHandle,
    request_id: String,
    id: i64,
    intent: String,
    on_delta: tauri::ipc::Channel<String>,
) -> Result<llm::Reply> {
    let state = app.state::<State>();
    let (mail, connector) = item_with_connector(&state, id)?;
    if mail.kind != items::Kind::Mail {
        return Err(Error::Message("Only a mail can be answered.".into()));
    }
    let (provider, model) = llm::chosen(&state.db(), llm::Tier::Strong)?;
    let key = secrets::get(&provider.id)?;
    let workdir = app.path().app_cache_dir()?.join("cli");
    let deliver = |piece: &str| {
        let _ = on_delta.send(piece.to_owned());
    };
    let call = async {
        // Only the start of a mail is stored: its whole text is read from where it lives.
        let text = connector.full_text(&mail.external_id).await?;
        let text = text.as_deref().unwrap_or(&mail.body);
        let request = assistant::reply_request(&mail, text, &intent, model);
        llm::stream(&provider, key.as_deref(), &request, &workdir, &deliver).await
    };
    Ok(cancellable(&state, request_id, ANSWER_LIMIT, call).await?)
}

/// Reads a calendar event out of what the user typed, with the fast model. Nothing is
/// created: the interface shows the event for the user to check. Cancellable like `ask`.
#[tauri::command]
async fn draft_event(
    app: AppHandle,
    request_id: String,
    text: String,
) -> Result<items::EventDraft> {
    let state = app.state::<State>();
    let (provider, model) = llm::chosen(&state.db(), llm::Tier::Fast)?;
    let request = assistant::event_request(&text, chrono::Local::now(), model);
    let key = secrets::get(&provider.id)?;
    let workdir = app.path().app_cache_dir()?.join("cli");
    let attempts = async {
        // Asked a second time when the first answer cannot be read, as for the sorting.
        for _ in 0..2 {
            let reply = llm::complete(&provider, key.as_deref(), &request, &workdir).await?;
            if let Some(event) = assistant::read_event(&reply.text) {
                return Ok(event);
            }
        }
        Err("The model's answer could not be read.".to_owned())
    };
    Ok(cancellable(&state, request_id, ANSWER_LIMIT, attempts).await?)
}

/// Sorts the mail still waiting without a label, with the user's fast model, and returns how
/// many mails got one. Cancellable like `ask`.
#[tauri::command]
async fn sort_mail(app: AppHandle, request_id: String) -> Result<usize> {
    let state = app.state::<State>();
    let (provider, asked, request) = {
        let db = state.db();
        let (provider, model) = llm::chosen(&db, llm::Tier::Fast)?;
        match assistant::sort_request(&db, model)? {
            Some((asked, request)) => (provider, asked, request),
            None => return Ok(0),
        }
    };
    let key = secrets::get(&provider.id)?;
    let workdir = app.path().app_cache_dir()?.join("cli");
    let attempts = async {
        // Asked a second time when the first answer cannot be read: small models slip now and then.
        for _ in 0..2 {
            let reply = llm::complete(&provider, key.as_deref(), &request, &workdir).await?;
            let labels = assistant::read_labels(&reply.text, &asked);
            if !labels.is_empty() {
                return Ok(labels);
            }
        }
        Err(
            "The model's answer could not be read. Try again, or choose another fast model."
                .to_owned(),
        )
    };
    let labels = cancellable(&state, request_id, ANSWER_LIMIT, attempts).await?;
    for (id, label) in &labels {
        items::set_label(&state.db(), *id, *label)?;
    }
    app.emit(sync::ITEMS_CHANGED, ())?;
    Ok(labels.len())
}

/// Stops a call started by `ask`. Nothing happens if it has already ended.
#[tauri::command]
fn cancel(state: tauri::State<'_, State>, request_id: &str) {
    stop(&state, request_id);
}

fn stop(state: &State, request_id: &str) {
    if let Some(stop) = state.running().remove(request_id) {
        let _ = stop.send(());
    }
}

/// Runs `call` until it answers, is cancelled or runs out of time. Dropping it in the last two
/// cases closes the connection, or stops the command-line tool it started.
async fn cancellable<T>(
    state: &State,
    request_id: String,
    limit: Duration,
    call: impl Future<Output = std::result::Result<T, String>>,
) -> std::result::Result<T, String> {
    let (stop, stopped) = oneshot::channel();
    state.running().insert(request_id.clone(), stop);
    let outcome = tokio::select! {
        answer = tokio::time::timeout(limit, call) => {
            answer.unwrap_or_else(|_| Err("The model took too long to answer.".into()))
        }
        _ = stopped => Err("Cancelled.".into()),
    };
    state.running().remove(&request_id);
    outcome
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // A second launch brings the running window forward instead of opening another app.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            launcher::bring_forward(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = db::open(&dir.join("madow.db"))?;
            let shortcut = launcher::chosen(&db)?;
            app.manage(State {
                db: Mutex::new(db),
                running: Mutex::default(),
                sync_errors: Mutex::default(),
                system: Mutex::new(system::start()),
            });
            // Another application may hold the shortcut: Madow starts without it, and the
            // settings say so.
            let _ = launcher::bind(app.handle(), &shortcut);
            sync::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_items,
            set_triage,
            sync_now,
            create_task,
            set_task_done,
            create_note,
            save_note,
            delete_local,
            create_event,
            event_page,
            draft_event,
            log_focus,
            focus_total,
            act_on_item,
            list_repositories,
            system_reading,
            now_playing,
            media_control,
            list_agent_sessions,
            list_agents,
            launch_agent,
            pick_folder,
            open_repository,
            list_sources,
            save_source,
            delete_source,
            list_rules,
            save_rule,
            delete_rule,
            get_setting,
            set_setting,
            launcher_shortcut,
            set_launcher_shortcut,
            list_providers,
            save_provider,
            delete_provider,
            list_models,
            ask,
            briefing,
            review_numbers,
            review,
            chat,
            list_conversations,
            conversation,
            save_conversation,
            delete_conversation,
            sort_mail,
            draft_reply,
            send_reply,
            cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Madow");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{pending, ready};

    fn state() -> State {
        State {
            db: Mutex::new(db::open_in_memory()),
            running: Mutex::default(),
            sync_errors: Mutex::default(),
            system: Mutex::new(system::start()),
        }
    }

    const LONG: Duration = Duration::from_secs(60);

    #[tokio::test]
    async fn a_call_gives_its_answer_and_is_then_forgotten() {
        let state = state();
        let answer = cancellable(&state, "a".into(), LONG, ready(Ok::<_, String>(42))).await;
        assert_eq!(answer, Ok(42));
        assert!(state.running().is_empty());
    }

    #[tokio::test]
    async fn cancelling_ends_a_call_that_would_never_answer() {
        let state = state();
        let call = cancellable(&state, "a".into(), LONG, pending::<Outcome>());
        let (answer, ()) = tokio::join!(call, async { stop(&state, "a") });
        assert_eq!(answer, Err("Cancelled.".into()));
        assert!(state.running().is_empty());
    }

    #[tokio::test]
    async fn a_call_that_takes_too_long_is_given_up() {
        let state = state();
        let limit = Duration::from_millis(10);
        let answer = cancellable(&state, "a".into(), limit, pending::<Outcome>()).await;
        assert_eq!(answer, Err("The model took too long to answer.".into()));
    }

    #[tokio::test]
    async fn cancelling_an_unknown_call_does_nothing() {
        stop(&state(), "nobody");
    }

    type Outcome = std::result::Result<(), String>;
}
