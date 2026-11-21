//! Items: the single shape every source is translated into (a mail, a ticket, an event…).

use crate::error::{Error, Result};
use chrono::{Days, Local, NaiveDate, NaiveDateTime, TimeZone};
use rusqlite::{Connection, Row, params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source name of what the user creates in Madow itself. Never removed by a sync.
pub const LOCAL_SOURCE: &str = "local";

/// What an item is, whichever service it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Mail,
    Ticket,
    MergeRequest,
    Notification,
    Event,
    Task,
    Note,
    Message,
    Alert,
    Article,
}

/// Where the user stands with an item. Local only: a sync never changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Triage {
    Inbox,
    Done,
    Later,
}

/// What a mail is about, as a model sorted it. Local only: a sync never changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    /// Needs an answer or an action from the user.
    Action,
    /// Worth reading, nothing to do.
    Read,
    Finance,
    Notification,
    Promo,
}

// These enums are stored as the same lowercase text the interface receives.
macro_rules! stored_as_text {
    ($type:ty) => {
        impl rusqlite::ToSql for $type {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                let text = serde_json::to_value(self)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                Ok(text.as_str().unwrap_or_default().to_owned().into())
            }
        }
        impl rusqlite::types::FromSql for $type {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<Self> {
                serde_json::from_value(value.as_str()?.into())
                    .map_err(|e| rusqlite::types::FromSqlError::Other(Box::new(e)))
            }
        }
    };
}
stored_as_text!(Kind);
stored_as_text!(Triage);
stored_as_text!(Label);

/// An item as a connector produces it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewItem {
    /// Identifier of the item inside its source; stable across syncs.
    pub external_id: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub author: Option<String>,
    pub url: Option<String>,
    /// State in the source's own words ("unread", "in progress", "merged"…).
    pub state: String,
    /// Unix seconds: when it was received, created or starts.
    pub occurred_at: i64,
    /// Unix seconds: end of an event, due date of a task.
    pub ends_at: Option<i64>,
    /// Source-specific extras the interface may show (labels, CI status…).
    pub meta: serde_json::Value,
}

/// An item as stored and sent to the interface.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: i64,
    pub source: String,
    pub external_id: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub author: Option<String>,
    pub url: Option<String>,
    pub state: String,
    pub triage: Triage,
    pub occurred_at: i64,
    pub ends_at: Option<i64>,
    pub meta: serde_json::Value,
    /// Set by the sorting of mail; `None` until then.
    pub label: Option<Label>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Query {
    /// Empty means every kind.
    pub kinds: Vec<Kind>,
    pub triage: Option<Triage>,
    /// Words to find in title, body or author; the last one may be a prefix.
    pub search: String,
    /// Unix seconds. Keeps what is still going on at `since`, and what starts before `until`.
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Keeps the notes written about the item with this id.
    pub about: Option<i64>,
    pub limit: Option<u32>,
    /// Keeps only the items with these ids; empty means every item. Never set by the interface.
    #[serde(skip)]
    pub ids: Vec<i64>,
}

/// How the interface writes a date and time: local, to the minute.
pub const LOCAL_TIME: &str = "%Y-%m-%dT%H:%M";

/// An event the user is about to add, with its dates as the interface edits them: local
/// time (`2026-11-26T14:00`), or days (`2026-11-26`) for a whole-day event, whose `end` is
/// its last day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EventDraft {
    pub title: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: String,
}

