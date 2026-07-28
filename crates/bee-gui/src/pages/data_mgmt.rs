//! S-2: Data Management page (Datasource CRUD).
//!
//! For MVP, datasources live in an in-process `DatasourceRegistry` (mirrors
//! the `bee datasource …` CLI). Production wires the same actions through
//! AdminServer RPC + Raft KV (S30.x follow-up).

use iced::{
    alignment::Horizontal,
    widget::{Button, Column, Container, Row, Text, TextInput},
    Element, Length,
};

use crate::datasource_registry::{DataMgmtState, DatasourceInspectionView};
use crate::icons;
use crate::theme;

#[derive(Debug, Clone)]
pub enum DataMsg {
    NameChanged(String),
    AdapterChanged(String),
    VersionChanged(String),
    ConfigChanged(String),
    TenantChanged(String),
    CreatePressed,
    PausePressed(String),
    ResumePressed(String),
    DeletePressed(String),
    InspectPressed(String),
    CloseInspectPressed,
}

#[derive(Debug, Clone, Default)]
pub struct DataFormState {
    pub name: String,
    pub adapter: String,
    pub version: String,
    pub config: String,
    pub tenant: String,
    pub error: Option<String>,
    pub inspect_target: Option<String>,
}

pub fn view<'a>(
    state: &'a DataFormState,
    dm: &'a DataMgmtState,
) -> Element<'a, DataMsg> {
    let mut col = Column::new().spacing(theme::SPACE_6).padding(theme::SPACE_8);

    col = col.push(header());
    col = col.push(create_form(state));
    if let Some(err) = &state.error {
        col = col.push(
            Container::new(Text::new(format!("error: {}", err)))
                .padding(theme::SPACE_2)
                .style(iced::theme::Container::Box),
        );
    }
    col = col.push(list_table(state, dm));

    if let Some(name) = &state.inspect_target {
        // Build owned strings so the inspect panel doesn't borrow
        // temporaries that get dropped at col.into() time.
        let name_str = name.clone();
        let mut col_local = Column::new().spacing(theme::SPACE_2);
        if let Some(view) = dm.inspect(0, name) {
            col_local = col_local.push(
                Row::new()
                    .push(Text::new(format!("Inspect: {}", name_str)).size(15))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(
                        Button::new(Text::new("Close").size(11))
                            .on_press(DataMsg::CloseInspectPressed)
                            .padding([4, 8]),
                    )
                    .align_items(iced::alignment::Alignment::Center),
            );
            let ds = &view.datasource;
            let health_str = match &view.health {
                Some(h) => format!(
                    "  success_total: {}\n  failure_total: {}\n  referencing_job_count: {}",
                    h.connection_success_total,
                    h.connection_failure_total,
                    h.referencing_job_count
                ),
                None => "  (no health probes recorded)".to_string(),
            };
            col_local = col_local
                .push(Text::new(format!("adapter:           {}", ds.adapter)).size(11))
                .push(Text::new(format!("plugin_id:         {}", ds.plugin_id)).size(11))
                .push(Text::new(format!("version_spec:      {}", ds.version_spec)).size(11))
                .push(Text::new(format!("status:            {:?}", ds.status)).size(11))
                .push(Text::new(format!("tenant:            {}", ds.tenant)).size(11))
                .push(Text::new(format!("config:            {}", ds.config)).size(11))
                .push(Text::new(format!("Health:\n{}", health_str)).size(11));
            col = col.push(
                Container::new(col_local)
                    .padding(theme::SPACE_4)
                    .style(iced::theme::Container::Box),
            );
        } else {
            col = col.push(
                Container::new(Text::new(format!("datasource '{}' not found", name_str)))
                    .padding(theme::SPACE_2),
            );
        }
    }

    col.into()
}

fn header<'a>() -> Element<'a, DataMsg> {
    Row::new()
        .push(Text::new("数据管理").size(20))
        .push(iced::widget::Space::with_width(Length::Fill))
        .push(Text::new("Datasource Registry").size(11))
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

fn create_form<'a>(state: &'a DataFormState) -> Element<'a, DataMsg> {
    Container::new(
        Column::new()
            .spacing(theme::SPACE_2)
            .push(Text::new("Create Datasource").size(15))
            .push(form_field("Name", &state.name, DataMsg::NameChanged))
            .push(form_field("Adapter", &state.adapter, DataMsg::AdapterChanged))
            .push(form_field(
                "Plugin Version (SemVer range)",
                &state.version,
                DataMsg::VersionChanged,
            ))
            .push(form_field(
                "Config (JSON)",
                &state.config,
                DataMsg::ConfigChanged,
            ))
            .push(form_field(
                "Tenant (u16)",
                &state.tenant,
                DataMsg::TenantChanged,
            ))
            .push(
                Row::new()
                    .push(
                        Button::new(
                            Row::new()
                                .push(icons::render(icons::PLUS, 14, iced::Color::BLACK))
                                .push(iced::widget::Space::with_width(theme::SPACE_1))
                                .push(Text::new("Create").size(13)),
                        )
                        .on_press(DataMsg::CreatePressed)
                        .padding([6, 10]),
                    )
                    .align_items(iced::alignment::Alignment::Center),
            ),
    )
    .padding(theme::SPACE_4)
    .style(iced::theme::Container::Box)
    .into()
}

