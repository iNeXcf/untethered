//! Local presentation only: selecting a publication never changes its subscription.
use super::{BOLD, El, MUTED, Msg, RAIL, Room, TEXT, nav, surface};
use crate::desktop_call::{ActiveCall, Feed};
use iced::{
    Alignment::Center,
    Fill, Length,
    widget::{Column, button, column, container, image, row, scrollable, text},
};

fn tile(feed: Feed, room: &Room, id: uuid::Uuid, height: f32, selectable: bool) -> El<'static> {
    let caption = format!("{} · {}", room.display(&feed.user), feed.source.name());
    let content = column![
        text(caption).size(13).font(BOLD).color(TEXT),
        image(feed.frame)
            .height(height)
            .width(Fill)
            .content_fit(iced::ContentFit::Contain)
    ]
    .spacing(3);
    let card = container(content)
        .padding(5)
        .width(Fill)
        .style(|_| surface(RAIL, 6.));
    if selectable {
        button(card)
            .width(Fill)
            .padding(2)
            .style(nav(false))
            .on_press(Msg::FocusVideo(id, Some(feed.sid)))
            .into()
    } else {
        card.into()
    }
}

pub(super) fn view(call: &ActiveCall, room: &Room) -> Option<El<'static>> {
    let (focus, mut feeds) = call.previews.snapshot();
    if feeds.is_empty() {
        return None;
    }
    feeds.sort_by(|a, b| {
        room.display(&a.user)
            .cmp(room.display(&b.user))
            .then(a.sid.cmp(&b.sid))
    });
    let id = call.id;
    let wall = if let Some(i) = feeds.iter().position(|f| Some(&f.sid) == focus.as_ref()) {
        let mut stage = row![tile(feeds.remove(i), room, id, 280., false)].spacing(6);
        if !feeds.is_empty() {
            let strip =
                Column::with_children(feeds.into_iter().map(|f| tile(f, room, id, 75., true)))
                    .spacing(4);
            stage = stage.push(container(scrollable(strip)).width(150).max_height(305));
        }
        column![
            row![
                button(text("Back to grid").size(13)).on_press(Msg::FocusVideo(id, None)),
                text("Focus view").size(13)
            ]
            .spacing(8)
            .align_y(Center),
            stage
        ]
    } else {
        let cols = feeds.len().min(2);
        let height = if cols == 1 { 210. } else { 166. };
        let cells = iced::widget::grid(feeds.into_iter().map(|f| tile(f, room, id, height, true)))
            .columns(cols)
            .height(Length::Shrink)
            .spacing(6);
        column![
            text("Video grid · Select a tile to focus")
                .size(13)
                .color(MUTED),
            container(scrollable(cells)).width(Fill).max_height(350)
        ]
    };
    Some(wall.spacing(4).padding([5, 12]).into())
}
