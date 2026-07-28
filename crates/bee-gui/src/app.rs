//! Root App<Message> for iced.
//!
//! Implements the `iced::Application` trait so `iced::Application::run` is
//! the single entry point used by `main.rs`.
//!
//! - `update(msg)` routes messages to pages AND drains pending
//!   `ConnectionMsg`s from the connection thread.

use iced::{
    executor::Default,
    widget::{Column, Container, Row, Text},
    Application, Command, Element, Length, Subscription, Theme,
};

use crate::connection::{
    try_drain, ConnectionBundle, ConnectionHandle, ConnectionMsg, ConnectionState,
};
use crate::datasource_registry::DataMgmtState;
use crate::icons;
use crate::log_panel::LogRing;
use crate::pages::data_mgmt::{self, DataFormState, DataMsg};
use crate::pages::dashboard::{self, DashboardData, DashboardMsg};
use crate::pages::placeholder;
use crate::pages::settings::{self, LogLevel, SettingsMsg};
use crate::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    DataMgmt,
    Pipelines,
    Settings,
}

/// S-1b: 3-way theme selector. `System` follows the OS preference
/// (light by default in MVP — the OS signal hookup is a 1.x concern).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Light,
    Dark,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 2] = [Self::Light, Self::Dark];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
    pub fn cycle(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

impl std::fmt::Display for ThemeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Flags passed to `App::new` via `iced::Settings::with_flags`.
pub struct Flags {
    pub bundle: ConnectionBundle,
    pub log: LogRing,
    pub theme_kind: ThemeKind,
}

pub struct App {
    pub tab: Tab,
    pub conn: ConnectionHandle,
    pub msg_rx: tokio::sync::mpsc::Receiver<ConnectionMsg>,
    pub log: LogRing,
    pub dashboard: DashboardData,
    pub theme_kind: ThemeKind,
    pub log_level: LogLevel,
    pub dm: DataMgmtState,
    pub dm_form: DataFormState,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Dashboard(DashboardMsg),
    Data(DataMsg),
    Settings(SettingsMsg),
    CycleTheme,
    PumpTick,
}

impl Application for App {
    type Executor = Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = Flags;

    fn new(flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let bundle = flags.bundle;
        let conn = bundle.handle.clone();
        let msg_rx = bundle.receiver;
        let app = Self {
            tab: Tab::Dashboard,
            conn,
            msg_rx,
            log: flags.log,
            dashboard: DashboardData::default(),
            theme_kind: flags.theme_kind,
            log_level: LogLevel::Info,
            dm: DataMgmtState::new(),
            dm_form: DataFormState::default(),
        };
        // Kick the first Refresh so the dashboard has data on launch.
        dashboard::trigger_refresh(&app.conn);
        (app, Command::perform(async {}, |_| Message::PumpTick))
    }

    fn title(&self) -> String {
        "Bee GUI".to_string()
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::TabSelected(t) => {
                self.tab = t;
                Command::none()
            }
            Message::Dashboard(d) => match d {
                DashboardMsg::RefreshPressed => {
                    dashboard::trigger_refresh(&self.conn);
                    Command::perform(async {}, |_| Message::PumpTick)
                }
            },
            Message::CycleTheme => {
                let prev = self.theme_kind;
                self.theme_kind = self.theme_kind.cycle();
                self.log.push(
                    crate::log_panel::LogLevel::Info,
                    format!("theme: {} → {}", prev.as_str(), self.theme_kind.as_str()),
                );
                Command::none()
            }
            Message::Data(msg) => {
                data_mgmt::handle(&mut self.dm_form, &self.dm, &mut self.log, msg);
                Command::none()
            }
            Message::Settings(msg) => match msg {
                SettingsMsg::ThemeChanged(t) => {
                    if t != self.theme_kind {
                        self.theme_kind = t;
                        self.log.push(
                            crate::log_panel::LogLevel::Info,
                            format!("theme → {}", t.as_str()),
                        );
                    }
                    Command::none()
                }
                SettingsMsg::LogLevelChanged(l) => {
                    self.log_level = l.clone();
                    self.log.push(
                        crate::log_panel::LogLevel::Info,
                        format!("log level → {} (also re-run with --log-level={} or RUST_LOG=bee_gui={})", l.as_str(), l.as_str(), l.as_str()),
                    );
                    Command::none()
                }
                SettingsMsg::OpenSpecLink => {
                    self.log.push(
                        crate::log_panel::LogLevel::Info,
                        "spec: docs/superpowers/specs/2026-07-27-s1a-gui-foundation-design.md".to_string(),
                    );
                    Command::none()
                }
                SettingsMsg::ExportLogPressed => {
                    let entries = self.log.snapshot();
                    match crate::log_panel::export_to_file(&entries) {
                        Ok(path) => self.log.push(
                            crate::log_panel::LogLevel::Info,
                            format!("log exported to {}", path.display()),
                        ),
                        Err(e) => self.log.push(
                            crate::log_panel::LogLevel::Error,
                            format!("export failed: {}", e),
                        ),
                    }
                    Command::none()
                }
            },
            Message::PumpTick => {
                let drained = try_drain(&mut self.msg_rx);
                for m in drained {
                    self.apply_connection_msg(m);
                }
                Command::none()
            }
        }
    }

    fn theme(&self) -> Self::Theme {
        match self.theme_kind {
            ThemeKind::Light => Theme::Light,
            ThemeKind::Dark => Theme::Dark,
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Self::Theme, iced::Renderer> {
        let tabs_row = Row::new()
            .push(tab_button(
                "Dashboard",
                Tab::Dashboard,
                icons::GAUGE,
                &self.tab,
                self.theme_kind,
            ))
            .push(tab_button(
                "数据管理",
                Tab::DataMgmt,
                icons::DATABASE,
                &self.tab,
                self.theme_kind,
            ))
            .push(tab_button(
                "Pipelines",
                Tab::Pipelines,
                icons::WORKFLOW,
                &self.tab,
                self.theme_kind,
            ))
            .push(tab_button(
                "设置",
                Tab::Settings,
                icons::SETTINGS,
                &self.tab,
                self.theme_kind,
            ))
            .spacing(theme::SPACE_4)
            .padding([theme::SPACE_2, theme::SPACE_4]);

        let status_bar = Container::new(Text::new(format!(
            "bee-gui v0.1.0  ·  {}  ·  state: {}  ·  theme: {}",
            self.conn.addr(),
            self.conn.state().as_str(),
            self.theme_kind.as_str(),
        )))
        .padding([theme::SPACE_1, theme::SPACE_4]);

        let main: Element<Self::Message, Self::Theme, iced::Renderer> = match self.tab {
            Tab::Dashboard => dashboard::view(&self.dashboard, &self.conn, &self.log)
                .map(Message::Dashboard),
            Tab::DataMgmt => data_mgmt::view(&self.dm_form, &self.dm).map(Message::Data),
            Tab::Pipelines => placeholder::view("Pipelines", "S-3 / S-4", icons::WORKFLOW),
            Tab::Settings => {
                let addr = self.conn.addr();
                let addr_str: &'static str = Box::leak(addr.to_string().into_boxed_str());
                let state_str: &'static str = Box::leak(
                    self.conn.state().as_str().to_string().into_boxed_str(),
                );
                let log_path = crate::log_panel::export_path();
                let log_path_str: &'static str =
                    Box::leak(log_path.display().to_string().into_boxed_str());
                settings::view(
                    self.theme_kind,
                    &self.log_level,
                    addr,
                    state_str,
                    log_path_str,
                )
                .map(Message::Settings)
            }
        };

        let conn_state = self.conn.state();
        let connection_dot = connection_dot_static(&conn_state);

        let theme_button = theme_toggle_button(self.theme_kind);

        let app_bar = Container::new(
            Row::new()
                .push(connection_dot)
                .push(iced::widget::Space::with_width(theme::SPACE_2))
                .push(Text::new(self.conn.addr().to_string()).size(13))
                .push(iced::widget::Space::with_width(theme::SPACE_4))
                .push(tabs_row)
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(theme_button)
                .align_items(iced::alignment::Alignment::Center),
        )
        .padding([theme::SPACE_2, theme::SPACE_4]);

        Container::new(
            Column::new()
                .push(app_bar)
                .push(main)
                .push(status_bar),
        )
        .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }
}

impl App {
    fn apply_connection_msg(&mut self, m: ConnectionMsg) {
        match m {
            ConnectionMsg::StateChanged(s) => {
                self.log.push(
                    crate::log_panel::LogLevel::Info,
                    format!("state -> {:?}", s),
                );
            }
            ConnectionMsg::CallResult { result, .. } => match result {
                Ok(resp) => {
                    self.log.push(
                        crate::log_panel::LogLevel::Info,
                        format!("rpc ok: {:?}", resp),
                    );
                    self.apply_response(resp);
                }
                Err(e) => {
                    self.log.push(crate::log_panel::LogLevel::Error, e.to_string());
                    self.dashboard.last_error = Some(e.to_string());
                }
            },
        }
    }

    fn apply_response(&mut self, resp: bee_control::raft::AdminResponse) {
        use bee_control::raft::AdminResponse;
        match resp {
            AdminResponse::ClusterMetrics(detail) => {
                self.dashboard.cluster = Some(detail);
            }
            AdminResponse::JobList(jobs) => {
                self.dashboard.jobs = jobs;
            }
            _ => {}
        }
    }
}

fn tab_button<'a>(
    label: &'a str,
    tab: Tab,
    icon: &'a [u8],
    current: &'a Tab,
    theme_kind: ThemeKind,
) -> Element<'a, Message, Theme, iced::Renderer> {
    let active = current == &tab;
    let icon_color = if active {
        // accent blue in light, slightly lighter in dark
        match theme_kind {
            ThemeKind::Light => iced::Color::from_rgb(0.0, 0.478, 1.0),
            ThemeKind::Dark => iced::Color::from_rgb(0.32, 0.62, 1.0),
        }
    } else {
        // Tab labels use the theme's primary text color
        match theme_kind {
            ThemeKind::Light => iced::Color::from_rgb(0.04, 0.04, 0.04),
            ThemeKind::Dark => iced::Color::from_rgb(0.961, 0.961, 0.969),
        }
    };
    let row = Row::new()
        .push(icons::render(icon, 20, icon_color))
        .push(iced::widget::Space::with_width(theme::SPACE_1))
        .push(Text::new(label).size(11))
        .spacing(theme::SPACE_1)
        .padding([theme::SPACE_2, theme::SPACE_2])
        .align_items(iced::alignment::Alignment::Center);

    let btn = iced::widget::Button::new(row).on_press(Message::TabSelected(tab));
    btn.into()
}

