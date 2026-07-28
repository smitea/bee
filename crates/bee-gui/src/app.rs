//! Root App<Message> for iced.
//!
//! - `update(msg)` routes messages to pages
//! - `view()` renders the active tab (Dashboard / 数据管理 / Pipelines / 设置)
//! - `subscription()` consumes ConnectionMsg from the connection thread

use iced::{
    widget::{Column, Container, Row, Text},
    Element, Subscription,
};

use crate::connection::{ConnectionHandle, ConnectionMsg, ConnectionState};
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

pub struct App {
    pub tab: Tab,
    pub conn: ConnectionHandle,
    pub log: LogRing,
    pub dashboard: DashboardData,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Dashboard(DashboardMsg),
}

impl App {
    pub fn new(conn: ConnectionHandle, log: LogRing) -> Self {
        Self {
            tab: Tab::Dashboard,
            conn,
            log,
            dashboard: DashboardData::default(),
        }
    }

    pub fn title(&self) -> String {
        "Bee GUI".to_string()
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::TabSelected(t) => {
                self.tab = t;
            }
            Message::Dashboard(d) => match d {
                DashboardMsg::RefreshPressed => {
                    dashboard::trigger_refresh(&self.conn);
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

    pub fn view(&self) -> Element<'_, Message> {
        let tabs_row = Row::new()
            .push(tab_button("Dashboard", Tab::Dashboard, icons::GAUGE, &self.tab))
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

        let main: Element<Message> = match self.tab {
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

    pub fn subscription(&self) -> Subscription<Message> {
        // S-1a: no continuous subscription; the connection thread's mpsc is
        // drained via Task::perform from main(). S-1b can wire a real
        // Subscription if streaming updates become a requirement.
        Subscription::none()
    }
}

fn tab_button<'a>(
    label: &'a str,
    tab: Tab,
    icon: &'a [u8],
    current: &'a Tab,
) -> Element<'a, Message> {
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

fn connection_dot_static(_state: &ConnectionState) -> Element<'static, Message> {
    Container::new(iced::widget::Text::new("●"))
        .width(iced::Length::Fixed(14.0))
        .height(iced::Length::Fixed(14.0))
        .into()
}