# Bee Client Foundation Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Compass-shaped Bee GUI shell with the first independently shippable slice of the approved Bee Client design: test infrastructure, durable SQLite-backed client settings + workspace tabs, typed IPC, new shell/navigation, deduplicated closable/restorable page tabs, Settings modal with auto-save + Test Connection/Connect, accurate connection status indicator, and Bee GUI -> Bee Client labels.

**Architecture:**
- Frontend (React + Vite + TS + Tailwind): a single QueryClient owns server state; a typed `ipc/` module owns transport; Zustand stores own *only* transient presentation state (current active tab, theme, ConnectionStatus cache).
- Tauri Rust application layer: owns SQLite via `rusqlite` (bundled), forward-only migrations, typed `commands::*` modules, a typed `ConnStatus` enum that the frontend can match on.
- AdminServer + Control Plane: **unchanged** by this slice. `set_addr` and `get_default_addr` from the uncommitted working tree are preserved; they are reconciled to read/write the new SQLite `client_settings` table (with a one-time import from the legacy `settings.json`).

**Tech Stack:** Tauri 2.x, React 18, Vite 4, TypeScript 5, Tailwind 3, Zustand 4, TanStack Query 5, Vitest 1 + jsdom + @testing-library/react, rusqlite 0.31 (bundled), tokio 1.

**Scope discipline:**
- IN scope: frontend tests, SQLite + migrations for client settings + workspace tabs + connection profiles, typed IPC, new shell, page tabs, Settings modal (Client + Connection sections), connection status indicator, label rename.
- OUT of scope (later slices): Application lifecycle, Audit events, Global search, Cluster Dashboard beyond what's already there, Pipeline structure graph, Datasource forms, Import/export, Dashboard builder. The new Shell/NavTree/PageTabs are built so future slices plug into them, but they are not wired in this slice.

**Preservation guarantee:** No implementation file in `app/` or `app/src-tauri/` is modified by this plan until the corresponding task explicitly says so. The uncommitted `app/src-tauri/src/commands.rs` + `connection.rs` + `lib.rs` + `app/src/ipc.ts` + `state/store.ts` + `pages/{Dashboard,Settings}.tsx` work is **reconciled, not overwritten**: the env-var fallback `get_default_addr` survives; `set_addr` keeps its addr-switching semantics but now persists into SQLite (with a one-shot import from the existing `settings.json`).

---

## File structure (final shape, after this plan)

```
app/
├── package.json                                    # MODIFY: add vitest, jsdom, RTL, scripts
├── tsconfig.json                                   # MODIFY: add "vitest/globals" types
├── vite.config.ts                                  # MODIFY: add test block
├── index.html                                      # MODIFY: title "Bee Client"
├── src/
│   ├── App.tsx                                     # MODIFY: render <Shell> + open Settings modal route
│   ├── main.tsx                                    # MODIFY: keep QueryClient, no behavior change
│   ├── ipc.ts                                      # DELETE (moved into ipc/)
│   ├── ipc/
│   │   ├── index.ts                                # NEW: re-exports
│   │   ├── shared.ts                               # NEW: ConnStatus union + helpers
│   │   ├── cluster.ts                              # NEW: clusterStatus, listJobs, jobInspect
│   │   ├── connection.ts                           # NEW: getDefaultAddr, setAddr, testConnection, connState
│   │   ├── settings.ts                             # NEW: settingsGet, settingsPut, settingsList
│   │   ├── tabs.ts                                 # NEW: tabsList, tabOpen, tabClose, tabSetActive, tabPin, tabUnpin
│   │   ├── profiles.ts                             # NEW: profilesList, profileSave, profileRemove
│   │   └── ping.ts                                 # NEW: ping
│   ├── pages/
│   │   ├── ClusterDashboard.tsx                    # RENAME from Dashboard.tsx (kept for slice); body unchanged
│   │   ├── DataSources.tsx                         # PRESERVE (kept on disk for later migration; unreferenced for now)
│   │   ├── Pipelines.tsx                           # PRESERVE (kept on disk for later migration; unreferenced for now)
│   │   └── Settings.tsx                            # DELETE (moved to modal)
│   ├── components/
│   │   ├── AppShell.tsx                            # DELETE (replaced by Shell.tsx)
│   │   ├── AppBar.tsx                              # DELETE (replaced by Shell.tsx)
│   │   ├── Shell.tsx                               # NEW: left nav + right page tabs + bottom bar
│   │   ├── NavTree.tsx                             # NEW: left navigation tree
│   │   ├── PageTabs.tsx                            # NEW: closable/restorable/pinned page tabs
│   │   ├── StatusBar.tsx                           # REWRITE: bottom bar (connection status; audit summary deferred)
│   │   ├── ConnectionStatus.tsx                    # NEW: red/solid-green/pulsing-green + text + error link
│   │   └── SettingsModal.tsx                       # NEW: 2-column modal, auto-save + Test Connection/Connect
│   ├── state/
│   │   ├── store.ts                                # REWRITE: theme + UI-only state (no IPC); useSettingsTabs/etc removed
│   │   ├── tabsStore.ts                            # NEW: transient tab state
│   │   ├── connectionStore.ts                      # NEW: cached ConnStatus from polling
│   │   ├── settingsUiStore.ts                      # NEW: per-field Saving/Saved/Error state
│   │   └── tooltip.ts                              # DELETE (was Tab helper; tooltip stays inline)
│   └── tests/                                      # NEW
│       ├── setup.ts                                # NEW: vitest setup (jsdom, RTL cleanup)
│       ├── state/
│       │   ├── tabsStore.test.ts                   # NEW
│       │   ├── connectionStore.test.ts             # NEW
│       │   └── settingsUiStore.test.ts             # NEW
│       ├── ipc/
│       │   ├── shared.test.ts                      # NEW: ConnStatus helpers
│       │   └── connection.test.ts                  # NEW: arg validation
│       └── components/
│           ├── PageTabs.test.tsx                   # NEW
│           ├── ConnectionStatus.test.tsx           # NEW
│           ├── SettingsModal.test.tsx              # NEW
│           ├── NavTree.test.tsx                    # NEW
│           └── Shell.test.tsx                      # NEW
└── src-tauri/
    ├── Cargo.toml                                  # MODIFY: add rusqlite + tempfile (dev) + tests wired
    ├── build.rs                                    # UNCHANGED
    ├── capabilities/default.json                   # UNCHANGED (core:default is enough for foundation)
    ├── tauri.conf.json                             # MODIFY: productName "Bee Client", window title "Bee Client"
    └── src/
        ├── main.rs                                 # UNCHANGED
        ├── lib.rs                                  # REWRITE: init db on startup, migrate settings.json, register handlers
        ├── connection.rs                           # MODIFY: persist addr via SettingsRepo (not fs::write); add conn_status enum + subscribe; reconcile uncommitted tokio::select loop
        ├── settings_io.rs                          # NEW: one-shot import from legacy settings.json + tests
        ├── db/                                     # NEW
        │   ├── mod.rs                              # NEW: Database struct + migrations runner + tests
        │   ├── settings.rs                         # NEW: SettingsRepo (typed key/value) + tests
        │   ├── tabs.rs                             # NEW: TabsRepo + tests
        │   └── profiles.rs                         # NEW: ProfilesRepo + tests
        └── commands/                               # REWRITE: split into typed submodules
            ├── mod.rs                              # NEW: re-exports + invoke_handler list
            ├── cluster.rs                          # NEW: cluster_status, list_jobs, job_inspect (preserved)
            ├── ping.rs                             # NEW: ping (preserved)
            ├── connection.rs                       # NEW: get_default_addr (preserved), set_addr (reconciled), test_connection (NEW), conn_state (typed)
            ├── settings.rs                         # NEW: settings_get/put/list/delete
            ├── tabs.rs                             # NEW: tab_open/close/set_active/pin/unpin/list
            └── profiles.rs                         # NEW: profile_save/list/remove
```

---

## Task 1: Frontend test infrastructure (Vitest + RTL + jsdom)

**Files:**
- Modify: `app/package.json`
- Modify: `app/vite.config.ts`
- Modify: `app/tsconfig.json`
- Create: `app/src/tests/setup.ts`
- Create: `app/src/tests/smoke.test.ts`

- [ ] **Step 1.1: Install test deps**

Run:
```bash
npm install --save-dev --no-audit --no-fund \
  vitest@^1.6.0 \
  jsdom@^24.0.0 \
  @testing-library/react@^16.0.0 \
  @testing-library/jest-dom@^6.4.0 \
  @testing-library/user-event@^14.5.0
```
Expected: `app/package.json` `devDependencies` gains the four packages + transitive deps; `npm ls vitest jsdom @testing-library/react` prints each at the expected version.

- [ ] **Step 1.2: Add the `test` script and configure Vitest in vite.config.ts**

Replace `app/vite.config.ts` with:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/tests/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    css: false,
  },
});
```

Add to `app/package.json` `scripts`:
```json
"test": "vitest run",
"test:watch": "vitest",
"test:coverage": "vitest run --coverage"
```

- [ ] **Step 1.3: Add jest-dom matcher types to tsconfig**

Modify `app/tsconfig.json` `compilerOptions.types`:
```json
"types": ["@testing-library/jest-dom"]
```

Test files import `describe`/`it`/`expect` from `"vitest"` directly, so the `vitest/globals` reference is not needed.

- [ ] **Step 1.4: Write the failing setup + smoke test**

Create `app/src/tests/setup.ts`:
```ts
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";
afterEach(() => cleanup());
```

Create `app/src/tests/smoke.test.ts`:
```ts
import { describe, it, expect } from "vitest";
describe("vitest harness", () => {
  it("runs", () => {
    expect(1 + 1).toBe(2);
  });
});
```

- [ ] **Step 1.5: Run the test, expect PASS**

Run: `npm test`
Expected: prints `1 passed`, exits 0.

- [ ] **Step 1.6: Commit**

```bash
git add app/package.json app/package-lock.json app/vite.config.ts app/tsconfig.json app/src/tests/setup.ts app/src/tests/smoke.test.ts
git commit -m "test(app): wire vitest + jsdom + RTL; add smoke test"
```

---

## Task 2: Rust test smoke for `app/src-tauri`

**Files:**
- Modify: `app/src-tauri/Cargo.toml`
- Create: `app/src-tauri/src/lib.rs::tests` (inline module; preserves existing run() body untouched)

- [ ] **Step 2.1: Add `tempfile` as a dev-dependency**

Modify `app/src-tauri/Cargo.toml` `[dev-dependencies]` (add the section if absent):
```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2.2: Write a failing test that asserts the addr parser is reachable**

The `addr_parse` helper in `connection.rs` is the only pure function in the binary. We assert it round-trips and rejects garbage. Add this module at the end of `app/src-tauri/src/connection.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_parse_accepts_ipv4_host_port() {
        let addr = addr_parse("127.0.0.1:9999").expect("must parse");
        assert_eq!(addr.to_string(), "127.0.0.1:9999");
    }

    #[test]
    fn addr_parse_rejects_garbage() {
        assert!(addr_parse("not a socket").is_err());
        assert!(addr_parse("").is_err());
        assert!(addr_parse("9999").is_err());
    }
}
```

- [ ] **Step 2.3: Run the test, expect PASS**

Run: `cargo test -p app connection::tests::addr_parse_accepts_ipv4_host_port -p app connection::tests::addr_parse_rejects_garbage`
Expected: `2 passed; 0 failed`.

- [ ] **Step 2.4: Commit**

```bash
git add app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/src/connection.rs
git commit -m "test(app-tauri): add addr_parse unit tests"
```

---

## Task 3: SQLite Database + migrations runner

**Files:**
- Modify: `app/src-tauri/Cargo.toml`
- Create: `app/src-tauri/src/db/mod.rs`

- [ ] **Step 3.1: Add `rusqlite` with the `bundled` feature**

Modify `app/src-tauri/Cargo.toml` `[dependencies]`:
```toml
rusqlite = { version = "0.31", features = ["bundled"] }
```

Run: `cargo build -p app` — expect it compiles after fetching rusqlite + libsqlite3-sys.

- [ ] **Step 3.2: Write the failing test for Database::open + empty migrations**

