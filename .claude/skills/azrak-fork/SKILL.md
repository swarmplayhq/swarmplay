---
name: azrak-fork
description: Build, test, install, update and release SwarmPlay, the user's fork of Azrak Launcher (github.com/swarmplayhq/swarmplay, website swarmplayhq.github.io from github.com/swarmplayhq/swarmplayhq.github.io, a Rust/Slint game launcher) on their Bazzite machine. Use when asked to rebuild/reinstall Azrak, pull upstream changes into the fork, debug Azrak install/library problems, change the launcher's code, or publish a fork release.
---

# SwarmPlay (fork of Azrak Launcher)

The program and everything a person sees is **SwarmPlay** (binary `swarmplay`, icon `assets/swarmplay.svg`,
menu entry `swarmplay.desktop`). The Cargo package, data folders (`util::APP_NAME = "azrak"`), file markers
(`.azrak-owned`, `.azrak-game.json`), the game catalog (`update::REPO`) and AzrakOS keep upstream's names on
purpose: installs keep working and upstream merges stay easy.

- Source: `~/Projects/swarmplay` (`origin` = swarmplayhq/swarmplay, `upstream` = AzrakHQ/azrak-launcher).
- Installed binary: `~/.local/bin/swarmplay` (previous one kept as `swarmplay.previous`). Menu entry: `~/.local/share/applications/swarmplay.desktop`.
- User data: config `~/.config/azrak/config.json`, download jobs `~/.config/azrak/transfers/jobs.json`,
  install records `~/.config/azrak/installs/jobs.json`, emulators `~/.local/share/azrak`, cache `~/.cache/azrak`.
- Logs: the launcher logs to stderr, which lands in the user journal:
  `journalctl --user --since today --no-pager | grep 'swarmplay.desktop'`. Install failures are NOT logged there;
  they are in the UI and in `~/.config/azrak/installs/jobs.json` (`error` field).

## Change workflow (the user's rule)

Every fix or customization goes through a pull request into the fork's `main`:

1. `git checkout main && git pull --ff-only origin main && git checkout -b fix/<short-name>` (or `feat/<short-name>`).
2. Make the change with tests; build and run the full suite (below). One topic per branch, one commit per logical change.
3. `git push -u origin <branch>` and open the PR **on the fork**, always passing the repo explicitly, or `gh`
   targets upstream: `gh pr create --repo swarmplayhq/swarmplay --base main --head <branch> ...`
   (summary + test plan; end the body with the Claude Code attribution line).
4. Merge with a merge commit, `gh pr merge <n> --repo swarmplayhq/swarmplay --merge`, then sync local `main`
   and install the build.

Never push to `AzrakHQ/azrak-launcher` and never open a PR there unless the user asks for that specific change.
The user has read-only access upstream. Fixes useful to everyone (like the symlink fix) can be offered upstream
later, as their own branch made from `upstream/main` with only that commit.

Git identity in this repo: `Matheus Martucci Rodrigues <88255921+m1theusr@users.noreply.github.com>` (repo-local config).

## Host facts that matter

- Bazzite (Fedora Atomic): `/home` is a root-owned symlink to `var/home`, `$HOME=/home/m1theusr`.
  The installer walks paths with `O_NOFOLLOW`; `system_resolved()` in `src/installer.rs` follows only
  root-owned links in root-owned, non-world-writable folders. Don't weaken that check; extend it.
- The host has no Rust toolchain or `-devel` packages. Build inside the `azrak-dev` distrobox
  (Ubuntu 22.04 = same glibc 2.35 baseline as upstream releases). Rust lives in `~/.cargo` (shared home).
  If the box is missing, recreate it:
  ```bash
  distrobox create --yes --name azrak-dev --image docker.io/library/ubuntu:22.04
  distrobox enter azrak-dev -- bash -lc 'sudo apt-get update && sudo apt-get install -y build-essential pkg-config libfontconfig1-dev libxkbcommon-dev curl git nodejs && (command -v cargo || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal)'
  ```

## Build and test

While working, one command builds, runs the tests and shoots the screens (tests and screens side by
side), in about 2–3 minutes; a UI edit rebuilds in under a minute:

```bash
distrobox enter azrak-dev -- bash -c 'cd ~/Projects/swarmplay && scripts/check.sh'   # or: scripts/check.sh 1280x720 1920x1080
```

It uses the `quick` profile (Cargo.toml: our code unoptimised and incremental, libraries fully optimised
and built once, shared with the tests) and links with mold (`sudo apt install mold` in the box).
Group a whole area's work in one PR and run this once at the end, not after every step.

Only the build to install is a release build (~3 min), once per PR, after check.sh passed:

```bash
distrobox enter azrak-dev -- bash -c 'cd ~/Projects/swarmplay && ~/.cargo/bin/cargo build --release --locked'
```

Report test failures with their output; never install a build whose tests fail.

## Install (or reinstall) the build

The launcher must be closed first or it keeps running the old binary (Settings → Quit, or Ctrl+Q).

```bash
pgrep -x swarmplay && echo "close SwarmPlay first"   # ask the user to quit it; don't kill it while a game/install runs
install -Dm755 ~/.local/bin/swarmplay ~/.local/bin/swarmplay.previous 2>/dev/null || true
install -Dm755 ~/Projects/swarmplay/target/release/swarmplay ~/.local/bin/swarmplay
~/.local/bin/swarmplay --version
```

