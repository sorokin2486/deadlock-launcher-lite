//! "My mods" tab: on/off, load order, updates, removal and the hero setting.

use eframe::egui::{self, Color32, RichText};

use crate::app::App;
use crate::library::{self, ConflictPolicy};

enum Action {
    Toggle(usize),
    Up(usize),
    Down(usize),
    Delete(usize),
    Update(u64, u64),
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    toolbar(app, ui);
    ui.weak("Чем выше мод в списке, тем выше его приоритет.");
    ui.separator();

    if app.state.mods.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label("Модов пока нет. Установите их из каталога или перетащите архив в окно.");
        });
        return;
    }

    let mut action = None;
    let busy = app.busy.is_some();
    let count = app.state.mods.len();
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for (index, entry) in app.state.mods.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut enabled = entry.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        action = Some(Action::Toggle(index));
                    }
                    if ui
                        .add_enabled(index > 0, egui::Button::new("Выше").small())
                        .clicked()
                    {
                        action = Some(Action::Up(index));
                    }
                    let down = egui::Button::new("Ниже").small();
                    if ui.add_enabled(index + 1 < count, down).clicked() {
                        action = Some(Action::Down(index));
                    }
                    let name = RichText::new(&entry.name).size(15.0);
                    ui.label(if entry.enabled { name } else { name.weak() });
                    if let Some(hero) = entry.hero() {
                        ui.weak(app.hero_names.get(&hero.name));
                    }
                    if app.updates.contains(&entry.uid)
                        && let Some(source) = &entry.source
                    {
                        let text = RichText::new("Обновить").color(Color32::from_rgb(255, 202, 40));
                        if ui.add_enabled(!busy, egui::Button::new(text)).clicked() {
                            action = Some(Action::Update(source.mod_id, entry.uid));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("Удалить"))
                            .clicked()
                        {
                            action = Some(Action::Delete(index));
                        }
                        if let Some(source) = &entry.source
                            && ui.small_button("Страница мода").clicked()
                        {
                            let url = format!("https://gamebanana.com/mods/{}", source.mod_id);
                            let _ = crate::steam::open_uri(&url);
                        }
                    });
                });
            }
        });

    if let Some(action) = action {
        apply(app, action);
    }
}

fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let checking = app.checking_updates;
        let check = egui::Button::new("Проверить обновления");
        if ui.add_enabled(!checking, check).clicked() {
            app.check_updates();
        }
        if checking {
            ui.spinner();
        }
        if ui.button("Папка модов").clicked()
            && let Some(paths) = &app.game
        {
            let _ = crate::steam::open_uri(&paths.addons().display().to_string());
        }
        ui.add_space(12.0);
        conflict_policy_picker(app, ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled = app.state.enabled_count();
            ui.weak(format!("Включено {enabled} из {}", library::MAX_ENABLED));
        });
    });
}

fn conflict_policy_picker(app: &mut App, ui: &mut egui::Ui) {
    ui.label("Второй мод на героя:");
    let before = app.state.hero_conflict;
    egui::ComboBox::from_id_salt("hero_conflict")
        .selected_text(app.state.hero_conflict.label())
        .show_ui(ui, |ui| {
            for policy in ConflictPolicy::ALL {
                ui.selectable_value(&mut app.state.hero_conflict, policy, policy.label());
            }
        });
    if before != app.state.hero_conflict
        && let Err(e) = app.state.save()
    {
        app.error(format!("Не удалось сохранить настройки: {e}"));
    }
}

fn apply(app: &mut App, action: Action) {
    let enabled_count = app.state.enabled_count();
    let mods = &mut app.state.mods;
    match action {
        Action::Toggle(i) => {
            if !mods[i].enabled && enabled_count >= library::MAX_ENABLED {
                app.error(format!(
                    "Включить можно не больше {} модов",
                    library::MAX_ENABLED
                ));
                return;
            }
            mods[i].enabled = !mods[i].enabled;
        }
        Action::Up(i) => mods.swap(i, i - 1),
        Action::Down(i) => mods.swap(i, i + 1),
        Action::Delete(i) => {
            let removed = mods.remove(i);
            if let Err(e) = library::delete_files(&removed) {
                mods.insert(i, removed);
                app.error(format!(
                    "Не удалось удалить файлы: {e}. Закройте игру и повторите."
                ));
                return;
            }
            app.updates.remove(&removed.uid);
            app.info(format!("Удалено: {}", removed.name));
        }
        Action::Update(mod_id, uid) => {
            app.open_details(mod_id, Some(uid));
            return;
        }
    }
    app.commit();
}
