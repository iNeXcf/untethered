use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub const VOICE_ROOM: &str = "org.matrix.msc3417.call";

pub fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
fn pairs(v: &Value) -> impl Iterator<Item = (&String, &Value)> {
    v.as_object().into_iter().flatten()
}
pub fn string(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}
pub fn short(id: &str) -> &str {
    id.trim_start_matches('@').split(':').next().unwrap_or(id)
}

#[derive(Default)]
pub struct Rooms {
    pub list: BTreeMap<String, Room>,
    pub children: BTreeMap<String, BTreeSet<String>>,
}
#[derive(Default)]
pub struct Room {
    pub name: String,
    pub topic: String,
    pub space: bool,
    pub dm: bool,
    pub voice: bool,
    #[cfg(target_os = "linux")]
    pub voice_members: BTreeMap<String, (String, u64)>,
    #[cfg(target_os = "linux")]
    pub dm_peer: Option<String>,
    pub unread: bool,
    pub encrypted: bool,
    pub has_metadata: bool,
    pub end: bool,
    pub cursor: Option<i64>,
    pub members: BTreeMap<String, String>,
    pub history: BTreeMap<i64, Post>,
    pub(super) event_rows: BTreeMap<String, i64>,
    pub(super) related: BTreeMap<String, Quote>,
    pub(super) edits: crate::edits::Replacements,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Quote {
    pub id: String,
    pub sender: String,
    pub excerpt: String,
}
impl Quote {
    pub fn relate(&self, params: &mut Value) {
        params["relates_to"] = json!({"m.in_reply_to":{"event_id":self.id}});
        params["mentions"] = json!({"user_ids":[self.sender]});
    }
}
pub struct Post {
    pub reactable: bool,
    pub chips: crate::reactions::Chips,
    pub(super) text_message: bool,
    pub(super) encrypted: bool,
    pub(super) mentions: Value,
    pub revision: Option<std::sync::Arc<crate::edits::Revision>>,
    pub event_id: Option<String>,
    pub reply: Option<String>,
    pub media: Option<crate::media::Media>,
    pub sender: String,
    pub body: String,
    pub time: i64,
}
impl Post {
    pub fn quote(&self) -> Option<Quote> {
        let mut chars = self.text().trim().chars();
        let mut excerpt: String = chars
            .by_ref()
            .take(72)
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect();
        if chars.next().is_some() {
            excerpt.push('…');
        }
        Some(Quote {
            id: self.event_id.clone()?,
            sender: self.sender.clone(),
            excerpt,
        })
    }
}
// gomuks assigns ~txn IDs to local echoes. A $ ID comes from the homeserver.
// In v0.2611.1, actuallySend updates the ID but can leave the initial "not sent"
// sentinel on both the completion event and the persisted row.
pub fn server_confirmed(e: &Value) -> bool {
    e["event_id"]
        .as_str()
        .is_some_and(|s| s.starts_with('$') && s.len() > 1)
}
pub fn real_send_error(e: &Value) -> bool {
    !server_confirmed(e)
        && e["send_error"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s != "not sent")
}
impl Room {
    pub fn by_id(&self, id: &str) -> Option<&Post> {
        self.event_rows.get(id).and_then(|r| self.history.get(r))
    }
    pub fn quote(&self, id: &str) -> Option<Quote> {
        self.by_id(id)
            .and_then(Post::quote)
            .or_else(|| self.related.get(id).cloned())
    }
    #[cfg(target_os = "linux")]
    pub fn voice_users(&self) -> BTreeSet<&str> {
        let now = crate::desktop_call::now();
        self.voice_members
            .values()
            .filter(|(_, expires)| *expires > now)
            .map(|(user, _)| user.as_str())
            .collect()
    }
    #[cfg(target_os = "linux")]
    pub fn voice_state(&mut self, e: &Value) {
        if !self.voice || e["type"] != untethered_iced::rtc_membership::EVENT_TYPE {
            return;
        }
        let Some(key) = e["state_key"].as_str() else {
            return;
        };
        self.voice_members.remove(key);
        if crate::desktop_call::active_member(e)
            && let Some(user) = e["sender"].as_str()
        {
            let expires = e["content"]["created_ts"]
                .as_u64()
                .or(e["timestamp"].as_u64())
                .unwrap_or(0)
                .saturating_add(e["content"]["expires"].as_u64().unwrap_or(14_400_000));
            self.voice_members
                .insert(key.into(), (user.into(), expires));
        }
    }
    pub fn already_confirmed(&self, e: &Value) -> bool {
        e["rowid"]
            .as_i64()
            .and_then(|id| self.history.get(&id))
            .is_some_and(|p| p.event_id.is_some())
    }
    pub fn insert(&mut self, e: &Value) {
        let Some(row) = e["rowid"].as_i64().filter(|&r| r > 0) else {
            return;
        };
        // An opaque encrypted row can later decrypt to a reaction, call signal
        // or edit. It must not remain an independent, replyable message.
        let edit = crate::edits::is_replacement(e);
        if e["type"] == "m.room.encrypted"
            && e["decrypted_type"]
                .as_str()
                .is_some_and(|ty| !matches!(ty, "m.room.message" | "m.sticker"))
            || edit
        {
            if let Some(old) = self.history.remove(&row).and_then(|p| p.event_id) {
                self.event_rows.remove(&old);
            }
            if edit
                && let Some(target) = self.edits.ingest(e)
                && let Some(p) = self
                    .event_rows
                    .get(&target)
                    .and_then(|row| self.history.get_mut(row))
            {
                self.edits.apply(p);
            }
            return;
        }
        let body = match e["type"].as_str() {
            Some("m.room.encrypted") => e["decrypted"]["body"]
                .as_str()
                .unwrap_or("Waiting for encryption keys…"),
            Some("m.room.message" | "m.sticker") => {
                e["content"]["body"].as_str().unwrap_or("[Attachment]")
            }
            _ => return,
        };
        let mut body = body.to_owned();
        let confirmed = server_confirmed(e);
        // A late local echo must not overwrite the synced event for this row.
        if !confirmed && self.already_confirmed(e) {
            return;
        }
        let redacted = e["redacted_by"].as_str().is_some_and(|s| !s.is_empty());
        let content = if e["type"] == "m.room.encrypted" {
            &e["decrypted"]
        } else {
            &e["content"]
        };
        let relation = &content["m.relates_to"];
        let reply = relation["m.in_reply_to"]["event_id"]
            .as_str()
            .filter(|id| {
                !redacted
                    && relation["is_falling_back"] != true
                    && id.starts_with('$')
                    && id.len() > 1
            })
            .map(str::to_owned);
        if reply.is_some()
            && body.starts_with("> <")
            && let Some((fallback, message)) = body.split_once("\n\n")
            && fallback.lines().all(|s| s.starts_with('>'))
        {
            body = message.to_owned();
        }
        if redacted {
            body = "Message removed".into();
        }
        if real_send_error(e) {
            body.push_str("  ·  Failed to send");
        } else if !confirmed && e["send_error"] == "not sent" {
            body.push_str("  ·  Sending…");
        }
        let event_id = confirmed.then(|| string(&e["event_id"]));
        if let Some(id) = &event_id {
            self.event_rows.insert(id.clone(), row);
        }
        let mut post = Post {
            reactable: confirmed
                && !redacted
                && e["state_key"].is_null()
                && (matches!(e["type"].as_str(), Some("m.room.message" | "m.sticker"))
                    || matches!(
                        e["decrypted_type"].as_str(),
                        Some("m.room.message" | "m.sticker")
                    )),
            chips: crate::reactions::chips(e),
            text_message: !redacted
                && e["state_key"].is_null()
                && content["msgtype"] == "m.text"
                && content["body"].is_string()
                && (e["type"] == "m.room.message" || e["decrypted_type"] == "m.room.message"),
            encrypted: e["type"] == "m.room.encrypted",
            mentions: content["m.mentions"].clone(),
            revision: None,
            event_id,
            reply,
            media: if redacted {
                None
            } else {
                crate::media::Media::parse(content)
            },
            sender: string(&e["sender"]),
            body,
            time: e["timestamp"].as_i64().unwrap_or(0),
        };
        self.edits.apply(&mut post);
        self.history.insert(row, post);
    }
    pub fn related_events(&mut self, room: &str, data: &Value) {
        let mut quotes = Room::default();
        for e in items(&data["related_events"]).filter(|e| e["room_id"] == room) {
            if crate::edits::is_replacement(e) {
                self.insert(e);
            } else {
                quotes.insert(e);
            }
        }
        for p in quotes.history.values_mut() {
            self.edits.apply(p);
            if let Some(q) = p.quote() {
                self.related.insert(q.id.clone(), q);
            }
        }
    }
    pub fn page(&mut self, room: &str, data: &Value) {
        let old = self.cursor;
        for e in items(&data["events"]).filter(|e| e["room_id"] == room) {
            if let Some(t) = e["timeline_rowid"].as_i64().filter(|&x| x != 0) {
                self.cursor = Some(self.cursor.map_or(t, |v| v.min(t)));
            }
            self.insert(e);
        }
        self.related_events(room, data);
        self.end = data["has_more"] != true || self.cursor == old;
    }
    pub fn posts(&self) -> Vec<&Post> {
        let mut posts: Vec<_> = self.history.iter().collect();
        posts.sort_by_key(|(row, p)| (p.time, **row));
        posts.into_iter().map(|(_, p)| p).collect()
    }
    pub fn display<'a>(&'a self, sender: &'a str) -> &'a str {
        self.members
            .get(sender)
            .map(String::as_str)
            .unwrap_or_else(|| short(sender))
    }
}
impl Rooms {
    pub fn sync(&mut self, d: &Value) {
        if d["clear_state"] == true {
            *self = Self::default();
        }
        for (space, edges) in pairs(&d["space_edges"]) {
            self.children.insert(
                space.clone(),
                items(edges)
                    .filter_map(|e| e["child_id"].as_str().map(str::to_owned))
                    .collect(),
            );
        }
        for (id, delta) in pairs(&d["rooms"]) {
            let room = self.list.entry(id.clone()).or_default();
            let meta = &delta["meta"];
            if meta.is_object() {
                room.has_metadata = true;
                room.name = meta["name"]
                    .as_str()
                    .or(meta["canonical_alias"].as_str())
                    .unwrap_or(id)
                    .into();
                room.topic = string(&meta["topic"]);
                room.space =
                    meta["creation_content"]["type"] == "m.space" || self.children.contains_key(id);
                room.dm = meta["dm_user_id"].is_string();
                room.voice = meta["creation_content"]["type"] == VOICE_ROOM;
                #[cfg(target_os = "linux")]
                {
                    room.dm_peer = meta["dm_user_id"]
                        .as_str()
                        .filter(|v| v.starts_with('@'))
                        .map(str::to_owned);
                }
                room.unread = meta["marked_unread"] == true
                    || meta["unread_messages"].as_u64().unwrap_or(0) > 0;
                room.encrypted = meta["encryption_event"].is_object();
            }
            if delta["reset"] == true {
                room.history.clear();
                room.event_rows.clear();
                room.related.clear();
                room.edits = Default::default();
                room.cursor = None;
                room.end = false;
            }
            let timeline: BTreeSet<_> = items(&delta["timeline"])
                .filter_map(|t| t["event_rowid"].as_i64())
                .collect();
            for e in items(&delta["events"]) {
                if e["room_id"].as_str().is_some_and(|room| room != id) {
                    continue;
                }
                #[cfg(target_os = "linux")]
                if let Some(key) = e["state_key"].as_str()
                    && delta["state"][untethered_iced::rtc_membership::EVENT_TYPE][key]
                        == e["rowid"]
                {
                    room.voice_state(e);
                }
                if e["type"] == "m.room.member"
                    && let Some(user) = e["state_key"].as_str()
                    && delta["state"]["m.room.member"][user] == e["rowid"]
                    && let Some(name) = e["content"]["displayname"].as_str()
                {
                    room.members.insert(user.into(), name.into());
                }
                if timeline.contains(&e["rowid"].as_i64().unwrap_or(0))
                    || crate::edits::is_replacement(e)
                    || e["event_id"]
                        .as_str()
                        .is_some_and(|id| room.by_id(id).is_some())
                {
                    room.insert(e);
                }
            }
        }
        for id in items(&d["left_rooms"]).filter_map(Value::as_str) {
            self.list.remove(id);
            self.children.remove(id);
        }
    }
    pub fn event(&mut self, fallback: &Value, event: &Value) {
        if let Some(room) = event["room_id"]
            .as_str()
            .or(fallback.as_str())
            .and_then(|id| self.list.get_mut(id))
        {
            room.insert(event);
        }
    }
    pub fn demo() -> Self {
        let mut rooms = Self::default();
        let space = "!workshop";
        let mut add = |id: &str, name: &str, topic: &str, dm: bool| {
            rooms.list.insert(
                id.into(),
                Room {
                    name: name.into(),
                    topic: topic.into(),
                    space: id == space,
                    dm,
                    end: true,
                    ..Room::default()
                },
            );
        };
        add(space, "The Workshop", "", false);
        add(
            "!general",
            "general",
            "The common room for ideas, updates, and hellos.",
            false,
        );
        add(
            "!design",
            "design-lab",
            "A place for work in progress.",
            false,
        );
        add(
            "!music",
            "off-topic",
            "Music, recommendations, and the occasional tangent.",
            false,
        );
        add("!dm", "Mira", "Direct message", true);
        rooms.children.insert(
            space.into(),
            ["!general", "!design", "!music"].map(String::from).into(),
        );
        let now = chrono::Utc::now().timestamp_millis();
        for (i,(room,sender,body)) in [
            ("!general","Mira","Morning! I made a little space for us to try out the new client."),
            ("!general","Jules","This already feels like our usual hangout. I found the room without thinking about it."),
            ("!general","Jules","And the design sketches have their own channel now."),
            ("!general","River","I’m here too. Try sending a message below, or jump over to #design-lab."),
            ("!design","River","First sketch: server rail on the left, the conversation front and center."),
            ("!music","Jules","What’s everyone listening to while they build?"),
            ("!dm","Mira","Found the direct messages! This conversation is just a local preview."),
        ].into_iter().enumerate() {
            rooms.list.get_mut(room).unwrap().insert(&json!({"rowid":i+1,"type":"m.room.message","sender":sender,"timestamp":now-((8-i) as i64)*60000,"content":{"body":body}}));
        }
        rooms.list.get_mut("!design").unwrap().unread = true;
        rooms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_decrypt_page_and_leave() {
        let mut rooms = Rooms::default();
        let event = json!({"type":"m.room.encrypted","sender":"@me:h","rowid":4,"room_id":"!r"});
        rooms.sync(&json!({"rooms":{"!r":{"meta":{"name":"Chat"},"timeline":[{"event_rowid":4}],"events":[event,{"rowid":5,"type":"m.room.message","content":{"body":"not in timeline"}}]}}}));
        assert_eq!(rooms.list["!r"].history.len(), 1);
        assert_eq!(
            rooms.list["!r"].history[&4].body,
            "Waiting for encryption keys…"
        );
        let mut decrypted = event;
        decrypted["decrypted"] = json!({"body":"hello"});
        rooms.event(&Value::Null, &decrypted);
        decrypted["timeline_rowid"] = json!(-3);
        let r = rooms.list.get_mut("!r").unwrap();
        r.page("!r", &json!({"events":[decrypted],"has_more":true}));
        assert_eq!(r.history.len(), 1);
        assert_eq!(r.history[&4].body, "hello");
        assert_eq!(r.cursor, Some(-3));
        rooms.sync(&json!({"left_rooms":["!r"]}));
        assert!(rooms.list.is_empty());
    }
}
