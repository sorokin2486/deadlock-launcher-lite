//! Installed mods: on-disk layout, load order and the small JSON state file.
//!
//! Enabled mods sit in `citadel/addons` as `pakNN_dir.vpk` (plus optional
//! `pakNN_000.vpk` parts); `pak01` has the highest priority, so the list order
//! is the load order. Disabled mods are parked in `citadel/addons_disabled`.
//! Every change is a rename on the same disk: no copies, no temp archives.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::steam::GamePaths;

/// The engine only mounts `pak01`..`pak99` from one folder.
pub const MAX_ENABLED: usize = 99;
const TMP_PREFIX: &str = ".dl_tmp_";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GbSource {
    pub mod_id: u64,
    pub file_id: u64,
    /// `_tsDateUpdated` of the mod at install time, used for update checks.
    pub updated_at: i64,
    /// Set for hero skins: used to warn about a second mod on the same hero.
    #[serde(default)]
    pub hero: Option<HeroRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HeroRef {
    /// GameBanana category id of the hero.
    pub id: u64,
    /// English hero name as GameBanana spells it; translated for display.
    pub name: String,
}

/// What to do when a new mod targets a hero that already has one.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictPolicy {
    #[default]
    Ask,
    Replace,
    Keep,
}

impl ConflictPolicy {
    pub const ALL: [ConflictPolicy; 3] = [Self::Ask, Self::Replace, Self::Keep];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Спрашивать",
            Self::Replace => "Заменять старый",
            Self::Keep => "Оставлять оба",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModEntry {
    pub uid: u64,
    pub name: String,
    pub enabled: bool,
    /// VPK part suffixes: `dir` for the main file, `000`, `001`... for data parts.
    pub parts: Vec<String>,
    /// Current location of every part, aligned with `parts`.
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub source: Option<GbSource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub game_root: Option<PathBuf>,
    #[serde(default)]
    pub next_uid: u64,
    #[serde(default)]
    pub mods: Vec<ModEntry>,
    #[serde(default = "default_true")]
    pub hide_adult: bool,
    #[serde(default)]
    pub hero_conflict: ConflictPolicy,
}

fn default_true() -> bool {
    true
}

impl Default for State {
    fn default() -> Self {
        Self {
            game_root: None,
            next_uid: 0,
            mods: Vec::new(),
            hide_adult: true,
            hero_conflict: ConflictPolicy::Ask,
        }
    }
}

impl State {
    pub fn file() -> PathBuf {
        let base = std::env::var_os("APPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
        // Folder name kept from the first version so existing installs keep their mod list.
        base.join("deadlock-launcher").join("state.json")
    }

    pub fn load() -> Self {
        fs::read(Self::file())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = Self::file();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(tmp, path)
    }

    pub fn take_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }

    pub fn enabled_count(&self) -> usize {
        self.mods.iter().filter(|m| m.enabled).count()
    }

