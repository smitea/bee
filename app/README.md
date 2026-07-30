# Bee Client

The desktop workspace for managing [Bee](../CONTEXT.md) clusters and Applications. Bee Client is a Tauri 2.x application that pairs a React + TypeScript workspace with a Rust application layer. It speaks to a Bee Cluster over the AdminServer protocol, surfaces Pipeline/Datasource/Plugin state, and persists Applications, Dashboards, and connection profiles in a local SQLite database.

## Overview

Bee Client replaces the original four-tab Bee GUI with a resource-oriented workspace. A Navigation tree on the left opens Cluster, Application, Dashboard, Pipeline, and Datasource nodes as deduplicated, closable tabs in the right workspace. A fixed bottom bar carries the connection indicator, the latest audit event, and the activity dialog. Application lifecycle (enable / disable / import / export) is an idempotent, restartable state machine that complements the Raft-backed Control Plane without ever bypassing it.

## Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.89.0 | Pinned via `rust-toolchain.toml` at the repo root. The `app/src-tauri` crate declares `rust-version = "1.77.2"` but the workspace build targets 1.89.0. |
| Node.js | 20 LTS | Matches the `tauri-frontend-build` CI job. |
| Tauri CLI | `^2.0` | Install once per machine: `cargo install tauri-cli --version '^2.0'`. |
| npm | 10+ | Bundled with Node 20. |

Platform notes:

- **macOS**: Xcode Command Line Tools (or full Xcode) are required for the Rust linker and for the WebView. The Rust toolchain handles codesigning locally during development; production builds use the bundle output.
- **Linux**: `libwebkit2gtk-4.1-dev`, `libssl-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, and `librsvg2-dev` are required by Tauri 2.x. See the Tauri docs for the current Linux dependency list.
- **Windows**: Microsoft Edge WebView2 (preinstalled on Windows 10/11) and the Visual Studio C++ build tools.

## Quick start

```bash
# 1. Install JavaScript dependencies.
cd app
npm install

# 2. Launch in dev mode (Vite + Tauri together).
#    The Tauri shell opens a window that talks to the Vite dev server on :1420.
cargo tauri dev

