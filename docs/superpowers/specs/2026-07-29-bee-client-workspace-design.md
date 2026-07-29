# Bee Client Workspace and Control-Plane Management Design

**Date:** 2026-07-29
**Status:** Approved design
**Supersedes UI shape in:** `2026-07-28-s-tauri-gui-design.md`

## Purpose

Replace the current Compass-inspired four-tab Bee GUI with Bee Client: a resource-oriented workspace for managing a Bee Cluster and client-defined Applications. The design adds end-to-end management APIs, durable audit events, global search, Application migration, Pipeline structure visualization, configurable Dashboards, and safe cluster configuration rollout.

## Product terminology

- **Bee Client** is the Tauri desktop application. All existing “Bee GUI” labels become “Bee Client.”
- **Application** is a Bee Client grouping, not a Control Plane entity. It groups Dashboard layouts, Pipeline definitions and Jobs, and Datasource references in local SQLite.
- **Pipeline** is a static, named DAG of Phases.
- **Pipeline Job** is a running instance of a Pipeline.
- **Phase** is a Pipeline DAG vertex whose Handler transforms typed streams.
- **Datasource** is a managed Provider connection. Per-call arguments remain in Pipeline SQL rather than Datasource configuration.

## Delivery strategy

Implement the design as end-to-end vertical slices. Establish persistence, navigation, audit and typed APIs first, then deliver Settings, Application lifecycle, Datasources, Pipelines, Dashboards, search and Cluster management. Every slice must be usable and tested before the next slice begins.

## Architecture

Bee Client has three layers:

1. **React workspace** renders navigation, page tabs, dialogs, editors and live status.
2. **Tauri Rust application layer** owns SQLite, encrypted import/export, local settings, workspace restoration and typed IPC.
3. **AdminServer and Control Plane** own cluster settings, Plugins, Datasources, Pipeline definitions and Jobs, audit events, global cluster-resource search and rolling restart orchestration.

TanStack Query owns server state. Zustand owns temporary presentation state only. React components must not access `localStorage` or SQLite directly. Tauri repositories provide migrations and transactional access to persistent client data.

Long-running mutations return an Operation ID and stream progress. Mutations accept idempotency keys so retries cannot create duplicate Jobs, Datasources or restart operations. Live updates use subscriptions when available and fall back to bounded polling after a subscription failure.

## Application persistence

SQLite stores:

- Application identity, name, enabled state and display order;
- references to Pipeline definitions, Pipeline Jobs and Datasources;
- Dashboard definitions and panel layouts;
- the resource-state snapshot captured before Application disablement;
- open page tabs, selected tab and pinned tabs;
- Bee Client settings and connection profiles;
- import/export operation metadata.

Schema changes use explicit, forward-only migrations. Repository methods define transaction boundaries and return typed domain errors.

Application data remains local to Bee Client. AdminServer search covers cluster-owned resources and audit events; Bee Client concurrently searches local Application and Dashboard records, merges both result sets by relevance, and presents one navigable result list.

## Shell and navigation

The top-level navigation tabs are removed. The window is organized as:

- a left navigation tree;
- a right page-tab workspace;
- a fixed bottom activity and connection bar.

The left header displays **Bee** and a Settings icon. The icon replaces the current plus button and opens the Settings modal. Below it appear:

1. Cluster;
2. global search;
3. `Applications (count)` with an add button;
4. the expandable Application tree.

Every refreshable tree section has a refresh icon. Refresh invalidates and reloads only that section’s data, with visible progress and failure state.

Every tree node opens a page in the right workspace as a closable tab, including Cluster, Application, Dashboard, Pipelines, individual Pipeline, Datasources and individual Datasource nodes. Opening an already-open resource focuses its existing tab instead of duplicating it. Tabs support pin, close, close others and session restoration. Settings remains a modal and does not create a page tab.

When no Application exists, the workspace displays “No Applications yet” and a prominent Add Application action.

## Global search

Search reacts to text-change events with debounce and cancellation of stale requests. It searches:

