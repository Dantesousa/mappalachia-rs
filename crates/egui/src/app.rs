pub struct App {
    pub db_path: Option<String>,
    pub assets_root: Option<String>,
    pub status: String,
}

impl App {
    pub fn new() -> Self {
        let db_path = std::env::var("MAPPALACHIA_DB").ok();
        let assets_root = std::env::var("MAPPALACHIA_ASSETS").ok();
        let mut missing = Vec::new();
        if db_path.is_none() {
            missing.push("MAPPALACHIA_DB");
        }
        if assets_root.is_none() {
            missing.push("MAPPALACHIA_ASSETS");
        }
        let status = if missing.is_empty() {
            "Ready.".to_string()
        } else {
            format!("Missing env: {}. Set and restart.", missing.join(", "))
        };
        Self {
            db_path,
            assets_root,
            status,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Mappalachia RS");
            ui.label(&self.status);
            ui.label(format!(
                "DB: {}",
                self.db_path.as_deref().unwrap_or("(not set)")
            ));
            ui.label(format!(
                "Assets: {}",
                self.assets_root.as_deref().unwrap_or("(not set)")
            ));
            ui.separator();
            ui.label("Search panel (Task 2): pending");
            ui.label("Preview panel (Task 3): pending");
            ui.label("Plot panel (Task 4): pending");
            ui.label("Export panel (Task 5): pending");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_without_envs_records_status_error() {
        // SAFETY: single-threaded unit test manipulating process env.
        unsafe {
            std::env::remove_var("MAPPALACHIA_DB");
            std::env::remove_var("MAPPALACHIA_ASSETS");
        }
        let app = App::new();
        assert!(app.db_path.is_none());
        assert!(app.assets_root.is_none());
        assert!(
            app.status.contains("MAPPALACHIA_DB"),
            "status should name missing envs, got: {}",
            app.status
        );
    }
}
