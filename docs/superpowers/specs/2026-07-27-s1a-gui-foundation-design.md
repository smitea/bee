# S-1a · Bee GUI Foundation (Minimal)

**Date:** 2026-07-27
**Type:** AFK
**Blocked by:** none
**ADRs:** none (this is a new frontend subsystem)
**Status:** Draft (pending review)

## Why this story

`product-design.md:338` explicitly defers UI to 1.x, but the user wants to observe Bee usage and runtime from a user perspective **now**. The backend already exposes a complete, stable Admin RPC surface (`AdminRequest` / `AdminResponse` over TCP + bincode, 10 request variants covering read + write). All data needed for an MVP observation UI is already on the wire.

This story creates a standalone Rust GUI client (iced) that connects directly to a Bee cluster's `AdminServer` via the existing `AdminClient`. S-1a is the **Minimal Foundation** slice: prove end-to-end (iced GUI → AdminClient → in-process 3-node Cluster → see live data), establish the design system, and ship installable binaries. Subsequent stories (S-1b, S-1c, S-2 … S-5) layer on top without re-architecting.

This is the first GUI in the project (zero HTML / JS / CSS / webview assets exist outside `.superpowers/` tooling).

## What already exists at HEAD

- `AdminServer` (TCP + bincode) on every Bee node, accepts 10 `AdminRequest` variants — `crates/bee-control/src/raft/admin_server.rs:61-142`
- `AdminClient::connect(addr)` / `AdminClient::call(req)` (persistent TCP, multi-call) — `crates/bee-control/src/raft/admin_client.rs:34-70`
- Wire format: 15-byte BRP frame + bincode body, `MessageType::Admin = 0x04` — `crates/bee-codec/src/lib.rs:8-9, 23, 71-97`
- Forwarder path: follower → leader via `RpcMessage::AdminForward` (5s timeout) — `crates/bee-control/src/raft/admin_server.rs:874-998`
- Read APIs needed by S-1a Dashboard (all already exist):
  - `AdminRequest::Ping` → `AdminResponse::Pong`
  - `AdminRequest::ClusterStatus` → `AdminResponse::ClusterMetrics(ClusterMetricsDetail)`
  - `AdminRequest::ListJobs` → `AdminResponse::JobList(Vec<JobSummary>)`
- ClusterMetricsDetail has `nodes: Vec<NodeMetricsSummary>` (id, role, term, commit_index, log_length) — covers Dashboard's Nodes table without a new RPC.
- JobSummary has (job_id, dag_hash, lifecycle, mode, task_count, owner_node) — covers Recent Jobs table.
- Zero frontend code in the repo (`.opencode/`, `docs/book/` are unrelated tooling/docs).
- 5 unused mock plugin entries in `bee plugin list` — S-2 replaces this.

## Scope

### In scope

1. **New Cargo workspace member `crates/bee-gui/`** — independent crate, no `bee` CLI integration, no shared lib.
2. **iced 0.12 application** with single-window, top-tab navigation (Dashboard / 数据管理 / Pipelines / 设置).
3. **Design system** — Apple-minimal tokens (light + dark themes, 4px spacing base, 6-10px radii, SF/Segoe/Inter fonts, accent palette reserved but unused in S-1a).
4. **Lucide Icons** — 30 selected SVG icons, compiled in via `include_bytes!`, rendered through iced's `Svg` widget with theme-aware fill.
5. **Single AdminClient connection** with lifecycle state machine (`Disconnected` / `Connecting` / `Connected` / `Error(reason)`), 5s ping × 3 failure threshold, auto-reconnect.
6. **tokio bridge** — `std::thread::spawn`'d runtime for AdminClient I/O; `iced::Subscription` consumes results via `tokio::sync::mpsc`.
7. **Dashboard Minimal page** — 3 stat cards (Cluster / Jobs / Tasks) + Nodes table + Recent Jobs table + manual Refresh button.
8. **Detailed error logging** — `GuiError` enum (7 kinds), `tracing::error!` chain logging on every RPC failure path, `LogPanel` ring-buffer (1000 entries, time-stamped, copy-to-clipboard, export to `~/Library/Logs/bee-gui/`). Server-side `AdminServer` error paths get matching `tracing::error!` calls (no protocol change).
9. **Placeholder pages** for the 3 non-Dashboard tabs (data management / pipelines / settings) — clearly labeled "Coming in S-2/S-3/S-5".
10. **Install scripts** — Homebrew formula (macOS), `cargo-deb` metadata (Debian/Ubuntu), `scripts/build-release.sh` for cross-platform tarballs.
11. **Unit + integration tests** in `crates/bee-gui/tests/` (see §6).

