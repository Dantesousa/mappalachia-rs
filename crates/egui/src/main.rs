// GUI-only binary: hide the CMD console window on Windows. The CLI crate
// (`mappalachia-cli`) is unaffected and keeps its console.
#![windows_subsystem = "windows"]

mod app;
mod spotlight_dl;

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Mappalachia RS",
        opts,
        Box::new(|_cc| Ok(Box::new(app::App::new()))),
    )
}
