#!/usr/bin/env python3
"""Human-readable golden delta (R3): numeric leaves of insta ``*.snap`` JSON.

Compare two snapshots (or all ``*.snap`` changed vs a git base) and print a
Markdown table: fixture / metric / was / is / Δ / Δ%, sorted by |Δ|.

Usage:
  python tools/golden_delta.py --old old.snap --new new.snap
  python tools/golden_delta.py --git-base origin/main
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any, Iterable

NUMBER_RE = re.compile(r"^-?\d+(?:\.\d+)?$")


def parse_snap_json(text: str) -> Any:
    """Insta json snapshot: YAML header, then ``---``, then JSON body."""
    raw = text.replace("\r\n", "\n").strip()
    if not raw:
        return None
    marker = "\n---\n"
    idx = raw.find(marker)
    if raw.startswith("---") and idx != -1:
        body = raw[idx + len(marker) :].strip()
    else:
        body = raw
    if not body:
        return None
    return json.loads(body)


def try_number(value: Any) -> Decimal | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return Decimal(value)
    if isinstance(value, float):
        return Decimal(str(value))
    if isinstance(value, str) and NUMBER_RE.match(value.strip()):
        try:
            return Decimal(value.strip())
        except InvalidOperation:
            return None
    return None


def walk_numeric(obj: Any, prefix: str = "") -> dict[str, Decimal]:
    out: dict[str, Decimal] = {}
    if isinstance(obj, dict):
        for key, val in obj.items():
            path = f"{prefix}.{key}" if prefix else str(key)
            out.update(walk_numeric(val, path))
    elif isinstance(obj, list):
        for i, val in enumerate(obj):
            path = f"{prefix}[{i}]"
            out.update(walk_numeric(val, path))
    else:
        num = try_number(obj)
        if num is not None:
            out[prefix or "(root)"] = num
    return out


def fmt_dec(value: Decimal | None) -> str:
    if value is None:
        return "-"
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text or "0"


def pct_change(old: Decimal | None, new: Decimal | None) -> str:
    if old is None or new is None or old == 0:
        return "-"
    pct = (new - old) / abs(old) * Decimal(100)
    return f"{fmt_dec(pct)}%"


def diff_maps(
    fixture: str,
    old_map: dict[str, Decimal],
    new_map: dict[str, Decimal],
) -> list[tuple[Decimal, str, str, Decimal | None, Decimal | None]]:
    keys = set(old_map) | set(new_map)
    rows: list[tuple[Decimal, str, str, Decimal | None, Decimal | None]] = []
    for key in keys:
        old = old_map.get(key)
        new = new_map.get(key)
        if old == new:
            continue
        if old is None:
            delta = abs(new) if new is not None else Decimal(0)
        elif new is None:
            delta = abs(old)
        else:
            delta = abs(new - old)
        rows.append((delta, fixture, key, old, new))
    return rows


def render_table(
    rows: list[tuple[Decimal, str, str, Decimal | None, Decimal | None]],
) -> str:
    lines = [
        "## Golden delta",
        "",
        "| Fixture | Metric | Was | Is | Delta | Delta% |",
        "| --- | --- | ---: | ---: | ---: | ---: |",
    ]
    if not rows:
        lines.append("| - | *(no numeric leaf changes)* | - | - | - | - |")
        lines.extend(["", "Copy this table into the PR **Golden delta** section."])
        return "\n".join(lines) + "\n"

    rows = sorted(rows, key=lambda r: (-r[0], r[1], r[2]))
    for _abs, fixture, metric, old, new in rows:
        if old is None:
            delta_s = fmt_dec(new)
        elif new is None:
            delta_s = fmt_dec(-old)
        else:
            delta_s = fmt_dec(new - old)
        lines.append(
            f"| `{fixture}` | `{metric}` | {fmt_dec(old)} | {fmt_dec(new)} | {delta_s} | {pct_change(old, new)} |"
        )
    lines.extend(["", "Copy this table into the PR **Golden delta** section."])
    return "\n".join(lines) + "\n"


def snap_name(path: str) -> str:
    return Path(path).name.removesuffix(".snap")


def git_show(ref: str, path: str) -> str | None:
    proc = subprocess.run(
        ["git", "show", f"{ref}:{path}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        return None
    return proc.stdout


def git_changed_snaps(base: str) -> list[tuple[str, str]]:
    """Return (status, path) for snap files vs ``base...HEAD`` (A/M/D)."""
    proc = subprocess.run(
        ["git", "diff", "--name-status", f"{base}...HEAD", "--", "*.snap"],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise SystemExit(proc.stderr or f"git diff failed against {base}")
    out: list[tuple[str, str]] = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split("\t", 1)
        if len(parts) != 2:
            continue
        status, path = parts[0][0], parts[1]
        if path.endswith(".snap"):
            out.append((status, path))
    return out


def rows_from_texts(fixture: str, old_text: str | None, new_text: str | None) -> list:
    old_obj = parse_snap_json(old_text) if old_text else None
    new_obj = parse_snap_json(new_text) if new_text else None
    old_map = walk_numeric(old_obj) if old_obj is not None else {}
    new_map = walk_numeric(new_obj) if new_obj is not None else {}
    return diff_maps(fixture, old_map, new_map)


def rows_from_git(base: str) -> tuple[list, list[str]]:
    changed = git_changed_snaps(base)
    rows: list = []
    files: list[str] = []
    for status, path in changed:
        files.append(path)
        old_text = None if status == "A" else git_show(base, path)
        new_text = None if status == "D" else Path(path).read_text(encoding="utf-8")
        rows.extend(rows_from_texts(snap_name(path), old_text, new_text))
    return rows, files


def main(argv: Iterable[str] | None = None) -> int:
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, OSError, ValueError):
        pass
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old", help="Old .snap path")
    parser.add_argument("--new", help="New .snap path")
    parser.add_argument("--fixture", help="Label for --old/--new (default: new filename)")
    parser.add_argument(
        "--git-base",
        help="Compare *.snap on HEAD against this ref (e.g. origin/main)",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    if args.git_base:
        rows, files = rows_from_git(args.git_base)
        if not files:
            print("## Golden delta\n\nNo `*.snap` files changed vs "
                  f"`{args.git_base}`.\n")
            return 0
        sys.stdout.write(render_table(rows))
        return 0

    if not args.old or not args.new:
        parser.error("pass --old and --new, or --git-base")

    old_text = Path(args.old).read_text(encoding="utf-8")
    new_text = Path(args.new).read_text(encoding="utf-8")
    fixture = args.fixture or snap_name(args.new)
    rows = rows_from_texts(fixture, old_text, new_text)
    sys.stdout.write(render_table(rows))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
