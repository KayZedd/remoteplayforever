//! Starts a non-Steam game with the Steam overlay inside it (runs as its own process:
//! `--overlay-launch <gameid> <exe> [args]`).
//!
//! The overlay is what gives a game Steam Input and Remote Play Together guest controllers, and
//! lets Steam capture the game window instead of the desktop. Steam only puts it into games it
//! launches itself, but the overlay DLL follows child processes on its own: load Steam's
//! `GameOverlayRenderer64.dll` here, tell it which game id we are through the environment, and
//! every process started from here gets the overlay under that id. Nothing in the game's folder
//! is touched and no foreign code is written into the game by us.
//!
//! It is a separate, windowless process so the overlay never hooks the GUI's own renderer.

use crate::runner::log;
use std::path::Path;

/// Run `target` under the overlay and wait for it. Returns the process exit code.
pub fn run_launcher(game_id: u32, target: &Path, args: &[String]) -> i32 {
    let id = game_id.to_string();
    // Read by the overlay DLL when it loads, and inherited by the game.
    unsafe {
        std::env::set_var("SteamGameId", &id);
        std::env::set_var("SteamOverlayGameId", &id);
    }

    match load_overlay() {
        Ok(path) => log(&format!("overlay: loaded {path} for game id {id}")),
        Err(e) => log(&format!("overlay: not loaded ({e}); starting the game without it")),
    }

    let mut command = crate::runner::command_for(target);
    command.args(args);
    match command.spawn() {
        Ok(mut child) => {
            log(&format!("overlay: started {:?}, PID {}", target, child.id()));
            child.wait().ok().and_then(|s| s.code()).unwrap_or(0)
        }
        Err(e) => {
            log(&format!("overlay: could not start {:?}: {e}", target));
            1
        }
    }
}

fn load_overlay() -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH};

    let steam = crate::steam::SteamInstallation::detect().map_err(|e| e.to_string())?;
    let dll = steam.steam_path.join("GameOverlayRenderer64.dll");
    if !dll.exists() {
        return Err(format!("missing {}", dll.display()));
    }
    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
    if module.is_null() {
        return Err(format!("LoadLibrary failed for {}", dll.display()));
    }
    Ok(dll.display().to_string())
}
