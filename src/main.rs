//! Azrak Launcher: a fast, native console-style game launcher for Linux.

mod app;
mod audio;
mod battery;
mod boot;
mod catalog;
mod compat;
mod platform;
mod sfo;
mod shad;
mod shad_ui;
mod pkgx;
mod sandbox;
mod search;
mod search_keys;
mod vpn;
mod pc;
mod pc_ui;
mod pcgame;
mod repack;
mod repack_ui;
mod os;
mod exfat;
mod control_center;
mod welcome;
mod theme;
mod toast;
mod sound;
mod storage;
mod folder_pick;
mod games;
mod game_library;
mod store;
mod profile_card;
mod onboard;
mod avatar;
mod profiles;
mod overlay;
mod overlay_ui;
mod perf_hud;
mod ipc;
mod wine_pads;
mod pc_pads;
mod pfs;
mod rar;
mod package;
mod results;
mod trailer;
mod config;
mod display;
mod download_ui;
mod setup_ui;
mod downloads;
mod gamepad;
mod i18n;
mod game_groups;
mod images;
mod installer;
mod install_ui;
mod kyty;
mod kyty_ui;
mod update;
mod library;
mod library_layout;
mod present;
mod psn;
mod sessions;
mod settings;
mod util;
mod versions;
mod words;

slint::include_modules!();

const HELP: &str = "\
SwarmPlay — a console-style game launcher for Linux

USAGE:
    swarmplay [OPTIONS]

OPTIONS:
    --windowed          Open in a normal window instead of fullscreen
    --monitor <NAME>    Display to use (e.g. DP-2); overrides Settings
    --sync              Reload the local RuTracker catalog on start
    --catalog <PATH>    Use an English RuTracker PS5 JSON snapshot
    --overlay           Open the in-game overlay of the SwarmPlay that's running
                        (bind it to a key in your desktop's keyboard settings)
    --home              Bring the running SwarmPlay's Home to the front
    --screenshot        Take a screenshot of the game, as Share does
    --sounds            Play every interface sound in turn, to hear them
    --version           Print the version
    -h, --help          Show this help
";

/// A PC game's torrent has thousands of files, and the download engine keeps every one open while
/// it writes them. Programs started from the desktop get 1024 open files at most, far too few:
/// starting such a download failed with "error opening … in read/write mode". The soft limit goes
/// up to what the system allows (the hard limit, capped), as Steam does; games started from here
/// inherit it, which Wine's esync wants too.
fn raise_open_files() -> Option<(u64, u64)> {
    let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) } != 0 { return None; }
    let before = r.rlim_cur as u64;
    let want = (r.rlim_max as u64).min(524_288);
    if before >= want { return Some((before, before)); }
    r.rlim_cur = want as libc::rlim_t;
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &r) } != 0 { return Some((before, before)); }
    Some((before, want))
}

fn main() {
    raise_open_files();
    // `--supports FEATURE`: exit 0 when this launcher has it, so AzrakOS can tell (older releases
    // reject the option). gamescope: it tags its window, so Gamescope shows it; os-install: the
    // USB stick's install screens (the stick's build checks the launcher it bundles has them).
    // See docs/AZRAKOS.md.
    if std::env::args().nth(1).as_deref() == Some("--supports") {
        let known = ["gamescope", "system-settings", "os-install"];
        std::process::exit(if std::env::args().nth(2).is_some_and(|f| known.contains(&f.as_str())) { 0 } else { 1 });
    }
    if std::env::args().nth(1).as_deref() == Some("--probe-gpu") {
        pc::print_gpus();
        return;
    }
    let mut windowed = false;
    let mut monitor: Option<String> = None;
    let mut force_sync = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--windowed" => windowed = true,
            "--monitor" => monitor = args.next(),
            "--catalog" => {
                let Some(path) = args.next().filter(|value| !value.starts_with("--")) else {
                    eprintln!("--catalog requires a JSON file path");
                    std::process::exit(2);
                };
                std::env::set_var("AZRAK_CATALOG_PATH", path);
            }
            "--sync" => force_sync = true,
            "--overlay" | "--home" | "--screenshot" => {
                if crate::ipc::send(&a[2..]) { return; }
                eprintln!("SwarmPlay isn't running");
                std::process::exit(1);
            }
            "--sounds" => {
                crate::audio::demo();
                return;
            }
            "--version" | "-V" => {
                println!("swarmplay {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return;
            }
            other => {
                eprintln!("unknown option: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }
    // First: a signal that ends the launcher is logged (see app.rs).
    app::watch_exit_signals();
    if force_sync {
        // Keep the last-known-good cache if an external snapshot is invalid.
        if let Err(error) = catalog::sync(&|message| eprintln!("{message}")) {
            eprintln!("Could not reload RuTracker catalog: {error}");
        }
    }

    let cfg = config::Config::load();
    let mons = display::monitors();
    let target = display::pick(&mons, monitor.as_deref().unwrap_or(&cfg.monitor));
    // The UI scales itself to the window (see App::update_scale); keep the toolkit at 1:1 so
    // window sizes are never changed behind our back.
    if std::env::var_os("SLINT_SCALE_FACTOR").is_none() {
        std::env::set_var("SLINT_SCALE_FACTOR", "1");
    }
    // Only the winit backend is compiled in; make sure Slint picks the OpenGL renderer.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        std::env::set_var("SLINT_BACKEND", "winit-femtovg");
    }
    let ui = AppWindow::new().expect("could not create the window (is a graphical session running?)");
    // The window's app id (Wayland) and class (X11): the desktop matches it to swarmplay.desktop
    // for the icon and name in the dock; scripts find the window by it. Without it there is none.
    // After AppWindow::new (the platform exists then) and before the window is shown (it's read
    // when the window is made).
    if let Err(e) = slint::set_xdg_app_id(crate::update::ICON_NAME) {
        eprintln!("could not set the app id: {e}");
    }
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    app::run(ui, mons, target, windowed);
}

#[cfg(test)]
mod open_files_tests {
    #[test]
    fn the_open_files_limit_goes_up_to_what_the_system_allows() {
        let (_, after) = super::raise_open_files().expect("readable limit");
        let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) }, 0);
        assert_eq!(r.rlim_cur as u64, after);
        assert!(after >= (r.rlim_max as u64).min(524_288), "as high as allowed (capped)");
    }
}
