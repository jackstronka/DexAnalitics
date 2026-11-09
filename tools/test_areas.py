#!/usr/bin/env python3
"""R1: assign each test id to a project area via ci/test-areas.toml.

Normalized id:
  nextest oneline ``pkg path::to::test`` → ``pkg::path::to::test``
  web / tools paths keep forward slashes.

Longest matching ``prefixes`` entry wins. Label ``Inne`` (id ``inne``) is the
fallback bucket; ``--strict`` fails if any test lands only on that area or is
completely unmatched (should not happen if catch-all prefixes exist).

Usage:
  cargo nextest list --workspace --lib --bins --tests --message-format oneline \\
    | python tools/test_areas.py --stdin
  python tools/test_areas.py --file ids.txt --web-glob 'web/src/**/*.test.ts'
  python tools/test_areas.py --check-map   # validate toml only
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

try:
    import tomllib  # py311+
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MAP = REPO_ROOT / "ci" / "test-areas.toml"
INNE_ID = "inne"


@dataclass(frozen=True)
class Area:
    id: str
    label: str
    prefixes: tuple[str, ...]


@dataclass(frozen=True)
class Assignment:
    test_id: str
    area_id: str
    area_label: str
    matched_prefix: str | None
    fallback_only: bool


def normalize_test_id(raw: str) -> str | None:
    line = raw.strip()
    if not line or line.startswith("#"):
        return None
    # nextest oneline: "clmm-lp-api openapi::tests::foo"
    if " " in line and not line.startswith("web/") and not line.startswith("tools/"):
        pkg, _, rest = line.partition(" ")
        if pkg and rest and "::" in rest:
            return f"{pkg}::{rest}"
        if pkg and rest:
            return f"{pkg}::{rest}"
    # already normalized or a path
    return line.replace("\\", "/")


def load_areas(path: Path) -> list[Area]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    areas_raw = data.get("area") or data.get("areas") or []
    if not isinstance(areas_raw, list) or not areas_raw:
        raise SystemExit(f"{path}: expected [[area]] tables")
    areas: list[Area] = []
    for row in areas_raw:
        prefixes = tuple(str(p) for p in row.get("prefixes") or [])
        areas.append(
            Area(
                id=str(row["id"]),
                label=str(row["label"]),
                prefixes=prefixes,
            )
        )
    return areas


def assign(test_id: str, areas: list[Area]) -> Assignment:
    best: tuple[int, Area, str] | None = None
    for area in areas:
        for prefix in area.prefixes:
            if test_id == prefix or test_id.startswith(prefix):
                # require boundary: end or :: or / after prefix when prefix shorter
                if len(test_id) > len(prefix):
                    nxt = test_id[len(prefix)]
                    if nxt not in (":", "/", ".", "-"):
                        # allow "clmm-lp-api" matching "clmm-lp-api::..." via ':'
                        # and web/src/lib/foo matching web/src/lib/foo.test.ts via '.'
                        if not (prefix.endswith("/") or prefix.endswith("::")):
                            if nxt not in (":", "/"):
                                continue
                score = len(prefix)
                if best is None or score > best[0]:
                    best = (score, area, prefix)
    if best is None:
        return Assignment(
            test_id=test_id,
            area_id=INNE_ID,
            area_label="Inne",
            matched_prefix=None,
            fallback_only=True,
        )
    _, area, prefix = best
    fallback_only = area.id == INNE_ID
    return Assignment(
        test_id=test_id,
        area_id=area.id,
        area_label=area.label,
        matched_prefix=prefix,
        fallback_only=fallback_only,
    )


def collect_web_ids(glob_pat: str) -> list[str]:
    # Simple recursive glob under repo root; pattern like web/src/**/*.test.ts
    if "**" in glob_pat:
        root, _, suffix = glob_pat.partition("**/")
        base = REPO_ROOT / root.rstrip("/")
        paths = sorted(base.rglob(suffix)) if base.is_dir() else []
    else:
        paths = sorted(REPO_ROOT.glob(glob_pat))
    out: list[str] = []
    for p in paths:
        try:
            rel = p.relative_to(REPO_ROOT).as_posix()
        except ValueError:
            continue
        out.append(rel)
    return out


def read_ids(lines: Iterable[str]) -> list[str]:
    ids: list[str] = []
    seen: set[str] = set()
    for raw in lines:
        nid = normalize_test_id(raw)
        if nid and nid not in seen:
            seen.add(nid)
            ids.append(nid)
    return ids


def render_markdown(assignments: list[Assignment]) -> str:
    counts: Counter[str] = Counter()
    fallback = 0
    for a in assignments:
        counts[a.area_label] += 1
        if a.fallback_only:
            fallback += 1
    lines = [
        f"Test areas — {len(assignments)} tests | Inne/fallback {fallback}",
        "",
        "| Obszar | Testy |",
        "| ------ | ----- |",
    ]
    for label, n in sorted(counts.items(), key=lambda x: (-x[1], x[0])):
        lines.append(f"| {label} | {n} |")
    if fallback:
        lines.append("")
        lines.append(f"warning: {fallback} test(s) mapped only to Inne / unmatched")
        for a in assignments:
            if a.fallback_only:
                lines.append(f"  - {a.test_id}")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--map", type=Path, default=DEFAULT_MAP, help="path to test-areas.toml")
    ap.add_argument("--stdin", action="store_true", help="read nextest oneline ids from stdin")
    ap.add_argument("--file", type=Path, help="file with one test id per line")
    ap.add_argument(
        "--web-glob",
        action="append",
        default=[],
        help="also include matching web/tool test files (repeatable)",
    )
    ap.add_argument("--json", action="store_true", help="emit JSON assignments")
    ap.add_argument("--strict", action="store_true", help="exit 1 if any Inne/fallback")
    ap.add_argument("--check-map", action="store_true", help="only validate the toml loads")
    args = ap.parse_args(argv)

    areas = load_areas(args.map)
    if args.check_map:
        print(f"ok: {args.map} ({len(areas)} areas)")
        return 0

    raw_lines: list[str] = []
    if args.stdin:
        raw_lines.extend(sys.stdin.read().splitlines())
    if args.file:
        raw_lines.extend(args.file.read_text(encoding="utf-8").splitlines())
    for g in args.web_glob:
        raw_lines.extend(collect_web_ids(g))

    if not raw_lines:
        ap.error("provide --stdin, --file, and/or --web-glob (or --check-map)")

    ids = read_ids(raw_lines)
    assignments = [assign(i, areas) for i in ids]
    fallback_n = sum(1 for a in assignments if a.fallback_only)

    if args.json:
        payload = {
            "map": str(args.map),
            "total": len(assignments),
            "fallback": fallback_n,
            "assignments": [
                {
                    "id": a.test_id,
                    "area_id": a.area_id,
                    "area": a.area_label,
                    "prefix": a.matched_prefix,
                    "fallback_only": a.fallback_only,
                }
                for a in assignments
            ],
        }
        json.dump(payload, sys.stdout, indent=2)
        sys.stdout.write("\n")
    else:
        sys.stdout.write(render_markdown(assignments))

    if args.strict and fallback_n:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
