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
# Gentle on the desktop it runs beside (twice it froze: 28 rustc at once, the tests and three
# screenshot sessions rendering in software together, on top of launchers a run had left behind):
# a third of the cores for rustc and the tests, the screenshot sessions one after another, all at
# low priority. The azrak-dev container is capped too (12 GB, no swap, 16 cores):
#   podman update --memory 12g --memory-swap 12g --cpus 16 azrak-dev
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
renice -n 10 $$ >/dev/null 2>&1 || true
# Screens open on a virtual display, never on the desktop this runs from.
unset WAYLAND_DISPLAY XDG_SESSION_TYPE

start=$(date +%s)
# The launcher and its tests in one go: cargo compiles the two side by side.
"$cargo" build --profile quick --locked --bins --tests
echo "built in $(( $(date +%s) - start ))s"

log=$(mktemp -d)
status=0
# Every text translated, and no translation with a mistyped {…}.
scripts/i18n.sh check || status=1
"$cargo" test --profile quick --locked -- --test-threads=8 >"$log/tests" 2>&1 || status=1
grep -E "^test result|FAILED|panicked" "$log/tests" || true
# The screens in English, in Portuguese (dist/ui-matrix/pt_BR/: its texts run longer) and the
# first-run setup's every page (dist/ui-matrix/onboard/), one session at a time.
for run in "" "--lang pt_BR" "--onboard"; do
    # shellcheck disable=SC2086
    SWARMPLAY_BIN=target/quick/swarmplay python3 scripts/ui_matrix.py $run "${@:-1920x1080}" >"$log/shots" 2>&1 \
        || { status=1; tail -20 "$log/shots"; }
done
echo "screenshots: dist/ui-matrix/ · done in $(( $(date +%s) - start ))s"
exit $status
