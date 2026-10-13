use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use winreg::enums::*;
use winreg::RegKey;

use crate::i18n::tr;

#[derive(Debug, Clone)]
pub struct SteamApp {
    pub appid: u32,
    pub name: String,
    pub install_dir: PathBuf,
    pub executable: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SteamInstallation {
    pub steam_path: PathBuf,
    pub libraries: Vec<PathBuf>,
}

impl SteamInstallation {
    /// Detect Steam installation path from Windows Registry or common locations
    pub fn detect() -> Result<Self> {
        let steam_path = Self::find_steam_path()?;
        let libraries = Self::find_libraries(&steam_path);
        Ok(Self {
            steam_path,
            libraries,
        })
    }

    fn find_steam_path() -> Result<PathBuf> {
        // Try reading HKCU\Software\Valve\Steam\SteamPath
        if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam") {
            if let Ok(path_str) = hkcu.get_value::<String, _>("SteamPath") {
                let path = PathBuf::from(path_str.replace('/', "\\"));
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        // Try reading HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath
        if let Ok(hklm) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam") {
            if let Ok(path_str) = hklm.get_value::<String, _>("InstallPath") {
                let path = PathBuf::from(path_str);
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        // Common fallback paths
        let fallbacks = [
            PathBuf::from(r"C:\Program Files (x86)\Steam"),
            PathBuf::from(r"C:\Program Files\Steam"),
        ];

        for fb in &fallbacks {
            if fb.exists() {
                return Ok(fb.clone());
            }
        }

        anyhow::bail!("{}", tr("Steam installation not found in the registry or the standard folders.", "Nie znaleziono instalacji Steam w rejestrze ani standardowych folderach."))
    }

    fn find_libraries(steam_path: &Path) -> Vec<PathBuf> {
        let mut libs: Vec<PathBuf> = Vec::new();
        let norm_steam = PathBuf::from(steam_path.to_string_lossy().replace('/', "\\"));
        libs.push(norm_steam);

        let vdf_path = steam_path.join("steamapps").join("libraryfolders.vdf");
        if let Ok(content) = std::fs::read_to_string(&vdf_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("\"path\"") {
                    let parts: Vec<&str> = trimmed.split('"').collect();
                    if parts.len() >= 4 {
                        let norm = PathBuf::from(parts[3].replace(r"\\", r"\").replace('/', "\\"));
                        let norm_lower = norm.to_string_lossy().to_lowercase();
                        if norm.exists() && !libs.iter().any(|p| p.to_string_lossy().to_lowercase() == norm_lower) {
                            libs.push(norm);
                        }
                    }
                }
            }
        }

        libs
    }

    /// Scan all libraries for installed Steam games
    pub fn get_installed_apps(&self) -> Vec<SteamApp> {
        let mut apps = Vec::new();

        for lib in &self.libraries {
            let steamapps_dir = lib.join("steamapps");
            if !steamapps_dir.exists() {
                continue;
            }

            if let Ok(entries) = std::fs::read_dir(&steamapps_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
                        if file_name.starts_with("appmanifest_") && file_name.ends_with(".acf") {
                            if let Some(app) = parse_appmanifest(&path, lib) {
                                if !apps.iter().any(|a: &SteamApp| a.appid == app.appid) {
                                    apps.push(app);
                                }
                            }
                        }
                    }
                }
            }
        }

        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        apps
    }
}

/// Parse a single appmanifest_*.acf file
fn parse_appmanifest(manifest_path: &Path, library_root: &Path) -> Option<SteamApp> {
    let content = std::fs::read_to_string(manifest_path).ok()?;

    let mut appid = None;
    let mut name = None;
    let mut installdir = None;

    for line in content.lines() {
        let trimmed = line.trim();
        let tokens = extract_quoted_tokens(trimmed);
        if tokens.len() >= 2 {
            match tokens[0].to_lowercase().as_str() {
                "appid" => {
                    if let Ok(id) = tokens[1].parse::<u32>() {
                        appid = Some(id);
                    }
                }
                "name" => {
                    name = Some(tokens[1].clone());
                }
                "installdir" => {
                    installdir = Some(tokens[1].clone());
                }
                _ => {}
            }
        }
    }

    let appid = appid?;
    let name = name.unwrap_or_else(|| format!("App {appid}"));
    let installdir = installdir.unwrap_or_default();

    let full_install_dir = library_root.join("steamapps").join("common").join(&installdir);
    let executable = find_game_executable(&full_install_dir, &installdir);

    Some(SteamApp {
        appid,
        name,
        install_dir: full_install_dir,
        executable,
    })
}

/// Extract quoted strings from a line (handles escaped quotes basic)
fn extract_quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;

