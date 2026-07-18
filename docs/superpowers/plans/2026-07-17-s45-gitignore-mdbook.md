# S45 — `.gitignore` Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Exclude `docs/book/book/` (mdbook build output) from git. Document the build command in `docs/book/README.md`.

**Architecture:** Add one line to `.gitignore` + create a one-paragraph `docs/book/README.md`. No code changes, no test changes. 2 commits (1 implementation + 1 verification + push).

**Tech Stack:** Git, mdbook (no code changes).

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `.gitignore` | Modify | Add `/docs/book/book/` |
| `docs/book/README.md` | Create | Document `mdbook serve` / `mdbook build` commands |

1 Task (very small).

---

## Task 1: Add `.gitignore` entry + create `docs/book/README.md` + push

- [ ] **Step 1.1: Add `/docs/book/book/` to `.gitignore`**

Run: `cat .gitignore`. The file currently has:

```
/target/
/Cargo.lock

.DS_Store
*.swp
*.swo
*~
.idea/
.vscode/
.superpowers/
```

Append `/docs/book/book/` to the end. Final `.gitignore`:

```
/target/
/Cargo.lock

.DS_Store
*.swp
*.swo
*~
.idea/
.vscode/
.superpowers/

# S45: mdbook build output. The source lives in
# docs/book/src/ + docs/book/book.toml and is tracked.
/docs/book/book/
```

(Add the 3-line comment block so future devs understand why this entry exists. If `.gitignore` style prefers no comments, drop the 3 lines and just add `/docs/book/book/`.)

- [ ] **Step 1.2: Create `docs/book/README.md`**

Create `docs/book/README.md`:

```markdown
# Bee Documentation Book

This directory is the source for Bee's rendered documentation site (mdbook).

## Build

```bash
# Install mdbook (one-time):
cargo install mdbook --version 0.4

# Local preview (serves on http://localhost:3000):
mdbook serve docs/book

# Build static output (output goes to docs/book/book/, gitignored):
mdbook build docs/book
```

The generated HTML / CSS / JS / fonts land in `docs/book/book/`. That directory is in `.gitignore` (since S45) — re-run `mdbook build` to regenerate.

## Source layout

- `book.toml` — mdbook configuration (title, authors, output dir, etc.)
- `src/` — markdown source files. One `.md` per chapter; nested directories become section headers in the rendered output.
- `SUMMARY.md` — table of contents. Edit this to reorder / add / remove chapters.
```

- [ ] **Step 1.3: Verify `git status` doesn't show `docs/book/`**

Run: `git status`. Expected: `docs/book/README.md` appears as **untracked** (because it's new and not yet `git add`-ed). The `docs/book/book/` directory itself does NOT appear (it's in `.gitignore`).

If `docs/book/book/` appears in `git status` as untracked, the `.gitignore` pattern is wrong — fix the pattern (e.g., if `docs/book/` is also being created with other files inside, use `docs/book/book/` with a trailing slash).

- [ ] **Step 1.4: Verify `git ls-files docs/book/` shows only source files**

Run:

```bash
git ls-files docs/book/ | wc -l
git ls-files docs/book/
```

Expected first command: small number (5–10 source files: `book.toml` + a handful of `.md` files).

Expected second command: lists only the source files, e.g.:

```
docs/book/SUMMARY.md
docs/book/book.toml
docs/book/src/architecture/data_model.md
docs/book/src/architecture/internals.md
...
```

If the output includes `docs/book/book/index.html` or other build artifacts, the `.gitignore` is broken — fix.

- [ ] **Step 1.5: Full workspace build + test (no regression expected)**

Run: `cargo build --workspace 2>&1 | tail -3`. Expected: clean build.

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 425 failed: 0 ignored: 5` (no change — S45 touches no Rust code).

- [ ] **Step 1.6: Commit**

```bash
git add .gitignore docs/book/README.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S45: .gitignore docs/book/book/ + docs/book/README.md with build commands"
```

- [ ] **Step 1.7: Update spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s45-gitignore-mdbook-design.md` and flip all `[ ]` to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s45-gitignore-mdbook-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S45: flip acceptance criteria to [x]"
```

- [ ] **Step 1.8: Update `docs/stories.md` S45 acceptance criteria**

Edit `docs/stories.md` (S45 section, line ~1212). Flip the `[ ]` to `[x]`. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: S45 acceptance criteria flipped"
```

- [ ] **Step 1.9: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S45 spec's in-scope items:
- Add `/docs/book/book/` to `.gitignore`: Step 1.1 ✓
- Create `docs/book/README.md` with build commands: Step 1.2 ✓
- Verify `git status` doesn't show `docs/book/`: Step 1.3 ✓
- Verify `git ls-files` only shows source: Step 1.4 ✓

**2. Placeholder scan:** Searched for "TBD" / "TODO" — none in the plan body.

**3. Type consistency:** No Rust types touched in S45. No type changes needed.

**4. Ambiguity check:** Each step specifies the exact file path + exact content to write. The `.gitignore` pattern is unambiguous (`/docs/book/book/` excludes only the build output). The `README.md` content is complete.

---

## Estimated Total

- 1 Task
- 3 commits (impl + criteria flip + stories flip)
- 2 files modified/created (`.gitignore` + `docs/book/README.md`)
- Estimated wall-clock: 5 minutes