### Out of scope (deferred)

- **Multi-connection management / cluster switcher** → S-1b
- **Rich Dashboard (live event stream, metrics charts, multi-cluster comparison)** → S-1c
- **Tooltip self-drawn** → S-1b
- **Theme switch UI (toggle button in top bar)** → S-1b
- **System-package distribution for Windows (MSI)** → post-S-1a follow-up if requested
- **All data management / pipelines / settings functionality** → S-2, S-3, S-4, S-5
- **`bee --gui` subcommand integration** → not planned (the GUI is a separate `bee-gui` binary)
- **HTTP/REST proxy layer** → not needed (GUI connects directly to bincode AdminServer)
- **Tauri / web frontend / webview** → explicitly excluded (per user direction)

## File structure

| File | Action |
|---|---|
| `crates/bee-gui/Cargo.toml` | new |
| `crates/bee-gui/README.md` | new |
| `crates/bee-gui/src/main.rs` | new (entry: `iced::Application::run`, CLI args, tokio runtime spawn) |
| `crates/bee-gui/src/app.rs` | new (root `App<Message>` + `update` / `view` / `subscription`) |
| `crates/bee-gui/src/theme.rs` | new (design tokens + light/dark theme builders) |
| `crates/bee-gui/src/icons.rs` | new (30 Lucide SVG constants + render helper) |
| `crates/bee-gui/src/connection.rs` | new (single `AdminClient`, state machine, request ID, mpsc bridge) |
| `crates/bee-gui/src/error.rs` | new (`GuiError` enum + chain-logging) |
| `crates/bee-gui/src/log_panel.rs` | new (ring buffer + `LogPanel` widget + export) |
| `crates/bee-gui/src/pages/mod.rs` | new |
| `crates/bee-gui/src/pages/dashboard.rs` | new |
| `crates/bee-gui/src/pages/placeholder.rs` | new (3 generic placeholders) |
| `crates/bee-gui/icons/*.svg` | new (30 Lucide SVG files) |
| `crates/bee-gui/packaging/homebrew/bee-gui.rb` | new |
| `crates/bee-gui/Cargo.toml` | modified (add `[package.metadata.deb]` inline for `cargo-deb`) |
| `crates/bee-gui/tests/connection_smoke.rs` | new |
| `crates/bee-gui/tests/refresh_updates_data.rs` | new |
| `crates/bee-gui/tests/connection_lost_recovery.rs` | new |
| `crates/bee-gui/tests/error_log_panel.rs` | new |
| `crates/bee-control/src/raft/admin_server.rs` | modified (add `tracing::error!` at each `AdminResponse::Error` construction; no wire change) |
| `scripts/build-release.sh` | new |
| `docs/testing/s-1a-visual-checklist.md` | new (manual visual regression checklist) |
| `Cargo.toml` (workspace root) | modified (add `crates/bee-gui` to members) |

## Architecture

### 1. Crate layout & dependencies

```
crates/bee-gui/
├── Cargo.toml            # crate-type = ["bin"], depends on iced 0.12
├── README.md
├── src/                  # see file structure table
├── icons/                # 30 SVG files
├── packaging/            # brew + deb
└── tests/                # integration tests
```

**Key dependencies** (`crates/bee-gui/Cargo.toml`):

| Crate | Version | Purpose |
|---|---|---|
| `iced` | 0.12 | GUI framework |
| `tokio` | 1 (full) | async runtime for AdminClient |
| `bee-control` | path = ../bee-control | `AdminClient`, `AdminRequest`, `AdminResponse`, `ClusterMetricsDetail`, `JobSummary` |
| `serde` / `serde_json` | 1 / 1 | (de)serialization for future config file (S-5) |
| `anyhow` | 1 | error type for CLI glue |
| `tracing` / `tracing-subscriber` | 0.1 | structured logs (stderr + ring buffer) |
| `directories` | 5 | cross-platform config + log paths (`~/Library/Logs/bee-gui/` etc.) |
| `clap` | 4 (derive) | CLI args (`--connect`, `--log-level`) |