    /// Uids of installed mods made for the given hero.
    pub fn mods_for_hero(&self, hero_id: u64) -> Vec<u64> {
        self.mods
            .iter()
            .filter(|m| m.hero().is_some_and(|h| h.id == hero_id))
            .map(|m| m.uid)
            .collect()
    }
}

impl ModEntry {
    pub fn hero(&self) -> Option<&HeroRef> {
        self.source.as_ref()?.hero.as_ref()
    }
}

/// Where a freshly extracted part should be written before layout runs.
pub fn staging_path(paths: &GamePaths, uid: u64, part: &str) -> PathBuf {
    paths.disabled().join(format!("m{uid}_{part}.vpk"))
}

/// Moves every file to where its mod's state and position say it belongs.
///
/// Renames go through temporary names first so swapping two slots never
/// collides. `files` is updated after each rename, so the state matches the
/// disk even if a rename fails halfway (e.g. the game holds a file open).
pub fn apply_layout(paths: &GamePaths, mods: &mut [ModEntry]) -> io::Result<()> {
    if mods.iter().filter(|m| m.enabled).count() > MAX_ENABLED {
        return Err(io::Error::other(format!(
            "включить можно не больше {MAX_ENABLED} модов"
        )));
    }
    fs::create_dir_all(paths.addons())?;
    fs::create_dir_all(paths.disabled())?;

    let targets = desired_paths(paths, mods);
    let mut pending = Vec::new();
    for (mod_index, mod_targets) in targets.iter().enumerate() {
        for (part_index, target) in mod_targets.iter().enumerate() {
            if mods[mod_index].files[part_index] != *target {
                pending.push((mod_index, part_index));
            }
        }
    }
    for &(mi, pi) in &pending {
        let entry = &mut mods[mi];
        let tmp = paths
            .disabled()
            .join(format!("{TMP_PREFIX}{}_{}.vpk", entry.uid, entry.parts[pi]));
        fs::rename(&entry.files[pi], &tmp)?;
        entry.files[pi] = tmp;
    }
    for &(mi, pi) in &pending {
        let target = targets[mi][pi].clone();
        fs::rename(&mods[mi].files[pi], &target)?;
        mods[mi].files[pi] = target;
    }
    Ok(())
}

fn desired_paths(paths: &GamePaths, mods: &[ModEntry]) -> Vec<Vec<PathBuf>> {
    let mut slot = 0;
    mods.iter()
        .map(|entry| {
            if entry.enabled {
                slot += 1;
            }
            entry
                .parts
                .iter()
                .map(|part| match entry.enabled {
                    true => paths.addons().join(format!("pak{slot:02}_{part}.vpk")),
                    false => staging_path(paths, entry.uid, part),
                })
                .collect()
        })
        .collect()
}

/// Syncs the state with the disk: forgets mods whose files were deleted by
/// hand and adopts VPKs that something else put into `addons`.
pub fn reconcile(paths: &GamePaths, state: &mut State) {
    // A crash between the two rename phases leaves `.dl_tmp_` names behind;
    // they are still tracked in `files`, so the next layout pass fixes them.
    state.mods.retain(|m| m.files.iter().all(|f| f.is_file()));

    let known: HashSet<PathBuf> = state.mods.iter().flat_map(|m| m.files.clone()).collect();
    for (name, parts, files) in foreign_packages(&paths.addons(), &known) {
        let uid = state.take_uid();
        state.mods.push(ModEntry {
            uid,
            name,
            enabled: true,
            parts,
            files,
            source: None,
        });
    }
}

/// Groups untracked `*_dir.vpk` files in `addons` with their numbered parts.
fn foreign_packages(
    addons: &Path,
    known: &HashSet<PathBuf>,
) -> Vec<(String, Vec<String>, Vec<PathBuf>)> {
    let Ok(read) = fs::read_dir(addons) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| !known.contains(p) && has_vpk_ext(p))
        .collect();
    files.sort();

