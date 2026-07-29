# S-Tauri · Bee GUI Foundation (Tauri + React)

**Date:** 2026-07-28
**Type:** AFK
**Blocked by:** none
**Replaces:** S-1a (iced 0.12) — `crates/bee-gui/` removed
**Status:** Draft (in progress)

## Why switch

Original S-1a spec used iced (Rust-native). User decision 2026-07-28: switch to Tauri for a larger UI ecosystem and broader contributor base. The backend (Raft + KV + Plugins + Control Plane) is framework-agnostic and stays unchanged; only the GUI binary changes.

## Tech stack

- **Tauri 2.x** (Rust shell + system webview)
- **React 18** + **TypeScript** + **Vite** (frontend)
- **Tailwind CSS** (styling — ships with great-looking defaults)
- **React Query** (server-state + cache for RPC calls)
- **Lucide React** (icons — same icon set as the previous S-1a SVGs)
- **Zustand** (small client-state slice — connection addr, theme, log level)

## Directory layout

```
app/                                  # new Tauri project
├── package.json                       # frontend deps + scripts
├── tsconfig.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
├── index.html
├── src/                               # React frontend
│   ├── main.tsx
│   ├── App.tsx                        # tab routing + layout
│   ├── ipc.ts                         # thin wrapper over `invoke()` commands
│   ├── components/
│   │   ├── AppBar.tsx                 # addr pill + tabs + theme
│   │   ├── StatusBar.tsx
│   │   └── ConnectionPill.tsx
│   ├── pages/
│   │   ├── Dashboard.tsx              # 3 stat cards + 2 tables + Refresh
│   │   ├── DataSources.tsx            # CRUD form + list
│   │   ├── Pipelines.tsx              # job list + Inspect panel
│   │   └── Settings.tsx
│   ├── state/
│   │   └── store.ts                   # Zustand: addr, theme, logLevel
│   └── styles.css                     # tailwind directives
└── src-tauri/                         # Rust backend
    ├── Cargo.toml                     # path deps: bee-control, bee-plugin-sdk
    ├── tauri.conf.json                # window 1100x720, no decorations opt
    ├── build.rs
    └── src/
        ├── main.rs                    # tauri::Builder setup
        ├── lib.rs                     # commands module
        ├── connection.rs               # AdminClient lifecycle (port from crates/bee-gui)
        └── commands.rs                # #[tauri::command] per RPC
```

## Backend commands (Rust → JS)

The Rust side exposes these Tauri commands to JS:

```rust
#[tauri::command]
async fn ping(addr: String) -> Result<String, String>
#[tauri::command]
async fn cluster_status(addr: String) -> Result<ClusterMetricsDetail, String>
#[tauri::command]
async fn list_jobs(addr: String) -> Result<Vec<JobSummary>, String>
#[tauri::command]
async fn job_inspect(addr: String, id: u32) -> Result<Option<JobDetail>, String>
#[tauri::command]
async fn list_datasources(addr: String) -> Result<Vec<Datasource>, String>
#[tauri::command]
async fn create_datasource(addr: String, name: String, adapter: String,
                          plugin_version: String, config: String, tenant: u16)
    -> Result<Datasource, String>
#[tauri::command]
async fn pause_datasource(addr: String, name: String) -> Result<(), String>
// ... resume, delete, inspect
```

For MVP, `addr` is a single global connection (matches S-1a's "single AdminClient").
Future: S-Tauri.x adds multi-connection (mirrors S-1b deferred).

## Frontend shape

- **App.tsx** — `<AppBar />` + `<TabRouter activeTab={...} onChange={...} />` + `<main>{tabBody}</main>` + `<StatusBar />`
- **ipc.ts** — `export async function ping(addr: string) { return invoke('ping', { addr }) }` etc.
- **React Query** — `useQuery(['cluster', addr], () => ipc.clusterStatus(addr), { refetchInterval: 5000 })` for Dashboard auto-refresh
- **Zustand store** — `addr`, `theme` ('light' | 'dark'), `logLevel`, `setAddr`, etc.

## Acceptance criteria (blocking)

- [ ] `cargo tauri build` produces a working `.app` (macOS) / `.exe` (Windows) / `.AppImage` (Linux)
- [ ] `npm run build` (in `app/`) produces a static frontend bundle
- [ ] App launches, connects to AdminServer at user-supplied addr (via UI form, persisted to localStorage)
- [ ] Dashboard shows 3 stat cards + Nodes table + Recent Jobs table, auto-refresh every 5s
- [ ] Data Sources tab: create / list / inspect / pause / resume / delete via in-process DatasourceRegistry (MVP; production wires through AdminServer)
- [ ] Pipelines tab: list + inspect via AdminServer
- [ ] Settings tab: theme toggle (light/dark), log level, diagnostics export
- [ ] All 4 tabs reachable; placeholder content where S-1a spec already had placeholders
- [ ] `cargo test --workspace` green
- [ ] `npm test` in `app/` green (Vitest unit tests for IPC + components)

## Deferred per S-1a spec items

Everything previously deferred for S-1a remains deferred for S-Tauri:

- Multi-cluster management (S-1b feature)
- Live event stream (S-1c feature)
- Metrics charts (S-1c feature)
- Self-drawn tooltip (replaced by Tailwind tooltips)
- Visual regression framework
- S-2/3/4/5 implementation details (now ported to React)

## Migration plan

1. **Phase A** (this slice): scaffold Tauri + minimal Rust backend (ping only) + single tab (Dashboard)
2. **Phase B**: implement Data Sources / Pipelines / Settings pages + Zustand store
3. **Phase C**: remove `crates/bee-gui/` and S-1a spec
4. **Phase D**: update CI (remove iced gui-smoke job, add Node.js setup + `npm run build` + Tauri build verification)
5. **Phase E**: update CONTEXT.md + stories.md (close out S-1a references)

## Build commands

```sh
# Frontend dev (hot reload)
cd app && npm run dev

# Tauri dev (full app)
cd app && cargo tauri dev

# Production build
cd app && cargo tauri build
```

## See also

- Original S-1a spec (archived): `docs/superpowers/specs/2026-07-27-s1a-gui-foundation-design.md` (status: **Superseded**)
- Tauri 2 docs: <https://tauri.app/v2/guides/>
- React Query: <https://tanstack.com/query/latest>
- Bee backend: `crates/bee-control/` (unchanged)