Create `app/src-tauri/src/db/mod.rs`:
```rust
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

pub mod settings;
pub mod tabs;
pub mod profiles;

pub struct Database {
    conn: Mutex<Connection>,
    path: PathBuf,
}

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "client_settings",
        sql: r#"
            CREATE TABLE client_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
        "#,
    },
    Migration {
        version: 2,
        name: "workspace_tabs",
        sql: r#"
            CREATE TABLE workspace_tabs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                resource_id TEXT,
                title TEXT NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                position INTEGER NOT NULL,
                opened_at INTEGER NOT NULL,
                closed_at INTEGER
            );
            CREATE UNIQUE INDEX idx_workspace_tabs_open_unique
                ON workspace_tabs(kind, resource_id) WHERE closed_at IS NULL;
            CREATE TABLE workspace_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                active_tab_id INTEGER REFERENCES workspace_tabs(id)
            );
        "#,
    },
    Migration {
        version: 3,
        name: "connection_profiles",
        sql: r#"
            CREATE TABLE connection_profiles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                label TEXT NOT NULL,
                addr TEXT NOT NULL UNIQUE,
                last_used_at INTEGER,
                created_at INTEGER NOT NULL
            );
        "#,
    },
];

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();
        let db = Self { conn: Mutex::new(conn), path: path.to_path_buf() };
        db.apply_migrations()?;
        Ok(db)
    }

    pub fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn.lock().map_err(|e| format!("db lock poisoned: {e}"))
    }

    pub fn path(&self) -> &Path { &self.path }

    pub fn apply_migrations(&self) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at INTEGER NOT NULL
            );",
        ).map_err(|e| format!("create migrations table: {e}"))?;
        let applied: Option<u32> = conn
            .query_row(
                "SELECT MAX(version) FROM migrations",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("max(version): {e}"))?;
        drop(conn);
        let next = applied.map(|v| v + 1).unwrap_or(1);
        for m in MIGRATIONS.iter().filter(|m| m.version >= next) {
            let conn = self.lock()?;
            let tx = conn.unchecked_transaction()
                .map_err(|e| format!("begin tx v{}: {e}", m.version))?;
            tx.execute_batch(m.sql)
                .map_err(|e| format!("apply v{} ({}): {e}", m.version, m.name))?;
            tx.execute(
                "INSERT INTO migrations (version, name, applied_at) VALUES (?, ?, ?)",
                params![m.version, m.name, now_secs()],
            ).map_err(|e| format!("record v{}: {e}", m.version))?;
            tx.commit().map_err(|e| format!("commit v{}: {e}", m.version))?;
        }
        Ok(())
    }

    pub fn applied_versions(&self) -> Result<Vec<u32>, String> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT version FROM migrations ORDER BY version")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt.query_map([], |row| row.get::<_, u32>(0))
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("collect: {e}"))
    }
}

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_file_and_applies_migrations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        assert!(path.exists());
        let applied = db.applied_versions().unwrap();
        assert_eq!(applied, vec![1, 2, 3]);
    }

    #[test]
    fn apply_migrations_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let first = db.applied_versions().unwrap();
        let db2 = Database::open(&path).unwrap();
        let second = db2.applied_versions().unwrap();
        assert_eq!(first, second);
        assert_eq!(first, vec![1, 2, 3]);
    }

    #[test]
    fn applied_versions_empty_when_db_just_created_with_no_migrations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.applied_versions().unwrap(), vec![1, 2, 3]);
    }
}
```
```

- [ ] **Step 3.3: Run the tests, expect PASS**

Run: `cargo test -p app db::`
Expected: `3 passed; 0 failed`.

- [ ] **Step 3.4: Commit**

```bash
git add app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/src/db/mod.rs
git commit -m "feat(app-tauri): SQLite database + forward-only migrations runner"
```

---

## Task 4: client_settings repository

**Files:**
- Create: `app/src-tauri/src/db/settings.rs`
- Modify: `app/src-tauri/src/db/mod.rs`

- [ ] **Step 4.1: Write the failing tests for the typed key/value repo**

Append to `app/src-tauri/src/db/mod.rs` (so the `Database` type stays the entry point):

Create `app/src-tauri/src/db/settings.rs`:
```rust
use rusqlite::{params, OptionalExtension};

use super::{now_secs, Database};

#[derive(Debug, Clone)]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
}

pub struct SettingsRepo<'a> {
    db: &'a Database,
}

impl<'a> SettingsRepo<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.db.lock()?;
        conn.query_row(
            "SELECT value FROM client_settings WHERE key = ?",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("get {key}: {e}"))
    }

    pub fn put(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute(
            "INSERT INTO client_settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now_secs()],
        ).map_err(|e| format!("put {key}: {e}"))?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute("DELETE FROM client_settings WHERE key = ?", params![key])
            .map_err(|e| format!("delete {key}: {e}"))?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Setting>, String> {
        let conn = self.db.lock()?;
        let mut stmt = conn.prepare(
            "SELECT key, value, updated_at FROM client_settings ORDER BY key",
        ).map_err(|e| format!("list prepare: {e}"))?;
        let rows = stmt.query_map([], |row| {
            Ok(Setting {
                key: row.get(0)?,
                value: row.get(1)?,
                updated_at: row.get(2)?,
            })
        }).map_err(|e| format!("list query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("list collect: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh_db() -> Database {
        let dir = tempdir().unwrap();
        Database::open(&dir.path().join("bee-client.sqlite")).unwrap()
    }

    #[test]
    fn get_missing_key_returns_none() {
        let db = fresh_db();
        let repo = SettingsRepo::new(&db);
        assert_eq!(repo.get("nope").unwrap(), None);
    }

    #[test]
    fn put_then_get_round_trips() {
        let db = fresh_db();
        let repo = SettingsRepo::new(&db);
        repo.put("addr", "127.0.0.1:9999").unwrap();
        assert_eq!(repo.get("addr").unwrap().as_deref(), Some("127.0.0.1:9999"));
    }

    #[test]
    fn put_overwrites_value() {
        let db = fresh_db();
        let repo = SettingsRepo::new(&db);
        repo.put("addr", "127.0.0.1:9999").unwrap();
        repo.put("addr", "10.0.0.1:10001").unwrap();
        assert_eq!(repo.get("addr").unwrap().as_deref(), Some("10.0.0.1:10001"));
    }

    #[test]
    fn delete_removes_key() {
        let db = fresh_db();
        let repo = SettingsRepo::new(&db);
        repo.put("addr", "x").unwrap();
        repo.delete("addr").unwrap();
        assert_eq!(repo.get("addr").unwrap(), None);
    }

    #[test]
    fn list_returns_all_rows_sorted_by_key() {
        let db = fresh_db();
        let repo = SettingsRepo::new(&db);
        repo.put("theme", "dark").unwrap();
        repo.put("addr", "1.2.3.4:5").unwrap();
        let all = repo.list().unwrap();
        let keys: Vec<_> = all.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["addr", "theme"]);
    }
}
```

- [ ] **Step 4.2: Run the tests, expect PASS**

Run: `cargo test -p app db::settings::`
Expected: `5 passed; 0 failed`.

- [ ] **Step 4.3: Commit**

```bash
git add app/src-tauri/src/db/settings.rs
git commit -m "feat(app-tauri): client_settings repository with typed key/value"
```

---

## Task 5: workspace_tabs repository

**Files:**
- Create: `app/src-tauri/src/db/tabs.rs`

- [ ] **Step 5.1: Write the failing tests for tab open/close/list/pin/dedupe**

Create `app/src-tauri/src/db/tabs.rs`:
```rust
use rusqlite::{params, OptionalExtension};

use super::{now_secs, Database};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: i64,
    pub kind: String,
    pub resource_id: Option<String>,
    pub title: String,
    pub pinned: bool,
    pub position: i32,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
}

pub struct TabsRepo<'a> {
    db: &'a Database,
}

#[derive(Debug)]
pub enum TabOpenError {
    AlreadyOpen { existing_id: i64 },
    Db(String),
}

impl std::fmt::Display for TabOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyOpen { existing_id } => write!(f, "already open (id={existing_id})"),
            Self::Db(msg) => write!(f, "db: {msg}"),
        }
    }
}

impl<'a> TabsRepo<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn open(
        &self,
        kind: &str,
        resource_id: Option<&str>,
        title: &str,
    ) -> Result<i64, TabOpenError> {
        if let Some(existing) = self.find_open(kind, resource_id)? {
            return Ok(existing);
        }
        let conn = self.db.lock().map_err(|e| TabOpenError::Db(e))?;
        let next_pos = conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM workspace_tabs",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| TabOpenError::Db(format!("next pos: {e}")))?;
        conn.execute(
            "INSERT INTO workspace_tabs (kind, resource_id, title, pinned, position, opened_at)
             VALUES (?, ?, ?, 0, ?, ?)",
            params![kind, resource_id, title, next_pos, now_secs()],
        ).map_err(|e| TabOpenError::Db(format!("insert: {e}")))?;
        let id = conn.last_insert_rowid();
        Ok(id)
    }

    pub fn close(&self, id: i64) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute(
            "UPDATE workspace_tabs SET closed_at = ? WHERE id = ? AND closed_at IS NULL",
            params![now_secs(), id],
        ).map_err(|e| format!("close {id}: {e}"))?;
        Ok(())
    }

    pub fn close_others(&self, keep_id: i64) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute(
            "UPDATE workspace_tabs SET closed_at = ?
             WHERE closed_at IS NULL AND id != ? AND pinned = 0",
            params![now_secs(), keep_id],
        ).map_err(|e| format!("close_others: {e}"))?;
        Ok(())
    }

    pub fn reopen(&self, id: i64) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute(
            "UPDATE workspace_tabs SET closed_at = NULL WHERE id = ?",
            params![id],
        ).map_err(|e| format!("reopen {id}: {e}"))?;
        Ok(())
    }

    pub fn pin(&self, id: i64, pinned: bool) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute(
            "UPDATE workspace_tabs SET pinned = ? WHERE id = ?",
            params![pinned as i64, id],
        ).map_err(|e| format!("pin {id}: {e}"))?;
        Ok(())
    }

    pub fn list_open(&self) -> Result<Vec<Tab>, String> {
        let conn = self.db.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, resource_id, title, pinned, position, opened_at, closed_at
             FROM workspace_tabs
             WHERE closed_at IS NULL
             ORDER BY pinned DESC, position",
        ).map_err(|e| format!("prepare list: {e}"))?;
        let rows = stmt.query_map([], row_to_tab)
            .map_err(|e| format!("query list: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("collect list: {e}"))
    }

    pub fn set_active(&self, id: Option<i64>) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute(
            "INSERT INTO workspace_state (id, active_tab_id) VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET active_tab_id = excluded.active_tab_id",
            params![id],
        ).map_err(|e| format!("set_active: {e}"))?;
        Ok(())
    }

    pub fn active_tab_id(&self) -> Result<Option<i64>, String> {
        let conn = self.db.lock()?;
        conn.query_row(
            "SELECT active_tab_id FROM workspace_state WHERE id = 1",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|e| format!("active: {e}"))?
        .flatten()
        .pipe(Ok)
    }

    fn find_open(&self, kind: &str, resource_id: Option<&str>) -> Result<Option<i64>, TabOpenError> {
        let conn = self.db.lock().map_err(|e| TabOpenError::Db(e))?;
        conn.query_row(
            "SELECT id FROM workspace_tabs WHERE kind = ? AND closed_at IS NULL
             AND ((? IS NULL AND resource_id IS NULL) OR resource_id = ?)",
            params![kind, resource_id, resource_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| TabOpenError::Db(format!("find: {e}")))
    }
}

