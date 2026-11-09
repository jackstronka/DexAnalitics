#!/usr/bin/env python3
"""R4: weekly trend over R2 ``report.json`` snapshots.

Hermetic mode (tests / offline):
  python tools/test_trend.py --reports-dir tools/fixtures/r4_reports --out-dir target/test-trend

GitHub mode (Actions weekly job):
  python tools/test_trend.py --fetch-github --branch main --days 14 --out-dir target/test-trend

Summary: tests/fails/golden/niezm per area, CI/test duration, flake candidates
(test id that both passed and failed across snapshots in the window).
"""

from __future__ import annotations

import argparse
import io
import json
import os
import sys
import zipfile
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

REPO_ROOT = Path(__file__).resolve().parents[1]


@dataclass
class AreaTrend:
    label: str
    runs: int = 0
    tests_last: int = 0
    tests_avg: float = 0.0
    fails_total: int = 0
    fails_last: int = 0
    golden_last: int = 0
    niezm_last: int = 0
    time_s_avg: float = 0.0


@dataclass
class TrendReport:
    title: str
    branch: str
    window_days: int
    generated_at: str
    snapshots: int
    areas: list[dict] = field(default_factory=list)
    flake_candidates: list[dict] = field(default_factory=list)
    pass_avg: float = 0.0
    fail_avg: float = 0.0
    time_s_avg: float = 0.0
    ci_seconds_avg: float | None = None
    notes: list[str] = field(default_factory=list)


def _parse_ts(raw: str | None) -> datetime | None:
    if not raw:
        return None
    try:
        # report.json: "2026-11-09 13:22 UTC"
        if raw.endswith(" UTC") and "T" not in raw:
            return datetime.strptime(raw, "%Y-%m-%d %H:%M UTC").replace(tzinfo=timezone.utc)
        return datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except ValueError:
        return None


def load_report(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def collect_local_reports(reports_dir: Path) -> list[dict[str, Any]]:
    files = sorted(reports_dir.glob("**/report.json")) + sorted(reports_dir.glob("*.json"))
    # de-dupe paths
    seen: set[Path] = set()
    out: list[dict[str, Any]] = []
    for f in files:
        rp = f.resolve()
        if rp in seen:
            continue
        seen.add(rp)
        try:
            data = load_report(f)
        except (OSError, json.JSONDecodeError):
            continue
        if "areas" not in data:
            continue
        data["_source"] = str(f)
        out.append(data)
    return out


def _gh_request(url: str, token: str) -> Any:
    req = Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "clmm-test-trend",
        },
    )
    with urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode("utf-8"))


def _gh_download(url: str, token: str) -> bytes:
    req = Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "clmm-test-trend",
        },
    )
    with urlopen(req, timeout=120) as resp:
        return resp.read()


def fetch_github_reports(
    *,
    owner: str,
    repo: str,
    branch: str,
    days: int,
    token: str,
    artifact_name: str = "test-report",
) -> tuple[list[dict[str, Any]], list[float]]:
    """Return (report.json payloads, ci_run_seconds list for successful CI on branch)."""
    since = datetime.now(timezone.utc) - timedelta(days=days)
    reports: list[dict[str, Any]] = []
    page = 1
    while page <= 5:
        q = urlencode({"name": artifact_name, "per_page": 100, "page": page})
        url = f"https://api.github.com/repos/{owner}/{repo}/actions/artifacts?{q}"
        try:
            payload = _gh_request(url, token)
        except (HTTPError, URLError, TimeoutError) as e:
            raise SystemExit(f"GitHub artifacts list failed: {e}") from e
        arts = payload.get("artifacts") or []
        if not arts:
            break
        for art in arts:
            if art.get("expired"):
                continue
            created = _parse_ts(art.get("created_at"))
            if created and created < since:
                continue
            # Prefer artifacts from main CI; workflow_run may be absent on list endpoint
            try:
                raw = _gh_download(art["archive_download_url"], token)
            except (HTTPError, URLError, TimeoutError):
                continue
            try:
                with zipfile.ZipFile(io.BytesIO(raw)) as zf:
                    names = [n for n in zf.namelist() if n.endswith("report.json")]
                    if not names:
                        continue
                    data = json.loads(zf.read(names[0]).decode("utf-8"))
            except (zipfile.BadZipFile, json.JSONDecodeError, KeyError):
                continue
            if "areas" not in data:
                continue
            data["_source"] = f"artifact:{art.get('id')}"
            data["_created_at"] = art.get("created_at")
            reports.append(data)
        if len(arts) < 100:
            break
        page += 1

    # CI wall times from workflow runs (best-effort)
    ci_secs: list[float] = []
    q = urlencode(
        {
            "branch": branch,
            "status": "completed",
            "per_page": 30,
        }
    )
    try:
        runs = _gh_request(
            f"https://api.github.com/repos/{owner}/{repo}/actions/runs?{q}",
            token,
        )
        for run in runs.get("workflow_runs") or []:
            if run.get("head_branch") != branch:
                continue
            path = str(run.get("path") or "")
            name = str(run.get("name") or "")
            if not (path.endswith("ci.yml") or name == "CI"):
                continue
            created = _parse_ts(run.get("created_at"))
            updated = _parse_ts(run.get("updated_at"))
            if created and created < since:
                continue
            if created and updated:
                ci_secs.append(max(0.0, (updated - created).total_seconds()))
    except (HTTPError, URLError, TimeoutError):
        pass

    return reports, ci_secs


