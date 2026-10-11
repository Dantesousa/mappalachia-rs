use std::path::{Path, PathBuf};

use mappalachia_core::{
    common::MAP_IMAGE_RESOLUTION,
    db,
    enums::{LockLevel, Signature},
    model::{EntityKind, Space},
    recipe::{load_recipe_file, MapSettingsStub, PlotSettingsStub, Recipe},
    search::{search, Grouped, SearchParams, SearchResult},
};
use mappalachia_render::{
    assets::Assets,
    color::Rgba,
    draw::{draw, DrawInput, Plottable},
    icon::PlotIcon,
    settings::{
        map_from_value, plot_from_value, BackgroundImageType, CompassStyle,
        LegendHorizontalAlignment, LegendStyle, LegendVerticalAlignment, MapSettings, PlotMode,
        PlotSettings, PlotStyleSettings,
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
    /// Cache of [`App::next_available_group`] (C# max+1 semantics),
    /// refreshed after every items mutation.
    pub next_group: usize,
    /// Map background (`MapSettings.BackgroundImage`; C# default `Menu`).
    pub background: BackgroundImageType,
    /// Map brightness (`MapSettings.Brightness`; setter clamps to
    /// 0.1–2.0). Deliberate product deviation: fresh UI state starts at
    /// full brightness `1.0`, not the C# default `0.5` (see
    /// `render::settings::default_brightness`).
    pub brightness: f32,
    /// Map title (`MapSettings.Title`; C# default `""`).
    pub title: String,
    /// Export/recipe filename stem (no extension), editable in the bottom
    /// panel. Defaults to the recommended name (title, else
    /// `Mappalachia Map of {space}`), sanitized like
    /// `FileIO.GetRecommendedMapFileName`.
    pub filename: String,
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
    /// Pan/zoom view into the preview texture, owned by the central
    /// `egui::Scene`. `Rect::ZERO` means "fit to panel" (Scene auto-fits
    /// an invalid rect); the Reset-view button restores it. Never touches
    /// the Render/texture flow — it only re-frames the uploaded texture.
    pub scene_rect: egui::Rect,
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
        let (db_path, assets_root) = match (
            std::env::var("MAPPALACHIA_DB").ok(),
            std::env::var("MAPPALACHIA_ASSETS").ok(),
        ) {
            (Some(db), Some(assets)) => (Some(db), Some(assets)),
            _ => match mappalachia_render::assets::exe_adjacent() {
                Some((db, assets)) => (
                    Some(db.to_string_lossy().into_owned()),
                    Some(assets.to_string_lossy().into_owned()),
                ),
                None => (None, None),
            },
        };
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
            format!("Missing env: {}. Set and restart, or place data/ img/ font/ next to the executable.", missing.join(", "))
        };
        let space_idx = spaces.iter().position(|s| s.is_appalachia()).unwrap_or(0);
        let filename = recommended_stem("", spaces.get(space_idx));
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
            // Deliberate product deviation from the C# 0.5 default (see
            // `render::settings::default_brightness`).
            brightness: 1.0,
            title: String::new(),
            filename,
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
            scene_rect: egui::Rect::ZERO,
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
                if bytes.len() != side * side * 4 {
                    self.preview = PreviewState::Failed(format!(
                        "Preview bytes corrupt ({} != {}).",
                        bytes.len(),
                        side * side * 4
                    ));
                    self.status = "Preview bytes corrupt.".to_string();
                    return;
                }
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
    /// group with the current plot palette/size, stamp the group onto the
    /// result, and push a `PlotItem`. Drops any stale preview.
    pub fn add_selected(&mut self) {
        let style = self.plot_style.clone();
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
            // C# non-grouped path calls `GetNextAvailableLegendGroup()`
            // per item, so groups stay dense after removals.
            let group = self.next_available_group();
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
        self.next_group = self.next_available_group();
        self.status = format!("Added {added} item(s); {} on map.", self.items.len());
        self.invalidate_preview();
    }

    /// C# `FormMain.GetNextAvailableLegendGroup` (`FormMain.cs:1490-1498`):
    /// max assigned group + 1 (0 when empty), so removing the highest
    /// group frees it for reuse. `next_group` is a cache of this value,
    /// refreshed after every mutation — not a monotonic counter.
    pub fn next_available_group(&self) -> usize {
        self.items
            .iter()
            .map(|it| it.legend_group)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0)
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
        self.next_group = self.next_available_group();
        self.status = format!(
            "{} item(s) on map; {} result(s).",
            self.items.len(),
            self.results.len()
        );
        self.invalidate_preview();
    }

    /// Recommended filename stem for the current map (mirrors
    /// `FileIO.GetRecommendedMapFileName`): the sanitized title, else
    /// `Mappalachia Map of {space DisplayName}`, sanitized.
    pub fn recommended_filename(&self) -> String {
        recommended_stem(&self.title, self.spaces.get(self.space_idx))
    }

    /// Filename stem for export/recipe files: the sanitized `filename`
    /// field, falling back to the recommended default when blank.
    pub fn effective_stem(&self) -> String {
        let clean = sanitize_filename(&self.filename);
        if clean.is_empty() {
            self.recommended_filename()
        } else {
            clean
        }
    }

    /// Mirrors `FileIO.GetImageFileTypeRecommendation` for the current UI
    /// state: JPEG for a worldspace with a background image, else PNG.
    /// `false` when no spaces are loaded (`export_map` then fails).
    pub fn export_uses_jpeg(&self) -> bool {
        match self.spaces.get(self.space_idx) {
            Some(space) => recommend_jpeg(space, self.background),
            None => false,
        }
    }

    /// Fixed output path (no file dialogs in v1 — `rfd` is OUT):
    /// `./<stem>.<ext>` in the process working directory.
    pub fn export_path(&self) -> PathBuf {
        let ext = if self.export_uses_jpeg() {
            "jpg"
        } else {
            "png"
        };
        PathBuf::from(format!("./{}.{}", self.effective_stem(), ext))
    }

    /// Fixed recipe path (no dialogs in v1): `./<stem>.json`, sharing the
    /// export stem so one filename field drives both.
    pub fn recipe_path(&self) -> PathBuf {
        PathBuf::from(format!("./{}.json", self.effective_stem()))
    }

    /// Render the current map synchronously and save it at
    /// [`App::export_path`], reusing the preview worker body
    /// ([`render_preview_bytes`]) for the pixels. v1 blocks the UI thread
    /// (a 4096² render takes seconds); a threaded export is a later-task
    /// option. Reports via `status`.
    pub fn export_map(&mut self) -> Result<PathBuf, String> {
        let settings = self
            .draw_settings()
            .ok_or_else(|| "Cannot export: no spaces loaded.".to_string())?;
        let (db_path, assets_root) = match (self.db_path.clone(), self.assets_root.clone()) {
            (Some(d), Some(a)) => (d, a),
            _ => {
                return Err(
                    "Cannot export: MAPPALACHIA_DB / MAPPALACHIA_ASSETS not set.".to_string(),
                );
            }
        };
        let bytes = render_preview_bytes(&db_path, &assets_root, settings, self.items.clone())?;
        let side = MAP_IMAGE_RESOLUTION as u32;
        let rgba = image::RgbaImage::from_raw(side, side, bytes)
            .ok_or_else(|| "Export bytes corrupt.".to_string())?;
        let path = self.export_path();
        if self.export_uses_jpeg() {
            // JPEG has no alpha: flatten over black like the CLI exporter.
            let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
            for (x, y, px) in rgba.enumerate_pixels() {
                let a = f32::from(px[3]) / 255.0;
                rgb.put_pixel(
                    x,
                    y,
                    image::Rgb([
                        (f32::from(px[0]) * a).round() as u8,
                        (f32::from(px[1]) * a).round() as u8,
                        (f32::from(px[2]) * a).round() as u8,
                    ]),
                );
            }
            rgb.save(&path).map_err(|e| e.to_string())?;
        } else {
            rgba.save(&path).map_err(|e| e.to_string())?;
        }
        self.status = format!("Exported {}.", path.display());
        Ok(path)
    }

    /// Save the current map as a recipe at `path` (mirrors
    /// `FormMain.Map_SaveAsRecipe_Click` minus the dialog): the current
    /// full-DB space, the typed settings serialized to the PascalCase
    /// stubs, and the items with their groups + legend text stamped
    /// (icons stay fresh-assigned on open, so no `PlotIcon` is stored).
    pub fn save_recipe_to(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let settings = self
            .draw_settings()
            .ok_or_else(|| "Cannot save recipe: no spaces loaded.".to_string())?;
        let map_value = serde_json::to_value(&settings.map).map_err(|e| e.to_string())?;
        let plot_value = serde_json::to_value(&settings.plot).map_err(|e| e.to_string())?;
        Recipe {
            space: settings.space,
            map_settings: MapSettingsStub::from(map_value),
            plot_settings: PlotSettingsStub::from(plot_value),
            items_to_plot: self.items.iter().map(plot_item_to_result).collect(),
        }
        .save_to_file(path)
    }

    /// Open a recipe file (mirrors `FormMain.Map_OpenRecipe_Click` +
    /// `Recipe.AsSettings`): resolve the recipe space and every item space
    /// via `populate_from_data`, apply the typed map/plot settings to the
    /// panels, and rebuild the items with fresh icons at their stored
    /// groups (like the C# CLI path, which plots stored groups with
    /// re-derived icons). Resets `next_group` and `filename` to the opened
    /// map. Reports via `status`.
    pub fn open_recipe_from(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let recipe = load_recipe_file(&path)?;
        let db_path = self
            .db_path
            .clone()
            .ok_or_else(|| "MAPPALACHIA_DB is not set".to_string())?;
        let conn = db::open_readonly(&db_path).map_err(|e| format!("DB open failed: {e}"))?;
        let space = db::populate_from_data(&conn, &recipe.space).map_err(|e| {
            format!(
                "recipe space {} not in database: {e}",
                recipe.space.editor_id()
            )
        })?;
        let map = map_from_value(recipe.map_settings.raw())
            .map_err(|e| format!("recipe map settings: {e}"))?;
        let plot = plot_from_value(recipe.plot_settings.raw())
            .map_err(|e| format!("recipe plot settings: {e}"))?;
        match self.spaces.iter().position(|s| *s == space) {
            Some(i) => self.space_idx = i,
            None => {
                self.spaces.push(space);
                self.space_idx = self.spaces.len() - 1;
            }
        }
        self.background = map.background_image;
        self.brightness = map.brightness;
        self.title = map.title.clone();
        self.legend_style = map.legend_style;
        self.legend_halign = map.legend_horizontal_alignment;
        self.legend_valign = map.legend_vertical_alignment;
        self.compass = map.compass_style;
        self.show_grid = map.show_coordinate_grid;
        // v1 has a single markers toggle driving both C# flags.
        self.show_markers = map.map_marker_icons || map.map_marker_labels;
        self.show_infestations = map.show_infestations;
        // Standard-locked until the modes plan: the v1 combo cannot
        // represent the other modes, so clamp (a recipe saved elsewhere may
        // store Topographic/Heatmap/Cluster, which `draw` would reject).
        self.plot_mode = PlotMode::Standard;
        self.plot_style = plot.plot_style_settings.clone();
        let include_count = plot.include_count_in_legend;
        let style = self.plot_style.clone();
        let mut items = Vec::with_capacity(recipe.items_to_plot.len());
        for result in recipe.items_to_plot {
            let resolved = db::populate_from_data(&conn, result.space()).map_err(|e| {
                format!(
                    "item space {} not in database: {e}",
                    result.space().editor_id()
                )
            })?;
            let mut owned = result;
            match &mut owned {
                SearchResult::Grouped(g) => g.space = resolved,
                SearchResult::Singular { inner, .. } => inner.space = resolved,
            }
            let group = grouped_of(&owned).legend_group.max(0) as usize;
            let legend = grouped_of(&owned)
                .legend_text
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| grouped_of(&owned).legend_text(include_count));
            match &mut owned {
                SearchResult::Grouped(g) => g.legend_text = Some(legend.clone()),
                SearchResult::Singular { inner, .. } => inner.legend_text = Some(legend.clone()),
            }
            let icon = PlotIcon::assign(
                group,
                &style.palette,
                style.size as u32,
                is_region_or_location(&owned),
                PLOT_ICON_COUNT,
            );
            items.push(PlotItem {
                result: owned,
                legend_group: group,
                icon,
                legend,
            });
        }
        let n = items.len();
        self.items = items;
        self.next_group = self.next_available_group();
        self.filename = self.recommended_filename();
        self.status = format!("Opened recipe ({} item(s)).", n);
        self.invalidate_preview();
        Ok(())
    }
}

