//! Discord, through a bot of the user's own: the messages of the last day in the channels the
//! bot can see, on the servers it was invited to. Discord forbids reading a person's private
//! messages with another program. Read-only.

use super::{Connector, REQUEST_LIMIT, headline, unreachable};
use crate::{
    http::client,
    items::{Kind, NewItem, now},
};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{Value, json};

const API: &str = "https://discord.com/api/v10";
/// Discord asks every client to name itself this way.
const AGENT: &str = concat!(
    "DiscordBot (https://github.com/Kaysuto/madow, ",
    env!("CARGO_PKG_VERSION"),
    ")"
);

const DAY: i64 = 86_400;
/// Only the channels that spoke last are read: one request each, every few minutes.
const CHANNELS: usize = 20;
const PER_CHANNEL: &str = "20";

/// Text channels and announcement channels. Voice, forums and categories hold no messages
/// to list.
const TEXT: [i64; 2] = [0, 5];
/// Plain messages and replies. The rest is what Discord says itself (joins, pins, boosts).
const WRITTEN: [i64; 2] = [0, 19];

pub struct Discord {
    source: String,
    api: String,
    token: Option<String>,
}

impl Discord {
    pub fn new(source: String, token: Option<String>) -> Self {
        Self {
            source,
            api: API.into(),
            token,
        }
    }

    /// One call to Discord. An error keeps the status, so the caller can tell a channel that
    /// is closed to the bot from a token that is wrong.
    async fn get(&self, token: &str, path: &str) -> Result<Value, (StatusCode, String)> {
        let response = client()
            .get(format!("{}{path}", self.api))
            .timeout(REQUEST_LIMIT)
            .header("Authorization", format!("Bot {token}"))
            .header("User-Agent", AGENT)
            .send()
            .await
            .map_err(|error| (StatusCode::BAD_GATEWAY, unreachable(error)))?;
        let status = response.status();
        let answer: Value = response.json().await.unwrap_or_default();
        if status.is_success() {
            return Ok(answer);
        }
        Err((
            status,
            match answer["message"].as_str() {
                Some(message) => format!("Discord: {message}"),
                None => format!("Discord answered {status}"),
            },
        ))
    }
}

/// A channel worth reading, with what its messages are signed and linked with.
struct Channel {
    id: String,
    name: String,
    guild: String,
    guild_name: String,
    last_spoke: i64,
}

#[async_trait]
impl Connector for Discord {
    fn source(&self) -> &str {
        &self.source
    }

    async fn sync(&self) -> Result<Vec<NewItem>, String> {
        let token = self.token.as_deref().map(str::trim).unwrap_or_default();
        if token.is_empty() {
            return Err("This source has no bot token yet.".into());
        }
        let since = now() - DAY;
        let failed = |(_, message): (StatusCode, String)| message;

        let guilds = self.get(token, "/users/@me/guilds").await.map_err(failed)?;
        let mut channels = Vec::new();
        for guild in guilds.as_array().into_iter().flatten() {
            let Some(guild_id) = id(&guild["id"]) else {
                continue;
            };
            let listed = self
                .get(token, &format!("/guilds/{guild_id}/channels"))
                .await
                .map_err(failed)?;
            channels.extend(
                listed
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|channel| {
                        let last_spoke = sent_at(channel["last_message_id"].as_str()?)?;
                        (TEXT.contains(&channel["type"].as_i64()?) && last_spoke >= since)
                            .then_some(())?;
                        Some(Channel {
                            id: id(&channel["id"])?.to_owned(),
                            name: channel["name"].as_str().unwrap_or_default().to_owned(),
                            guild: guild_id.to_owned(),
                            guild_name: guild["name"].as_str().unwrap_or_default().to_owned(),
                            last_spoke,
                        })
                    }),
            );
        }
        channels.sort_by_key(|channel| std::cmp::Reverse(channel.last_spoke));
        channels.truncate(CHANNELS);

        let (mut items, mut wordless) = (Vec::new(), 0);
        for channel in &channels {
            let path = format!("/channels/{}/messages?limit={PER_CHANNEL}", channel.id);
            let listed = match self.get(token, &path).await {
                Ok(listed) => listed,
                // A channel the bot may see but not read: the others still are.
                Err((StatusCode::FORBIDDEN, _)) => continue,
                Err(other) => return Err(failed(other)),
            };
            for message in listed.as_array().into_iter().flatten() {
                match translate(message, channel, since) {
                    Some(Some(item)) => items.push(item),
                    Some(None) => wordless += 1,
                    None => {}
                }
            }
        }
        // Discord blanks the text of every message for a bot that was not allowed to read it.
        if items.is_empty() && wordless > 0 {
            return Err(
                "Discord sent the messages without their text. Turn on \"Message Content Intent\" \
                 for the bot, in the Discord developer portal."
                    .into(),
            );
        }
        Ok(items)
    }
}

/// A message written in the last day, as an item. `Some(None)` when it has no text, `None`
/// when it is too old or not something a person wrote.
fn translate(message: &Value, channel: &Channel, since: i64) -> Option<Option<NewItem>> {
    let message_id = id(&message["id"])?;
    let sent = sent_at(message_id)?;
    (sent >= since && WRITTEN.contains(&message["type"].as_i64()?)).then_some(())?;
    let text = message["content"].as_str().unwrap_or_default().trim();
    if text.is_empty() {
        return Some(None);
    }
    let author = &message["author"];
    let person = author["global_name"]
        .as_str()
        .or(author["username"].as_str())
        .unwrap_or_default();
    let (title, body) = headline(text);
    Some(Some(NewItem {
        external_id: message_id.to_owned(),
        kind: Kind::Message,
        title,
        body,
        author: Some(format!("{person} · #{}", channel.name)),
        url: Some(format!(
            "https://discord.com/channels/{}/{}/{message_id}",
            channel.guild, channel.id
        )),
        state: String::new(),
        occurred_at: sent,
        ends_at: None,
        meta: json!({ "chat": format!("{} #{}", channel.guild_name, channel.name) }),
    }))
}

