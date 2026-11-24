use super::{App, Msg, Request, Section, rooms::Room};
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Color, Element, Fill, Font, Length, Theme, color, font, keyboard,
    widget::{
        Column, Row, Space, button, column, container, image, row, rule, scrollable, stack, text,
        text_editor, text_input, tooltip,
    },
};

const RAIL: Color = color!(0x1e1f22);
const SIDE: Color = color!(0x2b2d31);
const CHAT: Color = color!(0x313338);
const TRAY: Color = color!(0x232428);
const HOVER: Color = color!(0x35373c);
const ACTIVE: Color = color!(0x404249);
const BLURPLE: Color = color!(0x5865f2);
const TEXT: Color = color!(0xf2f3f5);
const MUTED: Color = color!(0xb5bac1);
const DIM: Color = color!(0x969ba4);
const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::with_name("Liberation Sans")
};
type El<'a> = Element<'a, Msg>;

fn surface(c: Color, r: f32) -> container::Style {
    container::Style {
        background: Some(c.into()),
        border: Border::default().rounded(r),
        ..Default::default()
    }
}
fn nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| button::Style {
        background: Some(
            (if active {
                ACTIVE
            } else if s == button::Status::Hovered {
                HOVER
            } else {
                Color::TRANSPARENT
            })
            .into(),
        ),
        text_color: if active { TEXT } else { MUTED },
        border: Border::default().rounded(5),
        ..Default::default()
    }
}
fn avatar(name: &str, size: f32) -> El<'static> {
    let colors = [
        color!(0x5865a1),
        color!(0x337a79),
        color!(0x95613d),
        color!(0x865b89),
        color!(0x536bab),
    ];
    let c = colors[name.bytes().fold(0usize, |a, b| a + b as usize) % colors.len()];
    let initials = name
        .split_whitespace()
        .take(2)
        .filter_map(|s| s.chars().next())
        .collect::<String>()
        .to_uppercase();
    container(
        text(initials)
            .size(size * 0.42)
            .font(BOLD)
            .color(Color::WHITE),
    )
    .center(size)
    .style(move |_| surface(c, size / 2.))
    .into()
}
fn label(s: &str) -> El<'_> {
    text(s).font(BOLD).size(11).color(MUTED).into()
}
fn attachment_card<'a>(room: &str, a: &'a crate::media::Attachment) -> El<'a> {
    let name = &a.source.name;
    let ext = name
        .rsplit_once('.')
        .map(|(_, s)| s)
        .filter(|s| !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("FILE")
        .to_ascii_uppercase();
    let tint = match ext.as_str() {
        "PDF" => color!(0xf2969d),
        "ZIP" | "7Z" | "GZ" => color!(0xe7be7c),
        "WAV" | "MP3" | "OGG" | "MP4" | "WEBM" => color!(0xc4adee),
        _ => color!(0xa7b6f6),
    };
    let visual: El = if let Some(thumb) = &a.thumbnail {
        image(thumb)
            .width(Fill)
            .height(76)
            .content_fit(iced::ContentFit::Contain)
            .into()
    } else {
        container(
            container(text(ext).size(13).font(BOLD).color(RAIL))
                .padding([5, 8])
                .style(move |_| surface(tint, 5.)),
        )
        .center_x(Fill)
        .center_y(76)
        .into()
    };
    let action = if let Some(u) = &a.upload {
        Some(Msg::CancelUpload(room.into(), a.id, u.request))
    } else {
        (!a.busy).then_some(Msg::RemoveFile(a.id))
    };
    let remove = tooltip(
        button(
            text(if a.upload.is_some() { "Cancel" } else { "×" })
                .size(14)
                .font(BOLD),
        )
        .padding([0, 6])
        .on_press_maybe(action)
        .style(|_, s| button::Style {
            background: Some(
                if s == button::Status::Hovered {
                    color!(0x943e47)
                } else {
                    TRAY
                }
                .into(),
            ),
            border: Border::default().rounded(6),
            text_color: TEXT,
            ..Default::default()
        }),
        if a.upload.is_some() {
            "Cancel upload and keep draft"
        } else {
            "Remove attachment"
        },
        tooltip::Position::Top,
    );
    let picture = stack![
        container(visual).width(Fill).style(|_| surface(RAIL, 6.)),
        container(remove).padding(3).align_right(Fill)
    ];
    let short = if name.chars().count() > 21 {
        let head: String = name.chars().take(13).collect();
        let tail: String = name
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}…{tail}")
    } else {
        name.clone()
    };
    let state = a.status_label();
    let meta = text(format!("{}  ·  {state}", crate::media::size(a.source.size)))
        .size(11)
        .color(if a.error.is_some() {
            color!(0xffa5ad)
        } else {
            MUTED
        });
    let meta: El = if let Some(err) = &a.error {
        tooltip(meta, text(err).size(13), tooltip::Position::Top).into()
    } else {
        meta.into()
    };
    let mut card = column![
        picture,
        tooltip(
            text(short).size(13).height(17).font(BOLD),
            text(name).size(13),
            tooltip::Position::Top
        ),
        meta
    ]
    .spacing(3);
    if let Some(n) = a.upload.as_ref().and_then(|u| u.percent) {
        card = card.push(
            iced::widget::progress_bar(0.0..=100.0, n as f32)
                .length(Fill)
                .girth(3),
        );
    }
    container(card)
        .padding(7)
        .width(158)
        .style(|_| surface(SIDE, 8.))
        .into()
}
fn rail_button<'a>(glyph: String, hint: &'a str, active: bool, msg: Msg) -> El<'a> {
    let b = button(container(text(glyph).font(BOLD).color(TEXT).size(19)).center(44))
        .padding(0)
        .on_press(msg)
        .style(move |_, s| {
            let highlighted = active || s == button::Status::Hovered;
            button::Style {
                background: Some(if highlighted {
                    BLURPLE.into()
                } else {
                    CHAT.into()
                }),
                border: Border::default().rounded(if highlighted { 14 } else { 22 }),
                ..Default::default()
            }
        });
    let marker = container(Space::new())
        .width(4)
        .height(if active { 30 } else { 0 })
        .style(|_| surface(TEXT, 3.));
    tooltip(
        row![marker, b].spacing(7).align_y(Center),
        container(text(hint).size(14)).padding(6),
        tooltip::Position::Right,
    )
    .style(|_| surface(RAIL, 5.))
    .gap(8)
    .into()
}

