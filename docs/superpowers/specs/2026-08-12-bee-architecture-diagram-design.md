# Bee Architecture Diagram (Draw.io + AI MCP) Design Note

**Date:** 2026-08-12
**Status:** Draft (pending user review)

## Purpose

Produce a high-level architecture overview of Bee as a single Draw.io diagram, committed to the repository so anyone cloning the repo can open it offline, and wired into opencode's MCP so future sessions can edit it with AI assistance via Draw.io.

The deliverable is documentation, not code. No Rust / TypeScript / CI changes are introduced. Only:
- one new file under `docs/diagrams/`,
- one README in the same folder,
- one small validator script under `scripts/`,
- one edit to the global `~/.config/opencode/opencode.jsonc`.

## Non-goals

- Editing Bee source code, the Cargo workspace, or the Bee Client Tauri app.
- Wiring a Draw.io rendering step into CI (the validator only checks XML structure; no PNG/SVG export in this iteration).
- Containerising Draw.io or the MCP server with Docker.
- Authoring diagrams of subsystems other than the architecture overview (Pipeline DAG lifecycle, plugin model, cluster topology are deferred).
- Auto-syncing the diagram with `CONTEXT.md` (only manual + reviewer-driven sync via PR convention).

## File layout

```
/Users/shaw/Developer/rust/bee/
└── docs/
    └── diagrams/                                  (new directory)
        ├── bee-architecture-overview.drawio       (the deliverable)
        └── README.md                              (how to open / how to edit via MCP)

/Users/shaw/Developer/rust/bee/
└── scripts/
    └── validate-drawio.py                         (one-shot XML structure validator)

~/                                                         (global)
└── .config/opencode/
    └── opencode.jsonc                             (append mcp.drawio section)
```

`.drawio` files are plain UTF-8 XML, so they should be tracked by git. No `.gitignore` change is required.

## opencode MCP configuration

Append to `~/.config/opencode/opencode.jsonc` without touching the existing `plugin` field:

```jsonc
{
  "plugin": ["superpowers@git+https://github.com/obra/superpowers.git"],
  "mcp": {
    "drawio": {
      "type": "local",
      "command": ["npx", "-y", "drawio-mcp"],
      "enabled": true
    }
  }
}
```

The MCP server runs on demand via `npx`; it is only invoked when the AI calls a Draw.io tool. The actual package name is `drawio-mcp` (verified against the npm registry; `@hediet/drawio-mcp` returns 404). opencode's MCP `type` is `"local"` (not `"stdio"`) and `command` is a single string array (no separate `args` field) — verified against the opencode schema at `https://opencode.ai/docs/mcp-servers`.

## Diagram contents

The diagram is sized to fit a single screen. It contains ~13 named boxes and ~15 labelled edges, grouped into three rows plus two external actors.

### External actors (outside the cluster boundary)

| Box | Represents |
|---|---|
| **User** | Human operator submitting Pipelines, running the Bee Client |
| **Plugins Directory** | On-disk folder of `.so` / `.dylib` plugin crates loaded by each node at startup |
| **External Systems** | Third-party endpoints a Datasource connects to (Binance, MongoDB, ...) |
| **Bee Client (Tauri)** | The `app/` Tauri 2.x + React desktop GUI |

### Bee Cluster (one large boundary box, "Bee Cluster (5× Nodes)")

Inside the cluster, six logical regions:

| Region | Box | Responsibility |
|---|---|---|
| RPC entry | **AdminServer** | Receives Bee Client RPC: `Deploy`, `RegisterDatasource`, `list_jobs`, `cluster_status`, ... |
| Consensus family | **Control Plane** | Membership, ownership, orphan detection, work-stealing arbitration |
| Consensus family | **Raft Cluster** | Quorum commit; backs both Control Plane and KV Cluster |
| Consensus family | **KV Cluster** | Task State, Checkpoints, Saved Offsets (second logical state machine on the same Raft cluster) |
| Data plane | **Data Plane (BRP)** | Peer-to-peer Phase↔Phase data, remote Handler invocations, I/O flows |
| Workload | **Pipeline Job** (representative) | Phase A → Phase B → Phase C, each Phase invokes a Handler |
| I/O exit | **Datasources** (Providers + Adapters) | Managed Providers wrapping one Adapter each; reaches External Systems |

