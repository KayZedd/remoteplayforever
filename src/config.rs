use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::i18n::Language;
use crate::session::GuestInputs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentGame {
    pub name: String,
    pub path: PathBuf,
    pub args: Vec<String>,
    pub last_played: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GuestControllerMode {
    /// Decide per game (see `compat::detect_controller_support`)
    #[default]
    Auto,
    Disabled,
    Enabled,
}

/// User settings. Every field has a default, so configs written by other versions still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: Language,
    pub guest_controller_mode: GuestControllerMode,
    /// Guests may use the keyboard / mouse as soon as they join
    pub guest_keyboard: bool,
    pub guest_mouse: bool,
    /// Put each new guest link on the clipboard
    pub copy_link_automatically: bool,
    pub recent_games: Vec<RecentGame>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: Language::default(),
            guest_controller_mode: GuestControllerMode::default(),
            guest_keyboard: true,
            guest_mouse: true,
            copy_link_automatically: true,
            recent_games: Vec::new(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        let base = dirs_fallback().unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("RemotePlayForever");
        let _ = std::fs::create_dir_all(&dir);
        dir.join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str::<Config>(&content) {
                    return cfg;
                }
            }
        }
        let default = Self::default();
        let _ = default.save();
        default
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = serde_json::to_string_pretty(self).context("Could not serialize the config")?;
        std::fs::write(&path, content).with_context(|| format!("Could not write {}", path.display()))?;
        Ok(())
    }

    pub fn add_recent(&mut self, path: &Path, args: &[String]) {
        let path_buf = path.to_path_buf();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Game")
            .to_string();

        let now = chrono_now();

        // Remove previous entry with same path
        self.recent_games.retain(|g| g.path != path_buf);

        self.recent_games.insert(0, RecentGame {
            name,
            path: path_buf,
            args: args.to_vec(),
            last_played: now,
        });

        // Limit to 15 recent games
        if self.recent_games.len() > 15 {
            self.recent_games.truncate(15);
        }

        let _ = self.save();
    }

    pub fn resolve_controller_allowed(&self, target_path: &Path) -> bool {
        match self.guest_controller_mode {
            GuestControllerMode::Auto => crate::compat::detect_controller_support(target_path),
            GuestControllerMode::Disabled => false,
            GuestControllerMode::Enabled => true,
        }
    }

    /// What guests may use when they join a session for `target_path`.
    pub fn guest_inputs(&self, target_path: &Path) -> GuestInputs {
        GuestInputs {
            keyboard: self.guest_keyboard,
            mouse: self.guest_mouse,
            controller: self.resolve_controller_allowed(target_path),
        }
    }
}

fn dirs_fallback() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(PathBuf::from(appdata));
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        return Some(PathBuf::from(userprofile).join(".config"));
    }
    None
}

fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_utc(secs)
}

/// Unix seconds -> "YYYY-MM-DD HH:MM" (UTC), without pulling in a date crate.
fn format_utc(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;

    // Civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_utc_known_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
        assert_eq!(format_utc(1_735_689_599), "2024-12-31 23:59");
        assert_eq!(format_utc(1_791_831_298), "2026-10-12 18:54");
    }

    #[test]
    fn defaults_are_english_with_keyboard_and_mouse_for_guests() {
        let cfg = Config::default();
        assert_eq!(cfg.language, Language::English);
        assert_eq!(cfg.guest_controller_mode, GuestControllerMode::Auto);
        assert!(cfg.guest_keyboard && cfg.guest_mouse && cfg.copy_link_automatically);
    }

    #[test]
    fn partial_config_loads_with_defaults_and_ignores_unknown_fields() {
        let json = r#"{"guest_controller_mode":"Disabled","guest_mouse":false,"something_else":1}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.guest_controller_mode, GuestControllerMode::Disabled);
        assert!(!cfg.guest_mouse);
        assert!(cfg.guest_keyboard);
        assert_eq!(cfg.language, Language::English);
        assert!(cfg.recent_games.is_empty());
    }

    #[test]
    fn guest_inputs_follow_settings() {
        let cfg = Config { guest_keyboard: false, guest_controller_mode: GuestControllerMode::Enabled, ..Config::default() };
        let inputs = cfg.guest_inputs(Path::new("game.exe"));
        assert_eq!(inputs, GuestInputs { keyboard: false, mouse: true, controller: true });
    }
}
