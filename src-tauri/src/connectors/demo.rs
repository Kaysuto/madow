//! Sample data, to try the app (and take screenshots) without connecting any account.

use super::Connector;
use crate::items::{Item, Kind, NewItem, now};
use async_trait::async_trait;
use chrono::{Local, TimeZone};
use serde_json::json;

pub struct Demo;

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

#[async_trait]
impl Connector for Demo {
    fn source(&self) -> &str {
        "demo"
    }

    async fn sync(&self) -> Result<Vec<NewItem>, String> {
        let now = now();
        // (id, kind, title, body, author, state, seconds from now, duration)
        let samples = [
            (
                "mr-482",
                Kind::MergeRequest,
                "Add retry with backoff to the sync worker",
                "Review requested. CI is green, 3 files changed.",
                "Noor Haddad",
                "review requested",
                -25 * MINUTE,
                None,
            ),
            (
                "message-1",
                Kind::Message,
                "Can someone review the changelog before tonight?",
                "",
                "Sam Lee · Release crew",
                "",
                -35 * MINUTE,
                None,
            ),
            (
                "mail-1",
                Kind::Mail,
                "Invoice #2041 for November",
                "Your invoice is attached. Payment is due within 30 days.",
                "Billing at Northwind",
                "unread",
                -50 * MINUTE,
                None,
            ),
            (
                "mail-3",
                Kind::Mail,
                "This week: 5 articles on async Rust",
                "Cancellation safety, structured concurrency, and what changed in the last release.",
                "Rust Digest",
                "unread",
                -7 * HOUR,
                None,
            ),
            (
                "mail-4",
                Kind::Mail,
                "Last days: 40% off every annual plan",
                "Upgrade before Sunday and save on storage, bandwidth and support.",
                "Cloudstead",
                "read",
                -9 * HOUR,
                None,
            ),
            (
                "event-1",
                Kind::Event,
                "Weekly sync",
                "Agenda: release checklist, open incidents, planning.",
                "Team calendar",
                "confirmed",
                2 * HOUR,
                Some(30 * MINUTE),
            ),
            (
                "ticket-77",
                Kind::Ticket,
                "Search returns stale results after an edit",
                "Steps to reproduce are in the first comment. Affects the desktop app only.",
                "Imani Okafor",
                "in progress",
                -3 * HOUR,
                None,
            ),
            (
                "alert-1",
                Kind::Alert,
                "api-eu-2 is not responding",
                "No answer on port 443 for the last 4 minutes.",
                "Servers",
                "down",
                -4 * MINUTE,
                None,
            ),
            (
                "mail-2",
                Kind::Mail,
                "Re: API rate limits for the partner integration",
                "Thanks for the details. Could we raise the limit to 600 requests per minute?",
                "Tomás Rivera",
                "unread",
                -5 * HOUR,
                None,
            ),
            (
                "notif-1",
                Kind::Notification,
                "You were mentioned in \"Drop Node 20 support\"",
                "Can you confirm the desktop build no longer needs it?",
                "Kenji Watanabe",
                "unread",
                -7 * HOUR,
                None,
            ),
            (
                "task-1",
                Kind::Task,
                "Write the release notes for 0.4",
                "",
                "",
                "todo",
                -20 * HOUR,
                None,
            ),
            (
                "article-3",
                Kind::Article,
                "What changed in the 2.4 release",
                "Faster cold start, a new plugin API, and two breaking changes to plan for.",
                "Framework releases",
                "unread",
                -3 * HOUR,
                None,
            ),
            (
                "article-2",
                Kind::Article,
                "A practical guide to cancellation in async code",
                "What happens when a task is dropped half-way, and how to make it safe.",
                "Weekly reading",
                "unread",
                -8 * HOUR,
                None,
            ),
            (
                "article-1",
                Kind::Article,
                "SQLite's full-text search, explained with examples",
                "How tokenizers, prefix queries and ranking work together.",
                "Weekly reading",
                "unread",
                -26 * HOUR,
                None,
            ),
        ];
        let mut items: Vec<NewItem> = samples
            .into_iter()
            .map(|(id, kind, title, body, author, state, offset, duration)| {
                let occurred_at = now + offset;
                NewItem {
                    external_id: id.into(),
                    kind,
                    title: title.into(),
                    body: body.into(),
                    author: (!author.is_empty()).then(|| author.to_owned()),
                    url: None,
                    state: state.into(),
                    occurred_at,
                    ends_at: duration.map(|d: i64| occurred_at + d),
                    // Trackers say how far along a ticket is; the Tasks space groups by it.
                    meta: match kind {
                        Kind::Ticket => json!({
                            "identifier": "APP-77",
                            "stateType": "started",
                            "project": "Desktop app",
                            "repository": "acme/desktop",
                        }),
                        Kind::MergeRequest => json!({
                            "repository": "acme/sync",
                            "checks": "SUCCESS",
                        }),
                        Kind::Event => json!({
                            "allDay": false,
                            "location": "Video call",
                            "link": "https://meet.example.com/weekly-sync",
                        }),
                        // A reply can be drafted on a sample mail, never sent.
                        Kind::Mail => json!({
                            "actions": ["reply"],
                            "replyTo": "sender@example.com",
                        }),
                        // A server that stopped answering, as the status bar counts them.
                        Kind::Alert => json!({ "server": true }),
                        _ => json!({}),
                    },
                }
            })
            .collect();

        // A few more days of calendar, anchored on local midnight like real whole-day events.
        let midnight = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|start| Local.from_local_datetime(&start).earliest())
            .map_or(now, |start| start.timestamp());
        let event =
            |id: &str, title: &str, start: i64, length: i64, all_day: bool, place: &str| NewItem {
                external_id: id.into(),
                kind: Kind::Event,
                title: title.into(),
                body: String::new(),
                author: None,
                url: None,
                state: "confirmed".into(),
                occurred_at: start,
                ends_at: Some(start + length),
                meta: json!({ "allDay": all_day, "location": place, "link": null }),
            };
        // What a forge says about the dependencies of a repository.
        items.push(NewItem {
            external_id: "alerts-docs".into(),
            kind: Kind::Alert,
            title: "marked, glob-parent".into(),
            body: "1 high, 1 moderate".into(),
            author: Some("acme/docs".into()),
            url: None,
            state: "high".into(),
            occurred_at: now - 2 * DAY,
            ends_at: None,
            meta: json!({ "repository": "acme/docs", "count": 2, "serious": 1 }),
        });
        items.extend([
            event(
                "event-2",
                "Design review",
                midnight + 38 * HOUR,
                HOUR,
                false,
                "Room 4",
            ),
            event("event-3", "Release day", midnight + 3 * DAY, DAY, true, ""),
            event(
                "event-4",
                "Conference",
                midnight + 8 * DAY,
                3 * DAY,
                true,
                "Lyon",
            ),
        ]);
        Ok(items)
    }

    async fn reply(&self, _item: &Item, _text: &str) -> Result<(), String> {
        Err("This is sample data: nothing was sent.".into())
    }
}
