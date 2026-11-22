#!/usr/bin/env python3
"""Screenshot every screen of the launcher at several screen sizes, for visual review.

Runs the real launcher fullscreen under a real window manager (xfwm4) on hidden X displays,
with a throwaway home folder seeded from your cache (no network wait, nothing personal).

    scripts/ui_matrix.py                      # all sizes
    scripts/ui_matrix.py 1280x720 3440x1440   # just these
    scripts/ui_matrix.py --lang pt_BR 1920x1080   # in another language (src/i18n.rs)

Writes dist/ui-matrix/<size>/<screen>.png and a contact sheet per size; another language's go to
dist/ui-matrix/<lang>/.
Needs: Xvfb, xfwm4, xdotool, ImageMagick.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from ui_fixture import NO_UPDATES, install_game, seed_cache, seed_downloads

ROOT = Path(__file__).resolve().parent.parent
SIZES = ["1280x720", "1366x768", "1920x1080", "1920x1200", "2560x1440", "3440x1440", "3840x2160", "1280x800"]

# (screen name, keys to press before the shot)
STEPS = [
    ("00-boot", []),                                       # the start screen (held: SWARMPLAY_BOOT_HOLD_MS)
    ("01-home", ["sleep:9"]),
    ("01b-welcome", ["Left", "Down", "sleep:0.8"]),         # the Welcome tile's widgets
    ("01c-top-buttons", ["Up", "Up", "Right", "Right", "sleep:0.5"]),  # Search, round, focused
    ("01d-avatar", ["Right", "Right", "sleep:0.5"]),        # past Settings: you (the avatar)
    ("02-home-new-game", ["Down", "Right", "Right", "Right"]),
    ("02b-home-peek", ["Down", "Down", "sleep:0.8"]),  # Home scrolled to the Hub's cards
    ("03-hub", ["Return", "sleep:1.5"]),   # the Compatibility card opens the Hub (never Play)
    ("03b-hub-up-to-home", ["Up", "sleep:1"]),              # Up from the Hub's top: back to the cards
    ("04-hub-scrolled", ["Down", "sleep:1.2", "Down", "Down", "sleep:0.8"]),
    ("04a-gallery", ["Return", "sleep:1.2", "Right", "sleep:1"]),                 # a screenshot, full screen
    ("04a2-back-in-hub", ["Escape", "sleep:0.6"]),
    ("04b-hub-details", ["Down", "Return", "sleep:0.8"]),
    ("05-library", ["Escape", "Escape", "Up", "Up", "Right", "Return", "sleep:1.5"]),
    ("06-library-down", ["Down", "Down", "Down", "sleep:1"]),
    ("07-search", ["slash", "r", "e", "s", "i", "sleep:1"]),
    ("08-menu", ["Escape", "Escape", "sleep:0.5", "Escape", "sleep:0.8", "o", "sleep:0.6"]),
    ("08b-menu-beside", ["Escape", "Down", "Right", "Return", "sleep:0.6"]),           # from the ··· button: beside it
    ("09-settings", ["Escape", "Escape", "Escape", "sleep:0.5", "s", "sleep:0.8"]),   # the first page: sections
    ("09b-settings-games", ["Return", "sleep:0.6"]),                                 # a section's page
    ("09b2-notification", ["Down", "Down", "Down", "Return", "sleep:0.4", "Return", "sleep:0.9"]),  # Rescan twice: the second takes the first one's place
    ("09c-settings-theme", ["Escape"] + ["Down"] * 7 + ["Return", "Right", "sleep:0.8"]),  # Wallpaper: Deep blue
    ("09d-settings-themed", ["Escape", "sleep:0.6"]),                                # back on the first page, themed
    ("09e-settings-sound", ["Up", "Up", "Up", "Up", "Return", "sleep:0.8"]),               # Sound: volume, mute, output
    ("09f-settings-storage", ["Escape", "sleep:0.5", "Up", "Up", "Return", "sleep:0.8"]),  # Storage: the places and their room
    ("09f2-storage-drives", ["Down"] * 5 + ["Return", "sleep:0.9"]),                   # Add a place: the drives
    ("09f3-storage-folders", ["Return", "sleep:0.8", "Down", "sleep:0.4"]),            # Home folder: its folders
    ("09f4-storage-back", ["Escape", "sleep:0.4", "Escape", "sleep:0.8"]),              # Up, closed: Storage again
    ("09g-settings-notifications", ["Escape", "sleep:0.5", "Down", "Down", "Down", "Return", "sleep:0.8"]),
    ("10-settings-bottom", ["Escape", "sleep:0.6"] + ["Down"] * 12 + ["sleep:0.8"]), # Quit, at the end
    # The language switched live (Launcher → Language, its first row), then back.
    ("10b-language-switched", ["Up"] * 12 + ["Down"] * 8 + ["Return", "sleep:0.6", "Right", "sleep:1"]),
    ("10c-language-back", ["Left", "sleep:0.8", "Escape", "sleep:0.6"]),
    ("11-toasts", ["Escape", "sleep:0.5", "Home", "Down", "Right", "Right", "Right", "Return", "sleep:0.4",
                   "Up", "Right", "Right", "Down", "Return", "sleep:0.8"]),
    ("12-power", ["Escape", "Escape", "F1", "sleep:0.7", "Down", "Right", "Right", "Right", "Return", "sleep:0.6"]),  # Control Center → Power
    ("12b-notifications", ["Escape", "sleep:0.5", "F1", "sleep:0.7", "Right", "Return", "sleep:0.9"]),  # Control Center → the list
    # The in-game overlay, over a stand-in game: Play, then the overlay as a keyboard shortcut opens
    # it, then a screenshot (the panel goes, a notification shows over the game).
    ("13-to-play", ["run:touch $HOME/.play", "Escape", "sleep:0.4", "Escape", "sleep:0.4", "Up", "Up", "Left", "Return", "sleep:0.6",
                    "Down", "Home", "Right", "sleep:0.6", "Down", "sleep:0.4"]),
    ("13a-in-game", ["Return", "sleep:0.6", "Down", "sleep:0.4", "Return", "sleep:5"]),   # Your games → the game → Play
    ("13b-overlay", ["run:{bin} --overlay", "sleep:1.2"]),
    ("13c-screenshot-notice", ["run:{bin} --screenshot", "sleep:1.6"]),
]


def run(cmd, **kw):
    return subprocess.run(cmd, **kw)


def shoot(size: str, n: int, out: Path, binary: Path, game: Path | None, lang: str = ""):
    w, h = size.split("x")
    disp = f":{90 + n}"
    work = Path(tempfile.mkdtemp(prefix="ps5-ui-"))
    home = work / "home"
    (home / ".config/azrak").mkdir(parents=True)
    seed_cache(home)
    install_game(home / "Games", game)
    # A stand-in emulator: screenshots must never start a real game. It exits at once, unless the
    # run asks for a game in front (the in-game overlay's shots): then a full-screen picture.
    picture = work / "game.png"
    run(["convert", "-size", f"{w}x{h}", "gradient:#6c4b2a-#0c0907", "-gravity", "center", "-pointsize", "140",
         "-fill", "#ffffff30", "-annotate", "0", "STAND-IN GAME", str(picture)])
    fake = work / "kyty_emulator"
    fake.write_text(f"#!/bin/sh\n[ -e \"$HOME/.play\" ] && exec display -backdrop -background '#0c0907' -title 'Stand-in game' {picture}\nexit 0\n")
    fake.chmod(0o755)
    (home / ".config/azrak/config.json").write_text(json.dumps(
        {"game_dirs": ["~/Games"], "sounds": False, **NO_UPDATES, "emulator": str(fake), "seed_after_download": False,
         "language": lang or "en"}))
    seed_downloads(home)
    # No browsers or players may pop up; pretend to be an old version so the update notification shows.
    stubs = work / "stubs"
    stubs.mkdir()
    for tool in ("xdg-open", "mpv", "vlc", "ffplay", "firefox"):
        (stubs / tool).write_text("#!/bin/sh\nexit 0\n")
        (stubs / tool).chmod(0o755)
    # A stand-in sound server for Settings → Sound (the box has none): 62%, not muted, two outputs.
    (stubs / "pactl").write_text("""#!/bin/sh
