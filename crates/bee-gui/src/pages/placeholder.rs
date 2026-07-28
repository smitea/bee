//! Generic "Coming in S-X" placeholder for tabs whose real functionality
//! ships in later stories (S-2 data management, S-3 pipelines, S-5 settings).

use iced::{
    widget::{Column, Container, Text},
    Element, Length,
};
use crate::app::Message;
use crate::icons;
use crate::theme;

pub fn view<'a>(tab_name: &'a str, target_story: &'a str, icon: &'a [u8]) -> Element<'a, Message> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_4)
            .align_items(iced::alignment::Alignment::Center)
            .push(icons::render(icon, 64, iced::Color::from_rgb(0.6, 0.6, 0.6)))
            .push(Text::new(tab_name).size(20))
            .push(
                Text::new(format!("此功能将在 {} 中实现", target_story)).size(13),
            ),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x()
    .center_y()
    .into()
}