impl EventDraft {
    /// Start and end in local time. A whole-day event ends at the midnight after its last day.
    pub fn span(&self) -> std::result::Result<(NaiveDateTime, NaiveDateTime), String> {
        let (start, end) = if self.all_day {
            let day = |text: &str| {
                NaiveDate::parse_from_str(text, "%Y-%m-%d")
                    .ok()?
                    .and_hms_opt(0, 0, 0)
            };
            (
                day(&self.start),
                day(&self.end).and_then(|end| end.checked_add_days(Days::new(1))),
            )
        } else {
            let time = |text: &str| NaiveDateTime::parse_from_str(text, LOCAL_TIME).ok();
            (time(&self.start), time(&self.end))
        };
        match (start, end) {
            (Some(start), Some(end)) if end > start => Ok((start, end)),
            (Some(_), Some(_)) => Err("The event ends before it starts.".into()),
            _ => Err("The event needs a start and an end.".into()),
        }
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Makes the stored items of `source` match `items`: adds, updates, and removes what is gone.
/// The user's triage of an item that is still there is kept. Returns the ids of the items
/// that were not there before.
pub fn replace_source(conn: &mut Connection, source: &str, items: &[NewItem]) -> Result<Vec<i64>> {
    let tx = conn.transaction()?;
    // A new row always gets an id above every id in use.
    let highest: i64 = tx.query_row("SELECT COALESCE(MAX(id), 0) FROM items", [], |row| {
        row.get(0)
    })?;
    let stamp: i64 = tx.query_row(
        "SELECT COALESCE(MAX(synced_at), 0) + 1 FROM items WHERE source = ?1",
        [source],
        |row| row.get(0),
    )?;
    {
        let mut upsert = tx.prepare(
            "INSERT INTO items (source, external_id, kind, title, body, author, url, state,
                                occurred_at, ends_at, meta, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (source, external_id) DO UPDATE SET
                kind = excluded.kind, title = excluded.title, body = excluded.body,
                author = excluded.author, url = excluded.url, state = excluded.state,
                occurred_at = excluded.occurred_at, ends_at = excluded.ends_at,
                meta = excluded.meta, synced_at = excluded.synced_at",
        )?;
        for item in items {
            upsert.execute(params![
                source,
                item.external_id,
                item.kind,
                item.title,
                item.body,
                item.author,
                item.url,
                item.state,
                item.occurred_at,
                item.ends_at,
                item.meta.to_string(),
                stamp,
            ])?;
        }
    }
    tx.execute(
        "DELETE FROM items WHERE source = ?1 AND synced_at <> ?2",
        params![source, stamp],
    )?;
    let fresh = tx
        .prepare("SELECT id FROM items WHERE source = ?1 AND id > ?2 ORDER BY id")?
        .query_map(params![source, highest], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    tx.commit()?;
    Ok(fresh)
}

/// Removes the items of every source that is no longer configured.
pub fn retain_sources(conn: &Connection, sources: &[&str]) -> Result<()> {
    let keep = sources.iter().copied().chain([LOCAL_SOURCE]);
    let marks = vec!["?"; sources.len() + 1].join(", ");
    conn.execute(
        &format!("DELETE FROM items WHERE source NOT IN ({marks})"),
        params_from_iter(keep),
    )?;
    Ok(())
}

/// Creates something of the user's own (a task, a note). It starts out of the inbox: nobody
/// has to be told about what they just wrote themselves.
pub fn create_local(conn: &Connection, kind: Kind, title: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO items (source, external_id, kind, title, state, triage, occurred_at, synced_at)
         VALUES (?1, lower(hex(randomblob(8))), ?2, ?3, 'todo', 'done', ?4, 0)",
        params![LOCAL_SOURCE, kind, title, now()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Turns something that arrived into a task of the user's own, which keeps its title and its
/// page.
pub fn create_task_from(conn: &Connection, item: &Item) -> Result<i64> {
    let id = create_local(conn, Kind::Task, &item.title)?;
    conn.execute(
        "UPDATE items SET url = ?2 WHERE id = ?1",
        params![id, item.url],
    )?;
    Ok(id)
}

/// Starts a note, about an item when `about` is one that exists. What the note is about is
/// kept inside it (`meta.about`: id, kind, title and page), so that it still says so once a
/// sync has taken the item away. A note with no title of its own takes the item's.
pub fn create_note(conn: &Connection, title: &str, about: Option<i64>) -> Result<i64> {
    let about = match about {
        Some(id) => get(conn, id)?,
        None => None,
    };
    let title = match (title.trim(), &about) {
        ("", Some(item)) => &item.title,
        (title, _) => title,
    };
    let id = create_local(conn, Kind::Note, title)?;
    if let Some(item) = &about {
        let meta = json!({ "about": {
            "id": item.id, "kind": item.kind, "title": item.title, "url": item.url,
        } });
        conn.execute(
            "UPDATE items SET meta = ?2 WHERE id = ?1",
            params![id, meta.to_string()],
        )?;
    }
    Ok(id)
}

/// Keeps what the user wrote in one of their notes. Its date becomes the time of the save:
/// the note written last comes first. A note that is gone is an error, not a silent loss.
pub fn save_note(conn: &Connection, id: i64, title: &str, body: &str) -> Result<()> {
    let saved = conn.execute(
        "UPDATE items SET title = ?4, body = ?5, occurred_at = ?6
         WHERE id = ?1 AND source = ?2 AND kind = ?3",
        params![id, LOCAL_SOURCE, Kind::Note, title.trim(), body, now()],
    )?;
    if saved == 0 {
        return Err(Error::Message("This note no longer exists.".into()));
    }
    Ok(())
}

/// Adds an event of the user's own to the calendar. Like a task, it stays out of the inbox.
pub fn create_local_event(conn: &Connection, event: &EventDraft) -> Result<i64> {
    let title = event.title.trim();
    if title.is_empty() {
        return Err(Error::Message("An event needs a title.".into()));
    }
    let (start, end) = event.span()?;
    let seconds = |time: NaiveDateTime| {
        // An hour skipped by a clock change does not exist.
        Local
            .from_local_datetime(&time)
            .earliest()
            .map(|time| time.timestamp())
            .ok_or_else(|| Error::Message("That hour does not exist on that day.".into()))
    };
    let meta = json!({ "allDay": event.all_day, "location": event.location.trim() });
    conn.execute(
        "INSERT INTO items (source, external_id, kind, title, state, triage,
                            occurred_at, ends_at, meta, synced_at)
         VALUES (?1, lower(hex(randomblob(8))), ?2, ?3, '', 'done', ?4, ?5, ?6, 0)",
        params![
            LOCAL_SOURCE,
            Kind::Event,
            title,
            seconds(start)?,
            seconds(end)?,
            meta.to_string()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Changes the state of one of the user's own items. Items of a source are left alone:
/// their state belongs to the source. When it becomes "done", the time is kept in
/// `meta.doneAt` for the reviews.
pub fn set_local_state(conn: &Connection, id: i64, state: &str) -> Result<()> {
    conn.execute(
        "UPDATE items SET state = ?3,
                meta = json_set(meta, '$.doneAt', CASE WHEN ?3 = 'done' THEN ?4 END)
         WHERE id = ?1 AND source = ?2",
        params![id, LOCAL_SOURCE, state, now()],
    )?;
    Ok(())
}

/// The user's own tasks ticked off at `since` (Unix seconds) or later.
pub fn done_since(conn: &Connection, since: i64) -> Result<Vec<Item>> {
    let tasks = list(
        conn,
        &Query {
            kinds: vec![Kind::Task],
            limit: Some(1000),
            ..Query::default()
        },
    )?;
    Ok(tasks
        .into_iter()
        .filter(|task| task.source == LOCAL_SOURCE && task.state == "done")
        .filter(|task| task.meta["doneAt"].as_i64() >= Some(since))
        .collect())
}

/// Deletes one of the user's own items. Items of a source cannot be deleted from here.
pub fn delete_local(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM items WHERE id = ?1 AND source = ?2",
        params![id, LOCAL_SOURCE],
    )?;
    Ok(())
}

/// The item with this id, if it still exists.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Item>> {
    let mut statement = conn.prepare(
        "SELECT id, source, external_id, kind, title, body, author, url,
                state, triage, occurred_at, ends_at, meta, label
         FROM items WHERE id = ?1",
    )?;
    let mut rows = statement.query_map([id], read_item)?;
    Ok(rows.next().transpose()?)
}

/// Records the new state of an item after an action on its source succeeded, without waiting
/// for the next sync to bring it back.
pub fn set_state(conn: &Connection, id: i64, state: &str) -> Result<()> {
    conn.execute(
        "UPDATE items SET state = ?2 WHERE id = ?1",
        params![id, state],
    )?;
    Ok(())
}

pub fn set_triage(conn: &Connection, id: i64, triage: Triage) -> Result<()> {
    conn.execute(
        "UPDATE items SET triage = ?2 WHERE id = ?1",
        params![id, triage],
    )?;
    Ok(())
}

pub fn set_label(conn: &Connection, id: i64, label: Label) -> Result<()> {
    conn.execute(
        "UPDATE items SET label = ?2 WHERE id = ?1",
        params![id, label],
    )?;
    Ok(())
}

/// The lowercase text an enum is stored as.
fn stored_text(value: impl Serialize) -> Result<Value> {
    let text = serde_json::to_value(value)?;
    Ok(text.as_str().unwrap_or_default().to_owned().into())
}

pub fn list(conn: &Connection, query: &Query) -> Result<Vec<Item>> {
    let mut sql = String::from(
        "SELECT items.id, source, external_id, kind, items.title, items.body, items.author, url,
                state, triage, occurred_at, ends_at, meta, label
         FROM items",
    );
    let mut values: Vec<Value> = Vec::new();
    let mut filters: Vec<String> = Vec::new();

    let search = fts_query(&query.search);
    if let Some(search) = &search {
        sql.push_str(" JOIN items_fts ON items_fts.rowid = items.id");
        filters.push("items_fts MATCH ?".into());
        values.push(search.clone().into());
    }
    if !query.kinds.is_empty() {
        filters.push(format!(
            "kind IN ({})",
            vec!["?"; query.kinds.len()].join(", ")
        ));
        for kind in &query.kinds {
            values.push(stored_text(kind)?);
        }
    }
    if let Some(triage) = query.triage {
        filters.push("triage = ?".into());
        values.push(stored_text(triage)?);
    }
    if let Some(since) = query.since {
        filters.push("COALESCE(ends_at, occurred_at) >= ?".into());
        values.push(since.into());
    }
    if let Some(until) = query.until {
        filters.push("occurred_at < ?".into());
        values.push(until.into());
    }
    if let Some(about) = query.about {
        filters.push("json_extract(meta, '$.about.id') = ?".into());
        values.push(about.into());
    }
    if !query.ids.is_empty() {
        filters.push(format!(
            "items.id IN ({})",
            vec!["?"; query.ids.len()].join(", ")
        ));
        values.extend(query.ids.iter().map(|&id| Value::from(id)));
    }
    if !filters.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&filters.join(" AND "));
    }
    // Best match first when searching, most recent first otherwise.
    sql.push_str(if search.is_some() {
        " ORDER BY rank, occurred_at DESC"
    } else {
        " ORDER BY occurred_at DESC"
    });
    sql.push_str(" LIMIT ?");
    values.push(i64::from(query.limit.unwrap_or(200).min(1000)).into());

    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), read_item)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn read_item(row: &Row<'_>) -> rusqlite::Result<Item> {
    let meta: String = row.get(12)?;
    Ok(Item {
        id: row.get(0)?,
        source: row.get(1)?,
        external_id: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        author: row.get(6)?,
        url: row.get(7)?,
        state: row.get(8)?,
        triage: row.get(9)?,
        occurred_at: row.get(10)?,
        ends_at: row.get(11)?,
        meta: serde_json::from_str(&meta).unwrap_or_default(),
        label: row.get(13)?,
    })
}

/// Turns what the user typed into a full-text query: every word must match, the last one as a
/// prefix so results follow the typing. Words are quoted, so no character is an operator.
fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect();
    let (last, rest) = words.split_last()?;
    Some(
        rest.iter()
            .cloned()
            .chain([format!("{last}*")])
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn item(external_id: &str, kind: Kind, title: &str, occurred_at: i64) -> NewItem {
        NewItem {
            external_id: external_id.into(),
            kind,
            title: title.into(),
            body: String::new(),
            author: None,
            url: None,
            state: String::new(),
            occurred_at,
            ends_at: None,
            meta: serde_json::json!({}),
        }
    }

    fn titles(conn: &Connection, query: Query) -> Vec<String> {
        list(conn, &query)
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect()
    }

    #[test]
    fn replace_adds_updates_and_removes() {
        let mut conn = open_in_memory();
        replace_source(
            &mut conn,
            "a",
            &[
                item("1", Kind::Mail, "First", 10),
                item("2", Kind::Mail, "Second", 20),
            ],
        )
        .unwrap();
        replace_source(
            &mut conn,
            "a",
            &[
                item("2", Kind::Mail, "Second, edited", 20),
                item("3", Kind::Ticket, "Third", 30),
            ],
        )
        .unwrap();
        assert_eq!(titles(&conn, Query::default()), ["Third", "Second, edited"]);
    }

    #[test]
    fn replace_keeps_the_label_of_a_mail() {
        let mut conn = open_in_memory();
        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Mail", 10)]).unwrap();
        let mail = list(&conn, &Query::default()).unwrap().pop().unwrap();
        assert_eq!(mail.label, None);
        set_label(&conn, mail.id, Label::Finance).unwrap();

        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Mail, edited", 10)]).unwrap();

        let mail = get(&conn, mail.id).unwrap().unwrap();
        assert_eq!(
            (mail.title.as_str(), mail.label),
            ("Mail, edited", Some(Label::Finance))
        );
    }

    #[test]
    fn replace_keeps_triage_and_other_sources() {
        let mut conn = open_in_memory();
        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Mail", 10)]).unwrap();
        replace_source(&mut conn, "b", &[item("1", Kind::Ticket, "Ticket", 20)]).unwrap();
        let mail = list(&conn, &Query::default()).unwrap().pop().unwrap();
        set_triage(&conn, mail.id, Triage::Done).unwrap();

        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Mail", 10)]).unwrap();

        let done = Query {
            triage: Some(Triage::Done),
            ..Query::default()
        };
        assert_eq!(titles(&conn, done), ["Mail"]);
        assert_eq!(titles(&conn, Query::default()), ["Ticket", "Mail"]);
    }

    fn draft(start: &str, end: &str, all_day: bool) -> EventDraft {
        EventDraft {
            title: " Lunch ".into(),
            start: start.into(),
            end: end.into(),
            all_day,
            location: "Bistro".into(),
        }
    }

    #[test]
    fn an_event_draft_gives_its_span_or_says_what_is_wrong() {
        let at = |text: &str| NaiveDateTime::parse_from_str(text, LOCAL_TIME).unwrap();
        assert_eq!(
            draft("2026-11-26T12:30", "2026-11-26T13:30", false).span(),
            Ok((at("2026-11-26T12:30"), at("2026-11-26T13:30")))
        );
        // A whole day ends at the midnight after its last day, across a month end too.
        assert_eq!(
            draft("2026-11-30", "2026-11-30", true).span(),
            Ok((at("2026-11-30T00:00"), at("2026-12-01T00:00")))
        );
        assert!(
            draft("2026-11-26T12:30", "2026-11-26T12:30", false)
                .span()
                .is_err()
        );
        assert!(draft("2026-11-27", "2026-11-26", true).span().is_err());
        assert!(draft("2026-11-26", "2026-11-26", false).span().is_err());
        assert!(draft("soon", "", true).span().is_err());
    }

    #[test]
    fn a_local_event_is_stored_on_its_day_and_out_of_the_inbox() {
        let conn = open_in_memory();
        let id = create_local_event(&conn, &draft("2026-11-26T12:30", "2026-11-26T13:30", false))
            .unwrap();
        let event = get(&conn, id).unwrap().unwrap();
        let local = |text: &str| {
            Local
                .from_local_datetime(&NaiveDateTime::parse_from_str(text, LOCAL_TIME).unwrap())
                .earliest()
                .unwrap()
                .timestamp()
        };
        assert_eq!(event.title, "Lunch");
        assert_eq!(event.kind, Kind::Event);
        assert_eq!(event.triage, Triage::Done);
        assert_eq!(event.occurred_at, local("2026-11-26T12:30"));
        assert_eq!(event.ends_at, Some(local("2026-11-26T13:30")));
        assert_eq!(event.meta, json!({ "allDay": false, "location": "Bistro" }));

        let mut untitled = draft("2026-11-26", "2026-11-26", true);
        untitled.title = "  ".into();
        assert!(create_local_event(&conn, &untitled).is_err());
        assert!(create_local_event(&conn, &draft("2026-11-27", "2026-11-26", true)).is_err());

        delete_local(&conn, id).unwrap();
        assert_eq!(get(&conn, id).unwrap(), None);
    }

    #[test]
    fn retain_drops_unconfigured_sources_but_never_local() {
        let mut conn = open_in_memory();
        replace_source(&mut conn, "gone", &[item("1", Kind::Mail, "Gone", 1)]).unwrap();
        replace_source(&mut conn, "kept", &[item("1", Kind::Mail, "Kept", 2)]).unwrap();
        replace_source(&mut conn, LOCAL_SOURCE, &[item("1", Kind::Task, "Mine", 3)]).unwrap();
        retain_sources(&conn, &["kept"]).unwrap();
        assert_eq!(titles(&conn, Query::default()), ["Mine", "Kept"]);
    }

    #[test]
    fn local_items_stay_out_of_the_inbox_and_can_be_finished_and_deleted() {
        let mut conn = open_in_memory();
        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Mail", 1)]).unwrap();
        let mail = list(&conn, &Query::default()).unwrap().pop().unwrap();
        let task = create_local(&conn, Kind::Task, "Call the bank").unwrap();

        let inbox = Query {
            triage: Some(Triage::Inbox),
            ..Query::default()
        };
        assert_eq!(titles(&conn, inbox), ["Mail"]);

        set_local_state(&conn, task, "done").unwrap();
        // The state of a source's item is not ours to change, nor the item ours to delete.
        set_local_state(&conn, mail.id, "done").unwrap();
        delete_local(&conn, mail.id).unwrap();
        let states: Vec<_> = list(&conn, &Query::default())
            .unwrap()
            .into_iter()
            .map(|i| (i.title, i.state))
            .collect();
        assert!(states.contains(&("Call the bank".into(), "done".into())));
        assert!(states.contains(&("Mail".into(), String::new())));

        delete_local(&conn, task).unwrap();
        assert_eq!(titles(&conn, Query::default()), ["Mail"]);
    }

    #[test]
    fn a_ticked_task_remembers_when_until_it_is_unticked() {
        let conn = open_in_memory();
        let task = create_local(&conn, Kind::Task, "Call the bank").unwrap();
        create_local(&conn, Kind::Task, "Not done").unwrap();
        let before = now();

        set_local_state(&conn, task, "done").unwrap();
        let done = done_since(&conn, before).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].title, "Call the bank");
        assert!(done_since(&conn, now() + 60).unwrap().is_empty());

        set_local_state(&conn, task, "todo").unwrap();
        assert!(done_since(&conn, before).unwrap().is_empty());
    }

