//! Automations: rules of the user's own, run on what a sync brings in.
//!
//! A rule only ever acts inside Madow (a task, the inbox, a notification). It never writes to
//! a source, sends or deletes anything, so no rule needs a confirmation.

use crate::{
    error::{Error, Result},
    items::{self, Kind, Query, Triage},
    settings,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const RULES: &str = "rules";
/// The sources whose first sync is behind them: what they bring from now on is news.
const SEEN: &str = "rules.seen";

/// "When an item like this arrives, do that."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// The kind of item it is about; `None` for every kind.
    pub kind: Option<Kind>,
    /// Words to find in the title, the text or the author, read like a search. May be empty.
    #[serde(default)]
    pub search: String,
    /// Makes a task of the item.
    pub task: bool,
    /// Tells the user with a notification of the system.
    pub notify: bool,
    /// Where the item goes; `None` leaves it where it arrived.
    pub triage: Option<Triage>,
    /// How many items it has acted on, and when it last did (Unix seconds). Kept by the core.
    #[serde(default)]
    pub runs: u32,
    #[serde(default)]
    pub last_run: Option<i64>,
}

/// Something a rule wants the user told. Showing it is left to the caller: this module only
/// touches the database.
#[derive(Debug, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

fn stored<T: DeserializeOwned + Default>(conn: &Connection, key: &str) -> Result<T> {
    Ok(match settings::get(conn, key)? {
        Some(value) => serde_json::from_value(value)?,
        None => T::default(),
    })
}

pub fn list(conn: &Connection) -> Result<Vec<Rule>> {
    stored(conn, RULES)
}

/// Adds the rule, or replaces the one with the same id, whose count it keeps.
pub fn save(conn: &Connection, mut rule: Rule) -> Result<()> {
    rule.name = rule.name.trim().to_owned();
    rule.search = rule.search.trim().to_owned();
    let missing = if rule.name.is_empty() {
        Some("A rule needs a name.")
    } else if rule.kind.is_none() && rule.search.is_empty() {
        // A rule about everything is more likely a slip than a wish.
        Some("A rule needs a kind of item or words to look for.")
    } else if !rule.task && !rule.notify && rule.triage.is_none() {
        Some("A rule needs something to do.")
    } else {
        None
    };
    if let Some(message) = missing {
        return Err(Error::Message(message.into()));
    }
    let mut all = list(conn)?;
    match all.iter_mut().find(|existing| existing.id == rule.id) {
        Some(existing) => {
            (rule.runs, rule.last_run) = (existing.runs, existing.last_run);
            *existing = rule;
        }
        None => {
            (rule.runs, rule.last_run) = (0, None);
            all.push(rule);
        }
    }
    settings::set(conn, RULES, &serde_json::to_value(all)?)
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    let mut all = list(conn)?;
    all.retain(|rule| rule.id != id);
    settings::set(conn, RULES, &serde_json::to_value(all)?)
}

/// Forgets the sources that are no longer configured: one that comes back starts over.
pub fn retain_sources(conn: &Connection, sources: &[&str]) -> Result<()> {
    let seen: Vec<String> = stored(conn, SEEN)?;
    let kept: Vec<&String> = seen
        .iter()
        .filter(|source| sources.contains(&source.as_str()))
        .collect();
    if kept.len() < seen.len() {
        settings::set(conn, SEEN, &serde_json::to_value(kept)?)?;
    }
    Ok(())
}

