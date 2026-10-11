mod app;

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