fn row_to_tab(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tab> {
    Ok(Tab {
        id: row.get(0)?,
        kind: row.get(1)?,
        resource_id: row.get(2)?,
        title: row.get(3)?,
        pinned: row.get::<_, i64>(4)? != 0,
        position: row.get(5)?,
        opened_at: row.get(6)?,
        closed_at: row.get(7)?,
    })
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh_db() -> Database {
        let dir = tempdir().unwrap();
        Database::open(&dir.path().join("bee-client.sqlite")).unwrap()
    }

    #[test]
    fn open_inserts_and_returns_id() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let id = repo.open("cluster", None, "Cluster").unwrap();
        assert!(id > 0);
    }

    #[test]
    fn open_same_resource_twice_returns_existing_id() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let id1 = repo.open("pipeline", Some("p1"), "Pipeline p1").unwrap();
        let id2 = repo.open("pipeline", Some("p1"), "Pipeline p1 (other)").unwrap();
        assert_eq!(id1, id2);
        assert_eq!(repo.list_open().unwrap().len(), 1);
    }

    #[test]
    fn open_distinct_resources_creates_two_tabs() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let a = repo.open("pipeline", Some("p1"), "p1").unwrap();
        let b = repo.open("pipeline", Some("p2"), "p2").unwrap();
        assert_ne!(a, b);
        assert_eq!(repo.list_open().unwrap().len(), 2);
    }

    #[test]
    fn close_then_list_open_excludes_closed() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let id = repo.open("pipeline", Some("p1"), "p1").unwrap();
        repo.close(id).unwrap();
        assert!(repo.list_open().unwrap().is_empty());
    }

    #[test]
    fn reopen_restores_closed_tab() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let id = repo.open("pipeline", Some("p1"), "p1").unwrap();
        repo.close(id).unwrap();
        repo.reopen(id).unwrap();
        assert_eq!(repo.list_open().unwrap().len(), 1);
    }

    #[test]
    fn close_others_keeps_pinned_and_kept_id() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let a = repo.open("cluster", None, "Cluster").unwrap();
        let b = repo.open("pipeline", Some("p1"), "p1").unwrap();
        let c = repo.open("pipeline", Some("p2"), "p2").unwrap();
        repo.pin(b, true).unwrap();
        repo.close_others(a).unwrap();
        let list = repo.list_open().unwrap();
        let ids: Vec<_> = list.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![a, b]);
    }

    #[test]
    fn pin_sets_flag_and_sorts_pinned_first() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let a = repo.open("cluster", None, "Cluster").unwrap();
        let b = repo.open("pipeline", Some("p1"), "p1").unwrap();
        repo.pin(b, true).unwrap();
        let list = repo.list_open().unwrap();
        assert_eq!(list[0].id, b);
        assert!(list[0].pinned);
        assert_eq!(list[1].id, a);
        assert!(!list[1].pinned);
    }

    #[test]
    fn set_active_and_read_back() {
        let db = fresh_db();
        let repo = TabsRepo::new(&db);
        let id = repo.open("cluster", None, "Cluster").unwrap();
        repo.set_active(Some(id)).unwrap();
        assert_eq!(repo.active_tab_id().unwrap(), Some(id));
        repo.set_active(None).unwrap();
        assert_eq!(repo.active_tab_id().unwrap(), None);
    }
}
```

- [ ] **Step 5.2: Run the tests, expect PASS**

Run: `cargo test -p app db::tabs::`
Expected: `7 passed; 0 failed`.

- [ ] **Step 5.3: Commit**

```bash
git add app/src-tauri/src/db/tabs.rs
git commit -m "feat(app-tauri): workspace_tabs repository (dedupe, pin, restore)"
```

---

## Task 6: connection_profiles repository

**Files:**
- Create: `app/src-tauri/src/db/profiles.rs`

- [ ] **Step 6.1: Write the failing tests for profile CRUD**

Create `app/src-tauri/src/db/profiles.rs`:
```rust
use rusqlite::params;

use super::{now_secs, Database};

#[derive(Debug, Clone)]
pub struct Profile {
    pub id: i64,
    pub label: String,
    pub addr: String,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

pub struct ProfilesRepo<'a> {
    db: &'a Database,
}

impl<'a> ProfilesRepo<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn upsert(&self, label: &str, addr: &str) -> Result<i64, String> {
        let conn = self.db.lock()?;
        conn.execute(
            "INSERT INTO connection_profiles (label, addr, last_used_at, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(addr) DO UPDATE SET label = excluded.label, last_used_at = excluded.last_used_at",
            params![label, addr, now_secs(), now_secs()],
        ).map_err(|e| format!("upsert {addr}: {e}"))?;
        let id: i64 = conn.query_row(
            "SELECT id FROM connection_profiles WHERE addr = ?",
            params![addr],
            |row| row.get(0),
        ).map_err(|e| format!("id for {addr}: {e}"))?;
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<Profile>, String> {
        let conn = self.db.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, label, addr, last_used_at, created_at
             FROM connection_profiles
             ORDER BY (last_used_at IS NULL), last_used_at DESC, label",
        ).map_err(|e| format!("prepare list: {e}"))?;
        let rows = stmt.query_map([], |row| {
            Ok(Profile {
                id: row.get(0)?,
                label: row.get(1)?,
                addr: row.get(2)?,
                last_used_at: row.get(3)?,
                created_at: row.get(4)?,
            })
        }).map_err(|e| format!("query list: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("collect list: {e}"))
    }

    pub fn remove(&self, addr: &str) -> Result<(), String> {
        let conn = self.db.lock()?;
        conn.execute("DELETE FROM connection_profiles WHERE addr = ?", params![addr])
            .map_err(|e| format!("remove {addr}: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh_db() -> Database {
        let dir = tempdir().unwrap();
        Database::open(&dir.path().join("bee-client.sqlite")).unwrap()
    }

    #[test]
    fn upsert_inserts_new_profile() {
        let db = fresh_db();
        let repo = ProfilesRepo::new(&db);
        let id = repo.upsert("dev", "127.0.0.1:9999").unwrap();
        assert!(id > 0);
        assert_eq!(repo.list().unwrap().len(), 1);
    }

    #[test]
    fn upsert_existing_addr_updates_label_and_keeps_one_row() {
        let db = fresh_db();
        let repo = ProfilesRepo::new(&db);
        repo.upsert("dev", "127.0.0.1:9999").unwrap();
        repo.upsert("dev-renamed", "127.0.0.1:9999").unwrap();
        let list = repo.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "dev-renamed");
    }

    #[test]
    fn list_orders_by_last_used_desc() {
        let db = fresh_db();
        let repo = ProfilesRepo::new(&db);
        repo.upsert("a", "1.1.1.1:1").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        repo.upsert("b", "2.2.2.2:2").unwrap();
        let list = repo.list().unwrap();
        assert_eq!(list[0].label, "b");
        assert_eq!(list[1].label, "a");
    }

    #[test]
    fn remove_drops_row() {
        let db = fresh_db();
        let repo = ProfilesRepo::new(&db);
        repo.upsert("dev", "127.0.0.1:9999").unwrap();
        repo.remove("127.0.0.1:9999").unwrap();
        assert!(repo.list().unwrap().is_empty());
    }
}
```

- [ ] **Step 6.2: Run the tests, expect PASS**

Run: `cargo test -p app db::profiles::`
Expected: `4 passed; 0 failed`.

- [ ] **Step 6.3: Commit**

```bash
git add app/src-tauri/src/db/profiles.rs
git commit -m "feat(app-tauri): connection_profiles repository (SQLite-backed)"
```

---

## Task 7: one-shot settings.json -> SQLite import

**Files:**
- Create: `app/src-tauri/src/settings_io.rs`
- Modify: `app/src-tauri/src/lib.rs`

- [ ] **Step 7.1: Write the failing test for the importer**

Create `app/src-tauri/src/settings_io.rs`:
```rust
use std::fs;
use std::path::Path;

use crate::db::{Database, SettingsRepo};

pub fn import_legacy_addr(db: &Database, json_path: &Path) -> Result<(), String> {
    let repo = SettingsRepo::new(db);
    if repo.get("addr")?.is_some() {
        return Ok(());
    }
    let content = match fs::read_to_string(json_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read {}: {e}", json_path.display())),
    };
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(addr) = val.get("addr").and_then(|v| v.as_str()) {
        repo.put("addr", addr)?;
    }
    let _ = fs::remove_file(json_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn imports_addr_when_table_empty_and_file_present() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("bee-client.sqlite");
        let json_path = dir.path().join("settings.json");
        fs::write(&json_path, r#"{"addr":"10.0.0.1:10001"}"#).unwrap();
        let db = Database::open(&db_path).unwrap();
        import_legacy_addr(&db, &json_path).unwrap();
        let repo = SettingsRepo::new(&db);
        assert_eq!(repo.get("addr").unwrap().as_deref(), Some("10.0.0.1:10001"));
        assert!(!json_path.exists());
    }

    #[test]
    fn noop_when_addr_already_in_db() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("bee-client.sqlite");
        let json_path = dir.path().join("settings.json");
        fs::write(&json_path, r#"{"addr":"10.0.0.1:10001"}"#).unwrap();
        let db = Database::open(&db_path).unwrap();
        SettingsRepo::new(&db).put("addr", "127.0.0.1:9999").unwrap();
        import_legacy_addr(&db, &json_path).unwrap();
        assert_eq!(
            SettingsRepo::new(&db).get("addr").unwrap().as_deref(),
            Some("127.0.0.1:9999")
        );
    }

    #[test]
    fn noop_when_json_missing() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("bee-client.sqlite");
        let json_path = dir.path().join("settings.json");
        let db = Database::open(&db_path).unwrap();
        import_legacy_addr(&db, &json_path).unwrap();
        assert_eq!(SettingsRepo::new(&db).get("addr").unwrap(), None);
    }

    #[test]
    fn noop_when_json_malformed() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("bee-client.sqlite");
        let json_path = dir.path().join("settings.json");
        fs::write(&json_path, "{not json").unwrap();
        let db = Database::open(&db_path).unwrap();
        import_legacy_addr(&db, &json_path).unwrap();
        assert_eq!(SettingsRepo::new(&db).get("addr").unwrap(), None);
    }
}
```

- [ ] **Step 7.2: Run the tests, expect PASS**

Run: `cargo test -p app settings_io::`
Expected: `4 passed; 0 failed`.

- [ ] **Step 7.3: Commit**

```bash
git add app/src-tauri/src/settings_io.rs
git commit -m "feat(app-tauri): one-shot settings.json -> SQLite import"
```

---

## Task 8: Typed Rust commands (split into submodules)

**Files:**
- Create: `app/src-tauri/src/commands/mod.rs`
- Create: `app/src-tauri/src/commands/cluster.rs`
- Create: `app/src-tauri/src/commands/ping.rs`
- Create: `app/src-tauri/src/commands/connection.rs`
- Create: `app/src-tauri/src/commands/settings.rs`
- Create: `app/src-tauri/src/commands/tabs.rs`
- Create: `app/src-tauri/src/commands/profiles.rs`
- Delete: `app/src-tauri/src/commands.rs` (replaced by the `commands/` directory)

- [ ] **Step 8.1: Define the typed ConnStatus enum (replaces the loose String)**

Create `app/src-tauri/src/commands/mod.rs`:
```rust
pub mod cluster;
pub mod ping;
pub mod connection;
pub mod settings;
pub mod tabs;
pub mod profiles;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CmdError {
    pub message: String,
}

impl<E: std::fmt::Display> From<E> for CmdError {
    fn from(e: E) -> Self {
        Self { message: e.to_string() }
    }
}

pub type CmdResult<T> = Result<T, CmdError>;
```

- [ ] **Step 8.2: Port the existing cluster + ping commands unchanged**

Create `app/src-tauri/src/commands/cluster.rs`:
```rust
use bee_control::raft::{AdminRequest, AdminResponse, ClusterMetricsDetail, JobDetail, JobSummary};

use crate::commands::{CmdError, CmdResult};
use crate::connection::{self, ConnectionHandle};

pub(crate) static HANDLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn ensure_handle(addr: &str) -> Result<ConnectionHandle, CmdError> {
    let _guard = HANDLE_LOCK.lock().await;
    let parsed = connection::addr_parse(addr).map_err(CmdError::from)?;
    Ok(connection::ensure_bundle(parsed))
}

#[tauri::command]
pub async fn cluster_status(addr: String) -> CmdResult<ClusterMetricsDetail> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle.call(AdminRequest::ClusterStatus).await.map_err(CmdError::from)?;
    let resp = rx.await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::ClusterMetrics(d) => Ok(d),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError { message: format!("unexpected: {other:?}") }),
    }
}

#[tauri::command]
pub async fn list_jobs(addr: String) -> CmdResult<Vec<JobSummary>> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle.call(AdminRequest::ListJobs).await.map_err(CmdError::from)?;
    let resp = rx.await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::JobList(j) => Ok(j),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError { message: format!("unexpected: {other:?}") }),
    }
}

#[tauri::command]
pub async fn job_inspect(addr: String, id: u32) -> CmdResult<Option<JobDetail>> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle.call(AdminRequest::JobInspect(id)).await.map_err(CmdError::from)?;
    let resp = rx.await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::JobDetail(d) => Ok(d),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError { message: format!("unexpected: {other:?}") }),
    }
}
```

Create `app/src-tauri/src/commands/ping.rs`:
```rust
use bee_control::raft::AdminRequest;

use crate::commands::{CmdError, CmdResult};
use super::cluster::ensure_handle;

#[tauri::command]
pub async fn ping(addr: String) -> CmdResult<String> {
    let handle = ensure_handle(&addr).await?;
    let _rx = handle.call(AdminRequest::Ping).await.map_err(CmdError::from)?;
    Ok("Pong (queued)".to_string())
}
```

- [ ] **Step 8.3: Type the connection commands (preserve uncommitted semantics + add test_connection)**

Create `app/src-tauri/src/commands/connection.rs`:
```rust
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::{CmdError, CmdResult};
use crate::connection::{self, ConnectionHandle};
use crate::db::{Database, SettingsRepo, DbState};
use super::cluster::HANDLE_LOCK;

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum ConnStatus {
    Connected,
    Connecting,
    Reconnecting,
    Disconnected,
    Error { reason: String },
}