### Edges (grouped by semantic family)

1. **Client path**
   - User → Bee Client
   - Bee Client → AdminServer (RPC)

2. **Raft path** (one cluster, two state machines)
   - AdminServer → Raft Cluster (propose)
   - Raft Cluster → Control Plane (apply membership / ownership changes)
   - Raft Cluster → KV Cluster (apply Task State / Checkpoint / Offset writes)

3. **Control ↔ data**
   - Control Plane → Data Plane (orphan detection signals, work-stealing arbitration)
   - Control Plane → Pipeline Job (assigns Phase Assignment = Task)
   - Pipeline Job → Data Plane (emit / consume Phase↔Phase data, invoke Handlers)

4. **Data egress**
   - Pipeline Job → Data Plane → Datasources → External Systems
   - Datasources ↔ KV Cluster (read / write checkpoints and saved offsets)

5. **Plugin loading**
   - Plugins Directory → nodes (loaded at startup; Adapters and Handlers registered to the local Registry)

### Visual conventions

- Big outer box = "Bee Cluster (5× Nodes)", light grey background.
- Consensus family (Control Plane / Raft Cluster / KV Cluster) — one accent colour.
- Data Plane — second accent colour.
- Pipeline Job — lightest fill (most variable content).
- External actors — plain borders, no fill emphasis.

## Workflows

### Production flow (this session)

```
CONTEXT.md ──► author assembles ──► docs/diagrams/bee-architecture-overview.drawio
CONTEXT.md ──► author writes  ──► docs/diagrams/README.md
schema rules ─► author writes ──► scripts/validate-drawio.py
opencode.jsonc plan ─► user edits ─► ~/.config/opencode/opencode.jsonc
```

The first deliverable is the on-disk `.drawio` file. The MCP path is not invoked in this session because `opencode.jsonc` was already loaded when the session started; MCP servers typically require a session restart to register.

### Iteration flow (future sessions via MCP)

```
┌─────────────────────────┐    WebSocket    ┌──────────────────────────┐
│ OpenCode (AI)           │◄───────────────►│ Draw.io (browser/desktop)│
│ + drawio-mcp            │   MCP over stdio│ Extras → Enable MCP      │
│ (stdio MCP server)      │                 │ (built-in WS server)     │
└─────────────────────────┘                 └──────────────────────────┘
```

User-side steps (documented in `docs/diagrams/README.md`):

1. Open Draw.io in a browser (`https://app.diagrams.net`) or the desktop app.
2. Draw.io menu → Extras → Enable MCP Server (starts a local WebSocket, default port ~6006).
3. Confirm `opencode.jsonc` contains the `mcp.drawio` section; restart opencode so the server is registered.
4. Ask the AI to read / edit the diagram; the AI calls MCP tools (`add_shape`, `add_connection`, `read_diagram`, `search_diagram`, `apply_template`); Draw.io updates live.

### Direct open (no MCP required)

Any of:

- Browser: navigate to `https://app.diagrams.net/?p=local&url=file:///Users/shaw/Developer/rust/bee/docs/diagrams/bee-architecture-overview.drawio`.
- macOS: `open -a "draw.io" docs/diagrams/bee-architecture-overview.drawio` (requires the desktop app to be installed).
- Draw.io menu: File → Open From → Device → pick the file.

### Version conventions

- Commit prefix: `docs(diagrams): ...`.
- A comment cell at the top of the `.drawio` file records `Last synced with CONTEXT.md on YYYY-MM-DD`. Update the date on every PR that changes either side.
- If a PR touches `CONTEXT.md` architecture wording, the reviewer is expected to check whether the diagram is still accurate and require a matching `.drawio` change in the same PR if not.

