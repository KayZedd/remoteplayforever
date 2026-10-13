use anyhow::Result;
use colored::Colorize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use crate::config::Config;
use crate::i18n::tr;
use crate::runner::{start_launch, LaunchPlan};
use crate::session::HostEvent;
use crate::steam::SteamInstallation;

const RULE: &str = "─────────────────────────────────────────────────────────────────────────────";

/// Console interface (`--cli`).
pub struct UI;

impl UI {
    pub fn clear_screen() {
        print!("{esc}[2J{esc}[1;1H", esc = 27 as char);
        let _ = io::stdout().flush();
    }

    pub fn print_banner(steam_inst: Option<&SteamInstallation>) {
        println!("{}", format!("🎮 RemotePlayForever v{}", env!("CARGO_PKG_VERSION")).bold().yellow());
        println!("{}", tr("Steam Remote Play Together for any game", "Steam Remote Play Together dla dowolnej gry").bright_white());
        println!("{}", RULE.bright_black());

        match steam_inst {
            Some(steam) => println!("  Steam:  {}", steam.steam_path.display().to_string().cyan()),
            None => println!("  Steam:  {}", tr("installation not found!", "nie wykryto instalacji!").red()),
        }
        if crate::steam::active_user() != 0 {
            println!("  Status: {}\n", tr("Steam is running and signed in", "Steam działa, użytkownik zalogowany").green());
        } else {
            println!("  Status: {}\n", tr("Steam is not running or you are not signed in", "Steam nie działa lub nie jesteś zalogowany").red());
        }
    }

    pub fn pick_file_dialog() -> Option<PathBuf> {
        println!("{}", tr("Opening the file picker...", "Otwieranie okna wyboru pliku...").cyan());
        rfd::FileDialog::new()
            .add_filter(tr("Executables (*.exe)", "Pliki wykonywalne (*.exe)"), &["exe"])
            .set_title(tr("Pick the game executable (.exe)", "Wybierz plik wykonywalny gry (.exe)"))
            .pick_file()
    }

    pub fn prompt(text: &str) -> String {
        print!("{} ", text);
        let _ = io::stdout().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        input.trim().to_string()
    }

    pub fn pause() {
        println!("\n{}", tr("Press Enter to continue...", "Wciśnij Enter, aby kontynuować...").dimmed());
        let _ = io::stdout().flush();
        let mut buf = String::new();
        let _ = io::stdin().read_line(&mut buf);
    }

    /// Put text on the Windows clipboard (links are ASCII, which is all clip.exe handles reliably).
    pub fn copy_to_clipboard(text: &str) {
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("clip").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }

