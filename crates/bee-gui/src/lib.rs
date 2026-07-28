//! bee-gui: iced-based desktop client for Bee clusters.
//!
//! Module structure:
//! - `app`        — root `App<Message>` + update/view/subscription
//! - `connection` — single AdminClient lifecycle + tokio bridge
//! - `error`      — `GuiError` enum + chain logging
//! - `icons`      — 30 Lucide SVG constants + render helper
//! - `log_panel`  — ring buffer + export
//! - `pages`      — Dashboard + placeholder
//! - `theme`      — design tokens + light/dark builders

pub mod app;
pub mod connection;
pub mod error;
pub mod icons;
pub mod log_panel;
pub mod pages;
pub mod theme;