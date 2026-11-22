//! Telegram, through a bot of the user's own: the messages of the last day in the groups and
//! channels the bot was added to, and those written to the bot itself. Telegram gives no
//! other program a way to read a person's private chats. Read-only.

use super::{Connector, REQUEST_LIMIT, excerpt, unreachable};
use crate::{
    http::client,
    items::{Kind, NewItem},
};
use async_trait::async_trait;
use serde_json::{Value, json};

const API: &str = "https://api.telegram.org";

pub struct Telegram {
    source: String,
    api: String,
    token: Option<String>,
}

impl Telegram {
    pub fn new(source: String, token: Option<String>) -> Self {
        Self {
            source,
            api: API.into(),
            token,
        }
    }
}

#[async_trait]
impl Connector for Telegram {
    fn source(&self) -> &str {
        &self.source
    }

    async fn sync(&self) -> Result<Vec<NewItem>, String> {
        let token = self.token.as_deref().map(str::trim).unwrap_or_default();
        // The token goes into the address: nothing but what a token is made of may get there.
        let made_of = |c: char| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-');
        if token.is_empty() || !token.chars().all(made_of) {
            return Err("A bot token looks like 123456:ABC-DEF, as BotFather gave it.".into());
        }
        // Telegram keeps what a bot received for a day, and hands it out again as long as
        // nothing newer is confirmed. A negative offset asks for the latest hundred and drops
        // what came before them.
        let address = reqwest::Url::parse_with_params(
            &format!("{}/bot{token}/getUpdates", self.api),
            [
                ("offset", "-100"),
                ("allowed_updates", r#"["message","channel_post"]"#),
            ],
        )
        .map_err(|_| "The address of Telegram could not be read.")?;
        let response = client()
            .get(address)
            .timeout(REQUEST_LIMIT)
            .send()
            .await
            .map_err(unreachable)?;
        let status = response.status();
        let answer: Value = response.json().await.unwrap_or_default();
        if answer["ok"] != true {
            return Err(match answer["description"].as_str() {
                Some(description) => format!("Telegram: {description}"),
                None => format!("Telegram answered {status}"),
            });
        }
        Ok(messages(&answer["result"]))
    }
}

fn messages(updates: &Value) -> Vec<NewItem> {
    updates
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|update| {
            let message = [&update["message"], &update["channel_post"]]
                .into_iter()
                .find(|message| message.is_object())?;
            // A sticker, a photo without a caption, someone joining: nothing to read.
            let text = message["text"]
                .as_str()
                .or(message["caption"].as_str())?
                .trim();
            let chat = &message["chat"];
            let (chat_id, id) = (chat["id"].as_i64()?, message["message_id"].as_i64()?);
            let title = excerpt(text.lines().next().unwrap_or_default(), 120);
            let place = chat["title"].as_str();
            let author = match (name(&message["from"]), place) {
                (Some(person), Some(place)) => Some(format!("{person} · {place}")),
                (person, place) => person.or(place.map(str::to_owned)),
            };
            Some(NewItem {
                external_id: format!("{chat_id}:{id}"),
                kind: Kind::Message,
                body: if text == title {
                    String::new()
                } else {
                    excerpt(text, 500)
                },
                title,
                author,
                url: page(chat, chat_id, id),
                state: String::new(),
                occurred_at: message["date"].as_i64().unwrap_or(0),
                ends_at: None,
                meta: json!({ "chat": place }),
            })
        })
        .collect()
}

/// "Sam Lee", from the first and last names Telegram gives.
fn name(user: &Value) -> Option<String> {
    let first = user["first_name"].as_str()?;
    Some(match user["last_name"].as_str() {
        Some(last) => format!("{first} {last}"),
        None => first.to_owned(),
    })
}

