# Bee Diagrams

Hand-authored architecture and flow diagrams for Bee. This folder currently holds the high-level architecture overview; additional diagrams (Pipeline DAG lifecycle, plugin model, cluster topology) are planned.

## Files

| File | Purpose |
|---|---|
| `bee-architecture-overview.drawio` | One-page overview of Bee's subsystems and their relationships. The source-of-truth diagram. |
| `../../CONTEXT.md` (in repo root) | The prose vocabulary this diagram is anchored to. |
| `../../scripts/validate-drawio.py` | Validates every `.drawio` file in this folder for structural correctness. |

## How to open the diagram

Pick whichever you have:

1. **Browser (draw.io web app).** Go to <https://app.diagrams.net> and use *File → Open From → Device* to pick `bee-architecture-overview.drawio`. Or paste this URL into your browser:
   ```
   https://app.diagrams.net/?p=local&url=file:///Users/shaw/Developer/rust/bee/docs/diagrams/bee-architecture-overview.drawio
   ```
2. **macOS desktop app.** `open -a "draw.io" docs/diagrams/bee-architecture-overview.drawio` from the repo root (requires the Draw.io desktop app to be installed).
3. **From inside Draw.io.** File → Open From → Device → pick the file. The web and desktop apps both recognise `.drawio` files.

## How to edit it with AI (MCP path)

The Draw.io MCP server lets an AI assistant in opencode call tools that mutate the diagram live. Setup:

1. Open Draw.io (browser or desktop).
2. In Draw.io's menu, go to *Extras → Enable MCP Server*. This starts a local WebSocket server (default port `6006`).
3. Confirm `~/.config/opencode/opencode.jsonc` has the `mcp.drawio` block that registers `drawio-mcp` (see the spec at `../../superpowers/specs/2026-08-12-bee-architecture-diagram-design.md`).
4. Restart opencode so the MCP server is registered, then ask the AI to read or edit the diagram. The AI calls tools such as `add_shape`, `add_connection`, `read_diagram`, `search_diagram`, `apply_template`; Draw.io updates in real time.

Before editing, confirm the package name resolves: run `npm view drawio-mcp name version`. If that returns 404, try `npm view @hediet/drawio-mcp name version` and `npm view @hediet/drawio-mcp-server name version` in that order; substitute the resolving name in the `args` array inside `opencode.jsonc`. The four steps above assume the resolved name is `npx -y drawio-mcp`.

If the AI's tool calls time out with `connection refused`, the Draw.io MCP server is not enabled — return to step 2.

## Verify a diagram

```bash
python3 ../../scripts/validate-drawio.py bee-architecture-overview.drawio
```

Or for the XML well-formedness check alone (no structural assertions):

```bash
xmllint --noout bee-architecture-overview.drawio
```

Both should exit 0 on a healthy diagram.

## Version conventions

- Commit prefix: `docs(diagrams): ...`.
- The diagram contains a comment cell at the top recording `Last synced with CONTEXT.md on YYYY-MM-DD`. Update that date on any PR that changes either side.
- PRs that touch `CONTEXT.md` architecture wording should also update this diagram in the same PR if the change affects any of the boxes or edges. Reviewers are expected to enforce this.
