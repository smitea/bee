# Bee Architecture Diagram (Draw.io + AI MCP) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a hand-authored Draw.io `.drawio` XML file at `docs/diagrams/bee-architecture-overview.drawio` containing Bee's high-level architecture overview, validated by a structural-checker script, with usage docs and an MCP server config wired into opencode for future AI-assisted edits.

**Architecture:** Pure documentation deliverable. No Rust / TypeScript / CI changes. Three new artifacts: a `.drawio` XML file authored by hand against the §"Diagram contents" list in the spec, a stdlib-only Python validator (`scripts/validate-drawio.py`) that asserts XML well-formedness + Draw.io structure + required-keyword presence + edge integrity, and a README explaining three ways to open the file and four steps to enable MCP-based editing. A single edit to `~/.config/opencode/opencode.jsonc` (outside the repo) registers `@hediet/drawio-mcp`.

**Tech Stack:** Draw.io XML (`.drawio`) — the native `mxfile` / `diagram` / `mxGraphModel` schema, which is plain UTF-8 XML. Python 3 standard library (`xml.etree.ElementTree`, `collections.Counter`, `pathlib`). `xmllint` (already at `/usr/bin/xmllint`) as an optional secondary well-formedness check. `@hediet/drawio-mcp` (npm) launched via `npx` from opencode's MCP registry.

**Reference docs:**
- Design: `docs/superpowers/specs/2026-08-12-bee-architecture-diagram-design.md`
- Domain vocabulary: `CONTEXT.md` (Data Plane, Control Plane, Raft Cluster, KV Cluster, Pipeline / Phase / Handler, Datasource / Adapter / Plugin)

**Pre-flight (read these before starting):**
- The spec's §"Diagram contents" (boxes + edges + visual conventions) — this is the acceptance contract for what goes into the `.drawio` file
- A `.drawio` file opened in any text editor shows the `mxfile` / `mxGraphModel` structure; no schema docs are needed for hand-authoring at this scale

**Out of scope:** any other `.drawio` file (Pipeline DAG lifecycle, plugin model, cluster topology are follow-ups); PNG/SVG export; CI integration of the validator; containerising the MCP server; edits to `Cargo.toml`, `app/`, `.github/`, `docker/`, or `docker-compose.yml`.

---

## File structure

**New files in this repo:**

| Path | Responsibility |
|---|---|
| `docs/diagrams/bee-architecture-overview.drawio` | The hand-authored diagram. mxfile root, 1 diagram, 17 cells (4 external actors + cluster boundary + 5 internal regions + 1 representative Pipeline Job container + 3 Phases + 1 Handler + 1 comment cell), 13 edges. |
| `docs/diagrams/README.md` | How to open the file (3 paths), how to enable MCP-based editing (4 steps), version conventions (commit prefix, sync rule, last-synced date). |
| `scripts/validate-drawio.py` | Stdlib-only Python validator. Exits 0 on success, non-zero on failure with a clear stderr message. Catches: XML parse errors, wrong root element, missing `<diagram>`/`<mxGraphModel>`, duplicate cell IDs, edge source/target not in cell-id set, total cell count < 10, missing required keywords. |

**Edited file outside this repo:**

| Path | Change |
|---|---|
| `~/.config/opencode/opencode.jsonc` | Append an `mcp.drawio` section registering `@hediet/drawio-mcp` via `npx`. Existing `plugin` field untouched. |

**Unchanged (deliberately):** `Cargo.toml`, `crates/*`, `app/*`, `tests/*`, `.github/*`, `docker/*`, `docker-compose.yml`, `Dockerfile*`, `.gitignore`, CI workflows, `CONTEXT.md`, `README.md`.

---

## Task 1: Validator script — failing test before implementation

**Files:**
- Create: `scripts/validate-drawio.py`

- [ ] **Step 1: Create `scripts/validate-drawio.py` with the full assertion set**

Write exactly this file:

