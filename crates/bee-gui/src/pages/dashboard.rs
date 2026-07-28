//! Dashboard page (S-1a spec §6.2 + UI polish 2026-07-28):
//!   - 3 stat cards (Cluster / Jobs / Tasks) with colored status dots
//!   - Nodes table with role-colored dots + column headers
//!   - Recent Jobs table with lifecycle-colored dots
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

pub fn view(
    data: &DashboardData,
    _conn: &ConnectionHandle,
    _log: &LogRing,
) -> Element<'static, DashboardMsg> {
    let cluster = cluster_summary(data);
    let jobs = jobs_summary(data);
    let tasks = tasks_summary(data);

    let mut col = Column::new().spacing(theme::SPACE_6).padding([theme::SPACE_8, theme::SPACE_8]);

    // Header row: title + refresh button
    col = col.push(
        Row::new()
            .push(Text::new("Dashboard").size(20))
            .push(iced::widget::Space::with_width(Length::Fill))
            .push(refresh_button(data.last_error.is_some()))
            .align_items(iced::alignment::Alignment::Center),
    );

    // Stat cards row (3 columns)
    col = col.push(
        Row::new()
            .push(stat_card(
                "Cluster",
                cluster.0,
                cluster.1,
                icons::NETWORK,
            ))
            .push(iced::widget::Space::with_width(theme::SPACE_4))
            .push(stat_card("Jobs", jobs.0, jobs.1, icons::WORKFLOW))
            .push(iced::widget::Space::with_width(theme::SPACE_4))
            .push(stat_card("Tasks", tasks.0, tasks.1, icons::ACTIVITY)),
    );

    // Nodes table
    col = col.push(view_nodes_table(data));

    // Recent Jobs table
    col = col.push(view_jobs_table(data));

    // Connection / error banner
    if let Some(err) = &data.last_error {
        col = col.push(
            Container::new(
                Row::new()
                    .push(icons::render(icons::ALERT_TRIANGLE, 14, iced::Color::from_rgb(1.0, 0.231, 0.188)))
                    .push(iced::widget::Space::with_width(theme::SPACE_2))
                    .push(Text::new(format!("RPC error: {}", err)).size(12))
                    .align_items(iced::alignment::Alignment::Center),
            )
            .padding(theme::SPACE_3)
            .style(iced::theme::Container::Box),
        );
    }

    col.into()
}

fn refresh_button(has_error: bool) -> Element<'static, DashboardMsg> {
    let color = if has_error {
        iced::Color::from_rgb(1.0, 0.231, 0.188)
    } else {
        iced::Color::BLACK
    };
    Button::new(
        Row::new()
            .push(icons::render(icons::REFRESH_CW, 14, color))
            .push(iced::widget::Space::with_width(theme::SPACE_1))
            .push(Text::new("Refresh").size(12))
            .align_items(iced::alignment::Alignment::Center),
    )
    .on_press(DashboardMsg::RefreshPressed)
    .padding([theme::SPACE_1, theme::SPACE_2])
    .into()
}

/// Stat card with: icon, title, big metric, sub-line. `accent_color` dots
/// the metric line per spec §5.5 (status colors reserved for accent).
fn stat_card(
    title: &'static str,
    big: String,
    sub: String,
    icon_bytes: &'static [u8],
) -> Element<'static, DashboardMsg> {
    let icon_color = iced::Color::from_rgb(0.32, 0.62, 1.0);
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(
                Row::new()
                    .push(icons::render(icon_bytes, 18, icon_color))
                    .push(iced::widget::Space::with_width(theme::SPACE_2))
                    .push(Text::new(title).size(12))
                    .align_items(iced::alignment::Alignment::Center),
            )
            .push(Text::new(big).size(26))
            .push(
                Text::new(sub)
                    .size(10)
                    .style(iced::theme::Text::Color(iced::Color::from_rgb(0.42, 0.42, 0.42))),
            ),
    )
    .padding(theme::SPACE_4)
    .width(Length::Fixed(240.0))
    .height(Length::Fixed(120.0))
    .style(iced::theme::Container::Box)
    .into()
}

/// Returns (big metric, sub-line).
fn cluster_summary(d: &DashboardData) -> (String, String) {
    match &d.cluster {
        Some(c) => (
            c.nodes.len().to_string(),
            format!(
                "leader {} · term {} · commit {}",
                c.leader_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "—".to_string()),
                c.term,
                c.commit_index
            ),
        ),
        None => ("—".to_string(), "click Refresh".to_string()),
    }
}

fn jobs_summary(d: &DashboardData) -> (String, String) {
    let total = d.jobs.len();
    let running = d
        .jobs
        .iter()
        .filter(|j| matches!(j.lifecycle, bee_control::kv::JobLifecycleState::Running))
        .count();
    (total.to_string(), format!("{} running", running))
}

