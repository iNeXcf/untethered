use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

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
    #[cfg(target_os = "linux")]
    pub dm_peer: Option<String>,
    pub unread: bool,
    pub encrypted: bool,
    pub has_metadata: bool,
    pub end: bool,
    pub cursor: Option<i64>,
    pub members: BTreeMap<String, String>,
    pub history: BTreeMap<i64, Post>,
}
pub struct Post {
    confirmed: bool,
    pub media: Option<crate::media::Media>,
    pub sender: String,
    pub body: String,
    pub time: i64,
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
    pub fn already_confirmed(&self, e: &Value) -> bool {
        e["rowid"]
            .as_i64()
            .and_then(|id| self.history.get(&id))
            .is_some_and(|p| p.confirmed)
    }
    pub fn insert(&mut self, e: &Value) {
        let Some(row) = e["rowid"].as_i64().filter(|&r| r > 0) else {
            return;
        };
        if matches!(
            e["decrypted_type"].as_str(),
            Some("org.matrix.msc4075.rtc.notification" | "org.matrix.msc4310.rtc.decline")
        ) {
            self.history.remove(&row);
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
        if e["relation_type"] == "m.replace" {
            return;
        }
        let mut body = body.to_owned();
        let confirmed = server_confirmed(e);
        // A late local echo must not overwrite the synced event for this row.
        if !confirmed && self.already_confirmed(e) {
            return;
        }
        if e["redacted_by"].as_str().is_some_and(|s| !s.is_empty()) {
            body = "Message removed".into();
        }
        if real_send_error(e) {
            body.push_str("  ·  Failed to send");
        } else if !confirmed && e["send_error"] == "not sent" {
            body.push_str("  ·  Sending…");
        }
        self.history.insert(
            row,
            Post {
                confirmed,
                media: if e["redacted_by"].as_str().is_some_and(|s| !s.is_empty()) {
                    None
                } else {
                    crate::media::Media::parse(if e["type"] == "m.room.encrypted" {
                        &e["decrypted"]
                    } else {
                        &e["content"]
                    })
                },
                sender: string(&e["sender"]),
                body,
                time: e["timestamp"].as_i64().unwrap_or(0),
            },
        );
    }
    pub fn page(&mut self, data: &Value) {
        let old = self.cursor;
        for e in items(&data["events"]) {
            if let Some(t) = e["timeline_rowid"].as_i64().filter(|&x| x != 0) {
                self.cursor = Some(self.cursor.map_or(t, |v| v.min(t)));
            }
            self.insert(e);
        }
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
                room.cursor = None;
                room.end = false;
            }
            let timeline: BTreeSet<_> = items(&delta["timeline"])
                .filter_map(|t| t["event_rowid"].as_i64())
                .collect();
            for e in items(&delta["events"]) {
                if e["type"] == "m.room.member"
                    && let Some(user) = e["state_key"].as_str()
                    && delta["state"]["m.room.member"][user] == e["rowid"]
                    && let Some(name) = e["content"]["displayname"].as_str()
                {
                    room.members.insert(user.into(), name.into());
                }
                if timeline.contains(&e["rowid"].as_i64().unwrap_or(0)) {
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
        r.page(&json!({"events":[decrypted],"has_more":true}));
        assert_eq!(r.history.len(), 1);
        assert_eq!(r.history[&4].body, "hello");
        assert_eq!(r.cursor, Some(-3));
        rooms.sync(&json!({"left_rooms":["!r"]}));
        assert!(rooms.list.is_empty());
    }
}