case "$*" in
  "get-sink-volume @DEFAULT_SINK@") echo "Volume: front-left: 40632 /  62% / -12.4 dB,   front-right: 40632 /  62% / -12.4 dB" ;;
  "get-sink-mute @DEFAULT_SINK@") echo "Mute: no" ;;
  "get-default-sink") echo "alsa_output.speakers" ;;
  "-f json list sinks") echo '[{"name":"alsa_output.speakers","description":"Built-in Audio Analog Stereo","active_port":"analog-output-speaker"},{"name":"alsa_output.hdmi","description":"HDMI","active_port":"hdmi-output-0"}]' ;;
esac
exit 0
""")
    (stubs / "pactl").chmod(0o755)
    # Its own runtime folder (the command socket: never the real SwarmPlay's), pictures folder and
    # desktop (an X11 one: no GNOME helper, no session bus).
    (work / "run").mkdir(mode=0o700)
    env = {**os.environ, "DISPLAY": disp, "HOME": str(home), "PATH": f"{stubs}:{os.environ['PATH']}",
           "AZRAK_PRETEND_VERSION": "1.3.0", "SWARMPLAY_BOOT_HOLD_MS": "6000",
           "XDG_RUNTIME_DIR": str(work / "run"), "XDG_PICTURES_DIR": str(work / "pictures"), "XDG_CURRENT_DESKTOP": "XFCE",
           "SWARMPLAY_NO_PAD": "1"}
    for k in ("XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "DBUS_SESSION_BUS_ADDRESS"):
        env.pop(k, None)
    procs = [subprocess.Popen(["Xvfb", disp, "-screen", "0", f"{w}x{h}x24", "+extension", "GLX", "-noreset"],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)]
    time.sleep(1.5)
    # The compositor on: the in-game overlay is a see-through window over the game.
    procs.append(subprocess.Popen(["xfwm4", "--compositor=on"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
    time.sleep(1.5)
    # What it says goes to launcher.log in the run's folder (kept with UI_MATRIX_KEEP=1).
    log = open(work / "launcher.log", "w")
    app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT)
    procs.append(app)
    d = out / size
    d.mkdir(parents=True, exist_ok=True)
    try:
        # Its window by process, else by class (some X setups leave _NET_WM_PID unset); a start
        # with a big seeded cache can take several seconds.
        ids = []
        for _ in range(30):
            ids = (run(["xdotool", "search", "--pid", str(app.pid)], env=env, capture_output=True, text=True).stdout.split()
                   or run(["xdotool", "search", "--class", "swarmplay|azrak"], env=env, capture_output=True, text=True).stdout.split())
            if ids or app.poll() is not None:
                break
            time.sleep(1)
        if not ids:
            raise SystemExit(f"the launcher showed no window (exit code {app.poll()})")
        time.sleep(2)
        win = ids[-1]
        for name, keys in STEPS:
            for k in keys:
                if k.startswith("sleep:"):
                    time.sleep(float(k[6:]))
                    continue
                # A command, as a desktop shortcut would run it ({bin}: this launcher).
                if k.startswith("run:"):
                    run(["sh", "-c", k[4:].replace("{bin}", str(binary))], env=env, cwd=work)
                    time.sleep(0.25)
                    continue
                run(["xdotool", "windowfocus", win], env=env)
                run(["xdotool", "key", "--window", win, k], env=env)
                time.sleep(0.25)
            time.sleep(0.7)
            run(["import", "-window", "root", str(d / f"{name}.png")], env=env)
        pngs = sorted(str(p) for p in d.glob("[0-9]*.png"))
        run(["montage", *pngs, "-tile", "3x", "-geometry", "960x+6+6", "-background", "#333", str(out / f"sheet-{size}.png")])
        print("done", size, flush=True)
    finally:
        for p in reversed(procs):
            p.terminate()
        time.sleep(0.5)
        # The launcher's log beside the shots (the run's folder holds a copy of the cache: big).
        shutil.copy(work / "launcher.log", d / "launcher.log")
        if os.environ.get("UI_MATRIX_KEEP"):
            print("kept", work, flush=True)
        else:
            shutil.rmtree(work, ignore_errors=True)


def main():
    args = sys.argv[1:]
    lang = ""
    if "--lang" in args:
        i = args.index("--lang")
        lang = args[i + 1]
        del args[i:i + 2]
    sizes = args or SIZES
    # scripts/check.sh shoots the quick build; by itself, the release one.
    binary = Path(os.environ.get("SWARMPLAY_BIN", ROOT / "target/release/swarmplay")).resolve()
    out = ROOT / "dist/ui-matrix"
    if lang and lang != "en":
        out = out / lang
    out.mkdir(parents=True, exist_ok=True)
    game = Path(os.environ["DEMO_GAME"]) if os.environ.get("DEMO_GAME") else None
    # Another language's run gets displays of its own (check.sh runs both side by side).
    base = 20 if lang and lang != "en" else 0
    for i, s in enumerate(sizes):
        shoot(s, base + i, out, binary, game, lang)


if __name__ == "__main__":
    main()