fn tasks_summary(d: &DashboardData) -> (String, String) {
    let total: usize = d.jobs.iter().map(|j| j.task_count).sum();
    let completed = d
        .jobs
        .iter()
        .filter(|j| matches!(j.lifecycle, bee_control::kv::JobLifecycleState::Completed))
        .count();
    (total.to_string(), format!("{} completed", completed))
}

fn view_nodes_table(d: &DashboardData) -> Element<'static, DashboardMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(table_header(&["ID", "Role", "Commit", "Log length"]));
    match &d.cluster {
        Some(c) => {
            for n in &c.nodes {
                col = col.push(node_row(n));
            }
        }
        None => {
            col = col.push(
                Text::new("(no data — click Refresh)")
                    .size(11)
                    .style(iced::theme::Text::Color(iced::Color::from_rgb(0.5, 0.5, 0.5))),
            );
        }
    }
    Container::new(col)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

fn table_header(labels: &[&'static str]) -> Element<'static, DashboardMsg> {
    let widths = [60.0, 120.0, 80.0, 120.0];
    let mut row = Row::new().spacing(theme::SPACE_4);
    for (i, l) in labels.iter().enumerate() {
        row = row.push(
            Text::new(*l)
                .size(10)
                .width(Length::Fixed(widths.get(i).copied().unwrap_or(80.0)))
                .style(iced::theme::Text::Color(iced::Color::from_rgb(0.42, 0.42, 0.42))),
        );
    }
    row.into()
}

fn role_color(role: &str) -> iced::Color {
    match role {
        "Leader" => iced::Color::from_rgb(0.0, 0.478, 1.0),
        "Follower" => iced::Color::from_rgb(0.42, 0.42, 0.42),
        "Candidate" => iced::Color::from_rgb(1.0, 0.584, 0.0),
        _ => iced::Color::BLACK,
    }
}

fn node_row(n: &bee_control::raft::NodeMetricsSummary) -> Element<'static, DashboardMsg> {
    let dot_color = role_color(&n.role);
    Row::new()
        .push(
            Text::new(format!("● node #{}", n.id))
                .size(11)
                .width(Length::Fixed(60.0))
                .style(iced::theme::Text::Color(dot_color)),
        )
        .push(
            Text::new(n.role.clone())
                .size(11)
                .width(Length::Fixed(120.0)),
        )
        .push(
            Text::new(format!("{}", n.commit_index))
                .size(11)
                .width(Length::Fixed(80.0)),
        )
        .push(
            Text::new(format!("{}", n.log_length))
                .size(11)
                .width(Length::Fixed(120.0)),
        )
        .into()
}

fn lifecycle_color(state: &bee_control::kv::JobLifecycleState) -> iced::Color {
    use bee_control::kv::JobLifecycleState;
    match state {
        JobLifecycleState::Pending => iced::Color::from_rgb(0.5, 0.5, 0.5),
        JobLifecycleState::Scheduled => iced::Color::from_rgb(0.65, 0.65, 0.68),
        JobLifecycleState::WaitingForUpstream => iced::Color::from_rgb(1.0, 0.584, 0.0),
        JobLifecycleState::Running => iced::Color::from_rgb(0.204, 0.78, 0.349),
        JobLifecycleState::Completed => iced::Color::from_rgb(0.32, 0.62, 1.0),
        JobLifecycleState::Failed => iced::Color::from_rgb(1.0, 0.231, 0.188),
    }
}

fn view_jobs_table(d: &DashboardData) -> Element<'static, DashboardMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(table_header(&["Job", "Lifecycle", "Mode", "Tasks", "Node"]));
    if d.jobs.is_empty() {
        col = col.push(
            Text::new("(no jobs)")
                .size(11)
                .style(iced::theme::Text::Color(iced::Color::from_rgb(0.5, 0.5, 0.5))),
        );
    } else {
        for j in &d.jobs {
            col = col.push(job_row(j));
        }
    }
    Container::new(col)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

fn job_row(j: &JobSummary) -> Element<'static, DashboardMsg> {
    let dot_color = lifecycle_color(&j.lifecycle);
    Row::new()
        .push(
            Text::new(format!("● #{}", j.job_id))
                .size(11)
                .width(Length::Fixed(60.0))
                .style(iced::theme::Text::Color(dot_color)),
        )
        .push(
            Text::new(format!("{:?}", j.lifecycle))
                .size(11)
                .width(Length::Fixed(120.0)),
        )
        .push(
            Text::new(j.mode.clone())
                .size(11)
                .width(Length::Fixed(80.0)),
        )
        .push(
            Text::new(format!("{}", j.task_count))
                .size(11)
                .width(Length::Fixed(80.0)),
        )
        .push(Text::new(format!("#{}", j.owner_node)).size(11))
        .into()
}

pub fn trigger_refresh(conn: &ConnectionHandle) {
    use bee_control::raft::AdminRequest;
    let _ = conn.call(AdminRequest::ClusterStatus);
    let _ = conn.call(AdminRequest::ListJobs);
}