    /// Launch a game with a Remote Play Together session and block until the session ends.
    pub fn launch_and_wait(config: &Config, target: &Path, args: &[String]) -> Result<()> {
        let inputs = config.guest_inputs(target);
        let launch = start_launch(target, args, inputs)?;

        println!("\n{}", RULE.cyan());
        println!("  🎮 {}", format!("{}: {}", tr("STARTING", "URUCHAMIANIE"), launch.game_name).bold().green());
        match launch.plan {
            LaunchPlan::SteamApp { appid } => println!(
                "  {} (AppID {appid}): {}",
                tr("Steam game", "Gra ze Steam"),
                tr("Steam starts it, guests see the game window.", "startuje przez Steam, goście widzą okno gry.")
            ),
            LaunchPlan::External => println!(
                "  {}",
                tr(
                    "Non-Steam game: runs with the Steam overlay, guests see your WHOLE desktop.",
                    "Gra spoza Steam: startuje z nakładką Steam, goście widzą CAŁY pulpit.",
                )
                .yellow()
            ),
        }
        let allowed: Vec<&str> = [
            (inputs.keyboard, tr("keyboard", "klawiatura")),
            (inputs.mouse, tr("mouse", "mysz")),
            (inputs.controller, tr("controller", "kontroler")),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        println!("  {}: {}", tr("Guests may use", "Goście mogą używać"), allowed.join(", "));
        println!("  {}", tr("Close the game or press Ctrl+C to end the session.", "Zamknij grę albo wciśnij Ctrl+C, aby zakończyć sesję."));
        println!("{}\n", RULE.cyan());

        while let Some(event) = launch.wait_event() {
            match event {
                HostEvent::Status(msg) => println!("⏳ {msg}"),
                HostEvent::Link(url) => {
                    if config.copy_link_automatically {
                        Self::copy_to_clipboard(&url);
                    }
                    println!("\n🔗 {} {}", tr("Guest link:", "Link dla gościa:").bold().green(), url.bold().yellow());
                    println!(
                        "   {}\n",
                        tr(
                            "Send it to a friend. They do not need the game, only Steam or the Steam Link app.",
                            "Wyślij go znajomemu. Nie musi mieć gry, wystarczy Steam lub aplikacja Steam Link.",
                        )
                    );
                }
                HostEvent::Ready => println!("{}", tr("✅ The Remote Play Together session is live.", "✅ Sesja Remote Play Together jest aktywna.").green()),
                HostEvent::GuestConnected => println!("{}", tr("🟢 A guest connected to the stream.", "🟢 Gość połączył się ze strumieniem.").green()),
                HostEvent::Player { name, .. } => println!("👤 {}: {}", tr("Joined", "Dołączył"), name.bold()),
                HostEvent::Error(msg) => println!("{} {msg}", tr("❌ Error:", "❌ Błąd:").red().bold()),
                HostEvent::Stopped => println!("🛑 {}", tr("Session ended.", "Sesja zakończona.")),
            }
        }
        Ok(())
    }

    /// Run the main interactive menu
    pub fn main_menu(mut config: Config, steam_inst: Option<SteamInstallation>) -> Result<()> {
        loop {
            Self::clear_screen();
            Self::print_banner(steam_inst.as_ref());

            println!("{}", RULE.bright_black());
            println!("  {} {}", "[1]".bold().green(), tr("Pick a game file (.exe) and launch it", "Wybierz plik gry (.exe) i uruchom"));
            println!("  {} {}", "[2]".bold().green(), tr("Pick an installed Steam game", "Wybierz zainstalowaną grę Steam"));
            println!("  {} {} ({})", "[3]".bold().green(), tr("Recent games", "Ostatnio uruchamiane"), config.recent_games.len().to_string().cyan());
            println!("  {} {}", "[4]".bold().cyan(), tr("Create a desktop shortcut for a game", "Utwórz skrót na pulpicie do gry"));
            println!("  {} {}", "[5]".bold().cyan(), tr("Guide: how it works and how to invite", "Poradnik: jak to działa i jak zapraszać"));
            println!("  {} {}", "[0]".bold().red(), tr("Exit", "Wyjście"));
            println!("{}", RULE.bright_black());

            match Self::prompt(tr("Choose [0-5]:", "Wybierz opcję [0-5]:")).as_str() {
                "1" => Self::menu_pick_and_launch(&mut config),
                "2" => match steam_inst {
                    Some(ref steam) => Self::menu_launch_steam_game(&mut config, steam),
                    None => {
                        println!("{}", tr("Steam installation not found!", "Nie znaleziono instalacji Steam!").red());
                        Self::pause();
                    }
                },
                "3" => Self::menu_recent_games(&mut config),
                "4" => Self::menu_create_shortcut(),
                "5" => Self::show_guide(),
                "0" | "q" | "exit" => break,
                _ => {}
            }
        }

        Ok(())
    }

    fn launch_from_menu(config: &mut Config, path: &Path, args: &[String]) {
        config.add_recent(path, args);
        if let Err(e) = Self::launch_and_wait(config, path, args) {
            println!("{} {e:#}", tr("❌ Launch failed:", "❌ Błąd uruchamiania:").red().bold());
        }
        Self::pause();
    }

    /// File picker with a typed-path fallback; `None` when the user gave nothing usable.
    fn ask_for_game() -> Option<PathBuf> {
        let path = Self::pick_file_dialog().or_else(|| {
            let manual = Self::prompt(tr("Path to the .exe (empty to cancel):", "Ścieżka do pliku .exe (puste, aby anulować):"));
            (!manual.is_empty()).then(|| PathBuf::from(manual))
        })?;
        if path.exists() {
            return Some(path);
        }
        println!("{}", tr("That file does not exist!", "Podany plik nie istnieje!").red());
        Self::pause();
        None
    }

    fn menu_pick_and_launch(config: &mut Config) {
        let Some(path) = Self::ask_for_game() else { return };
        let args_str = Self::prompt(tr("Extra arguments (optional, e.g. -fullscreen):", "Dodatkowe argumenty (opcjonalnie, np. -fullscreen):"));
        let args: Vec<String> = args_str.split_whitespace().map(|s| s.to_string()).collect();
        Self::launch_from_menu(config, &path, &args);
    }

    fn menu_launch_steam_game(config: &mut Config, steam: &SteamInstallation) {
        let apps: Vec<_> = steam.get_installed_apps().into_iter().filter(|a| a.executable.is_some()).collect();
        if apps.is_empty() {
            println!("{}", tr("No installed Steam games found!", "Nie znaleziono zainstalowanych gier Steam!").yellow());
            Self::pause();
            return;
        }

        Self::clear_screen();
        println!("{}\n", tr("INSTALLED STEAM GAMES:", "ZAINSTALOWANE GRY STEAM:").bold().cyan());
        for (idx, app) in apps.iter().enumerate() {
            println!("  [{:2}] {} (AppID: {})", idx + 1, app.name.bold(), app.appid.to_string().yellow());
        }
        println!("\n  {} {}", "[0]".red(), tr("Cancel", "Anuluj"));

        let choice = Self::prompt(tr("Game number:", "Numer gry:"));
        let Some(app) = choice.parse::<usize>().ok().and_then(|i| i.checked_sub(1)).and_then(|i| apps.get(i)) else { return };
        if let Some(ref exe) = app.executable {
            Self::launch_from_menu(config, exe, &[]);
        }
    }

    fn menu_recent_games(config: &mut Config) {
        if config.recent_games.is_empty() {
            println!("\n{}", tr("No recent games yet.", "Brak historii uruchamianych gier.").yellow());
            Self::pause();
            return;
        }

        Self::clear_screen();
        println!("{}\n", tr("RECENT GAMES:", "OSTATNIO URUCHAMIANE:").bold().cyan());
        for (idx, game) in config.recent_games.iter().enumerate() {
            let state = if game.path.exists() { "OK".green() } else { tr("file missing", "brak pliku").red() };
            println!("  [{:2}] {} [{}]", idx + 1, game.name.bold(), state);
            println!("       {}", game.path.display().to_string().dimmed());
            if !game.args.is_empty() {
                println!("       {}", game.args.join(" ").dimmed());
            }
        }
        println!("\n  {} {}", "[0]".red(), tr("Cancel", "Anuluj"));

        let choice = Self::prompt(tr("Game number:", "Numer gry:"));
        let Some(game) = choice.parse::<usize>().ok().and_then(|i| i.checked_sub(1)).and_then(|i| config.recent_games.get(i)).cloned() else { return };
        Self::launch_from_menu(config, &game.path, &game.args);
    }

    fn menu_create_shortcut() {
        let Some(path) = Self::ask_for_game() else { return };
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Game");
        match crate::installer::create_desktop_shortcut(name, &path) {
            Ok(p) => println!("{} {}", tr("Created:", "Utworzono:").green(), p.display().to_string().cyan()),
            Err(e) => println!("{} {e:#}", tr("Could not create the shortcut:", "Nie udało się utworzyć skrótu:").red()),
        }
        Self::pause();
    }

    pub fn show_guide() {
        Self::clear_screen();
        println!("{}\n", tr("HOW IT WORKS", "JAK TO DZIAŁA").bold().cyan());
        for line in crate::gui::guide_steps() {
            println!("{}", line.0.bold());
            println!("   {}\n", line.1);
        }
        Self::pause();
    }
}
