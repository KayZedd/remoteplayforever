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
use std::process::Command;

fn game_command(target: &Path, args: &[String]) -> Command {
    let mut command = crate::runner::command_for(target);
    command.args(args);
    // SDL filters Steam's emulated controllers unless the game opts in. Steam
    // normally supplies this when it launches a game; loading the overlay here
    // does not. Set it only on the child so SDL games can see local Steam Input
    // controllers and Remote Play Together guests, without changing the GUI's
    // environment or disabling SDL's other device filters.
    command.env("SDL_GAMECONTROLLER_ALLOW_STEAM_VIRTUAL_GAMEPAD", "1");
    command
}

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
        Err(e) => log(&format!(
            "overlay: not loaded ({e}); starting the game without it"
        )),
    }

    let mut command = game_command(target, args);
    match command.spawn() {
        Ok(mut child) => {
            log(&format!(
                "overlay: started {:?}, PID {}",
                target,
                child.id()
            ));
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
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
    };

    let steam = crate::steam::SteamInstallation::detect().map_err(|e| e.to_string())?;
    let dll = steam.steam_path.join("GameOverlayRenderer64.dll");
    if !dll.exists() {
        return Err(format!("missing {}", dll.display()));
    }
    let wide: Vec<u16> = dll
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_WITH_ALTERED_SEARCH_PATH,
        )
    };
    if module.is_null() {
        return Err(format!("LoadLibrary failed for {}", dll.display()));
    }
    Ok(dll.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launched_child_can_read_the_sdl_steam_virtual_gamepad_opt_in() {
        let key = "SDL_GAMECONTROLLER_ALLOW_STEAM_VIRTUAL_GAMEPAD";
        let parent_value = std::env::var_os(key);
        let shell = std::env::var_os("COMSPEC").expect("Windows command interpreter");
        let output = game_command(
            Path::new(&shell),
            &["/d".into(), "/c".into(), format!("echo %{key}%")],
        )
        .output()
        .expect("launch child to inspect its environment");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1");
        assert_eq!(std::env::var_os(key), parent_value);
    }
}
