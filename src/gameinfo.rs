//! Patching `game/citadel/gameinfo.gi` so the engine mounts `citadel/addons`.
//!
//! The addons mount has to come before the base `Game citadel` entry so mods
//! override stock files. Because the engine takes the first `Game` path as the
//! default `Mod`/`Write` location, explicit `Mod citadel` and `Write citadel`
//! lines are added too, otherwise settings would be written into addons.
//!
//! Game updates overwrite this file, so the patch is re-applied on every launch.

use std::fs;
use std::io;
use std::path::Path;

const MARK_START: &str = "// deadlock-launcher start";
const MARK_END: &str = "// deadlock-launcher end";
const MOUNT_LINES: [&str; 3] = [
    "Game\t\t\t\tcitadel/addons",
    "Mod\t\t\t\t\tcitadel",
    "Write\t\t\t\tcitadel",
];

/// Ensures the patch is present. Returns `true` when the file was rewritten.
/// Before rewriting, the stock file is copied to `gameinfo.gi.backup`.
pub fn ensure_patched(gameinfo: &Path) -> io::Result<bool> {
    let text = fs::read_to_string(gameinfo)?;
    match patch(&text) {
        Ok(Some(patched)) => {
            // Refreshed on every re-patch, so it tracks the latest game update.
            fs::write(backup_path(gameinfo), &text)?;
            write_atomic(gameinfo, &patched)?;
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(reason) => Err(io::Error::new(io::ErrorKind::InvalidData, reason)),
    }
}

/// Returns the patched text, or `None` when the text already mounts addons.
pub fn patch(text: &str) -> Result<Option<String>, &'static str> {
    if text.contains(MARK_START) || mounts_addons(text) {
        return Ok(None);
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split(newline).collect();
    let anchor = find_base_game_line(&lines).ok_or("в gameinfo.gi нет строки Game citadel")?;

    let indent: String = lines[anchor]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let mut block = vec![format!("{indent}{MARK_START}")];
    block.extend(MOUNT_LINES.iter().map(|line| format!("{indent}{line}")));
    block.push(format!("{indent}{MARK_END}"));

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + block.len());
    out.extend(lines[..anchor].iter().map(|l| l.to_string()));
    out.extend(block);
    out.extend(lines[anchor..].iter().map(|l| l.to_string()));
    Ok(Some(out.join(newline)))
}

/// True when some uncommented `Game citadel/addons` entry already exists
/// (for example added by hand or by another mod manager).
fn mounts_addons(text: &str) -> bool {
    text.lines()
        .map(strip_comment)
        .any(|line| key_value(line) == Some(("game", "citadel/addons")))
}

/// Index of the first `Game citadel` entry inside the `SearchPaths` block.
fn find_base_game_line(lines: &[&str]) -> Option<usize> {
    let start = lines
        .iter()
        .position(|l| strip_comment(l).trim().eq_ignore_ascii_case("SearchPaths"))?;
    lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .take_while(|(_, l)| strip_comment(l).trim() != "}")
        .find(|(_, l)| key_value(strip_comment(l)) == Some(("game", "citadel")))
        .map(|(index, _)| index)
}

fn strip_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

/// Splits `Key   value` into a lowercase key and an unquoted value.
fn key_value(line: &str) -> Option<(&'static str, &str)> {
    let mut parts = line.split_whitespace();
    let key = parts.next()?;
    let value = parts.next()?.trim_matches('"');
    let key = match key.trim_matches('"').to_ascii_lowercase().as_str() {
        "game" => "game",
        "mod" => "mod",
        "write" => "write",
        _ => return None,
    };
    Some((key, value))
}

pub fn backup_path(gameinfo: &Path) -> std::path::PathBuf {
    gameinfo.with_extension("gi.backup")
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let tmp = path.with_extension("gi.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\t// comment Game citadel\n\t\t\tGame_LowViolence\tcitadel_lv\n\n\t\t\tGame\t\t\t\tcitadel\n\t\t\tGame\t\t\t\tcore\n\t\t}\n\t}\n}\n";

    #[test]
    fn inserts_mounts_before_base_game_entry() {
        let patched = patch(STOCK).unwrap().unwrap();
        let addons = patched.find("citadel/addons").unwrap();
        let base = patched.find("Game\t\t\t\tcitadel\n").unwrap();
        assert!(addons < base);
        assert!(patched.contains("\t\t\tMod\t\t\t\t\tcitadel"));
        assert!(patched.contains("\t\t\tWrite\t\t\t\tcitadel"));
    }

    #[test]
    fn patching_twice_changes_nothing() {
        let once = patch(STOCK).unwrap().unwrap();
        assert_eq!(patch(&once).unwrap(), None);
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let crlf = STOCK.replace('\n', "\r\n");
        let patched = patch(&crlf).unwrap().unwrap();
        assert!(!patched.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn respects_existing_manual_addons_mount() {
        let manual = STOCK.replace(
            "Game_LowViolence",
            "Game citadel/addons\n\t\t\tGame_LowViolence",
        );
        assert_eq!(patch(&manual).unwrap(), None);
    }

    #[test]
    fn commented_out_mount_does_not_count() {
        let commented = STOCK.replace(
            "Game_LowViolence",
            "// Game citadel/addons\n\t\t\tGame_LowViolence",
        );
        assert!(patch(&commented).unwrap().is_some());
    }

    #[test]
    fn ensure_patched_keeps_a_backup_of_the_stock_file() {
        let dir = std::env::temp_dir().join(format!("dl-gameinfo-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gameinfo.gi");
        fs::write(&file, STOCK).unwrap();

        assert!(ensure_patched(&file).unwrap());
        assert!(!ensure_patched(&file).unwrap());
        assert_eq!(fs::read_to_string(backup_path(&file)).unwrap(), STOCK);
        assert!(fs::read_to_string(&file).unwrap().contains(MARK_START));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fails_without_search_paths() {
        assert!(patch("\"GameInfo\"\n{\n}\n").is_err());
    }
}
