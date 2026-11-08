//! Application state, catalog navigation and background message handling.
//! The install flow lives in `install.rs`; drawing in the `ui_*` modules.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, TextureHandle, TextureOptions};

use crate::gamebanana::{self, Category, GbFile, ModCard, ModDetails, Sort};
use crate::gameinfo;
use crate::library::{self, HeroRef, State};
use crate::locale::{self, HeroNames};
use crate::steam::{self, GamePaths};
use crate::tasks::{Catalog, Msg, Prepared, Query, Tasks};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Catalog,
    Library,
}

/// What the catalog list shows: everything, or one section or hero.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    All,
    Category(u64),
}

pub struct DetailsView {
    pub id: u64,
    pub data: Option<ModDetails>,
    /// Set when the dialog was opened to update an installed mod.
    pub replaces: Option<u64>,
}

pub struct Choice {
    pub prepared: Prepared,
    pub selected: Vec<bool>,
}

/// A new hero mod waits here while the user decides what to do with the old ones.
pub struct ConflictPrompt {
    pub file: GbFile,
    pub details: ModDetails,
    pub hero: String,
    pub existing: Vec<u64>,
    pub remember: bool,
}

pub struct Status {
    pub text: String,
    pub error: bool,
}

pub struct App {
    pub state: State,
    pub game: Option<GamePaths>,
    pub tasks: Tasks,
    pub tab: Tab,
    pub hero_names: HeroNames,

    pub catalog: Option<Catalog>,
    pub catalog_error: Option<String>,
    pub section: Section,
    pub sort: Sort,
    pub search_input: String,
    pub active_search: Option<String>,
    pub page: usize,
    pub generation: u64,
    pub cards: Vec<ModCard>,
    pub has_more: bool,
    pub total: u64,
    pub loading_page: bool,
    pub page_error: Option<String>,
    pub textures: HashMap<String, TextureHandle>,
    pub requested: HashSet<String>,

