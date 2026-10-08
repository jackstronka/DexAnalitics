#!/usr/bin/env python3
"""Human-readable golden delta (R3): numeric leaves of insta ``*.snap`` JSON.

Compare two snapshots (or all ``*.snap`` changed vs a git base) and print a
Markdown table: fixture / metric / was / is / Δ / Δ%, sorted by |Δ|.

Usage:
  python tools/golden_delta.py --old old.snap --new new.snap
  python tools/golden_delta.py --git-base origin/main
  python tools/golden_delta.py --require-pr-section --changed-file path.snap --pr-body-file body.md
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
# D1: heading on its own line, optional colon / markdown emphasis (**Golden delta:**).
_SECTION_HEADING = re.compile(
    r"(?im)^\s*(?:#{1,6}\s+|\*\*)?golden[ \t]+delta\b(?:\*\*)?\s*:?\s*(?:\*\*)?\s*$"
)
# Same heading with justification on the same line.
_SECTION_INLINE = re.compile(
    r"(?im)^\s*(?:#{1,6}\s+|\*\*)?golden[ \t]+delta\b(?:\*\*)?\s*:\s*(?:\*\*)?\s+\S"
)
GATED_PATHSPECS = (
    "*.snap",
    "**/tests/fixtures/**",
    "**/snapshots/**",
    "openapi.json",
    "**/openapi.json",
)


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


def normalize_repo_path(path: str) -> str:
    return path.replace("\\", "/").lstrip("./")


def is_golden_gated_path(path: str) -> bool:
    """True when a PR path is in D1 scope (snap / fixtures / snapshots / openapi)."""
    p = normalize_repo_path(path)
    if not p:
        return False
    name = p.rsplit("/", 1)[-1]
    if name.endswith(".snap"):
        return True
    if name == "openapi.json":
        return True
    wrapped = f"/{p}/"
    if "/tests/fixtures/" in wrapped:
        return True
    if "/snapshots/" in wrapped:
        return True
    return False


def gated_paths(paths: Iterable[str]) -> list[str]:
    return [p for p in paths if is_golden_gated_path(p)]


def has_golden_delta_section(body: str | None) -> bool:
    """PR body has a Golden delta section with at least one line of substance."""
    if not body or not str(body).strip():
        return False
    text = str(body).replace("\r\n", "\n")
    if _SECTION_INLINE.search(text):
        return True
    match = _SECTION_HEADING.search(text)
    if not match:
        return False
    rest = text[match.end() :]
    next_heading = re.search(r"(?m)^#{1,6}\s+\S", rest)
    block = rest[: next_heading.start()] if next_heading else rest
    for line in block.splitlines():
        stripped = line.strip().strip("*_-")
        if stripped:
            return True
    return False


def check_pr_golden_delta_section(
    body: str | None, changed_paths: Iterable[str]
) -> tuple[bool, str]:
    gated = gated_paths(changed_paths)
    if not gated:
        return True, (
            "golden-delta: no gated fixture/snap/openapi changes; "
            "section not required"
        )
    if has_golden_delta_section(body):
        return True, "golden-delta: PR has Golden delta section"
    listed = "\n".join(f"  {p}" for p in gated)
    return False, (
        "golden-delta: gated files changed but PR body has no "
        "'Golden delta:' section (heading plus why).\n"
        f"Gated files:\n{listed}\n"
        "Add a Golden delta section (paste output of "
        "`python tools/golden_delta.py --git-base origin/main`).\n"
        "Number diffs do not fail this job; missing justification does."
    )


def git_changed_gated_paths(base: str) -> list[str]:
    proc = subprocess.run(
        ["git", "diff", "--name-only", f"{base}...HEAD", "--", *GATED_PATHSPECS],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise SystemExit(proc.stderr or f"git diff failed against {base}")
    paths = [line.strip() for line in proc.stdout.splitlines() if line.strip()]
    return gated_paths(paths)


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
    parser.add_argument(
        "--require-pr-section",
        action="store_true",
        help="D1: fail if gated paths changed and PR body lacks Golden delta",
    )
    parser.add_argument("--pr-body", help="PR description text for --require-pr-section")
    parser.add_argument(
        "--pr-body-file",
        help="Read PR description from this file (UTF-8)",
    )
    parser.add_argument(
        "--changed-file",
        action="append",
        default=[],
        help="Changed path for --require-pr-section (repeatable; skips git)",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    if args.require_pr_section:
        body = args.pr_body
        if args.pr_body_file:
            body = Path(args.pr_body_file).read_text(encoding="utf-8")
        if args.changed_file:
            paths = args.changed_file
        elif args.git_base:
            paths = git_changed_gated_paths(args.git_base)
        else:
            parser.error("--require-pr-section needs --changed-file or --git-base")
        ok, msg = check_pr_golden_delta_section(body, paths)
        print(msg)
        return 0 if ok else 1

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