- local Applications and Dashboards through SQLite;
- Cluster nodes and settings exposed for search;
- Pipeline definitions and Pipeline Jobs;
- Datasources and Plugins;
- audit events.

Results carry a resource type, title, contextual path, match highlights and navigation target. While search text is non-empty, the left navigation shows only matching nodes and enough ancestors to preserve context. Selecting a result opens or focuses its page tab. Empty, loading, partial-failure and no-result states are distinct.

## Application lifecycle

Application disablement is an orchestrated, recoverable operation:

1. capture running and queued Pipeline Jobs and connected Datasources;
2. stop or cancel the captured Jobs;
3. disconnect the captured Datasources;
4. persist the completed snapshot and disabled state.

Application enablement performs the inverse operation:

1. reconnect only Datasources that were connected before disablement;
2. test their health;
3. resubmit only Pipeline Jobs that were running or queued before disablement;
4. persist the enabled state and new Job identities.

The workflow is a durable state machine. It is idempotent, resumes after Bee Client restart, and exposes per-resource progress. Partial failure yields `Degraded`, not success, and offers targeted retry. Every step emits audit events.

## Application import and export

Application pages provide Import and Export actions for migration and recovery.

An export is a versioned Bee Application package containing the complete Application structure, Dashboard layouts, Pipeline definitions and references, Datasource definitions and credentials, metadata, and integrity information. The package is never emitted in plaintext. A user passphrase is processed by a memory-hard password KDF with stored salt and parameters; the resulting key encrypts the package with an authenticated-encryption algorithm. Plaintext keys and decrypted package contents are not persisted.

Import performs these stages before mutation:

1. authenticate and decrypt;
2. verify package checksum and supported schema version;
3. validate referenced Plugin and Adapter compatibility;
4. display a conflict preview;
5. let the user overwrite, duplicate with a new name, or skip each conflict;
6. apply the import transactionally.

A failed import rolls back local changes and any server resources created by the operation where possible. Incomplete compensation is reported as `Degraded` with direct navigation to affected resources.

## Settings modal

Settings uses a two-column modal patterned after the supplied reference: category navigation on the left and the selected form on the right. Categories are:

- Client;
- Connection;
- Appearance;
- Logging;
- Diagnostics;
- Cluster;
- Raft;
- KV;
- Scheduling;
- Plugins;
- Security.

All editable settings auto-save. Field changes use validation, debounce and serialized writes, and display `Saving`, `Saved` or `Error`. There is no general Save button.

Connection String is persisted as a draft automatically, but changing it does not silently switch the active cluster. The Connection section exposes exactly two actions:

- **Test Connection** validates reachability and protocol compatibility without changing the active connection;
- **Connect** activates the draft connection and begins reconnect handling.

Cluster settings are versioned. Dynamically mutable fields apply after quorum commit. Fields requiring restart initiate a safe rolling restart: restart healthy Followers one at a time, wait for each to rejoin and catch up, transfer leadership when required, preserve quorum at every step, and stop immediately on a failed health gate. The operation is resumable and fully audited.

## Connection status

The bottom bar shows both color and text:

- red dot: disconnected or failed;
- solid green dot: connected;
- pulsing green dot: connecting or reconnecting.

The status includes the active connection string and accessible text; color is not the sole indicator. Detailed connection errors include a link that opens Settings at the Connection category.

## Audit activity

Control Plane mutations append durable audit events as part of the same Raft-committed state transition. An audit event contains:

- timestamp and actor;
- action and result;
- safe summary and structured technical details;
- resource type and stable resource identifier;
- related Application identifier when Bee Client supplies one;
- correlation and Operation IDs;
- a structured navigation target.

Sensitive fields are redacted before proposal and never enter audit storage, logs or search indexes. If the required audit append cannot commit, the associated Control Plane mutation does not commit.

The bottom bar displays the latest audit-event summary. Clicking it opens the activity dialog, which supports refresh, pagination and filters. Selecting an event opens its details. “Go to related page” opens or focuses the target page tab; a connection error targets Settings → Connection, a Datasource error targets its Datasource page, and a Pipeline Job error targets its Job detail.

## Cluster page