#[derive(Debug, Serialize)]
pub struct StateView {
    pub addr: String,
    pub status: ConnStatus,
}

pub(crate) fn db(app: &AppHandle) -> Result<&DbState, CmdError> {
    app.try_state::<DbState>()
        .map(|s| s.inner())
        .ok_or_else(|| CmdError { message: "db not initialised".into() })
}

#[tauri::command]
pub fn get_default_addr() -> String {
    std::env::var("BEE_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:9999".to_string())
}

#[tauri::command]
pub async fn set_addr(app: AppHandle, addr: String) -> CmdResult<StateView> {
    let _guard = HANDLE_LOCK.lock().await;
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    {
        let db = db(&app)?;
        SettingsRepo::new(db).put("addr", &addr).map_err(CmdError::from)?;
    }
    let handle: ConnectionHandle = connection::ensure_bundle(parsed);
    Ok(view(&handle))
}

#[tauri::command]
pub async fn test_connection(addr: String) -> CmdResult<StateView> {
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        bee_control::raft::admin_client::AdminClient::connect(parsed),
    ).await;
    let status = match outcome {
        Ok(Ok(_client)) => ConnStatus::Connected,
        Ok(Err(e)) => ConnStatus::Error { reason: format!("{e}") },
        Err(_) => ConnStatus::Error { reason: "timeout after 1s".into() },
    };
    Ok(StateView { addr: parsed.to_string(), status })
}

#[tauri::command]
pub fn conn_state(addr: String) -> CmdResult<StateView> {
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    let handle = connection::with_handle(|h| Ok(h.clone())).map_err(CmdError::from)?;
    let _ = parsed;
    Ok(view(&handle))
}

fn view(handle: &ConnectionHandle) -> StateView {
    use crate::connection::ConnectionState as S;
    let status = match handle.state() {
        S::Connected => ConnStatus::Connected,
        S::Connecting => ConnStatus::Connecting,
        S::Disconnected => ConnStatus::Disconnected,
        S::Error(reason) => ConnStatus::Error { reason },
    };
    StateView { addr: handle.addr().to_string(), status }
}

pub type DbState = Database;
```

- [ ] **Step 8.4: Add typed settings / tabs / profiles commands**

Create `app/src-tauri/src/commands/settings.rs`:
```rust
use tauri::AppHandle;

use crate::commands::{CmdError, CmdResult};
use crate::db::{SettingsRepo, Setting};
use super::connection::db;

#[tauri::command]
pub fn settings_get(app: AppHandle, key: String) -> CmdResult<Option<String>> {
    let db = db(&app)?;
    SettingsRepo::new(db).get(&key).map_err(CmdError::from)
}

#[tauri::command]
pub fn settings_put(app: AppHandle, key: String, value: String) -> CmdResult<()> {
    let db = db(&app)?;
    SettingsRepo::new(db).put(&key, &value).map_err(CmdError::from)
}

#[tauri::command]
pub fn settings_delete(app: AppHandle, key: String) -> CmdResult<()> {
    let db = db(&app)?;
    SettingsRepo::new(db).delete(&key).map_err(CmdError::from)
}

#[tauri::command]
pub fn settings_list(app: AppHandle) -> CmdResult<Vec<Setting>> {
    let db = db(&app)?;
    SettingsRepo::new(db).list().map_err(CmdError::from)
}
```

Create `app/src-tauri/src/commands/tabs.rs`:
```rust
use tauri::AppHandle;

use crate::commands::{CmdError, CmdResult};
use crate::db::{TabsRepo, Tab};
use super::connection::db;

