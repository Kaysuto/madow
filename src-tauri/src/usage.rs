//! The journal of the user's gestures on what arrives, and what it suggests.
//!
//! It stays in the local database and is only read by counting: no model sees it. A finding
//! is a proposal the user accepts or ignores; nothing here changes a rule or an item by
//! itself.

use crate::{
    error::Result,
    items::{self, Item, Kind, LOCAL_SOURCE, Triage},
    rules, settings,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// True when the user turned the journal off.
pub const OFF: &str = "usage.off";
/// The suggestions the user does not want to see again.
const IGNORED: &str = "usage.ignored";

/// The same gesture this many times in a row on what comes from an author is worth a rule.
const STREAK: u32 = 5;
/// Older gestures say nothing about today's habits, and are forgotten.
const MEMORY: i64 = 60 * 86_400;

/// "You did the same thing to the last items of this author: make it a rule?"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub kind: Kind,
    pub author: String,
    /// What the user did each time: marked as done, or set aside for later.
    pub triage: Triage,
    /// How many times in a row, up to now.
    pub count: u32,
}

pub fn enabled(conn: &Connection) -> Result<bool> {
    Ok(!settings::flag(conn, OFF)?)
}

/// Turns the journal on or off. Turning it off erases what it held.
pub fn set_enabled(conn: &Connection, enabled: bool) -> Result<()> {
    settings::set(conn, OFF, &(!enabled).into())?;
    if !enabled {
        conn.execute("DELETE FROM usage", [])?;
    }
    Ok(())
}

/// Moves an item at the user's own request, and notes the gesture. Rules move items through
/// `items::set_triage`: what a rule does is not a habit of the user.
pub fn triage(conn: &Connection, id: i64, triage: Triage) -> Result<()> {
    if let Some(item) = items::get(conn, id)? {
        record(conn, &item, triage, items::now())?;
    }
    items::set_triage(conn, id, triage)
}

/// Notes a gesture on something that arrived from someone. What the user wrote themselves,
/// and what has no author, cannot lead to a rule and is left out.
fn record(conn: &Connection, item: &Item, triage: Triage, now: i64) -> Result<()> {
    let author = item.author.as_deref().map(str::trim).unwrap_or_default();
    if author.is_empty() || item.source == LOCAL_SOURCE || !enabled(conn)? {
        return Ok(());
    }
    conn.execute("DELETE FROM usage WHERE at < ?1", [now - MEMORY])?;
    conn.execute(
        "INSERT INTO usage (at, triage, kind, author) VALUES (?1, ?2, ?3, ?4)",
        params![now, triage, item.kind, author],
    )?;
    Ok(())
}

fn ignored(conn: &Connection) -> Result<Vec<Suggestion>> {
    Ok(match settings::get(conn, IGNORED)? {
        Some(value) => serde_json::from_value(value)?,
        None => Vec::new(),
    })
}

/// Never proposes this again, however long the habit goes on.
pub fn ignore(conn: &Connection, suggestion: Suggestion) -> Result<()> {
    let mut all = ignored(conn)?;
    all.push(suggestion);
    settings::set(conn, IGNORED, &serde_json::to_value(all)?)
}

