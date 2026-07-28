//! Lucide icons compiled into the binary via `include_bytes!`.
//! 29 selected icons per S-1a spec §5.4 (ISC license, MIT-compatible).
//!
//! Sizes: 16/20/24/32 px (see `render`).

use iced::widget::svg::{Handle, Svg};
use iced::{Color, Length};

pub const GAUGE: &[u8] = include_bytes!("../icons/gauge.svg");
pub const DATABASE: &[u8] = include_bytes!("../icons/database.svg");
pub const WORKFLOW: &[u8] = include_bytes!("../icons/workflow.svg");
pub const SETTINGS: &[u8] = include_bytes!("../icons/settings.svg");
pub const NETWORK: &[u8] = include_bytes!("../icons/network.svg");
pub const CROWN: &[u8] = include_bytes!("../icons/crown.svg");
pub const ACTIVITY: &[u8] = include_bytes!("../icons/activity.svg");
pub const CHECK_CIRCLE: &[u8] = include_bytes!("../icons/check-circle.svg");
pub const ALERT_TRIANGLE: &[u8] = include_bytes!("../icons/alert-triangle.svg");
pub const REFRESH_CW: &[u8] = include_bytes!("../icons/refresh-cw.svg");
pub const SEARCH: &[u8] = include_bytes!("../icons/search.svg");
pub const X: &[u8] = include_bytes!("../icons/x.svg");
pub const CHECK: &[u8] = include_bytes!("../icons/check.svg");
pub const CHEVRON_RIGHT: &[u8] = include_bytes!("../icons/chevron-right.svg");
pub const INFO: &[u8] = include_bytes!("../icons/info.svg");
pub const CIRCLE_DOT: &[u8] = include_bytes!("../icons/circle-dot.svg");
pub const PLUS: &[u8] = include_bytes!("../icons/plus.svg");
pub const TRASH_2: &[u8] = include_bytes!("../icons/trash-2.svg");
pub const PLAY: &[u8] = include_bytes!("../icons/play.svg");
pub const PAUSE: &[u8] = include_bytes!("../icons/pause.svg");
pub const STOP_CIRCLE: &[u8] = include_bytes!("../icons/stop-circle.svg");
pub const LOADER: &[u8] = include_bytes!("../icons/loader.svg");
pub const DOWNLOAD: &[u8] = include_bytes!("../icons/download.svg");
pub const UPLOAD: &[u8] = include_bytes!("../icons/upload.svg");
pub const UNPLUG: &[u8] = include_bytes!("../icons/unplug.svg");
pub const TERMINAL: &[u8] = include_bytes!("../icons/terminal.svg");
pub const HISTORY: &[u8] = include_bytes!("../icons/history.svg");
pub const BAR_CHART_3: &[u8] = include_bytes!("../icons/bar-chart-3.svg");
pub const COPY: &[u8] = include_bytes!("../icons/copy.svg");
pub const SUN: &[u8] = include_bytes!("../icons/sun.svg");
pub const MOON: &[u8] = include_bytes!("../icons/moon.svg");

/// Render an icon SVG at the given pixel size.
///
/// The `color` is reserved for S-1b (the theme switch UI); S-1a uses the
/// default Lucide `currentColor` stroke via the `StyleSheet` default. To
/// style the icon now, callers can wrap with a Button or apply a
/// container style that sets a tinted text color.
pub fn render(bytes: &[u8], size: u16, _color: Color) -> Svg {
    let handle = Handle::from_memory(bytes.to_vec());
    Svg::new(handle)
        .width(Length::Fixed(size as f32))
        .height(Length::Fixed(size as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[(&str, &[u8])] = &[
        ("gauge", GAUGE),
        ("database", DATABASE),
        ("workflow", WORKFLOW),
        ("settings", SETTINGS),
        ("network", NETWORK),
        ("crown", CROWN),
        ("activity", ACTIVITY),
        ("check-circle", CHECK_CIRCLE),
        ("alert-triangle", ALERT_TRIANGLE),
        ("refresh-cw", REFRESH_CW),
        ("search", SEARCH),
        ("x", X),
        ("check", CHECK),
        ("chevron-right", CHEVRON_RIGHT),
        ("info", INFO),
        ("circle-dot", CIRCLE_DOT),
        ("plus", PLUS),
        ("trash-2", TRASH_2),
        ("play", PLAY),
        ("pause", PAUSE),
        ("stop-circle", STOP_CIRCLE),
        ("loader", LOADER),
        ("download", DOWNLOAD),
        ("upload", UPLOAD),
        ("unplug", UNPLUG),
        ("terminal", TERMINAL),
        ("history", HISTORY),
        ("bar-chart-3", BAR_CHART_3),
        ("copy", COPY),
        ("sun", SUN),
        ("moon", MOON),
    ];

    #[test]
    fn lucide_icon_loads() {
        assert_eq!(ALL.len(), 31);
        for (name, bytes) in ALL {
            assert!(!bytes.is_empty(), "icon {} is 0 bytes", name);
            let s = std::str::from_utf8(bytes).expect("UTF-8");
            assert!(s.contains("<svg"), "icon {} missing <svg> tag", name);
            assert!(
                s.contains("lucide-static"),
                "icon {} missing lucide-static license header",
                name
            );
        }
    }
}