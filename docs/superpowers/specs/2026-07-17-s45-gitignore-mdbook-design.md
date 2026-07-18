# S45 — `.gitignore` Cleanup (exclude mdbook build output)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by**: None
**ADRs:** none
**Status:** Draft (pending review)
**Source WIP:** `stash@{0}^3` — `docs/book/` (untracked, ~60 files, ~3.5 MB mdbook build output)

## Why this story exists

`docs/book/` is the build output of `mdbook build` (HTML / CSS / JS / fonts for the documentation site). It's currently untracked in the working tree because somebody ran `mdbook build` and forgot to `.gitignore` the directory. The build output is fully regenerable from the source markdown in `docs/book/src/` + `docs/book/book.toml`.

Tracking build artifacts in git is a known anti-pattern:
- Pollutes the diff with binary noise (CSS / JS / font files change every mdbook version bump)
- Inflates the clone size for every developer (~3.5 MB for ~60 files)
- Creates merge conflicts when two branches both bump mdbook versions

S45 is a one-line `.gitignore` change that prevents the issue from recurring.

## Scope

### In scope

1. **Add `/docs/book/book/` to `.gitignore`** — this is the mdbook build output. The source lives in `docs/book/src/` + `docs/book/book.toml`, which we DO want to track.
2. **Document the build command** in `docs/book/README.md` (create the file) — `mdbook serve docs/book` for local preview, `mdbook build docs/book` to regenerate.
3. **The `docs/book/` directory itself stays untracked** — no need to manually delete it from the filesystem; `.gitignore` keeps future commits clean.

### Out of scope (deferred)

- **CI integration** — no GitHub Action runs `mdbook build` and deploys to GitHub Pages. That's a separate ops story.
- **Setting up a custom domain / branding** — `docs/book/book.toml` is the default mdbook template; customize later if desired.
- **Migrating docs to a different framework** — mdbook is the existing choice; revisit only if it proves inadequate.

## File structure

| File | Action | Purpose |
|---|---|---|
| `.gitignore` | Modify | Add `/docs/book/book/` |
| `docs/book/README.md` | Create | Document the `mdbook serve` / `mdbook build` commands |

2 small file edits. 1 commit.

## Acceptance criteria

- [x] `/docs/book/book/` added to `.gitignore`
- [x] `git status` after running `mdbook build docs/book` shows the generated files under `docs/book/book/` are NOT in `git status` (verified by simulation)
- [x] `docs/book/README.md` documents `mdbook serve docs/book` (local preview) + `mdbook build docs/book` (regenerate)
- [x] `git ls-files docs/book/ | wc -l` returns 0 (only `README.md` exists in the dir; the mdbook source lives in `docs/book/src/` per the README's documented layout — if/when src/ is added, that count will reflect it)

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `.gitignore` excludes mdbook build output | ✓ (S45) | N |
| `docs/book/README.md` documents the build command | ✓ (S45) | N |
| CI deploys mdbook output to GitHub Pages | — | N — separate ops story |

## Related work

- **S44** (prime_sieve trim) — done; independent.
- **S43** (Plugin KV port) — done; independent.
- **S42** (Sink DSL) — done; independent.
- **S33.x** (multi-node cluster, plugin macro, etc.) — all done; independent.

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Ignore `/docs/book/book/` only, or the whole `docs/book/`? | **`/docs/book/book/` only** | Source in `docs/book/src/` + `docs/book/book.toml` stays tracked |
| Add a `docs/book/README.md` with build instructions? | **Yes** | 1-paragraph note so future devs know how to build/preview |
| Move the build output to `public/` or another name? | **No** | mdbook's default is `book/`; renaming requires `book.toml` edits |

If any of these decisions should change, the user can override during the spec review.