def build_trend(
    reports: list[dict[str, Any]],
    *,
    branch: str,
    window_days: int,
    ci_seconds: list[float] | None = None,
) -> TrendReport:
    # Sort oldest -> newest by generated_at / _created_at
    def sort_key(r: dict[str, Any]) -> str:
        return str(r.get("_created_at") or r.get("generated_at") or "")

    reports = sorted(reports, key=sort_key)
    notes: list[str] = []
    if not reports:
        notes.append("No report.json snapshots in window.")
        return TrendReport(
            title=f"Test trend — {branch}",
            branch=branch,
            window_days=window_days,
            generated_at=datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
            snapshots=0,
            notes=notes,
        )

    area_series: dict[str, list[dict[str, Any]]] = defaultdict(list)
    pass_ns: list[int] = []
    fail_ns: list[int] = []
    time_ns: list[float] = []
    # test_id -> set of statuses seen
    id_status: dict[str, set[str]] = defaultdict(set)

    for rep in reports:
        pass_ns.append(int(rep.get("pass_n") or 0))
        fail_ns.append(int(rep.get("fail_n") or 0))
        time_ns.append(float(rep.get("time_s") or 0))
        for a in rep.get("areas") or []:
            label = str(a.get("label") or "Inne")
            area_series[label].append(a)
        for f in rep.get("failures") or []:
            tid = str(f.get("id") or "")
            if tid:
                id_status[tid].add("fail")
        # Treat non-failed ids as pass only if we have full case list — R2 JSON
        # does not list passes. Flake heuristic: id failed in some snapshot and
        # later snapshots have fail_n==0 with same area test count (weak), OR
        # id appears in failures of a subset of snapshots only.
        # Record failure presence per snapshot index instead:
        # (handled below after loop using failure sets per report)

    fail_sets: list[set[str]] = []
    for rep in reports:
        s: set[str] = set()
        for f in rep.get("failures") or []:
            tid = str(f.get("id") or "")
            if tid:
                s.add(tid)
                id_status[tid].add("fail")
        fail_sets.append(s)

    # Any id that failed in at least one snapshot and is absent from failures
    # in another snapshot (while that other snapshot exists) is a flake candidate.
    flakes: list[dict[str, Any]] = []
    all_failed_ids = set().union(*fail_sets) if fail_sets else set()
    for tid in sorted(all_failed_ids):
        appeared = sum(1 for s in fail_sets if tid in s)
        absent = sum(1 for s in fail_sets if tid not in s)
        if appeared and absent:
            flakes.append(
                {
                    "id": tid,
                    "fail_snapshots": appeared,
                    "pass_or_absent_snapshots": absent,
                    "note": "failed in some main reports, absent in others (candidate flake)",
                }
            )

    areas_out: list[AreaTrend] = []
    last = reports[-1]
    last_areas = {str(a.get("label")): a for a in (last.get("areas") or [])}
    for label, series in sorted(area_series.items(), key=lambda x: (-len(x[1]), x[0])):
        tests_vals = [int(a.get("tests") or 0) for a in series]
        fails_vals = [int(a.get("fails") or 0) for a in series]
        time_vals = [float(a.get("time_s") or 0) for a in series]
        last_a = last_areas.get(label) or series[-1]
        areas_out.append(
            AreaTrend(
                label=label,
                runs=len(series),
                tests_last=int(last_a.get("tests") or 0),
                tests_avg=(sum(tests_vals) / len(tests_vals)) if tests_vals else 0.0,
                fails_total=sum(fails_vals),
                fails_last=int(last_a.get("fails") or 0),
                golden_last=int(last_a.get("golden") or 0),
                niezm_last=int(last_a.get("niezm") or 0),
                time_s_avg=(sum(time_vals) / len(time_vals)) if time_vals else 0.0,
            )
        )

    areas_out.sort(key=lambda a: (-a.tests_last, a.label))
    ci_avg = None
    if ci_seconds:
        ci_avg = sum(ci_seconds) / len(ci_seconds)

    return TrendReport(
        title=f"Test trend — {branch} ({window_days}d, {len(reports)} snapshots)",
        branch=branch,
        window_days=window_days,
        generated_at=datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        snapshots=len(reports),
        areas=[asdict(a) for a in areas_out],
        flake_candidates=flakes[:50],
        pass_avg=(sum(pass_ns) / len(pass_ns)) if pass_ns else 0.0,
        fail_avg=(sum(fail_ns) / len(fail_ns)) if fail_ns else 0.0,
        time_s_avg=(sum(time_ns) / len(time_ns)) if time_ns else 0.0,
        ci_seconds_avg=ci_avg,
        notes=notes,
    )


