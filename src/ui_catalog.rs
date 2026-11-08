//! Catalog tab: section and hero menu, mod list, and the mod details dialog.

use eframe::egui::{self, RichText, Sense, Vec2};

use crate::app::{App, Section};
use crate::gamebanana::{self, Sort};
use crate::locale;
use crate::ui_shell::mb;

const THUMB: Vec2 = Vec2::new(176.0, 99.0);
const DETAILS_IMAGE: Vec2 = Vec2::new(530.0, 298.0);
const SIDEBAR_WIDTH: f32 = 190.0;
/// Room the details dialog keeps for its header, buttons and window margins.
const DIALOG_CHROME: f32 = 190.0;

pub fn sidebar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::left("sections")
        .resizable(false)
        .exact_size(SIDEBAR_WIDTH)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    // Full-width rows with left-aligned names.
                    let layout = egui::Layout::top_down_justified(egui::Align::Min);
                    ui.with_layout(layout, |ui| sidebar_items(app, ui));
                });
        });
}

fn sidebar_items(app: &mut App, ui: &mut egui::Ui) {
    let current = match app.active_search {
        Some(_) => None,
        None => Some(app.section),
    };
    let mut picked = None;
    let mut item = |ui: &mut egui::Ui, section: Section, text: &str| {
        let label = egui::Button::selectable(current == Some(section), text)
            .min_size(egui::vec2(0.0, 24.0));
        if ui.add(label).clicked() {
            picked = Some(section);
        }
    };

    ui.weak("Разделы");
    item(ui, Section::All, "Все моды");
    match (&app.catalog, &app.catalog_error) {
        (Some(catalog), _) => {
            for section in catalog.sections.iter() {
                item(
                    ui,
                    Section::Category(section.id),
                    locale::section(&section.name),
                );
            }
            ui.add_space(10.0);
            ui.weak("Герои");
            for hero in catalog.heroes.iter() {
                item(
                    ui,
                    Section::Category(hero.id),
                    app.hero_names.get(&hero.name),
                );
            }
        }
        (None, Some(_)) => {
            ui.add_space(6.0);
            ui.weak("Разделы не загрузились");
            if ui.button("Повторить").clicked() {
                app.catalog_error = None;
                app.tasks.load_catalog();
            }
        }
        (None, None) => {
            ui.spinner();
        }
    }
    if let Some(section) = picked {
        app.select_section(section);
    }
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    header(app, ui);
    ui.separator();
    if let Some(error) = app.page_error.clone() {
        let color = ui.visuals().error_fg_color;
        ui.colored_label(color, format!("Не удалось загрузить список: {error}"));
        if ui.button("Повторить").clicked() {
            app.reload_page();
        }
        return;
    }
    if app.loading_page && app.cards.is_empty() {
        ui.spinner();
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            // Taken out for the loop so rows can borrow `app` mutably; no per-frame clone.
            let cards = std::mem::take(&mut app.cards);
            for card in &cards {
                card_row(app, ui, card);
                ui.separator();
            }
            app.cards = cards;
            if app.cards.is_empty() {
                ui.weak("Ничего не найдено");
            } else {
                pager(app, ui);
            }
        });
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(app.section_title()).size(18.0).strong());
        if !app.loading_page {
            ui.weak(format!("модов: {}", app.total));
        } else {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .checkbox(&mut app.state.hide_adult, "Скрывать превью 18+")
                .changed()
                && let Err(e) = app.state.save()
            {
                app.error(format!("Не удалось сохранить настройки: {e}"));
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let field = egui::TextEdit::singleline(&mut app.search_input)
            .hint_text("Поиск по всем модам")
            .desired_width(280.0);
        let search = ui.add(field);
        let submitted = search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if submitted || ui.button("Найти").clicked() {
            let text = app.search_input.trim().to_string();
            match text.is_empty() {
                true => app.select_section(app.section),
                false => app.search(&text),
            }
        }
        if app.active_search.is_some() && ui.button("Сбросить").clicked() {
            app.select_section(app.section);
        }
        if app.active_search.is_none() {
            ui.add_space(12.0);
            sort_picker(app, ui);
        }
    });
}

fn sort_picker(app: &mut App, ui: &mut egui::Ui) {
    let before = app.sort;
    egui::ComboBox::from_id_salt("sort")
        .selected_text(app.sort.label())
        .show_ui(ui, |ui| {
            for sort in Sort::ALL {
                ui.selectable_value(&mut app.sort, sort, sort.label());
            }
        });
    if before != app.sort {
        app.page = 1;
        app.reload_page();
    }
}