    #[test]
    fn filters_by_time_keeping_what_is_still_going_on() {
        let mut conn = open_in_memory();
        let mut long = item("1", Kind::Event, "Three-day trip", 10);
        long.ends_at = Some(40);
        replace_source(
            &mut conn,
            "a",
            &[
                long,
                item("2", Kind::Event, "Before", 5),
                item("3", Kind::Event, "Inside", 25),
                item("4", Kind::Event, "After", 30),
            ],
        )
        .unwrap();
        let query = Query {
            since: Some(20),
            until: Some(30),
            ..Query::default()
        };
        assert_eq!(titles(&conn, query), ["Inside", "Three-day trip"]);
    }

    #[test]
    fn filters_by_kind() {
        let mut conn = open_in_memory();
        replace_source(
            &mut conn,
            "a",
            &[
                item("1", Kind::Mail, "Mail", 1),
                item("2", Kind::Ticket, "Ticket", 2),
                item("3", Kind::MergeRequest, "Merge request", 3),
            ],
        )
        .unwrap();
        let query = Query {
            kinds: vec![Kind::Ticket, Kind::MergeRequest],
            ..Query::default()
        };
        assert_eq!(titles(&conn, query), ["Merge request", "Ticket"]);
    }

    #[test]
    fn search_ignores_accents_and_completes_the_last_word() {
        let mut conn = open_in_memory();
        replace_source(
            &mut conn,
            "a",
            &[
                item("1", Kind::Mail, "Réunion d'équipe jeudi", 1),
                item("2", Kind::Mail, "Facture de novembre", 2),
            ],
        )
        .unwrap();
        let search = |text: &str| {
            titles(
                &conn,
                Query {
                    search: text.into(),
                    ..Query::default()
                },
            )
        };
        assert_eq!(search("reunion equ"), ["Réunion d'équipe jeudi"]);
        assert_eq!(search("fact"), ["Facture de novembre"]);
        assert!(search("absent").is_empty());
        // Characters that are operators in the search syntax are plain text here.
        assert!(search("\"novembre\" OR (").is_empty());
    }