### 2. CLI surface

```
bee-gui --connect <admin_addr>          # required (S-1a single-connection)
         [--log-level <level>]          # debug | info | warn | error (default: info)
         [--no-window-decorations]      # macOS: hide traffic-light buttons
```

### 3. Data flow

```
┌─────────────────────────────────────────────────────────┐
│                 iced main thread                        │
│                                                         │
│  App::update(Message) → mutates state → View::view()    │
│       ▲                                       │         │
│       │                                       ▼         │
│  Subscription<ConnectionMsg>              draw widgets  │
└──────│──────────────────────────────────────────────────┘
       │ tokio::sync::mpsc<ConnectionMsg>
       ▼
┌─────────────────────────────────────────────────────────┐
│            tokio runtime (own std::thread)              │
│                                                         │
│  loop {                                                 │
│    client = AdminClient::connect(addr).await;           │
│    if Ok(c)  → send(StateChanged(Connected));           │
│                run_request_loop(c, tx).await;           │
│    if Err(e) → send(StateChanged(Error(e)));            │
│                sleep 2s, retry;                         │
│  }                                                      │
└─────────────────────────────────────────────────────────┘
```

`Connection` holds `addr`, `Arc<std::sync::Mutex<ConnectionState>>`, `mpsc::Sender<ConnectionMsg>` (to main), and a `Sender<Cmd>` to the request loop.

### 4. Connection state machine

```
            (start, --connect)
                    │
                    ▼
            ┌──────────────┐
            │ Connecting   │
            └──────┬───────┘
                   │  TCP handshake + Ping OK
                   ▼
            ┌──────────────┐
   ┌───────►│  Connected   │◄────┐
   │        └──────┬───────┘     │
   │               │             │
   │  RPC fail    │   5s × 3 ping miss
   │  (counted)   │             │
   │               ▼             │
   │        ┌──────────────┐     │
   │        │ Error(reason)│─────┘  (Retry button)
   │        └──────┬───────┘
   │               │ unreachable / explicit close
   │               ▼
   │        ┌──────────────┐
   └────────│ Disconnected │
            └──────────────┘
```

- **Ping cadence:** every 5s while `Connected`. Failure counter resets on success.
- **Disconnect threshold:** 3 consecutive ping failures → transition to `Error(reason)`.
- **RPC errors during `Connected`:** logged + surfaced in UI banner, but **do not** downgrade connection state (avoid flicker on transient failures).

### 5. tokio bridge protocol

```rust
pub enum ConnectionMsg {
    StateChanged(ConnectionState),
    CallResult { id: u64, result: Result<AdminResponse, GuiError> },
}

pub enum Cmd {
    Call { id: u64, req: AdminRequest, reply: oneshot::Sender<Result<AdminResponse, GuiError>> },
    Ping,  // internal 5s tick
    Shutdown,
}
```

- Main thread sends `Cmd::Call{ id, req, reply }` → spawned task awaits `reply`.
- Spawned task calls `AdminClient::call(req).await`, wraps in `CallResult`, sends back.
- Main thread receives `CallResult { id, result }` in `Subscription<ConnectionMsg>`, routes to the waiting `oneshot::Sender` by id.
- `App::update` collects the result and updates the relevant state slice (Cluster data, Jobs list, etc.).

## Design system

### 5.1 Color tokens

**Light theme**

```
Background:
  --bg-base        #FAFAFA   (main canvas, off-white)
  --bg-surface     #FFFFFF   (cards, panels)
  --bg-elevated    #FFFFFF + subtle shadow (hover/floating)

Text:
  --text-primary   #0A0A0A
  --text-secondary #6B6B6B
  --text-tertiary  #A8A8A8   (disabled / placeholder)

Borders:
  --border-subtle  #ECECEC   (hairline separators)
  --border-default #E0E0E0

Accent palette (reserved for S-1b/c; S-1a references only via StatusDot):
  --accent-blue    #007AFF
  --accent-green   #34C759
  --accent-red     #FF3B30
  --accent-orange  #FF9500
  --accent-purple  #AF52DE
```