/// An identifier as Discord writes them: digits only. Anything else is not put in an address.
fn id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Unix seconds at which a message was sent, read from its identifier: its high bits count
/// the milliseconds since the first second of 2015.
fn sent_at(id: &str) -> Option<i64> {
    const EPOCH: u64 = 1_420_070_400_000;
    let milliseconds = (id.parse::<u64>().ok()? >> 22).checked_add(EPOCH)?;
    i64::try_from(milliseconds / 1000).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    /// The identifier of something sent `ago` seconds ago; `n` tells two of them apart.
    fn flake(ago: i64, n: u64) -> String {
        let milliseconds = (now() - ago) as u64 * 1000 - 1_420_070_400_000;
        ((milliseconds << 22) + n).to_string()
    }

    fn message(id: &str, kind: i64, content: &str) -> Value {
        json!({
            "id": id, "type": kind, "content": content,
            "author": { "id": "1", "username": "sam", "global_name": "Sam Lee" },
        })
    }

    async fn serve(server: &MockServer, route: &str, answer: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(route))
            .and(header("authorization", "Bot secret"))
            .respond_with(answer)
            .mount(server)
            .await;
    }

    fn discord(server: &MockServer, token: Option<&str>) -> Discord {
        Discord {
            source: "bot".into(),
            api: server.uri(),
            token: token.map(str::to_owned),
        }
    }

    /// A server with one guild and these channels.
    async fn guild(channels: Value) -> MockServer {
        let server = MockServer::start().await;
        let guilds = json!([{ "id": "10", "name": "Acme" }]);
        serve(
            &server,
            "/users/@me/guilds",
            ResponseTemplate::new(200).set_body_json(guilds),
        )
        .await;
        serve(
            &server,
            "/guilds/10/channels",
            ResponseTemplate::new(200).set_body_json(channels),
        )
        .await;
        server
    }

    #[test]
    fn the_time_of_a_message_is_read_from_its_identifier() {
        // The example of Discord's own documentation: 2016-04-30 11:18:25 UTC.
        assert_eq!(sent_at("175928847299117063"), Some(1_462_015_105));
        assert_eq!(sent_at("soon"), None);
        assert_eq!(id(&json!("12/../34")), None);
    }

    #[tokio::test]
    async fn reads_the_text_channels_that_spoke_in_the_last_day() {
        let server = guild(json!([
            { "id": "20", "type": 0, "name": "releases", "last_message_id": flake(60, 0) },
            { "id": "21", "type": 2, "name": "Voice", "last_message_id": flake(60, 0) },
            { "id": "22", "type": 0, "name": "archive", "last_message_id": flake(3 * DAY, 0) },
            { "id": "23", "type": 0, "name": "empty", "last_message_id": null },
            { "id": "24", "type": 5, "name": "staff", "last_message_id": flake(30, 0) },
        ]))
        .await;
        let (fresh, joined, old) = (flake(60, 1), flake(90, 2), flake(2 * DAY, 3));
        Mock::given(method("GET"))
            .and(path("/channels/20/messages"))
            .and(query_param("limit", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                message(
                    &fresh,
                    0,
                    "Can someone review the changelog?\nIt ships tonight."
                ),
                message(&joined, 7, ""),
                message(&old, 0, "Shipped last week."),
            ])))
            .mount(&server)
            .await;
        // The bot sees this channel in the list but may not read it.
        serve(
            &server,
            "/channels/24/messages",
            ResponseTemplate::new(403)
                .set_body_json(json!({ "message": "Missing Access", "code": 50001 })),
        )
        .await;

        let items = discord(&server, Some(" secret ")).sync().await.unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.kind, Kind::Message);
        assert_eq!(item.external_id, fresh);
        assert_eq!(item.title, "Can someone review the changelog?");
        assert!(item.body.ends_with("It ships tonight."));
        assert_eq!(item.author.as_deref(), Some("Sam Lee · #releases"));
        assert_eq!(
            item.url.as_deref(),
            Some(format!("https://discord.com/channels/10/20/{fresh}").as_str())
        );
        assert_eq!(item.meta["chat"], "Acme #releases");
        assert!((now() - item.occurred_at - 60).abs() <= 2);
    }

    #[tokio::test]
    async fn messages_that_all_come_without_text_point_to_the_missing_permission() {
        let server = guild(json!([
            { "id": "20", "type": 0, "name": "releases", "last_message_id": flake(60, 0) },
        ]))
        .await;
        serve(
            &server,
            "/channels/20/messages",
            ResponseTemplate::new(200).set_body_json(json!([message(&flake(60, 1), 0, "")])),
        )
        .await;
        let error = discord(&server, Some("secret")).sync().await.unwrap_err();
        assert!(error.contains("Message Content Intent"), "{error}");
    }

    #[tokio::test]
    async fn a_wrong_or_missing_token_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/@me/guilds"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(json!({ "message": "401: Unauthorized", "code": 0 })),
            )
            .mount(&server)
            .await;
        assert_eq!(
            discord(&server, Some("revoked")).sync().await.unwrap_err(),
            "Discord: 401: Unauthorized"
        );
        let missing = discord(&server, None).sync().await.unwrap_err();
        assert!(missing.contains("no bot token"));
    }
}
