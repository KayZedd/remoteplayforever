use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::i18n::tr;
use crate::session::{GuestInputs, HostCommand, HostEvent, Session};

/// AppID a Remote Play Together session is labelled with when the game is not a Steam app.
/// Every account has it (Spacewar, Valve's public test app); nothing is installed. The game runs
/// with the Steam overlay registered under this id (see `overlay.rs`), which is what routes
/// Steam Input and guest controllers into it. Video is the desktop stream.
pub const NON_STEAM_SESSION_APPID: u32 = 480;

const STEAM_LAUNCH_TIMEOUT: Duration = Duration::from_secs(180);
/// Time for the game to create its window before the session is opened on top of it.
const GAME_WARMUP: Duration = Duration::from_secs(4);
/// A non-Steam process that exits this fast was most likely a launcher handing off to the real game.
const LAUNCHER_HANDOFF: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(300);

pub fn get_log_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("RemotePlayForever");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("launch.log")
}

pub fn log(msg: &str) {
    use std::io::Write;
    let path = get_log_path();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{now}] {msg}");
        let _ = f.flush();
    }
}

/// How a target gets started and which AppID its Remote Play Together session is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Installed Steam game: Steam launches it under its own AppID (own license, overlay, DRM).
    SteamApp { appid: u32 },
    /// Anything else: started with the Steam overlay, shared by streaming the desktop.
    External,
}

impl LaunchPlan {
    pub fn for_target(target: &Path) -> Self {
        match crate::steam::detect_target_steam_appid(target) {
            Some(appid) => LaunchPlan::SteamApp { appid },
            None => LaunchPlan::External,
        }
    }

    pub fn session_appid(&self) -> u32 {
        match self {
            LaunchPlan::SteamApp { appid } => *appid,
            LaunchPlan::External => NON_STEAM_SESSION_APPID,
        }
    }

    pub fn desktop_stream(&self) -> bool {
        matches!(self, LaunchPlan::External)
    }
}

/// Paths arrive from text fields, drag & drop and shell verbs: strip quotes and stray separators.
pub fn clean_target_path(raw: &Path) -> PathBuf {
    let cleaned = raw
        .to_string_lossy()
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .trim_end_matches('\\')
        .trim_end_matches('/')
        .to_string();
    PathBuf::from(cleaned)
}

/// A game launch with its Remote Play Together session, running on a background thread.
pub struct ActiveLaunch {
    events: Receiver<HostEvent>,
    commands: Sender<HostCommand>,
    pub plan: LaunchPlan,
    pub game_name: String,
}

impl ActiveLaunch {
    /// Pending events, plus whether the launch is over (`HostEvent::Stopped` is always last).
    pub fn poll(&self) -> (Vec<HostEvent>, bool) {
        let mut events = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(ev) => events.push(ev),
                Err(TryRecvError::Empty) => return (events, false),
                Err(TryRecvError::Disconnected) => return (events, true),
            }
        }
    }

    /// Block until the next event; `None` once the launch is over.
    pub fn wait_event(&self) -> Option<HostEvent> {
        self.events.recv().ok()
    }

    pub fn send(&self, cmd: HostCommand) {
        let _ = self.commands.send(cmd);
    }
}

/// Start `target` and open a Remote Play Together session for it.
pub fn start_launch(target: &Path, args: &[String], inputs: GuestInputs) -> Result<ActiveLaunch> {
    let target = clean_target_path(target);
    if !target.exists() {
        log(&format!("Error: target executable does not exist: {:?}", target));
        anyhow::bail!("{}: {}", tr("The game executable does not exist", "Plik wykonywalny gry nie istnieje"), target.display());
    }
    if crate::steam::active_user() == 0 {
        anyhow::bail!("{}", tr("Steam is not running or you are not signed in. Start Steam and try again.", "Steam nie jest uruchomiony albo nie jesteś zalogowany. Uruchom Steam i spróbuj ponownie."));
    }

    let plan = LaunchPlan::for_target(&target);
    let game_name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("Game").to_string();
    log(&format!("start_launch: {:?} args={:?} plan={:?} inputs={:?}", target, args, plan, inputs));

    let (event_tx, events) = mpsc::channel();
    let (commands, command_rx) = mpsc::channel();
    let args = args.to_vec();
    std::thread::spawn(move || {
        if let Err(e) = run_launch(&target, &args, plan, inputs, &event_tx, &command_rx) {
            log(&format!("run_launch failed: {e:#}"));
            let _ = event_tx.send(HostEvent::Error(format!("{e:#}")));
        }
        let _ = event_tx.send(HostEvent::Stopped);
    });

    Ok(ActiveLaunch { events, commands, plan, game_name })
}

enum Game {
    Steam { appid: u32, misses: u32 },
    Child { child: std::process::Child, started: Instant, detached: bool },
}

impl Game {
    /// False once the game is gone and the session should end with it.
    fn alive(&mut self, events: &Sender<HostEvent>) -> bool {
        match self {
            // RunningAppID flickers while Steam swaps processes (launchers); require a few misses
            Game::Steam { appid, misses } => {
                *misses = if crate::steam::running_appid() == *appid { 0 } else { *misses + 1 };
                *misses < 5
            }
            Game::Child { detached: true, .. } => true,
            Game::Child { child, started, detached } => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(exit)) if started.elapsed() < LAUNCHER_HANDOFF => {
                    log(&format!("Start process exited after {:?} ({exit}); treating it as a launcher", started.elapsed()));
                    let _ = events.send(HostEvent::Status(
                        tr("The start process exited right away (a launcher). The session stays open until you end it.", "Proces startowy zakończył się od razu (launcher). Sesja trwa do ręcznego zakończenia.").into(),
                    ));
                    *detached = true;
                    true
                }
                _ => false,
            },
        }
    }
}