fn theme_toggle_button(theme_kind: ThemeKind) -> Element<'static, Message, Theme, iced::Renderer> {
    // Show the icon for the OPPOSITE theme (the action that will be taken
    // when clicked). Mouse-over tooltip is OS-native in S-1b; the self-drawn
    // tooltip ships in S-1c.
    let (icon, label) = match theme_kind {
        ThemeKind::Light => (icons::MOON, "Dark"),
        ThemeKind::Dark => (icons::SUN, "Light"),
    };
    let icon_color = match theme_kind {
        ThemeKind::Light => iced::Color::from_rgb(0.04, 0.04, 0.04),
        ThemeKind::Dark => iced::Color::from_rgb(0.961, 0.961, 0.969),
    };
    let row = Row::new()
        .push(icons::render(icon, 16, icon_color))
        .push(iced::widget::Space::with_width(theme::SPACE_1))
        .push(Text::new(label).size(11))
        .align_items(iced::alignment::Alignment::Center);
    iced::widget::Button::new(row)
        .on_press(Message::CycleTheme)
        .padding([theme::SPACE_1, theme::SPACE_2])
        .into()
}

fn connection_dot_static(
    _state: &ConnectionState,
) -> Element<'static, Message, Theme, iced::Renderer> {
    Container::new(Text::new("●"))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .into()
}

