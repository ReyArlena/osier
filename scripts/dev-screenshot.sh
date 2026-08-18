#!/usr/bin/env bash
# Build Osier, launch it, capture a screenshot, stop it.
# Usage: scripts/dev-screenshot.sh [seconds-to-wait] [extra args for osier...]
# Output: scratch/shot.png and scratch/run.log (scratch/ is gitignored).
set -euo pipefail

cd "$(dirname "$0")/.."
wait_seconds="${1:-4}"
shift || true

mkdir -p scratch
cargo build -p osier

if ! command -v grim >/dev/null 2>&1; then
    echo "grim not found: install it (wlroots/Hyprland screenshot tool) or capture manually." >&2
    exit 1
fi

./target/debug/osier "$@" > scratch/run.log 2>&1 &
pid=$!
sleep "$wait_seconds"
grim scratch/shot.png
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true

echo "screenshot: $PWD/scratch/shot.png"
if [ -s scratch/run.log ]; then
    echo "log (non-empty, check it): $PWD/scratch/run.log"
    tail -n 20 scratch/run.log
else
    echo "log empty: no panics or warnings"
fi
