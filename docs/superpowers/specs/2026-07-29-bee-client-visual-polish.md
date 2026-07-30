# Bee Client Visual Polish Slice

**Date:** 2026-07-29
**Status:** Approved
**Related:** `2026-07-29-bee-client-workspace-design.md` (foundation design)

## Purpose

Capture visual refinements decided during interactive Tauri verification. These are wireframe-approved changes that ship on top of the existing foundation slice.

## Scope

### 1. Settings modal: Connection and Cluster merged

The Settings modal opens on the **Connection** category by default. Connection and Cluster are **merged into a single category** with three zones:

- **Saved clusters list** at the top. Each row shows a status dot (green = active+healthy, amber = saved-but-not-active, red = last-error) plus label + addr + edit/remove. Active row is highlighted. An `+ Add cluster` action sits below the list.
- **AdminServer address** input below the list, pre-filled with the active cluster's addr.
- **Test Connection** and **Connect** buttons below the address.

Selecting a different cluster row pre-fills the address. Clicking Connect switches the global AdminClient bundle and closes the modal. Test Connection does not change the active connection.

The other 10 categories (Tenant, Appearance, Logging, Diagnostics, Raft, KV, Scheduling, Plugins, Security) keep placeholder content but are real, navigable items in the left list. Clicking a category highlights it and swaps the right pane. The list scrolls independently when needed.

### 2. Top header layout (single row)

The shell header holds everything in one 36-42px tall row, left to right:

- **Bee** brand label (leftmost).
- **Cluster dropdown** (Node selector) showing the active cluster with a status dot + label + addr + caret. Clicking opens a read-only list of saved profiles. Selecting a profile calls `cluster_profile_activate` to switch the bundle.
- **Refresh** icon.
- **Settings** cog (opens Settings modal).
- **Theme** toggle (Moon/Sun).

The left sidebar **loses its Bee header** (moved up) and the dedicated **Clusters section** (moved into header dropdown + Settings → Cluster).

### 3. Cluster dropdown is read-only

The header cluster dropdown does NOT include `+ Add new cluster`. Add lives only in Settings → Cluster. Each row in the dropdown shows:

- Status dot (green / amber / red)
- Label
- Addr + tenant (`t0` suffix)
- Active row highlighted

### 4. Pipeline page overflow fix

The Pipelines page main container uses `min-w-0` and an `overflow-x-auto` wrapper so the right-side action buttons (`+ New Pipeline` / `+ Create pipeline`) are never clipped by the workspace.

### 5. Page tab right-click context menu

The "Close other tabs" and "Toggle pinned" sidebar buttons are removed. Right-clicking a page tab opens a small floating context menu with: **Close**, **Close Others**, **Pin / Unpin**, **Close to the Right**. Click-outside or Escape closes the menu.

### 6. Grafana-style customizable Dashboard

The Application Dashboard supports a layout editor. Panels subscribe to Pipeline Job latest results. Users can:

- Add panels (K-line chart, Active jobs stat, Tasks/sec stat, CPU usage, Pipeline status, Audit feed, Cluster topology, etc.)
- Drag panels to reposition
- Resize panels via bottom-right handle
- Remove panels via header context menu
- Edit mode shows dashed outlines; view mode shows clean chrome

Layout persists per application_id in a new SQLite `dashboards` table (migration v11). Default layout: 6-panel grid (K-line 2x2 + Active jobs + Tasks/sec + CPU + Pipeline status 2x1).

### 7. Cluster topology visual graph

Replaces the text-only Topology card. Render rules:

- **Layout**: symmetric ring with leader chassis centered. Followers placed 60° apart on a 100px radius arc. 20px grid backdrop.
- **Node icon**: 20x24 px rectangle, 4 vent-line stripes, no internal labels.
- **Status dot**: 10px circle positioned 4px to the LEFT of the chassis.
- **Leader**: 70x40 chassis (3.5x follower size), amber border + pulsing amber LED, "LEADER" label inside.
- **Edges**: orthogonal L-shape (horizontal + vertical segments), arrow marker in matching color.
- **Edge colors**: green = healthy replication, amber = in-flight vote, red dashed = down.
- **Down chassis**: dims to gray, red X overlay.
- **Empty state**: faint static ghost topology + "No Bee cluster reachable" + Select cluster button.
- **Tooltip**: on hover shows Node id, addr, term, role, lag, last error.

## Constraints

- No source-code comments per repo rule.
- Preserve existing tests.
- Frontend-only changes (no crates/ modifications).
- All new UI flows through the typed IPC + Zustand stores already in place.

## Acceptance criteria

- Top header row shows Bee + cluster dropdown + refresh + settings + theme, in that left-to-right order.
- Sidebar has no Bee header and no cluster list.
- `+ Add cluster` only exists in Settings → Cluster.
- Settings → Connection shows saved clusters above the address field.
- Cluster dropdown items show status dots, no Add action.
- Right-click on a page tab opens context menu with 4 actions; clicking outside closes it.
- Pipelines page never clips the right-side buttons at 1100x720.
- Application Dashboard supports Add / Drag / Resize / Remove panels.
- Topology graph renders symmetric ring with server icons + status dots + orthogonal edges.
- All existing tests still pass; new tests cover context menu, topology helpers, dashboard store.

## See also

- `2026-07-29-bee-client-workspace-design.md` — foundation design
- `2026-07-29-bee-client-foundation.md` — implementation plan