# 3. Build a production bundle.
#    Output appears under app/src-tauri/target/release/bundle/.
cargo tauri build
```

The dev mode reads `BEE_ADMIN_ADDR` (default `127.0.0.1:9999`) for the cluster to manage. Point it at a running Bee node before launching:

```bash
BEE_ADMIN_ADDR=127.0.0.1:9999 cargo tauri dev
```

## Project structure

```
app/
├── README.md                      # this file
├── package.json                   # JS deps + npm scripts (dev, build, test, tauri)
├── package-lock.json
├── tsconfig.json                  # TypeScript config
├── tsconfig.node.json
├── vite.config.ts                 # Vite + Vitest config
├── postcss.config.js
├── tailwind.config.js
├── index.html                     # Vite entry HTML
├── src/                           # React + TypeScript workspace
│   ├── App.tsx                    # root component, QueryClient + theme provider
│   ├── main.tsx                   # ReactDOM entry
│   ├── styles.css                 # Tailwind layers + base styles
│   ├── tooltip.ts                 # tiny tooltip helper
│   ├── components/                # shell + reusable UI (NavTree, AppShell, Settings, ...)
│   ├── pages/                     # page-tab content (Cluster, Pipelines, Datasources, ...)
│   ├── ipc/                       # typed wrappers around `invoke()` for each Tauri command
│   ├── state/                     # Zustand stores (tabs, connection, search, tenant, ...)
│   ├── domain/                    # pure domain helpers (status colors, pipeline utils, ...)
│   └── tests/                     # Vitest unit + component tests
├── src-tauri/                     # Tauri Rust application layer
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json            # window, CSP, bundle, build/dev commands
│   ├── capabilities/              # Tauri 2 capabilities (default.json is `core:default` only)
│   ├── icons/                     # bundled icons for installer
│   ├── src/
│   │   ├── lib.rs                 # Tauri builder + plugin registration
│   │   ├── main.rs                # binary entry
│   │   ├── connection.rs          # AdminClient lifecycle + tokio bridge
│   │   ├── commands/              # one file per command module (cluster, pipelines, ...)
│   │   ├── db/                    # SQLite migrations + repository modules
│   │   ├── import_export.rs       # encrypted Application package (argon2 + AES-GCM)
│   │   ├── plugin_registry.rs     # live Plugin Registry loader
│   │   ├── rolling_restart.rs     # quorum-safe rolling restart orchestration
│   │   ├── settings_io.rs         # draft/save settings I/O
│   │   ├── tenant.rs              # tenant access rules (ADR-0010)
│   │   └── audit_seed.rs          # audit-event seed data
│   └── tests/                     # Rust integration tests (cluster harness, plugin load)
└── dist/                          # Vite build output (committed-ready for `cargo tauri build`)
```

## Tech stack

| Layer | Library | Version |
|---|---|---|
| Desktop shell | Tauri | 2.0.6 |
| UI framework | React | 18.3 |
| Language | TypeScript | 5.5 |
| Styling | Tailwind CSS | 3.4 |
| Server state | TanStack Query | 5.51 |
| Presentation state | Zustand | 4.5 |
| Graph rendering | `@xyflow/react` | 12.3 |
| Visualization | Apache ECharts + `echarts-for-react` | 5.5 / 3.0 |
| K-line charts | klinecharts | 10.0 |
| SQL editor | CodeMirror 6 (`@codemirror/lang-sql`, `@uiw/react-codemirror`) | 6 / 4.23 |
| Icons | lucide-react | 0.408 |
| Frontend tests | Vitest + Testing Library + jsdom | 1.6 / 16 / 24 |
| Local DB | rusqlite (bundled) | 0.31 |

Backend Rust deps include `serde`, `serde_json`, `tokio`, `tauri-plugin-log`, `bee-control`, `bee-plugin-sdk`, `argon2`, `aes-gcm`, `libloading`, `sha2`, `hex`, and `rand`.

## Architecture

Bee Client is a three-layer system:

1. **React workspace** — renders navigation, page tabs, dialogs, editors, and live status. Server state lives in TanStack Query; temporary presentation state lives in Zustand. **Components never touch `localStorage` or SQLite directly.**
2. **Tauri Rust application layer** — owns SQLite, encrypted import/export, local Settings, workspace restoration, the Plugin Registry loader, the Connection bundle, rolling-restart orchestration, and typed IPC commands. State is migrated through 14 forward-only SQLite migrations (`app/src-tauri/src/db/mod.rs`).
3. **AdminServer + Control Plane** — owns cluster settings, Plugins, Datasources, Pipeline definitions, Jobs, audit events, cluster-resource search, and rolling restart on the cluster side. Bee Client calls these via typed IPC and never bypasses the AdminServer protocol.

Key boundaries:

- **Frontend** consumes `src/ipc/*` wrappers that call `invoke()` against commands declared in `app/src-tauri/src/commands/`.
- **Rust** holds the AdminClient bundle keyed by current connection addr; switching the active cluster runs through `connection::ensure_bundle`.
- **Persistence** is split: Bee Cluster state is re-fetched from AdminServer; Bee Client state (Applications, Dashboards, profiles, settings, tabs, audit buffer) lives in SQLite.
- **Long-running mutations** return an Operation ID and stream progress; mutations accept idempotency keys to prevent duplicate Jobs/Datasources/restarts.

## Features

| Area | What it does |
|---|---|
| Shell | Single-row header (Bee brand + cluster dropdown + refresh + settings + theme) + Navigation tree + page-tab workspace + bottom activity/connection bar. Setting is a modal, not a tab. |
| Navigation | Cluster, global search, Applications (count) + add, expandable Application tree. Every refreshable section has a refresh icon. |
| Page tabs | Cluster, Application, Dashboard, Pipelines, Pipeline detail, Datasources, Datasource detail. Opening an already-open resource focuses its tab. Pin, close, close others, close to the right, and session restoration. |
| Search | Text-change debounced, cancels stale requests, merges local Application/Dashboard results with AdminServer cluster-resource search by relevance. |
| Settings | Two-column modal: Client, Connection, Appearance, Logging, Diagnostics, Cluster, Raft, KV, Scheduling, Plugins, Security. All auto-save; Connection exposes Test Connection and Connect. |
| Plugins | Application Datasource creation uses the live Plugin Registry; `Plugins registered: N` badge on the Datasources page header. |
| Cluster Dashboard | Topology graph (symmetric ring, leader chassis centered, orthogonal edges), Raft Leader, Quorum Health, Commit Index, queued/running/historical/failed Jobs, longest-running and highest-consuming Jobs, average runtime, active configuration + rolling-restart operations. |
| Application Dashboard | Per-application Grafana-style configurable layout: add/drag/resize/remove panels (K-line, Active jobs, Tasks/sec, CPU, Pipeline status, Audit feed, Cluster topology). Edit mode shows dashed outlines; view mode shows clean chrome. |
| Pipelines | Pipeline definitions, queued/running/historical/failed Jobs, Pipeline detail with interactive structure graph (Input Datasource → Phase Handler(s) → Output Datasource), cross-Pipeline edges, runtime status overlay, accessible non-graph fallback. |
| Datasources | Per-application list, Add Datasource modal that fetches Plugin Registry state, schema-driven connection form, Test Connection + Connect and Save. Credentials are redacted from logs and not retained in caches. |
| Activity bar | Bottom bar shows the latest durable audit event; clicking opens the activity dialog (refresh, pagination, filters, navigable details). |
| Tenant | Tenant enum on Application, Cluster Profile, and AdminServer Job submission. `tenant.rs` exposes `can_access_datasource` and `validate_tenant` (0..=65535). |
| Multi-cluster | Migration v10 introduces `cluster_profiles` (first-class saved connections). Settings → Cluster lists, edits, and removes profiles; the header dropdown lets you switch the active cluster. Legacy `bee-gui.connections` localStorage entries are migrated once at first run. |

## IPC surface

The Frontend never speaks to `invoke()` directly. Each command is wrapped in `app/src/ipc/<name>.ts` and re-exported through `app/src/ipc/index.ts`. The corresponding Rust handler lives in `app/src-tauri/src/commands/<name>.rs` and is wired into the `tauri::generate_handler!` macro in `app/src-tauri/src/commands/mod.rs`.

Coverage:

- Cluster: `cluster_status`, `cluster_topology`, `cluster_settings`, `rolling_restart_*`.
- Connection: `connection_state`, `connection_test`, `connection_activate`, `cluster_profile_*`.
- Pipelines: `pipeline_list`, `pipeline_create`, `pipeline_update`, `pipeline_visible_ast`, `pipeline_dump`, `pipeline_submit`, `pipeline_stop`, `pipeline_cancel`, `pipeline_resume`.
- Datasources: `datasource_list`, `datasource_create`, `datasource_inspect`, `datasource_pause`, `datasource_resume`, `datasource_delete`, `datasource_form_schema`, `datasource_connect_test`.
- Plugins: `plugin_list`, `plugin_settings_*`.
- Audit: `audit_list`, `audit_subscribe`, `audit_summary`.
- Search: `search_global` (merges local + AdminServer).
- Tenant: `tenant_get`, `tenant_set`.
- Applications / Dashboards / Settings / Tabs / Profiles: full CRUD plus workspace and dashboard persistence.

Operational conventions:

- Long-running mutations return an `OperationId` and stream progress.
- Mutations accept idempotency keys.
- Subscriptions fall back to bounded polling on failure.

## Testing

```bash
# Frontend (Vitest + Testing Library + jsdom)
cd app
npm test                # one-shot
npm run test:watch      # watch mode
npm run test:coverage   # coverage report

# Backend (Rust unit + integration)
cd app/src-tauri
cargo test
# or from the repo root: cargo test -p app

# Visual Tauri launch (smoke)
cargo tauri dev
# Then interact with the window; macOS screenshots use:
# screencapture -R<x>,<y>,<w>,<h> /tmp/bee-client.png
```

What the suites cover:

- Vitest: NavTree filtering and counts, page-tab deduplication / pinning / restoration, Settings auto-save and connection actions, connection-state indicators, audit dialog details and navigation, Application empty/enabling/disabling/degraded states, import conflict flow, schema-driven Datasource forms, Pipeline graph interactions and accessible fallback, merged local/server search with stale cancellation, dashboard store, context menu, topology helpers.
- Cargo: SQLite migrations and repository transactions, Application lifecycle state-machine recovery and idempotency, encrypted package round trips + wrong-passphrase + tampering + schema migration, AdminServer protocol additions, audit-event atomicity and redaction, global search and pagination, configuration versioning and quorum-safe rolling restart, Pipeline structure-graph serialization, cluster harness integration.

## Development workflow

### Add a new IPC command

1. **Rust** — add a `#[tauri::command]` to `app/src-tauri/src/commands/<area>.rs`, register it in `app/src-tauri/src/commands/mod.rs`, and (if it owns state) wire the repository through `app/src-tauri/src/lib.rs`.
2. **TypeScript** — add a typed wrapper in `app/src/ipc/<area>.ts`, re-export it from `app/src/ipc/index.ts`, and consume it via TanStack Query in the relevant page.
3. **Tests** — add a unit test next to the Rust handler and a Vitest mock at the IPC wrapper.

### Add a new SQLite migration

1. Open `app/src-tauri/src/db/mod.rs`.
2. Append a new `Migration { version: N, name: ..., up: |tx| { ... } }` to the `MIGRATIONS` array. Migrations are forward-only and idempotent at the version-check layer.
3. Add a repository module in `app/src-tauri/src/db/<area>.rs` and expose it through `db/mod.rs`.
4. Add a Rust unit test that runs the migration against an in-memory database.

### Add a new tab kind / route

1. Register the resource type in `app/src/state/tabsStore.ts` (open, focus, close, pin, close-others, close-to-right).
2. Add a page component under `app/src/pages/<Resource>.tsx` and link it from the `NavTree`.
3. Add a Vitest under `app/src/tests/pages/<Resource>.test.tsx` covering the tab + nav flow.

### Run a single test file

```bash
# Frontend
cd app && npx vitest run src/tests/pages/ClusterDashboard.test.tsx

# Backend
cd app/src-tauri && cargo test --test bee_cluster_integration
```

## Building & deploying

```bash
# Production bundle (installer + executable).
cd app
cargo tauri build
# Outputs:
#   app/src-tauri/target/release/bundle/dmg/         (macOS)
#   app/src-tauri/target/release/bundle/msi/         (Windows)
#   app/src-tauri/target/release/bundle/deb/         (Debian)
#   app/src-tauri/target/release/bundle/appimage/    (Linux)
#   app/src-tauri/target/release/bundle/<exe>        (raw binary)
```

### Headless CLI

`bee-cli` reads the same SQLite state as Bee Client without starting a WebView. Its output is pipe-friendly: list records use one tab-delimited row per entity, while describe and status commands use tab-delimited key/value rows.

```bash
scripts/bee-cli list applications
scripts/bee-cli describe application 1
BEE_CLIENT_DB=/path/to/bee-client.sqlite scripts/bee-cli list plugins
scripts/bee-cli list pipelines
scripts/bee-cli list datasources
scripts/bee-cli describe connection
scripts/bee-cli migrate-status
```

The wrapper selects Bee Client's platform application-data path unless `BEE_CLIENT_DB` overrides it. `scripts/bee-cli reset --force` permanently deletes `bee-client.sqlite` and its `-wal` and `-shm` sidecars; the next GUI or CLI launch creates a fresh database and applies all migrations.

Run Bee in a containerised 5-node cluster for end-to-end testing:

```bash
# From the repo root.
docker compose -f docker/docker-compose.yml up -d --build
# See docker/README.md for the full layout and volume conventions.
```

Build the workspace plugins and deploy them into the per-node plugin volumes (or directly into a live container):

```bash
scripts/deploy-plugins.sh --build --mode docker
```

The shipped Bee Client image recipe lives at `docker/Dockerfile.bee-client`.

## Troubleshooting

- **Port 1420 already in use.** The Vite dev server defaults to `:1420`. Either stop the stale `vite` process (`pkill -f vite` or `lsof -ti:1420 | xargs kill -9`) or change `devUrl` and `beforeDevCommand` in `app/src-tauri/tauri.conf.json` to a free port.
- **`cargo tauri dev` loads a stale `dist/`.** Make sure `app/src-tauri/tauri.conf.json` has `build.devUrl` set to `http://localhost:1420`. If you set `frontendDist` to a path, Tauri will serve the static bundle instead of hot-reloading through Vite. Run `npm run dev` first to confirm Vite is serving on the expected port.
- **WebView clicks not registering when driven via `osascript` on macOS.** macOS requires explicit Input Monitoring permission for the calling terminal. Open **System Settings → Privacy & Security → Input Monitoring**, add the terminal you are driving (Terminal.app, iTerm2, or the agent host), and re-run the script. Headless inspection via `screencapture -R` still works without that permission.
- **CSP blocks a script.** The production CSP is intentionally restrictive (`default-src 'self'; ... frame-src 'none'; object-src 'none'; ...`). If you need to add a new external origin, document it in `app/src-tauri/tauri.conf.json` and update the capabilities file under `app/src-tauri/capabilities/`.
- **`sqlite` lock errors on Windows.** Ensure no other Bee Client process is running and that the per-user data directory (`%APPDATA%/io.smitea.beeclient/`) is writable. Close any stale shell that started the bundle and retry.
- **Plugin `.so` not loading.** The Plugin Manager reads Plugin Registry from the configured plugin directory. Verify the plugin's `crate-type` is `["cdylib"]` and that it was built against the same Rust toolchain as Bee Client. See `CONTEXT.md` (Plugin Identity) and ADR-0009 for the binding contract.

## License

Bee Client is part of the Bee project and is released under the **Apache License 2.0**. See [LICENSE](../LICENSE) (or the `license = "Apache-2.0"` declaration in the workspace `Cargo.toml`) for the full text.
