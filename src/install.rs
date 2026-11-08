//! Install flow: file choice, the second-mod-on-a-hero question, download,
//! variant choice, extraction and placing the result in the load order.

use crate::app::{App, Choice, ConflictPrompt, DetailsView};
use crate::gamebanana::{GbFile, ModDetails};
use crate::library::{self, ConflictPolicy, GbSource, MAX_ENABLED, ModEntry};
use crate::tasks::{Extracted, InstallTarget, Prepared};

impl App {
    /// "Install" on a catalog card: fetch the file list and install at once
    /// when there is a single file, otherwise open the dialog to pick one.
    pub fn quick_install(&mut self, id: u64) {
        self.pending_quick = Some(id);
        self.busy = Some("Получение списка файлов…".into());
        self.download_total = 0;
        self.tasks.load_details(id);
    }

    pub(crate) fn quick_install_ready(&mut self, details: ModDetails) {
        match details.files.as_slice() {
            [file] => {
                let file = file.clone();
                self.begin_install(file, details);
            }
            _ => {
                self.details = Some(DetailsView {
                    id: details.id,
                    data: Some(details),
                    replaces: None,
                });
            }
        }
    }

    /// Starts a fresh install, first asking what to do when the hero already has a mod.
    pub fn begin_install(&mut self, file: GbFile, details: ModDetails) {
        let hero = self.hero_for(&details);
        let existing = hero
            .as_ref()
            .map(|h| self.state.mods_for_hero(h.id))
            .unwrap_or_default();
        let Some(hero) = hero.filter(|_| !existing.is_empty()) else {
            self.start_download(file, &details, Vec::new(), false);
            return;
        };
        match self.state.hero_conflict {
            ConflictPolicy::Ask => {
                self.conflict = Some(ConflictPrompt {
                    file,
                    details,
                    hero: self.hero_names.get(&hero.name).to_string(),
                    existing,
                    remember: false,
                });
            }
            ConflictPolicy::Replace => self.start_download(file, &details, existing, false),
            ConflictPolicy::Keep => self.start_download(file, &details, Vec::new(), false),
        }
    }

    /// Installs a newer file of an installed mod in its place, keeping its on/off state.
    pub fn update_mod(&mut self, file: GbFile, details: &ModDetails, uid: u64) {
        self.start_download(file, details, vec![uid], true);
    }

    /// `None` cancels; `Replace` or `Keep` continues the install.
    pub fn resolve_conflict(&mut self, prompt: ConflictPrompt, decision: Option<ConflictPolicy>) {
        let Some(policy) = decision else {
            return;
        };
        if prompt.remember {
            self.state.hero_conflict = policy;
            if let Err(e) = self.state.save() {
                self.error(format!("Не удалось сохранить настройки: {e}"));
            }
        }
        let replaces = match policy {
            ConflictPolicy::Replace => prompt.existing,
            ConflictPolicy::Keep | ConflictPolicy::Ask => Vec::new(),
        };
        self.start_download(prompt.file, &prompt.details, replaces, false);
    }

    fn start_download(
        &mut self,
        file: GbFile,
        details: &ModDetails,
        replaces: Vec<u64>,
        keep_state: bool,
    ) {
        self.busy = Some(format!("Скачивание: {}", file.name));
        self.download_total = file.size;
        let target = InstallTarget {
            name: details.name.clone(),
            source: Some(GbSource {
                mod_id: details.id,
                file_id: file.id,
                updated_at: details.updated_at,
                hero: self.hero_for(details),
            }),
            replaces,
            keep_state,
        };
        self.tasks.download(file, target);
    }

    pub fn install_dropped(&mut self, path: std::path::PathBuf) {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.busy = Some(format!("Открытие: {name}"));
        self.download_total = 0;
        let target = InstallTarget {
            name,
            source: None,
            replaces: Vec::new(),
            keep_state: false,
        };
        self.tasks.open_local(path, target);
    }

    pub(crate) fn on_prepared(&mut self, prepared: Prepared) {
        // Parts of one archive are usually meant to go in together (a skin plus
        // its soul jar); for colour variants the user unticks the extras.
        let selected = vec![true; prepared.packages.len()];
        self.download_total = 0;
        let choice = Choice { prepared, selected };
        if choice.prepared.packages.len() == 1 {
            self.extract_selected(choice);
        } else {
            self.busy = Some("Выберите части мода".into());
            self.choice = Some(choice);
        }
    }

    pub fn extract_selected(&mut self, choice: Choice) {
        let Some(paths) = self.game.clone() else {
            return;
        };
        let Choice {
            mut prepared,
            selected,
        } = choice;
        let mut keep = selected.into_iter();
        prepared.packages.retain(|_| keep.next().unwrap_or(false));
        if prepared.packages.is_empty() {
            self.busy = None;
            return;
        }
        let uids = prepared
            .packages
            .iter()
            .map(|_| self.state.take_uid())
            .collect();
        self.busy = Some("Распаковка…".into());
        self.tasks.extract(prepared, uids, move |uid, part| {
            library::staging_path(&paths, uid, part)
        });
    }

    pub(crate) fn on_extracted(&mut self, extracted: Extracted) {
        self.busy = None;
        let Some(paths) = self.game.clone() else {
            return;
        };
        let target = extracted.target;
        let several = extracted.items.len() > 1;
        let replaced: Vec<&ModEntry> = self
            .state
            .mods
            .iter()
            .filter(|m| target.replaces.contains(&m.uid))
            .collect();
        let enabled = !target.keep_state || replaced.first().is_none_or(|m| m.enabled);
        let freed = replaced.iter().filter(|m| m.enabled).count();

        let mut added: Vec<ModEntry> = extracted
            .items
            .into_iter()
            .map(|(uid, label, parts)| ModEntry {
                uid,
                name: match several {
                    true => format!("{} — {}", target.name, variant_name(&label)),
                    false => target.name.clone(),
                },
                enabled,
                files: parts
                    .iter()
                    .map(|p| library::staging_path(&paths, uid, p))
                    .collect(),
                parts,
                source: target.source.clone(),
            })
            .collect();

        let room = MAX_ENABLED.saturating_sub(self.state.enabled_count()) + freed;
        let over_limit = added.iter().filter(|m| m.enabled).count() > room;
        if over_limit {
            added.iter_mut().for_each(|m| m.enabled = false);
        }

        let names: Vec<String> = added.iter().map(|m| m.name.clone()).collect();
        let removed = library::place_new(&mut self.state.mods, &target.replaces, added);
        for old in &removed {
            self.updates.remove(&old.uid);
            if let Err(e) = library::delete_files(old) {
                self.error(format!("Не удалось удалить «{}»: {e}", old.name));
            }
        }
        self.commit();
        if self.status.as_ref().is_none_or(|s| !s.error) {
            let suffix = match over_limit {
                true => " — выключен: включено уже 99 модов",
                false => "",
            };
            self.info(format!("Установлено: {}{suffix}", names.join(", ")));
        }
    }
}

/// `Red/pak01_dir.vpk` gives `Red`; `cool_mod.vpk` gives `cool_mod`.
fn variant_name(label: &str) -> String {
    let path = std::path::Path::new(label);
    path.parent()
        .and_then(|p| p.file_name())
        .or_else(|| path.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::variant_name;

    #[test]
    fn variant_names_come_from_folder_or_file() {
        assert_eq!(variant_name("Red/pak01_dir.vpk"), "Red");
        assert_eq!(variant_name("a/Blue/pak01_dir.vpk"), "Blue");
        assert_eq!(variant_name("cool_mod.vpk"), "cool_mod");
    }
}
