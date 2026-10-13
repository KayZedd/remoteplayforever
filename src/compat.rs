use std::path::Path;
use crate::runner;

/// Automatically detect whether the target game has controller/gamepad support.
/// Decides the initial controller permission for Remote Play Together guests.
pub fn detect_controller_support(target_path: &Path) -> bool {
    let lower_name = target_path.to_string_lossy().to_lowercase();

    // 1. Known keyboard-only games
    let keyboard_games = [
        "the farmer was replaced",
        "thefarmerwasreplaced",
        "farmer",
        "duskers",
        "hacknet",
        "shenzhen i/o",
        "shenzhenio",
        "tis-100",
        "exapunks",
        "typoman",
        "epistory",
        "textorcist",
    ];
    for title in &keyboard_games {
        if lower_name.contains(title) {
            runner::log(&format!("compat: Game matches keyboard-only known title: '{}'", title));
            return false;
        }
    }

    // 2. Check game directory for options / controls files
    if let Some(parent) = target_path.parent() {
        let candidate_files = [
            parent.join("options.txt"),
            parent.join("config.ini"),
            parent.join("settings.ini"),
            parent.join("controls.txt"),
            parent.join("keybinds.txt"),
        ];
        for f in &candidate_files {
            if f.exists() {
                if let Ok(content) = std::fs::read_to_string(f) {
                    let lower = content.to_lowercase();
                    if (lower.contains("move up") || lower.contains("up =") || lower.contains("key_"))
                        && !lower.contains("controller")
                        && !lower.contains("gamepad")
                        && !lower.contains("joystick")
                    {
                        runner::log(&format!("compat: Controls in {:?} indicate keyboard-only game", f));
                        return false;
                    }
                }
            }
        }

        // Check Unity StreamingAssets for languages/options
        let file_stem = target_path.file_stem().unwrap_or_default().to_string_lossy();
        let streaming_assets = parent.join(format!("{}_Data", file_stem)).join("StreamingAssets");
        if streaming_assets.exists() {
            let en_options = streaming_assets.join("Languages").join("EN").join("Strings").join("options.txt");
            if en_options.exists() {
                if let Ok(content) = std::fs::read_to_string(&en_options) {
                    let lower = content.to_lowercase();
                    if lower.contains("@move up") && !lower.contains("@gamepad") && !lower.contains("@controller") {
                        runner::log("compat: Unity StreamingAssets options indicates keyboard-only game");
                        return false;
                    }
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_controller_support_farmer() {
        let p = Path::new("C:\\Gry\\The Farmer Was Replaced\\TheFarmerWasReplaced.exe");
        assert!(!detect_controller_support(p));
    }

    #[test]
    fn test_detect_controller_support_generic() {
        let p = Path::new("C:\\Steam\\steamapps\\common\\Duck Game\\DuckGame.exe");
        assert!(detect_controller_support(p));
    }
}
