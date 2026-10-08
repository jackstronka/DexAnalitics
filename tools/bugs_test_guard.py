#!/usr/bin/env python3
"""C5: high/critical BUGS.md entries must name tests that exist in the repo.

Each ``high`` / ``critical`` entry's ``Guards/tests`` field must contain at least
one test identifier that exists as a Rust ``fn``, a vitest ``it(…)``, a Python
``test_*``, or a ``tests/*.rs`` / ``*.test.ts`` file stem — unless the field
explicitly uses ``manual:`` (no named automated test).

``cargo check`` / ``tsc`` / crate-wide ``cargo test --lib`` are not tests.

Usage:
  python tools/bugs_test_guard.py
  python tools/bugs_test_guard.py --bugs doc/BUGS.md --root .
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

ENTRY_SPLIT = re.compile(r"(?m)^### (BUG-\d{8}-\d{2}) — ")
SEVERITY_RE = re.compile(r"(?m)^severity:\s*(high|critical)\b")
GUARDS_RE = re.compile(
    r"- \*\*Guards/tests:\*\*\s*(.*?)(?=\n- \*\*|\n---\s*$|\n### |\Z)",
    re.S,
)
BACKTICK_RE = re.compile(r"`([^`]+)`")
IDENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
FN_RE = re.compile(
    r"(?m)^\s*(?:pub(?:\([^)]+\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"
)
IT_RE = re.compile(r"""(?:\b(?:it|test)\()\s*['`]([^'`]+)['`]""")
PY_TEST_RE = re.compile(r"(?m)^\s*def\s+(test_[A-Za-z0-9_]*)")
MANUAL_RE = re.compile(r"(?i)\bmanual:")
CARGO_TEST_RE = re.compile(r"cargo\s+test\b([^`\n]*)", re.I)
_CARGO_SKIP_FLAGS = {
    "--lib",
    "--bins",
    "--offline",
    "--nocapture",
    "--ignored",
    "--exact",
    "--all-targets",
    "--all-features",
    "--workspace",
    "--",
}
_CARGO_SKIP_WITH_VAL = {"-p", "--package", "--bin", "--features", "--color"}

SKIP_DIRS = {
    ".git",
    "target",
    "node_modules",
    "dist",
    "__pycache__",
    ".venv",
}
SKIP_FILES = {"api.gen.ts"}
# Not test names: toolchain / crate / cargo flags.
DENY_IDENTS = {
    "all",
    "all_features",
    "all_targets",
    "api",
    "build",
    "capture",
    "check",
    "clippy",
    "crate",
    "data",
    "default",
    "emit",
    "execution",
    "features",
    "format",
    "ignored",
    "integration",
    "lib",
    "migrate",
    "nocapture",
    "postgres",
    "rust",
    "test",
    "tests",
    "typescript",
    "verify",
    "vitest",
    "warnings",
    "web",
    "workspace",
}

MIN_IDENT_LEN = 8


@dataclass(frozen=True)
class BugEntry:
    bug_id: str
    severity: str
    guards: str


@dataclass(frozen=True)
class GuardNames:
    names: tuple[str, ...]
    manual: bool


@dataclass(frozen=True)
class Violation:
    bug_id: str
    reason: str
    missing: tuple[str, ...] = ()


def parse_high_critical_entries(text: str) -> list[BugEntry]:
    parts = ENTRY_SPLIT.split(text)
    out: list[BugEntry] = []
    for i in range(1, len(parts), 2):
        bug_id = parts[i]
        body = parts[i + 1]
        sev_m = SEVERITY_RE.search(body)
        if not sev_m:
            continue
        g_m = GUARDS_RE.search(body)
        guards = g_m.group(1).strip() if g_m else ""
        out.append(BugEntry(bug_id=bug_id, severity=sev_m.group(1), guards=guards))
    return out


def _last_ident(token: str) -> str | None:
    raw = token.strip().strip("`'\"")
    raw = raw.split()[0] if raw.split() else raw
    raw = raw.rstrip(".,;:)[]")
    if raw.endswith("_*"):
        raw = raw[:-2]
    if "::" in raw:
        raw = raw.rsplit("::", 1)[-1]
    if not IDENT_RE.match(raw):
        return None
    if raw.isupper():
        return None
    if "_" not in raw:
        return None
    if len(raw) < MIN_IDENT_LEN:
        return None
    if raw.lower() in DENY_IDENTS:
        return None
    if raw.startswith("clmm_lp_") or raw.startswith("clmm-lp-"):
        return None
    return raw


def _filters_from_cargo_args(args: str) -> list[str]:
    tokens = args.split()
    out: list[str] = []
    i = 0
    while i < len(tokens):
        tok = tokens[i]
        if tok == "--test" and i + 1 < len(tokens):
            out.append(tokens[i + 1])
            i += 2
            continue
        if tok in _CARGO_SKIP_WITH_VAL:
            i += 2
            continue
        if tok.startswith("-") or tok in _CARGO_SKIP_FLAGS:
            i += 1
            continue
        out.append(tok)
        i += 1
    return out


def extract_guard_names(guards: str) -> GuardNames:
    names: list[str] = []
    seen: set[str] = set()

    def add(raw: str | None) -> None:
        ident = _last_ident(raw or "")
        if ident and ident not in seen:
            seen.add(ident)
            names.append(ident)

    for chunk in BACKTICK_RE.findall(guards):
        add(chunk)
    for m in CARGO_TEST_RE.finditer(guards):
        for filt in _filters_from_cargo_args(m.group(1)):
            add(filt)
    return GuardNames(names=tuple(names), manual=bool(MANUAL_RE.search(guards)))


def _iter_source_files(root: Path) -> Iterable[Path]:
    scan_roots = [
        root / "crates",
        root / "web" / "src",
        root / "tools",
        root / "scripts",
    ]
    for base in scan_roots:
        if not base.is_dir():
            continue
        for path in base.rglob("*"):
            if not path.is_file():
                continue
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            if path.name in SKIP_FILES:
                continue
            if path.suffix in {".rs", ".ts", ".tsx", ".py"}:
                yield path


@dataclass
class TestIndex:
    fn_names: frozenset[str]
    file_stems: frozenset[str]

    def contains(self, ident: str) -> bool:
        if ident in self.fn_names or ident in self.file_stems:
            return True
        return any(ident in name for name in self.fn_names if len(ident) >= MIN_IDENT_LEN)


def build_test_index(root: Path) -> TestIndex:
    fn_names: set[str] = set()
    file_stems: set[str] = set()
    for path in _iter_source_files(root):
        rel = path.as_posix()
        if path.suffix == ".rs" and "/tests/" in f"/{rel}" and path.name != "mod.rs":
            file_stems.add(path.stem)
        if path.suffix in {".ts", ".tsx"} and path.name.endswith(".test.ts"):
            file_stems.add(path.name[: -len(".test.ts")])
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if path.suffix == ".rs":
            fn_names.update(FN_RE.findall(text))
            if "#[test]" in text or "#[tokio::test]" in text:
                file_stems.add(path.stem)
            for mod_name in re.findall(
                r"(?m)^\s*mod\s+([A-Za-z_][A-Za-z0-9_]*_tests)\b", text
            ):
                file_stems.add(mod_name)
                if mod_name.endswith("_tests"):
                    file_stems.add(mod_name[: -len("_tests")])
        elif path.suffix in {".ts", ".tsx"}:
            fn_names.update(IT_RE.findall(text))
        elif path.suffix == ".py" and (
            path.name.startswith("test_") or path.name.endswith("_test.py")
        ):
            fn_names.update(PY_TEST_RE.findall(text))
    return TestIndex(fn_names=frozenset(fn_names), file_stems=frozenset(file_stems))


def check_entries(entries: list[BugEntry], index: TestIndex) -> list[Violation]:
    violations: list[Violation] = []
    for entry in entries:
        parsed = extract_guard_names(entry.guards)
        if not parsed.names:
            if parsed.manual:
                continue
            violations.append(
                Violation(
                    bug_id=entry.bug_id,
                    reason="no test name in Guards/tests (add a real test fn / file stem, or explicit `manual:`)",
                )
            )
            continue
        missing = tuple(n for n in parsed.names if not index.contains(n))
        found = tuple(n for n in parsed.names if index.contains(n))
        if not found:
            violations.append(
                Violation(
                    bug_id=entry.bug_id,
                    reason="Guards/tests names not found in repo",
                    missing=missing,
                )
            )
    return violations


def render_report(entries: list[BugEntry], violations: list[Violation]) -> str:
    lines = [
        f"C5 bugs-test-guard: {len(entries)} high/critical entries, "
        f"{len(violations)} violation(s)"
    ]
    for v in violations:
        extra = f" missing={', '.join(v.missing)}" if v.missing else ""
        lines.append(f"  FAIL {v.bug_id}: {v.reason}{extra}")
    if not violations:
        lines.append("  ok")
    return "\n".join(lines) + "\n"


def run(bugs_path: Path, root: Path) -> int:
    text = bugs_path.read_text(encoding="utf-8")
    entries = parse_high_critical_entries(text)
    index = build_test_index(root)
    violations = check_entries(entries, index)
    sys.stdout.write(render_report(entries, violations))
    return 1 if violations else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bugs",
        type=Path,
        default=None,
        help="path to BUGS.md (default: <root>/doc/BUGS.md)",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="repo root to scan for tests (default: cwd)",
    )
    args = parser.parse_args(argv)
    root = (args.root or Path.cwd()).resolve()
    bugs = (args.bugs or root / "doc" / "BUGS.md").resolve()
    if not bugs.is_file():
        sys.stderr.write(f"bugs-test-guard: missing {bugs}\n")
        return 2
    return run(bugs, root)


if __name__ == "__main__":
    raise SystemExit(main())
