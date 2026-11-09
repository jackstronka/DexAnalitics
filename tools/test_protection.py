#!/usr/bin/env python3
"""R5: subsystem test-protection levels for MASTER_IMPLEMENTATION_PLAN §2.2.

Derive ``brak`` / ``unit`` / ``golden`` / ``golden+niezmiennik`` from an R2
``report.json`` (or R4 ``trend.json`` last-snapshot fields) using
``ci/subsystem-protection.toml``.

Usage:
  python tools/test_protection.py --report tools/fixtures/r5_report/report.json
  python tools/test_protection.py --report target/test-report/report.json --check-master
  python tools/test_protection.py --report … --write-master
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    import tomllib  # py311+
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MAP = REPO_ROOT / "ci" / "subsystem-protection.toml"
DEFAULT_MASTER = REPO_ROOT / "doc" / "MASTER_IMPLEMENTATION_PLAN.md"

LEVELS = ("brak", "unit", "golden", "golden+niezmiennik")
LEVEL_RANK = {name: i for i, name in enumerate(LEVELS)}


@dataclass(frozen=True)
class SubsystemSpec:
    name: str
    areas: tuple[str, ...]
    force: str | None


def load_map(path: Path) -> list[SubsystemSpec]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    out: list[SubsystemSpec] = []
    for row in data.get("subsystem") or []:
        force = row.get("force")
        if force is not None:
            force = str(force).strip()
            if force not in LEVEL_RANK:
                raise ValueError(f"unknown force level {force!r} for {row.get('name')}")
        out.append(
            SubsystemSpec(
                name=str(row["name"]),
                areas=tuple(str(a) for a in (row.get("areas") or [])),
                force=force,
            )
        )
    if not out:
        raise ValueError(f"no [[subsystem]] entries in {path}")
    return out


def area_level(area: dict[str, Any]) -> str:
    golden = int(area.get("golden") or area.get("golden_last") or 0)
    niezm = int(area.get("niezm") or area.get("niezm_last") or 0)
    tests = int(area.get("tests") or area.get("tests_last") or 0)
    if golden > 0 and niezm > 0:
        return "golden+niezmiennik"
    if golden > 0:
        return "golden"
    if tests > 0:
        return "unit"
    return "brak"


def max_level(levels: list[str]) -> str:
    if not levels:
        return "brak"
    return max(levels, key=lambda x: LEVEL_RANK[x])


def areas_from_report(doc: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Accept R2 report.json or R4 trend.json (``areas`` list)."""
    rows = doc.get("areas") or []
    out: dict[str, dict[str, Any]] = {}
    for a in rows:
        label = str(a.get("label") or "")
        if label:
            out[label] = a
    return out


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def compute_levels(
    specs: list[SubsystemSpec],
    areas_by_label: dict[str, dict[str, Any]],
) -> list[tuple[str, str, str]]:
    """Return list of (name, level, note)."""
    rows: list[tuple[str, str, str]] = []
    for spec in specs:
        if spec.force is not None:
            note = "force"
            if spec.areas:
                note = f"force (areas={', '.join(spec.areas)})"
            rows.append((spec.name, spec.force, note))
            continue
        derived: list[str] = []
        missing: list[str] = []
        for label in spec.areas:
            if label in areas_by_label:
                derived.append(area_level(areas_by_label[label]))
            else:
                missing.append(label)
        level = max_level(derived)
        note_parts: list[str] = []
        if derived:
            note_parts.append("areas=" + "+".join(spec.areas))
        if missing:
            note_parts.append("missing=" + ",".join(missing))
        if not derived and not missing:
            note_parts.append("no areas")
        rows.append((spec.name, level, "; ".join(note_parts) or "—"))
    return rows


def render_markdown_table(rows: list[tuple[str, str, str]]) -> str:
    lines = [
        "| Subsystem | Ochrona testami | Source |",
        "| --------- | --------------- | ------ |",
    ]
    for name, level, note in rows:
        lines.append(f"| {name} | **{level}** | {note} |")
    return "\n".join(lines) + "\n"


_SECTION_RE = re.compile(
    r"(### 2\.2 Dojrzałość produktowa \(subsystemy\)\n\n)(.*?)(\n### 2\.3 )",
    re.S,
)
_ROW_RE = re.compile(
    r"^\|\s*(?P<name>[^|]+?)\s*\|\s*(?P<ocena>[^|]+?)\s*\|\s*(?P<ochrona>[^|]+?)\s*\|\s*(?P<uwaga>[^|]*)\|\s*$"
)


