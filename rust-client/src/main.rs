#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod account;
mod backend;
#[cfg(target_arch = "wasm32")]
mod browser_verification;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod live_test;
mod rooms;
mod view;

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
    Login,
    Verify,
    VerifyState,
    Page(String),
    Members(String),
    Send(String, String),
}
struct App {
    commands: Sender<Value>,
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
        let task = Task::batch([task, browser_verification::listen()]);
        (Self::new(commands), task)
    }
    fn new(commands: Sender<Value>) -> Self {
        Self {
            commands,
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
                .try_send(json!({"command":cmd,"request_id":self.seq,"data":data}))
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
            .any(|p| matches!(p, Request::Verify | Request::VerifyState))
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
            "Use your Matrix recovery key or recovery passphrase. Grouping spaces are fine."
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
        let key = account::recovery_input(&self.recovery);
        self.rpc("verify", json!({"recovery_key":key}), Request::Verify);
        if !self.verifying() {
            self.verification_note.clone_from(&self.status);
        }
    }
    fn frame(&mut self, f: Value) {
        let data = &f["data"];
        match f["command"].as_str().unwrap_or("") {
            "client_state" => {
                let was = self.logged_in;
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
                if was != self.logged_in {
                    self.pending.retain(|_, p| matches!(p, Request::Login));
                    self.drafts.clear();
                    self.editor = text_editor::Content::new();
                    self.loaded.clear();
                    self.selected.clear();
                    self.section = Section::All;
                    self.account = !self.logged_in;
                    if !self.logged_in {
                        self.live = Rooms::default();
                    }
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
                self.reconcile();
            }
            "events_decrypted" => {
                for e in items(&data["events"]) {
                    self.live.event(&data["room_id"], e);
                }
            }
            "send_complete" => {
                self.live.event(&Value::Null, &data["event"]);
                if !data["error"].is_null() {
                    self.status = "Message delivery failed.".into();
                }
            }
            "response" | "error" => {
                let Some(p) = self.pending.remove(&f["request_id"].as_i64().unwrap_or(0)) else {
                    return;
                };
                if f["command"] == "error" {
                    self.status = match p {
                        Request::Login => {
                            "Sign-in failed. Check your homeserver, username, and password."
                        }
                        Request::Verify | Request::VerifyState => {
                            self.verification_note = account::verify_error(data).into();
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
                    Request::Verify if !self.verified => {
                        self.rpc("get_state", json!({}), Request::VerifyState);
                    }
                    Request::VerifyState => {
                        self.frame(json!({"command":"client_state","data":data}));
                        if !self.verified {
                            self.verification_note="gomuks finished the request but still reports this device as unverified. You can retry.".into();
                        }
                    }
                    Request::Verify => {}
                }
            }
            _ => {}
        }
    }
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Frame(f) => self.frame(f),
            Msg::Stopped(s) => {
                self.stopped = true;
                self.connected = false;
                self.pending.clear();
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
                let body = self.editor.text();
                let id = self.selected.clone();
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
        Task::none()
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
