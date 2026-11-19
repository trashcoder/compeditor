#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod compose;
mod docker_hub;
mod environment;
mod history;
mod storage;
mod yaml_format;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 720.0])
        .with_min_inner_size([540.0, 400.0])
        .with_app_id("io.docksie.app");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Docksie",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx, path)))),
    )
}
