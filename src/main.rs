#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod config;
mod ui;
mod update;

use eframe::egui;

fn load_icon() -> Option<egui::IconData> {
    // Icon is embedded in the binary at compile time
    let icon_bytes = include_bytes!("../icons/app-32.png");
    let image = image::load_from_memory(icon_bytes).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 900.0])
        .with_min_inner_size([900.0, 700.0])
        .with_title("Timebox");

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Timebox",
        options,
        Box::new(|cc| Ok(Box::new(ui::JiraTimeApp::new(cc)))),
    )
}