Cluster is the first navigation item and opens a Cluster Dashboard page tab. It displays:

- topology, Nodes, Raft Leader and quorum health;
- queued, running, historical and failed Pipeline Jobs;
- longest-running Pipeline Jobs;
- highest resource-consuming Pipeline Jobs;
- average runtime and resource trends;
- active configuration and rolling-restart operations.

Metrics name Pipeline definitions and Pipeline Jobs precisely. Topology does not describe the hybrid Bee architecture as a purely decentralized network.

## Application Dashboard

Each Application has configurable Dashboards. A Dashboard uses a draggable and resizable grid similar in capability to Grafana without copying its visual design. Panels subscribe to the latest results required from Pipeline Jobs or Phase outputs and expose live, paused, stale, loading and error states.

Dashboard definitions persist in SQLite. Runtime data travels through existing Bee Data Plane mechanisms or a purpose-built read subscription API; it is not stored as business data in Task State.

## Pipelines pages

An Application Pipelines page separates:

- Pipeline definitions;
- queued Pipeline Jobs;
- running Pipeline Jobs;
- historical Pipeline Jobs;
- failed Pipeline Jobs.

Users can create and edit a Pipeline in its own page tab. Pipeline and Pipeline Job labels remain distinct throughout the UI.

### Pipeline detail structure graph

Pipeline detail includes a complete interactive structure graph rather than a separate dependencies page. It visualizes:

`Input Datasource/Adapter → Phase Handler(s) → Output Datasource/Adapter`

It also shows upstream and downstream cross-Pipeline edges relevant to the selected Pipeline. BRP-backed cross-Pipeline channels are visually distinguished from in-process edges.

Interactions are:

- clicking an Input or Output opens a right-side detail drawer for its Datasource, Adapter, connection status and safe configuration fields;
- hovering a Handler displays a tooltip with Handler and parameter summaries;
- clicking a Handler opens Phase, Handler, Plugin version, parameters, runtime metrics and error details;
- clicking a referenced Pipeline opens or focuses that Pipeline’s detail in a new page tab.

The graph switches between definition structure and runtime status. It highlights failed Phases, disconnected Datasources, missing Plugins and failing cross-Pipeline channels. Pan, zoom, keyboard navigation, fit-to-view and accessible non-graph alternatives are required.

## Datasources pages

Each Application Datasources page lists its referenced managed Providers and provides Add Datasource. Creation opens a modal that fetches the latest Plugin Registry state. Selecting a Plugin and Adapter loads its configuration schema and generates validated connection-level fields.

Per-call arguments such as symbols, intervals and query strings are excluded from Datasource configuration and remain at Pipeline call sites.

The form provides:

- **Test Connection** to validate configuration without persistence;
- **Connect and Save** to create the Datasource, connect it and add its reference to the Application.

Credentials use secret controls, are redacted from diagnostics and logs, and are not retained in React Query caches longer than required.

## Server and IPC surface

Typed protocol additions include:

- Pipeline definition CRUD, submit, stop, cancel, resume and structure inspection;
- Datasource CRUD, connect, disconnect and connection test;
- Plugin Registry listing, Adapter schemas and compatibility checks;
- Cluster settings read/update, configuration versions and rolling restart;
- audit-event query and subscription;
- cluster-resource global search;
- operation progress and cancellation where safe;
- live Dashboard result subscriptions.

Tauri IPC additions include SQLite-backed Application, Dashboard, workspace and settings repositories; encrypted import/export; merged search; and orchestration methods that combine local Application state with AdminServer mutations.

## Error handling

User-visible errors contain:

- a concise summary;
- expandable technical details;
- retry or recovery actions where safe;
- correlation and Operation IDs;
- a related-page navigation target;
- the corresponding audit event.

Optimistic UI is limited to reversible presentation state. Control Plane mutations display pending state until quorum commit. Application lifecycle, import, configuration rollout and rolling restart are explicit state machines that survive process interruption.

## Security

