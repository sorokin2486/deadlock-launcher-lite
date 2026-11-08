//! Window frame: top bar, status bar, game folder setup and modal dialogs.

use std::sync::atomic::Ordering;

use eframe::egui::{self, Color32, RichText};

use crate::app::{App, Tab};
use crate::library::ConflictPolicy;
use crate::steam::GamePaths;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    top_bar(app, ui);
    status_bar(app, ui);
    if app.game.is_some() && app.tab == Tab::Catalog {
        crate::ui_catalog::sidebar(app, ui);
    }
    egui::CentralPanel::default_margins().show(ui, |ui| {
        if app.game.is_none() {
            game_setup(app, ui);
            return;
        }
        match app.tab {
            Tab::Catalog => crate::ui_catalog::draw(app, ui),
            Tab::Library => crate::ui_library::draw(app, ui),
        }
    });
    let ctx = ui.ctx().clone();
    crate::ui_catalog::details_dialog(app, &ctx);
    conflict_dialog(app, &ctx);
    variant_dialog(app, &ctx);
}

fn top_bar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("top").show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading(crate::APP_NAME);
            ui.add_space(16.0);
            let catalog = RichText::new("Каталог").size(16.0);
            ui.selectable_value(&mut app.tab, Tab::Catalog, catalog);
            let installed =
                RichText::new(format!("Мои моды ({})", app.state.mods.len())).size(16.0);
            ui.selectable_value(&mut app.tab, Tab::Library, installed);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let play = egui::Button::new(RichText::new("Играть").size(18.0).strong())
                    .fill(Color32::from_rgb(46, 125, 50))
                    .min_size(egui::vec2(130.0, 34.0));
                if ui.add_enabled(app.game.is_some(), play).clicked() {
                    app.launch();
                }
            });
        });
        ui.add_space(6.0);
    });
}

fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if let Some(busy) = &app.busy {
                ui.spinner();
                ui.label(busy);
                let done = app.tasks.progress.load(Ordering::Relaxed);
                if app.download_total > 0 && done > 0 {
                    let fraction = done as f32 / app.download_total as f32;
                    let text = format!("{:.1} из {:.1} МБ", mb(done), mb(app.download_total));
                    let bar = egui::ProgressBar::new(fraction.min(1.0)).desired_width(220.0);
                    ui.add(bar.text(text));
                }
            } else if let Some(status) = &app.status {
                let color = match status.error {
                    true => ui.visuals().error_fg_color,
                    false => ui.visuals().text_color(),
                };
                ui.label(RichText::new(&status.text).color(color));
            } else {
                ui.weak("Архив мода можно перетащить в окно: zip, 7z, rar или vpk");
            }
        });
        ui.add_space(4.0);
    });
}

fn game_setup(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.heading("Deadlock не найден");
        ui.label("Укажите папку игры, например X:\\SteamLibrary\\steamapps\\common\\Deadlock");
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut app.path_input).desired_width(480.0));
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 240.0).max(0.0) / 2.0);
            if ui.button("Использовать папку").clicked() {
                match GamePaths::new(app.path_input.trim().into()) {
                    Some(paths) => app.set_game(Some(paths)),
                    None => app.error("В этой папке нет game\\citadel\\gameinfo.gi"),
                }
            }
            if ui.button("Искать снова").clicked() {
                match crate::steam::detect_game() {
                    Some(paths) => app.set_game(Some(paths)),
                    None => app.error("Deadlock не найден в библиотеках Steam"),
                }
            }
        });
    });
}

fn conflict_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(prompt) = app.conflict.as_mut() else {
        return;
    };
    let old_names: Vec<String> = app
        .state
        .mods
        .iter()
        .filter(|m| prompt.existing.contains(&m.uid))
        .map(|m| format!("«{}»", m.name))
        .collect();
    let mut decision = None;
    egui::Modal::new(egui::Id::unique("hero_conflict")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.heading(format!("У героя «{}» уже есть мод", prompt.hero));
        ui.add_space(4.0);
        ui.label(format!("Установлено: {}.", old_names.join(", ")));
        let them = if old_names.len() > 1 {
            "ними"
        } else {
            "ним"
        };
        ui.label(format!(
            "Новый мод «{}» может конфликтовать с {them}.",
            prompt.details.name
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Заменить").clicked() {
                decision = Some(Some(ConflictPolicy::Replace));
            }
            if ui.button("Оставить оба").clicked() {
                decision = Some(Some(ConflictPolicy::Keep));
            }
            if ui.button("Отмена").clicked() {
                decision = Some(None);
            }
        });
        ui.add_space(4.0);
        ui.checkbox(&mut prompt.remember, "Запомнить выбор");
    });
    // Take the prompt only once a button was pressed; it must survive idle frames.
    let Some(decision) = decision else {
        return;
    };
    if let Some(prompt) = app.conflict.take() {
        app.resolve_conflict(prompt, decision);
    }
}

fn variant_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(choice) = app.choice.as_mut() else {
        return;
    };
    let mut decision = None;
    egui::Modal::new(egui::Id::unique("variants")).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading("Выберите части мода");
        ui.label(
            "В архиве несколько частей. Обычно нужны все.
Если это варианты одного мода, оставьте один.",
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                let rows = choice
                    .prepared
                    .packages
                    .iter()
                    .zip(choice.selected.iter_mut());
                for (package, selected) in rows {
                    ui.checkbox(selected, &package.label);
                }
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let any = choice.selected.iter().any(|s| *s);
            if ui
                .add_enabled(any, egui::Button::new("Установить"))
                .clicked()
            {
                decision = Some(true);
            }
            if ui.button("Отмена").clicked() {
                decision = Some(false);
            }
        });
    });
    let (Some(accept), Some(choice)) = (decision, app.choice.take_if(|_| decision.is_some()))
    else {
        return;
    };
    if accept {
        app.extract_selected(choice);
    } else {
        cleanup_temp(choice.prepared);
        app.busy = None;
    }
}

fn cleanup_temp(prepared: crate::tasks::Prepared) {
    if let (true, crate::archive::Source::File(path)) = (prepared.temp, &prepared.archive) {
        let _ = std::fs::remove_file(path);
    }
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
