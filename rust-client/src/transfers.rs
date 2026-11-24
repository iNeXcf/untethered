#[cfg(any(test, target_os = "linux"))]
use crate::Msg;
use crate::{App, Request, backend::Command, media};
use iced::widget::text_editor;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct Batch {
    pub room: String,
    ids: VecDeque<u64>,
    caption: String,
    // send_complete can race the RPC reply containing the local transaction ID.
    early: std::collections::BTreeMap<String, bool>,
}

impl App {
    pub fn can_stage(&self) -> bool {
        !self.account && !self.selected.is_empty() && (!self.logged_in || self.verified)
    }
    pub fn send_files(&mut self, room: String, caption: String) {
        if !self.can_stage() || self.batch.is_some() {
            return;
        }
        if !self.logged_in {
            if let Some(q) = self.outbox.rooms.remove(&room)
                && let Some(r) = self.demo.list.get_mut(&room)
            {
                for (i, a) in q.into_iter().enumerate() {
                    let body = format!(
                        "📎 {}{} (local preview)",
                        a.source.name,
                        if i == 0 && !caption.is_empty() {
                            format!(" — {caption}")
                        } else {
                            String::new()
                        }
                    );
                    r.insert(&json!({"rowid":r.history.keys().next_back().copied().unwrap_or(0)+1,"type":"m.room.message","sender":"You","timestamp":chrono::Utc::now().timestamp_millis(),"content":{"body":body}}));
                }
                self.editor = text_editor::Content::new();
            }
            return;
        }
        if !self.connected {
            return;
        }
        let ids = self.outbox.rooms[&room].iter().map(|a| a.id).collect();
        self.batch = Some(Batch {
            room,
            ids,
            caption,
            early: Default::default(),
        });
        self.advance_file();
    }
    fn advance_file(&mut self) {
        let Some(b) = &mut self.batch else { return };
        let Some(&id) = b.ids.front() else {
            self.batch = None;
            return;
        };
        let room = b.room.clone();
        let caption = b.caption.clone();
        let Some(r) = self.live.list.get(&room) else {
            self.batch = None;
            return;
        };
        let encrypt = r.encrypted;
        if !r.has_metadata {
            self.fail_file(
                &room,
                id,
                "Waiting for room encryption information. Retry after sync.",
            );
            return;
        }
        let Some(a) = self.outbox.get_mut(&room, id) else {
            b.ids.pop_front();
            self.advance_file();
            return;
        };
        a.busy = true;
        a.error = None;
        if let Some(descriptor) = &a.descriptor {
            let mut content = descriptor.clone();
            if encrypt && !content["file"].is_object() {
                a.descriptor = None;
                self.fail_file(
                    &room,
                    id,
                    "Room encryption changed. Send again to encrypt the file.",
                );
                return;
            }
            if !caption.trim().is_empty() {
                content["filename"] = json!(a.source.name);
                content["body"] = json!(caption);
            }
            // Put captions in base_content, not in the slash-command parser.
            let next = self.seq + 1;
            let (command, data) = if let Some(txn) = &a.txn {
                ("resend_event", json!({"transaction_id":txn}))
            } else {
                (
                    "send_message",
                    json!({"room_id":room,"base_content":content,"text":""}),
                )
            };
            self.rpc(command, data, Request::MediaSend(room.clone(), id));
            if !self.pending.contains_key(&next) {
                self.fail_file(&room, id, "Could not queue the message.");
            }
        } else {
            self.seq += 1;
            if self
                .commands
                .try_send(Command::Upload {
                    id: self.seq,
                    file: a.source.clone(),
                    encrypt,
                    limit: self.media_limit,
                })
                .is_ok()
            {
                self.pending.insert(self.seq, Request::Upload(room, id));
            } else {
                self.fail_file(&room, id, "Could not queue the upload.");
            }
        }
    }
    pub fn uploaded(&mut self, room: &str, id: u64, value: &Value) {
        if !self.current_file(room, id) {
            return;
        }
        if media::Media::parse(value).is_none() {
            self.fail_file(room, id, "gomuks returned an incomplete attachment.");
            return;
        }
        if let Some(a) = self.outbox.get_mut(room, id) {
            a.descriptor = Some(value.clone());
        }
        self.advance_file();
    }
    fn current_file(&self, room: &str, id: u64) -> bool {
        self.batch
            .as_ref()
            .is_some_and(|b| b.room == room && b.ids.front() == Some(&id))
    }
    pub fn file_sent(&mut self, room: &str, id: u64, event: &Value) {
        if !self.current_file(room, id) {
            return;
        }
        let Some(txn) = event["transaction_id"].as_str() else {
            self.fail_file(
                room,
                id,
                "No send confirmation from gomuks. Check the timeline before retrying.",
            );
            return;
        };
        if let Some(a) = self.outbox.get_mut(room, id) {
            a.txn = Some(txn.into());
        }
        if let Some(ok) = self.batch.as_mut().unwrap().early.remove(txn) {
            self.finish_delivery(room, id, ok);
        }
    }
    pub fn media_delivery(&mut self, data: &Value) {
        let event = &data["event"];
        let (Some(txn), Some(room)) = (event["transaction_id"].as_str(), event["room_id"].as_str())
        else {
            return;
        };
        let Some(b) = &mut self.batch else { return };
        let Some(&id) = b.ids.front() else { return };
        if room != b.room {
            return;
        }
        let ok = data["error"].is_null() && event["send_error"].as_str().unwrap_or("").is_empty();
        if self
            .outbox
            .get_mut(room, id)
            .is_some_and(|a| a.txn.as_deref() == Some(txn))
        {
            self.finish_delivery(room, id, ok);
        } else if self
            .pending
            .values()
            .any(|p| matches!(p,Request::MediaSend(r,i) if r==room && *i==id))
            && b.early.len() < 32
        {
            b.early.insert(txn.into(), ok);
        }
    }
    fn finish_delivery(&mut self, room: &str, id: u64, ok: bool) {
        if !ok {
            self.fail_file(
                room,
                id,
                "Delivery failed. Send again to retry the same Matrix event.",
            );
            return;
        }
        let b = self.batch.as_mut().unwrap();
        b.early.clear();
        if !b.caption.is_empty() {
            if room == self.selected && self.editor.text() == b.caption {
                self.editor = text_editor::Content::new();
            }
            if self.drafts.get(room) == Some(&b.caption) {
                self.drafts.remove(room);
            }
            b.caption.clear();
        }
        self.outbox.remove(room, id);
        b.ids.pop_front();
        self.advance_file();
    }
    pub fn fail_file(&mut self, room: &str, id: u64, why: &str) {
        if let Some(a) = self.outbox.get_mut(room, id) {
            a.busy = false;
            a.error = Some(why.into());
        }
        if self.current_file(room, id) {
            self.batch = None;
        }
    }
}

