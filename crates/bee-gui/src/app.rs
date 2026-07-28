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
use crate::icons;
use crate::log_panel::LogRing;
use crate::pages::dashboard::{self, DashboardData, DashboardMsg};
use crate::pages::placeholder;
use crate::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    DataMgmt,
    Pipelines,
    Settings,
}

/// Flags passed to `App::new` via `iced::Settings::with_flags`.
pub struct Flags {
    pub bundle: ConnectionBundle,
    pub log: LogRing,
}

pub struct App {
    pub tab: Tab,
    pub conn: ConnectionHandle,
    pub msg_rx: tokio::sync::mpsc::Receiver<ConnectionMsg>,
    pub log: LogRing,
    pub dashboard: DashboardData,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Dashboard(DashboardMsg),
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
            Message::PumpTick => {
                // Drain any pending ConnectionMsg without awaiting; the
                // iced runtime polls `update` again on the next frame, so
                // chaining Command::none here yields control to the runtime.
                let drained = try_drain(&mut self.msg_rx);
                for m in drained {
                    self.apply_connection_msg(m);
                }
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Self::Theme, iced::Renderer> {
        let tabs_row = Row::new()
            .push(tab_button(
                "Dashboard",
                Tab::Dashboard,
                icons::GAUGE,
                &self.tab,
            ))
            .push(tab_button(
                "数据管理",
                Tab::DataMgmt,
                icons::DATABASE,
                &self.tab,
            ))
            .push(tab_button(
                "Pipelines",
                Tab::Pipelines,
                icons::WORKFLOW,
                &self.tab,
            ))
            .push(tab_button(
                "设置",
                Tab::Settings,
                icons::SETTINGS,
                &self.tab,
            ))
            .spacing(theme::SPACE_4)
            .padding([theme::SPACE_2, theme::SPACE_4]);

        let status_bar = Container::new(Text::new(format!(
            "bee-gui v0.1.0  ·  {}  ·  state: {}",
            self.conn.addr(),
            self.conn.state().as_str(),
        )))
        .padding([theme::SPACE_1, theme::SPACE_4]);

        let main: Element<Self::Message, Self::Theme, iced::Renderer> = match self.tab {
            Tab::Dashboard => dashboard::view(&self.dashboard, &self.conn, &self.log)
                .map(Message::Dashboard),
            Tab::DataMgmt => placeholder::view("数据管理", "S-2", icons::DATABASE),
            Tab::Pipelines => placeholder::view("Pipelines", "S-3 / S-4", icons::WORKFLOW),
            Tab::Settings => placeholder::view("设置", "S-5", icons::SETTINGS),
        };

        let conn_state = self.conn.state();
        let connection_dot = connection_dot_static(&conn_state);

        let app_bar = Container::new(
            Row::new()
                .push(connection_dot)
                .push(iced::widget::Space::with_width(theme::SPACE_2))
                .push(Text::new(self.conn.addr().to_string()).size(13))
                .push(iced::widget::Space::with_width(theme::SPACE_4))
                .push(tabs_row)
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
        // The connection thread is drained by the recurring
        // `Command::perform(PumpTick)` cycle kicked off in `new`. S-1b can
        // upgrade this to a real `Subscription` for streaming updates.
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
) -> Element<'a, Message, Theme, iced::Renderer> {
    let active = current == &tab;
    let icon_color = if active {
        iced::Color::from_rgb(0.0, 0.478, 1.0)
    } else {
        iced::Color::BLACK
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

fn connection_dot_static(
    _state: &ConnectionState,
) -> Element<'static, Message, Theme, iced::Renderer> {
    Container::new(Text::new("●"))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .into()
}