/// Preview invalidation + settings fingerprint (stale-preview fixes).
impl App {
    /// Drop any rendered/stale preview: the panel must never show a map
    /// that no longer matches state. Called after every mutation that
    /// affects the render (items, opened recipe, settings edits).
    fn invalidate_preview(&mut self) {
        self.preview_rx = None;
        self.preview = PreviewState::Idle;
    }

    /// Snapshot of every panel field that feeds [`App::draw_settings`].
    /// The settings widgets write these fields directly (no setters to
    /// hook), so `update` compares this across the panels and drops a
    /// preview made stale by any settings edit.
    fn settings_fingerprint(
        &self,
    ) -> (
        usize,
        BackgroundImageType,
        f32,
        String,
        LegendStyle,
        LegendHorizontalAlignment,
        LegendVerticalAlignment,
        CompassStyle,
        bool,
        bool,
        bool,
        PlotMode,
    ) {
        (
            self.space_idx,
            self.background,
            self.brightness,
            self.title.clone(),
            self.legend_style,
            self.legend_halign,
            self.legend_valign,
            self.compass,
            self.show_grid,
            self.show_markers,
            self.show_infestations,
            self.plot_mode,
        )
    }
}

/// Mirrors `FileIO.SanitizeForFileName` (plus the Windows invalid-char set
/// so recipes stay portable): strip invalid filename chars, trim
/// whitespace, drop trailing periods.
pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|c| {
            !matches!(
                c,
                '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0'
            ) && !c.is_control()
        })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string()
}

