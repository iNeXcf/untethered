//! gomuks evaluates Matrix push rules on decrypted content. A room rule can
//! mute ordinary chat without overriding the higher-priority mention rule.
use crate::{
    App, Msg, Request,
    rooms::{self, items},
};
use ashpd::{zbus, zvariant};
use iced::Task;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Default,
    All,
    Quiet,
}
impl Mode {
    pub const CHOICES: [Self; 3] = [Self::Default, Self::All, Self::Quiet];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Account default",
            Self::All => "All messages",
            Self::Quiet => "Mute ordinary",
        }
    }
}
#[derive(Clone)]
pub struct Change {
    room: String,
    mode: Mode,
    enable_next: bool,
}
#[derive(Default)]
pub struct Alerts {
    pub panel: bool,
    pub focused: bool,
    pub ready: bool,
    rules: BTreeMap<String, (Mode, bool)>,
    recent: VecDeque<String>,
    waiting: VecDeque<(String, String, Instant)>,
    outgoing: Vec<Notice>,
    pub delivery_failed: bool,
}
struct Notice {
    title: String,
    body: String,
    sound: bool,
}
impl Alerts {
    pub fn reset(&mut self) {
        *self = Self {
            focused: self.focused,
            ..Default::default()
        };
    }
    fn rules(&mut self, data: &Value) {
        if data["clear_state"] == true {
            self.ready = false;
            self.rules.clear();
            self.waiting.clear();
        }
        let Some(rules) = data["account_data"]["m.push_rules"]["content"]["global"].as_object()
        else {
            return;
        };
        self.ready = true;
        self.rules.clear();
        for r in rules
            .get("room")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = r["rule_id"].as_str() {
                self.rules.insert(
                    id.into(),
                    (
                        if items(&r["actions"]).any(|a| a == "notify") {
                            Mode::All
                        } else {
                            Mode::Quiet
                        },
                        r["enabled"] == true,
                    ),
                );
            }
        }
    }
    pub fn mode(&self, room: &str) -> Mode {
        self.rules
            .get(room)
            .filter(|(_, on)| *on)
            .map_or(Mode::Default, |(mode, _)| *mode)
    }
    fn observe(
        &mut self,
        room: &str,
        info: &rooms::Room,
        me: &str,
        delta: &Value,
        late: bool,
        reading: bool,
    ) {
        self.waiting
            .retain(|(_, _, at)| at.elapsed() < Duration::from_secs(120));
        let timeline: BTreeSet<_> = items(&delta["timeline"])
            .filter_map(|e| e["event_rowid"].as_i64())
            .collect();
        let approved: BTreeMap<_, _> = items(&delta["notifications"])
            .filter_map(|n| Some((n["event_rowid"].as_i64()?, n["sound"] == true)))
            .collect();
        for e in items(&delta["events"]) {
            let (Some(row), Some(id), Some(sender)) = (
                e["rowid"].as_i64(),
                e["event_id"].as_str(),
                e["sender"].as_str(),
            ) else {
                continue;
            };
            if !rooms::server_confirmed(e)
                || e["room_id"] != room
                || sender == me
                || e["redacted_by"].is_string()
                || !e["state_key"].is_null()
            {
                continue;
            }
            let c = if e["type"] == "m.room.encrypted" {
                &e["decrypted"]
            } else {
                &e["content"]
            };
            let sound = if late {
                let Some(index) = self
                    .waiting
                    .iter()
                    .position(|(r, i, _)| r == room && i == id)
                else {
                    continue;
                };
                self.waiting.remove(index);
                // gomuks recomputes these bits when it finally has the key.
                const NOTIFY: u64 = 0b0010;
                const SOUND: u64 = 0b1000;
                let flags = e["unread_type"].as_u64().unwrap_or_default();
                if flags & NOTIFY == 0 {
                    continue;
                }
                flags & SOUND != 0
            } else {
                if !timeline.contains(&row) {
                    continue;
                }
                if e["type"] == "m.room.encrypted" && !c.is_object() {
                    if !reading && !self.waiting.iter().any(|(r, i, _)| r == room && i == id) {
                        self.waiting
                            .push_back((room.into(), id.into(), Instant::now()));
                        if self.waiting.len() > 512 {
                            self.waiting.pop_front();
                        }
                    }
                    continue;
                }
                let Some(&sound) = approved.get(&row) else {
                    continue;
                };
                sound
            };
            let ty = if e["type"] == "m.room.encrypted" {
                &e["decrypted_type"]
            } else {
                &e["type"]
            };
            if !matches!(ty.as_str(), Some("m.room.message" | "m.sticker"))
                || crate::edits::is_replacement(e)
                || self.recent.iter().any(|old| old == id)
            {
                continue;
            }
            self.recent.push_back(id.into());
            if self.recent.len() > 512 {
                self.recent.pop_front();
            }
            if reading {
                continue;
            }
            let preview = e["local_content"]["preview_text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or(c["body"].as_str())
                .unwrap_or("New message");
            self.outgoing.push(Notice {
                title: format!(
                    "{} · {}",
                    single_line(info.display(sender), 64),
                    single_line(&info.name, 80)
                ),
                body: single_line(preview, 240)
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
                sound,
            });
        }
    }
}
fn single_line(s: &str, limit: usize) -> String {
    s.chars()
        .take(limit)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
async fn deliver(notices: Vec<Notice>) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &bus,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await?;
    for n in notices {
        let hints = HashMap::from([
            ("desktop-entry", zvariant::Value::from("untethered")),
            ("category", zvariant::Value::from("im.received")),
            ("suppress-sound", zvariant::Value::from(!n.sound)),
        ]);
        let _: u32 = proxy
            .call(
                "Notify",
                &(
                    "Untethered",
                    0u32,
                    "internet-chat",
                    n.title,
                    n.body,
                    Vec::<String>::new(),
                    hints,
                    -1i32,
                ),
            )
            .await?;
    }
    Ok(())
}
impl App {
    pub fn notifications_pending(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|req| matches!(req,Request::NotifyRule(c) if c.room==room))
    }
    pub fn set_notifications(&mut self, room: String, mode: Mode) {
        if self.selected != room
            || !self.logged_in
            || !self.verified
            || !self.connected
            || !self.alerts.ready
            || !self.live.list.contains_key(&room)
            || self.notifications_pending(&room)
            || self.alerts.mode(&room) == mode
        {
            return;
        }
        let mut params = json!({"kind":"room","rule_id":room,"action":"delete"});
        if mode != Mode::Default {
            params["action"] = "put".into();
            params["new_content"] =
                json!({"actions":if mode==Mode::All{json!(["notify"])}else{json!([])}});
        }
        let enable_next =
            mode != Mode::Default && self.alerts.rules.get(&room).is_some_and(|(_, on)| !*on);
        self.rpc(
            "update_push_rule",
            params,
            Request::NotifyRule(Change {
                room,
                mode,
                enable_next,
            }),
        );
    }
    pub fn saved_notifications(&mut self, change: Change) {
        if change.enable_next {
            self.rpc(
                "update_push_rule",
                json!({"kind":"room","rule_id":change.room,"action":"enable"}),
                Request::NotifyRule(Change {
                    enable_next: false,
                    ..change
                }),
            );
        } else if change.mode == Mode::Default {
            self.alerts.rules.remove(&change.room);
        } else {
            self.alerts.rules.insert(change.room, (change.mode, true));
        }
    }
    pub fn notifications_sync(&mut self, data: &Value) {
        self.alerts.rules(data);
        if data["clear_state"] == true
            || data["catchup"] == true
            || data["since"].as_str().is_none_or(str::is_empty)
        {
            return;
        }
        if let Some(rooms) = data["rooms"].as_object() {
            for (id, delta) in rooms {
                self.room_alerts(id, delta, false);
            }
        }
    }
    pub fn room_alerts(&mut self, id: &str, delta: &Value, late: bool) {
        if !self.logged_in || !self.verified || self.closing {
            return;
        }
        if let Some(room) = self.live.list.get(id) {
            let reading = self.alerts.focused
                && self.selected == id
                && !self.account
                && self.search.is_none()
                && self.original.is_none();
            self.alerts
                .observe(id, room, &self.matrix_id, delta, late, reading);
        }
    }
    pub fn send_alerts(&mut self) -> Task<Msg> {
        let notices = std::mem::take(&mut self.alerts.outgoing);
        if notices.is_empty() {
            return Task::none();
        }
        Task::perform(
            async move {
                matches!(
                    tokio::time::timeout(Duration::from_secs(5), deliver(notices)).await,
                    Ok(Ok(()))
                )
            },
            Msg::DesktopDelivery,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_live_candidates_notify_once_when_keys_arrive() {
        let (tx, _) = async_channel::bounded(4);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.verified = true;
        app.matrix_id = "@alice:h".into();
        app.live.list.insert(
            "!r".into(),
            rooms::Room {
                name: "Room".into(),
                ..Default::default()
            },
        );
        let opaque = json!({"type":"m.room.encrypted","room_id":"!r","rowid":9,"event_id":"$hello","sender":"@bruno:h"});
        let mut clear = opaque.clone();
        clear["decrypted_type"] = "m.room.message".into();
        clear["decrypted"] = json!({"msgtype":"m.text","body":"Hi <Alice> & friends"});
        clear["unread_type"] = json!(14);
        let delta = json!({"timeline":[{"event_rowid":9}],"events":[opaque]});
        let keys = json!({"room_id":"!r","events":[clear]});
        for sync in [
            json!({"catchup":true,"since":"old","rooms":{"!r":delta}}),
            json!({"clear_state":true,"rooms":{"!r":delta}}),
        ] {
            app.notifications_sync(&sync);
            app.room_alerts("!r", &keys, true);
            assert!(app.alerts.outgoing.is_empty());
        }
        app.notifications_sync(&json!({"since":"live","rooms":{"!r":delta}}));
        app.room_alerts("!r", &keys, true);
        app.room_alerts("!r", &keys, true);
        app.notifications_sync(&json!({"since":"live","rooms":{"!r":{"timeline":[{"event_rowid":9}],"notifications":[{"event_rowid":9}],"events":[clear]}}}));
        assert_eq!(app.alerts.outgoing.len(), 1);
        assert_eq!(
            app.alerts.outgoing[0].body,
            "Hi &lt;Alice&gt; &amp; friends"
        );
        assert!(app.alerts.outgoing[0].sound);
        // A late ordinary message suppressed by a room rule remains suppressed.
        let mut muted = opaque;
        muted["event_id"] = "$quiet".into();
        muted["rowid"] = 10.into();
        app.notifications_sync(&json!({"since":"live","rooms":{"!r":{"timeline":[{"event_rowid":10}],"events":[muted]}}}));
        muted["decrypted_type"] = "m.room.message".into();
        muted["decrypted"] = json!({"body":"Quiet"});
        muted["unread_type"] = 1.into();
        app.room_alerts("!r", &json!({"events":[muted]}), true);
        assert!(app.alerts.waiting.is_empty());
        assert_eq!(app.alerts.outgoing.len(), 1);
    }
}
