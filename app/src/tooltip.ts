/**
 * Tauri-side tooltip helper.
 *
 * Mirrors the S-1c iced self-drawn tooltip widget (see
 * crates/bee-gui/src/app.rs::tooltip_label_for_tab in the S-1a/git history).
 * Kept here so the React frontend can adopt it without re-implementing
 * the `tooltip_label_for_tab` helper in every Tauri page.
 */

export function tooltipLabelForTab(tab: Tab): string {
  switch (tab) {
    case "dashboard":
      return "Live cluster + job status (S-1a, Tauri rewrite)";
    case "dataSources":
      return "Datasource CRUD (S-2)";
    case "pipelines":
      return "Job list + inspect (S-3/4)";
    case "settings":
      return "Theme, log level, diagnostics (S-5)";
  }
}

export type Tab = "dashboard" | "dataSources" | "pipelines" | "settings";