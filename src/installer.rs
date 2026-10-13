use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::i18n::tr;

/// Create a desktop shortcut (.bat) that launches `game_path` with a Remote Play Together session.
pub fn create_desktop_shortcut(game_name: &str, game_path: &Path) -> Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    let desktop = desktop_dir().unwrap_or_else(|| PathBuf::from("."));
    let shortcut_file = desktop.join(shortcut_file_name(game_name));

    std::fs::write(&shortcut_file, shortcut_script(&current_exe, game_path)).with_context(|| {
        format!("{}: {}", tr("Could not write the shortcut", "Nie udało się zapisać skrótu"), shortcut_file.display())
    })?;

    Ok(shortcut_file)
}

/// File name for the shortcut; characters Windows does not allow in names are dropped.
fn shortcut_file_name(game_name: &str) -> String {
    let clean: String = game_name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .collect();
    format!("{} (Remote Play).bat", clean.trim())
}

fn shortcut_script(app_exe: &Path, game_path: &Path) -> String {
    format!("@echo off\r\nstart \"\" \"{}\" --run \"{}\"\r\n", app_exe.display(), game_path.display())
}

fn desktop_dir() -> Option<PathBuf> {
    let desktop = PathBuf::from(std::env::var("USERPROFILE").ok()?).join("Desktop");
    desktop.exists().then_some(desktop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_name_drops_characters_windows_rejects() {
        assert_eq!(shortcut_file_name("LEGO: Marvel? 2"), "LEGO Marvel 2 (Remote Play).bat");
        assert_eq!(shortcut_file_name("gra_1-x"), "gra_1-x (Remote Play).bat");
    }

    #[test]
    fn shortcut_script_quotes_both_paths() {
        let script = shortcut_script(Path::new("C:/Program Files/rpf.exe"), Path::new("D:/My Games/game.exe"));
        assert_eq!(script, "@echo off\r\nstart \"\" \"C:/Program Files/rpf.exe\" --run \"D:/My Games/game.exe\"\r\n");
    }
}
