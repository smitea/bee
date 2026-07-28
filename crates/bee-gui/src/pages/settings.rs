//! S-5: Settings page.
//!
//! Shows app version, connection details, theme picker (mirrors the
//! AppBar toggle for discoverability), and a log-level selector that
//! pushes a tracing directive into the runtime filter. The "About"
//! block links to the S-1a spec for the design contract.

use iced::{
    widget::{Button, Column, Container, PickList, Row, Text},
    Element, Length,
};

use crate::app::ThemeKind;
use crate::icons;
use crate::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const ALL: [LogLevel; 4] = [Self::Debug, Self::Info, Self::Warn, Self::Error];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    ThemeChanged(ThemeKind),
    LogLevelChanged(LogLevel),
    OpenSpecLink,
    ExportLogPressed,
}

pub fn view(
    current_theme: ThemeKind,
    current_log_level: &LogLevel,
    connection_addr: std::net::SocketAddr,
    connection_state: &'static str,
    log_path_str: &'static str,
) -> Element<'static, SettingsMsg> {
    let _ = connection_addr;
    Column::new()
        .spacing(theme::SPACE_6)
        .padding(theme::SPACE_8)
        .push(header())
        .push(section_title("Application"))
        .push(app_section())
        .push(section_title("Connection"))
        .push(connection_section_static(connection_state))
        .push(section_title("Theme"))
        .push(theme_section(current_theme))
        .push(section_title("Log Level"))
        .push(log_level_section(current_log_level))
        .push(section_title("Diagnostics"))
        .push(diagnostics_section(log_path_str))
        .push(section_title("About"))
        .push(about_section())
        .into()
}

fn header() -> Element<'static, SettingsMsg> {
    Row::new()
        .push(Text::new("Settings").size(20))
        .push(iced::widget::Space::with_width(Length::Fill))
        .push(Text::new("Settings").size(11))
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

fn section_title(title: &'static str) -> Element<'static, SettingsMsg> {
    Container::new(Text::new(title).size(15))
        .padding([theme::SPACE_2 as f32, 0.0])
        .into()
}

fn app_section() -> Element<'static, SettingsMsg> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(field("Name", "bee-gui"))
            .push(field("Version", env!("CARGO_PKG_VERSION")))
            .push(field("Crate type", "binary (cdylib optional via cargo-deb)")),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn connection_section_static(state: &'static str) -> Element<'static, SettingsMsg> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(field("Connection state", state)),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn theme_section(current: ThemeKind) -> Element<'static, SettingsMsg> {
    let choices = ThemeKind::ALL.to_vec();
    let label = Text::new(format!("current: {}", current.as_str())).size(11);
    let pick = PickList::new(choices, Some(current), SettingsMsg::ThemeChanged)
        .width(Length::Fixed(160.0));
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(label)
            .push(pick),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn log_level_section(current: &LogLevel) -> Element<'static, SettingsMsg> {
    let choices = LogLevel::ALL.to_vec();
    let pick = PickList::new(choices, Some(current.clone()), SettingsMsg::LogLevelChanged)
        .width(Length::Fixed(160.0));
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(
                Text::new(
                    "Tracing filter applied via RUST_LOG or --log-level CLI flag.",
                )
                .size(11),
            )
            .push(pick),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn diagnostics_section(log_path_str: &'static str) -> Element<'static, SettingsMsg> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(field(
                "Log file (1000-entry FIFO ring)",
                log_path_str,
            ))
            .push(
                Row::new()
                    .push(
                        Button::new(
                            Row::new()
                                .push(icons::render(icons::DOWNLOAD, 14, iced::Color::BLACK))
                                .push(iced::widget::Space::with_width(theme::SPACE_1))
                                .push(Text::new("Export log to file").size(13)),
                        )
                        .on_press(SettingsMsg::ExportLogPressed)
                        .padding([6, 10]),
                    )
                    .align_items(iced::alignment::Alignment::Center),
            ),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn about_section() -> Element<'static, SettingsMsg> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(Text::new("Bee cluster management GUI").size(13))
            .push(Text::new("iced 0.12 + Rust stable 1.89.0").size(11))
            .push(
                Text::new(
                    "Spec: docs/superpowers/specs/2026-07-27-s1a-gui-foundation-design.md",
                )
                .size(11),
            )
            .push(
                Button::new(Text::new("Open spec (local file)").size(13))
                    .on_press(SettingsMsg::OpenSpecLink)
                    .padding([4, 8]),
            ),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn field(label: &'static str, value: &'static str) -> Element<'static, SettingsMsg> {
    Row::new()
        .push(Text::new(format!("{}: ", label)).size(11).width(Length::Fixed(160.0)))
        .push(Text::new(value).size(13))
        .spacing(theme::SPACE_2)
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_as_str_matches_cli() {
        // The CLI parses `--log-level <debug|info|warn|error>`. The
        // SettingsPage LogLevel enum must serialize to those strings
        // so RUST_LOG can be set from the picker.
        assert_eq!(LogLevel::Debug.as_str(), "debug");
        assert_eq!(LogLevel::Info.as_str(), "info");
        assert_eq!(LogLevel::Warn.as_str(), "warn");
        assert_eq!(LogLevel::Error.as_str(), "error");
    }

    #[test]
    fn env_pkg_version_is_not_empty() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}