    for c in line.chars() {
        if escaped {
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            if in_quote {
                tokens.push(current.clone());
                current.clear();
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            current.push(c);
        }
    }

    tokens
}

/// Find the main executable in a game's installation directory
pub fn find_game_executable(install_dir: &Path, folder_name: &str) -> Option<PathBuf> {
    if !install_dir.exists() {
        return None;
    }

    let mut candidate_exes = Vec::new();

    // Check directly in root directory
    if let Ok(entries) = std::fs::read_dir(install_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_potential_game_exe(&path) {
                candidate_exes.push(path);
            }
        }
    }

    // Check in common subdirectories (bin, bin/x64, x64, etc.)
    let subdirs = ["bin", "bin/x64", "bin/win64", "x64", "Win64", "build"];
    for sub in &subdirs {
        let sub_path = install_dir.join(sub);
        if sub_path.exists() {
            if let Ok(entries) = std::fs::read_dir(&sub_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if is_potential_game_exe(&path) {
                        candidate_exes.push(path);
                    }
                }
            }
        }
    }

    if candidate_exes.is_empty() {
        return None;
    }

    // Score candidates: prefer exe matching folder_name or game name
    let lower_folder = folder_name.to_lowercase().replace(' ', "");
    for exe in &candidate_exes {
        if let Some(stem) = exe.file_stem().and_then(|s| s.to_str()) {
            let lower_stem = stem.to_lowercase().replace(' ', "");
            if lower_stem == lower_folder || lower_stem.contains(&lower_folder) || lower_folder.contains(&lower_stem) {
                return Some(exe.clone());
            }
        }
    }

    // Default to the first candidate (or biggest by file size)
    candidate_exes.sort_by(|a, b| {
        let size_a = std::fs::metadata(a).map(|m| m.len()).unwrap_or(0);
        let size_b = std::fs::metadata(b).map(|m| m.len()).unwrap_or(0);
        size_b.cmp(&size_a)
    });

    candidate_exes.into_iter().next()
}

fn is_potential_game_exe(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("exe") {
        return false;
    }

    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    // Ignore helper / updater / crash tools
    let ignored = [
        "crash", "report", "unins", "setup", "helper", "update", "redist", "vcredist",
        "dxsetup", "dotnet", "eac", "anticheat", "launcher_prereq", "install", "cleanup",
        "cef", "crashpad", "unitycrashhandler",
    ];

    for ign in &ignored {
        if name.contains(ign) {
            return false;
        }
    }

    true
}

fn steam_registry_dword(subkey: &str, value: &str) -> u32 {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(subkey)
        .and_then(|k| k.get_value::<u32, _>(value))
        .unwrap_or(0)
}

/// AppID of the game the Steam client currently tracks as running (0 if none).
pub fn running_appid() -> u32 {
    steam_registry_dword(r"Software\Valve\Steam", "RunningAppID")
}

/// Account id of the user signed in to the running Steam client (0 if Steam is not running / signed out).
pub fn active_user() -> u32 {
    steam_registry_dword(r"Software\Valve\Steam\ActiveProcess", "ActiveUser")
}