/// What the journal suggests today: an author whose last items all got the same gesture, as
/// long as no rule already does it and the user did not say no. The longest habit first.
pub fn suggestions(conn: &Connection) -> Result<Vec<Suggestion>> {
    if !enabled(conn)? {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "SELECT kind, author, triage FROM usage WHERE at >= ?1 ORDER BY at DESC, id DESC",
    )?;
    let gestures = statement.query_map([items::now() - MEMORY], |row| {
        Ok((
            row.get::<_, Kind>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Triage>(2)?,
        ))
    })?;

    // Latest gesture first: the streak of an author is how long its first gesture repeats.
    // The bool says whether the streak is still running.
    let mut streaks: HashMap<(Kind, String), (Suggestion, bool)> = HashMap::new();
    for gesture in gestures {
        let (kind, author, triage) = gesture?;
        let (streak, running) = streaks
            .entry((kind, author.to_lowercase()))
            .or_insert_with(|| {
                let count = 0;
                (
                    Suggestion {
                        kind,
                        author,
                        triage,
                        count,
                    },
                    true,
                )
            });
        if *running && streak.triage == triage {
            streak.count += 1;
        } else {
            *running = false;
        }
    }

    let (rules, ignored) = (rules::list(conn)?, ignored(conn)?);
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let mut found: Vec<Suggestion> = streaks
        .into_values()
        .map(|(streak, _)| streak)
        .filter(|found| found.count >= STREAK && found.triage != Triage::Inbox)
        .filter(|found| {
            !rules.iter().any(|rule| {
                rule.kind == Some(found.kind)
                    && rule.triage == Some(found.triage)
                    && same(&rule.search, &found.author)
            })
        })
        .filter(|found| {
            !ignored.iter().any(|no| {
                no.kind == found.kind
                    && no.triage == found.triage
                    && same(&no.author, &found.author)
            })
        })
        .collect();
    found.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.author.cmp(&b.author)));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_in_memory, items::NewItem};

    fn mail(external_id: &str, author: Option<&str>) -> NewItem {
        NewItem {
            external_id: external_id.into(),
            kind: Kind::Mail,
            title: format!("Mail {external_id}"),
            body: String::new(),
            author: author.map(str::to_owned),
            url: None,
            state: "unread".into(),
            occurred_at: 10,
            ends_at: None,
            meta: serde_json::json!({}),
        }
    }

    /// A database holding `count` mails of each author, and their ids in the same order.
    fn inbox(authors: &[(Option<&str>, usize)]) -> (Connection, Vec<i64>) {
        let mut conn = open_in_memory();
        let mails: Vec<NewItem> = authors
            .iter()
            .flat_map(|(author, count)| (0..*count).map(move |n| (author, n)))
            .enumerate()
            .map(|(index, (author, _))| mail(&index.to_string(), *author))
            .collect();
        let ids = items::replace_source(&mut conn, "box", &mails).unwrap();
        (conn, ids)
    }

    fn digest(count: u32) -> Suggestion {
        Suggestion {
            kind: Kind::Mail,
            author: "Rust Digest".into(),
            triage: Triage::Done,
            count,
        }
    }

    #[test]
    fn the_same_gesture_five_times_in_a_row_is_suggested_as_a_rule() {
        let (conn, ids) = inbox(&[(Some("Rust Digest"), 6), (Some("Sam"), 2)]);
        for id in &ids[..4] {
            triage(&conn, *id, Triage::Done).unwrap();
        }
        triage(&conn, ids[6], Triage::Done).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), [], "four is not a habit yet");

        triage(&conn, ids[4], Triage::Done).unwrap();
        triage(&conn, ids[5], Triage::Done).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), [digest(6)]);
        let inbox = items::Query {
            triage: Some(Triage::Inbox),
            ..items::Query::default()
        };
        assert_eq!(items::list(&conn, &inbox).unwrap().len(), 1);
    }

    #[test]
    fn another_gesture_breaks_the_streak() {
        let (conn, ids) = inbox(&[(Some("Rust Digest"), 6)]);
        for id in &ids[..5] {
            triage(&conn, *id, Triage::Done).unwrap();
        }
        // Taken back out: the user did not mean "always".
        triage(&conn, ids[4], Triage::Inbox).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), []);
    }

    #[test]
    fn what_has_no_author_or_is_the_users_own_is_not_noted() {
        let (conn, ids) = inbox(&[(None, 5), (Some("  "), 5)]);
        for id in &ids {
            triage(&conn, *id, Triage::Later).unwrap();
        }
        let task = items::create_local(&conn, Kind::Task, "Mine").unwrap();
        triage(&conn, task, Triage::Later).unwrap();
        let noted: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage", [], |row| row.get(0))
            .unwrap();
        assert_eq!(noted, 0);
    }

    #[test]
    fn old_gestures_are_forgotten() {
        let (conn, ids) = inbox(&[(Some("Rust Digest"), 5)]);
        let long_ago = items::now() - MEMORY - 1;
        for id in &ids[..4] {
            let item = items::get(&conn, *id).unwrap().unwrap();
            record(&conn, &item, Triage::Done, long_ago).unwrap();
        }
        triage(&conn, ids[4], Triage::Done).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), []);
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage", [], |row| row.get(0))
            .unwrap();
        assert_eq!(kept, 1);
    }

    #[test]
    fn a_rule_that_does_it_or_a_refusal_ends_the_suggestion() {
        let (conn, ids) = inbox(&[(Some("Rust Digest"), 5), (Some("Cloudstead"), 5)]);
        for id in &ids {
            triage(&conn, *id, Triage::Done).unwrap();
        }
        assert_eq!(suggestions(&conn).unwrap().len(), 2);

        ignore(&conn, digest(5)).unwrap();
        let rule = rules::Rule {
            id: "a".into(),
            name: "Cloudstead".into(),
            enabled: true,
            kind: Some(Kind::Mail),
            search: "Cloudstead".into(),
            task: false,
            notify: false,
            triage: Some(Triage::Done),
            runs: 0,
            last_run: None,
        };
        rules::save(&conn, rule).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), []);
    }

    #[test]
    fn turning_the_journal_off_erases_it_and_stops_it() {
        let (conn, ids) = inbox(&[(Some("Rust Digest"), 6)]);
        for id in &ids[..5] {
            triage(&conn, *id, Triage::Done).unwrap();
        }
        set_enabled(&conn, false).unwrap();
        triage(&conn, ids[5], Triage::Done).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), []);

        set_enabled(&conn, true).unwrap();
        assert_eq!(suggestions(&conn).unwrap(), [], "nothing was kept");
        let moved = items::get(&conn, ids[5]).unwrap().unwrap();
        assert_eq!(moved.triage, Triage::Done, "the gesture itself still works");
    }
}
