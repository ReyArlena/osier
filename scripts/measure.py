#!/usr/bin/env python3
"""Measure the numbers quoted on the website (site/index.html), from a release build.

Usage: scripts/measure.py [--runs N] [--repos N] [--no-build]

- Builds `target/release/osier` (the real optimised binary, not `cargo run`).
- Creates a throwaway demo setup under scratch/measure/ (gitignored): a HOME with N
  repositories (a local clone of this repository plus small generated ones) and its own
  config, state and cache dirs, so your own projects and settings are never touched.
- Launches the binary N times. Time to window: process start until the compositor reports a
  mapped window for that pid (Hyprland only, via `hyprctl clients -j`; skipped elsewhere).
  The first run is reported separately as the cold start; the medians use the warm runs.
- Memory, read from /proc four seconds after launch with all repositories open:
  RSS (everything resident, including shared libraries, GPU driver and font files),
  anonymous (the app's own heap: what most system monitors show), PSS (shared pages split
  between the processes using them) and USS (pages private to Osier).
- `git status` timing: the ignored osier-git perf test (10,000 files, 2,000 changed),
  release profile, median of five runs.
"""

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "scratch" / "measure"
BIN = ROOT / "target" / "release" / "osier"


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def git(repo, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="Demo", GIT_AUTHOR_EMAIL="demo@example.com",
               GIT_COMMITTER_NAME="Demo", GIT_COMMITTER_EMAIL="demo@example.com")
    run(["git", "-C", str(repo), *args], env=env, stdout=subprocess.DEVNULL)


def setup(repos):
    if WORK.exists():
        shutil.rmtree(WORK)
    projects = WORK / "home" / "Projects"
    projects.mkdir(parents=True)
    for d in ("config", "state", "cache"):
        (WORK / d).mkdir()
    paths = []
    clone = projects / "osier"
    run(["git", "clone", "-q", str(ROOT), str(clone)])
    paths.append(clone)
    for i in range(1, repos):
        repo = projects / f"demo-{i}"
        repo.mkdir()
        git(repo, "init", "-q", "-b", "main")
        for n in range(30):
            (repo / f"file-{n % 7}.txt").write_text(f"line {n}\n" * (n + 1))
            git(repo, "add", "-A")
            git(repo, "commit", "-q", "-m", f"Commit {n}")
        paths.append(repo)
    (WORK / "config" / "config.toml").write_text(
        '[appearance]\nmode = "dark"\ndark_theme = "Catppuccin Mocha"\ndux_shell_sync = false\n'
    )
    quoted = ", ".join(f'"{p}"' for p in paths)
    (WORK / "state" / "state.toml").write_text(
        f'open_projects = [{quoted}]\nactive_project = "{paths[0]}"\n'
        "[layout]\nsidebar_visible = true\n"
    )
    return paths


def memory(pid):
    status = Path(f"/proc/{pid}/status").read_text()
    kb = {l.split(":")[0]: int(l.split()[1]) for l in status.splitlines() if l.startswith(("VmRSS", "RssAnon"))}
    rollup = Path(f"/proc/{pid}/smaps_rollup").read_text()
    roll = {l.split(":")[0]: int(l.split()[1]) for l in rollup.splitlines()[1:] if l.split()[1].isdigit()}
    return {
        "rss": kb["VmRSS"] / 1024,
        "anon": kb["RssAnon"] / 1024,
        "pss": roll.get("Pss", 0) / 1024,
        "uss": (roll.get("Private_Clean", 0) + roll.get("Private_Dirty", 0)) / 1024,
    }


def mapped(pid):
    try:
        out = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True).stdout
        return any(c.get("pid") == pid and c.get("mapped") for c in json.loads(out))
    except (OSError, ValueError):
        return False


def launch_runs(runs, hyprland):
    env = dict(os.environ, HOME=str(WORK / "home"), OSIER_CONFIG_DIR=str(WORK / "config"),
               OSIER_STATE_DIR=str(WORK / "state"), OSIER_CACHE_DIR=str(WORK / "cache"),
               OSIER_LOG="error", GIT_CONFIG_NOSYSTEM="1")
    results = []
    for i in range(runs):
        t0 = time.perf_counter()
        p = subprocess.Popen([str(BIN)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        ms = None
        if hyprland:
            while time.perf_counter() - t0 < 10:
                if mapped(p.pid):
                    ms = (time.perf_counter() - t0) * 1000
                    break
        time.sleep(4)
        mem = memory(p.pid)
        p.terminate()
        p.wait()
        results.append((ms, mem))
        print(f"  run {i + 1}: window {f'{ms:.0f} ms' if ms else 'n/a'}, "
              + ", ".join(f"{k} {v:.0f} MB" for k, v in mem.items()), flush=True)
        time.sleep(0.5)
    return results


def status_timing(runs=5):
    """Median of several runs: a single run swings between ~20 and ~70 ms."""
    times = []
    for _ in range(runs):
        out = subprocess.run(
            ["cargo", "test", "--release", "-p", "osier-git", "status_of_ten_thousand", "--",
             "--ignored", "--nocapture"], cwd=ROOT, capture_output=True, text=True)
        for line in (out.stdout + out.stderr).splitlines():
            if line.startswith("status of 10k files:"):
                value = line.split(":", 1)[1].strip()
                ms = float(value[:-2]) if value.endswith("ms") else float(value[:-1]) * 1000
                times.append(ms)
    if not times:
        return "not found (did the test run?)"
    return f"{statistics.median(times):.0f} ms (runs: {', '.join(f'{t:.0f}' for t in times)})"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=7)
    ap.add_argument("--repos", type=int, default=3)
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        run(["cargo", "build", "--release", "-p", "osier"], cwd=ROOT)
    hyprland = shutil.which("hyprctl") is not None and "HYPRLAND_INSTANCE_SIGNATURE" in os.environ
    cpu = next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), "?")
    print(f"CPU: {cpu}\nrepos open: {args.repos}, runs: {args.runs}, time to window: "
          f"{'hyprctl' if hyprland else 'skipped (needs Hyprland)'}")
    setup(args.repos)
    results = launch_runs(args.runs, hyprland)
    warm = results[1:] or results
    med = lambda xs: statistics.median(xs) if xs else None
    times = [r[0] for r in warm if r[0] is not None]
    print("\nSummary (medians of the warm runs)")
    if results[0][0] is not None:
        print(f"  time to window: {med(times):.0f} ms (cold first run {results[0][0]:.0f} ms)")
    for k in ("rss", "anon", "pss", "uss"):
        print(f"  {k}: {med([r[1][k] for r in warm]):.0f} MB")
    print(f"  git status, 10k files / 2k changed: {status_timing()}")


if __name__ == "__main__":
    sys.exit(main())
