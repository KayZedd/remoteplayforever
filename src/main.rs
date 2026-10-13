#![windows_subsystem = "windows"]

mod compat;
mod config;
mod gui;
mod i18n;
mod installer;
mod overlay;
mod rpt;
mod runner;
mod session;
mod steam;
mod ui;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Default, Clone)]
#[command(
    name = "remoteplayforever",
    about = "Steam Remote Play Together for any game, Steam or not",
    version
)]
struct Cli {
    /// Game executable to launch with a Remote Play Together session
    #[arg(short, long)]
    run: Option<PathBuf>,

    /// Command-line arguments passed to the game
    #[arg(short, long, num_args = 1..)]
    args: Vec<String>,

    /// Use the console interface instead of the window
    #[arg(long)]
    cli: bool,

    /// List installed Steam games and exit
    #[arg(long)]
    list_games: bool,

    /// Internal: session host process for the given AppID
    #[arg(long, hide = true, value_name = "APPID")]
    host_session: Option<u32>,

    /// Internal: start a non-Steam game with the Steam overlay under the given game id
    #[arg(long, hide = true, value_name = "GAMEID")]
    overlay_launch: Option<u32>,

    /// Internal (session host): stream the desktop instead of the game window
    #[arg(long, hide = true)]
    desktop_stream: bool,

    /// Internal (session host): guests start without the keyboard
    #[arg(long, hide = true)]
    no_keyboard: bool,

    /// Internal (session host): guests start without the mouse
    #[arg(long, hide = true)]
    no_mouse: bool,

    /// Internal (session host): guests start without a controller
    #[arg(long, hide = true)]
    no_controller: bool,

    /// A dropped game file, or extra arguments for the game
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub extra_args: Vec<String>,
}

fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        // If launched from an existing console (PowerShell / cmd), attach to it so stdout works.
        // If launched from Explorer or Steam, this fails silently and creates NO window.
        windows_sys::Win32::System::Console::AttachConsole(u32::MAX);
        // UTF-8 console output (non-ASCII text, box drawing)
        windows_sys::Win32::System::Console::SetConsoleOutputCP(65001);
    }

    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {}", info);
        runner::log(&msg);
    }));

    let mut cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) if matches!(e.kind(), clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion) => {
            let _ = e.print();
            return Ok(());
        }
        Err(e) => {
            runner::log(&format!("Cli parse fallback: {}", e));
            Cli::default()
        }
    };

    let mut config = config::Config::load();
    i18n::set_language(config.language);

    // Helper processes are children of this same executable; they must not run the app startup below.
    if let Some(game_id) = cli.overlay_launch {
        let Some((target, args)) = cli.extra_args.split_first() else { std::process::exit(2) };
        std::process::exit(overlay::run_launcher(game_id, std::path::Path::new(target), args));
    }
    if let Some(appid) = cli.host_session {
        let inputs = session::GuestInputs {
            keyboard: !cli.no_keyboard,
            mouse: !cli.no_mouse,
            controller: !cli.no_controller,
        };
        std::process::exit(rpt::run_host(appid, cli.desktop_stream, inputs));
    }

    runner::log("--- RemotePlayForever started ---");
    runner::log(&format!("Args: {:?}", std::env::args().collect::<Vec<_>>()));

    // If cli.run is a partial path because of unquoted spaces in CLI arguments:
    if let Some(ref r) = cli.run {
        if !r.exists() && !cli.extra_args.is_empty() {
            let mut combined = r.to_string_lossy().to_string();
            let mut matched_index = None;
            for (idx, arg) in cli.extra_args.iter().enumerate() {
                combined.push(' ');
                combined.push_str(arg);
                let test_p = PathBuf::from(&combined);
                if test_p.exists() {
                    cli.run = Some(test_p);
                    matched_index = Some(idx);
                    break;
                }
            }
            if let Some(idx) = matched_index {
                cli.extra_args.drain(0..=idx);
            }
        }
    }

    // Support drag & drop or a direct executable argument:
    if cli.run.is_none() {
        if let Some(first) = cli.extra_args.first().cloned() {
            let path = PathBuf::from(&first);
            if path.exists() && (first.to_lowercase().ends_with(".exe") || first.to_lowercase().ends_with(".bat") || first.to_lowercase().ends_with(".cmd")) {
                cli.run = Some(path);
                cli.extra_args.remove(0);
                if cli.args.is_empty() {
                    cli.args = cli.extra_args.clone();
                }
            }
        }
    }

    if let Err(e) = run_app(cli, &mut config) {
        runner::log(&format!("FATAL ERROR in run_app: {:?}", e));
        eprintln!("\n{}: {:?}", i18n::tr("RemotePlayForever failed", "Wystąpił błąd w RemotePlayForever"), e);
        eprintln!("{}: {}", i18n::tr("Log", "Log"), runner::get_log_path().display());
        return Err(e);
    }
    runner::log("run_app finished cleanly");

    Ok(())
}

fn run_app(cli: Cli, config: &mut config::Config) -> Result<()> {
    let steam_inst = steam::SteamInstallation::detect().ok();

    if cli.list_games {
        if let Some(ref steam) = steam_inst {
            let apps = steam.get_installed_apps();
            println!("{} ({}):", i18n::tr("Installed Steam games", "Zainstalowane gry Steam"), apps.len());
            for app in apps {
                println!("- {} (AppID: {}) [{}]", app.name, app.appid, app.install_dir.display());
            }
        } else {
            eprintln!("{}", i18n::tr("Steam installation not found.", "Nie wykryto instalacji Steam."));
        }
        return Ok(());
    }

    if cli.cli {
        if let Some(ref target) = cli.run {
            config.add_recent(target, &cli.args);
            return ui::UI::launch_and_wait(config, target, &cli.args);
        }
        return ui::UI::main_menu(config.clone(), steam_inst);
    }

    let autostart = cli.run.map(|target| (target, cli.args));
    gui::RemotePlayApp::run(autostart).map_err(|e| anyhow::anyhow!("{e}"))
}