fn form_field<'a>(
    label: &'static str,
    value: &'a str,
    on_change: fn(String) -> DataMsg,
) -> Element<'a, DataMsg> {
    Row::new()
        .push(Text::new(format!("{}: ", label)).size(11).width(Length::Fixed(180.0)))
        .push(
            TextInput::new("", value)
                .on_input(on_change)
                .size(13)
                .width(Length::Fill),
        )
        .spacing(theme::SPACE_2)
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

fn list_table<'a>(state: &'a DataFormState, dm: &'a DataMgmtState) -> Element<'a, DataMsg> {
    let mut col = Column::new().spacing(theme::SPACE_2);
    col = col.push(Text::new("Datasources").size(15));
    let list = dm.list();
    if list.is_empty() {
        col = col.push(Text::new("(no datasources — create one above)").size(11));
    } else {
        for ds in list {
            col = col.push(datasource_row(&ds, state));
        }
    }
    Container::new(col)
        .padding(theme::SPACE_4)
        .style(iced::theme::Container::Box)
        .into()
}

fn datasource_row<'a>(ds: &bee_control::datasource::Datasource, _state: &'a DataFormState) -> Element<'a, DataMsg> {
    let status_color = match ds.status {
        bee_control::datasource::DatasourceStatus::Active => iced::Color::from_rgb(0.204, 0.78, 0.349),
        bee_control::datasource::DatasourceStatus::Paused => iced::Color::from_rgb(1.0, 0.584, 0.0),
        bee_control::datasource::DatasourceStatus::Disabled => iced::Color::from_rgb(0.5, 0.5, 0.5),
        _ => iced::Color::from_rgb(0.0, 0.478, 1.0),
    };
    Row::new()
        .push(Text::new(format!("{}", ds.name)).width(Length::Fixed(120.0)))
        .push(
            Text::new(format!("{:?}", ds.status))
                .width(Length::Fixed(80.0))
                .style(color_to_style(status_color)),
        )
        .push(Text::new(format!("{}", ds.adapter)).width(Length::Fixed(180.0)))
        .push(Text::new(format!("{}", ds.version_spec)).width(Length::Fixed(80.0)))
        .push(iced::widget::Space::with_width(Length::Fill))
        .push(
            Button::new(Text::new("Inspect").size(11))
                .on_press(DataMsg::InspectPressed(ds.name.clone()))
                .padding([4, 8]),
        )
        .push(iced::widget::Space::with_width(theme::SPACE_1))
        .push(
            Button::new(Text::new("Pause").size(11))
                .on_press(DataMsg::PausePressed(ds.name.clone()))
                .padding([4, 8]),
        )
        .push(iced::widget::Space::with_width(theme::SPACE_1))
        .push(
            Button::new(Text::new("Resume").size(11))
                .on_press(DataMsg::ResumePressed(ds.name.clone()))
                .padding([4, 8]),
        )
        .push(iced::widget::Space::with_width(theme::SPACE_1))
        .push(
            Button::new(Text::new("Delete").size(11))
                .on_press(DataMsg::DeletePressed(ds.name.clone()))
                .padding([4, 8]),
        )
        .align_items(iced::alignment::Alignment::Center)
        .into()
}

fn color_to_style(c: iced::Color) -> iced::theme::Text {
    iced::theme::Text::Color(c)
}

pub fn handle(
    state: &mut DataFormState,
    dm: &DataMgmtState,
    log: &mut crate::log_panel::LogRing,
    msg: DataMsg,
) {
    match msg {
        DataMsg::NameChanged(s) => state.name = s,
        DataMsg::AdapterChanged(s) => state.adapter = s,
        DataMsg::VersionChanged(s) => state.version = s,
        DataMsg::ConfigChanged(s) => state.config = s,
        DataMsg::TenantChanged(s) => state.tenant = s,
        DataMsg::CreatePressed => {
            let tenant = state.tenant.parse::<u16>().unwrap_or(0);
            match dm.create(
                state.name.trim().to_string(),
                state.adapter.trim().to_string(),
                state.version.trim().to_string(),
                state.config.trim().to_string(),
                tenant,
            ) {
                Ok(ds) => {
                    log.push(
                        crate::log_panel::LogLevel::Info,
                        format!("datasource created: {}", ds.name),
                    );
                    state.error = None;
                    state.name.clear();
                }
                Err(e) => {
                    log.push(crate::log_panel::LogLevel::Error, e.clone());
                    state.error = Some(e);
                }
            }
        }
        DataMsg::PausePressed(name) => match dm.pause(0, &name) {
            Ok(_) => log.push(
                crate::log_panel::LogLevel::Info,
                format!("paused {}", name),
            ),
            Err(e) => log.push(crate::log_panel::LogLevel::Error, e),
        },
        DataMsg::ResumePressed(name) => match dm.resume(0, &name) {
            Ok(_) => log.push(
                crate::log_panel::LogLevel::Info,
                format!("resumed {}", name),
            ),
            Err(e) => log.push(crate::log_panel::LogLevel::Error, e),
        },
        DataMsg::DeletePressed(name) => match dm.delete(0, &name) {
            Ok(_) => {
                log.push(
                    crate::log_panel::LogLevel::Info,
                    format!("deleted {}", name),
                );
                if state.inspect_target.as_deref() == Some(&name) {
                    state.inspect_target = None;
                }
            }
            Err(e) => log.push(crate::log_panel::LogLevel::Error, e),
        },
        DataMsg::InspectPressed(name) => state.inspect_target = Some(name),
        DataMsg::CloseInspectPressed => state.inspect_target = None,
    }
}