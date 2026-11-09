#!/usr/bin/env python3
"""E1: fast local verify for crates / web touched by a git diff.

Selects Cargo packages from changed paths (plus reverse dependents), then runs
fmt-check / clippy / nextest only for those packages. Web (api.gen + tsc +
vitest + eslint) runs only when ``web/`` or ``openapi.json`` changed.

Usage:
  python tools/verify_fast.py --base origin/main
  python tools/verify_fast.py --base origin/main --dry-run
  make verify-fast
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# path prefix under crates/ → package name
CRATE_DIR_TO_PKG = {
    "domain": "clmm-lp-domain",
    "protocols": "clmm-lp-protocols",
    "data": "clmm-lp-data",
    "simulation": "clmm-lp-simulation",
    "optimization": "clmm-lp-optimization",
    "execution": "clmm-lp-execution",
    "api": "clmm-lp-api",
    "cli": "clmm-lp-cli",
}

# package → direct workspace deps (edges for reverse closure)
PKG_DEPS: dict[str, tuple[str, ...]] = {
    "clmm-lp-domain": (),
    "clmm-lp-protocols": ("clmm-lp-domain",),
    "clmm-lp-data": ("clmm-lp-domain",),
    "clmm-lp-simulation": ("clmm-lp-domain",),
    "clmm-lp-optimization": ("clmm-lp-domain", "clmm-lp-simulation"),
    "clmm-lp-execution": (
        "clmm-lp-domain",
        "clmm-lp-protocols",
        "clmm-lp-data",
        "clmm-lp-optimization",
    ),
    "clmm-lp-api": (
        "clmm-lp-domain",
        "clmm-lp-execution",
        "clmm-lp-protocols",
        "clmm-lp-data",
        "clmm-lp-simulation",
        "clmm-lp-optimization",
    ),
    "clmm-lp-cli": (
        "clmm-lp-domain",
        "clmm-lp-data",
        "clmm-lp-simulation",
        "clmm-lp-optimization",
        "clmm-lp-protocols",
        "clmm-lp-execution",
    ),
}

# Paths that force a full Rust workspace verify.
FULL_RUST_PREFIXES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rustfmt.toml",
    "clippy.toml",
    ".config/nextest.toml",
    ".github/workflows/",
    "Makefile",
)


def reverse_dependents() -> dict[str, set[str]]:
    rev: dict[str, set[str]] = {p: set() for p in PKG_DEPS}
    for pkg, deps in PKG_DEPS.items():
        for d in deps:
            rev.setdefault(d, set()).add(pkg)
    # transitive closure
    changed = True
    while changed:
        changed = False
        for pkg in list(rev):
            extra: set[str] = set()
            for child in rev[pkg]:
                extra |= rev.get(child, set())
            if not extra <= rev[pkg]:
                rev[pkg] |= extra
                changed = True
    return rev


REV = reverse_dependents()


def git_diff_names(base: str) -> list[str]:
    r = subprocess.run(
        ["git", "diff", "--name-only", f"{base}...HEAD"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if r.returncode != 0:
        # unborn / no remote: fall back to staged+unstaged vs HEAD
        r = subprocess.run(
            ["git", "diff", "--name-only", "HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        r2 = subprocess.run(
            ["git", "diff", "--name-only", "--cached"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        files = set()
        for out in (r.stdout, r2.stdout):
            files.update(line.strip().replace("\\", "/") for line in out.splitlines() if line.strip())
        return sorted(files)
    return [line.strip().replace("\\", "/") for line in r.stdout.splitlines() if line.strip()]


def package_for_path(path: str) -> str | None:
    if not path.startswith("crates/"):
        return None
    parts = path.split("/")
    if len(parts) < 2:
        return None
    return CRATE_DIR_TO_PKG.get(parts[1])


def forces_full_rust(paths: list[str]) -> bool:
    for p in paths:
        for pref in FULL_RUST_PREFIXES:
            if p == pref or p.startswith(pref):
                return True
    return False


def needs_web(paths: list[str]) -> bool:
    for p in paths:
        if p.startswith("web/") or p == "crates/api/openapi.json":
            return True
    return False


def select_packages(paths: list[str]) -> tuple[list[str], bool, bool]:
    """Return (packages_sorted, full_rust, run_web)."""
    run_web = needs_web(paths)
    if not paths:
        return ([], False, False)
    if forces_full_rust(paths):
        return (sorted(PKG_DEPS.keys()), True, run_web)

    selected: set[str] = set()
    for p in paths:
        pkg = package_for_path(p)
        if pkg:
            selected.add(pkg)
            selected |= REV.get(pkg, set())

    # Non-crate rust-ish paths under crates/ already handled; other paths ignore for rust.
    return (sorted(selected), False, run_web)


def run(cmd: list[str]) -> int:
    print("+", " ".join(cmd), flush=True)
    return subprocess.call(cmd, cwd=REPO_ROOT)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="E1 verify-fast")
    ap.add_argument("--base", default="origin/main", help="git diff base (default origin/main)")
    ap.add_argument("--dry-run", action="store_true", help="print plan only")
    ap.add_argument("--skip-fmt", action="store_true")
    ap.add_argument("--skip-clippy", action="store_true")
    args = ap.parse_args(argv)

    paths = git_diff_names(args.base)
    pkgs, full_rust, run_web = select_packages(paths)

    print(f"verify-fast base={args.base}")
    print(f"changed files: {len(paths)}")
    if full_rust:
        print("rust scope: FULL workspace (root/CI/toolchain touch)")
    elif pkgs:
        print("rust packages:", ", ".join(pkgs))
    else:
        print("rust packages: (none)")
    print("web:", "yes" if run_web else "no")

    if args.dry_run:
        return 0

    if not pkgs and not run_web:
        print("verify-fast: nothing to check")
        return 0

    if pkgs:
        pkg_args: list[str] = []
        for p in pkgs:
            pkg_args.extend(["-p", p])

        if not args.skip_fmt:
            # fmt --check does not take -p reliably across versions; check changed crates via path.
            code = run(["cargo", "fmt", "--all", "--check"])
            if code != 0:
                return code

        if not args.skip_clippy:
            code = run(
                ["cargo", "clippy", *pkg_args, "--all-targets", "--all-features", "--", "-D", "warnings"]
            )
            if code != 0:
                return code

        code = run(["cargo", "nextest", "run", *pkg_args])
        if code != 0:
            return code

    if run_web:
        web = REPO_ROOT / "web"
        if not (web / "node_modules").is_dir():
            print("verify-fast: web/node_modules missing — run cd web && npm install", file=sys.stderr)
            return 1
        for cmd in (
            ["npm", "run", "check:api-gen"],
            ["npx", "tsc", "--noEmit"],
            [
                "npx",
                "vitest",
                "run",
                "--reporter=default",
                "--reporter=junit",
                "--outputFile=../target/test-report/vitest-junit.xml",
            ],
            ["npm", "run", "lint"],
        ):
            print("+ (web)", " ".join(cmd), flush=True)
            code = subprocess.call(cmd, cwd=web, shell=(sys.platform == "win32"))
            if code != 0:
                return code

    print("verify-fast: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
