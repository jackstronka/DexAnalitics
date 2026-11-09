#!/usr/bin/env python3
"""Aggregate cargo-mutants outcomes for D5 scoped runs.

Looks for ``outcomes.json`` (or ``mutants.out/outcomes.json``) under each
subdir of ``--root``. Prints survivor rate; exits 1 if any module has
missed/survived share >= ``--max-survive-pct`` (default 10).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def load_outcomes(dir_path: Path) -> dict[str, Any] | None:
    for cand in (
        dir_path / "outcomes.json",
        dir_path / "mutants.out" / "outcomes.json",
    ):
        if cand.is_file():
            return json.loads(cand.read_text(encoding="utf-8"))
    return None


def count_outcomes(doc: dict[str, Any]) -> dict[str, int]:
    """Return tallies: caught, missed, unviable, timeout, other."""
    tallies = {
        "caught": 0,
        "missed": 0,
        "unviable": 0,
        "timeout": 0,
        "other": 0,
        "total_tested": 0,
    }
    # cargo-mutants ≥0.18: top-level caught/missed/… counts (preferred).
    if any(k in doc for k in ("caught", "missed", "timeout", "unviable")):
        for key in ("caught", "missed", "unviable", "timeout"):
            if key in doc:
                tallies[key] = int(doc[key])
        tallies["total_tested"] = (
            tallies["caught"] + tallies["missed"] + tallies["timeout"]
        )
        return tallies

    # Nested summary object (older / fixture shape).
    summary = doc.get("summary") or {}
    if isinstance(summary, dict) and any(
        k in summary for k in ("caught", "missed", "timeout", "unviable")
    ):
        for key in ("caught", "missed", "unviable", "timeout"):
            if key in summary:
                tallies[key] = int(summary[key])
        tallies["total_tested"] = (
            tallies["caught"] + tallies["missed"] + tallies["timeout"]
        )
        return tallies

    outcomes = doc.get("outcomes") or doc.get("mutants") or []
    if isinstance(outcomes, dict):
        outcomes = outcomes.get("outcomes") or []
    for row in outcomes:
        if not isinstance(row, dict):
            continue
        # Formats: "caught", "Caught", "CaughtMutant", "MissedMutant", …
        raw = (
            row.get("summary")
            or row.get("outcome")
            or row.get("result")
            or ""
        )
        name = str(raw).strip().lower()
        if "caught" in name or name in ("killed",):
            tallies["caught"] += 1
            tallies["total_tested"] += 1
        elif "missed" in name or name in ("survived", "alive"):
            tallies["missed"] += 1
            tallies["total_tested"] += 1
        elif "timeout" in name or name in ("timedout", "timed_out"):
            tallies["timeout"] += 1
            tallies["total_tested"] += 1
        elif "unviable" in name or name in ("unbuildable",):
            tallies["unviable"] += 1
        elif name in ("success",) or "baseline" in name:
            # Unmutated baseline row — ignore.
            pass
        else:
            tallies["other"] += 1
    return tallies


def survive_pct(t: dict[str, int]) -> float | None:
    n = t["total_tested"]
    if n <= 0:
        return None
    return 100.0 * (t["missed"] + t["timeout"]) / n


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="D5 cargo-mutants summary")
    p.add_argument("--root", type=Path, required=True)
    p.add_argument("--out", type=Path, help="Write JSON summary")
    p.add_argument("--md", type=Path, help="Write Markdown summary")
    p.add_argument(
        "--max-survive-pct",
        type=float,
        default=10.0,
        help="Fail if any module survive+timeout pct >= this (default 10)",
    )
    p.add_argument(
        "--allow-missing",
        action="store_true",
        help="Do not fail when a subdir has no outcomes.json yet",
    )
    args = p.parse_args(argv)

    root: Path = args.root
    if not root.is_dir():
        print(f"missing root dir: {root}", file=sys.stderr)
        return 2

    modules: list[dict[str, Any]] = []
    lines = [
        "| Module | Caught | Missed | Timeout | Unviable | Survive% |",
        "| ------ | -----: | -----: | ------: | -------: | -------: |",
    ]
    fail = False
    for child in sorted(root.iterdir()):
        if not child.is_dir():
            continue
        doc = load_outcomes(child)
        if doc is None:
            if not args.allow_missing:
                print(f"no outcomes.json under {child}", file=sys.stderr)
                fail = True
            modules.append({"module": child.name, "missing": True})
            lines.append(f"| {child.name} | — | — | — | — | missing |")
            continue
        t = count_outcomes(doc)
        pct = survive_pct(t)
        row = {
            "module": child.name,
            **t,
            "survive_pct": pct,
        }
        modules.append(row)
        pct_s = "n/a" if pct is None else f"{pct:.1f}"
        lines.append(
            f"| {child.name} | {t['caught']} | {t['missed']} | {t['timeout']} "
            f"| {t['unviable']} | {pct_s} |"
        )
        if pct is not None and pct >= args.max_survive_pct:
            fail = True

    md = "\n".join(lines) + "\n"
    print(md, end="")
    if args.md:
        args.md.parent.mkdir(parents=True, exist_ok=True)
        args.md.write_text(md, encoding="utf-8")
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(
            json.dumps(
                {
                    "max_survive_pct": args.max_survive_pct,
                    "modules": modules,
                    "ok": not fail,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    if fail:
        print(
            f"D5: survivor rate >= {args.max_survive_pct}% in at least one module "
            "(or missing outcomes)",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