def parse_master_protection(text: str) -> dict[str, str]:
    m = _SECTION_RE.search(text)
    if not m:
        raise ValueError("§2.2 section not found in MASTER plan")
    body = m.group(2)
    out: dict[str, str] = {}
    for line in body.splitlines():
        if not line.startswith("|") or line.startswith("| ---") or "Subsystem" in line:
            continue
        rm = _ROW_RE.match(line)
        if not rm:
            continue
        name = rm.group("name").strip()
        raw = rm.group("ochrona").strip()
        level = raw.replace("**", "").strip()
        if level in LEVEL_RANK:
            out[name] = level
    return out


def build_master_section(
    existing_text: str,
    levels: dict[str, str],
) -> str:
    """Rebuild §2.2 table keeping Ocena/Uwaga, updating Ochrona testami."""
    m = _SECTION_RE.search(existing_text)
    if not m:
        raise ValueError("§2.2 section not found in MASTER plan")

    legend = (
        "**Ochrona testami** (R5): `brak` / `unit` / `golden` / `golden+niezmiennik` — "
        "z raportu R2/R4 + mapa [`ci/subsystem-protection.toml`](../ci/subsystem-protection.toml); "
        "odśwież przy zamknięciu fali (`make test-protection` / `--write-master`).\n\n"
    )

    # Collect existing ocena/uwaga from current rows (3- or 4-col).
    old_rows: list[tuple[str, str, str]] = []
    for line in m.group(2).splitlines():
        if not line.startswith("|") or "Subsystem" in line or line.startswith("| ---"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) == 3:
            name, ocena, uwaga = cells
            old_rows.append((name, ocena, uwaga))
        elif len(cells) >= 4:
            name, ocena, _och, uwaga = cells[0], cells[1], cells[2], cells[3]
            old_rows.append((name, ocena, uwaga))

    lines = [
        "| Subsystem | Ocena | Ochrona testami | Uwaga |",
        "| --------- | ----- | --------------- | ----- |",
    ]
    for name, ocena, uwaga in old_rows:
        level = levels.get(name, "brak")
        lines.append(f"| {name} | {ocena} | **{level}** | {uwaga} |")

    new_body = legend + "\n".join(lines) + "\n"
    return existing_text[: m.start(2)] + new_body + existing_text[m.end(2) :]


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="R5 subsystem protection levels")
    p.add_argument("--map", type=Path, default=DEFAULT_MAP)
    src = p.add_mutually_exclusive_group(required=True)
    src.add_argument("--report", type=Path, help="R2 report.json")
    src.add_argument("--trend", type=Path, help="R4 trend.json")
    p.add_argument("--master", type=Path, default=DEFAULT_MASTER)
    p.add_argument(
        "--check-master",
        action="store_true",
        help="Fail if MASTER §2.2 Ochrona column ≠ computed levels",
    )
    p.add_argument(
        "--write-master",
        action="store_true",
        help="Rewrite MASTER §2.2 Ochrona column from computed levels",
    )
    p.add_argument("--json", action="store_true", help="Print JSON instead of markdown")
    args = p.parse_args(argv)

    specs = load_map(args.map)
    doc = load_json(args.report if args.report else args.trend)
    areas = areas_from_report(doc)
    rows = compute_levels(specs, areas)
    levels = {name: level for name, level, _ in rows}

    if args.write_master:
        text = args.master.read_text(encoding="utf-8")
        new_text = build_master_section(text, levels)
        args.master.write_text(new_text, encoding="utf-8", newline="\n")
        print(f"wrote {args.master}", file=sys.stderr)

    if args.check_master:
        text = args.master.read_text(encoding="utf-8")
        parsed = parse_master_protection(text)
        errors: list[str] = []
        for name, level, _ in rows:
            got = parsed.get(name)
            if got is None:
                errors.append(f"missing row: {name}")
            elif got != level:
                errors.append(f"{name}: master={got} computed={level}")
        for name in parsed:
            if name not in levels:
                errors.append(f"extra master row (not in map): {name}")
        if errors:
            print("test_protection --check-master FAILED:", file=sys.stderr)
            for e in errors:
                print(f"  {e}", file=sys.stderr)
            return 1
        print("test_protection --check-master OK", file=sys.stderr)

    if args.json:
        print(json.dumps([{"name": n, "level": lv, "note": note} for n, lv, note in rows], indent=2))
    else:
        print(render_markdown_table(rows), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