/// Filename-stem counterpart of [`App::recommended_filename`], usable
/// before an [`App`] exists (e.g. in `from_parts`).
fn recommended_stem(title: &str, space: Option<&Space>) -> String {
    let clean = sanitize_filename(title);
    if !clean.is_empty() {
        return clean;
    }
    match space {
        Some(s) => sanitize_filename(&format!("Mappalachia Map of {}", s.display_name())),
        None => "Mappalachia Map".to_string(),
    }
}

/// Mirrors `FileIO.GetImageFileTypeRecommendation`: JPEG for a worldspace
/// with a background image, PNG otherwise (cell spaces and
/// `BackgroundImageType::None` always PNG).
pub fn recommend_jpeg(space: &Space, background: BackgroundImageType) -> bool {
    space.is_worldspace && background != BackgroundImageType::None
}

/// Convert an items-to-plot entry back to a recipe `SearchResult`,
/// stamping the assigned group + legend text.
fn plot_item_to_result(item: &PlotItem) -> SearchResult {
    let mut result = item.result.clone();
    let grouped = match &mut result {
        SearchResult::Grouped(g) => g,
        SearchResult::Singular { inner, .. } => inner,
    };
    grouped.legend_group = item.legend_group as i32;
    grouped.legend_text = Some(item.legend.clone());
    result
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
        // Snapshot before the panels: any settings edit below stales the
        // preview (compared again after the settings panel).
        let settings_before = self.settings_fingerprint();
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

        // Any settings edit above stales the preview. The widgets write the
        // fields directly (no setters to hook), so diff the fingerprint.
        if self.settings_fingerprint() != settings_before {
            self.invalidate_preview();
        }

        egui::TopBottomPanel::bottom("export_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Filename:");
                ui.text_edit_singleline(&mut self.filename);
                if ui.button("Export").clicked() {
                    if let Err(e) = self.export_map() {
                        self.status = e;
                    }
                }
                if ui.button("Save Recipe").clicked() {
                    let path = self.recipe_path();
                    match self.save_recipe_to(&path) {
                        Ok(()) => self.status = format!("Saved recipe {}.", path.display()),
                        Err(e) => self.status = e,
                    }
                }
                if ui.button("Open Recipe").clicked() {
                    let path = self.recipe_path();
                    if let Err(e) = self.open_recipe_from(&path) {
                        self.status = e;
                    }
                }
            });
            ui.label(&self.status);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Mappalachia RS");
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
                    ui.horizontal(|ui| {
                        ui.label("Preview ready. Drag to pan, scroll to zoom.");
                        // Restores the fit-to-panel view: Scene auto-fits an
                        // invalid rect, so resetting to ZERO re-fits. This
                        // only re-frames the uploaded texture; the
                        // Render-button/texture flow is untouched.
                        if ui.button("Reset view").clicked() {
                            self.scene_rect = egui::Rect::ZERO;
                        }
                    });
                    let native = tex.size_vec2();
                    egui::Scene::new()
                        .zoom_range(egui::Rangef::new(0.0, 8.0))
                        .max_inner_size(native + egui::Vec2::splat(32.0))
                        .show(ui, &mut self.scene_rect, |ui| {
                            ui.add(egui::Image::new(tex).fit_to_exact_size(native));
                        });
                }
                PreviewState::Failed(e) => {
                    ui.label(format!("Preview failed: {e}"));
                }
            }
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

    #[test]
    fn add_selected_resets_stale_preview_to_idle() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let item = test_plot_item(&space, 0);
        app.results = vec![item.result.clone()];
        app.checked = vec![true];
        app.preview = PreviewState::Rendering;
        let (_tx, rx) = std::sync::mpsc::channel::<PreviewResult>();
        app.preview_rx = Some(rx);
        app.add_selected();
        assert_eq!(app.items.len(), 1);
        assert!(
            matches!(app.preview, PreviewState::Idle),
            "add must reset a stale preview to Idle"
        );
        assert!(app.preview_rx.is_none());
    }

    #[test]
    fn remove_selected_resets_stale_preview_to_idle() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let item = test_plot_item(&space, 0);
        app.results = vec![item.result.clone()];
        app.checked = vec![true];
        app.items = vec![item];
        app.preview = PreviewState::Failed("stale".to_string());
        app.remove_selected();
        assert!(app.items.is_empty());
        assert!(
            matches!(app.preview, PreviewState::Idle),
            "remove must reset a stale preview to Idle"
        );
        assert!(app.preview_rx.is_none());
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
        // Deliberate product deviation from the C# 0.5 (see
        // `render::settings::default_brightness`).
        assert_eq!(app.brightness, 1.0);
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
    fn default_brightness_is_full_and_scene_starts_fit_to_panel() {
        // Deliberate product deviation from the C# 0.5 default: fresh UI
        // state renders at full brightness.
        let app = settings_app();
        assert_eq!(app.brightness, 1.0);
        // The preview Scene starts fit-to-panel (`Rect::ZERO` auto-fits);
        // the Reset-view button restores exactly this value.
        assert_eq!(app.scene_rect, egui::Rect::ZERO);
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

    /// Task 5 RED: export filename/format/recipe behaviours.
    use mappalachia_core::model::Entity;
    use mappalachia_core::recipe::load_recipe_file;

    /// Minimal items-to-plot entry for DB-free Task 5 tests.
    fn test_plot_item(space: &Space, group: usize) -> PlotItem {
        let entity = Entity::new(
            0x1000 + group as u32,
            format!("EDID{group}"),
            "Display".to_string(),
            Signature::Misc,
            EntityKind::Standard,
        );
        let grouped = Grouped {
            entity,
            space: space.clone(),
            count: 1,
            spawn_weight: 1.0,
            label: String::new(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: group as i32,
            legend_text: None,
            plot_icon: None,
        };
        let style = PlotStyleSettings::default();
        PlotItem {
            result: SearchResult::Grouped(grouped),
            legend_group: group,
            icon: PlotIcon::assign(
                group,
                &style.palette,
                style.size as u32,
                false,
                PLOT_ICON_COUNT,
            ),
            legend: format!("EDID{group}"),
        }
    }

    #[test]
    fn format_rule_jpeg_for_appalachia_render() {
        let mut app = settings_app();
        app.background = BackgroundImageType::Render;
        let space = app.spaces[app.space_idx].clone();
        assert!(space.is_worldspace);
        assert!(recommend_jpeg(&space, app.background));
    }

    #[test]
    fn format_rule_png_for_cell_space_or_no_background() {
        let cell = Space::new(
            0x0000_0010,
            "CELL0001",
            "Test Cell",
            false,
            false,
            0.0,
            0.0,
            1000.0,
            0.0,
        );
        assert!(!recommend_jpeg(&cell, BackgroundImageType::Render));
        let mut app = settings_app();
        app.background = BackgroundImageType::None;
        let space = app.spaces[app.space_idx].clone();
        assert!(space.is_worldspace);
        assert!(!recommend_jpeg(&space, app.background));
    }

    #[test]
    fn sanitize_strips_slash_and_trims() {
        assert_eq!(sanitize_filename("  a/b  "), "ab");
        assert_eq!(
            sanitize_filename("Mappalachia Map of Appalachia"),
            "Mappalachia Map of Appalachia"
        );
        assert_eq!(sanitize_filename("map."), "map");
    }

    #[test]
    fn recommended_filename_falls_back_to_space() {
        let app = settings_app();
        assert_eq!(app.filename, "Mappalachia Map of Appalachia");
        let mut titled = settings_app();
        titled.title = "  Nuka/map  ".to_string();
        assert_eq!(titled.recommended_filename(), "Nukamap");
    }

    #[test]
    fn recipe_save_load_round_trip_preserves_items() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        app.items.push(test_plot_item(&space, 0));
        app.items.push(test_plot_item(&space, 1));
        let path = std::env::temp_dir().join("mappalachia-egui-task5-recipe.json");
        app.save_recipe_to(&path).expect("save succeeds");
        let recipe = load_recipe_file(&path).expect("load succeeds");
        assert_eq!(recipe.items_to_plot.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_recipe_round_trip_restores_items_and_settings() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.run_search().expect("search succeeds");
        assert!(
            app.results.len() >= 2,
            "need 2+ rows to check post-open adds, got {}",
            app.results.len()
        );
        app.checked[0] = true;
        app.add_selected();
        assert_eq!(app.items.len(), 1);
        app.title = "Task5 Open".to_string();
        // Non-default plot style: items added after opening this recipe
        // must use the recipe palette/size, not the default.
        app.plot_style.size = 30;
        app.plot_style.palette[0] = Rgba::new(1, 2, 3, 255);
        app.plot_style.palette[1] = Rgba::new(4, 5, 6, 255);
        let path = std::env::temp_dir().join("mappalachia-egui-task5-open.json");
        app.save_recipe_to(&path).expect("save succeeds");
        let db_path = app.db_path.clone();
        let spaces = app.spaces.clone();
        let mut fresh = App::from_parts(db_path, None, spaces);
        // A stale preview must not survive opening a recipe.
        fresh.preview = PreviewState::Failed("stale".to_string());
        fresh.open_recipe_from(&path).expect("open succeeds");
        assert!(
            matches!(fresh.preview, PreviewState::Idle),
            "open must reset a stale preview to Idle"
        );
        assert!(fresh.preview_rx.is_none());
        assert_eq!(fresh.items.len(), 1);
        assert_eq!(fresh.title, "Task5 Open");
        assert_eq!(fresh.next_group, 1);
        assert_eq!(fresh.items[0].legend_group, 0);
        assert!(fresh.spaces[fresh.space_idx].is_appalachia());
        assert!(fresh.status.contains("Opened recipe"));
        assert_eq!(fresh.plot_style.size, 30);
        assert_eq!(fresh.plot_style.palette[0], Rgba::new(1, 2, 3, 255));
        // A newly added item uses the recipe style, not the default.
        fresh.results = app.results.clone();
        fresh.checked = vec![false; fresh.results.len()];
        fresh.checked[1] = true;
        fresh.add_selected();
        assert_eq!(fresh.items.len(), 2);
        assert_eq!(fresh.items[1].legend_group, 1);
        assert_eq!(fresh.items[1].icon.size, 30);
        assert_eq!(fresh.items[1].icon.color, Rgba::new(4, 5, 6, 255));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_recipe_clamps_plot_mode_to_standard() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.run_search().expect("search succeeds");
        app.checked[0] = true;
        app.add_selected();
        assert_eq!(app.items.len(), 1);
        let path = std::env::temp_dir().join("mappalachia-egui-task5-mode.json");
        app.save_recipe_to(&path).expect("save succeeds");
        // Rewrite the stored mode to a mode the v1 combo cannot represent.
        let raw = std::fs::read_to_string(&path).expect("recipe readable");
        let mut value: serde_json::Value = serde_json::from_str(&raw).expect("recipe is JSON");
        value["PlotSettings"]["Mode"] = serde_json::Value::String("Topographic".to_string());
        std::fs::write(
            &path,
            serde_json::to_string(&value).expect("recipe serializes"),
        )
        .expect("recipe writable");
        let mut fresh = App::from_parts(app.db_path.clone(), None, app.spaces.clone());
        fresh.open_recipe_from(&path).expect("open succeeds");
        assert_eq!(
            fresh.plot_mode,
            PlotMode::Standard,
            "open must clamp plot_mode to Standard until the modes plan"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn remove_selected_recomputes_next_group_like_csharp() {
        // C# `GetNextAvailableLegendGroup` is max+1, so removing the
        // highest group frees it for reuse (monotonic would keep 2).
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let item0 = test_plot_item(&space, 0);
        let item1 = test_plot_item(&space, 1);
        app.results = vec![item0.result.clone(), item1.result.clone()];
        app.checked = vec![false, true];
        app.items = vec![item0, item1];
        app.next_group = 2;
        app.remove_selected();
        assert_eq!(app.items.len(), 1);
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.next_group, 1);
    }
}