```python
#!/usr/bin/env python3
"""Validate a Bee architecture diagram (.drawio XML).

Usage:
    python3 scripts/validate-drawio.py <path-to-drawio>

Exits 0 on success; non-zero with a clear stderr message on any failure.
Standard library only.
"""

import sys
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path

REQUIRED_KEYWORDS = [
    "User",
    "Plugins",
    "Bee Client",
    "AdminServer",
    "Control Plane",
    "Raft Cluster",
    "KV Cluster",
    "Data Plane",
    "Pipeline Job",
    "Phase",
    "Handler",
    "Datasources",
    "External Systems",
]

MIN_CELL_COUNT = 10


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("Usage: validate-drawio.py <path-to-drawio>", file=sys.stderr)
        return 2

    path = Path(argv[1])
    if not path.is_file():
        print(f"File not found: {path}", file=sys.stderr)
        return 2

    try:
        tree = ET.parse(path)
    except ET.ParseError as exc:
        print(f"XML parse error: {exc}", file=sys.stderr)
        return 1

    root = tree.getroot()
    errors: list[str] = []

    if root.tag != "mxfile":
        errors.append(f"Root element is <{root.tag}>, expected <mxfile>")

    diagrams = root.findall("diagram")
    if not diagrams:
        errors.append("No <diagram> element found")
    for i, diag in enumerate(diagrams):
        if diag.find("mxGraphModel") is None:
            errors.append(f"<diagram>[{i}] missing <mxGraphModel>")

    cells = root.findall(".//mxCell")
    ids = [c.get("id") for c in cells if c.get("id") is not None]
    duplicates = [i for i, n in Counter(ids).items() if n > 1]
    if duplicates:
        errors.append(f"Duplicate cell ids: {sorted(duplicates)}")

    if len(cells) < MIN_CELL_COUNT:
        errors.append(
            f"Cell count {len(cells)} < required minimum {MIN_CELL_COUNT}"
        )

    id_set = set(ids)
    for c in cells:
        if c.get("edge") == "1":
            src = c.get("source")
            tgt = c.get("target")
            if src and src not in id_set:
                errors.append(
                    f"Edge id={c.get('id')!r} source={src!r} not in cell id set"
                )
            if tgt and tgt not in id_set:
                errors.append(
                    f"Edge id={c.get('id')!r} target={tgt!r} not in cell id set"
                )

    label_texts = [c.get("value", "") for c in cells]
    all_labels = " | ".join(label_texts)
    missing = [k for k in REQUIRED_KEYWORDS if k not in all_labels]
    if missing:
        errors.append(f"Missing required keywords in cell labels: {missing}")

    if errors:
        for e in errors:
            print(f"FAIL: {e}", file=sys.stderr)
        return 1

    edge_count = sum(1 for c in cells if c.get("edge") == "1")
    print(
        f"OK: {path} — {len(cells)} cells, {edge_count} edges, "
        "all structural checks pass"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
```

- [ ] **Step 2: Run the validator with no diagram — expect non-zero exit**

Run:
```bash
python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio
```

Expected output (stderr):
```
File not found: docs/diagrams/bee-architecture-overview.drawio
```

Expected exit code: `2` (not zero). This is the TDD "test fails before implementation exists" state.

- [ ] **Step 3: Commit the failing validator**

```bash
git add scripts/validate-drawio.py
git commit -m "test(diagrams): add Draw.io structural validator (failing)"
```

---

## Task 2: Minimal `.drawio` skeleton — make validator pass on structure but fail on content

**Files:**
- Create: `docs/diagrams/bee-architecture-overview.drawio`

- [ ] **Step 1: Create the `docs/diagrams/` directory**

```bash
mkdir -p docs/diagrams
```

- [ ] **Step 2: Write the minimal valid `.drawio` skeleton**

