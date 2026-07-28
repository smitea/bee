# Contributing to Bee

Bee is a distributed dataflow pipeline compute service written in Rust + (UI in either iced or Tauri).

## Project layout

```
bee/
├── bee/                          # main binary: bee CLI + node process
├── crates/                       # library crates (Rust workspace)
│   ├── bee-control/              # Raft, KV, AdminServer, plugins, ...
│   ├── bee-runtime/              # DAG executor, scheduler, plugins
│   ├── bee-dsl-sql/              # DataFusion integration, SQL preprocessor
│   ├── bee-plugin-sdk/           # Plugin trait + BeeHostV1
│   ├── bee-registry/              # Plugin Manager
│   ├── bee-adapter/              # Adapter trait
│   ├── bee-transport/             # TCP / TLS
│   ├── bee-codec/                # BRP frame + bincode
│   ├── bee-types/                 # shared types
│   ├── bee-plugin-macro/         # `#[bee_adapter]` proc-macro
│   └── bee-kv-test/              # in-memory KV state machine
├── plugins/                      # cdylib plugins
│   ├── bee-plugin-perf-fib/      # fib_step UDF for S41 perf demo
│   ├── bee-plugin-onnx-ml/       # ONNX ML scoring
│   └── quant/                    # reference Datasource plugins
├── examples/                     # example SQL pipelines
├── docs/                          # adr/ + stories.md + superpowers/
├── tests/                        # integration tests
├── scripts/                       # build scripts
└── rust-toolchain.toml            # pins rustc 1.89.0 (kstring 2.0.2 compat)
```

## Toolchain

- **Rust**: 1.89.0 (pinned via `rust-toolchain.toml`)
  - **Why pinned**: `kstring 2.0.2` is the highest version compatible with rustc 1.89; newer kstring requires rustc 1.96+. The pin is enforced via the lockfile.
- **Cargo**: bundled with Rust 1.89.0

## Initial setup

```sh
git clone https://github.com/smitea/bee.git
cd bee
# No bootstrap needed — Cargo can build everything from the lockfile.
cargo build --workspace --release
cargo test --workspace --release
```

### Performance

The first build is slow (especially on cold caches). For incremental work:

```sh
# Use a single job to avoid RAM spikes (Bee has ~12 workspace members + plugins)
cargo build -j 4
```

## Running tests

```sh
# All tests, release mode (matches CI)
cargo test --workspace --release

# Specific tests
cargo test -p bee-control --release
cargo test -p bee-dsl-sql --release
```

Test count: ~477 pass / 0 fail / 5 ignored (the 5 ignored are deferred test-utils stubs).

## GUI

Bee has had two GUI implementations over the project lifetime. The current `main` is on **iced 0.12**; a Tauri 2.x + React + Vite + Tailwind rewrite lives on the `feat/taui-frontend` + `feat/taui-cleanup` + `feat/taui-helpers` branches.

### iced (current `main`)

```sh
cargo run -p bee                              # the CLI / node process
cargo build -p bee-gui --release              # the iced GUI binary
./target/release/bee-gui --connect 127.0.0.1:10001
```

The iced GUI workspace member is `crates/bee-gui/`. It pins no extra toolchain.

### Tauri (in-flight branches)

```sh
git fetch origin
git checkout -b feat/taui-final origin/feat/tauri-helpers
git merge origin/feat/taui-cleanup
cd app
npm ci --no-audit --no-fund
npm run build
cd src-tauri
cargo build --release           # ~30 min first build
# Then run via `cargo tauri dev` or `cargo tauri build` for the bundle.
```

See [`docs/superpowers/specs/2026-07-28-s-tauri-gui-design.md`](docs/superpowers/specs/2026-07-28-s-tauri-gui-design.md) for the full Tauri design.

## Story workflow

1. Pick a story from [`docs/stories.md`](docs/stories.md) whose `Blocked by` list is satisfied.
2. AFK stories can be implemented without a human — see `type: AFK` in the story header.
3. Implement + write tests + flip the `[ ]` to `[x]` in the story's acceptance criteria.
4. Commit in a worktree (per `using-git-worktrees` skill; never commit on `main`).
5. PR from your worktree branch to `main`.
6. CI runs build + test + (for Tauri) frontend-build.

## Common recipes

### Add a new AdminRequest variant

1. Add the variant to `AdminRequest` + `AdminResponse` in `crates/bee-control/src/raft/admin_protocol.rs`.
2. Wire it in `crates/bee-control/src/raft/admin_server.rs::dispatch` (handle + return).
3. Add a Tauri command in `app/src-tauri/src/commands.rs` + `app/src/ipc.ts` (if using Tauri).
4. Add a unit test in `crates/bee-control/tests/`.
5. Add an integration test in `crates/bee-control/tests/`.

### Add a new story

1. Add a section to `docs/stories.md` following the existing pattern (see "Story format" below).
2. Create a design spec in `docs/superpowers/specs/` only if the story is non-trivial.
3. Reference the story's spec from the story header.
4. Mark boxes as `[x]` as you implement each one.

### Add a new plugin

```sh
mkdir -p plugins/bee-plugin-<name>
cd plugins/bee-plugin-<name>
# Copy an existing plugin's Cargo.toml structure (start with bee-plugin-perf-fib)
# Implement the `bee_plugin_init` exported function (see crates/bee-plugin-sdk/src/lib.rs)
# Add the plugin to the workspace in root Cargo.toml
# Add a `bee plugin load ./target/release/libbee_plugin_<name>.dylib` test fixture
```

## Conventions

- **Commits**: present-tense, scoped (`feat(S-1a): ...`, `fix(raft): ...`, `docs: ...`).
- **Atomic commits**: one logical change per commit.
- **Test layout**: unit tests in `#[cfg(test)] mod tests` next to the code; integration tests in
  `tests/` of the relevant crate.
- **Lint**: `cargo clippy --workspace --all-targets -- -D warnings` (S-1c follow-up).
- **Format**: `cargo fmt --all` (S-1c follow-up).
- **Branch names**: `feat/<story>-<short-desc>`, `fix/<scope>-<short-desc>`, `docs/<short-desc>`.
- **Worktrees**: always use `git worktree add .worktrees/<branch> -b <branch> main` before
  committing anything. Never commit on `main`.

## Story format

Reproduce this skeleton when adding a new story (see `docs/stories.md` for the canonical example):

```markdown
### SXX · <title>

- **Type**: AFK | HITL
- **Blocked by**: SAA, SBB
- **ADRs**: [0001](./adr/0001-data-plane-p2p-control-plane-raft.md)

**What to build**
<one paragraph + bullet list>

**Acceptance criteria**
- [ ] <verifiable criterion 1>
- [ ] <verifiable criterion 2>
```

## CI

`.github/workflows/rust.yml` runs on every push to `main` + every PR:

- `rust-build` (ubuntu-latest, rustc 1.89.0) — `cargo build --workspace --release`
- `rust-test` (ubuntu-latest) — `cargo test --workspace --release`
- `tauri-frontend-build` (ubuntu-latest, Node.js 20) — `npm ci && tsc --noEmit && vite build`

The Tauri Rust build is **not** in CI yet (Tauri 2.x first build takes ~30 min on M2).
Once the Tauri migration merges, the CI workflow will gain a `tauri-build` job.

## See also

- [CONTEXT.md](CONTEXT.md) — current state + recent commits
- [docs/stories.md](docs/stories.md) — 38 stories with acceptance criteria
- [docs/adr/](docs/adr/) — 10 numbered ADRs
- [docs/superpowers/specs/](docs/superpowers/specs/) — design specs

## License

Apache-2.0 (see headers in each file).