Rollback: `mv ~/.local/bin/swarmplay.previous ~/.local/bin/swarmplay`.

## Update from upstream

```bash
cd ~/Projects/swarmplay
git fetch upstream
git checkout main && git merge upstream/main   # resolve conflicts keeping the fork's fixes (see below)
```

Then build, test, install, and `git push origin main`.
Bump nothing in `Cargo.toml` by hand unless publishing a release; after a merge the version is upstream's.

## Self-update and releases

`src/update.rs`: `UPDATE_REPO` points self-update at the fork (`AZRAK_UPDATE_REPO` env overrides);
`REPO` stays upstream because the game catalog is published there. The fork has no releases unless
we publish one, so self-update reports "no releases found" — harmless. To publish one, the version
tag must be newer than the installed version (`v<Cargo version>`), Actions must be enabled on the fork,
and `.github/workflows/release.yml` builds the `azrak-linux-x86_64.tar.gz` asset on release publish.
Publishing is outward-facing: ask the user first.

## Fork changes to preserve when merging

1. `system_resolved()` + its use in `Dir::absolute` (`src/installer.rs`): installs work when `$HOME` is under a system symlink.
2. `UPDATE_REPO` in `src/update.rs`.
3. `existing_pc_game()` in `src/library.rs`: PC games installed some other way (a top-level `start`/`start.sh`/… script)
   show in the Library without writing a manifest into their folder.

## Screenshots (required for UI changes)

Never call a UI change done without looking at it. `scripts/ui_matrix.py` runs the real launcher on a
hidden X display with a throwaway home seeded from `~/.cache/azrak` and shoots 12 screens to
`dist/ui-matrix/<size>/` (check.sh runs it on the quick build; `SWARMPLAY_BIN` picks the binary,
the release one by default). Run it in the box WITHOUT the host's Wayland variables, or winit opens
the window on the user's real desktop instead of Xvfb:

```bash
distrobox enter azrak-dev -- env -u WAYLAND_DISPLAY python3 ~/Projects/swarmplay/scripts/ui_matrix.py 1920x1080
```

The run is isolated from the real SwarmPlay: its own runtime folder (the `swarmplay --overlay` socket),
pictures folder and desktop (`XDG_CURRENT_DESKTOP=XFCE`, no session bus), and `SWARMPLAY_NO_PAD=1` so it never
reads (or, with the overlay open, grabs) the real controller. The last steps play a stand-in game (a full-screen
picture) and drive the in-game overlay with `swarmplay --overlay` / `--screenshot`. The launcher's log is copied to
`dist/ui-matrix/<size>/launcher.log`; `UI_MATRIX_KEEP=1` keeps the run's folder (a ~2 GB copy of the cache, the
artwork linked, in /tmp: delete it after). `--lang pt_BR` shoots the same screens in Portuguese to
`dist/ui-matrix/pt_BR/<size>/` (check.sh runs both); `scripts/ui_audit.py --lang pt_BR` looks for cut-off highlights in it.
Look at the Portuguese ones too: its texts run 20–30% longer.

Never start the launcher for a test through `distrobox enter … bash -l…`: the login shell sets
`WAYLAND_DISPLAY=wayland-0` again and the window opens on the user's real desktop. In a shell, run
`unset WAYLAND_DISPLAY XDG_SESSION_TYPE` first. The window's class/app id is `swarmplay` (main.rs).

Box packages it needs (once): `xvfb xfwm4 xdotool imagemagick libgl1-mesa-dri libegl1 libxkbcommon-x11-0
libxcursor1 libxrandr2 libxi6 libx11-xcb1`. Take a "before" set on `main`, an "after" set on the branch,
compare the PNGs (Read them), and say in the PR what changed on screen.

## Translations (English + Brazilian Portuguese)

Every visible text goes through the translation: `@tr("…")` in `ui/*.slint`, `tr!("…")` (crate `tr`, the same
syntax) in Rust; plurals `tr!("{n} game" | "{n} games" % n)`; no format specs inside (`crate::i18n::num`,
`::date`, `::clock` format values). Values saved in config stay English keys, shown through a `label()` with
`tr!` literals. Not translated: game titles, paths, raw OS errors (after a translated label), logs, test texts.
One catalog, `i18n/pt_BR/LC_MESSAGES/azrak.po`, built into the binary; English is the source text. Words follow
`i18n/GLOSSARY.md` (the console's Brazilian terms). After adding or changing texts, in the box (it has gettext,
`xtr`, `slint-tr-extractor`):

```bash
distrobox enter azrak-dev -- bash -c 'cd ~/Projects/swarmplay && scripts/i18n.sh extract'   # new texts come in empty
```

then translate the empty `msgstr`s in the .po. `scripts/i18n.sh check` (run by check.sh) fails on an untranslated
text or a translation whose `{…}` differ from the English. The language switches live (Settings → Launcher →
Language → `App::relocalize`): anything new pushed to the window must be rebuilt there too.

## Style

Match the surrounding code: dense one-line `ensure!`/`match` arms, comments that say why in plain words,
tests named as sentences. The general `rust-skills` guidelines apply only where they don't fight this style;
upstream merges stay easier the closer the fork's code looks to upstream's.

## Review

For changes to `src/installer.rs` (it is the security boundary for untrusted downloads) run `/security-review`
and `/code-review` before installing.