    #[test]
    fn search_follows_edits_and_removals() {
        let mut conn = open_in_memory();
        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Alpha", 1)]).unwrap();
        replace_source(&mut conn, "a", &[item("1", Kind::Mail, "Beta", 1)]).unwrap();
        let search = |text: &str| {
            titles(
                &conn,
                Query {
                    search: text.into(),
                    ..Query::default()
                },
            )
        };
        assert!(search("alpha").is_empty());
        assert_eq!(search("beta"), ["Beta"]);
    }

    #[test]
    fn a_note_remembers_what_it_is_about_and_keeps_what_is_written_in_it() {
        let mut conn = open_in_memory();
        replace_source(
            &mut conn,
            "a",
            &[item("1", Kind::Ticket, "Fix the login", 1)],
        )
        .unwrap();
        let ticket = list(&conn, &Query::default()).unwrap()[0].id;

        let about = create_note(&conn, " ", Some(ticket)).unwrap();
        let free = create_note(&conn, "Ideas", None).unwrap();
        // An item that does not exist leaves a note about nothing.
        let lost = create_note(&conn, "Lost", Some(999)).unwrap();
        let note = get(&conn, about).unwrap().unwrap();
        assert_eq!(
            (note.kind, note.title.as_str(), note.triage),
            (Kind::Note, "Fix the login", Triage::Done)
        );
        assert_eq!(
            note.meta["about"],
            json!({ "id": ticket, "kind": "ticket", "title": "Fix the login", "url": null })
        );
        assert_eq!(get(&conn, lost).unwrap().unwrap().meta, json!({}));

        let notes_about = |id| Query {
            kinds: vec![Kind::Note],
            about: Some(id),
            ..Query::default()
        };
        assert_eq!(titles(&conn, notes_about(ticket)), ["Fix the login"]);
        assert!(titles(&conn, notes_about(free)).is_empty());

        save_note(&conn, about, " Login plan ", "Check the **token** first.").unwrap();
        let note = get(&conn, about).unwrap().unwrap();
        assert_eq!(
            (note.title.as_str(), note.body.as_str()),
            ("Login plan", "Check the **token** first.")
        );
        // The note says what it was about even after a sync took the ticket away.
        replace_source(&mut conn, "a", &[]).unwrap();
        assert_eq!(titles(&conn, notes_about(ticket)), ["Login plan"]);
        // What is written in a note is found by the search.
        let found = Query {
            search: "token".into(),
            ..Query::default()
        };
        assert_eq!(titles(&conn, found), ["Login plan"]);
    }

    #[test]
    fn only_a_note_of_the_user_can_be_saved() {
        let mut conn = open_in_memory();
        replace_source(&mut conn, "a", &[item("1", Kind::Note, "From a source", 1)]).unwrap();
        let foreign = list(&conn, &Query::default()).unwrap()[0].id;
        let task = create_local(&conn, Kind::Task, "Call the bank").unwrap();
        let note = create_note(&conn, "Mine", None).unwrap();
        delete_local(&conn, note).unwrap();

        for id in [foreign, task, note] {
            let error = save_note(&conn, id, "Changed", "text").unwrap_err();
            assert_eq!(error.to_string(), "This note no longer exists.");
        }
        let untouched = titles(&conn, Query::default());
        assert!(!untouched.contains(&"Changed".to_owned()), "{untouched:?}");
    }
}