/// Sleep for `duration`; false if a Stop arrived (or the caller went away) meanwhile.
fn wait_unless_stopped(duration: Duration, commands: &Receiver<HostCommand>) -> bool {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        match commands.try_recv() {
            Ok(HostCommand::Stop) | Err(TryRecvError::Disconnected) => return false,
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    true
}

/// Command that starts `target` from its own folder (batch files go through cmd.exe).
pub fn command_for(target: &Path) -> std::process::Command {
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut command = if ext == "bat" || ext == "cmd" {
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.arg("/c").arg(target);
        cmd
    } else {
        std::process::Command::new(target)
    };
    command.current_dir(target.parent().unwrap_or_else(|| Path::new(".")));
    command
}

/// Start a non-Steam game through the overlay launcher (see `overlay.rs`), which stays alive
/// exactly as long as the game does.
fn spawn_external(target: &Path, args: &[String]) -> Result<std::process::Child> {
    let exe = std::env::current_exe().context("current_exe")?;
    let mut command = std::process::Command::new(exe);
    command.arg("--overlay-launch").arg(NON_STEAM_SESSION_APPID.to_string()).arg(target).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().with_context(|| format!("{}: {}", tr("Could not start the process", "Nie udało się uruchomić procesu"), target.display()))
}

fn run_launch(
    target: &Path,
    args: &[String],
    plan: LaunchPlan,
    inputs: GuestInputs,
    events: &Sender<HostEvent>,
    commands: &Receiver<HostCommand>,
) -> Result<()> {
    let status = |msg: String| {
        log(&msg);
        let _ = events.send(HostEvent::Status(msg));
    };

    let mut game = match plan {
        LaunchPlan::SteamApp { appid } => {
            if crate::steam::running_appid() != appid {
                status(format!("{} (AppID {appid})...", tr("Starting the game through Steam", "Uruchamianie gry przez Steam")));
                crate::steam::launch_steam_app(appid, args)?;
                let deadline = Instant::now() + STEAM_LAUNCH_TIMEOUT;
                while crate::steam::running_appid() != appid {
                    if Instant::now() > deadline {
                        anyhow::bail!("{} (AppID {appid}, {} s)", tr("Steam did not start the game in time", "Steam nie uruchomił gry na czas"), STEAM_LAUNCH_TIMEOUT.as_secs());
                    }
                    if !wait_unless_stopped(Duration::from_millis(500), commands) {
                        return Ok(());
                    }
                }
            }
            Game::Steam { appid, misses: 0 }
        }
        LaunchPlan::External => {
            status(format!("{} {}...", tr("Starting", "Uruchamianie"), target.display()));
            let child = spawn_external(target, args)?;
            log(&format!("Child process spawned, PID: {}", child.id()));
            Game::Child { child, started: Instant::now(), detached: false }
        }
    };

    status(tr("Game started. Opening the Remote Play Together session...", "Gra uruchomiona. Otwieranie sesji Remote Play Together...").into());
    if !wait_unless_stopped(GAME_WARMUP, commands) {
        return Ok(());
    }

    let mut session = Session::start(plan.session_appid(), plan.desktop_stream(), inputs)?;
    loop {
        loop {
            match session.events.try_recv() {
                Ok(HostEvent::Stopped) => {}
                Ok(ev) => {
                    let _ = events.send(ev);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    log("Session host exited");
                    return Ok(());
                }
            }
        }
        loop {
            match commands.try_recv() {
                Ok(HostCommand::Stop) | Err(TryRecvError::Disconnected) => {
                    session.stop();
                    return Ok(());
                }
                Ok(cmd) => session.send(cmd),
                Err(TryRecvError::Empty) => break,
            }
        }
        if !game.alive(events) {
            status(tr("Game closed. Ending the session...", "Gra zakończona. Zamykanie sesji...").into());
            session.stop();
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_target_path_strips_quotes_and_separators() {
        let expect = PathBuf::from(r"C:\Gry\Gra\gra.exe");
        assert_eq!(clean_target_path(Path::new(r#""C:\Gry\Gra\gra.exe""#)), expect);
        assert_eq!(clean_target_path(Path::new(r" C:\Gry\Gra\gra.exe\ ")), expect);
        assert_eq!(clean_target_path(Path::new(r"'C:\Gry\Gra\gra.exe'")), expect);
        assert_eq!(clean_target_path(Path::new(r"C:\Gry\Gra\gra.exe")), expect);
    }

    #[test]
    fn batch_files_run_through_cmd_from_their_own_folder() {
        let bat = command_for(Path::new(r"C:\Gry\Gra\start.bat"));
        assert_eq!(bat.get_program(), "cmd.exe");
        assert_eq!(bat.get_current_dir(), Some(Path::new(r"C:\Gry\Gra")));

        let exe = command_for(Path::new(r"C:\Gry\Gra\gra.exe"));
        assert_eq!(exe.get_program(), r"C:\Gry\Gra\gra.exe");
        assert_eq!(exe.get_args().count(), 0);
    }

    #[test]
    fn plan_for_non_steam_path_streams_desktop_under_fallback_appid() {
        let plan = LaunchPlan::for_target(Path::new(r"C:\Windows\System32\notepad.exe"));
        assert_eq!(plan, LaunchPlan::External);
        assert_eq!(plan.session_appid(), NON_STEAM_SESSION_APPID);
        assert!(plan.desktop_stream());
    }

    #[test]
    fn plan_for_steam_app_uses_its_own_appid_and_game_capture() {
        let plan = LaunchPlan::SteamApp { appid: 312530 };
        assert_eq!(plan.session_appid(), 312530);
        assert!(!plan.desktop_stream());
    }
}