#[cfg(target_os = "linux")]
pub fn wayland_drops() -> iced::Task<Msg> {
    let (tx, rx) = async_channel::unbounded();
    smithay_clipboard::on_file_drag(move |event| {
        use smithay_clipboard::FileDrag;
        let msg = match event {
            FileDrag::Enter => Msg::HoverFiles(true),
            FileDrag::Leave => Msg::HoverFiles(false),
            FileDrag::Uris(s) => Msg::DropPaths(media::drop_paths(&s)),
        };
        let _ = tx.try_send(msg);
    });
    iced::Task::run(rx, |m| m)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::rooms::Room;
    #[test]
    fn staged_file_only_sends_on_submit_and_retries_in_original_room() {
        let (tx, rx) = async_channel::bounded(4);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.selected = "!a".into();
        app.live.list.insert(
            "!a".into(),
            Room {
                has_metadata: true,
                encrypted: true,
                ..Default::default()
            },
        );
        app.live.list.insert("!b".into(), Room::default());
        let _ = app.update(Msg::Files(
            "!a".into(),
            vec![media::LocalFile {
                name: "report.pdf".into(),
                size: 2,
                path: "unused".into(),
            }],
        ));
        assert!(rx.try_recv().is_err());
        let _ = app.update(Msg::Send);
        let Command::Upload { id, encrypt, .. } = rx.try_recv().unwrap() else {
            panic!("not an upload")
        };
        assert!(encrypt);
        app.selected = "!b".into();
        let descriptor =
            json!({"msgtype":"m.file","filename":"report.pdf","file":{"url":"mxc://test/blob"}});
        app.frame(json!({"command":"response","request_id":id,"data":descriptor}));
        let first = rx.try_recv().unwrap().into_rpc();
        assert_eq!(first["data"]["room_id"], "!a");
        app.frame(json!({"command":"error","request_id":first["request_id"]}));
        app.selected = "!a".into();
        let _ = app.update(Msg::Send);
        let retry = rx.try_recv().unwrap().into_rpc();
        assert_eq!(retry["command"], "send_message");
        assert_eq!(retry["data"]["base_content"], descriptor);
        let echo = json!({"room_id":"!a","rowid":9,"transaction_id":"hicli-test","type":"m.room.message","content":descriptor});
        app.frame(json!({"command":"response","request_id":retry["request_id"],"data":echo}));
        assert!(app.batch.is_some(), "a local echo isn't confirmed delivery");
        app.frame(json!({"command":"send_complete","data":{"event":echo,"error":"network"}}));
        let _ = app.update(Msg::Send);
        let resend = rx.try_recv().unwrap().into_rpc();
        assert_eq!(resend["command"], "resend_event");
        assert_eq!(resend["data"]["transaction_id"], "hicli-test");
        app.frame(json!({"command":"send_complete","data":{"event":echo,"error":null}}));
        app.frame(json!({"command":"response","request_id":resend["request_id"],"data":echo}));
        assert!(app.batch.is_none() && app.outbox.rooms.is_empty());
    }
}