**Dark theme**

```
  --bg-base        #1C1C1E   (Apple dark base)
  --bg-surface     #2C2C2E
  --text-primary   #F5F5F7
  --text-secondary #98989D
  --border-subtle  #38383A
  --accent-*       same hexes, perceptual lightness adjusted if contrast < 4.5:1
```

### 5.2 Spacing, radii, typography

```
Spacing (4px base):
  --space-1  4px
  --space-2  8px
  --space-3  12px
  --space-4  16px    ← default card padding
  --space-6  24px
  --space-8  32px    ← page padding
  --space-12 48px

Radii:
  --radius-sm  4px   (chips, small buttons)
  --radius-md  6px   (inputs, primary buttons — Apple style)
  --radius-lg  10px  (cards)
  --radius-xl  16px  (modals)

Fonts (platform-adaptive):
  macOS:   -apple-system, "SF Pro Text"
  Windows: "Segoe UI"
  Linux:   "Inter", "Helvetica Neue"

Type scale:
  --font-caption  11px / 1.3
  --font-body     13px / 1.45   (Apple default body)
  --font-body-lg  15px / 1.4
  --font-h2       20px / 1.3
  --font-h1       28px / 1.2    (Dashboard hero numbers)

Weights: regular 400, medium 500, semibold 600
```

### 5.3 iced Theme integration

```rust
// theme.rs
pub fn light() -> Theme { Theme::Custom(Custom::new("Bee Light", palette_light())) }
pub fn dark()  -> Theme { Theme::Custom(Custom::new("Bee Dark",  palette_dark())) }

// S-1a: current() returns light by default; S-1b adds the toggle UI.
pub fn current() -> Theme { light() }
```

`light()` and `dark()` are both implemented and reachable from code; the **switch button in the top bar is not rendered in S-1a** — it ships in S-1b alongside the system-follow toggle.

### 5.4 Lucide Icons (30 selected)

Curated from https://lucide.dev (ISC license, MIT-compatible). Each is a single `*.svg` file (1-3 KB), compiled into the binary via `include_bytes!`.

| File | Semantic |
|---|---|
| `gauge.svg` | Dashboard tab |
| `database.svg` | 数据管理 tab |
| `workflow.svg` | Pipelines tab |
| `settings.svg` | 设置 tab |
| `network.svg` | Cluster card |
| `crown.svg` | Leader badge |
| `activity.svg` | Active tasks |
| `check-circle.svg` | Completed |
| `alert-triangle.svg` | Failed / warning |
| `refresh-cw.svg` | Refresh button |
| `search.svg` | (placeholder for S-2 search bars) |
| `x.svg` | Close / cancel |
| `check.svg` | Confirm |
| `chevron-right.svg` | Navigation |
| `info.svg` | Info tooltip |
| `circle-dot.svg` | Status indicator |
| `plus.svg` | Add (S-2 data-source create) |
| `trash-2.svg` | Delete (S-2 datasource unregister, S-3 pipeline delete) |
| `play.svg` | Run / start |
| `pause.svg` | Pause |
| `stop-circle.svg` | Stop / cancel |
| `loader.svg` | Loading spinner |
| `download.svg` | Download (S-2 plugin install) |
| `upload.svg` | Upload |
| `unplug.svg` | Unplug (S-2 datasource disconnect) |
| `terminal.svg` | (reserved for future CLI panel) |
| `history.svg` | (reserved for job history view in S-3) |
| `bar-chart-3.svg` | (reserved for metrics chart in S-1c) |
| `copy.svg` | Copy to clipboard |

Sizes: 16px (small inline), 20px (sidebar/tabs), 24px (card titles), 32px (empty-state illustration).

```rust
// icons.rs
pub const GAUGE: &[u8]      = include_bytes!("../icons/gauge.svg");
pub const DATABASE: &[u8]   = include_bytes!("../icons/database.svg");
// ... 28 more

pub fn render(bytes: &[u8], size: u16, color: Color) -> Svg { /* fill override */ }
```

### 5.5 Accent palette usage rules (for S-1b/c)

- ✅ Status dots (Job / node state)
- ✅ Numeric badges ("5 failed")
- ✅ Active-tab underline (2px blue bar)
- ❌ No large fills (keep backgrounds neutral)
- ❌ No accent on main background

