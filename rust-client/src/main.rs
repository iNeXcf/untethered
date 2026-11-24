#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod account;
mod backend;
#[cfg(target_arch = "wasm32")]
mod browser_verification;
#[cfg(not(target_arch = "wasm32"))]
mod fonts;
mod inline;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod live_test;
mod media;
#[cfg(not(target_arch = "wasm32"))]
mod paste;
mod preview;
mod rooms;
mod transfers;
mod view;
#[cfg(target_arch = "wasm32")]
mod web_files;

use async_channel::Sender;
use iced::{
    Task, Theme,
    widget::{operation, text_editor},
};
use rooms::{Rooms, items};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Eq)]
enum Section {
    All,
    Direct,
    Space(String),
}
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
enum Field {
    Server,
    Username,
    Password,
    Composer(String),
}
#[derive(Clone)]
enum Msg {
    #[cfg(not(target_arch = "wasm32"))]
    PasteComposer,
    #[cfg(not(target_arch = "wasm32"))]
    ClipboardReady(u64, String, paste::Result),
    HoverFiles(bool),
    #[cfg(not(target_arch = "wasm32"))]
    DropPaths(Vec<std::path::PathBuf>),
    Files(String, Vec<media::LocalFile>),
    Thumbnail(String, u64, Option<iced::widget::image::Handle>),
    PickFiles,
    RemoveFile(u64),
    SaveFile(media::Media),
    InlineReady(u64, media::PreviewRef, Option<iced::widget::image::Handle>),
    ShowInline(media::PreviewRef),
    #[cfg(not(target_arch = "wasm32"))]
    SaveTo(media::Media, Option<std::path::PathBuf>),
    CheckCalls,
    Frame(Value),
    Stopped(String),
    Section(Section),
    Open(String),
    Draft(text_editor::Action),
    Send,
    More,
    Copy,
    TogglePeople,
    Account,
    Back,
    Server(String),
    Username(String),
    Password(String),
    Login,
    #[cfg(not(target_arch = "wasm32"))]
    Recovery(String),
    #[cfg(not(target_arch = "wasm32"))]
    Verify,
    #[cfg(target_arch = "wasm32")]
    SubmitWebRecovery(String),
    Tab(bool),
    Close,
    #[cfg(target_arch = "wasm32")]
    WebPaste(Field),
    #[cfg(target_arch = "wasm32")]
    Pasted(Field, String),
    #[cfg(target_arch = "wasm32")]
    ClipboardResult(bool),
}
// Iced requires Debug on messages. Never put passwords, keys or RPC data in debug output.
impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UI({:?})", std::mem::discriminant(self))
    }
}
enum Request {
    Upload(String, u64),
    MediaSend(String, u64),
    Download,
    MediaConfig,
    RtcDiscovery,
    Login,
    Verify(account::RecoveryFormat),
    VerifyState,
    Page(String),
    Members(String),
    Send(String, String),
}
struct App {
    #[cfg(not(target_arch = "wasm32"))]
    paste_seq: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pasting: bool,
    commands: Sender<backend::Command>,
    outbox: media::Outbox,
    inline: inline::InlineImages,
    batch: Option<transfers::Batch>,
    hover_files: bool,
    media_limit: u64,
    rtc_status: String,
    pending: BTreeMap<i64, Request>,
    seq: i64,
    live: Rooms,
    demo: Rooms,
    section: Section,
    selected: String,
    drafts: BTreeMap<String, String>,
    editor: text_editor::Content,
    loaded: BTreeSet<String>,
    connected: bool,
    logged_in: bool,
    verified: bool,
    account: bool,
    people: bool,
    profile: String,
    matrix_id: String,
    status: String,
    server: String,
    username: String,
    password: String,
    recovery: String,
    verification_note: String,
    recovery_unavailable: bool,
    closing: bool,
    stopped: bool,
}
impl App {
    fn boot() -> (Self, Task<Msg>) {
        let (commands, rx) = async_channel::bounded(64);
        let (tx, events) = async_channel::bounded(64);
        let task = Task::batch([
            Task::perform(backend::run(rx, tx), |result| {
                Msg::Stopped(result.err().unwrap_or_else(|| "gomuks closed.".into()))
            }),
            Task::run(events, Msg::Frame),
        ]);
        #[cfg(target_arch = "wasm32")]
        let task = Task::batch([task, browser_verification::listen(), web_files::listen()]);
        #[cfg(target_os = "linux")]
        let task = Task::batch([task, transfers::wayland_drops()]);
        (Self::new(commands), task)
    }
    fn new(commands: Sender<backend::Command>) -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            paste_seq: 0,
            #[cfg(not(target_arch = "wasm32"))]
            pasting: false,
            commands,
            outbox: Default::default(),
            inline: Default::default(),
            batch: None,
            hover_files: false,
            media_limit: media::MAX_BYTES,
            rtc_status: String::new(),
            pending: Default::default(),
            seq: 0,
            live: Default::default(),
            demo: Rooms::demo(),
            section: Section::Space("!workshop".into()),
            selected: "!general".into(),
            drafts: Default::default(),
            editor: text_editor::Content::new(),
            loaded: Default::default(),
            connected: false,
            logged_in: false,
            verified: false,
            account: false,
            people: false,
            profile: String::new(),
            matrix_id: String::new(),
            status: "Starting gomuks…".into(),
            server: String::new(),
            username: String::new(),
            password: String::new(),
            recovery: String::new(),
            verification_note: String::new(),
            recovery_unavailable: false,
            closing: false,
            stopped: false,
        }
    }
    fn rooms(&self) -> &Rooms {
        if self.logged_in {
            &self.live
        } else {
            &self.demo
        }
    }
    fn visible(&self) -> Vec<(&String, &rooms::Room)> {
        let rooms = self.rooms();
        let mut list: Vec<_> = rooms
            .list
            .iter()
            .filter(|(id, r)| {
                !r.space
                    && match &self.section {
                        Section::All => true,
                        Section::Direct => r.dm,
                        Section::Space(space) => rooms
                            .children
                            .get(space)
                            .is_some_and(|kids| kids.contains(*id)),
                    }
            })
            .collect();
        list.sort_by_key(|(id, r)| (r.name.to_lowercase(), *id));
        list
    }
    fn rpc(&mut self, cmd: &str, data: Value, req: Request) {
        self.seq += 1;
        if self.connected
            && self
                .commands
                .try_send(backend::Command::Rpc(
                    json!({"command":cmd,"request_id":self.seq,"data":data}),
                ))
                .is_ok()
        {
            self.pending.insert(self.seq, req);
        } else {
            self.status = "Could not queue the request. Try again.".into();
        }
    }
    fn page(&mut self) {
        let id = self.selected.clone();
        if !self.logged_in
            || !self.verified
            || !self.connected
            || self
                .pending
                .values()
                .any(|p| matches!(p,Request::Page(r) if r==&id))
        {
            return;
        }
        if let Some(r) = self.live.list.get(&id).filter(|r| !r.end) {
            self.rpc(
                "paginate",
                json!({"room_id":id,"limit":50,"max_timeline_id":r.cursor.unwrap_or(0)}),
                Request::Page(id),
            );
        }
    }
    fn open(&mut self, id: String) {
        if self.selected == id {
            return;
        }
        self.drafts
            .insert(self.selected.clone(), self.editor.text());
        self.editor =
            text_editor::Content::with_text(self.drafts.get(&id).map(String::as_str).unwrap_or(""));
        self.selected = id.clone();
        if self.logged_in && self.verified && !id.is_empty() && self.loaded.insert(id.clone()) {
            self.page();
            self.rpc(
                "get_room_state",
                json!({"room_id":id,"include_members":true}),
                Request::Members(id),
            );
        } else if !self.logged_in
            && let Some(r) = self.demo.list.get_mut(&id)
        {
            r.unread = false;
        }
    }
    fn reconcile(&mut self) {
        if matches!(&self.section,Section::Space(s) if !self.rooms().list.contains_key(s)) {
            self.section = Section::All;
        }
        let visible = self.visible();
        if !visible.iter().any(|(id, _)| **id == self.selected) {
            let first = visible
                .first()
                .map(|(id, _)| (*id).clone())
                .unwrap_or_default();
            self.open(first);
        }
    }
    fn verifying(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::Verify(_) | Request::VerifyState))
    }
    fn verification_feedback(&self) -> &str {
        if !self.connected {
            &self.status
        } else if self.verifying() {
            "Verifying with your homeserver…"
        } else if !self.verification_note.is_empty() {
            &self.verification_note
        } else if self.recovery_unavailable {
            "gomuks reports that recovery isn’t set up for this account. Check recovery in a trusted Matrix client."
        } else {
            "Use the key or passphrase from Matrix recovery setup. Your sign-in password is separate. Grouping spaces are fine."
        }
    }
    fn submit_verification(&mut self) {
        if !self.logged_in
            || self.verified
            || !self.connected
            || self.verifying()
            || self.recovery_unavailable
        {
            return;
        }
        if self.recovery.trim().is_empty() {
            self.verification_note = "Enter your recovery key or passphrase first.".into();
            return;
        }
        self.verification_note.clear();
        let (format, key) = account::recovery_input(&self.recovery);
        self.rpc(
            "verify",
            json!({"recovery_key":key}),
            Request::Verify(format),
        );
        if !self.verifying() {
            self.verification_note.clone_from(&self.status);
        }
    }
    fn frame(&mut self, f: Value) {
        let data = &f["data"];
        match f["command"].as_str().unwrap_or("") {
            #[cfg(target_arch = "wasm32")]
            "profile-waiting" => {
                self.status = "Another Untethered tab is using this Matrix device. Use that tab, or close it and this one will connect automatically.".into();
            }
            #[cfg(target_arch = "wasm32")]
            "profile-opening" => {
                self.status = "Opening your browser’s Matrix device…".into();
            }
            "client_state" => {
                let was = self.logged_in;
                let was_verified = self.verified;
                self.connected = true;
                self.logged_in = data["is_logged_in"] == true;
                self.verified = data["is_verified"] == true;
                let vs = &data["verification_state"];
                self.recovery_unavailable = vs["state_checked"] == true && vs["has_ssss"] == false;
                if self.verified || !self.logged_in {
                    self.recovery.clear();
                    self.verification_note.clear();
                }
                self.profile = data["displayname"]
                    .as_str()
                    .or(data["user_id"].as_str())
                    .unwrap_or("")
                    .into();
                self.matrix_id = rooms::string(&data["user_id"]);
                if was != self.logged_in {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        self.paste_seq += 1;
                        self.pasting = false;
                    }
                    self.pending.retain(|_, p| matches!(p, Request::Login));
                    self.drafts.clear();
                    self.editor = text_editor::Content::new();
                    self.loaded.clear();
                    self.selected.clear();
                    self.section = Section::All;
                    self.account = !self.logged_in;
                    self.outbox.rooms.clear();
                    self.inline.reset();
                    self.batch = None;
                    self.rtc_status.clear();
                    self.media_limit = media::MAX_BYTES;
                    if !self.logged_in {
                        self.live = Rooms::default();
                    }
                }
                if self.logged_in && self.verified && !was_verified {
                    self.rpc("get_media_config", json!({}), Request::MediaConfig);
                }
                self.status.clear();
                self.reconcile();
            }
            "sync_complete" => {
                if data["clear_state"] == true {
                    self.loaded.clear();
                    self.selected.clear();
                    self.pending
                        .retain(|_, p| !matches!(p, Request::Members(_) | Request::Page(_)));
                }
                self.live.sync(data);
                if let Some(rooms) = data["rooms"].as_object() {
                    for (room, delta) in rooms {
                        if self.outbox.rooms.contains_key(room) {
                            for e in items(&delta["events"]).filter(|e| rooms::server_confirmed(e))
                            {
                                self.media_delivery(e, true);
                            }
                        }
                    }
                }
                for left in items(&data["left_rooms"]).filter_map(Value::as_str) {
                    self.outbox.rooms.remove(left);
                    if self.batch.as_ref().is_some_and(|b| b.room == left) {
                        self.batch = None;
                    }
                }
                self.reconcile();
            }
            "events_decrypted" => {
                for e in items(&data["events"]) {
                    self.live.event(&data["room_id"], e);
                }
            }
            "send_complete" => {
                let e = &data["event"];
                let accepted = rooms::server_confirmed(e)
                    || e["room_id"]
                        .as_str()
                        .and_then(|id| self.live.list.get(id))
                        .is_some_and(|r| r.already_confirmed(e));
                let ok = accepted || (data["error"].is_null() && !rooms::real_send_error(e));
                if !ok && !rooms::real_send_error(e) && e.is_object() {
                    let mut failed = e.clone();
                    failed["send_error"] = json!("delivery failed");
                    self.live.event(&Value::Null, &failed);
                } else {
                    self.live.event(&Value::Null, e);
                }
                self.media_delivery(e, ok);
            }
            "response" | "error" => {
                let Some(p) = self.pending.remove(&f["request_id"].as_i64().unwrap_or(0)) else {
                    return;
                };
                if f["command"] == "error" {
                    self.status = match p {
                        Request::MediaSend(ref room,id) if data.as_str()==Some("event was already sent successfully") &&
                            self.outbox.get_mut(room,id).is_some_and(|a|a.txn.is_some()) => {
                            self.finish_delivery(room,id,true);return
                        }
                        Request::Upload(room,id) | Request::MediaSend(room,id) => {
                            self.fail_file(&room,id,"Transfer failed. Retry or remove the attachment.");return
                        }
                        Request::MediaConfig=>return,
                        Request::Download=>"Could not save the attachment.",
                        Request::RtcDiscovery=>{self.rtc_status="MatrixRTC discovery failed or is unsupported on this homeserver.".into();return}
                        Request::Login => {
                            "Sign-in failed. Check your homeserver, username, and password."
                        }
                        Request::Verify(format) => {
                            self.verification_note = account::verify_error(data,format).into();
                            return;
                        }
                        Request::VerifyState => {
                            self.verification_note = "gomuks finished verification, but the device status couldn’t be refreshed.".into();
                            return;
                        }
                        Request::Page(_) => {
                            "Could not load history. Use Earlier messages to retry."
                        }
                        Request::Members(_) => "Could not load room members.",
                        Request::Send(_, _) => "Could not send. Your draft has been kept.",
                    }
                    .into();
                    return;
                }
                match p {
                    Request::MediaConfig => {
                        if let Some(n) = data["m.upload.size"].as_u64().filter(|n| *n > 0) {
                            self.media_limit = media::MAX_BYTES.min(n);
                        }
                    }
                    Request::Upload(room, id) => self.uploaded(&room, id, data),
                    Request::MediaSend(room, id) => self.file_sent(&room, id, data),
                    Request::Download => self.status = "Attachment saved.".into(),
                    Request::RtcDiscovery => {
                        let n = items(&data["rtc_transports"])
                            .filter(|v| {
                                v["type"] == "livekit"
                                    && v["livekit_service_url"].as_str().is_some_and(|s| {
                                        url::Url::parse(s).is_ok_and(|u| {
                                            u.scheme() == "https" && u.host().is_some()
                                        })
                                    })
                            })
                            .count();
                        self.rtc_status = if n > 0 {
                            format!(
                                "Found {n} LiveKit call transport(s). Audio interoperability is not tested yet."
                            )
                        } else {
                            "No supported LiveKit transport advertised by this endpoint.".into()
                        };
                    }
                    Request::Send(id, sent) => {
                        if let Some(r) = self.live.list.get_mut(&id) {
                            r.insert(data);
                        }
                        if self.selected == id && self.editor.text() == sent {
                            self.editor = text_editor::Content::new();
                        }
                        if self.drafts.get(&id) == Some(&sent) {
                            self.drafts.remove(&id);
                        }
                        self.status.clear();
                    }
                    Request::Page(id) => {
                        if let Some(r) = self.live.list.get_mut(&id) {
                            r.page(data);
                        }
                    }
                    Request::Members(id) => {
                        if let Some(r) = self.live.list.get_mut(&id) {
                            for e in items(data).filter(|e| e["type"] == "m.room.member") {
                                if let (Some(user), Some(name)) = (
                                    e["state_key"].as_str(),
                                    e["content"]["displayname"].as_str(),
                                ) {
                                    r.members.entry(user.into()).or_insert_with(|| name.into());
                                }
                            }
                        }
                    }
                    Request::Login => self.password.clear(),
                    Request::Verify(_) if !self.verified => {
                        self.rpc("get_state", json!({}), Request::VerifyState);
                    }
                    Request::VerifyState => {
                        self.frame(json!({"command":"client_state","data":data}));
                        if !self.verified {
                            self.verification_note="gomuks finished the request but still reports this device as unverified. You can retry.".into();
                        }
                    }
                    Request::Verify(_) => {}
                }
            }
            _ => {}
        }
    }
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        let refresh_inline = match &msg {
            Msg::Frame(f) => matches!(
                f["command"].as_str(),
                Some(
                    "response"
                        | "sync_complete"
                        | "events_decrypted"
                        | "client_state"
                        | "send_complete"
                )
            ),
            Msg::Open(_)
            | Msg::Section(_)
            | Msg::Back
            | Msg::InlineReady(..)
            | Msg::ShowInline(_) => true,
            _ => false,
        };
        match msg {
            #[cfg(not(target_arch = "wasm32"))]
            Msg::PasteComposer if self.can_stage() && !self.pasting => {
                self.paste_seq += 1;
                self.pasting = true;
                let seq = self.paste_seq;
                let room = self.selected.clone();
                return Task::perform(paste::read(self.media_limit), move |result| {
                    Msg::ClipboardReady(seq, room, result)
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::ClipboardReady(seq, room, result) if seq == self.paste_seq => {
                self.pasting = false;
                if self.can_stage() && self.rooms().list.contains_key(&room) {
                    match result {
                        Ok(paste::Content::Image(f)) => {
                            return self.update(Msg::Files(room, vec![f]));
                        }
                        Ok(paste::Content::Text(s)) if self.selected == room => {
                            self.editor.perform(text_editor::Action::Edit(
                                text_editor::Edit::Paste(s.into()),
                            ))
                        }
                        Err(reason) if self.selected == room => self.status = reason.into(),
                        _ => {}
                    }
                }
            }
            Msg::InlineReady(epoch, key, img) => self.inline.complete(epoch, key, img),
            Msg::ShowInline(key) => self.inline.request(key),
            Msg::Frame(f) => self.frame(f),
            Msg::Stopped(s) => {
                self.stopped = true;
                self.connected = false;
                self.pending.clear();
                self.batch = None;
                for q in self.outbox.rooms.values_mut() {
                    for a in q {
                        a.busy = false;
                    }
                }
                self.status = s;
                if self.closing {
                    return iced::exit();
                }
            }
            Msg::Close => {
                self.closing = true;
                self.commands.close();
                if self.stopped {
                    return iced::exit();
                }
            }
            Msg::CheckCalls
                if self.logged_in
                    && self.connected
                    && !self
                        .pending
                        .values()
                        .any(|p| matches!(p, Request::RtcDiscovery)) =>
            {
                self.rtc_status = "Checking MatrixRTC transports…".into();
                self.rpc("get_rtc_transports", json!({}), Request::RtcDiscovery);
            }
            Msg::HoverFiles(v) => self.hover_files = v,
            #[cfg(not(target_arch = "wasm32"))]
            Msg::DropPaths(paths) => {
                self.hover_files = false;
                if self.can_stage() {
                    let room = self.selected.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                paths
                                    .into_iter()
                                    .take(media::MAX_FILES + 1)
                                    .filter_map(media::LocalFile::from_path)
                                    .collect()
                            })
                            .await
                            .unwrap_or_default()
                        },
                        move |files| Msg::Files(room, files),
                    );
                }
            }
            Msg::Files(mut room, files) => {
                self.hover_files = false;
                if room.is_empty() {
                    room = self.selected.clone();
                }
                if self.can_stage() && self.rooms().list.contains_key(&room) {
                    if files.is_empty() {
                        self.status = "Select regular files to attach.".into();
                    } else {
                        let (all, jobs) = self.outbox.stage(&room, files, self.media_limit);
                        if !all {
                            self.status = format!(
                                "Up to {} files per room, {} each.",
                                media::MAX_FILES,
                                media::size(self.media_limit)
                            );
                        }
                        return Task::batch(jobs.into_iter().map(|(id, file)| {
                            let room = room.clone();
                            Task::perform(preview::load(file), move |thumb| {
                                Msg::Thumbnail(room, id, thumb)
                            })
                        }));
                    }
                }
            }
            Msg::Thumbnail(room, id, thumb) => {
                if let Some(a) = self.outbox.get_mut(&room, id) {
                    a.thumbnail = thumb;
                }
            }
            Msg::PickFiles if self.can_stage() => {
                let room = self.selected.clone();
                #[cfg(not(target_arch = "wasm32"))]
                return Task::perform(
                    async {
                        let paths = rfd::AsyncFileDialog::new().pick_files().await?;
                        Some(
                            tokio::task::spawn_blocking(move || {
                                paths
                                    .into_iter()
                                    .take(media::MAX_FILES + 1)
                                    .filter_map(|p| {
                                        media::LocalFile::from_path(p.path().to_owned())
                                    })
                                    .collect()
                            })
                            .await
                            .unwrap_or_default(),
                        )
                    },
                    move |files| match files {
                        Some(f) => Msg::Files(room, f),
                        None => Msg::HoverFiles(false),
                    },
                );
                #[cfg(target_arch = "wasm32")]
                web_files::pick(&room);
            }
            Msg::RemoveFile(id) => {
                if self
                    .outbox
                    .get_mut(&self.selected, id)
                    .is_some_and(|a| !a.busy)
                {
                    self.outbox.remove(&self.selected, id);
                }
            }
            Msg::SaveFile(media) if self.connected && self.logged_in => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let name = media.name.clone();
                    return Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .set_file_name(name)
                                .save_file()
                                .await
                                .map(|p| p.path().to_owned())
                        },
                        move |p| Msg::SaveTo(media, p),
                    );
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.seq += 1;
                    if self
                        .commands
                        .try_send(backend::Command::Download {
                            id: self.seq,
                            media,
                        })
                        .is_ok()
                    {
                        self.pending.insert(self.seq, Request::Download);
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::SaveTo(media, Some(destination)) if self.connected && self.logged_in => {
                self.seq += 1;
                if self
                    .commands
                    .try_send(backend::Command::Download {
                        id: self.seq,
                        media,
                        destination,
                    })
                    .is_ok()
                {
                    self.pending.insert(self.seq, Request::Download);
                }
            }
            Msg::Section(s) => {
                self.section = s;
                self.reconcile();
            }
            Msg::Open(id) => self.open(id),
            Msg::TogglePeople => self.people = !self.people,
            Msg::Account => self.account = true,
            Msg::Back => self.account = false,
            Msg::Server(s) => self.server = s,
            Msg::Username(s) => self.username = s,
            Msg::Password(s) => self.password = s,
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Recovery(s) => self.recovery = s,
            Msg::Login
                if !self.logged_in
                    && self.connected
                    && !self.pending.values().any(|p| matches!(p, Request::Login)) =>
            {
                if let Some(url) = account::homeserver(&self.server)
                    && !self.username.trim().is_empty()
                    && !self.password.is_empty()
                {
                    self.server = url;
                    self.status = "Signing in…".into();
                    self.rpc("login",json!({"homeserver_url":self.server,"username":self.username.trim(),"password":self.password}),Request::Login);
                } else {
                    self.status =
                        "Enter your homeserver (for example, matrix.org), username, and password."
                            .into();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Verify => self.submit_verification(),
            #[cfg(target_arch = "wasm32")]
            Msg::SubmitWebRecovery(s) => {
                if self.logged_in && !self.verified {
                    self.recovery = s;
                    self.submit_verification();
                    self.recovery.clear();
                }
            }
            Msg::Draft(a) => self.editor.perform(a),
            Msg::Send => {
                // Do not send just the caption if Enter races PNG encoding.
                #[cfg(not(target_arch = "wasm32"))]
                if self.pasting {
                    return Task::none();
                }
                let body = self.editor.text();
                let id = self.selected.clone();
                if self.outbox.rooms.get(&id).is_some_and(|q| !q.is_empty()) {
                    self.send_files(id, body);
                    return Task::none();
                }
                if id.is_empty() || body.trim().is_empty() {
                    return Task::none();
                }
                if self.logged_in {
                    if self.verified
                        && !self
                            .pending
                            .values()
                            .any(|p| matches!(p,Request::Send(r,_) if r==&id))
                    {
                        self.rpc(
                            "send_message",
                            json!({"room_id":id,"text":body}),
                            Request::Send(id, body),
                        );
                    }
                } else if let Some(r) = self.demo.list.get_mut(&id) {
                    r.insert(&json!({"rowid":r.history.keys().next_back().copied().unwrap_or(0)+1,"sender":"You","type":"m.room.message",
                        "timestamp":chrono::Utc::now().timestamp_millis(),"content":{"body":body}}));
                    self.editor = text_editor::Content::new();
                }
            }
            Msg::More => self.page(),
            Msg::Copy => {
                if let Some(r) = self.rooms().list.get(&self.selected) {
                    let text = r
                        .posts()
                        .iter()
                        .map(|p| format!("{}: {}", r.display(&p.sender), p.body))
                        .collect::<Vec<_>>()
                        .join("\n");
                    #[cfg(not(target_arch = "wasm32"))]
                    return iced::clipboard::write(text);
                    #[cfg(target_arch = "wasm32")]
                    return Task::perform(
                        wasm_bindgen_futures::JsFuture::from(
                            web_sys::window()
                                .unwrap()
                                .navigator()
                                .clipboard()
                                .write_text(&text),
                        ),
                        |r| Msg::ClipboardResult(r.is_ok()),
                    );
                }
            }
            #[cfg(target_arch = "wasm32")]
            Msg::WebPaste(field) => {
                return Task::perform(
                    wasm_bindgen_futures::JsFuture::from(
                        web_sys::window()
                            .unwrap()
                            .navigator()
                            .clipboard()
                            .read_text(),
                    ),
                    move |v| {
                        v.ok()
                            .and_then(|v| v.as_string())
                            .map_or(Msg::ClipboardResult(false), |s| Msg::Pasted(field, s))
                    },
                );
            }
            #[cfg(target_arch = "wasm32")]
            Msg::Pasted(field, s) => match field {
                Field::Composer(room) if self.selected == room && !self.account => self
                    .editor
                    .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        s.into(),
                    ))),
                Field::Password if self.account && !self.logged_in => self.password = s,
                Field::Username if self.account && !self.logged_in => {
                    self.username = s.trim().into()
                }
                Field::Server if self.account && !self.logged_in => self.server = s.trim().into(),
                _ => {}
            },
            #[cfg(target_arch = "wasm32")]
            Msg::ClipboardResult(false) => {
                self.status = "Browser clipboard permission was denied.".into()
            }
            Msg::Tab(reverse) => {
                return if reverse {
                    operation::focus_previous()
                } else {
                    operation::focus_next()
                };
            }
            _ => {}
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            browser_verification::sync(self);
            let title = self.title();
            if doc.title() != title {
                doc.set_title(&title);
            }
        }
        if refresh_inline {
            self.schedule_inline()
        } else {
            Task::none()
        }
    }
    fn title(&self) -> String {
        let page = if self.account || self.logged_in && !self.verified {
            "Account"
        } else {
            self.rooms()
                .list
                .get(&self.selected)
                .map(|r| r.name.as_str())
                .unwrap_or("Rooms")
        };
        format!(
            "Untethered  ·  {page}{}",
            if self.logged_in { "" } else { "  ·  Preview" }
        )
    }
}
fn main() -> iced::Result {
    #[cfg(not(target_arch = "wasm32"))]
    fonts::configure();
    iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .font(include_bytes!("../assets/LiberationSans-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/LiberationSans-Bold.ttf").as_slice())
        .default_font(iced::Font::with_name("Liberation Sans"))
        .theme(|_: &App| Theme::Dark)
        .window(iced::window::Settings {
            size: iced::Size::new(1180., 760.),
            min_size: Some(iced::Size::new(760., 500.)),
            exit_on_close_request: false,
            ..Default::default()
        })
        .subscription(|_| {
            iced::Subscription::batch([
                #[cfg(not(target_arch = "wasm32"))]
                iced::event::listen_with(|event, _, _| match event {
                    iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                        Some(Msg::DropPaths(vec![path]))
                    }
                    iced::Event::Window(iced::window::Event::FileHovered(_)) => {
                        Some(Msg::HoverFiles(true))
                    }
                    iced::Event::Window(iced::window::Event::FilesHoveredLeft) => {
                        Some(Msg::HoverFiles(false))
                    }
                    _ => None,
                }),
                iced::window::close_requests().map(|_| Msg::Close),
                iced::keyboard::listen().filter_map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                        modifiers,
                        ..
                    } => Some(Msg::Tab(modifiers.shift())),
                    _ => None,
                }),
            ])
        })
        .run()
}