    let mut packages = Vec::new();
    for dir_file in files.iter().filter(|p| stem_of(p).ends_with("_dir")) {
        let stem = stem_of(dir_file);
        let base = &stem[..stem.len() - "_dir".len()];
        let mut parts = vec!["dir".to_string()];
        let mut paths = vec![dir_file.clone()];
        for other in &files {
            let other_stem = stem_of(other);
            if let Some(num) = other_stem
                .strip_prefix(base)
                .and_then(|r| r.strip_prefix('_'))
                && num.len() == 3
                && num.bytes().all(|b| b.is_ascii_digit())
            {
                parts.push(num.to_string());
                paths.push(other.clone());
            }
        }
        packages.push((format!("{stem}.vpk — установлен вручную"), parts, paths));
    }
    packages
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn has_vpk_ext(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vpk"))
}

/// Takes the mods listed in `replaces` out of the list and puts `added` where
/// the first of them stood (or at the end). Returns the removed entries so the
/// caller can delete their files.
pub fn place_new(
    mods: &mut Vec<ModEntry>,
    replaces: &[u64],
    added: Vec<ModEntry>,
) -> Vec<ModEntry> {
    let first = mods.iter().position(|m| replaces.contains(&m.uid));
    let insert_at = first.unwrap_or(mods.len());
    let (removed, kept): (Vec<_>, Vec<_>) = std::mem::take(mods)
        .into_iter()
        .enumerate()
        .partition(|(_, m)| replaces.contains(&m.uid));
    let shift = removed.iter().filter(|(i, _)| *i < insert_at).count();
    *mods = kept.into_iter().map(|(_, m)| m).collect();
    let at = insert_at - shift;
    mods.splice(at..at, added);
    removed.into_iter().map(|(_, m)| m).collect()
}

/// Deletes a mod's files. Missing files are fine: the goal is that they are gone.
pub fn delete_files(entry: &ModEntry) -> io::Result<()> {
    for file in &entry.files {
        match fs::remove_file(file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> (Self, GamePaths) {
            let root = std::env::temp_dir().join(format!("dl-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let citadel = root.join("game").join("citadel");
            fs::create_dir_all(&citadel).unwrap();
            fs::write(citadel.join("gameinfo.gi"), "x").unwrap();
            let paths = GamePaths::new(root.clone()).unwrap();
            fs::create_dir_all(paths.disabled()).unwrap();
            (Self(root), paths)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn staged(paths: &GamePaths, uid: u64, content: &str) -> ModEntry {
        let file = staging_path(paths, uid, "dir");
        fs::write(&file, content).unwrap();
        ModEntry {
            uid,
            name: content.into(),
            enabled: true,
            parts: vec!["dir".into()],
            files: vec![file],
            source: None,
        }
    }

    fn read(paths: &GamePaths, name: &str) -> String {
        fs::read_to_string(paths.addons().join(name)).unwrap()
    }

    #[test]
    fn enabled_mods_get_consecutive_slots_in_list_order() {
        let (_s, paths) = Sandbox::new("slots");
        let mut mods = vec![staged(&paths, 1, "a"), staged(&paths, 2, "b")];
        apply_layout(&paths, &mut mods).unwrap();
        assert_eq!(read(&paths, "pak01_dir.vpk"), "a");
        assert_eq!(read(&paths, "pak02_dir.vpk"), "b");
    }

    #[test]
    fn swapping_order_swaps_slots_without_collisions() {
        let (_s, paths) = Sandbox::new("swap");
        let mut mods = vec![staged(&paths, 1, "a"), staged(&paths, 2, "b")];
        apply_layout(&paths, &mut mods).unwrap();
        mods.swap(0, 1);
        apply_layout(&paths, &mut mods).unwrap();
        assert_eq!(read(&paths, "pak01_dir.vpk"), "b");
        assert_eq!(read(&paths, "pak02_dir.vpk"), "a");
    }

    #[test]
    fn disabling_moves_out_and_closes_the_gap() {
        let (_s, paths) = Sandbox::new("disable");
        let mut mods = vec![staged(&paths, 1, "a"), staged(&paths, 2, "b")];
        apply_layout(&paths, &mut mods).unwrap();
        mods[0].enabled = false;
        apply_layout(&paths, &mut mods).unwrap();
        assert_eq!(read(&paths, "pak01_dir.vpk"), "b");
        assert!(!paths.addons().join("pak02_dir.vpk").exists());
        assert!(staging_path(&paths, 1, "dir").is_file());
    }

    #[test]
    fn refuses_more_than_99_enabled() {
        let (_s, paths) = Sandbox::new("limit");
        let mut mods: Vec<ModEntry> = (1..=100).map(|uid| staged(&paths, uid, "x")).collect();
        assert!(apply_layout(&paths, &mut mods).is_err());
    }

    fn entry(uid: u64) -> ModEntry {
        ModEntry {
            uid,
            name: uid.to_string(),
            enabled: true,
            parts: Vec::new(),
            files: Vec::new(),
            source: None,
        }
    }

    fn uids(mods: &[ModEntry]) -> Vec<u64> {
        mods.iter().map(|m| m.uid).collect()
    }

    #[test]
    fn new_mod_takes_place_of_first_replaced() {
        let mut mods: Vec<ModEntry> = [1, 2, 3, 4].map(entry).into();
        let removed = place_new(&mut mods, &[3, 2], vec![entry(9)]);
        assert_eq!(uids(&mods), [1, 9, 4]);
        assert_eq!(uids(&removed), [2, 3]);
    }

    #[test]
    fn new_mod_goes_last_when_nothing_is_replaced() {
        let mut mods: Vec<ModEntry> = [1, 2].map(entry).into();
        assert!(place_new(&mut mods, &[], vec![entry(9), entry(8)]).is_empty());
        assert_eq!(uids(&mods), [1, 2, 9, 8]);
    }

    #[test]
    fn reconcile_adopts_foreign_vpks_and_drops_deleted_mods() {
        let (_s, paths) = Sandbox::new("reconcile");
        fs::create_dir_all(paths.addons()).unwrap();
        fs::write(paths.addons().join("pak05_dir.vpk"), "f").unwrap();
        fs::write(paths.addons().join("pak05_000.vpk"), "f0").unwrap();
        let mut state = State::default();
        let mut gone = staged(&paths, 1, "gone");
        fs::remove_file(&gone.files[0]).unwrap();
        gone.enabled = false;
        state.mods.push(gone);

        reconcile(&paths, &mut state);

        assert_eq!(state.mods.len(), 1);
        assert_eq!(state.mods[0].parts, vec!["dir", "000"]);
        apply_layout(&paths, &mut state.mods).unwrap();
        assert_eq!(read(&paths, "pak01_dir.vpk"), "f");
        assert_eq!(read(&paths, "pak01_000.vpk"), "f0");
    }
}
