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