Create `docs/diagrams/bee-architecture-overview.drawio` with this exact content (no cells yet — just the valid `mxfile` → `diagram` → `mxGraphModel` → `root` envelope plus the two standard root cells `id="0"` and `id="1"`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<mxfile host="app.diagrams.net" agent="Bee-docs" version="24.0.0">
  <diagram id="bee-arch-overview" name="Bee Architecture Overview">
    <mxGraphModel dx="1422" dy="794" grid="1" gridSize="10" guides="1" tooltips="1" connect="1" arrows="1" fold="1" page="1" pageScale="1" pageWidth="1400" pageHeight="900" math="0" shadow="0">
      <root>
        <mxCell id="0" />
        <mxCell id="1" parent="0" />
      </root>
    </mxGraphModel>
  </diagram>
</mxfile>
```

- [ ] **Step 3: Verify XML well-formedness with `xmllint`**

Run:
```bash
xmllint --noout docs/diagrams/bee-architecture-overview.drawio
```

Expected: no output, exit code 0. (If you ever see `parse error` or non-zero exit, the file is broken — fix the XML.)

- [ ] **Step 4: Run the validator — expect failure on cell count + keywords**

Run:
```bash
python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio
```

Expected stderr (two `FAIL:` lines):
```
FAIL: Cell count 2 < required minimum 10
FAIL: Missing required keywords in cell labels: ['User', 'Plugins', 'Bee Client', 'AdminServer', 'Control Plane', 'Raft Cluster', 'KV Cluster', 'Data Plane', 'Pipeline Job', 'Phase', 'Handler', 'Datasources', 'External Systems']
```

Expected exit code: `1`. This is expected — the skeleton is intentionally bare. The next two tasks populate it.

- [ ] **Step 5: Commit the skeleton**

```bash
git add docs/diagrams/bee-architecture-overview.drawio
git commit -m "docs(diagrams): add minimal Bee architecture .drawio skeleton"
```

---

## Task 3: Populate all 17 cells — make validator pass

**Files:**
- Modify: `docs/diagrams/bee-architecture-overview.drawio`

- [ ] **Step 1: Replace the `<root>` element with the full cell set**

Edit `docs/diagrams/bee-architecture-overview.drawio`. Replace the entire `<root>...</root>` block (currently two empty cells `id="0"` and `id="1"`) with the following block. Coordinates target a 1400×900 page; values can be nudged later in Draw.io without changing the XML's structural validity.

```xml
      <root>
        <mxCell id="0" />
        <mxCell id="1" parent="0" />

        <mxCell id="comment-sync" value="Last synced with CONTEXT.md on 2026-08-12" style="text;html=1;strokeColor=none;fillColor=none;align=left;verticalAlign=middle;whiteSpace=wrap;rounded=0;fontStyle=2;fontColor=#666666;" vertex="1" parent="1">
          <mxGeometry x="40" y="20" width="480" height="20" as="geometry" />
        </mxCell>

        <mxCell id="user" value="User" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
          <mxGeometry x="40" y="80" width="120" height="60" as="geometry" />
        </mxCell>
        <mxCell id="bee-client" value="Bee Client (Tauri)&#10;app/ Tauri 2.x + React" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
          <mxGeometry x="200" y="80" width="200" height="60" as="geometry" />
        </mxCell>
        <mxCell id="plugins-dir" value="Plugins Directory&#10;.so / .dylib" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
          <mxGeometry x="40" y="780" width="180" height="80" as="geometry" />
        </mxCell>
        <mxCell id="external-systems" value="External Systems&#10;Binance, MongoDB, ..." style="rounded=1;whiteSpace=wrap;html=1;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
          <mxGeometry x="1180" y="540" width="200" height="80" as="geometry" />
        </mxCell>

        <mxCell id="cluster-boundary" value="Bee Cluster (5× Nodes)" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#f5f5f5;strokeColor=#666666;strokeWidth=2;verticalAlign=top;fontStyle=1;fontSize=14;" vertex="1" parent="1">
          <mxGeometry x="40" y="180" width="1100" height="580" as="geometry" />
        </mxCell>

        <mxCell id="admin-server" value="AdminServer&#10;Deploy / Register / Inspect RPC" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#fff2cc;strokeColor=#d6b656;" vertex="1" parent="1">
          <mxGeometry x="500" y="220" width="200" height="60" as="geometry" />
        </mxCell>

        <mxCell id="control-plane" value="Control Plane&#10;membership · ownership&#10;orphans · work-stealing" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#d5e8d4;strokeColor=#82b366;" vertex="1" parent="1">
          <mxGeometry x="120" y="320" width="200" height="80" as="geometry" />
        </mxCell>
        <mxCell id="raft-cluster" value="Raft Cluster&#10;quorum commit" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#d5e8d4;strokeColor=#82b366;fontStyle=1;" vertex="1" parent="1">
          <mxGeometry x="500" y="330" width="200" height="60" as="geometry" />
        </mxCell>
        <mxCell id="kv-cluster" value="KV Cluster&#10;Task State · Checkpoints&#10;Saved Offsets" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#d5e8d4;strokeColor=#82b366;" vertex="1" parent="1">
          <mxGeometry x="880" y="320" width="200" height="80" as="geometry" />
        </mxCell>

        <mxCell id="data-plane" value="Data Plane (BRP) — Phase↔Phase · Remote Handler · I/O" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#e1d5e7;strokeColor=#9673a6;fontStyle=1;" vertex="1" parent="1">
          <mxGeometry x="120" y="440" width="940" height="60" as="geometry" />
        </mxCell>

        <mxCell id="pipeline-job" value="Pipeline Job (representative)" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#ffe6cc;strokeColor=#d79b00;verticalAlign=top;fontStyle=1;" vertex="1" parent="1">
          <mxGeometry x="120" y="540" width="500" height="200" as="geometry" />
        </mxCell>
        <mxCell id="phase-a" value="Phase A" style="rounded=0;whiteSpace=wrap;html=1;fillColor=#ffe6cc;strokeColor=#d79b00;" vertex="1" parent="1">
          <mxGeometry x="150" y="590" width="90" height="40" as="geometry" />
        </mxCell>
        <mxCell id="phase-b" value="Phase B" style="rounded=0;whiteSpace=wrap;html=1;fillColor=#ffe6cc;strokeColor=#d79b00;" vertex="1" parent="1">
          <mxGeometry x="270" y="590" width="90" height="40" as="geometry" />
        </mxCell>
        <mxCell id="phase-c" value="Phase C" style="rounded=0;whiteSpace=wrap;html=1;fillColor=#ffe6cc;strokeColor=#d79b00;" vertex="1" parent="1">
          <mxGeometry x="390" y="590" width="90" height="40" as="geometry" />
        </mxCell>
        <mxCell id="handler" value="Handler&#10;(stateless)" style="rounded=0;whiteSpace=wrap;html=1;fillColor=#ffe6cc;strokeColor=#d79b00;fontStyle=2;" vertex="1" parent="1">
          <mxGeometry x="290" y="660" width="140" height="40" as="geometry" />
        </mxCell>

        <mxCell id="datasources" value="Datasources&#10;(Providers + Adapters)" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#f8cecc;strokeColor=#b85450;fontStyle=1;" vertex="1" parent="1">
          <mxGeometry x="780" y="580" width="220" height="80" as="geometry" />
        </mxCell>
      </root>
```

- [ ] **Step 2: Verify XML well-formedness**

Run:
```bash
xmllint --noout docs/diagrams/bee-architecture-overview.drawio
```

Expected: no output, exit code 0.

- [ ] **Step 3: Run the validator — expect PASS (cells + keywords present, no edges yet)**

Run:
```bash
python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio
```

Expected stdout:
```
OK: docs/diagrams/bee-architecture-overview.drawio — 19 cells, 0 edges, all structural checks pass
```

(The cell count is 19: 2 root cells `id="0"` and `id="1"` + the `comment-sync` + 4 external actors + `cluster-boundary` + `admin-server` + `control-plane` + `raft-cluster` + `kv-cluster` + `data-plane` + `pipeline-job` + `phase-a` + `phase-b` + `phase-c` + `handler` + `datasources`.)

- [ ] **Step 4: Commit the populated cells**

```bash
git add docs/diagrams/bee-architecture-overview.drawio
git commit -m "docs(diagrams): populate Bee architecture cells (19 total)"
```

---

## Task 4: Add all 13 edges — make validator confirm edge integrity

**Files:**
- Modify: `docs/diagrams/bee-architecture-overview.drawio`

- [ ] **Step 1: Append the 13 edge cells inside `<root>`**

Edit `docs/diagrams/bee-architecture-overview.drawio`. Insert the following block immediately after the closing `<mxCell>` tag of the `datasources` cell (and before `</root>`):

```xml
        <mxCell id="edge-user-client" style="endArrow=classic;html=1;exitX=1;exitY=0.5;entryX=0;entryY=0.5;rounded=0;" edge="1" parent="1" source="user" target="bee-client">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-client-admin" style="endArrow=classic;html=1;exitX=0.5;exitY=1;entryX=0.5;entryY=0;rounded=0;" edge="1" parent="1" source="bee-client" target="admin-server">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-admin-raft" value="propose" style="endArrow=classic;html=1;exitX=0.5;exitY=1;entryX=0.5;entryY=0;rounded=0;" edge="1" parent="1" source="admin-server" target="raft-cluster">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-raft-control" value="apply" style="endArrow=classic;html=1;exitX=0;exitY=0.5;entryX=1;entryY=0.5;rounded=0;" edge="1" parent="1" source="raft-cluster" target="control-plane">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-raft-kv" value="apply" style="endArrow=classic;html=1;exitX=1;exitY=0.5;entryX=0;entryY=0.5;rounded=0;" edge="1" parent="1" source="raft-cluster" target="kv-cluster">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-control-dataplane" value="orphan detection · work-stealing" style="endArrow=classic;html=1;exitX=0.5;exitY=1;entryX=0.25;entryY=0;rounded=0;" edge="1" parent="1" source="control-plane" target="data-plane">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-control-job" value="assign Phase Assignment" style="endArrow=classic;html=1;exitX=0.5;exitY=1;entryX=0.25;entryY=0;rounded=0;" edge="1" parent="1" source="control-plane" target="pipeline-job">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-job-dataplane" value="Phase↔Phase data" style="endArrow=classic;html=1;exitX=1;exitY=0.25;entryX=0.5;entryY=1;rounded=0;" edge="1" parent="1" source="pipeline-job" target="data-plane">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-dataplane-datasources" style="endArrow=classic;html=1;exitX=1;exitY=0.5;entryX=0;entryY=0.5;rounded=0;" edge="1" parent="1" source="data-plane" target="datasources">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-datasources-external" style="endArrow=classic;html=1;exitX=1;exitY=0.5;entryX=0;entryY=0.5;rounded=0;" edge="1" parent="1" source="datasources" target="external-systems">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-datasources-kv" value="read checkpoints" style="endArrow=classic;html=1;exitX=0.5;exitY=0;entryX=0.5;entryY=1;rounded=0;" edge="1" parent="1" source="datasources" target="kv-cluster">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-kv-datasources" value="write offsets" style="endArrow=classic;html=1;exitX=0.5;exitY=1;entryX=0.5;entryY=0;rounded=0;" edge="1" parent="1" source="kv-cluster" target="datasources">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
        <mxCell id="edge-plugins-cluster" value="load at startup" style="endArrow=classic;html=1;exitX=1;exitY=0.5;entryX=0;entryY=0.5;rounded=0;" edge="1" parent="1" source="plugins-dir" target="cluster-boundary">
          <mxGeometry relative="1" as="geometry" />
        </mxCell>
```

- [ ] **Step 2: Verify XML well-formedness**

Run:
```bash
xmllint --noout docs/diagrams/bee-architecture-overview.drawio
```

Expected: no output, exit code 0.

- [ ] **Step 3: Run the validator — expect PASS with 13 edges**

Run:
```bash
python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio
```

Expected stdout:
```
OK: docs/diagrams/bee-architecture-overview.drawio — 32 cells, 13 edges, all structural checks pass
```

(32 = 19 from Task 3 + 13 new edge cells.)

If the validator reports `Edge id=... source=... not in cell id set` or `target=... not in cell id set`, the edge references a cell ID that doesn't exist. Re-check the `source=` and `target=` attributes against the cell IDs in Task 3.

- [ ] **Step 4: Commit the edges**

```bash
git add docs/diagrams/bee-architecture-overview.drawio
git commit -m "docs(diagrams): wire 13 edges between Bee architecture cells"
```

---

## Task 5: Write the README

**Files:**
- Create: `docs/diagrams/README.md`

- [ ] **Step 1: Create `docs/diagrams/README.md`**

Write exactly this file:

```markdown
# Bee Diagrams

Hand-authored architecture and flow diagrams for Bee. This folder currently holds the high-level architecture overview; additional diagrams (Pipeline DAG lifecycle, plugin model, cluster topology) are planned.

## Files

| File | Purpose |
|---|---|
| `bee-architecture-overview.drawio` | One-page overview of Bee's subsystems and their relationships. The source-of-truth diagram. |
| `../CONTEXT.md` (in repo root) | The prose vocabulary this diagram is anchored to. |
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
3. Confirm `~/.config/opencode/opencode.jsonc` has the `mcp.drawio` block that registers `@hediet/drawio-mcp` (see the spec at `../superpowers/specs/2026-08-12-bee-architecture-diagram-design.md`).
4. Restart opencode so the MCP server is registered, then ask the AI to read or edit the diagram. The AI calls tools such as `add_shape`, `add_connection`, `read_diagram`, `search_diagram`, `apply_template`; Draw.io updates in real time.

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
```

- [ ] **Step 2: Commit the README**

```bash
git add docs/diagrams/README.md
git commit -m "docs(diagrams): README with open paths, MCP setup, verify steps"
```

---

## Task 6: Register `@hediet/drawio-mcp` in opencode

**Files:**
- Edit: `~/.config/opencode/opencode.jsonc` (outside this repo)

- [ ] **Step 1: Verify the npm package name**

Run:
```bash
npm view @hediet/drawio-mcp name version
```

Expected stdout (approximate; version will vary):
```
@hediet/drawio-mcp
0.5.x
```

If the command fails with `npm ERR! 404 Not Found`, the package has been renamed. Try these fallbacks in order until one resolves:

```bash
npm view drawio-mcp name version
npm view @hediet/drawio name version
```

Whichever resolves, note the exact `<package-name>` and `<version>` you observed, and use it in step 2. If none resolve, stop and surface the failure to the user — the spec assumes the hediet package is available.

- [ ] **Step 2: Read the current opencode config**

Run:
```bash
cat ~/.config/opencode/opencode.jsonc
```

Expected: a small JSONC file with at least a `plugin` field. Capture the exact existing content — you will merge into it, not overwrite.

- [ ] **Step 3: Edit `~/.config/opencode/opencode.jsonc` to add the `mcp.drawio` section**

Open the file in your editor and append an `mcp` sibling to the existing `plugin` key. The final file should be valid JSONC and look like:

```jsonc
{
  "plugin": ["superpowers@git+https://github.com/obra/superpowers.git"],
  "mcp": {
    "drawio": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@hediet/drawio-mcp"],
      "enabled": true
    }
  }
}
```

If `<package-name>` from step 1 was not `@hediet/drawio-mcp`, substitute it inside `args`. Do not change the `plugin` field. Do not change any other field.

- [ ] **Step 4: Validate the JSONC is still well-formed JSON**

Run:
```bash
python3 -c "import json, sys; json.load(open('/Users/shaw/.config/opencode/opencode.jsonc'))" && echo OK
```

Expected stdout: `OK`. (Python's `json.load` tolerates trailing commas only in some versions; if it errors, the file has a syntax problem — open it and fix the comma placement.)

- [ ] **Step 5: Restart opencode to pick up the new MCP server**

This step is manual and outside the agent's control. Note in the README's "How to edit it with AI" section that a restart is required. Do not commit this step — the file lives in `$HOME`, not in this repo.

- [ ] **Step 6: No commit — this file is outside the repo**

There is no `git add` for this task. The change lives only in `~/.config/opencode/`.

---

## Task 7: Final verification — visual spot check and full acceptance walk-through

**Files:**
- Read-only: `docs/diagrams/bee-architecture-overview.drawio`
- Read-only: `scripts/validate-drawio.py`

- [ ] **Step 1: Re-run the full verification chain**

Run:
```bash
xmllint --noout docs/diagrams/bee-architecture-overview.drawio
python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio
```

Expected: both exit 0; the validator prints the success line with the cell/edge counts.

- [ ] **Step 2: Spot-check the spec's acceptance criteria**

Confirm each of these is satisfied. The validator handles the structural ones; the remaining are spot-checks:

- [ ] `docs/diagrams/bee-architecture-overview.drawio` exists.
- [ ] `xmllint --noout` on the file passes.
- [ ] `python3 scripts/validate-drawio.py docs/diagrams/bee-architecture-overview.drawio` exits 0.
- [ ] `docs/diagrams/README.md` exists and covers the four-step MCP edit flow, three direct-open paths, the version conventions, and the package-name verification step.
- [ ] `scripts/validate-drawio.py` exists and runs without external dependencies beyond the Python standard library. (`python3 -c "import ast; ast.parse(open('scripts/validate-drawio.py').read())"` confirms parse; no `import` of third-party modules is visible in the file.)
- [ ] `~/.config/opencode/opencode.jsonc` contains an `mcp.drawio` section with `command = "npx"` and `args = ["-y", "<verified-package-name>"]`.
- [ ] The diagram contains a cell whose label text matches each of the 13 keywords (validator-enforced).
- [ ] Every edge's `source` and `target` reference a valid existing cell (validator-enforced).
- [ ] A comment cell at the top of the diagram records `Last synced with CONTEXT.md on 2026-08-12`.

- [ ] **Step 3: Recommend a visual spot check**

Tell the user: "I recommend opening `docs/diagrams/bee-architecture-overview.drawio` in Draw.io once to confirm the layout looks right at your screen size. If any box overlaps or an edge crosses through a label, nudge the coordinates inside Draw.io — XML structure is unaffected as long as no cell IDs change."

This step is informational; it does not block task completion. Layout adjustments in Draw.io are an expected part of hand-authored diagrams and can be committed as a follow-up patch without re-running the validator (the structural assertions are coordinate-agnostic).

- [ ] **Step 4: No further commit**

Task 7 produces no file changes. All deliverables were committed in Tasks 1–5. The work is complete.

---

## Self-review

**Spec coverage:**

| Spec acceptance criterion | Task |
|---|---|
| `.drawio` file exists | Task 2 |
| `xmllint --noout` passes | Tasks 2/3/4 (Steps) + Task 7 |
| Validator exits 0 | Tasks 3/4 (Steps) + Task 7 |
| `README.md` exists with all four sections | Task 5 |
| Validator is stdlib-only | Task 1 (full source visible; no third-party imports) |
| `opencode.jsonc` has `mcp.drawio` with `npx -y @hediet/drawio-mcp` | Task 6 |
| Diagram contains all 13 keywords | Task 3 (validator enforces) |
| Every edge's source/target resolves | Task 4 (validator enforces) |
| Comment cell records `Last synced ... 2026-08-12` | Task 3 (`comment-sync` cell) |

**Placeholder scan:** No `TBD` / `TODO` / "implement later" / "similar to Task N". Every step has the actual content (validator code in Task 1, full XML in Tasks 2/3/4, full README in Task 5, full opencode.jsonc in Task 6, exact commands in Task 7).

**Type/name consistency:** Cell IDs (`user`, `bee-client`, `plugins-dir`, `external-systems`, `cluster-boundary`, `admin-server`, `control-plane`, `raft-cluster`, `kv-cluster`, `data-plane`, `pipeline-job`, `phase-a/b/c`, `handler`, `datasources`, `comment-sync`) are referenced identically across Tasks 3 and 4. Edge IDs (`edge-*`) are unique. The validator's keyword list (`REQUIRED_KEYWORDS` in Task 1) matches the labels used in Task 3's cells.

**Spec requirement with no task:** None. The "Last synced" comment cell is part of Task 3. The "package-name verification" step is Task 6 step 1.
