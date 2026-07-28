//! Dashboard Minimal page (S-1a spec §6.2):
//!   - 3 stat cards (Cluster / Jobs / Tasks)
//!   - Nodes table
//!   - Recent Jobs table
//!   - Refresh button (top-right)

use bee_control::raft::{ClusterMetricsDetail, JobSummary};
use iced::{
    widget::{Button, Column, Container, Row, Text},
    Element, Length,
};

use crate::connection::ConnectionHandle;
use crate::icons;
use crate::log_panel::LogRing;
use crate::theme;

#[derive(Debug, Clone)]
pub enum DashboardMsg {
    RefreshPressed,
}

#[derive(Debug, Default, Clone)]
pub struct DashboardData {
    pub cluster: Option<ClusterMetricsDetail>,
    pub jobs: Vec<JobSummary>,
    pub last_error: Option<String>,
}

pub fn view<'a>(
    data: &'a DashboardData,
    _conn: &'a ConnectionHandle,
    _log: &'a LogRing,
) -> Element<'a, DashboardMsg> {
    let cluster_s = cluster_summary(data);
    let jobs_s = jobs_summary(data);
    let tasks_s = tasks_summary(data);

    let mut col = Column::new().spacing(theme::SPACE_6).padding(theme::SPACE_8);

    // Header row: title + refresh button
    col = col.push(
        Row::new()
            .push(Text::new("Dashboard").size(20))
            .push(iced::widget::Space::with_width(Length::Fill))
            .push(
                Button::new(
                    Row::new()
                        .push(icons::render(icons::REFRESH_CW, 18, iced::Color::BLACK))
                        .push(Text::new("Refresh").size(13)),
                )
                .on_press(DashboardMsg::RefreshPressed)
                .padding([6, 10]),
            )
            .align_items(iced::alignment::Alignment::Center),
    );

    // Stat cards row
    col = col.push(
        Row::new()
            .push(stat_card("Cluster", cluster_s, icons::NETWORK))
            .push(iced::widget::Space::with_width(theme::SPACE_4))
            .push(stat_card("Jobs", jobs_s, icons::WORKFLOW))
            .push(iced::widget::Space::with_width(theme::SPACE_4))
            .push(stat_card("Tasks", tasks_s, icons::ACTIVITY)),
    );

    // Nodes table
    col = col.push(view_nodes_table(data));

    // Recent Jobs table
    col = col.push(view_jobs_table(data));

    // Connection / error banner
    if let Some(err) = &data.last_error {
        col = col.push(
            Container::new(Text::new(format!("RPC 失败: {}", err)))
                .padding(theme::SPACE_3)
                .style(iced::theme::Container::Box),
        );
    }

    col.into()
}

fn stat_card<'a>(title: &'a str, body: String, icon_bytes: &'a [u8]) -> Element<'a, DashboardMsg> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(
                Row::new()
                    .push(icons::render(icon_bytes, 24, iced::Color::BLACK))
                    .push(iced::widget::Space::with_width(theme::SPACE_2))
                    .push(Text::new(title).size(15)),
            )
            .push(Text::new(body).size(28)),
    )
    .padding(theme::SPACE_4)
    .width(Length::Fixed(240.0))
    .height(Length::Fixed(120.0))
    .style(iced::theme::Container::Box)
    .into()
}

fn cluster_summary(d: &DashboardData) -> String {
    match &d.cluster {
        Some(c) => format!(
            "{} nodes\nleader: {}\nterm {}\ncommit {}",
            c.nodes.len(),
            c.leader_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "—".to_string()),
            c.term,
            c.commit_index
        ),
        None => "—".to_string(),
    }
}

fn jobs_summary(d: &DashboardData) -> String {
    format!("{} total", d.jobs.len())
}

fn tasks_summary(d: &DashboardData) -> String {
    let total: usize = d.jobs.iter().map(|j| j.task_count).sum();
    format!("{} total", total)
}

fn view_nodes_table(d: &DashboardData) -> Element<'_, DashboardMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(Text::new("Nodes").size(15));
    match &d.cluster {
        Some(c) => {
            for n in &c.nodes {
                col = col.push(
                    Row::new()
                        .push(Text::new(format!("{}", n.id)).width(Length::Fixed(40.0)))
                        .push(Text::new(format!("{}", n.role)).width(Length::Fixed(100.0)))
                        .push(Text::new(format!("{}", n.commit_index)).width(Length::Fixed(80.0)))
                        .push(Text::new(format!("{}", n.log_length))),
                );
            }
        }
        None => {
            col = col.push(Text::new("(no data — click Refresh)").size(11));
        }
    }
    Container::new(col)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

fn view_jobs_table(d: &DashboardData) -> Element<'_, DashboardMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(Text::new("Recent Jobs").size(15));
    if d.jobs.is_empty() {
        col = col.push(Text::new("(no jobs)").size(11));
    } else {
        for j in &d.jobs {
            col = col.push(
                Row::new()
                    .push(Text::new(format!("#{}", j.job_id)).width(Length::Fixed(60.0)))
                    .push(Text::new(format!("{:?}", j.lifecycle)).width(Length::Fixed(120.0)))
                    .push(Text::new(format!("{} tasks", j.task_count))),
            );
        }
    }
    Container::new(col)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

pub fn trigger_refresh(conn: &ConnectionHandle) {
    use bee_control::raft::AdminRequest;
    let _ = conn.call(AdminRequest::ClusterStatus);
    let _ = conn.call(AdminRequest::ListJobs);
}