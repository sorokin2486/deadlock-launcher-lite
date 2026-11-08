//! Locating the Deadlock install through Steam and launching the game.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DEADLOCK_APP_ID: &str = "1422450";
const GAME_FOLDER: &str = "Deadlock";

/// Paths derived from the game root (`.../steamapps/common/Deadlock`).
#[derive(Clone, Debug)]
pub struct GamePaths {
    pub root: PathBuf,
}

impl GamePaths {
    pub fn new(root: PathBuf) -> Option<Self> {
        let paths = Self { root };
        paths.gameinfo().is_file().then_some(paths)
    }

    pub fn citadel(&self) -> PathBuf {
        self.root.join("game").join("citadel")
    }

    pub fn gameinfo(&self) -> PathBuf {
        self.citadel().join("gameinfo.gi")
    }

    pub fn addons(&self) -> PathBuf {
        self.citadel().join("addons")
    }

    /// Disabled mods live next to `addons` so toggling is a cheap rename on the same disk.
    pub fn disabled(&self) -> PathBuf {
        self.citadel().join("addons_disabled")
    }
}

/// Finds the Deadlock install by reading Steam's library list.
pub fn detect_game() -> Option<GamePaths> {
    let steam = steam_path()?;
    library_folders(&steam)
        .into_iter()
        .chain(std::iter::once(steam))
        .map(|lib| lib.join("steamapps").join("common").join(GAME_FOLDER))
        .find_map(GamePaths::new)
}

fn steam_path() -> Option<PathBuf> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let key = hkcu.open_subkey("Software\\Valve\\Steam").ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

fn library_folders(steam: &Path) -> Vec<PathBuf> {
    let vdf = steam.join("steamapps").join("libraryfolders.vdf");
    fs::read_to_string(vdf)
        .map(|text| parse_library_paths(&text))
        .unwrap_or_default()
}

/// Extracts every `"path"  "..."` value from `libraryfolders.vdf`.
fn parse_library_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            match (quoted.next(), quoted.next()) {
                (Some(key), Some(value)) if key.eq_ignore_ascii_case("path") => {
                    Some(PathBuf::from(value.replace("\\\\", "\\")))
                }
                _ => None,
            }
        })
        .collect()
}

/// Launches the game through Steam so overlays and updates keep working.
pub fn launch_game() -> std::io::Result<()> {
    open_uri(&format!("steam://rungameid/{DEADLOCK_APP_ID}"))
}

/// Opens a folder or URL with the default Windows handler.
pub fn open_uri(target: &str) -> std::io::Result<()> {
    Command::new("explorer").arg(target).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_paths_with_escaped_backslashes() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{
			"1422450"		"38992760534"
		}
	}
	"1"
	{
		"path"		"X:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            parse_library_paths(vdf),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"X:\SteamLibrary")
            ]
        );
    }

    #[test]
    fn rejects_folder_without_gameinfo() {
        assert!(GamePaths::new(std::env::temp_dir().join("no-such-deadlock")).is_none());
    }
}
