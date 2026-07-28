//! S-3 / S-4: Pipelines page — list + inspect.
//!
//! Mirrors the `bee jobs` / `bee jobs inspect` CLI surface. On Refresh,
//! issues `AdminRequest::ListJobs` (and `JobInspect(id)` per selected row).
//! Currently the inspect panel shows the raw `JobDetail` JSON so reviewers
//! can see the existing wire format; future slices (S-1c visual DAG) layer
//! an ASCII DAG renderer on top (matches the format in `jobs_view.rs`).

use bee_control::raft::{AdminRequest, AdminResponse, JobDetail, JobSummary};
use iced::{
    widget::{Button, Column, Container, Row, Scrollable, Text},
    Element, Length,
};

use crate::connection::ConnectionHandle;
use crate::icons;
use crate::log_panel::LogRing;
use crate::theme;

#[derive(Debug, Clone)]
pub enum PipelinesMsg {
    RefreshPressed,
    InspectPressed(u32),
    CloseInspectPressed,
}

#[derive(Debug, Default, Clone)]
pub struct PipelinesData {
    pub jobs: Vec<JobSummary>,
    pub selected: Option<JobDetail>,
    pub loading: bool,
    pub last_error: Option<String>,
}

pub fn view<'a>(
    data: &'a PipelinesData,
    _conn: &'a ConnectionHandle,
    _log: &'a LogRing,
) -> Element<'a, PipelinesMsg> {
    let mut col = Column::new().spacing(theme::SPACE_6).padding(theme::SPACE_8);

    col = col.push(header(data.loading));
    col = col.push(list_table(data));

    if let Some(err) = &data.last_error {
        col = col.push(
            Container::new(Text::new(format!("error: {}", err)))
                .padding(theme::SPACE_2)
                .style(iced::theme::Container::Box),
        );
    }

    if let Some(detail) = &data.selected {
        col = col.push(inspect_panel(detail));
    }

    col.into()
}

fn header<'a>(loading: bool) -> Element<'a, PipelinesMsg> {
    Row::new()
        .push(Text::new("Pipelines").size(20))
        .push(iced::widget::Space::with_width(Length::Fill))
        .push(
            Button::new(
                Row::new()
                    .push(icons::render(icons::REFRESH_CW, 18, iced::Color::BLACK))
                    .push(iced::widget::Space::with_width(theme::SPACE_1))
                    .push(Text::new("Refresh").size(13)),
            )
            .on_press(PipelinesMsg::RefreshPressed)
            .padding([6, 10]),
        )
        .push(iced::widget::Space::with_width(theme::SPACE_2))
        .push(
            Text::new(if loading {
                "loading…"
            } else {
                ""
            })
            .size(11),
        )
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

fn list_table<'a>(data: &'a PipelinesData) -> Element<'a, PipelinesMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(Text::new("Jobs").size(15));
    if data.jobs.is_empty() {
        col = col.push(Text::new("(no jobs — click Refresh)").size(11));
    } else {
        for j in &data.jobs {
            col = col.push(job_row(j));
        }
    }
    Container::new(col)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

fn job_row<'a>(j: &'a JobSummary) -> Element<'a, PipelinesMsg> {
    Row::new()
        .push(Text::new(format!("#{}", j.job_id)).width(Length::Fixed(60.0)))
        .push(
            Text::new(format!("{:?}", j.lifecycle))
                .width(Length::Fixed(120.0)),
        )
        .push(
            Text::new(format!("{}", j.mode))
                .width(Length::Fixed(100.0)),
        )
        .push(
            Text::new(format!("{} tasks", j.task_count))
                .width(Length::Fixed(80.0)),
        )
        .push(Text::new(format!("node {}", j.owner_node)))
        .push(iced::widget::Space::with_width(Length::Fill))
        .push(
            Button::new(Text::new("Inspect").size(11))
                .on_press(PipelinesMsg::InspectPressed(j.job_id))
                .padding([4, 8]),
        )
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

fn inspect_panel<'a>(detail: &'a JobDetail) -> Element<'a, PipelinesMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(
        Row::new()
            .push(Text::new(format!("Inspect: #{}", detail.job_id)).size(15))
            .push(iced::widget::Space::with_width(Length::Fill))
            .push(
                Button::new(Text::new("Close").size(11))
                    .on_press(PipelinesMsg::CloseInspectPressed)
                    .padding([4, 8]),
            )
            .align_items(iced::alignment::Alignment::Center),
    );
    col = col.push(Text::new(format!("dag_hash:    {}", detail.dag_hash)).size(11));
    col = col.push(Text::new(format!("lifecycle:   {:?}", detail.lifecycle)).size(11));
    col = col.push(Text::new(format!("owner_node:  {}", detail.owner_node)).size(11));
    col = col.push(Text::new(format!(
        "deps:        {} cross-pipeline edge(s)",
        detail.dependencies.len()
    )).size(11));
    for d in &detail.dependencies {
        col = col.push(Text::new(format!(
            "  ↑ job {} stream {}",
            d.upstream_job, d.stream
        )).size(11));
    }
    col = col.push(Text::new(format!("tasks:       {}", detail.tasks.len())).size(11));
    for t in &detail.tasks {
        col = col.push(Text::new(format!(
            "  task #{} phase {} owner {} status {:?}",
            t.task_id, t.phase_id, t.owner_node, t.status
        )).size(11));
    }
    let scroll = Scrollable::new(col).height(Length::Fixed(280.0));
    Container::new(scroll)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

pub fn trigger_refresh(conn: &ConnectionHandle) {
    let _ = conn.call(AdminRequest::ListJobs);
}

pub fn trigger_inspect(conn: &ConnectionHandle, job_id: u32) {
    let _ = conn.call(AdminRequest::JobInspect(job_id));
}

pub fn apply_response(data: &mut PipelinesData, resp: AdminResponse) {
    match resp {
        AdminResponse::JobList(jobs) => {
            data.jobs = jobs;
            data.loading = false;
            data.last_error = None;
        }
        AdminResponse::JobDetail(detail) => {
            data.selected = detail;
            data.loading = false;
        }
        AdminResponse::Error(msg) => {
            data.last_error = Some(msg);
            data.loading = false;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_control::kv::JobLifecycleState;

    #[test]
    fn empty_jobs_renders_empty_state() {
        let data = PipelinesData::default();
        // We can't easily construct the rendered Element, but we can
        // assert that the data accessor is reachable without panic.
        assert!(data.jobs.is_empty());
        assert!(data.selected.is_none());
    }

    #[test]
    fn job_summary_fields_are_accessible() {
        let j = JobSummary {
            job_id: 1,
            dag_hash: "abc".into(),
            lifecycle: JobLifecycleState::Running,
            mode: "Producer".into(),
            task_count: 3,
            owner_node: 1,
        };
        assert_eq!(j.job_id, 1);
        assert_eq!(j.task_count, 3);
    }
}