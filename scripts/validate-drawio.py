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