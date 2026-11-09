#!/usr/bin/env python3
"""R2: aggregate JUnit (nextest + vitest) into a per-area Markdown + JSON report.

Uses ``ci/test-areas.toml`` via ``test_areas.assign``. Same layout locally and in CI.

Usage:
  python tools/test_report.py --junit path1.xml --junit path2.xml \\
    --out-dir target/test-report --git-ref HEAD
  python tools/test_report.py --check-fixtures   # unit helpers via test_test_report.py
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path

import test_areas as ta

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MAP = REPO_ROOT / "ci" / "test-areas.toml"

_GOLDEN_RE = re.compile(r"golden|insta|snapshot", re.I)
_NIEZM_RE = re.compile(r"proptest|identity|roundtrip|invariant|niezm", re.I)
_COMPILE_RE = re.compile(r"compil|cannot find|unresolved import|error\[E\d+", re.I)
_NET_RE = re.compile(r"unshare|network|connection refused|timed out|dns", re.I)


@dataclass
class CaseResult:
    test_id: str
    status: str  # pass | fail | ignored
    time_s: float
    message: str = ""
    area_id: str = ""
    area_label: str = ""
    fallback_only: bool = False
    is_golden: bool = False
    is_niezm: bool = False
    fail_class: str | None = None


@dataclass
class AreaStats:
    label: str
    tests: int = 0
    fails: int = 0
    ignored: int = 0
    golden: int = 0
    niezm: int = 0
    time_s: float = 0.0


@dataclass
class Report:
    title: str
    git_ref: str
    commit: str
    generated_at: str
    pass_n: int = 0
    fail_n: int = 0
    ignored_n: int = 0
    time_s: float = 0.0
    areas: list[dict] = field(default_factory=list)
    failures: list[dict] = field(default_factory=list)
    fallback_n: int = 0


def _text(el: ET.Element | None) -> str:
    if el is None:
        return ""
    parts: list[str] = []
    if el.text:
        parts.append(el.text)
    for child in el:
        if child.text:
            parts.append(child.text)
        if child.tail:
            parts.append(child.tail)
    return "".join(parts).strip()


def junit_case_id(suite_name: str, classname: str, name: str) -> str:
    """Build a stable id for area matching."""
    name = (name or "").replace("\\", "/")
    classname = (classname or "").replace("\\", "/")
    suite_name = (suite_name or "").replace("\\", "/")

    # Vitest / path-like
    for candidate in (classname, name, suite_name):
        if candidate.endswith((".ts", ".tsx", ".js", ".jsx")) or "/src/" in candidate:
            # strip suite prefixes like "web/src/lib/foo.test.ts"
            path = candidate
            if path.startswith("web/") or "web/src/" in path:
                idx = path.find("web/")
                return path[idx:] if idx >= 0 else path
            if path.startswith("src/"):
                return f"web/{path}"
            return path

    # nextest: suite is binary id (clmm-lp-api or clmm-lp-data::session_gl_integration)
    # name is module::...::test_fn
    binary = suite_name or classname.split("::", 1)[0]
    if name.startswith(f"{binary}::"):
        return name
    if "::" in name:
        return f"{binary}::{name}"
    if classname and classname != binary and "::" in classname:
        return f"{classname}::{name}" if name else classname
    return f"{binary}::{name}" if name else binary


def parse_junit(path: Path) -> list[CaseResult]:
    tree = ET.parse(path)
    root = tree.getroot()
    suites = root.findall("testsuite")
    if root.tag == "testsuite":
        suites = [root]
    cases: list[CaseResult] = []
    for suite in suites:
        suite_name = suite.attrib.get("name", "")
        for tc in suite.findall("testcase"):
            name = tc.attrib.get("name", "")
            classname = tc.attrib.get("classname", suite_name)
            time_s = float(tc.attrib.get("time", "0") or 0)
            test_id = junit_case_id(suite_name, classname, name)
            failure = tc.find("failure")
            error = tc.find("error")
            skipped = tc.find("skipped")
            if failure is not None or error is not None:
                status = "fail"
                node = failure if failure is not None else error
                msg = (node.attrib.get("message") or "") + "\n" + _text(node)
            elif skipped is not None:
                status = "ignored"
                msg = _text(skipped)
            else:
                status = "pass"
                msg = ""
            cases.append(
                CaseResult(
                    test_id=test_id,
                    status=status,
                    time_s=time_s,
                    message=msg.strip(),
                )
            )
    return cases


def tag_case(case: CaseResult, areas: list[ta.Area]) -> CaseResult:
    a = ta.assign(case.test_id, areas)
    case.area_id = a.area_id
    case.area_label = a.area_label
    case.fallback_only = a.fallback_only
    case.is_golden = bool(_GOLDEN_RE.search(case.test_id))
    case.is_niezm = bool(_NIEZM_RE.search(case.test_id))
    if case.status == "fail":
        blob = f"{case.test_id}\n{case.message}"
        if _NET_RE.search(blob):
            case.fail_class = "non_hermetic"
        elif case.is_golden or case.is_niezm:
            case.fail_class = "economic_regression"
        elif _COMPILE_RE.search(blob):
            case.fail_class = "stale_test"
        else:
            case.fail_class = "stale_test"
    return case


def git_head() -> tuple[str, str]:
    def run(args: list[str]) -> str:
        try:
            return subprocess.check_output(
                args, cwd=REPO_ROOT, text=True, stderr=subprocess.DEVNULL
            ).strip()
        except (subprocess.CalledProcessError, FileNotFoundError):
            return ""

    ref = run(["git", "rev-parse", "--abbrev-ref", "HEAD"]) or "unknown"
    commit = run(["git", "rev-parse", "--short", "HEAD"]) or "unknown"
    return ref, commit


def build_report(
    cases: list[CaseResult],
    *,
    git_ref: str,
    commit: str,
    generated_at: str | None = None,
) -> Report:
    by_area: dict[str, AreaStats] = {}
    pass_n = fail_n = ignored_n = 0
    total_t = 0.0
    fallback_n = 0
    failures: list[dict] = []

    for c in cases:
        total_t += c.time_s
        if c.fallback_only:
            fallback_n += 1
        st = by_area.get(c.area_label)
        if st is None:
            st = AreaStats(label=c.area_label)
            by_area[c.area_label] = st
        st.tests += 1
        st.time_s += c.time_s
        if c.is_golden:
            st.golden += 1
        if c.is_niezm:
            st.niezm += 1
        if c.status == "pass":
            pass_n += 1
        elif c.status == "fail":
            fail_n += 1
            st.fails += 1
            failures.append(
                {
                    "area": c.area_label,
                    "id": c.test_id,
                    "class": c.fail_class,
                    "message": c.message[:500],
                }
            )
        else:
            ignored_n += 1
            st.ignored += 1

    areas_sorted = sorted(by_area.values(), key=lambda s: (-s.tests, s.label))
    return Report(
        title=f"Test report — {git_ref} @ {commit}",
        git_ref=git_ref,
        commit=commit,
        generated_at=generated_at
        or datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        pass_n=pass_n,
        fail_n=fail_n,
        ignored_n=ignored_n,
        time_s=total_t,
        areas=[asdict(a) for a in areas_sorted],
        failures=failures,
        fallback_n=fallback_n,
    )


def render_markdown(report: Report) -> str:
    headline = (
        f"{report.title} ({report.generated_at})   "
        f"PASS {report.pass_n} | FAIL {report.fail_n} | IGNORED {report.ignored_n} | "
        f"{report.time_s:.0f}s"
    )
    lines = [
        headline,
        "",
        f"{'Obszar':<24} {'Testy':>6} {'Pady':>6} {'Golden':>7} {'Niezm.':>7} {'Czas':>7}",
    ]
    for a in report.areas:
        lines.append(
            f"{a['label']:<24} {a['tests']:>6} {a['fails']:>6} {a['golden']:>7} "
            f"{a['niezm']:>7} {a['time_s']:>6.0f}s"
        )
    if report.fallback_n:
        lines.append("")
        lines.append(f"warning: {report.fallback_n} test(s) mapped only to Inne / catch-all")
    if report.failures:
        lines.append("")
        for f in report.failures:
            lines.append(f"FAIL  {f['area']}  {f['id']}")
            if f.get("message"):
                first = f["message"].splitlines()[0][:120]
                lines.append(f"      {first}")
            cls = f.get("class") or "unknown"
            hint = {
                "economic_regression": "economic_regression -> wymaga decyzji czlowieka (D4)",
                "non_hermetic": "non_hermetic -> izolacja EnvGuard/tempdir (bez retry)",
                "stale_test": "stale_test -> dopasuj test bez zmiany semantyki",
            }.get(cls, cls)
            lines.append(f"Klasa (sec 3): {hint}")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--junit", action="append", default=[], type=Path, help="JUnit XML (repeatable)")
    ap.add_argument("--map", type=Path, default=DEFAULT_MAP)
    ap.add_argument("--out-dir", type=Path, help="write report.md + report.json here")
    ap.add_argument("--git-ref", default="", help="override branch/ref label")
    ap.add_argument("--commit", default="", help="override short commit")
    ap.add_argument("--json-stdout", action="store_true")
    args = ap.parse_args(argv)

    if not args.junit:
        ap.error("pass at least one --junit")

    areas = ta.load_areas(args.map)
    cases: list[CaseResult] = []
    for p in args.junit:
        if not p.is_file():
            print(f"error: missing junit file {p}", file=sys.stderr)
            return 2
        cases.extend(tag_case(c, areas) for c in parse_junit(p))

    ref, commit = git_head()
    if args.git_ref:
        ref = args.git_ref
    if args.commit:
        commit = args.commit
    report = build_report(cases, git_ref=ref, commit=commit)
    md = render_markdown(report)
    payload = asdict(report)

    if args.out_dir:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        (args.out_dir / "report.md").write_text(md, encoding="utf-8")
        (args.out_dir / "report.json").write_text(
            json.dumps(payload, indent=2) + "\n", encoding="utf-8"
        )

    if args.json_stdout:
        json.dump(payload, sys.stdout, indent=2)
        sys.stdout.write("\n")
    else:
        sys.stdout.write(md)

    return 1 if report.fail_n else 0


if __name__ == "__main__":
    raise SystemExit(main())