impl App {
    #[cfg(target_os = "linux")]
    fn call_bar(&self) -> Option<El<'_>> {
        use crate::call_devices::Port;
        use iced::widget::pick_list;
        let call = self.call.as_ref()?;
        let mode = call.audio.borrow();
        let name = self
            .live
            .list
            .get(&call.room)
            .map_or("DM", |r| r.name.as_str());
        let active = !call.cancel.is_closed();
        let mic = if mode.deafened {
            "Mic muted"
        } else if mode.muted {
            "Unmute"
        } else {
            "Mute"
        };
        let controls = row![
            button(text(mic).size(15).font(BOLD))
                .style(nav(mode.muted || mode.deafened))
                .on_press_maybe((active && !mode.deafened).then_some(Msg::ToggleMic)),
            button(
                text(if mode.deafened { "Undeafen" } else { "Deafen" })
                    .size(15)
                    .font(BOLD)
            )
            .style(nav(mode.deafened))
            .on_press_maybe(active.then_some(Msg::ToggleDeafen)),
            button(text("Devices").size(15).font(BOLD))
                .style(nav(call.picker))
                .on_press_maybe(active.then_some(Msg::AudioDevices)),
            button(
                text(if call.group { "Leave voice" } else { "Hang up" })
                    .size(15)
                    .font(BOLD)
            )
            .on_press_maybe(active.then_some(Msg::EndCall))
        ]
        .spacing(8)
        .align_y(Center);
        let mut controls = controls;
        if call.group {
            for input in crate::desktop_call::VideoInput::ALL {
                let slot = input as usize;
                let requested = mode.video[slot].is_some();
                let label = input.control(requested, call.video_status[slot].live);
                controls = controls.push(
                    button(text(label).size(15).font(BOLD))
                        .style(nav(requested))
                        .on_press_maybe(active.then_some(Msg::ToggleVideo(input))),
                );
            }
        }
        let mut bar = column![
            text(format!("{} · {name}", call.label))
                .size(13)
                .color(TEXT),
            controls.wrap()
        ]
        .spacing(3);
        for status in &call.video_status {
            if !status.label.is_empty() {
                bar = bar.push(text(&status.label).size(13));
            }
        }
        if call.mic_error {
            bar =
                bar.push(text("Microphone unavailable · choose an input under Devices.").size(13));
        }
        if call.speaker_error {
            bar = bar.push(text("Speaker unavailable · choose an output under Devices.").size(13));
        }
        if call.picker {
            for (label, port, route) in [
                ("Microphone", Port::Mic, &mode.mic),
                ("Speaker", Port::Speaker, &mode.speaker),
            ] {
                bar = bar.push(
                    row![
                        text(label).width(90).size(14),
                        pick_list(
                            call.devices.list(port),
                            Some(call.devices.selected(port, route)),
                            move |choice| Msg::SelectDevice(port, choice)
                        )
                        .width(Fill)
                        .text_size(14)
                        .padding([5, 9])
                    ]
                    .align_y(Center)
                    .spacing(8),
                );
            }
            let refresh = button(
                text(if call.loading {
                    "Loading…"
                } else {
                    "Refresh devices"
                })
                .size(14)
                .font(BOLD),
            )
            .style(nav(false))
            .on_press_maybe((active && !call.loading).then_some(Msg::RefreshDevices));
            bar = bar.push(
                row![refresh, text(call.list_error.unwrap_or("")).size(13)]
                    .align_y(Center)
                    .spacing(8),
            );
        }
        Some(
            container(bar)
                .width(Fill)
                .padding([6, 12])
                .style(|_| surface(TRAY, 4.))
                .into(),
        )
    }
    #[cfg(target_os = "linux")]
    fn incoming_banner(&self) -> Option<El<'_>> {
        let (i, _) = self.inbox.ringing.as_ref()?;
        Some(
            container(
                row![
                    text(format!("Incoming voice call · {}", i.peer))
                        .width(Fill)
                        .size(15),
                    button(text("Decline").size(16).font(BOLD))
                        .padding([5, 14])
                        .style(nav(false))
                        .on_press(Msg::DecideRing(i.event.clone(), false)),
                    button(text("Accept").size(16).font(BOLD))
                        .padding([5, 14])
                        .style(button::success)
                        .on_press(Msg::DecideRing(i.event.clone(), true))
                ]
                .align_y(Center)
                .spacing(9),
            )
            .padding([7, 12])
            .style(|_| surface(TRAY, 4.))
            .into(),
        )
    }
    pub(super) fn view(&self) -> El<'_> {
        if self.account || self.logged_in && !self.verified {
            return self.account_view();
        }
        let store = self.rooms();
        let mut icons = column![
            rail_button(
                "U".into(),
                "Home · direct messages",
                self.section == Section::Direct,
                Msg::Section(Section::Direct)
            ),
            rail_button(
                "#".into(),
                "All rooms",
                self.section == Section::All,
                Msg::Section(Section::All)
            ),
            container(rule::horizontal(1)).width(36).padding([4, 0])
        ]
        .spacing(8)
        .align_x(Center);
        for (id, r) in &store.list {
            if r.space {
                let s = Section::Space(id.clone());
                let initials = r
                    .name
                    .split_whitespace()
                    .take(2)
                    .filter_map(|s| s.chars().next())
                    .collect::<String>()
                    .to_uppercase();
                icons = icons.push(rail_button(
                    initials,
                    &r.name,
                    self.section == s,
                    Msg::Section(s),
                ));
            }
        }
        let rail = container(scrollable(icons))
            .padding([14, 0])
            .width(70)
            .height(Fill)
            .style(|_| surface(RAIL, 0.));
        let section_name = match &self.section {
            Section::All => "All rooms",
            Section::Direct => "Direct messages",
            Section::Space(id) => store
                .list
                .get(id)
                .map(|r| r.name.as_str())
                .unwrap_or("Space"),
        };
        let head = container(text(section_name).font(BOLD).size(17).color(TEXT))
            .padding([0, 16])
            .center_y(52)
            .width(Fill);
        let mut links = Column::new().spacing(2).padding([0, 8]);
        for (id, r) in self.visible() {
            let prefix = if r.voice {
                "♪"
            } else if r.dm {
                "@"
            } else {
                "#"
            };
            let active = id == &self.selected;
            let entry = row![
                text(prefix).size(22).color(DIM),
                text(&r.name)
                    .size(15)
                    .width(Fill)
                    .color(if active || r.unread { TEXT } else { MUTED }),
                text(if r.unread { "•" } else { "" }).color(TEXT)
            ]
            .spacing(9)
            .align_y(Center);
            links = links.push(
                button(entry)
                    .width(Fill)
                    .padding([5, 9])
                    .on_press(Msg::Open(id.clone()))
                    .style(nav(active)),
            );
            #[cfg(target_os = "linux")]
            if r.voice {
                let participants: std::collections::BTreeSet<_> = r
                    .voice_members
                    .values()
                    .filter(|(_, expires)| *expires > crate::desktop_call::now())
                    .map(|(user, _)| user)
                    .collect();
                for user in participants {
                    links = links.push(
                        container(text(r.display(user)).size(13).color(MUTED)).padding([0, 28]),
                    );
                }
            }
        }
        let name = if self.logged_in {
            self.profile.as_str()
        } else {
            "Preview user"
        };
        let user = button(
            row![
                avatar(name, 31.),
                column![
                    text(name).size(14).font(BOLD),
                    text(if self.logged_in {
                        "Matrix account"
                    } else {
                        "Local preview"
                    })
                    .size(11)
                    .color(MUTED)
                ]
            ]
            .spacing(10)
            .align_y(Center),
        )
        .padding(9)
        .width(Fill)
        .on_press(Msg::Account)
        .style(nav(false));
        let side = container(column![
            head,
            rule::horizontal(1),
            container(label(if self.section == Section::Direct {
                "DIRECT MESSAGES"
            } else {
                "CHANNELS"
            }))
            .padding([14, 16]),
            scrollable(links).height(Fill),
            container(user).width(Fill).style(|_| surface(TRAY, 0.))
        ])
        .width(211)
        .height(Fill)
        .style(|_| surface(SIDE, 0.));
        let room = store.list.get(&self.selected);
        let channel = room.map(|r| r.name.as_str()).unwrap_or("Rooms");
        let header = row![
            text(if room.is_some_and(|r| r.voice) {
                "♪"
            } else if room.is_some_and(|r| r.dm) {
                "@"
            } else {
                "#"
            })
            .size(25)
            .color(DIM),
            text(channel).font(BOLD).size(17),
            Space::new().width(Fill),
            button(text("People").size(13))
                .on_press(Msg::TogglePeople)
                .style(nav(self.people)),
            button(text("Copy chat").size(13))
                .on_press_maybe(room.map(|_| Msg::Copy))
                .style(nav(false))
        ]
        .spacing(9)
        .align_y(Center);
        #[cfg(target_os = "linux")]
        let header = if self.call.is_none()
            && self.verified
            && self.connected
            && self.inbox.ringing.is_none()
            && room.is_some_and(|r| r.encrypted && (r.voice || r.dm_peer.is_some()))
        {
            if room.is_some_and(|r| r.voice) {
                header.push(
                    button(text("Join voice").size(16).font(BOLD))
                        .padding([5, 12])
                        .style(nav(false))
                        .on_press(Msg::JoinVoice),
                )
            } else {
                header.push(button("Voice call").on_press(Msg::VoiceCall))
            }
        } else {
            header
        };
        let banner = if !self.logged_in {
            Some(
                container(
                    row![
                        text("PREVIEW").size(11).font(BOLD).color(color!(0xb7bcff)),
                        text("Explore a local room").size(13).width(Fill),
                        button(text("Sign in to Matrix").size(13).font(BOLD))
                            .on_press(Msg::Account)
                            .style(button::primary)
                    ]
                    .spacing(10)
                    .align_y(Center),
                )
                .width(Fill)
                .padding([7, 16])
                .style(|_| surface(color!(0x30334a), 0.)),
            )
        } else {
            None
        };
        let mut chat = column![
            container(header).padding([0, 17]).center_y(52).width(Fill),
            rule::horizontal(1)
        ];
        if let Some(banner) = banner {
            chat = chat.push(banner)
        }
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            chat = chat.push(b);
        }
        #[cfg(target_os = "linux")]
        if let Some(bar) = self.call_bar() {
            chat = chat.push(bar);
        }
        #[cfg(target_os = "linux")]
        if let Some(call) = &self.call
            && call.room == self.selected
        {
            for (caption, frame) in call.previews.tiles() {
                chat = chat.push(
                    column![
                        text(caption).size(13),
                        image(frame)
                            .height(210)
                            .width(Fill)
                            .content_fit(iced::ContentFit::Contain)
                    ]
                    .padding([4, 16]),
                );
            }
        }
        if !self.status.is_empty() && !self.status.starts_with("Starting") {
            chat = chat.push(
                container(text(&self.status).size(13).color(color!(0xffc785))).padding([5, 16]),
            );
        }
        let body: El = if let Some(r) = room {
            self.conversation(r)
        } else {
            container(
                column![
                    text("No rooms here yet").font(BOLD).size(24),
                    text("Choose another space or open All rooms.").color(MUTED)
                ]
                .spacing(6),
            )
            .center(Fill)
            .into()
        };
        let mut middle = Row::new().push(body).height(Fill);
        if self.people
            && let Some(r) = room
        {
            let people: std::collections::BTreeSet<_> =
                r.history.values().map(|p| p.sender.as_str()).collect();
            let mut list = column![label("IN THIS CONVERSATION")]
                .spacing(13)
                .padding(14);
            for sender in people {
                let n = r.display(sender);
                list = list.push(
                    row![avatar(n, 28.), text(n).size(14)]
                        .spacing(8)
                        .align_y(Center),
                );
            }
            middle = middle.push(
                container(scrollable(list))
                    .width(178)
                    .height(Fill)
                    .style(|_| surface(SIDE, 0.)),
            );
        }
        chat = chat.push(middle);
        row![
            rail,
            side,
            container(chat)
                .height(Fill)
                .width(Fill)
                .style(|_| surface(CHAT, 0.))
        ]
        .height(Fill)
        .into()
    }
    fn conversation<'a>(&'a self, r: &'a Room) -> El<'a> {
        let mut feed = Column::new().padding([16, 18]).spacing(2);
        if self.logged_in && !r.end {
            feed = feed.push(
                button(text("Earlier messages").size(12))
                    .on_press_maybe(self.connected.then_some(Msg::More))
                    .style(nav(false)),
            );
        }
        if r.end || !self.logged_in {
            feed = feed
                .push(
                    container(text(if r.dm { "@" } else { "#" }).font(BOLD).size(30))
                        .center(52)
                        .style(|_| surface(ACTIVE, 26.)),
                )
                .push(
                    text(format!(
                        "{}{}",
                        if r.dm { "" } else { "Welcome to #" },
                        r.name
                    ))
                    .font(BOLD)
                    .size(28)
                    .color(TEXT),
                )
                .push(
                    text(if r.topic.is_empty() {
                        "This is the start of the conversation."
                    } else {
                        &r.topic
                    })
                    .size(14)
                    .color(MUTED),
                );
        }
        let mut prev: Option<(&str, i64, String)> = None;
        for p in r.posts() {
            let stamp = DateTime::from_timestamp_millis(p.time)
                .unwrap_or_default()
                .with_timezone(&Local);
            let day = stamp.format("%B %-d, %Y").to_string();
            let date_change = prev.as_ref().is_none_or(|(_, _, d)| d != &day);
            let group = date_change
                || prev
                    .as_ref()
                    .is_none_or(|(s, t, _)| *s != p.sender || p.time - *t > 300_000);
            if date_change {
                feed = feed.push(
                    container(
                        row![
                            rule::horizontal(1),
                            text(day.clone()).size(11).color(MUTED),
                            rule::horizontal(1)
                        ]
                        .spacing(8)
                        .align_y(Center),
                    )
                    .padding([9, 0]),
                );
            }
            let name = r.display(&p.sender);
            if group {
                let info = row![
                    text(name).size(16).font(BOLD).color(TEXT),
                    text(stamp.format("%H:%M").to_string())
                        .size(11)
                        .color(MUTED)
                ]
                .spacing(9)
                .align_y(Center);
                feed = feed.push(
                    container(
                        row![
                            avatar(name, 36.),
                            column![info, text(&p.body).size(16)].spacing(2).width(Fill)
                        ]
                        .spacing(13),
                    )
                    .padding([5, 0]),
                );
            } else {
                feed = feed.push(
                    row![Space::new().width(36), text(&p.body).size(16).width(Fill)].spacing(13),
                );
            }
            prev = Some((&p.sender, p.time, day));
            if let Some(file) = &p.media {
                if let Some(source) = &file.preview {
                    let preview: El = if let Some(handle) = self.inline.ready(source) {
                        image(handle)
                            .width(320)
                            .height(208)
                            .content_fit(iced::ContentFit::Contain)
                            .border_radius(8)
                            .into()
                    } else if self.inline.pending(source) {
                        text("Loading image…").size(12).color(DIM).into()
                    } else {
                        button(
                            text(if self.inline.failed(source) {
                                "Retry image preview"
                            } else {
                                "Show image preview"
                            })
                            .size(12),
                        )
                        .on_press(Msg::ShowInline(source.clone()))
                        .style(nav(false))
                        .into()
                    };
                    feed = feed.push(row![Space::new().width(36), preview].spacing(13));
                }
                feed = feed.push(
                    row![
                        Space::new().width(36),
                        button(
                            text(format!(
                                "Save {}  {}",
                                file.name,
                                file.size.map(crate::media::size).unwrap_or_default()
                            ))
                            .size(14)
                        )
                        .on_press(Msg::SaveFile(file.clone()))
                        .style(button::secondary)
                    ]
                    .spacing(13),
                );
            }
        }
        let feed = scrollable(feed)
            .id(iced::widget::Id::from(format!(
                "timeline-{}",
                self.selected
            )))
            .anchor_bottom()
            .height(Fill);
        let hint = format!("Message {}{}", if r.dm { "@" } else { "#" }, r.name);
        let editor = text_editor(&self.editor)
            .placeholder(hint)
            .on_action(Msg::Draft)
            .key_binding(|press| {
                #[cfg(target_arch = "wasm32")]
                if press.modifiers.command()
                    && press.key.as_ref() == keyboard::Key::Character("v")
                    && matches!(press.status, text_editor::Status::Focused { .. })
                {
                    return Some(text_editor::Binding::Custom(Msg::WebPaste(
                        super::Field::Composer(self.selected.clone()),
                    )));
                }
                if press.key == keyboard::Key::Named(keyboard::key::Named::Enter)
                    && press.modifiers.is_empty()
                    && matches!(press.status, text_editor::Status::Focused { .. })
                {
                    Some(text_editor::Binding::Custom(Msg::Send))
                } else {
                    let binding = text_editor::Binding::from_key_press(press);
                    #[cfg(not(target_arch = "wasm32"))]
                    if matches!(binding, Some(text_editor::Binding::Paste)) {
                        return Some(text_editor::Binding::Custom(Msg::PasteComposer));
                    }
                    binding
                }
            })
            .padding(11)
            .height(Length::Shrink)
            .min_height(42)
            .max_height(100);
        let sending = self
            .pending
            .values()
            .any(|p| matches!(p,Request::Send(id,_) if id==&self.selected));
        let files = self.outbox.rooms.get(&self.selected);
        let has_files = files.is_some_and(|q| !q.is_empty());
        let uploading = self.batch.as_ref().is_some_and(|b| b.room == self.selected);
        let mut tray = Column::new().spacing(4);
        if self.hover_files {
            tray = tray.push(
                text(format!("Drop files to attach to {}", r.name))
                    .color(TEXT)
                    .font(BOLD),
            );
        }
        if let Some(files) = files.filter(|q| !q.is_empty()) {
            let cards =
                Row::with_children(files.iter().map(|a| attachment_card(&self.selected, a)))
                    .spacing(8)
                    .padding([2, 0]);
            tray = tray.push(scrollable(cards).horizontal().width(Fill).height(138));
        }
        let composer = row![
            tooltip(
                button(text("+").size(20))
                    .on_press(Msg::PickFiles)
                    .style(button::secondary),
                "Attach files",
                tooltip::Position::Top
            ),
            editor,
            button(
                text(if sending || uploading {
                    "Sending…"
                } else if files.is_some_and(|q| q.iter().any(|a| a.cancelled || a.error.is_some()))
                {
                    "Retry"
                } else {
                    "Send"
                })
                .size(14)
                .font(BOLD)
            )
            .on_press_maybe(
                (!sending
                    && (!self.logged_in || self.connected)
                    && (has_files && self.batch.is_none()
                        || !has_files && !self.editor.text().trim().is_empty()))
                .then_some(Msg::Send)
            )
        ]
        .spacing(8)
        .align_y(Center);
        let foot = if !self.logged_in {
            "Local preview · Messages stay here"
        } else if r.encrypted {
            "End-to-end encrypted"
        } else {
            "Matrix room"
        };
        column![
            feed,
            container(
                column![
                    tray,
                    composer,
                    row![
                        text(foot).size(11).color(DIM),
                        Space::new().width(Fill),
                        text("Shift + Enter for a new line").size(11).color(DIM)
                    ]
                ]
                .spacing(5)
            )
            .padding([10, 16])
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }
    fn account_view(&self) -> El<'_> {
        macro_rules! field {
            ($label:expr,$value:expr,$kind:ident) => {{
                let input = text_input($label, $value).on_input(Msg::$kind).padding(11);
                #[cfg(target_arch = "wasm32")]
                let input = input.on_paste(|_| Msg::WebPaste(super::Field::$kind));
                input
            }};
        }
        let mut form = column![label("UNTETHERED  /  ACCOUNT")]
            .spacing(14)
            .width(362);
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            form = form.push(b);
        }
        #[cfg(target_os = "linux")]
        if let Some(bar) = self.call_bar() {
            form = form.push(bar);
        }
        if !self.logged_in {
            form = form
                .push(text("Welcome back").size(32).font(BOLD))
                .push(text("Sign in to your Matrix homeserver.").color(MUTED))
                .push(field!("Homeserver · matrix.org", &self.server, Server))
                .push(field!("Matrix username", &self.username, Username))
                .push(
                    field!("Password", &self.password, Password)
                        .secure(true)
                        .on_submit(Msg::Login),
                )
                .push(
                    button("Sign in").padding([9, 16]).on_press_maybe(
                        (self.connected
                            && !self.pending.values().any(|p| matches!(p, Request::Login)))
                        .then_some(Msg::Login),
                    ),
                );
        } else if !self.verified {
            // The web build uses a real password input in a browser dialog,
            // so keyboard, context-menu paste and password managers work normally.
            #[cfg(not(target_arch = "wasm32"))]
            {
                form = form
                    .push(text("Verify this device").size(28).font(BOLD))
                    .push(text(&self.matrix_id))
                    .push(
                        text_input("Recovery key or passphrase", &self.recovery)
                            .on_input(Msg::Recovery)
                            .padding(11)
                            .secure(true)
                            .on_submit(Msg::Verify),
                    )
                    .push(
                        button(if self.verifying() {
                            "Verifying…"
                        } else {
                            "Verify device"
                        })
                        .on_press_maybe(
                            (self.connected
                                && !self.verifying()
                                && !self.recovery_unavailable
                                && !self.recovery.trim().is_empty())
                            .then_some(Msg::Verify),
                        ),
                    )
                    .push(text(self.verification_feedback()).size(14))
                    .push(
                        text("Emoji comparison isn’t available with this version of gomuks.")
                            .size(13)
                            .color(MUTED),
                    );
            }
        } else {
            form = form
                .push(text(&self.profile).size(25).font(BOLD))
                .push(text("Verified Matrix device").color(MUTED))
                .push(
                    button("Check call server support")
                        .on_press_maybe(self.connected.then_some(Msg::CheckCalls)),
                )
                .push(text(&self.rtc_status).size(13));
        }
        if (!self.logged_in || self.verified) && !self.status.is_empty() {
            form = form.push(text(&self.status).size(14).color(MUTED));
        }
        if !self.logged_in || self.verified {
            form = form.push(
                button(if self.logged_in {
                    "Back to rooms"
                } else {
                    "Back to preview"
                })
                .on_press(Msg::Back)
                .style(button::secondary),
            );
        }
        container(container(form).padding(30).style(|_| surface(SIDE, 12.)))
            .center(Fill)
            .style(|_| surface(RAIL, 0.))
            .into()
    }
}