## Main window & Dashboard

### 6.1 Window chrome

```
┌──────────────────────────────────────────────────────────────┐
│ ● 127.0.0.1:10001     [gauge][database][workflow][settings] │ ← 40px AppBar
├──────────────────────────────────────────────────────────────┤
│                                                              │
│              Main content (changes per tab)                  │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│ bee-gui v0.1.0  ·  127.0.0.1:10001  ·  Last sync: 14:32:18  │ ← 24px StatusBar
└──────────────────────────────────────────────────────────────┘
```

- **AppBar (40px)**: left = connection dot + addr; center = 4 tab buttons (icon + label, label in `--font-caption`); right = empty (S-1b adds theme toggle).
- **StatusBar (24px)**: version, target addr, last sync timestamp.
- **No left sidebar** (top tabs suffice for 4 tabs). If a 5th is added later, sidebar becomes the migration path.
- macOS traffic-light buttons remain visible (unless `--no-window-decorations`).

### 6.2 Dashboard Minimal page

```
┌──────────────────────────────────────────────────────────────┐
│  Dashboard                                  [↻ Refresh]     │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐              │
│  │ Cluster    │  │ Jobs       │  │ Tasks      │              │
│  │            │  │            │  │            │              │
│  │ 3 nodes    │  │ 12 total   │  │ 8 running  │              │
│  │ 1 leader   │  │ 9 running  │  │ 3 completed│              │
│  │ term 5     │  │ 2 completed│  │ 1 failed   │              │
│  │ commit 142 │  │ 1 failed   │  │            │              │
│  └────────────┘  └────────────┘  └────────────┘              │
│                                                              │
│  ── Nodes ───────────────────────────────────────────────    │
│  ID    Role       Term    Commit    Log length               │
│  1     Leader     5       142       142                      │
│  2     Follower   5       142       141                      │
│  3     Follower   5       142       140                      │
│                                                              │
│  ── Recent Jobs ──────────────────────────────────────────   │
│  JobId  Name        Status      Tasks   Owner   Started      │
│  1      binance     ● Running   3/3     N1      14:30:12     │
│  2      news        ● Completed 2/2     N2      14:28:45     │
│  3      analytics   ● Failed    1/2     N1      14:25:01     │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

### 6.3 Visual specs

**Stat Card** (240×120 px, fixed 3-column layout in S-1a)
- `--bg-surface` background, `--border-subtle` 0.5px border, `--radius-lg` (10px), near-zero shadow (visible only in dark theme).
- Layout: top = icon (24px) + title (`--font-body-lg` medium); middle = hero number (`--font-h2` 28px semibold); bottom = sub-metrics (`--font-caption` secondary).

**Tables** (Nodes + Recent Jobs)
- Row height 32px (compact).
- Header: `--font-caption` 11px secondary, uppercase letter-spacing 0.05em.
- Body: `--font-body` 13px primary.
- Leader row in Nodes: left 3px `--accent-blue` bar.
- Status column in Jobs: colored dot (green/red/gray/orange) + label.
- No zebra striping, no hover highlight, no Material ripples.
- Column dividers: 0.5px `--border-subtle`. Outer border omitted.
- JobId / Name links: cursor pointer (S-1a shows tooltip "Detail view in S-2" on hover; no nav yet).

**Refresh button** (icon-only, top-right of Dashboard)
- `refresh-cw.svg`, 18px.
- Hover: tooltip "Refresh" (S-1a uses iced's built-in `tooltip` if available; otherwise OS-native hover label; S-1b replaces with custom drawn tooltip).
- On click: icon spins 360° over the request duration.

### 6.4 Interactions

| User action | Trigger | Behavior |
|---|---|---|
| Launch | `bee-gui --connect 127.0.0.1:10001` | Connecting → 3 RPCs (Ping / ClusterStatus / ListJobs) issued concurrently → render |
| Click Refresh | top-right icon | same 3 RPCs re-issued, button spins |
| Click tab | icon/label | swaps content; Dashboard state preserved across tab switches |
| Click JobId / Name | link hover + click | S-1a: cursor changes; S-2 will navigate |
| Connection lost | 5s × 3 ping fail | red banner appears at top; **last successful data preserved** (no flicker) |
| Click Retry (on banner) | button | forces `Disconnected → Connecting` immediately |

### 6.5 State preservation

Dashboard data persists across tab switches (held in `App` state). Re-fetch only on: (a) Refresh click, (b) launch, (c) recovery from error. This prevents tab-switch RPC storms.

### 6.6 Placeholder pages (3 of 4 tabs)

```
┌─────────────────────────────────────────────┐
│                                             │
│             [database icon 64px]            │
│                                             │
│            数据管理                          │
│                                             │
│   此功能将在 S-2 中实现                     │
│   添加 / 管理数据源、插件                   │
│                                             │
└─────────────────────────────────────────────┘
```

Same template for Pipelines (→ S-3 + S-4) and 设置 (→ S-5).

## Error handling & logging

### 7.1 `GuiError` enum

```rust
#[derive(Debug, thiserror::Error)]
pub enum GuiError {
    #[error("Connect failed to {addr} after {attempts} attempts: {last_err}")]
    Connect { addr: SocketAddr, attempts: u32, last_err: String },

