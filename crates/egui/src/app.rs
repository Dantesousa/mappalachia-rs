use mappalachia_core::{
    db,
    enums::{LockLevel, Signature},
    model::{EntityKind, Space},
    search::{search, Grouped, SearchParams, SearchResult},
};
use mappalachia_render::{color::Rgba, icon::PlotIcon, settings::PlotStyleSettings};

/// `FileIO.GetAvailablePlotIconCount`: the release tree ships 7 base icons
/// (`A.png..G.png`; cf. `render::icon::PlotIcon::assign` docs).
const PLOT_ICON_COUNT: usize = 7;

/// One entry of the items-to-plot list: a search result plus its
/// freshly-assigned legend group, icon and legend text
/// (mirrors `FormMain.ButtonAddToMap_Click`, non-grouped path).
#[derive(Debug, Clone)]
pub struct PlotItem {
    pub result: SearchResult,
    pub legend_group: usize,
    pub icon: PlotIcon,
    pub legend: String,
}

pub struct App {
    pub db_path: Option<String>,
    pub assets_root: Option<String>,
    pub status: String,
    pub search_term: String,
    /// All spaces, loaded at startup via `db::all_spaces` (empty when the DB
    /// path is unset or unreadable).
    pub spaces: Vec<Space>,
    /// Index into `spaces`; defaults to the APPALACHIA space.
    pub space_idx: usize,
    /// Last search results.
    pub results: Vec<SearchResult>,
    /// Checkbox state, parallel to `results`.
    pub checked: Vec<bool>,
    /// Items-to-plot.
    pub items: Vec<PlotItem>,
    /// Next fresh legend group for `add_selected` (non-grouped path: one
    /// fresh group per added item).
    pub next_group: usize,
}

