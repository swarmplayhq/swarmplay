#!/usr/bin/env bash
# One round of checks while working on the launcher: build, then the tests and the screenshots
# side by side. Run it in the azrak-dev distrobox; see .claude/skills/azrak-fork/SKILL.md.
#
#   scripts/check.sh                  # 1920x1080 screenshots
#   scripts/check.sh 1280x720 2560x1440
#
# It uses the `quick` profile (Cargo.toml): our code unoptimised and incremental, the libraries
# fully optimised and built once (shared with the tests), so a UI edit rebuilds in ~25 s. Linked
# with mold when it's there. Releases still build with --release.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo="${CARGO:-$HOME/.cargo/bin/cargo}"
if [ -x /usr/libexec/mold/ld ]; then
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-B/usr/libexec/mold"
fi
# Screens open on a virtual display, never on the desktop this runs from.
unset WAYLAND_DISPLAY XDG_SESSION_TYPE

start=$(date +%s)
# The launcher and its tests in one go: cargo compiles the two side by side.
"$cargo" build --profile quick --locked --bins --tests
echo "built in $(( $(date +%s) - start ))s"

log=$(mktemp -d)
"$cargo" test --profile quick --locked >"$log/tests" 2>&1 & tests=$!
SWARMPLAY_BIN=target/quick/swarmplay python3 scripts/ui_matrix.py "${@:-1920x1080}" >"$log/shots" 2>&1 & shots=$!
# The same screens in Portuguese (dist/ui-matrix/pt_BR/): its texts run longer.
SWARMPLAY_BIN=target/quick/swarmplay python3 scripts/ui_matrix.py --lang pt_BR "${@:-1920x1080}" >"$log/shots-pt" 2>&1 & shots_pt=$!

status=0
# Every text translated, and no translation with a mistyped {…}.
scripts/i18n.sh check || status=1
wait $tests || status=1
grep -E "^test result|FAILED|panicked" "$log/tests" || true
wait $shots || { status=1; tail -20 "$log/shots"; }
wait $shots_pt || { status=1; tail -20 "$log/shots-pt"; }
echo "screenshots: dist/ui-matrix/ · done in $(( $(date +%s) - start ))s"
exit $status