    pub details: Option<DetailsView>,
    /// Mod whose file list is being fetched for a one-click install from a card.
    pub pending_quick: Option<u64>,
    pub conflict: Option<ConflictPrompt>,
    pub choice: Option<Choice>,
    pub busy: Option<String>,
    pub download_total: u64,
    pub updates: HashSet<u64>,
    pub checking_updates: bool,
    pub status: Option<Status>,
    pub path_input: String,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let state = State::load();
        let game = state
            .game_root
            .clone()
            .and_then(GamePaths::new)
            .or_else(steam::detect_game);
        let mut app = Self {
            path_input: game
                .as_ref()
                .map(|g| g.root.display().to_string())
                .unwrap_or_default(),
            state,
            game: None,
            tasks: Tasks::new(ctx),
            tab: Tab::Catalog,
            hero_names: HeroNames::default(),
            catalog: None,
            catalog_error: None,
            section: Section::All,
            sort: Sort::Popular,
            search_input: String::new(),
            active_search: None,
            page: 1,
            generation: 0,
            cards: Vec::new(),
            has_more: false,
            total: 0,
            loading_page: false,
            page_error: None,
            textures: HashMap::new(),
            requested: HashSet::new(),
            details: None,
            pending_quick: None,
            conflict: None,
            choice: None,
            busy: None,
            download_total: 0,
            updates: HashSet::new(),
            checking_updates: false,
            status: None,
        };
        app.set_game(game);
        app.tasks.load_catalog();
        app.reload_page();
        app
    }

    /// Adopts a game folder: syncs the mod list with the disk and patches gameinfo.
    pub fn set_game(&mut self, game: Option<GamePaths>) {
        self.game = game;
        let Some(paths) = self.game.clone() else {
            return;
        };
        self.hero_names = HeroNames::load(&paths);
        self.state.game_root = Some(paths.root.clone());
        library::reconcile(&paths, &mut self.state);
        self.commit();
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: true,
        });
    }

    /// Writes the mod layout to disk, saves state and keeps gameinfo patched.
    pub fn commit(&mut self) {
        let Some(paths) = self.game.clone() else {
            return;
        };
        if let Err(e) = library::apply_layout(&paths, &mut self.state.mods) {
            self.error(format!(
                "Не удалось переместить файлы модов: {e}. Закройте игру и повторите."
            ));
        }
        if self.state.enabled_count() > 0
            && let Err(e) = gameinfo::ensure_patched(&paths.gameinfo())
        {
            self.error(format!("Не удалось изменить gameinfo.gi: {e}"));
        }
        if let Err(e) = self.state.save() {
            self.error(format!("Не удалось сохранить настройки: {e}"));
        }
    }

    pub fn select_section(&mut self, section: Section) {
        self.section = section;
        self.search_input.clear();
        self.active_search = None;
        self.page = 1;
        self.reload_page();
    }

    pub fn search(&mut self, text: &str) {
        self.active_search = Some(text.to_string());
        self.page = 1;
        self.reload_page();
    }

    pub fn reload_page(&mut self) {
        self.generation += 1;
        self.tasks
            .image_generation
            .store(self.generation, std::sync::atomic::Ordering::Relaxed);
        self.loading_page = true;
        self.page_error = None;
        let query = match &self.active_search {
            Some(text) => Query::Search(text.clone()),
            None => Query::Browse {
                sort: self.sort,
                category: match self.section {
                    Section::All => None,
                    Section::Category(id) => Some(id),
                },
            },
        };
        self.tasks.load_page(query, self.page, self.generation);
    }

    pub fn page_count(&self) -> u64 {
        let per_page = match self.active_search {
            Some(_) => gamebanana::SEARCH_PAGE_SIZE,
            None => gamebanana::PAGE_SIZE,
        } as u64;
        self.total.div_ceil(per_page).max(1)
    }

    /// Heading above the mod list.
    pub fn section_title(&self) -> String {
        if let Some(text) = &self.active_search {
            return format!("Поиск: «{text}»");
        }
        let Section::Category(id) = self.section else {
            return "Все моды".into();
        };
        let Some(catalog) = &self.catalog else {
            return String::new();
        };
        if let Some(hero) = catalog.heroes.iter().find(|h| h.id == id) {
            return self.hero_names.get(&hero.name).to_string();
        }
        catalog
            .sections
            .iter()
            .find(|s| s.id == id)
            .map(|s| locale::section(&s.name).to_string())
            .unwrap_or_default()
    }

    /// The hero a mod is for: its category or that category's parent
    /// (heroes have their own subcategories on GameBanana).
    pub fn hero_for(&self, details: &ModDetails) -> Option<HeroRef> {
        let ids = [details.category.id, details.super_category.id];
        let hero = self
            .catalog
            .as_ref()?
            .heroes
            .iter()
            .find(|h| ids.contains(&h.id))?;
        Some(HeroRef {
            id: hero.id,
            name: hero.name.clone(),
        })
    }

    pub fn open_details(&mut self, id: u64, replaces: Option<u64>) {
        self.details = Some(DetailsView {
            id,
            data: None,
            replaces,
        });
        self.tasks.load_details(id);
    }

    pub fn close_details(&mut self) {
        if let Some(url) = self
            .details
            .take()
            .and_then(|d| d.data)
            .and_then(|d| d.image())
        {
            self.textures.remove(&url);
            self.requested.remove(&url);
        }
    }

    /// Returns the texture for `url`, queueing a download the first time it is asked for.
    pub fn texture(&mut self, url: &str) -> Option<&TextureHandle> {
        if self.requested.insert(url.to_string()) {
            self.tasks.load_image(self.generation, url.to_string());
        }
        self.textures.get(url)
    }

    pub fn launch(&mut self) {
        let Some(paths) = self.game.clone() else {
            return;
        };
        if self.state.enabled_count() > 0
            && let Err(e) = gameinfo::ensure_patched(&paths.gameinfo())
        {
            self.error(format!(
                "Моды не загрузятся: не удалось изменить gameinfo.gi ({e})"
            ));
            return;
        }
        match steam::launch_game() {
            Ok(()) => self.info("Запуск Deadlock…"),
            Err(e) => self.error(format!("Не удалось запустить Steam: {e}")),
        }
    }

    pub fn check_updates(&mut self) {
        let ids: HashSet<u64> = self
            .state
            .mods
            .iter()
            .filter_map(|m| m.source.as_ref().map(|s| s.mod_id))
            .collect();
        if ids.is_empty() {
            self.info("Нет модов с GameBanana");
            return;
        }
        self.checking_updates = true;
        self.tasks.check_updates(ids.into_iter().collect());
    }

    pub fn handle_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.tasks.rx.try_recv() {
            match msg {
                Msg::Page { generation, result } if generation == self.generation => {
                    self.loading_page = false;
                    match result {
                        Ok(page) => {
                            self.textures.clear();
                            self.requested.clear();
                            self.cards = page.mods;
                            self.has_more = page.has_more;
                            self.total = page.total;
                        }
                        Err(e) => self.page_error = Some(e.to_string()),
                    }
                }
                Msg::Page { .. } => {}
                Msg::Catalog(result) => self.on_catalog(result),
                Msg::Details { id, result } => self.on_details(id, result),
                Msg::Image {
                    generation,
                    url,
                    image,
                } => {
                    let wanted = generation == self.generation && self.requested.contains(&url);
                    if let (true, Some(image)) = (wanted, image) {
                        let texture = ctx.load_texture(&url, image, TextureOptions::LINEAR);
                        self.textures.insert(url, texture);
                    }
                }
                Msg::Prepared(Ok(prepared)) => self.on_prepared(prepared),
                Msg::Prepared(Err(e)) | Msg::Extracted(Err(e)) => {
                    self.busy = None;
                    self.error(format!("Не удалось установить: {e}"));
                }
                Msg::Extracted(Ok(extracted)) => self.on_extracted(extracted),
                Msg::Updates(list) => self.on_updates(&list),
            }
        }
    }

    fn on_catalog(&mut self, result: std::io::Result<Catalog>) {
        match result {
            Ok(mut catalog) => {
                catalog
                    .sections
                    .sort_by_key(|s| locale::section_order(&s.name));
                let names = &self.hero_names;
                let key = |h: &Category| names.get(&h.name).to_lowercase();
                catalog.heroes.sort_by_cached_key(key);
                self.catalog = Some(catalog);
                self.catalog_error = None;
            }
            Err(e) => self.catalog_error = Some(e.to_string()),
        }
    }

    fn on_details(&mut self, id: u64, result: std::io::Result<ModDetails>) {
        if self.pending_quick == Some(id) {
            self.pending_quick = None;
            self.busy = None;
            match result {
                Ok(details) => self.quick_install_ready(details),
                Err(e) => self.error(format!("Не удалось загрузить мод: {e}")),
            }
            return;
        }
        let Some(view) = self.details.as_mut().filter(|v| v.id == id) else {
            return;
        };
        match result {
            Ok(data) => view.data = Some(data),
            Err(e) => {
                self.details = None;
                self.error(format!("Не удалось загрузить мод: {e}"));
            }
        }
    }

    fn on_updates(&mut self, list: &[(u64, i64)]) {
        self.checking_updates = false;
        let latest: HashMap<u64, i64> = list.iter().copied().collect();
        self.updates = self
            .state
            .mods
            .iter()
            .filter_map(|m| {
                let source = m.source.as_ref()?;
                (latest.get(&source.mod_id)? > &source.updated_at).then_some(m.uid)
            })
            .collect();
        match self.updates.len() {
            0 => self.info("Обновлений нет"),
            n => self.info(format!("Доступно обновлений: {n}")),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .find_map(|f| Some(f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()))
        });
        if let (Some(path), None, true) = (dropped, &self.busy, self.game.is_some()) {
            self.install_dropped(path);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::ui_shell::draw(self, ui);
    }
}