/// Launch a Steam application under its own AppID via steam.exe or the URL protocol
pub fn launch_steam_app(appid: u32, args: &[String]) -> Result<()> {
    let url = format!("steam://rungameid/{appid}");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // 1. Try invoking steam.exe -applaunch directly if steam path is known
        if let Ok(steam) = SteamInstallation::detect() {
            let steam_exe = steam.steam_path.join("steam.exe");
            if steam_exe.exists() {
                if let Ok(_) = std::process::Command::new(&steam_exe)
                    .args(["-applaunch", &appid.to_string()])
                    .args(args)
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                {
                    return Ok(());
                }
            }
        }

        // 2. Fallback to the URL protocol (cannot carry arguments)
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .with_context(|| format!("{} {appid}", tr("Could not open the Steam URL for AppID", "Nie udało się uruchomić protokołu Steam dla AppID")))?;
    }

    #[cfg(not(windows))]
    {
        let _ = args;
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .with_context(|| format!("{} {appid}", tr("Could not open the Steam URL for AppID", "Nie udało się uruchomić protokołu Steam dla AppID")))?;
    }

    Ok(())
}

/// Detect if a target executable belongs to an installed Steam game and return its genuine AppID.
pub fn detect_target_steam_appid(target_path: &Path) -> Option<u32> {
    let target_dir = target_path.parent().unwrap_or_else(|| Path::new("."));

    // 1. Walk up directory tree to find steamapps/appmanifest_*.acf
    let mut current = target_dir;
    while let Some(parent) = current.parent() {
        if let Some(name) = parent.file_name().and_then(|s| s.to_str()) {
            if name.eq_ignore_ascii_case("common") {
                if let Some(steamapps) = parent.parent() {
                    if let Some(sa_name) = steamapps.file_name().and_then(|s| s.to_str()) {
                        if sa_name.eq_ignore_ascii_case("steamapps") {
                            let folder_name = current.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            if let Ok(entries) = std::fs::read_dir(steamapps) {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                                        if fname.starts_with("appmanifest_") && fname.ends_with(".acf") {
                                            if let Ok(content) = std::fs::read_to_string(&path) {
                                                let low_content = content.to_lowercase();
                                                let target_needle = format!("\"{}\"", folder_name.to_lowercase());
                                                if low_content.contains(&target_needle) {
                                                    if let Some(id_str) = fname.strip_prefix("appmanifest_").and_then(|s| s.strip_suffix(".acf")) {
                                                        if let Ok(id) = id_str.parse::<u32>() {
                                                            return Some(id);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        current = parent;
    }

    // 2. Fallback: match against all detected Steam libraries
    if let Ok(steam_inst) = SteamInstallation::detect() {
        let apps = steam_inst.get_installed_apps();
        let target_norm = target_path.to_string_lossy().to_lowercase().replace('/', "\\");
        for app in apps {
            let install_norm = app.install_dir.to_string_lossy().to_lowercase().replace('/', "\\");
            if target_norm.starts_with(&install_norm) {
                return Some(app.appid);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_quoted_tokens() {
        let line = r#"    "appid"    "312530"    "#;
        let tokens = extract_quoted_tokens(line);
        assert_eq!(tokens, vec!["appid", "312530"]);

        let line_escaped = r#"    "path"    "C:\\Program Files (x86)\\Steam"    "#;
        let tokens_escaped = extract_quoted_tokens(line_escaped);
        assert_eq!(tokens_escaped, vec!["path", "C:\\Program Files (x86)\\Steam"]);
    }

    #[test]
    fn test_detect_and_installed_apps() {
        if let Ok(steam) = SteamInstallation::detect() {
            let apps = steam.get_installed_apps();
            println!("Found {} apps", apps.len());
        }
    }

    #[test]
    fn test_detect_target_steam_appid_duck_game() {
        let p = Path::new(r"C:\Program Files (x86)\Steam\steamapps\common\Duck Game\DuckGame.exe");
        if p.exists() {
            let id = detect_target_steam_appid(p);
            assert_eq!(id, Some(312530));
        }
    }
}