impl App {
    pub fn new() -> Self {
        let db_path = std::env::var("MAPPALACHIA_DB").ok();
        let assets_root = std::env::var("MAPPALACHIA_ASSETS").ok();
        let spaces = match &db_path {
            Some(path) => db::open_readonly(path)
                .and_then(|conn| db::all_spaces(&conn))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        Self::from_parts(db_path, assets_root, spaces)
    }

    pub fn from_parts(
        db_path: Option<String>,
        assets_root: Option<String>,
        spaces: Vec<Space>,
    ) -> Self {
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
        let space_idx = spaces.iter().position(|s| s.is_appalachia()).unwrap_or(0);
        Self {
            db_path,
            assets_root,
            status,
            search_term: String::new(),
            spaces,
            space_idx,
            results: Vec::new(),
            checked: Vec::new(),
            items: Vec::new(),
            next_group: 0,
        }
    }

    /// Mirrors the `FormMain` search flow: `core::search` with the default
    /// signatures/locks, the current space, `advanced = false`.
    fn search_params(&self) -> SearchParams {
        SearchParams {
            search_term: self.search_term.clone(),
            space_form_id: self.spaces.get(self.space_idx).map(|s| s.form_id()),
            advanced: false,
            search_in_instances_only: false,
            signatures: Signature::recommended_default(),
            lock_levels: LockLevel::all(),
        }
    }

    /// Run the search against the configured DB; on success replaces
    /// `results`/`checked` and reports the hit count in `status`.
    /// Opens its own connection so callers (UI button, tests) need no
    /// `rusqlite` dependency.
    pub fn run_search(&mut self) -> Result<(), String> {
        let path = self
            .db_path
            .clone()
            .ok_or_else(|| "MAPPALACHIA_DB is not set".to_string())?;
        let conn = db::open_readonly(&path).map_err(|e| format!("DB open failed: {e}"))?;
        let params = self.search_params();
        let results = search(&conn, &params).map_err(|e| format!("Search failed: {e}"))?;
        let n = results.len();
        self.checked = vec![false; n];
        self.results = results;
        self.status = format!("Search found {n} result(s).");
        Ok(())
    }

    /// Mirrors `FormMain.ButtonAddToMap_Click(addAsGroup: false)`
    /// (`FormMain.cs:1411-1435`, non-grouped path): for each checked result
    /// not already on the map, compute `legend_text(include_count = false)`
    /// (mirrors `GenerateLegendText`), fresh-assign a `PlotIcon` at the next
    /// group with the default palette/size, stamp the group onto the result,
    /// and push a `PlotItem`.
    pub fn add_selected(&mut self) {
        let style = PlotStyleSettings::default();
        let mut added = 0;
        for i in 0..self.results.len() {
            if !self.checked.get(i).copied().unwrap_or(false) {
                continue;
            }
            let key = result_key(&self.results[i]);
            if self.items.iter().any(|it| item_key(it) == key) {
                continue;
            }
            let legend = result_legend_text(&self.results[i]);
            let no_icon = is_region_or_location(&self.results[i]);
            let group = self.next_group;
            self.next_group += 1;
            let icon = PlotIcon::assign(
                group,
                &style.palette,
                style.size as u32,
                no_icon,
                PLOT_ICON_COUNT,
            );
            let mut result = self.results[i].clone();
            match &mut result {
                SearchResult::Grouped(g) => g.legend_group = group as i32,
                SearchResult::Singular { inner, .. } => inner.legend_group = group as i32,
            }
            self.items.push(PlotItem {
                result,
                legend_group: group,
                icon,
                legend,
            });
            added += 1;
        }
        self.status = format!("Added {added} item(s); {} on map.", self.items.len());
    }

    /// Mirrors `FormMain.ButtonRemoveFromMap_Click`: drops every checked
    /// result from the results list and any items-to-plot derived from them.
    pub fn remove_selected(&mut self) {
        let mut doomed = Vec::new();
        for (i, keep) in self.checked.iter().enumerate() {
            if *keep {
                doomed.push(result_key(&self.results[i]));
            }
        }
        self.items.retain(|it| !doomed.contains(&item_key(it)));
        let mut results = Vec::with_capacity(self.results.len());
        let mut checked = Vec::with_capacity(self.checked.len());
        for (r, c) in self.results.drain(..).zip(self.checked.drain(..)) {
            if !c {
                results.push(r);
                checked.push(false);
            }
        }
        self.results = results;
        self.checked = checked;
        self.status = format!(
            "{} item(s) on map; {} result(s).",
            self.items.len(),
            self.results.len()
        );
    }
}

/// Identity of a search result, used to dedupe adds and to match items back
/// to the results they were added from.
fn result_key(r: &SearchResult) -> (String, u32, u32, Option<u32>) {
    let instance = match r {
        SearchResult::Grouped(_) => None,
        SearchResult::Singular {
            instance_form_id, ..
        } => Some(*instance_form_id),
    };
    (
        r.editor_id().to_string(),
        r.form_id(),
        r.space().form_id(),
        instance,
    )
}

fn item_key(it: &PlotItem) -> (String, u32, u32, Option<u32>) {
    result_key(&it.result)
}

/// Mirrors `GroupedSearchResult.GenerateLegendText(settings)` with
/// `include_count = false` (the add path passes the live settings; the egui
/// v1 search panel has no include-count toggle yet).
fn result_legend_text(r: &SearchResult) -> String {
    grouped_of(r).legend_text(false)
}

fn grouped_of(r: &SearchResult) -> &Grouped {
    match r {
        SearchResult::Grouped(g) => g,
        SearchResult::Singular { inner, .. } => inner,
    }
}

/// Mirrors `result.Entity is Library.Region or Library.Location`.
fn is_region_or_location(r: &SearchResult) -> bool {
    matches!(r.entity_kind(), EntityKind::Region | EntityKind::Location)
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::left("search_panel")
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Search");
                ui.horizontal(|ui| {
                    ui.label("Term:");
                    ui.text_edit_singleline(&mut self.search_term);
                });
                let selected = self
                    .spaces
                    .get(self.space_idx)
                    .map(|s| s.friendly_name())
                    .unwrap_or_else(|| "(no spaces loaded)".to_string());
                egui::ComboBox::from_label("Space")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for i in 0..self.spaces.len() {
                            let name = self.spaces[i].friendly_name();
                            ui.selectable_value(&mut self.space_idx, i, name);
                        }
                    });
                if ui.button("Search").clicked() {
                    if let Err(e) = self.run_search() {
                        self.status = e;
                    }
                }
                ui.separator();
                ui.label(format!("Results ({})", self.results.len()));
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for i in 0..self.results.len() {
                            let label = {
                                let r = &self.results[i];
                                format!("{} (x{})", r.editor_id(), r.count())
                            };
                            let mut c = self.checked[i];
                            if ui.checkbox(&mut c, label).changed() {
                                self.checked[i] = c;
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.button("Add selected").clicked() {
                        self.add_selected();
                    }
                    if ui.button("Remove checked").clicked() {
                        self.remove_selected();
                    }
                });
            });

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
            ui.label(format!("Items to plot ({})", self.items.len()));
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for it in &self.items {
                        let c: Rgba = it.icon.color;
                        ui.horizontal(|ui| {
                            ui.colored_label(egui::Color32::from_rgb(c.r, c.g, c.b), "\u{25a0}");
                            ui.label(format!("[g{}] {}", it.legend_group, it.legend));
                        });
                    }
                });
            ui.separator();
            ui.label("Preview panel (Task 3): pending");
            ui.label("Plot panel (Task 4): pending");
            ui.label("Export panel (Task 5): pending");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mappalachia_core::db::all_spaces;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn new_without_envs_names_both_missing_vars() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Save + clear (single-threaded under the lock), snapshot the app,
        // then restore BEFORE asserting so a failure cannot leak cleared env
        // into the DB tests below.
        let saved_db = std::env::var("MAPPALACHIA_DB").ok();
        let saved_assets = std::env::var("MAPPALACHIA_ASSETS").ok();
        // SAFETY: env mutation is serialized by ENV_LOCK.
        unsafe {
            std::env::remove_var("MAPPALACHIA_DB");
            std::env::remove_var("MAPPALACHIA_ASSETS");
        }
        let app = App::new();
        let (db_none, assets_none, status) = (
            app.db_path.is_none(),
            app.assets_root.is_none(),
            app.status.clone(),
        );
        unsafe {
            if let Some(v) = saved_db {
                std::env::set_var("MAPPALACHIA_DB", v);
            }
            if let Some(v) = saved_assets {
                std::env::set_var("MAPPALACHIA_ASSETS", v);
            }
        }
        assert!(db_none);
        assert!(assets_none);
        for var in ["MAPPALACHIA_DB", "MAPPALACHIA_ASSETS"] {
            assert!(
                status.contains(var),
                "status should name {var}, got: {status}"
            );
        }
    }

    /// Build an app pointed at the real test DB searching APPALACHIA, or
    /// `None` (skip) when `MAPPALACHIA_DB` is unset. Reads env under the
    /// shared lock so the env-clearing test above cannot race it.
    fn search_app() -> Option<App> {
        let path = {
            let _guard = ENV_LOCK.lock().unwrap();
            std::env::var("MAPPALACHIA_DB").ok()
        }?;
        let conn = db::open_readonly(&path).expect("test DB opens");
        let spaces = all_spaces(&conn).expect("spaces load");
        assert!(
            spaces.iter().any(|s| s.is_appalachia()),
            "test DB has an APPALACHIA space"
        );
        let mut app = App::from_parts(Some(path), None, spaces);
        assert!(app.spaces[app.space_idx].is_appalachia());
        app.search_term = "Nuka Cola".to_string();
        Some(app)
    }

    #[test]
    fn search_nuka_cola_in_appalachia_returns_rows() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.run_search().expect("search succeeds");
        assert!(
            !app.results.is_empty(),
            "expected rows for 'Nuka Cola' in APPALACHIA"
        );
        assert_eq!(app.checked.len(), app.results.len());
    }

    #[test]
    fn add_first_result_assigns_orange_red_and_legend() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.run_search().expect("search succeeds");
        assert!(
            app.results.len() >= 2,
            "need 2+ rows to check group sequencing, got {}",
            app.results.len()
        );
        app.checked[0] = true;
        app.checked[1] = true;
        app.add_selected();
        assert_eq!(app.items.len(), 2);
        let orange_red = Rgba::new(255, 69, 0, 255);
        let cyan = Rgba::new(0, 255, 255, 255);
        assert_eq!(app.items[0].icon.color, orange_red);
        assert_eq!(app.items[0].icon.size, 50);
        assert_eq!(app.items[0].legend_group, 0);
        assert!(
            !app.items[0].legend.is_empty(),
            "legend text must be non-empty"
        );
        assert_eq!(app.items[1].icon.color, cyan);
        assert_eq!(app.items[1].legend_group, 1);
        assert_eq!(app.next_group, 2);
        // Re-adding the same checked rows is a no-op (mirrors the C#
        // `ItemsToPlot.Contains` skip).
        app.add_selected();
        assert_eq!(app.items.len(), 2);
    }

    #[test]
    fn remove_selected_clears_items_and_results() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.run_search().expect("search succeeds");
        let n = app.results.len();
        app.checked[0] = true;
        app.add_selected();
        assert_eq!(app.items.len(), 1);
        app.remove_selected();
        assert!(app.items.is_empty());
        assert_eq!(app.results.len(), n - 1);
        assert_eq!(app.checked.len(), n - 1);
    }
}
