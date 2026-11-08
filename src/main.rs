// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod archive;
mod gamebanana;
mod gameinfo;
mod install;
mod library;
mod locale;
mod steam;
mod tasks;
mod ui_catalog;
mod ui_library;
mod ui_shell;

use eframe::egui;

/// Name shown in the window title and the top bar.
pub const APP_NAME: &str = "Deadlock Launcher Lite";

/// The same picture as the exe icon; a broken PNG just means no window icon.
fn window_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default()
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
        renderer: eframe::Renderer::Glow,
        // A flat 2D UI needs no multisampling, depth or stencil buffers.
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    eframe::run_native(
        "deadlock-launcher",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc.egui_ctx.clone())))),
    )
}
