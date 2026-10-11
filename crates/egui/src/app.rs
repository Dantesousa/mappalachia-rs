use mappalachia_core::{
    common::MAP_IMAGE_RESOLUTION,
    db,
    enums::{LockLevel, Signature},
    model::{EntityKind, Space},
    search::{search, Grouped, SearchParams, SearchResult},
};
use mappalachia_render::{
    assets::Assets,
    color::Rgba,
    draw::{draw, DrawInput, Plottable},
    icon::PlotIcon,
    settings::{
        BackgroundImageType, CompassStyle, LegendHorizontalAlignment, LegendStyle,
        LegendVerticalAlignment, MapSettings, PlotMode, PlotSettings, PlotStyleSettings,
    },
};

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
    /// Map background (`MapSettings.BackgroundImage`; C# default `Menu`).
    pub background: BackgroundImageType,
    /// Map brightness (`MapSettings.Brightness`; C# default 0.5, setter
    /// clamps to 0.1–2.0).
    pub brightness: f32,
    /// Map title (`MapSettings.Title`; C# default `""`).
    pub title: String,
    /// Legend style/alignments (C# defaults Compact/Right/Center).
    pub legend_style: LegendStyle,
    pub legend_halign: LegendHorizontalAlignment,
    pub legend_valign: LegendVerticalAlignment,
    /// Compass style (C# default `WhenUseful`).
    pub compass: CompassStyle,
    /// `MapSettings.ShowCoordinateGrid` (C# default off).
    pub show_grid: bool,
    /// Drives both `MapMarkerIcons` and `MapMarkerLabels` (C# defaults off;
    /// v1 exposes a single markers toggle).
    pub show_markers: bool,
    /// `MapSettings.ShowInfestations` (C# default off).
    pub show_infestations: bool,
    /// Plot style (C# defaults: size 50, 8-color palette); display-only in
    /// v1, editable in a later task.
    pub plot_style: PlotStyleSettings,
    /// `PlotSettings.Mode`, locked to `Standard` until the modes plan
    /// (other modes appear greyed with "(soon)" in the combo).
    pub plot_mode: PlotMode,
    /// Background-thread preview state (Task 4): the worker thread renders
    /// and sends straight-alpha RGBA bytes over `preview_rx`; the UI
    /// thread polls and uploads the texture. Never touched off-thread.
    pub preview: PreviewState,
    /// Live channel while `preview == Rendering`; `None` otherwise.
    preview_rx: Option<std::sync::mpsc::Receiver<PreviewResult>>,
}

/// Preview lifecycle: idle, worker running, uploaded texture, or worker
/// error text. Only the `PreviewResult` payload crosses threads; the
/// `egui::TextureHandle` is created on the UI thread from those bytes.
pub enum PreviewState {
    Idle,
    Rendering,
    Ready(egui::TextureHandle),
    Failed(String),
}

/// What the preview worker sends home: straight-alpha RGBA bytes
/// (`4096*4096*4`, ready for
/// `egui::ColorImage::from_rgba_unmultiplied`) or an error string.
type PreviewResult = Result<Vec<u8>, String>;

