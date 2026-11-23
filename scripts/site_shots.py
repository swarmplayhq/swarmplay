#!/usr/bin/env python3
"""Screenshots for the website (swarmplayhq.github.io): the real launcher, like ui_matrix.py, but
with PC games installed and the Library on "Installed" (no catalog or downloads on screen).

    scripts/site_shots.py [out-folder]        # default: dist/site-shots

Writes <out>/1920x1080/<screen>.png. Runs the release build (SWARMPLAY_BIN overrides it).
Needs what ui_matrix.py needs: Xvfb, xfwm4, xdotool, ImageMagick, plus your PC catalogs
(~/.local/share/azrak/rutracker, which the launcher downloads) for the games' art.
"""

import json
import os
import shutil
import sys
from pathlib import Path

import ui_fixture
import ui_matrix

# PC games with Steam art in the catalogs, installed as stand-ins: a folder with a start script
# that does nothing.
GAMES = ["Hades II", "Cyberpunk 2077", "Baldur's Gate III", "Hollow Knight: Silksong",
         "Stardew Valley", "Celeste", "Balatro", "Dead Cells", "Outer Wilds", "Terraria"]

STEPS = [
    ("home", ["sleep:8", "Return", "sleep:5"]),   # past who's playing
    # Home's last tile: the Game Library (installed stand-ins only, no catalog on screen).
    ("game-library", ["End", "sleep:0.6", "Return", "sleep:2.5"]),
    ("welcome", ["Escape", "sleep:0.8", "Home", "sleep:0.4", "Down", "sleep:0.8"]),
    ("home-peek", ["Up", "Right", "Down", "Down", "sleep:1"]),
    ("hub", ["Return", "sleep:1.5"]),
    ("hub-scrolled", ["Down", "sleep:1.2", "Down", "Down", "sleep:0.8"]),
    ("gallery", ["Return", "sleep:1.2", "Right", "sleep:1"]),
    # The avatar at the top: the profile card.
    ("profile", ["Escape", "sleep:0.6", "Escape", "sleep:0.8", "Up", "Up", "sleep:0.3"] + ["Right", "sleep:0.2"] * 6 + ["Return", "sleep:1"]),
    ("settings-storage", ["Escape", "sleep:0.8", "s", "sleep:1", "Down", "Return", "sleep:1.2"]),
    ("control-center", ["Escape", "Escape", "sleep:0.5", "Escape", "sleep:0.5", "F1", "sleep:1"]),
]


def seed_cache(home: Path) -> None:
    """Your cache, plus the PC catalogs the launcher reads from its data folder."""
    ui_fixture.seed_cache(home)
    src = Path.home() / ".local/share/azrak/rutracker"
    dst = home / ".local/share/azrak/rutracker"
    dst.mkdir(parents=True, exist_ok=True)
    for name in ("linux-topics.json", "wine-topics.json"):
        if (src / name).exists():
            shutil.copy(src / name, dst / name)


def install_games(games: Path, _game) -> None:
    games.mkdir(parents=True, exist_ok=True)
    for name in GAMES:
        folder = games / name
        folder.mkdir(parents=True, exist_ok=True)
        start = folder / "start"
        start.write_text("#!/bin/sh\nexit 0\n")
        start.chmod(0o755)


def library_on_installed(home: Path) -> None:
    """Runs once ui_matrix has written the config: the Library opens on the installed games."""
    path = home / ".config/azrak/config.json"
    config = json.loads(path.read_text())
    config["library_status"] = "Installed"
    path.write_text(json.dumps(config))


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ui_matrix.ROOT / "dist/site-shots").resolve()
    binary = Path(os.environ.get("SWARMPLAY_BIN", ui_matrix.ROOT / "target/release/swarmplay")).resolve()
    os.environ.pop("WAYLAND_DISPLAY", None)  # the hidden X display only
    ui_matrix.STEPS = STEPS
    ui_matrix.seed_cache = seed_cache
    ui_matrix.install_game = install_games
    ui_matrix.seed_downloads = library_on_installed
    ui_matrix.shoot("1920x1080", 0, out, binary, None)


if __name__ == "__main__":
    main()