/// Runs the rules on what a sync of `source` just brought in: `fresh` are the ids of the
/// items that were not there before. The first sync of a source is not news: everything it
/// holds would look new, and a rule would act on months of old mail at once.
pub fn arrived(conn: &Connection, source: &str, fresh: &[i64]) -> Result<Vec<Notice>> {
    let mut seen: Vec<String> = stored(conn, SEEN)?;
    if !seen.iter().any(|known| known == source) {
        seen.push(source.to_owned());
        settings::set(conn, SEEN, &serde_json::to_value(seen)?)?;
        return Ok(Vec::new());
    }
    if fresh.is_empty() {
        return Ok(Vec::new());
    }

    let mut rules = list(conn)?;
    let mut notices = Vec::new();
    let mut ran = false;
    for rule in rules.iter_mut().filter(|rule| rule.enabled) {
        let matching = items::list(
            conn,
            &Query {
                kinds: rule.kind.into_iter().collect(),
                search: rule.search.clone(),
                ids: fresh.to_vec(),
                limit: Some(1000),
                ..Query::default()
            },
        )?;
        let Some(first) = matching.first() else {
            continue;
        };
        for item in &matching {
            if rule.task {
                items::create_task_from(conn, item)?;
            }
            if let Some(triage) = rule.triage {
                items::set_triage(conn, item.id, triage)?;
            }
        }
        if rule.notify {
            // One notification per rule and per sync, however many items arrived together.
            let body = match matching.len() - 1 {
                0 => first.title.clone(),
                others => format!("{} (+{others})", first.title),
            };
            notices.push(Notice {
                title: rule.name.clone(),
                body,
            });
        }
        rule.runs = rule.runs.saturating_add(matching.len() as u32);
        rule.last_run = Some(items::now());
        ran = true;
    }
    if ran {
        settings::set(conn, RULES, &serde_json::to_value(rules)?)?;
    }
    Ok(notices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_in_memory, items::NewItem};

    fn mail(external_id: &str, title: &str, author: &str) -> NewItem {
        NewItem {
            external_id: external_id.into(),
            kind: Kind::Mail,
            title: title.into(),
            body: String::new(),
            author: Some(author.into()),
            url: Some(format!("https://mail.example/{external_id}")),
            state: "unread".into(),
            occurred_at: 10,
            ends_at: None,
            meta: serde_json::json!({}),
        }
    }

    fn rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            name: "Invoices".into(),
            enabled: true,
            kind: Some(Kind::Mail),
            search: "invoice".into(),
            task: true,
            notify: true,
            triage: Some(Triage::Done),
            runs: 0,
            last_run: None,
        }
    }

    /// Syncs `source` with `items` and runs the rules on what is new.
    fn sync(conn: &mut Connection, source: &str, items: &[NewItem]) -> Vec<Notice> {
        let fresh = items::replace_source(conn, source, items).unwrap();
        arrived(conn, source, &fresh).unwrap()
    }

    fn tasks(conn: &Connection) -> Vec<items::Item> {
        let query = Query {
            kinds: vec![Kind::Task],
            ..Query::default()
        };
        items::list(conn, &query).unwrap()
    }

    #[test]
    fn a_rule_acts_on_what_arrives_after_the_first_sync_of_a_source() {
        let mut conn = open_in_memory();
        save(&conn, rule("a")).unwrap();
        let old = mail("1", "Invoice for October", "billing@shop.example");

        assert_eq!(sync(&mut conn, "box", std::slice::from_ref(&old)), []);
        assert!(tasks(&conn).is_empty(), "old mail is left alone");

        let new = mail("2", "Your invoice for November", "billing@shop.example");
        let notices = sync(&mut conn, "box", &[old.clone(), new.clone()]);
        assert_eq!(
            notices,
            [Notice {
                title: "Invoices".into(),
                body: "Your invoice for November".into()
            }]
        );
        let made = tasks(&conn);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].title, "Your invoice for November");
        assert_eq!(made[0].url.as_deref(), Some("https://mail.example/2"));
        let inbox = Query {
            triage: Some(Triage::Inbox),
            ..Query::default()
        };
        let waiting = items::list(&conn, &inbox).unwrap();
        assert_eq!(waiting.len(), 1, "the new mail left the inbox");
        assert_eq!(waiting[0].title, "Invoice for October");

        let kept = &list(&conn).unwrap()[0];
        assert_eq!(kept.runs, 1);
        assert!(kept.last_run.is_some());

        // The same mail seen again at the next sync is no longer new.
        assert_eq!(sync(&mut conn, "box", &[old, new]), []);
        assert_eq!(tasks(&conn).len(), 1);
    }

    #[test]
    fn a_rule_leaves_alone_what_it_is_not_about() {
        let mut conn = open_in_memory();
        save(&conn, rule("a")).unwrap();
        save(
            &conn,
            Rule {
                enabled: false,
                search: String::new(),
                ..rule("off")
            },
        )
        .unwrap();
        sync(&mut conn, "box", &[]);

        let ticket = NewItem {
            kind: Kind::Ticket,
            ..mail("1", "Invoice export is broken", "sam")
        };
        let other = mail("2", "Lunch on Thursday?", "sam");
        assert_eq!(sync(&mut conn, "box", &[ticket, other]), []);
        assert!(tasks(&conn).is_empty());
        assert!(list(&conn).unwrap().iter().all(|rule| rule.runs == 0));
    }

    #[test]
    fn words_are_also_looked_for_in_the_author_and_without_accents() {
        let mut conn = open_in_memory();
        save(
            &conn,
            Rule {
                search: "helene".into(),
                ..rule("a")
            },
        )
        .unwrap();
        sync(&mut conn, "box", &[]);
        let notices = sync(&mut conn, "box", &[mail("1", "Hello", "Hélène Martin")]);
        assert_eq!(notices.len(), 1);
    }

    #[test]
    fn items_arriving_together_make_one_notification() {
        let mut conn = open_in_memory();
        save(
            &conn,
            Rule {
                task: false,
                triage: None,
                ..rule("a")
            },
        )
        .unwrap();
        sync(&mut conn, "box", &[]);
        let arriving: Vec<NewItem> = ["1", "2", "3"]
            .iter()
            .map(|id| mail(id, "Invoice", "billing@shop.example"))
            .collect();
        let notices = sync(&mut conn, "box", &arriving);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].body, "Invoice (+2)");
        assert_eq!(list(&conn).unwrap()[0].runs, 3);
        assert!(tasks(&conn).is_empty());
    }

    #[test]
    fn a_source_that_comes_back_starts_over() {
        let mut conn = open_in_memory();
        save(&conn, rule("a")).unwrap();
        sync(&mut conn, "box", &[]);
        retain_sources(&conn, &["other"]).unwrap();
        let notices = sync(&mut conn, "box", &[mail("1", "Invoice", "billing")]);
        assert_eq!(notices, []);
    }

    #[test]
    fn a_rule_must_be_named_be_about_something_and_do_something() {
        let conn = open_in_memory();
        let refused = [
            Rule {
                name: "  ".into(),
                ..rule("a")
            },
            Rule {
                kind: None,
                search: " ".into(),
                ..rule("a")
            },
            Rule {
                task: false,
                notify: false,
                triage: None,
                ..rule("a")
            },
        ];
        for rule in refused {
            assert!(save(&conn, rule).is_err());
        }
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn editing_a_rule_keeps_its_count_and_deleting_removes_it() {
        let mut conn = open_in_memory();
        save(&conn, rule("a")).unwrap();
        sync(&mut conn, "box", &[]);
        sync(&mut conn, "box", &[mail("1", "Invoice", "billing")]);

        let edited = Rule {
            name: " Bills ".into(),
            runs: 99,
            ..rule("a")
        };
        save(&conn, edited).unwrap();
        let kept = list(&conn).unwrap();
        assert_eq!((kept[0].name.as_str(), kept[0].runs), ("Bills", 1));

        delete(&conn, "a").unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}