## Failure modes and recovery

| Phase | Failure | Detection | Recovery |
|---|---|---|---|
| XML generation | Malformed XML (unescaped chars, unclosed tags) | `xmllint --noout` exit code | Fix inline; regenerate |
| XML generation | Wrong `mxfile` / `mxGraphModel` structure | `scripts/validate-drawio.py` exit code | Same as above |
| XML generation | Duplicate cell IDs or edge references to non-existent source/target | `scripts/validate-drawio.py` | Correct the ID map |
| MCP startup | `npx` not on `PATH` | opencode startup log | Install Node.js (out of task scope; assumed available on this machine) |
| MCP startup | Wrong npm package name | npm 404 in stderr | Verify against `npm view <name>`; update `args` and the README |
| MCP connection | Draw.io MCP not enabled (no Extras → Enable MCP) | AI tool calls time out / `connection refused` | Tell user to enable MCP in Draw.io |
| MCP connection | WebSocket port mismatch (Draw.io default 6006 is configurable) | Same as above | Pass `DRAWIO_MCP_PORT` via `mcp.drawio.env` if needed |
| Open | User opens with non-Draw.io tool | Visual blank | README lists three supported opens |
| Drift | Diagram diverges from `CONTEXT.md` | The `Last synced` date in the diagram comment cell | Same-PR sync enforced by reviewer |

## Verification

After generation or modification, run all three:

1. **XML well-formedness**
   ```bash
   xmllint --noout docs/diagrams/bee-architecture-overview.drawio
   ```

2. **Draw.io structure validation** via `scripts/validate-drawio.py`:
   - Root element must be `<mxfile>`.
   - At least one `<diagram>` containing an `<mxGraphModel>`.
   - All cell `id` attributes are unique.
   - Every edge's `source` and `target` reference an existing cell ID.
   - Total cell count ≥ 10 (catches "half-drawn" submissions).
   - Cell label texts contain the keywords listed in §"Diagram contents" (catches missing or renamed boxes).

3. **Visual spot check**
   - Author self-check: before committing, verify every cell label matches the §"Diagram contents" table.
   - User spot check (in-session): author asks the user to open the file in Draw.io and confirm layout once.

Optional (deferred): PNG export via the `drawio` CLI for README embedding. Not in this iteration.

## Acceptance criteria

- [ ] `docs/diagrams/bee-architecture-overview.drawio` exists.
- [ ] `xmllint --noout` on the file passes.
- [ ] `python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio` exits 0.
- [ ] `docs/diagrams/README.md` exists and covers: the four-step MCP edit flow, three direct-open paths, the version conventions, and the package-name verification step.
- [ ] `scripts/validate-drawio.py` exists and runs without external dependencies beyond the Python standard library.
- [ ] `~/.config/opencode/opencode.jsonc` contains an `mcp.drawio` section with `type = "local"` and `command = ["npx", "-y", "drawio-mcp"]` (the opencode MCP schema, verified against `https://opencode.ai/docs/mcp-servers`).
- [ ] The diagram contains a cell whose label text matches each of the box names listed in §"Diagram contents" (validator-enforced by keyword check; keywords: `User`, `Plugins`, `Bee Client`, `AdminServer`, `Control Plane`, `Raft Cluster`, `KV Cluster`, `Data Plane`, `Pipeline Job`, `Phase`, `Handler`, `Datasources`, `External Systems`).
- [ ] Every edge's `source` and `target` reference a valid existing cell (validator-enforced).
- [ ] A comment cell at the top of the diagram records `Last synced with CONTEXT.md on 2026-08-12`.

## Out of scope

- Auto-regenerating the diagram from `CONTEXT.md` on every change (no parser for the prose yet).
- PNG / SVG export step in CI.
- Headless visual regression test for diagram layout.
- Adding a second diagram (Pipeline DAG lifecycle, plugin model, cluster topology) — deferred to a follow-up spec once the overview proves out the workflow.
- Multi-cluster Draw.io profile (only one profile is needed for one diagram).