    #[error("RPC timeout after {elapsed_ms}ms (rpc={rpc})")]
    Timeout { rpc: &'static str, elapsed_ms: u64 },

    #[error("Server returned error: {msg}")]
    RpcServer { msg: String },

    #[error("Wire {kind} error: {detail}")]
    Wire { kind: WireErrKind, detail: String },

    #[error("I/O error: {source}")]
    Io { #[source] source: std::io::Error },

    #[error("Connection lost (last seen {last_seen_ms}ms ago)")]
    ConnectionLost { last_seen_ms: u64 },

    #[error("Cancelled by user")]
    Cancelled,
}

pub enum WireErrKind { Decode, Encode }
```

### 7.2 Logging contract

Every `GuiError` is logged via `tracing::error!` with **full chain context**:

```rust
fn log_rpc_failure(ctx: &CallContext, err: &GuiError) {
    tracing::error!(
        target: "bee_gui.rpc",
        call_id = ctx.id,
        rpc = %ctx.rpc_kind,
        addr = %ctx.addr,
        started_at_ms = ctx.started_at_ms,
        elapsed_ms = ctx.elapsed_ms,
        attempt = ctx.attempt,
        connection_state = %ctx.conn_state,
        err.kind = ?err,
        err.detail = %err,
        err.chain = ?std::iter::successors(Some(err as &dyn std::error::Error), |e| e.source()).collect::<Vec<_>>(),
        "RPC call failed"
    );
}
```

Server-side mirroring (in `crates/bee-control/src/raft/admin_server.rs`, no protocol change):

```rust
Err(e) => {
    tracing::error!(
        target: "bee.admin",
        request_kind = ?std::mem::discriminant(&req),
        client = %client_addr,
        "AdminRequest dispatch failed: {e}"
    );
    AdminResponse::Error(format!("{e}"))
}
```

Applied at every existing `AdminResponse::Error(msg)` construction site.

### 7.3 UI error display (3-layer)

1. **Banner title** (single line): `RPC 失败：ClusterStatus`
2. **Banner detail** (multi-line, scrollable): full message + timestamp + RPC type + addr + retry count
3. **Action bar** (3 buttons): `重试` / `复制日志` / `查看完整日志`

### 7.4 `LogPanel` widget

Right-side drawer, real-time scroll (1s polling, no SSE in S-1a).

```
┌─ 日志 ────────────────────────────────────────┐
│ 14:32:18.245 ERROR  RPC 失败                  │
│   RPC: ListJobs                               │
│   Addr: 127.0.0.1:10001                       │
│   Kind: RpcServer                             │
│   Detail: "tenant=0 not authorized"           │
│   Chain:                                     │
│     RpcServer("tenant=0 not authorized")      │
│     bincode(...)                              │
│ 14:32:18.001 INFO    RPC 启动                 │
│   ...                                         │
└───────────────────────────────────────────────┘
```

- Ring buffer, max 1000 entries (FIFO eviction).
- Each entry click-to-expand chain.
- Top buttons: `导出日志` → writes `~/Library/Logs/bee-gui/bee-gui.log` (macOS) / `~/.local/share/bee-gui/log/` (Linux) / `%APPDATA%/bee-gui/log/` (Windows) — using `directories` crate.

## Packaging & release

### 8.1 Homebrew formula

`crates/bee-gui/packaging/homebrew/bee-gui.rb`:

```ruby
class BeeGui < Formula
  desc "Bee management GUI (iced / Rust)"
  homepage "https://github.com/smitea/bee"
  url "https://github.com/smitea/bee/releases/download/v#{VERSION}/bee-gui-v#{VERSION}-macos-universal.tar.gz"
  sha256 "<COMPUTED_AT_RELEASE_TIME_FROM_ACTUAL_TARBALL>"
  license "Apache-2.0"

  depends_on :macos

  def install
    bin.install "bee-gui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/bee-gui --version")
  end
end
```

### 8.2 Debian/Ubuntu package via `cargo-deb`

`crates/bee-gui/Cargo.toml`:

```toml
[package.metadata.deb]
name = "bee-gui"
maintainer = "smitea <smitea@example.com>"
description = "Bee cluster management GUI (iced / Rust)"
license-file = ["../../LICENSE"]
section = "admin"
priority = "optional"
depends = "$auto"
assets = [
    ["target/release/bee-gui", "usr/bin/", "755"],
]
```

Build: `cargo deb -p bee-gui --target x86_64-unknown-linux-gnu`

### 8.3 Cross-platform release script

`scripts/build-release.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
VERSION="${1:-0.1.0}"

build_target() {
    local target="$1"
    local out_dir="dist/bee-gui-${VERSION}-${target}"
    cargo build --release -p bee-gui --target "$target"
    mkdir -p "$out_dir"
    cp "target/${target}/release/bee-gui" "$out_dir/"
    tar czf "dist/bee-gui-${VERSION}-${target}.tar.gz" -C dist "bee-gui-${VERSION}-${target}"
}

build_target x86_64-apple-darwin
build_target aarch64-apple-darwin
build_target x86_64-unknown-linux-gnu
build_target x86_64-unknown-windows-msvc
```

### 8.4 Platform matrix (S-1a)

| OS | Install path | CI build |
|---|---|---|
| macOS (Intel + Apple Silicon) | Homebrew tap | ✅ |
| Debian / Ubuntu | `apt install ./bee-gui.deb` | ✅ |
| Windows (Server 2019+) | `tar xzf` + PATH | tarball only, no MSI in S-1a |

## Test strategy

### 9.1 Unit tests (`#[cfg(test)]`)

| Test | File | Asserts |
|---|---|---|
| `light_and_dark_themes_construct` | `theme.rs` | both `light()` / `dark()` return valid `Theme` |
| `wcag_aa_text_contrast` | `theme.rs` | `--text-primary` vs `--bg-surface` ≥ 4.5:1 in both themes |
| `lucide_icon_loads` | `icons.rs` | all 30 SVGs parse to a valid `Svg` widget (smoke check) |
| `connection_state_machine` | `connection.rs` | `Connecting→Connected→Error→Connecting` cycle; ping counter resets on success |
| `gui_error_chain` | `error.rs` | each `GuiError` variant's `Display` includes its source chain |
| `log_ring_buffer_eviction` | `log_panel.rs` | 1001st entry evicts oldest |
| `log_export_writes_file` | `log_panel.rs` | "导出日志" produces a file at the platform-correct path |

### 9.2 Integration tests (`crates/bee-gui/tests/`)

| Test | Asserts |
|---|---|
| `connection_smoke` | launch in-process 3-node Cluster → issue `Ping` → receive `Pong` within 3s |
| `refresh_updates_data` | register a job via `cluster.submit` → trigger Refresh → Dashboard's Recent Jobs count increments |
| `connection_lost_recovery` | spawn cluster → kill a node → 5s × 3 ping fail → state transitions to `Error` → restart node → click Retry → state returns to `Connected` |
| `error_log_panel_entries` | trigger an `RpcServer` error → LogPanel entry present + chain correct + "复制日志" returns expected text |

### 9.3 Visual / manual checklist

`docs/testing/s-1a-visual-checklist.md`:

```
□  bee-gui --connect 127.0.0.1:10001 launches
□  After connect, AppBar shows green dot + 3 stat cards filled
□  Nodes table renders with leader row having blue accent bar
□  Recent Jobs table renders
□  Refresh button spins during fetch, then stops
□  Switching tabs preserves Dashboard data
□  Killing bee node → 5s × 3 → red banner; last data still visible
□  Restarting node → Retry → green dot returns
□  Triggering 1 bincode error → LogPanel entry + 复制日志 works
□  macOS brew install ./bee-gui.rb runs `bee-gui --version` successfully
□  cargo deb produces a valid .deb
□  Light theme reads cleanly; toggling dark via code (`current()` swap) reads cleanly too
```

## Acceptance criteria

### 10.1 Blocking

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` green (includes new `crates/bee-gui` tests)
- [ ] `cargo run -p bee-gui -- --connect 127.0.0.1:10001` shows Dashboard with 3 stat cards + 2 tables
- [ ] Refresh button re-issues Ping / ClusterStatus / ListJobs and updates state
- [ ] All 4 tabs (Dashboard / 数据管理 / Pipelines / 设置) are reachable; non-Dashboard tabs show "Coming in S-X" placeholders
- [ ] All 30 Lucide icons render (no "missing glyph" / 0-byte / parse error)
- [ ] Disconnection: 5s × 3 ping fail → red banner; Retry restores
- [ ] Every error path emits a `tracing::error!` line with full chain context (verified by `error_log_panel_entries` test + stderr capture)
- [ ] Homebrew formula `brew install ./bee-gui.rb` runs `bee-gui --version` successfully
- [ ] `cargo deb -p bee-gui` produces a valid `.deb` file that installs and runs

### 10.2 Design conformance (review checklist)

- [ ] Apple-minimal aesthetic: white/light background, hairline borders, 6-10px radii, generous whitespace
- [ ] icon-first: top tabs render icon + label with icon dominant
- [ ] Lucide icons render at sizes 16 / 20 / 24 / 32 px correctly
- [ ] light + dark theme builders both compile and reach `Theme::Custom`
- [ ] Tables: no zebra, no hover highlight, only 0.5px column dividers
- [ ] Leader row has 3px left blue accent bar
- [ ] Status dots in Jobs table use green / red / gray / orange from accent palette

### 10.3 Performance baseline (recorded, not blocking)

- Cold-start to first paint < 1.5s on M1 / equivalent
- Refresh round-trip < 500ms (3 concurrent RPCs)
- Idle memory < 80 MB
- Idle CPU < 1%

### 10.4 Explicit non-goals

- ❌ Multi-connection management (→ S-1b)
- ❌ Rich Dashboard charts / live event stream / multi-cluster comparison (→ S-1c)
- ❌ Drawn tooltip widget (→ S-1b; S-1a uses OS-native where unavoidable)
- ❌ Theme-switch UI in AppBar (→ S-1b)
- ❌ Any functionality on 数据管理 / Pipelines / 设置 tabs (→ S-2, S-3, S-4, S-5)
- ❌ `bee --gui` subcommand integration
- ❌ HTTP/REST proxy layer (GUI connects directly to bincode AdminServer)
- ❌ Tauri / webview / web frontend of any kind

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| GUI framework | **iced 0.12** | per user direction (originally Nuxt.js, revised to GUI framework, then to iced for modern-minimal in pure Rust) |
| Connection mode | **Single (S-1a) → multi (S-1b)** | matches user's "多连接并发" intent, but S-1a only proves single works |
| Dashboard richness | **Minimal in S-1a**, Rich in S-1c | matches the S-1a/S-1b/S-1c split |
| Refresh strategy | **Manual Refresh button** in S-1a; server push (SSE/WebSocket) deferred to S-1c | matches the user's "Server push" choice but deferred to the slice that adds the RPC |
| Icon library | **Lucide Icons (30 selected)** | per user choice |
| Visual style | **Apple-minimal, icon-first, light + dark tokens, accent palette reserved** | per user direction |
| Backend changes | **`tracing::error!` only** (no protocol / no new RPC) | keeps S-1a as a frontend story; the requested "detailed server error logs" satisfied via server-side tracing + structured `Error(msg)` strings already passed to client |
| Crate layout | **Standalone `crates/bee-gui/` (not subcommand of `bee`)** | per option A choice; revisits later if shared config/auth becomes a concern |

If any of these should change, the user can override during the spec review.