fn card_row(app: &mut App, ui: &mut egui::Ui, card: &gamebanana::ModCard) {
    let installed = app
        .state
        .mods
        .iter()
        .any(|m| m.source.as_ref().is_some_and(|s| s.mod_id == card.id));
    let mut open = false;
    let mut install = false;
    ui.horizontal(|ui| {
        open |= thumbnail(app, ui, card.thumb(), card.adult).clicked();
        ui.vertical(|ui| {
            let title = egui::Label::new(RichText::new(&card.name).size(16.0).strong());
            open |= ui.add(title.sense(Sense::click())).clicked();
            let category = locale::section(card.category());
            ui.label(format!("{}  •  {}", card.author(), category));
            ui.horizontal(|ui| {
                ui.weak(format!("лайков: {}", card.likes));
                if card.adult {
                    ui.colored_label(ui.visuals().warn_fg_color, "18+");
                }
            });
            ui.horizontal(|ui| {
                if installed {
                    ui.colored_label(egui::Color32::from_rgb(102, 187, 106), "Установлен");
                } else {
                    let button = egui::Button::new("Установить");
                    install = ui.add_enabled(app.busy.is_none(), button).clicked();
                }
                open |= ui.button("Подробнее").clicked();
            });
        });
    });
    if install {
        app.quick_install(card.id);
    } else if open {
        app.open_details(card.id, None);
    }
}

fn thumbnail(app: &mut App, ui: &mut egui::Ui, url: Option<String>, adult: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(THUMB, Sense::click());
    let hidden = adult && app.state.hide_adult;
    match url
        .filter(|_| !hidden)
        .and_then(|u| app.texture(&u).map(|t| t.id()))
    {
        Some(id) => {
            egui::Image::new(egui::load::SizedTexture::new(id, THUMB)).paint_at(ui, rect);
        }
        None => {
            let visuals = ui.visuals();
            ui.painter().rect_filled(rect, 4.0, visuals.faint_bg_color);
            let label = if hidden { "18+" } else { "" };
            let font = egui::FontId::proportional(18.0);
            let color = visuals.weak_text_color();
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font,
                color,
            );
        }
    }
    response
}

fn pager(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let back = egui::Button::new("Назад");
        if ui
            .add_enabled(app.page > 1 && !app.loading_page, back)
            .clicked()
        {
            app.page -= 1;
            app.reload_page();
        }
        ui.label(format!("Страница {} из {}", app.page, app.page_count()));
        let next = egui::Button::new("Дальше");
        if ui
            .add_enabled(app.has_more && !app.loading_page, next)
            .clicked()
        {
            app.page += 1;
            app.reload_page();
        }
    });
    ui.add_space(6.0);
}

pub fn details_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(view) = app.details.as_ref() else {
        return;
    };
    let replaces = view.replaces;
    let data = view.data.clone();
    let mut close = false;
    let mut install = None;
    let response = egui::Modal::new(egui::Id::unique("details")).show(ctx, |ui| {
        ui.set_width(DETAILS_IMAGE.x + 20.0);
        let Some(details) = &data else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Загрузка…");
            });
            close = ui.button("Закрыть").clicked();
            return;
        };
        ui.heading(&details.name);
        let hero = app
            .hero_for(details)
            .map(|h| format!("  •  {}", app.hero_names.get(&h.name)))
            .unwrap_or_default();
        ui.label(format!(
            "{}{hero}  •  лайков: {}  •  скачиваний: {}",
            details.author(),
            details.likes,
            details.downloads
        ));
        // Header and buttons stay put; the middle scrolls when the window is short.
        let body_height = (ctx.content_rect().height() - DIALOG_CHROME).max(160.0);
        egui::ScrollArea::vertical()
            .max_height(body_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                install = details_body(app, ui, details, replaces.is_some());
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Открыть на GameBanana").clicked() {
                let _ = crate::steam::open_uri(&details.page_url());
            }
            close |= ui.button("Закрыть").clicked();
        });
    });
    if let (Some(file), Some(details)) = (install, data) {
        app.close_details();
        match replaces {
            Some(uid) => app.update_mod(file, &details, uid),
            None => app.begin_install(file, details),
        }
        return;
    }
    if close || response.should_close() {
        app.close_details();
    }
}

/// Picture, description and file list of the details dialog. Returns the file to install.
fn details_body(
    app: &mut App,
    ui: &mut egui::Ui,
    details: &gamebanana::ModDetails,
    updating: bool,
) -> Option<gamebanana::GbFile> {
    let hidden = details.adult && app.state.hide_adult;
    if let Some(id) = details
        .image()
        .filter(|_| !hidden)
        .and_then(|u| app.texture(&u).map(|t| t.id()))
    {
        ui.image(egui::load::SizedTexture::new(id, DETAILS_IMAGE));
    }
    ui.label(gamebanana::html_to_text(&details.text));
    ui.separator();
    if updating {
        let color = ui.visuals().warn_fg_color;
        ui.colored_label(
            color,
            "Выберите файл новой версии — он заменит установленный.",
        );
    }
    ui.label(RichText::new("Файлы").strong());
    let mut install = None;
    for file in &details.files {
        ui.horizontal(|ui| {
            let button = egui::Button::new("Установить");
            if ui.add_enabled(app.busy.is_none(), button).clicked() {
                install = Some(file.clone());
            }
            ui.label(format!("{} — {:.1} МБ", file.name, mb(file.size)));
            if !file.description.is_empty() {
                ui.weak(&file.description);
            }
        });
    }
    if details.files.is_empty() {
        ui.weak("У мода нет файлов");
    }
    install
}
