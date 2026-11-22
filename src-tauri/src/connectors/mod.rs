//! Connectors: each one translates a service (a mailbox, a tracker, a forge…) into items.
//! The rest of the app only ever sees items, so it never depends on a particular service.

mod demo;
mod discord;
mod feed;
mod github;
mod gitlab;
mod ical;
mod imap;
mod jira;
mod linear;
mod server;
mod smtp;
mod telegram;

use crate::{
    error::Result,
    items::{EventDraft, Item, NewItem},
    secrets, settings,
};
use async_trait::async_trait;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use time::{Date, Month, OffsetDateTime, format_description::well_known::Rfc3339};

/// No connector waits on a service longer than this.
const REQUEST_LIMIT: Duration = Duration::from_secs(30);

#[async_trait]
pub trait Connector: Send + Sync {
    /// Name of this configured source, stored on every item it produces.
    /// Unique among the configured connectors, and stable across restarts.
    fn source(&self) -> &str;

    /// Fetches everything the source currently holds. What is no longer returned is removed
    /// locally; the user's triage of what remains is kept. Errors are worded for the user.
    async fn sync(&self) -> std::result::Result<Vec<NewItem>, String>;

    /// Does something to an item in the source itself. Sources list what they can do in the
    /// item's `meta.actions`; the others refuse.
    async fn act(&self, _external_id: &str, _action: Action) -> std::result::Result<(), String> {
        Err("This source cannot do that.".into())
    }

    /// The whole text of an item, for sources that only store the start of it. `None` when
    /// the stored body is all there is.
    async fn full_text(&self, _external_id: &str) -> std::result::Result<Option<String>, String> {
        Ok(None)
    }

    /// Sends the user's answer to an item. Sources that can do it list "reply" in the item's
    /// `meta.actions`; the others refuse.
    async fn reply(&self, _item: &Item, _text: &str) -> std::result::Result<(), String> {
        Err("This source cannot do that.".into())
    }

    /// For a calendar that cannot be written to from here but has a web page that creates an
    /// event: the address of that page, filled in with `event`.
    fn event_page(&self, _event: &EventDraft) -> Option<String> {
        None
    }
}

/// What can be done to an item where it lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    MarkRead,
    /// Takes the item out of the inbox without deleting it.
    Archive,
}

/// The services a source can be. Screens never look at this: they only see item kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A forge and its issue tracker: merge requests, notifications, assigned issues.
    Github,
    /// Another forge and its issue tracker. `options` say where the instance is.
    Gitlab,
    /// A ticket tracker.
    Linear,
    /// Another ticket tracker. `options` say where the site is and who signs in.
    Jira,
    /// A mailbox, over IMAP. Its "token" is the password; `options` say where it is.
    Imap,
    /// A calendar, read from its iCalendar feed. Its "token" is the feed's secret address.
    Ical,
    /// A news feed (RSS, Atom). Its "token" is the feed's address.
    Feed,
    /// A server to keep an eye on. `options` say where it is; it has no secret.
    Server,
    /// A messaging service, read through a bot of the user's own. Its token is the bot's.
    Telegram,
    /// Another messaging service, read the same way.
    Discord,
}

/// A source the user has connected. Its token, if any, is in the keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub kind: SourceKind,
    pub name: String,
    /// What this kind of source needs besides its secret (a server, a user name…).
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Name of a source's token in the keychain.
pub fn secret_name(id: &str) -> String {
    format!("source:{id}")
}

const SOURCES: &str = "sources";

pub fn sources(conn: &Connection) -> Result<Vec<Source>> {
    Ok(match settings::get(conn, SOURCES)? {
        Some(value) => serde_json::from_value(value)?,
        None => Vec::new(),
    })
}

/// Adds the source, or replaces the one with the same id.
pub fn save_source(conn: &Connection, source: Source) -> Result<()> {
    let mut all = sources(conn)?;
    match all.iter_mut().find(|s| s.id == source.id) {
        Some(existing) => *existing = source,
        None => all.push(source),
    }
    settings::set(conn, SOURCES, &serde_json::to_value(all)?)
}

pub fn delete_source(conn: &Connection, id: &str) -> Result<()> {
    let mut all = sources(conn)?;
    all.retain(|s| s.id != id);
    settings::set(conn, SOURCES, &serde_json::to_value(all)?)
}