/// The address of a message, for the chats that have one: public ones by their name, private
/// supergroups and channels by their number, written without its `-100` prefix.
fn page(chat: &Value, chat_id: i64, id: i64) -> Option<String> {
    if let Some(public) = chat["username"].as_str() {
        return Some(format!("https://t.me/{public}/{id}"));
    }
    let number = chat_id.to_string();
    let number = number.strip_prefix("-100")?;
    Some(format!("https://t.me/c/{number}/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    fn updates() -> Value {
        json!([
            { "update_id": 1, "message": {
                "message_id": 7, "date": 60,
                "from": { "id": 1, "is_bot": false, "first_name": "Sam", "last_name": "Lee" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup", "title": "Release crew" },
                "text": "Can someone review the changelog?\nIt ships tonight.",
            } },
            { "update_id": 2, "channel_post": {
                "message_id": 8, "date": 120,
                "chat": { "id": -1009_i64, "type": "channel", "title": "Status", "username": "acme_status" },
                "caption": "All systems normal",
            } },
            { "update_id": 3, "message": {
                "message_id": 9, "date": 180,
                "from": { "id": 2, "is_bot": false, "first_name": "Lou" },
                "chat": { "id": 2, "type": "private", "first_name": "Lou" },
                "text": "Ping",
            } },
            { "update_id": 4, "message": {
                "message_id": 10, "date": 240,
                "from": { "id": 2, "is_bot": false, "first_name": "Lou" },
                "chat": { "id": 2, "type": "private", "first_name": "Lou" },
                "sticker": { "emoji": "👍" },
            } },
            { "update_id": 5, "edited_message": { "message_id": 7 } },
        ])
    }

    #[test]
    fn translates_the_messages_that_have_something_to_read() {
        let items = messages(&updates());
        assert_eq!(items.len(), 3);

        let group = &items[0];
        assert_eq!(group.kind, Kind::Message);
        assert_eq!(group.external_id, "-1001234567890:7");
        assert_eq!(group.title, "Can someone review the changelog?");
        assert!(group.body.ends_with("It ships tonight."));
        assert_eq!(group.author.as_deref(), Some("Sam Lee · Release crew"));
        assert_eq!(
            group.url.as_deref(),
            Some("https://t.me/c/1234567890/7"),
            "a private supergroup is reached by its number"
        );
        assert_eq!(group.occurred_at, 60);

        let channel = &items[1];
        assert_eq!(channel.title, "All systems normal");
        assert_eq!(channel.body, "", "a single line is not said twice");
        assert_eq!(channel.author.as_deref(), Some("Status"));
        assert_eq!(channel.url.as_deref(), Some("https://t.me/acme_status/8"));

        let private = &items[2];
        assert_eq!(private.author.as_deref(), Some("Lou"));
        assert_eq!(private.url, None, "a private chat has no address");
    }

    #[tokio::test]
    async fn asks_for_the_latest_updates_and_reports_telegram_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot123:abc_DEF-1/getUpdates"))
            .and(query_param("offset", "-100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "ok": true, "result": updates() })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bot999:revoked/getUpdates"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "ok": false, "error_code": 401, "description": "Unauthorized",
            })))
            .mount(&server)
            .await;
        let telegram = |token: Option<&str>| Telegram {
            source: "bot".into(),
            api: server.uri(),
            token: token.map(str::to_owned),
        };
        assert_eq!(
            telegram(Some(" 123:abc_DEF-1 "))
                .sync()
                .await
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            telegram(Some("999:revoked")).sync().await.unwrap_err(),
            "Telegram: Unauthorized"
        );
        for token in [None, Some(""), Some("123:abc/../getMe"), Some("1?x=2")] {
            let refused = telegram(token).sync().await.unwrap_err();
            assert!(refused.contains("bot token"), "{token:?}: {refused}");
        }
    }

    #[tokio::test]
    async fn an_unreachable_service_does_not_leak_the_token() {
        let telegram = Telegram {
            source: "bot".into(),
            // Nothing listens there.
            api: "http://127.0.0.1:9".into(),
            token: Some("123:secret".into()),
        };
        let error = telegram.sync().await.unwrap_err();
        assert!(!error.contains("secret"), "{error}");
    }
}