/// Typed draw inputs resolved from [`App`] UI state (cf. C#
/// `Recipe.AsSettings` minus recipe I/O): the stored full-DB [`Space`]
/// (cloned from `spaces`, never a recipe stub — `populate_from_data` is
/// only needed for opened recipes, Task 5) plus the typed
/// [`MapSettings`]/[`PlotSettings`].
///
/// There is no `render::settings::Settings` root type (the render crate
/// deliberately did not port C# `Settings.cs`: its `Space` setter is
/// DB-coupled), so this local struct is the conversion target. Consumed by
/// the Task 4 preview worker (`start_preview` clones it onto the thread).
#[derive(Debug, Clone)]
pub struct DrawSettings {
    pub space: Space,
    pub map: MapSettings,
    pub plot: PlotSettings,
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
            background: BackgroundImageType::Menu,
            brightness: 0.5,
            title: String::new(),
            legend_style: LegendStyle::Compact,
            legend_halign: LegendHorizontalAlignment::Right,
            legend_valign: LegendVerticalAlignment::Center,
            compass: CompassStyle::WhenUseful,
            show_grid: false,
            show_markers: false,
            show_infestations: false,
            plot_style: PlotStyleSettings::default(),
            plot_mode: PlotMode::Standard,
            preview: PreviewState::Idle,
            preview_rx: None,
        }
    }

    /// Resolve the UI state into typed draw inputs: the stored full-DB
    /// [`Space`] plus [`MapSettings`]/[`PlotSettings`] built from the panel
    /// state (all other typed fields keep their C# defaults). Brightness is
    /// clamped to 0.1–2.0 like the C# setter. Returns `None` when no spaces
    /// are loaded (DB path unset or unreadable).
    pub fn draw_settings(&self) -> Option<DrawSettings> {
        let space = self.spaces.get(self.space_idx)?.clone();
        let map = MapSettings {
            background_image: self.background,
            brightness: self.brightness.clamp(0.1, 2.0),
            title: self.title.clone(),
            legend_style: self.legend_style,
            legend_horizontal_alignment: self.legend_halign,
            legend_vertical_alignment: self.legend_valign,
            compass_style: self.compass,
            show_coordinate_grid: self.show_grid,
            map_marker_icons: self.show_markers,
            map_marker_labels: self.show_markers,
            show_infestations: self.show_infestations,
            ..MapSettings::default()
        };
        let plot = PlotSettings {
            mode: self.plot_mode,
            plot_style_settings: self.plot_style.clone(),
            ..PlotSettings::default()
        };
        Some(DrawSettings { space, map, plot })
    }

    /// Spawn the preview worker: clones the current items + [`DrawSettings`]
    /// plus the DB/assets paths onto a `std::thread` running
    /// [`render_preview_bytes`]; the bytes come back over an `mpsc`
    /// channel and are uploaded as a texture by [`App::poll_preview`] on
    /// the UI thread. No-op while already `Rendering`. Never touches the
    /// egui `Context` off-thread — only the [`PreviewResult`] crosses.
    pub fn start_preview(&mut self) {
        if matches!(self.preview, PreviewState::Rendering) {
            return;
        }
        let Some(settings) = self.draw_settings() else {
            let msg = "Cannot render: no spaces loaded.".to_string();
            self.status = msg.clone();
            self.preview = PreviewState::Failed(msg);
            return;
        };
        let (Some(db_path), Some(assets_root)) = (self.db_path.clone(), self.assets_root.clone())
        else {
            let msg = "Cannot render: MAPPALACHIA_DB / MAPPALACHIA_ASSETS not set.".to_string();
            self.status = msg.clone();
            self.preview = PreviewState::Failed(msg);
            return;
        };
        let items = self.items.clone();
        let (tx, rx) = std::sync::mpsc::channel::<PreviewResult>();
        self.preview_rx = Some(rx);
        self.preview = PreviewState::Rendering;
        self.status = "Rendering...".to_string();
        std::thread::spawn(move || {
            let out = render_preview_bytes(&db_path, &assets_root, settings, items);
            let _ = tx.send(out);
        });
    }

    /// Poll the preview worker (call once per frame from `update`):
    /// on payload upload `ColorImage::from_rgba_unmultiplied` bytes as a
    /// texture; on error record `Failed`. Keeps repainting while the
    /// worker runs so the result is picked up promptly.
    fn poll_preview(&mut self, ctx: &egui::Context) {
        enum Outcome {
            None,
            Empty,
            Got(PreviewResult),
            Disconnected,
        }
        let outcome = match &self.preview_rx {
            None => Outcome::None,
            Some(rx) => match rx.try_recv() {
                Ok(res) => Outcome::Got(res),
                Err(std::sync::mpsc::TryRecvError::Empty) => Outcome::Empty,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Outcome::Disconnected,
            },
        };
        match outcome {
            Outcome::None => {}
            Outcome::Empty => ctx.request_repaint(),
            Outcome::Got(Ok(bytes)) => {
                self.preview_rx = None;
                let side = MAP_IMAGE_RESOLUTION as usize;
                let img = egui::ColorImage::from_rgba_unmultiplied([side, side], &bytes);
                let tex = ctx.load_texture("preview", img, Default::default());
                self.preview = PreviewState::Ready(tex);
                self.status = "Preview ready.".to_string();
            }
            Outcome::Got(Err(e)) => {
                self.preview_rx = None;
                self.status = e.clone();
                self.preview = PreviewState::Failed(e);
            }
            Outcome::Disconnected => {
                self.preview_rx = None;
                let msg = "Preview worker died.".to_string();
                self.status = msg.clone();
                self.preview = PreviewState::Failed(msg);
            }
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
        if self.spaces.is_empty() {
            return Err("Space list is empty; cannot scope search".to_string());
        }
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

/// Run the Standard render off-thread: open the DB + assets on the worker,
/// plot the current items as-is (icons/groups already assigned by
/// `add_selected`, mirroring the C# CLI path which plots stored
/// `ItemsToPlot` untouched), and return straight-alpha RGBA bytes for
/// `ColorImage::from_rgba_unmultiplied`. Errors become strings so the
/// whole [`PreviewResult`] stays `Send + 'static`.
fn render_preview_bytes(
    db_path: &str,
    assets_root: &str,
    settings: DrawSettings,
    items: Vec<PlotItem>,
) -> PreviewResult {
    let conn = db::open_readonly(db_path).map_err(|e| format!("DB open failed: {e}"))?;
    let mut assets = Assets::open(assets_root);
    let plottables: Vec<Plottable> = items
        .into_iter()
        .map(|it| Plottable {
            result: it.result,
            legend_group: it.legend_group as i32,
            legend_text: it.legend,
            icon: it.icon,
        })
        .collect();
    let pix = draw(DrawInput {
        conn: &conn,
        assets: &mut assets,
        space: settings.space,
        map: settings.map,
        plot: settings.plot,
        items: plottables,
    })
    .map_err(|e| format!("Render failed: {e}"))?;
    Ok(straight_rgba_bytes(&pix))
}

/// Unpremultiply a rendered (premultiplied) canvas to straight-alpha RGBA
/// bytes, mirroring `canvas::straight_pixel` per pixel.
fn straight_rgba_bytes(pix: &tiny_skia::Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity((pix.width() * pix.height() * 4) as usize);
    let (chunks, _) = pix.data().as_chunks::<4>();
    for px in chunks {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        if a == 0 || a == 255 {
            out.extend_from_slice(&[r, g, b, a]);
        } else {
            let un = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)) as u8;
            out.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
    out
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
        self.poll_preview(ctx);
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

        egui::SidePanel::right("settings_panel")
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Map");
                egui::ComboBox::from_label("Background")
                    .selected_text(format!("{:?}", self.background))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.background,
                            BackgroundImageType::Render,
                            "Render",
                        );
                        ui.selectable_value(
                            &mut self.background,
                            BackgroundImageType::Menu,
                            "Menu",
                        );
                        ui.selectable_value(
                            &mut self.background,
                            BackgroundImageType::Military,
                            "Military",
                        );
                        ui.selectable_value(
                            &mut self.background,
                            BackgroundImageType::None,
                            "None",
                        );
                    });
                ui.add(egui::Slider::new(&mut self.brightness, 0.1..=2.0).text("Brightness"));
                ui.horizontal(|ui| {
                    ui.label("Title:");
                    ui.text_edit_singleline(&mut self.title);
                });
                egui::ComboBox::from_label("Legend")
                    .selected_text(format!("{:?}", self.legend_style))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.legend_style,
                            LegendStyle::Extended,
                            "Extended",
                        );
                        ui.selectable_value(
                            &mut self.legend_style,
                            LegendStyle::Compact,
                            "Compact",
                        );
                        ui.selectable_value(&mut self.legend_style, LegendStyle::Hidden, "Hidden");
                    });
                egui::ComboBox::from_label("Legend align")
                    .selected_text(format!(
                        "{:?} / {:?}",
                        self.legend_halign, self.legend_valign
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.legend_halign,
                            LegendHorizontalAlignment::Left,
                            "Left",
                        );
                        ui.selectable_value(
                            &mut self.legend_halign,
                            LegendHorizontalAlignment::Right,
                            "Right",
                        );
                        ui.selectable_value(
                            &mut self.legend_valign,
                            LegendVerticalAlignment::Top,
                            "Top",
                        );
                        ui.selectable_value(
                            &mut self.legend_valign,
                            LegendVerticalAlignment::Center,
                            "Center",
                        );
                        ui.selectable_value(
                            &mut self.legend_valign,
                            LegendVerticalAlignment::Bottom,
                            "Bottom",
                        );
                    });
                egui::ComboBox::from_label("Compass")
                    .selected_text(format!("{:?}", self.compass))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.compass, CompassStyle::Off, "Off");
                        ui.selectable_value(
                            &mut self.compass,
                            CompassStyle::WhenUseful,
                            "WhenUseful",
                        );
                        ui.selectable_value(&mut self.compass, CompassStyle::Always, "Always");
                    });
                ui.checkbox(&mut self.show_grid, "Coordinate grid");
                ui.checkbox(&mut self.show_markers, "Map markers");
                ui.checkbox(&mut self.show_infestations, "Infestations");
                ui.separator();
                ui.heading("Plot");
                // Standard-locked until the modes plan: the other modes are
                // visible but disabled, with no code paths behind them.
                egui::ComboBox::from_label("Mode")
                    .selected_text(format!("{:?}", self.plot_mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.plot_mode, PlotMode::Standard, "Standard");
                        for soon in ["Topographic (soon)", "Heatmap (soon)", "Cluster (soon)"] {
                            ui.add_enabled(false, egui::Button::selectable(false, soon));
                        }
                    });
                ui.label(format!("Plot size: {}", self.plot_style.size));
                ui.horizontal_wrapped(|ui| {
                    ui.label("Palette:");
                    for c in &self.plot_style.palette {
                        ui.colored_label(egui::Color32::from_rgb(c.r, c.g, c.b), "\u{25a0}");
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
            ui.heading("Preview");
            let rendering = matches!(self.preview, PreviewState::Rendering);
            if ui
                .add_enabled(!rendering, egui::Button::new("Render preview"))
                .clicked()
            {
                self.start_preview();
            }
            match &self.preview {
                PreviewState::Idle => {
                    ui.label("Preview idle. Add items, then Render preview.");
                }
                PreviewState::Rendering => {
                    ui.label("Rendering...");
                }
                PreviewState::Ready(tex) => {
                    ui.label("Preview ready.");
                    let avail = ui.available_size();
                    ui.add(egui::Image::new(tex).fit_to_exact_size(avail));
                }
                PreviewState::Failed(e) => {
                    ui.label(format!("Preview failed: {e}"));
                }
            }
            ui.label("Export panel (Task 5): pending");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mappalachia_core::db::all_spaces;
    use std::path::Path;

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

    /// Task 3 RED: settings state mirroring the C# defaults
    /// (`Settings/MapSettings.cs`, `PlotSettings.cs`,
    /// `PlotStyleSettings.cs`).
    fn settings_app() -> App {
        let space = Space::new(
            0x0000_0001,
            "APPALACHIA",
            "Appalachia",
            true,
            true,
            0.0,
            0.0,
            100000.0,
            0.0,
        );
        App::from_parts(None, None, vec![space])
    }

    #[test]
    fn settings_defaults_match_csharp() {
        let app = settings_app();
        assert_eq!(app.background, BackgroundImageType::Menu);
        assert_eq!(app.brightness, 0.5);
        assert!(app.title.is_empty());
        assert_eq!(app.legend_style, LegendStyle::Compact);
        assert_eq!(app.legend_halign, LegendHorizontalAlignment::Right);
        assert_eq!(app.legend_valign, LegendVerticalAlignment::Center);
        assert_eq!(app.compass, CompassStyle::WhenUseful);
        assert!(!app.show_grid);
        assert!(!app.show_markers);
        assert!(!app.show_infestations);
        assert_eq!(app.plot_style.size, 50);
        assert_eq!(app.plot_style.palette.len(), 8);
        assert_eq!(app.plot_mode, PlotMode::Standard);
    }

    #[test]
    fn draw_settings_conversion_matches_csharp_and_clamps_brightness() {
        let mut app = settings_app();
        app.title = "Nuka map".to_string();
        app.brightness = 9.0;
        app.show_grid = true;
        let ds = app.draw_settings().expect("spaces present");
        assert_eq!(ds.map.background_image, BackgroundImageType::Menu);
        assert_eq!(ds.plot.mode, PlotMode::Standard);
        assert_eq!(ds.map.brightness, 2.0);
        assert_eq!(ds.map.title, "Nuka map");
        assert!(ds.map.show_coordinate_grid);
        assert_eq!(ds.space, app.spaces[app.space_idx]);
        app.brightness = -3.0;
        assert_eq!(app.draw_settings().unwrap().map.brightness, 0.1);
    }

    #[test]
    fn draw_settings_none_without_spaces() {
        let app = App::from_parts(None, None, Vec::new());
        assert!(app.draw_settings().is_none());
    }

    /// Task 4 RED: background-thread preview.
    ///
    /// Requires `MAPPALACHIA_ASSETS` + `MAPPALACHIA_DB`; SKIPs when either
    /// is unset or missing (same skip-pattern as the render golden tests).
    fn preview_app_with_item() -> Option<App> {
        let (db_path, assets_root) = {
            let _guard = ENV_LOCK.lock().unwrap();
            (
                std::env::var("MAPPALACHIA_DB").ok(),
                std::env::var("MAPPALACHIA_ASSETS").ok(),
            )
        };
        let (db_path, assets_root) = match (db_path, assets_root) {
            (Some(d), Some(a)) if Path::new(&d).exists() && Path::new(&a).is_dir() => (d, a),
            _ => {
                eprintln!("SKIP: set MAPPALACHIA_ASSETS and MAPPALACHIA_DB");
                return None;
            }
        };
        let conn = db::open_readonly(&db_path).expect("test DB opens");
        let spaces = all_spaces(&conn).expect("spaces load");
        let mut app = App::from_parts(Some(db_path), Some(assets_root), spaces);
        app.search_term = "Nuka Cola".to_string();
        app.run_search().expect("search succeeds");
        assert!(
            !app.results.is_empty(),
            "expected rows for 'Nuka Cola' in APPALACHIA"
        );
        app.checked[0] = true;
        app.add_selected();
        assert_eq!(app.items.len(), 1);
        Some(app)
    }

    #[test]
    fn preview_renders_1_item_4096_and_nonempty() {
        let Some(app) = preview_app_with_item() else {
            return;
        };
        let ds = app.draw_settings().expect("spaces present");
        let bytes = render_preview_bytes(
            app.db_path.as_deref().expect("db path"),
            app.assets_root.as_deref().expect("assets root"),
            ds,
            app.items.clone(),
        )
        .expect("render succeeds");
        assert_eq!(
            bytes.len(),
            (MAP_IMAGE_RESOLUTION as usize) * (MAP_IMAGE_RESOLUTION as usize) * 4
        );
        assert!(
            bytes.iter().any(|&b| b != 0),
            "rendered 1-item Standard pixmap must be non-empty"
        );
    }

    #[test]
    fn preview_channel_round_trip_delivers_bytes() {
        let (tx, rx) = std::sync::mpsc::channel::<PreviewResult>();
        let payload = vec![1u8, 2, 3, 4];
        std::thread::spawn(move || {
            let _ = tx.send(Ok(payload));
        })
        .join()
        .expect("worker joins");
        let got = rx.try_recv().expect("payload delivered");
        assert_eq!(got.expect("ok payload"), vec![1u8, 2, 3, 4]);
    }

    #[test]
    fn preview_without_envs_fails_without_spawning() {
        let mut app = App::from_parts(None, None, Vec::new());
        app.start_preview();
        assert!(matches!(app.preview, PreviewState::Failed(_)));
        assert!(app.preview_rx.is_none());
    }
}