/// The connectors the user has turned on, each with its token.
pub fn configured(conn: &Connection) -> Result<Vec<Box<dyn Connector>>> {
    let mut connectors: Vec<Box<dyn Connector>> = Vec::new();
    if settings::flag(conn, settings::DEMO)? {
        connectors.push(Box::new(demo::Demo));
    }
    for source in sources(conn)? {
        let token = secrets::get(&secret_name(&source.id))?;
        connectors.push(match source.kind {
            SourceKind::Github => Box::new(github::Github::new(source.id, token)),
            SourceKind::Gitlab => {
                let options = serde_json::from_value(source.options).unwrap_or_default();
                Box::new(gitlab::Gitlab::new(source.id, options, token))
            }
            SourceKind::Linear => Box::new(linear::Linear::new(source.id, token)),
            SourceKind::Jira => {
                let options = serde_json::from_value(source.options).unwrap_or_default();
                Box::new(jira::Jira::new(source.id, options, token))
            }
            SourceKind::Ical => Box::new(ical::Ical::new(source.id, token)),
            SourceKind::Feed => Box::new(feed::Feed::new(source.id, token)),
            SourceKind::Telegram => Box::new(telegram::Telegram::new(source.id, token)),
            SourceKind::Discord => Box::new(discord::Discord::new(source.id, token)),
            SourceKind::Server => {
                let options = serde_json::from_value(source.options).unwrap_or_default();
                Box::new(server::Server::new(source.id, source.name, options))
            }
            SourceKind::Imap => {
                let options = serde_json::from_value(source.options).unwrap_or_default();
                Box::new(imap::Imap::new(source.id, options, token))
            }
        });
    }
    Ok(connectors)
}

// ---------- helpers shared by the connectors ----------

/// Unix seconds of a timestamp such as `2026-11-19T08:30:00Z`.
fn parse_time(text: &str) -> Option<i64> {
    Some(OffsetDateTime::parse(text, &Rfc3339).ok()?.unix_timestamp())
}

/// Unix seconds of the start (UTC) of a day such as `2026-11-19`.
fn parse_day(text: &str) -> Option<i64> {
    let mut parts = text.splitn(3, '-').map(str::parse::<i32>);
    let (year, month, day) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    let month = Month::try_from(u8::try_from(month).ok()?).ok()?;
    let date = Date::from_calendar_date(year, month, u8::try_from(day).ok()?).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp())
}

/// At most `limit` characters, cut on a character boundary.
fn excerpt(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

/// A chat message as an item shows it: its first line as the title, and the start of the whole
/// text as the body when there is more to read.
fn headline(text: &str) -> (String, String) {
    let text = text.trim();
    let title = excerpt(text.lines().next().unwrap_or_default(), 120);
    let body = if text == title {
        String::new()
    } else {
        excerpt(text, 500)
    };
    (title, body)
}

/// The address is left out of the message: for some sources it holds the secret.
fn unreachable(error: reqwest::Error) -> String {
    format!("Could not reach the service: {}", error.without_url())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn sources_are_added_replaced_and_deleted() {
        let conn = open_in_memory();
        let source = |id: &str, name: &str| Source {
            id: id.into(),
            kind: SourceKind::Linear,
            name: name.into(),
            options: serde_json::Value::Null,
        };
        save_source(&conn, source("a", "Work")).unwrap();
        save_source(&conn, source("b", "Side project")).unwrap();
        save_source(&conn, source("a", "Work, renamed")).unwrap();
        delete_source(&conn, "b").unwrap();
        assert_eq!(sources(&conn).unwrap(), [source("a", "Work, renamed")]);
    }

    #[test]
    fn reads_timestamps_and_days() {
        assert_eq!(parse_time("1970-01-01T00:01:00Z"), Some(60));
        assert_eq!(parse_time("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(parse_time("yesterday"), None);
        assert_eq!(parse_day("1970-01-02"), Some(86_400));
        assert_eq!(parse_day("1970-13-01"), None);
        assert_eq!(parse_day("soon"), None);
    }

    #[test]
    fn excerpts_never_split_a_character() {
        assert_eq!(excerpt("  été chaud  ", 3), "été");
    }
}