- Credentials and connection secrets are never written to ordinary logs, audit details, search indexes or unencrypted export files.
- Import packages use authenticated encryption and a memory-hard passphrase KDF.
- Secret inputs minimize browser-side retention and are cleared after use.
- Diagnostic exports apply central redaction.
- Tauri capabilities grant only the filesystem, dialog and cryptographic access required by these workflows.
- The disabled CSP is replaced with a restrictive production policy before release.

## Testing and CI

Rust tests cover:

- SQLite migrations and repository transactions;
- Application lifecycle state-machine recovery and idempotency;
- encrypted package round trips, wrong passphrases, tampering and schema migration;
- AdminServer protocol additions;
- audit-event atomicity and redaction;
- global search and pagination;
- configuration versioning and quorum-safe rolling restart;
- Pipeline structure-graph serialization.

Frontend tests cover:

- navigation tree filtering and resource counts;
- page-tab deduplication, pinning and restoration;
- Settings auto-save and connection actions;
- connection-state indicators;
- audit dialog details and navigation;
- Application empty, enabling, disabling and degraded states;
- import conflict flow;
- schema-driven Datasource forms;
- Pipeline graph interactions and accessible fallback;
- merged local/server search, stale request cancellation and partial failures.

Tauri end-to-end tests cover connection switching, Application migration, resource lifecycle orchestration, audit navigation and workspace restoration.

CI runs formatting, Clippy, Rust tests, TypeScript typecheck, frontend tests, frontend build and supported Tauri build checks.

## Acceptance criteria

- The product and window identify as Bee Client; the shell header says Bee.
- No legacy top-level Welcome, Data Sources, Pipelines or Settings tabs remain.
- Every tree resource opens a deduplicated, closable page tab; Settings opens a modal.
- The navigation shows Cluster first, Application count, add action, refresh actions and the required empty state.
- Global text-change search merges complete server results with local Application and Dashboard results and produces navigable matches.
- Application disable/enable restores only the previously active resources and reports partial failures accurately.
- Encrypted Application export/import supports complete credential-preserving migration and conflict handling.
- Settings auto-save; Connection has Test Connection and Connect but no Save button.
- Restart-required cluster settings use quorum-safe rolling restart with progress and audit history.
- The connection indicator accurately distinguishes disconnected, connected and connecting/reconnecting states.
- The bottom bar shows the latest durable audit event and opens navigable event details.
- Cluster Dashboard exposes topology and requested Pipeline Job operational metrics.
- Application Dashboards support configurable layouts and live Pipeline-derived results.
- Datasource creation uses current Plugin/Adapter schemas and provides Test Connection and Connect and Save.
- Pipeline detail renders complete Input → Handler(s) → Output structure and navigable cross-Pipeline dependencies.
- Automated tests and CI cover all critical state transitions and interactions.

## Out of scope

- Treating Application as a Raft-backed Control Plane entity;
- storing Dashboard business data in Task State or the KV Cluster;
- simultaneous active connections to multiple Bee Clusters;
- plaintext export of credentials;
- duplicating the same resource in multiple page tabs.

## Deferred items — resolution status

The original design deferred a few items to post-1.0. As of 2026-07-30:

- **Tenant enforcement (ADR-0010)** — _resolved_ via the `tenant.rs` module, `applications.tenant` migration v9, and `tenant_get` / `tenant_set` commands. Application creation now accepts a tenant (defaults to the active tenant from `client_settings["tenant"]`); the Settings modal exposes a Tenant section with debounced save and 0..=65535 validation; the NavTree Application creation form requires a tenant.
- **Multi-cluster saved connections** — _resolved_ via migration v10 (`cluster_profiles` table with `id PK, label, addr UNIQUE, tenant, last_used_at, created_at`), `db/clusters.rs` repo (`list` / `save` / `remove` / `set_active` / `get_active`), and `cluster_profile_*` commands. The new `ClusterProfilesSidebar` in `NavTree` lists, activates and removes profiles; `cluster_profile_activate(addr)` parses the addr, calls `connection::ensure_bundle` to swap the live connection, and persists the addr to `client_settings["addr"]`. Legacy `bee-gui.connections` localStorage entries are migrated once at first run by `cluster_profile_migrate_legacy`.