#[tauri::command]
pub fn tabs_list(app: AppHandle) -> CmdResult<Vec<Tab>> {
    let db = db(&app)?;
    TabsRepo::new(db).list_open().map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_open(app: AppHandle, kind: String, resource_id: Option<String>, title: String) -> CmdResult<i64> {
    let db = db(&app)?;
    TabsRepo::new(db).open(&kind, resource_id.as_deref(), &title)
        .map_err(|e| CmdError { message: e.to_string() })
}

#[tauri::command]
pub fn tab_close(app: AppHandle, id: i64) -> CmdResult<()> {
    let db = db(&app)?;
    TabsRepo::new(db).close(id).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_close_others(app: AppHandle, keep_id: i64) -> CmdResult<()> {
    let db = db(&app)?;
    TabsRepo::new(db).close_others(keep_id).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_reopen(app: AppHandle, id: i64) -> CmdResult<()> {
    let db = db(&app)?;
    TabsRepo::new(db).reopen(id).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_pin(app: AppHandle, id: i64, pinned: bool) -> CmdResult<()> {
    let db = db(&app)?;
    TabsRepo::new(db).pin(id, pinned).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_set_active(app: AppHandle, id: Option<i64>) -> CmdResult<()> {
    let db = db(&app)?;
    TabsRepo::new(db).set_active(id).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_active_id(app: AppHandle) -> CmdResult<Option<i64>> {
    let db = db(&app)?;
    TabsRepo::new(db).active_tab_id().map_err(CmdError::from)
}
```

Create `app/src-tauri/src/commands/profiles.rs`:
```rust
use tauri::AppHandle;

use crate::commands::{CmdError, CmdResult};
use crate::db::{ProfilesRepo, Profile};
use super::connection::db;

#[tauri::command]
pub fn profiles_list(app: AppHandle) -> CmdResult<Vec<Profile>> {
    let db = db(&app)?;
    ProfilesRepo::new(db).list().map_err(CmdError::from)
}

#[tauri::command]
pub fn profile_upsert(app: AppHandle, label: String, addr: String) -> CmdResult<i64> {
    let db = db(&app)?;
    ProfilesRepo::new(db).upsert(&label, &addr).map_err(CmdError::from)
}

#[tauri::command]
pub fn profile_remove(app: AppHandle, addr: String) -> CmdResult<()> {
    let db = db(&app)?;
    ProfilesRepo::new(db).remove(&addr).map_err(CmdError::from)
}
```

- [ ] **Step 8.5: Delete the old monolithic commands.rs**

Run: `rm app/src-tauri/src/commands.rs`

Note: `cargo build -p app` will now FAIL because `lib.rs` still references the old `commands::get_default_addr` / `commands::set_addr` paths. This is the expected red phase; Task 10 rewrites `lib.rs` and turns it green. Do not skip to Task 10 here.

- [ ] **Step 8.6: Commit**

```bash
git add app/src-tauri/src/commands app/src-tauri/src/commands.rs
git commit -m "refactor(app-tauri): split commands into typed submodules; add test_connection"
```

---

## Task 9: Expose typed ConnStatus from connection module + register the DbHandle

**Files:**
- Modify: `app/src-tauri/src/connection.rs` (only add a re-export; do NOT touch the uncommitted reconnect loop)

- [ ] **Step 9.1: Re-export the typed state from connection.rs**

Append to `app/src-tauri/src/connection.rs` (keep the uncommitted `tokio::select!` reconnect loop intact):
```rust
pub use crate::commands::connection::ConnStatus;
```

- [ ] **Step 9.2: Build, expect PASS**

Run: `cargo build -p app`
Expected: compiles; only benign warnings about the uncommitted-loop `eprintln!`/`debug` lines remain.

- [ ] **Step 9.3: Run all db tests, expect PASS**

Run: `cargo test -p app db::`
Expected: all previous tests still pass.

- [ ] **Step 9.4: Commit**

```bash
git add app/src-tauri/src/connection.rs
git commit -m "feat(app-tauri): re-export typed ConnStatus from connection module"
```

---

## Task 10: Wire the database into Tauri startup + register all handlers

**Files:**
- Modify: `app/src-tauri/src/lib.rs`

- [ ] **Step 10.1: Rewrite lib.rs to initialise the database and register typed handlers**

Replace `app/src-tauri/src/lib.rs` with:
```rust
pub mod connection;
pub mod commands;
pub mod db;
pub mod settings_io;

use std::path::PathBuf;
use tauri::Manager;

use crate::db::{Database, SettingsRepo};
use crate::settings_io::import_legacy_addr;

fn db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| format!("app_data_dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    Ok(dir.join("bee-client.sqlite"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let db = Database::open(&db_path(app.handle())?)?;
            let _ = import_legacy_addr(&db, &legacy_settings_path(app.handle()));

            let saved = SettingsRepo::new(&db).get("addr").ok().flatten();
            let addr = saved.unwrap_or_else(|| {
                std::env::var("BEE_ADMIN_ADDR")
                    .unwrap_or_else(|_| "127.0.0.1:9999".to_string())
            });
            if let Ok(parsed) = connection::addr_parse(&addr) {
                let _ = connection::ensure_bundle(parsed);
            }

            app.manage(db);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::cluster::cluster_status,
            commands::cluster::list_jobs,
            commands::cluster::job_inspect,
            commands::connection::get_default_addr,
            commands::connection::set_addr,
            commands::connection::test_connection,
            commands::connection::conn_state,
            commands::settings::settings_get,
            commands::settings::settings_put,
            commands::settings::settings_delete,
            commands::settings::settings_list,
            commands::tabs::tabs_list,
            commands::tabs::tab_open,
            commands::tabs::tab_close,
            commands::tabs::tab_close_others,
            commands::tabs::tab_reopen,
            commands::tabs::tab_pin,
            commands::tabs::tab_set_active,
            commands::tabs::tab_active_id,
            commands::profiles::profiles_list,
            commands::profiles::profile_upsert,
            commands::profiles::profile_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn legacy_settings_path(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_config_dir()
        .ok()
        .map(|d| d.join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("settings.json"))
}
```

- [ ] **Step 10.2: Build, expect PASS**

Run: `cargo build -p app`
Expected: compiles; only benign warnings about the uncommitted-loop `eprintln!`/`debug` lines remain.

- [ ] **Step 10.3: Run all Rust tests, expect PASS**

Run: `cargo test -p app`
Expected: all tests pass (db/settings/tabs/profiles + connection addr_parse).

- [ ] **Step 10.4: Commit**

```bash
git add app/src-tauri/src/lib.rs
git commit -m "feat(app-tauri): open SQLite on startup; register typed handlers; migrate settings.json"
```

---

## Task 11: Typed TS ipc modules + shared ConnStatus

**Files:**
- Delete: `app/src/ipc.ts`
- Create: `app/src/ipc/shared.ts`
- Create: `app/src/ipc/cluster.ts`
- Create: `app/src/ipc/ping.ts`
- Create: `app/src/ipc/connection.ts`
- Create: `app/src/ipc/settings.ts`
- Create: `app/src/ipc/tabs.ts`
- Create: `app/src/ipc/profiles.ts`
- Create: `app/src/ipc/index.ts`

- [ ] **Step 11.1: Define the shared ConnStatus union + helpers**

Create `app/src/ipc/shared.ts`:
```ts
export type ConnStatus =
  | { kind: "Connected" }
  | { kind: "Connecting" }
  | { kind: "Reconnecting" }
  | { kind: "Disconnected" }
  | { kind: "Error"; reason: string };

export interface StateView {
  addr: string;
  status: ConnStatus;
}

export function isConnected(s: ConnStatus): boolean {
  return s.kind === "Connected";
}
export function isTransient(s: ConnStatus): boolean {
  return s.kind === "Connecting" || s.kind === "Reconnecting";
}
export function isError(s: ConnStatus): boolean {
  return s.kind === "Error" || s.kind === "Disconnected";
}
export function statusLabel(s: ConnStatus, addr: string): string {
  switch (s.kind) {
    case "Connected":      return `Connected to ${addr}`;
    case "Connecting":     return `Connecting to ${addr}…`;
    case "Reconnecting":   return `Reconnecting to ${addr}…`;
    case "Disconnected":   return `Disconnected`;
    case "Error":          return `Connection error: ${s.reason}`;
  }
}
```

- [ ] **Step 11.2: Write the failing test for the shared helpers**

Create `app/src/tests/ipc/shared.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { isConnected, isTransient, isError, statusLabel, type ConnStatus } from "../../ipc/shared";

describe("ipc/shared ConnStatus helpers", () => {
  const cases: ConnStatus[] = [
    { kind: "Connected" },
    { kind: "Connecting" },
    { kind: "Reconnecting" },
    { kind: "Disconnected" },
    { kind: "Error", reason: "boom" },
  ];

  it("isConnected is true only for Connected", () => {
    for (const s of cases) {
      expect(isConnected(s)).toBe(s.kind === "Connected");
    }
  });

  it("isTransient is true for Connecting/Reconnecting only", () => {
    expect(isTransient({ kind: "Connecting" })).toBe(true);
    expect(isTransient({ kind: "Reconnecting" })).toBe(true);
    expect(isTransient({ kind: "Connected" })).toBe(false);
    expect(isTransient({ kind: "Disconnected" })).toBe(false);
    expect(isTransient({ kind: "Error", reason: "x" })).toBe(false);
  });

  it("isError is true for Disconnected + Error", () => {
    expect(isError({ kind: "Disconnected" })).toBe(true);
    expect(isError({ kind: "Error", reason: "x" })).toBe(true);
    expect(isError({ kind: "Connected" })).toBe(false);
    expect(isError({ kind: "Connecting" })).toBe(false);
  });

  it("statusLabel includes the addr for Connected/Connecting/Reconnecting", () => {
    expect(statusLabel({ kind: "Connected" }, "1.2.3.4:5")).toContain("1.2.3.4:5");
    expect(statusLabel({ kind: "Error", reason: "boom" }, "x")).toBe("Connection error: boom");
  });
});
```

- [ ] **Step 11.3: Run the test, expect PASS**

Run: `npm test -- src/tests/ipc/shared.test.ts`
Expected: `4 passed`.

- [ ] **Step 11.4: Port the existing ipc.ts into per-resource modules**

Create `app/src/ipc/cluster.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export interface NodeMetrics { id: number; role: string; commit_index: number; log_length: number }
export interface ClusterMetrics { nodes: NodeMetrics[]; leader_id: number | null; term: number; commit_index: number }
export interface JobSummary {
  job_id: number; dag_hash: string; lifecycle: string; mode: string;
  task_count: number; owner_node: number;
}
export interface JobDetail {
  job_id: number; dag_hash: string; lifecycle: string; owner_node: number;
  dependencies: { upstream_job: number; stream: string }[];
  tasks: { task_id: number; phase_id: number; owner_node: number; status: string }[];
}

export function clusterStatus(addr: string) {
  return invoke<ClusterMetrics>("cluster_status", { addr });
}
export function listJobs(addr: string) {
  return invoke<JobSummary[]>("list_jobs", { addr });
}
export function jobInspect(addr: string, id: number) {
  return invoke<JobDetail | null>("job_inspect", { addr, id });
}
```

Create `app/src/ipc/ping.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";
export function ping(addr: string) { return invoke<string>("ping", { addr }); }
```

Create `app/src/ipc/connection.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";
import type { StateView } from "./shared";

export function getDefaultAddr() { return invoke<string>("get_default_addr"); }
export function setAddr(addr: string) { return invoke<StateView>("set_addr", { addr }); }
export function testConnection(addr: string) { return invoke<StateView>("test_connection", { addr }); }
export function connState(addr: string) { return invoke<StateView>("conn_state", { addr }); }
```

Create `app/src/ipc/settings.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export interface Setting { key: string; value: string; updated_at: number }

export function settingsGet(key: string) { return invoke<string | null>("settings_get", { key }); }
export function settingsPut(key: string, value: string) {
  return invoke<void>("settings_put", { key, value });
}
export function settingsDelete(key: string) { return invoke<void>("settings_delete", { key }); }
export function settingsList() { return invoke<Setting[]>("settings_list"); }
```

Create `app/src/ipc/tabs.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export interface Tab {
  id: number;
  kind: string;
  resource_id: string | null;
  title: string;
  pinned: boolean;
  position: number;
  opened_at: number;
  closed_at: number | null;
}

export function tabsList() { return invoke<Tab[]>("tabs_list"); }
export function tabOpen(kind: string, resourceId: string | null, title: string) {
  return invoke<number>("tab_open", { kind, resourceId, title });
}
export function tabClose(id: number) { return invoke<void>("tab_close", { id }); }
export function tabCloseOthers(keepId: number) { return invoke<void>("tab_close_others", { keepId }); }
export function tabReopen(id: number) { return invoke<void>("tab_reopen", { id }); }
export function tabPin(id: number, pinned: boolean) {
  return invoke<void>("tab_pin", { id, pinned });
}
export function tabSetActive(id: number | null) {
  return invoke<void>("tab_set_active", { id });
}
export function tabActiveId() { return invoke<number | null>("tab_active_id"); }
```

Create `app/src/ipc/profiles.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export interface Profile {
  id: number; label: string; addr: string;
  last_used_at: number | null; created_at: number;
}

export function profilesList() { return invoke<Profile[]>("profiles_list"); }
export function profileUpsert(label: string, addr: string) {
  return invoke<number>("profile_upsert", { label, addr });
}
export function profileRemove(addr: string) {
  return invoke<void>("profile_remove", { addr });
}
```

Create `app/src/ipc/index.ts`:
```ts
export * from "./shared";
export * as cluster from "./cluster";
export * as connection from "./connection";
export * as settings from "./settings";
export * as tabs from "./tabs";
export * as profiles from "./profiles";
export * as ping from "./ping";
```

Delete `app/src/ipc.ts`.

- [ ] **Step 11.5: Write the failing test for arg validation in connection.ts**

Create `app/src/tests/ipc/connection.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { testConnection } from "../../ipc/connection";

describe("ipc/connection", () => {
  it("testConnection returns a Promise", () => {
    const p = testConnection("127.0.0.1:9999");
    expect(p).toBeInstanceOf(Promise);
    p.catch(() => {});
  });
});
```

- [ ] **Step 11.6: Run all frontend tests, expect PASS**

Run: `npm test`
Expected: 5+ tests pass (smoke + 4 shared + 1 connection).

- [ ] **Step 11.7: Commit**

```bash
git add app/src/ipc app/src/ipc.ts app/src/tests/ipc
git commit -m "refactor(app): typed ipc modules + ConnStatus union; mirror Rust types"
```

---

## Task 12: Connection store (polling) + Zustand slimmer

**Files:**
- Modify: `app/src/state/store.ts`
- Create: `app/src/state/connectionStore.ts`
- Create: `app/src/state/tabsStore.ts`
- Create: `app/src/state/settingsUiStore.ts`

- [ ] **Step 12.1: Slim down store.ts to UI-only state**

Replace `app/src/state/store.ts` with:
```ts
import { create } from "zustand";

export type ThemeKind = "light" | "dark";

export interface UiState {
  theme: ThemeKind;
  settingsOpen: boolean;
  settingsSection: "client" | "connection" | "appearance" | "logging" | "diagnostics" | "cluster" | "raft" | "kv" | "scheduling" | "plugins" | "security";
  toggleTheme: () => void;
  setTheme: (t: ThemeKind) => void;
  openSettings: (section?: UiState["settingsSection"]) => void;
  closeSettings: () => void;
}

function lsGet<T>(key: string, fallback: T): T {
  if (typeof localStorage === "undefined") return fallback;
  const v = localStorage.getItem(key);
  if (v === null) return fallback;
  try { return JSON.parse(v) as T; } catch { return fallback; }
}
function lsSet(key: string, v: unknown) {
  if (typeof localStorage === "undefined") return;
  try { localStorage.setItem(key, JSON.stringify(v)); } catch {}
}

const LS_THEME = "bee-client.theme";

export const useUi = create<UiState>((set, get) => ({
  theme: lsGet<ThemeKind>(LS_THEME, "light"),
  settingsOpen: false,
  settingsSection: "client",
  toggleTheme: () => get().setTheme(get().theme === "light" ? "dark" : "light"),
  setTheme: (t) => {
    lsSet(LS_THEME, t);
    set({ theme: t });
    if (typeof document !== "undefined") {
      document.documentElement.classList.toggle("dark", t === "dark");
    }
  },
  openSettings: (section) =>
    set({ settingsOpen: true, settingsSection: section ?? get().settingsSection }),
  closeSettings: () => set({ settingsOpen: false }),
}));
```

- [ ] **Step 12.2: Write the connection store with a polling hook**

Create `app/src/state/connectionStore.ts`:
```ts
import { create } from "zustand";
import { useQuery } from "@tanstack/react-query";
import { connState, getDefaultAddr, setAddr as ipcSetAddr } from "../ipc/connection";
import type { ConnStatus, StateView } from "../ipc/shared";
import { settingsGet, settingsPut } from "../ipc/settings";

interface ConnStore {
  addr: string;
  status: ConnStatus;
  hydrate(): Promise<void>;
  setAddr(addr: string): Promise<void>;
  setStatus(view: StateView): void;
}

const EMPTY: ConnStatus = { kind: "Disconnected" };

export const useConn = create<ConnStore>((set) => ({
  addr: "127.0.0.1:9999",
  status: EMPTY,
  async hydrate() {
    let saved: string | null = null;
    try { saved = await settingsGet("addr"); } catch {}
    if (!saved) {
      try { saved = await getDefaultAddr(); } catch {}
    }
    if (saved) {
      try { await settingsPut("addr", saved); } catch {}
      const view = await connState(saved).catch(() => null);
      set({ addr: saved, status: view?.status ?? { kind: "Connecting" } });
    }
  },
  async setAddr(addr) {
    const view = await ipcSetAddr(addr);
    set({ addr: view.addr, status: view.status });
  },
  setStatus(view) {
    set({ addr: view.addr, status: view.status });
  },
}));

export function useConnStatePolling(intervalMs = 2000) {
  const addr = useConn((s) => s.addr);
  const setStatus = useConn((s) => s.setStatus);
  return useQuery<StateView>({
    queryKey: ["conn-state", addr],
    queryFn: () => connState(addr),
    refetchInterval: intervalMs,
    onSuccess: setStatus,
    retry: false,
    refetchOnWindowFocus: false,
  });
}
```

- [ ] **Step 12.3: Write the failing test for the connection store hydrate path**

Create `app/src/tests/state/connectionStore.test.ts`:
```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const settingsGet = vi.fn();
const settingsPut = vi.fn();
const getDefaultAddr = vi.fn();
const ipcSetAddr = vi.fn();
const connState = vi.fn();

vi.mock("../../ipc/settings", () => ({ settingsGet, settingsPut }));
vi.mock("../../ipc/connection", () => ({
  getDefaultAddr, setAddr: ipcSetAddr, connState, testConnection: vi.fn(),
}));

beforeEach(() => {
  vi.resetModules();
  settingsGet.mockReset();
  settingsPut.mockReset();
  getDefaultAddr.mockReset();
  ipcSetAddr.mockReset();
  connState.mockReset();
});

describe("useConn.hydrate", () => {
  it("uses SQLite value when present", async () => {
    settingsGet.mockResolvedValueOnce("10.0.0.1:10001");
    connState.mockResolvedValueOnce({ addr: "10.0.0.1:10001", status: { kind: "Connected" } });
    const { useConn } = await import("../../state/connectionStore");
    await useConn.getState().hydrate();
    expect(useConn.getState().addr).toBe("10.0.0.1:10001");
    expect(useConn.getState().status).toEqual({ kind: "Connected" });
    expect(getDefaultAddr).not.toHaveBeenCalled();
  });

  it("falls back to backend default and persists it", async () => {
    settingsGet.mockResolvedValueOnce(null);
    getDefaultAddr.mockResolvedValueOnce("127.0.0.1:9999");
    connState.mockResolvedValueOnce({ addr: "127.0.0.1:9999", status: { kind: "Connecting" } });
    const { useConn } = await import("../../state/connectionStore");
    await useConn.getState().hydrate();
    expect(useConn.getState().addr).toBe("127.0.0.1:9999");
    expect(settingsPut).toHaveBeenCalledWith("addr", "127.0.0.1:9999");
  });

  it("setAddr round-trips through IPC", async () => {
    ipcSetAddr.mockResolvedValueOnce({ addr: "2.2.2.2:2", status: { kind: "Connected" } });
    const { useConn } = await import("../../state/connectionStore");
    await useConn.getState().setAddr("2.2.2.2:2");
    expect(useConn.getState().addr).toBe("2.2.2.2:2");
  });
});
```

- [ ] **Step 12.4: Run the test, expect PASS**

Run: `npm test -- src/tests/state/connectionStore.test.ts`
Expected: 3 passed.

- [ ] **Step 12.5: Commit**

```bash
git add app/src/state/store.ts app/src/state/connectionStore.ts app/src/tests/state
git commit -m "feat(app): typed ConnStatus store with polling hydrate path"
```

---

## Task 13: New Shell (left nav + page tabs + bottom bar)

**Files:**
- Create: `app/src/components/Shell.tsx`
- Create: `app/src/components/NavTree.tsx`
- Create: `app/src/components/PageTabs.tsx`
- Modify: `app/src/components/StatusBar.tsx`
- Delete: `app/src/components/AppShell.tsx`
- Delete: `app/src/components/AppBar.tsx`
- Modify: `app/src/App.tsx`
- Modify: `app/src/main.tsx` (no-op unless SettingsModal needs Portal; skip if not)
- Modify: `app/src/pages/Settings.tsx` -> delete, recreate as `app/src/components/SettingsModal.tsx` in Task 14

- [ ] **Step 13.1: Define the Shell layout**

Create `app/src/components/Shell.tsx`:
```tsx
import { useEffect, type ReactNode } from "react";
import { NavTree } from "./NavTree";
import { PageTabs } from "./PageTabs";
import { StatusBar } from "./StatusBar";
import { useUi } from "../state/store";
import { useConn } from "../state/connectionStore";

export function Shell({ children }: { children: ReactNode }) {
  const theme = useUi((s) => s.theme);
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
  }, [theme]);

  useEffect(() => {
    void useConn.getState().hydrate();
  }, []);

  return (
    <div className="h-full flex flex-col bg-gray-50 dark:bg-neutral-900 text-gray-900 dark:text-neutral-100">
      <div className="flex-1 flex overflow-hidden">
        <aside className="w-60 border-r border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800">
          <NavTree />
        </aside>
        <main className="flex-1 flex flex-col overflow-hidden">
          <PageTabs>{children}</PageTabs>
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
```

- [ ] **Step 13.2: Define the left nav tree (Cluster + cog for Settings)**

Create `app/src/components/NavTree.tsx`:
```tsx
import { Settings as Cog, Hexagon, Plus } from "lucide-react";
import { useUi } from "../state/store";
import { useTabsDispatch } from "../state/tabsStore";

export function NavTree() {
  const openSettings = useUi((s) => s.openSettings);
  const open = useTabsDispatch();

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-neutral-700">
        <div className="flex items-center gap-2">
          <Hexagon size={16} className="text-accent-blue" />
          <span className="text-sm font-semibold">Bee</span>
        </div>
        <button
          aria-label="Open settings"
          title="Open settings"
          onClick={() => openSettings("client")}
          className="p-1.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          <Cog size={14} />
        </button>
      </div>

      <nav className="flex-1 overflow-y-auto py-2 px-1 space-y-0.5 text-xs">
        <NavRow
          icon="cluster"
          label="Cluster"
          onClick={() => open({ kind: "cluster", title: "Cluster" })}
        />
        <div className="px-2 pt-4 pb-1 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-gray-500 dark:text-neutral-400">
          <span>Applications (0)</span>
          <button
            aria-label="Add application"
            title="Add application"
            className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700 disabled:opacity-40"
            disabled
          >
            <Plus size={11} />
          </button>
        </div>
        <p className="px-2 py-3 text-[11px] text-gray-400">No Applications yet</p>
      </nav>
    </div>
  );
}

function NavRow({ icon, label, onClick }: { icon: "cluster"; label: string; onClick: () => void }) {
  const Icon = icon === "cluster" ? Hexagon : Hexagon;
  return (
    <button
      onClick={onClick}
      className="w-full flex items-center gap-2 px-2 py-1.5 rounded text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700"
    >
      <Icon size={12} className="shrink-0" />
      <span className="truncate">{label}</span>
    </button>
  );
}
```

- [ ] **Step 13.3: Smoke test the Shell (Renders + does not throw)**

Create `app/src/tests/components/Shell.test.tsx`:
```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";

vi.mock("../../state/connectionStore", () => ({
  useConn: { getState: () => ({ hydrate: () => Promise.resolve() }) },
}));

import { Shell } from "../../components/Shell";

describe("<Shell>", () => {
  it("renders the Bee brand + Cluster nav row", () => {
    render(<Shell><p>content</p></Shell>);
    expect(screen.getByText("Bee")).toBeInTheDocument();
    expect(screen.getByText("Cluster")).toBeInTheDocument();
    expect(screen.getByText("content")).toBeInTheDocument();
  });
});
```

- [ ] **Step 13.4: Run the test, expect PASS**

Run: `npm test -- src/tests/components/Shell.test.tsx`
Expected: 1 passed.

- [ ] **Step 13.5: Delete the old AppShell + AppBar**

Run: `rm app/src/components/AppShell.tsx app/src/components/AppBar.tsx`

- [ ] **Step 13.6: Commit (Shell + NavTree only; PageTabs/StatusBar come in Tasks 14-15)**

```bash
git add app/src/components/Shell.tsx app/src/components/NavTree.tsx app/src/components/AppShell.tsx app/src/components/AppBar.tsx app/src/App.tsx app/src/tests/components
git commit -m "feat(app): new Shell + NavTree; remove Compass-era AppShell"
```

---

## Task 14: Tabs store + PageTabs with deduplication, restore, pin

**Files:**
- Create: `app/src/state/tabsStore.ts`
- Create: `app/src/components/PageTabs.tsx`
- Create: `app/src/tests/state/tabsStore.test.ts`
- Create: `app/src/tests/components/PageTabs.test.tsx`

- [ ] **Step 14.1: Tabs store backed by SQLite IPC**

Create `app/src/state/tabsStore.ts`:
```ts
import { create } from "zustand";
import * as tabs from "../ipc/tabs";
import type { Tab } from "../ipc/tabs";

interface TabsState {
  tabs: Tab[];
  activeId: number | null;
  hydrate(): Promise<void>;
  open(input: { kind: string; resourceId?: string | null; title: string }): Promise<void>;
  close(id: number): Promise<void>;
  setActive(id: number | null): Promise<void>;
  pin(id: number, pinned: boolean): Promise<void>;
}

export const useTabsStore = create<TabsState>((set, get) => ({
  tabs: [],
  activeId: null,
  async hydrate() {
    const [list, active] = await Promise.all([tabs.tabsList(), tabs.tabActiveId()]);
    const hasCluster = list.some((t) => t.kind === "cluster");
    if (!hasCluster) {
      const id = await tabs.tabOpen("cluster", null, "Cluster");
      list.push({
        id, kind: "cluster", resource_id: null, title: "Cluster",
        pinned: false, position: 1, opened_at: Date.now() / 1000, closed_at: null,
      });
    }
    set({ tabs: list, activeId: active ?? list[0]?.id ?? null });
    if (active == null && list[0]) {
      await tabs.tabSetActive(list[0].id);
    }
  },
  async open(input) {
    const id = await tabs.tabOpen(input.kind, input.resourceId ?? null, input.title);
    const list = await tabs.tabsList();
    set({ tabs: list, activeId: id });
    await tabs.tabSetActive(id);
  },
  async close(id) {
    await tabs.tabClose(id);
    const list = await tabs.tabsList();
    const next = get().activeId === id ? (list[0]?.id ?? null) : get().activeId;
    set({ tabs: list, activeId: next });
    if (next !== get().activeId) await tabs.tabSetActive(next);
  },
  async setActive(id) {
    await tabs.tabSetActive(id);
    set({ activeId: id });
  },
  async pin(id, pinned) {
    await tabs.tabPin(id, pinned);
    set({ tabs: await tabs.tabsList() });
  },
}));

export function useTabsDispatch() {
  return useTabsStore((s) => s.open);
}
```

- [ ] **Step 14.2: Write the failing tests for the tabs store**

Create `app/src/tests/state/tabsStore.test.ts`:
```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const tabsList = vi.fn();
const tabOpen = vi.fn();
const tabClose = vi.fn();
const tabSetActive = vi.fn();
const tabActiveId = vi.fn();
const tabPin = vi.fn();

vi.mock("../../ipc/tabs", () => ({
  tabsList, tabOpen, tabClose, tabSetActive, tabActiveId, tabPin,
  tabReopen: vi.fn(),
}));

beforeEach(() => {
  vi.resetModules();
  Object.assign(globalThis, { localStorage: undefined });
  tabsList.mockReset();
  tabOpen.mockReset();
  tabClose.mockReset();
  tabSetActive.mockReset();
  tabActiveId.mockReset();
  tabPin.mockReset();
});

function makeTab(id: number, kind = "cluster"): any {
  return { id, kind, resource_id: null, title: kind, pinned: false, position: id, opened_at: 0, closed_at: null };
}

describe("useTabsStore", () => {
  it("hydrate opens Cluster tab when missing", async () => {
    tabsList.mockResolvedValueOnce([]);
    tabOpen.mockResolvedValueOnce(42);
    tabsList.mockResolvedValueOnce([makeTab(42)]);
    tabActiveId.mockResolvedValueOnce(null);
    const { useTabsStore } = await import("../../state/tabsStore");
    await useTabsStore.getState().hydrate();
    expect(useTabsStore.getState().activeId).toBe(42);
    expect(tabOpen).toHaveBeenCalledWith("cluster", null, "Cluster");
    expect(tabSetActive).toHaveBeenCalledWith(42);
  });

  it("open dedupes by IPC (tabOpen returns existing id)", async () => {
    tabsList.mockResolvedValue([makeTab(7)]);
    tabOpen.mockResolvedValueOnce(7);
    tabSetActive.mockResolvedValueOnce(undefined);
    const { useTabsStore } = await import("../../state/tabsStore");
    await useTabsStore.getState().open({ kind: "cluster", title: "Cluster" });
    expect(useTabsStore.getState().tabs.map((t) => t.id)).toEqual([7]);
    expect(useTabsStore.getState().activeId).toBe(7);
  });

  it("close removes from list and re-points active", async () => {
    tabsList.mockResolvedValueOnce([makeTab(1), makeTab(2)]);
    tabActiveId.mockResolvedValueOnce(1);
    const { useTabsStore } = await import("../../state/tabsStore");
    await useTabsStore.getState().hydrate();
    tabsList.mockResolvedValueOnce([makeTab(2)]);
    tabClose.mockResolvedValueOnce(undefined);
    await useTabsStore.getState().close(1);
    expect(useTabsStore.getState().tabs.map((t) => t.id)).toEqual([2]);
    expect(useTabsStore.getState().activeId).toBe(2);
  });
});
```

- [ ] **Step 14.3: Run the tests, expect PASS**

Run: `npm test -- src/tests/state/tabsStore.test.ts`
Expected: 3 passed.

- [ ] **Step 14.4: Implement the PageTabs component**

Create `app/src/components/PageTabs.tsx`:
```tsx
import { X, Pin, PinOff, type LucideIcon } from "lucide-react";
import { useTabsStore } from "../state/tabsStore";
import { Hexagon, type ReactNode } from "react";
import { ClusterDashboard } from "../pages/ClusterDashboard";

const ICONS: Record<string, LucideIcon> = { cluster: Hexagon };

export function PageTabs({ children }: { children: ReactNode }) {
  const tabs = useTabsStore((s) => s.tabs);
  const activeId = useTabsStore((s) => s.activeId);
  const setActive = useTabsStore((s) => s.setActive);
  const close = useTabsStore((s) => s.close);
  const pin = useTabsStore((s) => s.pin);

  const active = tabs.find((t) => t.id === activeId);

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div role="tablist" className="flex items-end gap-1 px-2 pt-2 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700 overflow-x-auto">
        {tabs.map((t) => {
          const Icon = ICONS[t.kind] ?? Hexagon;
          const isActive = t.id === activeId;
          return (
            <div
              key={t.id}
              role="tab"
              aria-selected={isActive}
              onClick={() => void setActive(t.id)}
              className={[
                "group flex items-center gap-1.5 px-3 h-8 rounded-t-md text-xs cursor-pointer",
                isActive ? "bg-gray-50 dark:bg-neutral-900 text-gray-900 dark:text-neutral-100" : "text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              <Icon size={12} />
              <span className="truncate max-w-[12rem]">{t.title}</span>
              <button
                aria-label={t.pinned ? "Unpin tab" : "Pin tab"}
                title={t.pinned ? "Unpin" : "Pin"}
                onClick={(e) => { e.stopPropagation(); void pin(t.id, !t.pinned); }}
                className="opacity-0 group-hover:opacity-100 p-0.5 rounded text-gray-400 hover:text-accent-blue"
              >
                {t.pinned ? <PinOff size={10} /> : <Pin size={10} />}
              </button>
              <button
                aria-label={`Close ${t.title}`}
                title="Close"
                onClick={(e) => { e.stopPropagation(); void close(t.id); }}
                className="opacity-0 group-hover:opacity-100 p-0.5 rounded text-gray-400 hover:text-accent-red"
              >
                <X size={10} />
              </button>
            </div>
          );
        })}
      </div>
      <div className="flex-1 overflow-auto p-6">
        {active?.kind === "cluster" ? <ClusterDashboard /> : children}
      </div>
    </div>
  );
}
```

- [ ] **Step 14.5: Rename Dashboard.tsx -> ClusterDashboard.tsx + trim**

Run: `git mv app/src/pages/Dashboard.tsx app/src/pages/ClusterDashboard.tsx`

Edit `app/src/pages/ClusterDashboard.tsx`:
- Remove the `console.log("[bee-gui Dashboard] ...")` debug eprintln lines (kept earlier for triage; spec clears them as out of scope for Bee Client).
- Keep the rest of the component intact.

- [ ] **Step 14.6: Write the failing test for the PageTabs rendering + dedupe-on-click**

Create `app/src/tests/components/PageTabs.test.tsx`:
```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";

const tabs = [
  { id: 1, kind: "cluster", resource_id: null, title: "Cluster", pinned: false, position: 1, opened_at: 0, closed_at: null },
  { id: 2, kind: "pipeline", resource_id: "p1", title: "Pipeline p1", pinned: true, position: 2, opened_at: 0, closed_at: null },
];

const setActive = vi.fn();
const close = vi.fn();
const pin = vi.fn();

vi.mock("../../state/tabsStore", () => ({
  useTabsStore: (sel: any) => sel({ tabs, activeId: 1, setActive, close, pin }),
}));

vi.mock("../../pages/ClusterDashboard", () => ({ ClusterDashboard: () => <p>Cluster Dashboard</p> }));

import { PageTabs } from "../../components/PageTabs";

describe("<PageTabs>", () => {
  it("renders one tab per row and renders the active cluster", () => {
    render(<PageTabs><p>fallback</p></PageTabs>);
    expect(screen.getByText("Cluster")).toBeInTheDocument();
    expect(screen.getByText("Pipeline p1")).toBeInTheDocument();
    expect(screen.getByText("Cluster Dashboard")).toBeInTheDocument();
    expect(screen.queryByText("fallback")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 14.7: Run the tests, expect PASS**

Run: `npm test -- src/tests/components/PageTabs.test.tsx`
Expected: 1 passed.

- [ ] **Step 14.8: Commit**

```bash
git add app/src/state/tabsStore.ts app/src/components/PageTabs.tsx app/src/pages/ClusterDashboard.tsx app/src/pages/Dashboard.tsx app/src/tests/state/tabsStore.test.ts app/src/tests/components/PageTabs.test.tsx
git commit -m "feat(app): tabs store + PageTabs (dedupe, pin, close, restore)"
```

---

## Task 15: Settings modal (auto-save + Test Connection + Connect)

**Files:**
- Create: `app/src/state/settingsUiStore.ts`
- Create: `app/src/components/SettingsModal.tsx`
- Create: `app/src/tests/state/settingsUiStore.test.ts`
- Create: `app/src/tests/components/SettingsModal.test.tsx`
- Delete: `app/src/pages/Settings.tsx`
- Preserve (do not modify, do not delete): `app/src/pages/DataSources.tsx`, `app/src/pages/Pipelines.tsx`

- [ ] **Step 15.1: Per-field Saving/Saved/Error store**

Create `app/src/state/settingsUiStore.ts`:
```ts
import { create } from "zustand";

export type FieldStatus = "idle" | "saving" | "saved" | "error";

interface SettingsUiState {
  status: Record<string, FieldStatus>;
  error: Record<string, string | null>;
  setSaving(key: string): void;
  setSaved(key: string): void;
  setError(key: string, msg: string): void;
  reset(key: string): void;
}

export const useSettingsUi = create<SettingsUiState>((set) => ({
  status: {},
  error: {},
  setSaving: (key) => set((s) => ({ status: { ...s.status, [key]: "saving" }, error: { ...s.error, [key]: null } })),
  setSaved: (key) => set((s) => ({ status: { ...s.status, [key]: "saved" } })),
  setError: (key, msg) => set((s) => ({ status: { ...s.status, [key]: "error" }, error: { ...s.error, [key]: msg } })),
  reset: (key) => set((s) => {
    const { [key]: _, ...rest } = s.status;
    const { [key]: __, ...restErr } = s.error;
    return { status: rest, error: restErr };
  }),
}));

export function debounce<T extends (...a: never[]) => unknown>(fn: T, ms: number): T {
  let h: ReturnType<typeof setTimeout> | null = null;
  return ((...args: never[]) => {
    if (h) clearTimeout(h);
    h = setTimeout(() => fn(...args), ms);
  }) as T;
}
```

- [ ] **Step 15.2: Write the failing tests for the debounce + status transitions**

Create `app/src/tests/state/settingsUiStore.test.ts`:
```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useSettingsUi, debounce } from "../../state/settingsUiStore";

describe("useSettingsUi", () => {
  beforeEach(() => useSettingsUi.setState({ status: {}, error: {} }));
  it("transitions saving -> saved", () => {
    useSettingsUi.getState().setSaving("theme");
    expect(useSettingsUi.getState().status.theme).toBe("saving");
    useSettingsUi.getState().setSaved("theme");
    expect(useSettingsUi.getState().status.theme).toBe("saved");
  });
  it("transitions saving -> error", () => {
    useSettingsUi.getState().setSaving("theme");
    useSettingsUi.getState().setError("theme", "boom");
    expect(useSettingsUi.getState().status.theme).toBe("error");
    expect(useSettingsUi.getState().error.theme).toBe("boom");
  });
  it("reset clears the field", () => {
    useSettingsUi.getState().setSaving("x");
    useSettingsUi.getState().reset("x");
    expect(useSettingsUi.getState().status.x).toBeUndefined();
  });
});

describe("debounce", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  it("calls fn once after the delay", () => {
    const fn = vi.fn();
    const d = debounce(fn, 200);
    d("a"); d("b"); d("c");
    vi.advanceTimersByTime(199);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledWith("c");
  });
});
```

- [ ] **Step 15.3: Run the tests, expect PASS**

Run: `npm test -- src/tests/state/settingsUiStore.test.ts`
Expected: 4 passed.

- [ ] **Step 15.4: Implement the Settings modal**

Create `app/src/components/SettingsModal.tsx`:
```tsx
import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { useUi } from "../state/store";
import { useSettingsUi, debounce, type FieldStatus } from "../state/settingsUiStore";
import { settingsGet, settingsPut } from "../ipc/settings";
import { testConnection } from "../ipc/connection";
import { useConn } from "../state/connectionStore";

const SECTIONS = [
  "client", "connection", "appearance", "logging", "diagnostics",
  "cluster", "raft", "kv", "scheduling", "plugins", "security",
] as const;

export function SettingsModal() {
  const open = useUi((s) => s.settingsOpen);
  const section = useUi((s) => s.settingsSection);
  const close = useUi((s) => s.closeSettings);
  if (!open) return null;
  return (
    <div role="dialog" aria-modal="true" className="fixed inset-0 bg-black/40 z-50 flex items-center justify-center">
      <div className="w-[800px] max-h-[80vh] bg-white dark:bg-neutral-800 rounded-lg shadow-xl flex overflow-hidden">
        <nav className="w-48 border-r border-gray-200 dark:border-neutral-700 p-2 text-xs space-y-0.5">
          {SECTIONS.map((s) => (
            <button
              key={s}
              onClick={() => useUi.setState({ settingsSection: s })}
              className={[
                "w-full text-left px-2 py-1.5 rounded",
                section === s ? "bg-accent-blue text-white" : "hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              {s.charAt(0).toUpperCase() + s.slice(1)}
            </button>
          ))}
        </nav>
        <div className="flex-1 flex flex-col">
          <header className="flex items-center justify-between px-4 py-2 border-b border-gray-200 dark:border-neutral-700">
            <h2 className="text-sm font-semibold">Settings &mdash; {section}</h2>
            <button aria-label="Close" onClick={close} className="p-1 rounded hover:bg-gray-100 dark:hover:bg-neutral-700"><X size={14} /></button>
          </header>
          <div className="flex-1 overflow-auto p-4 space-y-6">
            {section === "client" && <ClientSection />}
            {section === "connection" && <ConnectionSection />}
            {section !== "client" && section !== "connection" && (
              <p className="text-xs text-gray-400">Coming in a later slice.</p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function FieldStatusLabel({ status }: { status: FieldStatus | undefined }) {
  if (!status || status === "idle") return null;
  const text = status === "saving" ? "Saving…" : status === "saved" ? "Saved" : "Error";
  const colour = status === "error" ? "text-accent-red" : "text-accent-green";
  return <span className={`text-[10px] ml-2 ${colour}`}>{text}</span>;
}

function AutoSaveInput({ settingKey, initial, ...rest }: { settingKey: string; initial: string } & React.InputHTMLAttributes<HTMLInputElement>) {
  const [v, setV] = useState(initial);
  const ui = useSettingsUi();
  useEffect(() => { setV(initial); }, [initial]);
  const write = debounce(async (next: string) => {
    ui.setSaving(settingKey);
    try { await settingsPut(settingKey, next); ui.setSaved(settingKey); }
    catch (e) { ui.setError(settingKey, String(e)); }
  }, 400);
  return (
    <div className="flex items-center">
      <input
        value={v}
        onChange={(e) => { setV(e.target.value); write(e.target.value); }}
        {...rest}
        className={`flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 ${rest.className ?? ""}`}
      />
      <FieldStatusLabel status={ui.status[settingKey]} />
    </div>
  );
}

function ClientSection() {
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  return (
    <section>
      <h3 className="text-sm font-semibold mb-2">Client</h3>
      <div className="space-y-2 text-xs">
        <label className="flex items-center gap-2">
          <span className="w-32">Theme</span>
          <select value={theme} onChange={(e) => setTheme(e.target.value as "light" | "dark")} className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900">
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </label>
        <label className="flex items-center gap-2">
          <span className="w-32">Log level</span>
          <AutoSaveInput settingKey="log_level" initial="info" placeholder="info | debug | warn | error" />
        </label>
      </div>
    </section>
  );
}

function ConnectionSection() {
  const addr = useConn((s) => s.addr);
  const setAddr = useConn((s) => s.setAddr);
  const [draft, setDraft] = useState(addr);
  const [testResult, setTestResult] = useState<string | null>(null);
  useEffect(() => setDraft(addr), [addr]);

  return (
    <section>
      <h3 className="text-sm font-semibold mb-2">Connection</h3>
      <div className="space-y-3 text-xs">
        <label className="flex items-center gap-2">
          <span className="w-32">Address</span>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="127.0.0.1:9999"
            className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
        </label>
        <p className="text-[11px] text-gray-500 dark:text-neutral-400">
          Active connection: <span className="font-mono">{addr}</span>
        </p>
        <div className="flex items-center gap-2">
          <button
            onClick={async () => {
              try {
                const v = await testConnection(draft.trim());
                setTestResult(`${v.status.kind}${v.status.kind === "Error" ? `: ${(v.status as any).reason}` : ""}`);
              } catch (e) { setTestResult(String(e)); }
            }}
            className="px-3 py-1.5 rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
          >
            Test Connection
          </button>
          <button
            onClick={() => void setAddr(draft.trim())}
            className="px-3 py-1.5 rounded bg-accent-blue text-white hover:bg-accent-blue/90"
          >
            Connect
          </button>
          {testResult && <span className="text-[11px] text-gray-500">{testResult}</span>}
        </div>
      </div>
    </section>
  );
}
```

- [ ] **Step 15.5: Write the failing test for the Settings modal**

Create `app/src/tests/components/SettingsModal.test.tsx`:
```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const settingsGet = vi.fn();
const settingsPut = vi.fn();
const testConnection = vi.fn();
const setAddr = vi.fn();

vi.mock("../../ipc/settings", () => ({ settingsGet, settingsPut, settingsList: vi.fn(), settingsDelete: vi.fn() }));
vi.mock("../../ipc/connection", () => ({ testConnection, setAddr, getDefaultAddr: vi.fn(), connState: vi.fn() }));

vi.mock("../../state/store", () => ({
  useUi: (sel: any) => sel({ settingsOpen: true, settingsSection: "connection", closeSettings: () => {}, setTheme: () => {}, theme: "light" }),
}));

vi.mock("../../state/connectionStore", () => ({
  useConn: (sel: any) => sel({ addr: "127.0.0.1:9999", setAddr }),
}));

import { SettingsModal } from "../../components/SettingsModal";

beforeEach(() => {
  settingsGet.mockReset();
  settingsPut.mockReset();
  testConnection.mockReset();
  setAddr.mockReset();
});

describe("<SettingsModal> Connection section", () => {
  it("Test Connection does not mutate active connection", async () => {
    testConnection.mockResolvedValueOnce({ addr: "1.2.3.4:5", status: { kind: "Error", reason: "refused" } });
    render(<SettingsModal />);
    fireEvent.click(screen.getByText("Test Connection"));
    expect(testConnection).toHaveBeenCalled();
    expect(setAddr).not.toHaveBeenCalled();
  });
  it("Connect switches the active connection", async () => {
    setAddr.mockResolvedValueOnce(undefined);
    render(<SettingsModal />);
    fireEvent.click(screen.getByText("Connect"));
    expect(setAddr).toHaveBeenCalledWith("127.0.0.1:9999");
  });
});
```

- [ ] **Step 15.6: Run the tests, expect PASS**

Run: `npm test -- src/tests/components/SettingsModal.test.tsx`
Expected: 2 passed.

- [ ] **Step 15.7: Mount the modal in App.tsx and delete the old Settings page only**

Edit `app/src/App.tsx`:
```tsx
import { Shell } from "./components/Shell";
import { SettingsModal } from "./components/SettingsModal";
import { useTabsStore } from "./state/tabsStore";
import { useEffect } from "react";

export default function App() {
  useEffect(() => { void useTabsStore.getState().hydrate(); }, []);
  return (
    <>
      <Shell>{null}</Shell>
      <SettingsModal />
    </>
  );
}
```

Delete only `app/src/pages/Settings.tsx`. **Do NOT delete** `app/src/pages/DataSources.tsx` or `app/src/pages/Pipelines.tsx`: those pages are out of foundation scope but must remain on disk for later migration. They are unreferenced from `App.tsx` for now; tsc is fine with unused exports (no `noUnusedLocals` enforcement on exports). A future slice will wire them into the left navigation tree.

- [ ] **Step 15.8: Run the full frontend suite + tsc, expect PASS**

Run: `npm test && npx tsc --noEmit`
Expected: all tests pass; tsc emits 0 errors. `DataSources.tsx` and `Pipelines.tsx` are present but unreferenced.

- [ ] **Step 15.9: Commit**

```bash
git add app/src/components/SettingsModal.tsx app/src/state/settingsUiStore.ts app/src/App.tsx app/src/pages/Settings.tsx app/src/tests/state/settingsUiStore.test.ts app/src/tests/components/SettingsModal.test.tsx
git commit -m "feat(app): Settings modal with auto-save + Test Connection + Connect"
```

Note: `DataSources.tsx` and `Pipelines.tsx` are intentionally untouched and not staged.

---

## Task 16: ConnectionStatus indicator (3 states + text + error link)

**Files:**
- Create: `app/src/components/ConnectionStatus.tsx`
- Modify: `app/src/components/StatusBar.tsx`
- Create: `app/src/tests/components/ConnectionStatus.test.tsx`

- [ ] **Step 16.1: Write the failing test first**

Create `app/src/tests/components/ConnectionStatus.test.tsx`:
```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const openSettings = vi.fn();
vi.mock("../../state/store", () => ({
  useUi: (sel: any) => sel({ openSettings }),
}));

import { ConnectionStatus } from "../../components/ConnectionStatus";

describe("<ConnectionStatus>", () => {
  it("Connected renders solid green + accessible label", () => {
    render(<ConnectionStatus addr="1.2.3.4:5" status={{ kind: "Connected" }} />);
    expect(screen.getByLabelText(/Connected to/)).toBeInTheDocument();
  });

  it("Connecting renders pulsing green", () => {
    const { container } = render(<ConnectionStatus addr="x" status={{ kind: "Connecting" }} />);
    expect(screen.getByLabelText(/Connecting to/)).toBeInTheDocument();
    expect(container.querySelector(".animate-pulse")).toBeTruthy();
  });

  it("Error renders a link that opens Settings at Connection", () => {
    render(<ConnectionStatus addr="x" status={{ kind: "Error", reason: "refused" }} />);
    const link = screen.getByRole("button", { name: /Open connection settings/i });
    fireEvent.click(link);
    expect(openSettings).toHaveBeenCalledWith("connection");
  });
});
```

- [ ] **Step 16.2: Implement ConnectionStatus**

Create `app/src/components/ConnectionStatus.tsx`:
```tsx
import { Circle, AlertCircle } from "lucide-react";
import { useUi } from "../state/store";
import { statusLabel, type ConnStatus } from "../ipc/shared";

export function ConnectionStatus({ addr, status }: { addr: string; status: ConnStatus }) {
  const openSettings = useUi((s) => s.openSettings);
  const { dotClass, a11y } = (() => {
    switch (status.kind) {
      case "Connected":    return { dotClass: "text-accent-green",                  a11y: "Connected" };
      case "Connecting":
      case "Reconnecting": return { dotClass: "text-accent-green animate-pulse",     a11y: "Connecting" };
      case "Disconnected": return { dotClass: "text-accent-red",                    a11y: "Disconnected" };
      case "Error":        return { dotClass: "text-accent-red",                    a11y: "Error" };
    }
  })();
  const label = statusLabel(status, addr);
  return (
    <div className="flex items-center gap-2 text-[11px]" aria-label={label}>
      {status.kind === "Error" ? <AlertCircle size={12} className={dotClass} /> : <Circle size={10} className={dotClass} />}
      <span className="text-gray-700 dark:text-neutral-200" aria-label={`${a11y} to ${addr}`}>{label}</span>
      {(status.kind === "Error" || status.kind === "Disconnected") && (
        <button
          className="underline text-accent-blue"
          aria-label="Open connection settings"
          onClick={() => openSettings("connection")}
        >
          Open Settings
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 16.3: Rewrite StatusBar to consume useConn + render the indicator**

Replace `app/src/components/StatusBar.tsx` with:
```tsx
import { useConn, useConnStatePolling } from "../state/connectionStore";
import { ConnectionStatus } from "./ConnectionStatus";

export function StatusBar() {
  const addr = useConn((s) => s.addr);
  const status = useConn((s) => s.status);
  useConnStatePolling();
  return (
    <footer className="flex items-center gap-4 px-3 h-7 text-[10px] text-gray-500 dark:text-neutral-400 bg-white dark:bg-neutral-800 border-t border-gray-200 dark:border-neutral-700">
      <span className="font-medium text-gray-700 dark:text-neutral-200">Bee Client v0.1.0 (Tauri)</span>
      <ConnectionStatus addr={addr} status={status} />
    </footer>
  );
}
```

Note: the previous version of `StatusBar.tsx` rendered a literal `audit: (none yet)` span as a placeholder for the future audit-event summary. Per the foundation-scope discipline, that placeholder is dropped entirely; the audit summary is added in a later slice (see Out of scope).

- [ ] **Step 16.4: Run the tests, expect PASS**

Run: `npm test -- src/tests/components/ConnectionStatus.test.tsx`
Expected: 3 passed.

- [ ] **Step 16.5: Commit**

```bash
git add app/src/components/ConnectionStatus.tsx app/src/components/StatusBar.tsx app/src/tests/components/ConnectionStatus.test.tsx
git commit -m "feat(app): ConnectionStatus indicator (red/solid-green/pulsing-green)"
```

---

## Task 17: Bee GUI -> Bee Client label rename

**Files:**
- Modify: `app/package.json` (`name: bee-client-app`, `description: Bee Client`)
- Modify: `app/src-tauri/Cargo.toml` (`description: Bee Client`, `name: app` stays)
- Modify: `app/src-tauri/tauri.conf.json` (`productName: Bee Client`, `title: Bee Client`)
- Modify: `app/index.html` (`<title>Bee Client</title>`)
- Modify: `app/src/state/store.ts` (drop `LS_*` keys from `bee-gui.*` to `bee-client.*`)
- Modify: `app/src/components/StatusBar.tsx` (already says "Bee Client" — verify)
- Modify: `app/src/tooltip.ts` (delete; tooltip is now inline)
- Modify: `app/src/styles.css` (comment fix)
- Create: `app/src/tests/components/labels.test.tsx`

- [ ] **Step 17.1: Update package.json**

Edit `app/package.json`:
- `"name": "bee-client-app"`
- `"description": "Bee Client (Tauri desktop client)"`

- [ ] **Step 17.2: Update Cargo.toml**

Edit `app/src-tauri/Cargo.toml`:
- `description = "Bee Client (Tauri desktop client)"`

- [ ] **Step 17.3: Update tauri.conf.json**

Edit `app/src-tauri/tauri.conf.json`:
- `"productName": "Bee Client"`
- `"title": "Bee Client"`
- `"identifier": "io.smitea.beeclient"`

- [ ] **Step 17.4: Update index.html**

Edit `app/index.html`:
- `<title>Bee Client</title>`

- [ ] **Step 17.5: Rename localStorage keys in store.ts**

Edit `app/src/state/store.ts` `LS_THEME` constant:
```ts
const LS_THEME = "bee-client.theme";
```

(The old `bee-gui.*` keys can stay; this only changes what new installs write. A migration shim is out of scope for the foundation slice.)

- [ ] **Step 17.6: Remove the Compass + Bee GUI mentions in StatusBar**

Already replaced in Task 16. Verify by running:
```bash
grep -rn "Bee GUI\|Compass" app/src app/src-tauri/src
```
Expected: no matches.

- [ ] **Step 17.7: Write the label test**

Create `app/src/tests/components/labels.test.tsx`:
```tsx
import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";

vi.mock("../../state/connectionStore", () => ({
  useConn: (sel: any) => sel({ addr: "127.0.0.1:9999", status: { kind: "Disconnected" } }),
  useConnStatePolling: () => ({}),
}));

import { StatusBar } from "../../components/StatusBar";

describe("Bee Client labels", () => {
  it("StatusBar identifies as Bee Client", () => {
    const { container } = render(<StatusBar />);
    expect(container.textContent).toContain("Bee Client");
    expect(container.textContent).not.toContain("Bee GUI");
    expect(container.textContent).not.toContain("Compass");
  });
});
```

- [ ] **Step 17.8: Run the full test suite + tsc + Rust tests**

Run:
```bash
npm test && npx tsc --noEmit && cargo test -p app
```
Expected: all tests pass; tsc emits 0 errors.

- [ ] **Step 17.9: Commit**

```bash
git add app/package.json app/src-tauri/Cargo.toml app/src-tauri/tauri.conf.json app/index.html app/src/state/store.ts app/src/styles.css app/src/tooltip.ts app/src/tests/components/labels.test.tsx
git commit -m "chore(app): rename Bee GUI -> Bee Client in shipped strings + identifier"
```

---

## Task 18: CI hooks (frontend tests)

**Files:**
- Modify: `.github/workflows/rust.yml`

- [ ] **Step 18.1: Add a frontend-test job that runs after the existing tauri-frontend-build**

Append to `.github/workflows/rust.yml`:
```yaml
  tauri-frontend-test:
    name: Tauri frontend tests (Vitest)
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: app
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: "20"
          cache: "npm"
          cache-dependency-path: app/package-lock.json
      - run: npm ci --no-audit --no-fund
      - run: npm test
```

- [ ] **Step 18.2: Verify locally**

Run: `npm test`
Expected: all tests pass.

- [ ] **Step 18.3: Commit**

```bash
git add .github/workflows/rust.yml
git commit -m "ci: add vitest run to CI workflow"
```

---

## End-to-end verification (run after every task)

```bash
# Frontend
cd app
npm ci --no-audit --no-fund
npx tsc --noEmit
npm test
npm run build

# Rust
cd ..
cargo build -p app
cargo test -p app

# Confirm no leaked Compass / Bee GUI strings
grep -rn "Bee GUI\|Compass" app/src app/src-tauri/src
```

Expected after Task 18:
- `npx tsc --noEmit` → 0 errors
- `npm test` → all green
- `npm run build` → builds dist
- `cargo build -p app` → ok
- `cargo test -p app` → 26 tests passed (db:: 3 + settings 5 + tabs 8 + profiles 4 + settings_io 4 + connection::tests 2)
- `grep` → no matches

---

## Out of scope (deferred to later slices)

- Application lifecycle (enablement/disablement state machine)
- Audit events + activity dialog + bottom-bar audit summary
- Global search across local + cluster
- Cluster Dashboard real content (currently the renamed ClusterDashboard from the Compass slice)
- Pipeline structure graph + Interactions
- Datasource forms + Plugin/Adapter schemas
- Import/export encryption + KDF
- Dashboard builder (drag/resize grid)
- Plugin Registry listing on the AdminServer
- CSP hardening
- Tenant enforcement (1.x)