def render_markdown(trend: TrendReport) -> str:
    lines = [
        f"{trend.title}  ({trend.generated_at})",
        f"PASS avg {trend.pass_avg:.0f} | FAIL avg {trend.fail_avg:.1f} | "
        f"test time avg {trend.time_s_avg:.0f}s"
        + (
            f" | CI wall avg {trend.ci_seconds_avg:.0f}s"
            if trend.ci_seconds_avg is not None
            else ""
        ),
        "",
        f"{'Obszar':<24} {'Runs':>5} {'Testy':>6} {'PadyS':>6} {'Pady*':>6} "
        f"{'Golden':>7} {'Niezm.':>7} {'t_avg':>7}",
    ]
    for a in trend.areas:
        lines.append(
            f"{a['label']:<24} {a['runs']:>5} {a['tests_last']:>6} {a['fails_total']:>6} "
            f"{a['fails_last']:>6} {a['golden_last']:>7} {a['niezm_last']:>7} "
            f"{a['time_s_avg']:>6.0f}s"
        )
    lines.append("")
    lines.append("Pady* = fails in newest snapshot; PadyS = sum of fails across snapshots.")
    if trend.flake_candidates:
        lines.append("")
        lines.append(f"Flake candidates ({len(trend.flake_candidates)}):")
        for f in trend.flake_candidates[:20]:
            lines.append(
                f"  - {f['id']}  (fail {f['fail_snapshots']} / absent {f['pass_or_absent_snapshots']})"
            )
    for n in trend.notes:
        lines.append("")
        lines.append(f"note: {n}")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--reports-dir", type=Path, help="directory of report.json files")
    ap.add_argument("--fetch-github", action="store_true")
    ap.add_argument("--branch", default="main")
    ap.add_argument("--days", type=int, default=14)
    ap.add_argument("--owner", default="")
    ap.add_argument("--repo", default="")
    ap.add_argument("--out-dir", type=Path, default=Path("target/test-trend"))
    ap.add_argument("--json-stdout", action="store_true")
    args = ap.parse_args(argv)

    reports: list[dict[str, Any]] = []
    ci_secs: list[float] = []

    if args.reports_dir:
        reports.extend(collect_local_reports(args.reports_dir))

    if args.fetch_github:
        token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
        if not token:
            print("error: GITHUB_TOKEN/GH_TOKEN required for --fetch-github", file=sys.stderr)
            return 2
        repo_full = os.environ.get("GITHUB_REPOSITORY", "")
        owner = args.owner
        repo = args.repo
        if not owner or not repo:
            if "/" in repo_full:
                owner, repo = repo_full.split("/", 1)
            else:
                print("error: pass --owner/--repo or set GITHUB_REPOSITORY", file=sys.stderr)
                return 2
        fetched, ci_secs = fetch_github_reports(
            owner=owner, repo=repo, branch=args.branch, days=args.days, token=token
        )
        reports.extend(fetched)

    if not args.reports_dir and not args.fetch_github:
        ap.error("pass --reports-dir and/or --fetch-github")

    trend = build_trend(
        reports, branch=args.branch, window_days=args.days, ci_seconds=ci_secs or None
    )
    md = render_markdown(trend)
    payload = asdict(trend)

    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "trend.md").write_text(md, encoding="utf-8")
    (args.out_dir / "trend.json").write_text(
        json.dumps(payload, indent=2) + "\n", encoding="utf-8"
    )

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write("## Weekly test trend (R4)\n\n")
            fh.write(md)

    if args.json_stdout:
        json.dump(payload, sys.stdout, indent=2)
        sys.stdout.write("\n")
    else:
        sys.stdout.write(md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
