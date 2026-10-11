use std::path::{Path, PathBuf};

use mappalachia_core::{
    common::MAP_IMAGE_RESOLUTION,
    db,
    enums::{LockLevel, Signature},
    instances,
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

/// One merged search-results row: raw `core::search` hits that render
/// identically (same entity + space + `legend_text(include_count = false)`;
/// hidden-field differences like quantity stay in `variants`) are grouped.
/// `display` is the first hit with the summed count; `variants` keeps every
/// raw hit so plotting loses no instances. Never merges across spaces (the
/// space id is part of the key).
#[derive(Debug, Clone)]
pub struct ResultRow {
    pub display: SearchResult,
    pub variants: Vec<SearchResult>,
}

impl ResultRow {
    /// Summed count over all variants (mirrors the row's `(xN)` badge).
    pub fn count(&self) -> i64 {
        self.variants.iter().map(|v| v.count()).sum()
    }
    #[allow(dead_code)]
    pub fn editor_id(&self) -> &str {
        self.display.editor_id()
    }
    #[allow(dead_code)]
    pub fn form_id(&self) -> u32 {
        self.display.form_id()
    }
    pub fn space(&self) -> &Space {
        self.display.space()
    }
    pub fn entity_kind(&self) -> &EntityKind {
        self.display.entity_kind()
    }
}

/// One entry of the items-to-plot list: a merged result row plus its
/// freshly-assigned legend group, icon and legend text
/// (mirrors `FormMain.ButtonAddToMap_Click`, non-grouped path).
/// `variants` carries every raw hit so the preview worker can expand one
/// `Plottable` per variant (sharing group/icon/legend) with no loss.
#[derive(Debug, Clone)]
pub struct PlotItem {
    pub result: SearchResult,
    pub variants: Vec<SearchResult>,
    pub legend_group: usize,
    pub icon: PlotIcon,
    pub legend: String,
}

/// One connected-interior hit: a CELL space reachable via a teleporter
/// from the current space that contains instances of the searched item.
/// `space_idx` indexes [`App::spaces`] so a row click can switch to it.
#[derive(Debug, Clone)]
pub struct InteriorHit {
    pub space_idx: usize,
    pub display_name: String,
    pub count: usize,
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
    /// C# `SearchSettings.SearchInAllSpaces` parity: when true the search
    /// is unscoped (`space_form_id = None`) so components like antiseptic
    /// are found outside the current space; when false (default) the
    /// search is scoped to `spaces[space_idx]`.
    pub search_all_spaces: bool,
    /// Last search results (one row per merged display group).
    pub results: Vec<ResultRow>,
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
    /// Auto-render toggle (default ON): when set, [`App::maybe_auto_render`]
    /// spawns a preview on the next frame after any render-affecting
    /// mutation. The Render button always works manually when off.
    pub auto_render: bool,
    /// True when render-affecting state changed since the last preview
    /// spawn. Set by [`App::invalidate_preview`]; cleared on spawn by
    /// [`App::maybe_auto_render`], so a finished preview never re-triggers.
    /// The export/recipe filename field is deliberately excluded (it never
    /// invalidates).
    pub dirty: bool,
    /// Last interiors-finder hits (connected CELL spaces holding the
    /// searched item), refreshed by [`App::find_interiors`].
    pub interiors: Vec<InteriorHit>,
    /// Editor id of the item [`App::interiors`] was searched for.
    pub interiors_for: String,
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
            search_all_spaces: false,
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
            auto_render: true,
            dirty: false,
            interiors: Vec::new(),
            interiors_for: String::new(),
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
            space_form_id: if self.search_all_spaces {
                None
            } else {
                self.spaces.get(self.space_idx).map(|s| s.form_id())
            },
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
        if self.spaces.is_empty() && !self.search_all_spaces {
            return Err("Space list is empty; cannot scope search".to_string());
        }
        let params = self.search_params();
        let raw = search(&conn, &params).map_err(|e| format!("Search failed: {e}"))?;
        let rows = merge_results(raw);
        let n = rows.len();
        self.checked = vec![false; n];
        self.results = rows;
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
            let legend = result_legend_text(&self.results[i].display);
            let no_icon = is_region_or_location(&self.results[i].display);
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
            let mut display = self.results[i].display.clone();
            match &mut display {
                SearchResult::Grouped(g) => g.legend_group = group as i32,
                SearchResult::Singular { inner, .. } => inner.legend_group = group as i32,
            }
            let variants = self.results[i]
                .variants
                .iter()
                .map(|v| {
                    let mut stamped = v.clone();
                    match &mut stamped {
                        SearchResult::Grouped(g) => g.legend_group = group as i32,
                        SearchResult::Singular { inner, .. } => {
                            inner.legend_group = group as i32;
                        }
                    }
                    stamped
                })
                .collect();
            self.items.push(PlotItem {
                result: display,
                variants,
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
        let removed = doomed.len();
        if removed == 0 {
            self.status = format!(
                "Nothing checked; {} item(s) on map; {} result(s).",
                self.items.len(),
                self.results.len()
            );
            return;
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
            "Removed {removed} checked; {} item(s) on map; {} result(s).",
            self.items.len(),
            self.results.len()
        );
        self.invalidate_preview();
    }

    /// Header-checkbox state: true when there is at least one result and
    /// every result is checked. Checkbox-only state — does not affect the
    /// render, so it never marks dirty.
    pub fn all_checked(&self) -> bool {
        !self.checked.is_empty() && self.checked.iter().all(|&c| c)
    }

    /// Bulk (un)check every result (the Results header checkbox).
    /// Checkbox-only state — does not affect the render, so it never
    /// marks dirty.
    pub fn set_all_checked(&mut self, v: bool) {
        self.checked.fill(v);
    }

    /// Empty the results list (and its checkbox state), leaving the
    /// items-to-plot untouched. The render is unaffected, so this never
    /// marks dirty.
    pub fn clear_results(&mut self) {
        let n = self.results.len();
        self.results.clear();
        self.checked.clear();
        self.status = format!("Cleared {n} result(s).");
    }

    /// The item the interiors finder searches for: the first checked
    /// row's display result, else the first item-to-plot (kept simple by design).
    fn interiors_target(&self) -> Option<SearchResult> {
        if let Some(i) = self.checked.iter().position(|&c| c) {
            return self.results.get(i).map(|row| row.display.clone());
        }
        self.items.first().map(|it| it.result.clone())
    }

    /// Interiors finder (no C# equivalent — the C# connected-spaces logic
    /// in `Map.cs:DrawConnectingSpacePlots` only draws door markers onto
    /// the map): via the teleporters leaving the current space (deduped by
    /// `(space, teleportsTo)` like the C# path), list the connected CELL
    /// spaces containing instances of the first checked/added item, with
    /// per-space counts. Runs synchronously (queries are ms-scale).
    /// Records [`App::interiors`] for the left-panel section and reports
    /// the hit count via `status`. Never marks dirty (the render is
    /// unaffected until a row click switches space).
    pub fn find_interiors(&mut self) -> Result<(), String> {
        let result = self
            .interiors_target()
            .ok_or_else(|| "Nothing selected: check a result or add an item first.".to_string())?;
        let editor = result.editor_id().to_string();
        let path = self
            .db_path
            .clone()
            .ok_or_else(|| "MAPPALACHIA_DB is not set".to_string())?;
        let conn = db::open_readonly(&path).map_err(|e| format!("DB open failed: {e}"))?;
        let space = self
            .spaces
            .get(self.space_idx)
            .ok_or_else(|| "No current space.".to_string())?
            .clone();
        let teleporters = instances::teleporters(&conn, &space)
            .map_err(|e| format!("Teleporters failed: {e}"))?;
        // C# `DistinctBy(i => (i.Space, i.TeleportsTo))`, CELL targets only.
        let mut seen = std::collections::HashSet::new();
        let mut targets: Vec<Space> = Vec::new();
        for t in &teleporters {
            let dest = match &t.teleports_to {
                Some(s) => s.clone(),
                None => continue,
            };
            if dest.is_worldspace {
                continue;
            }
            if seen.insert((t.space.form_id(), dest.form_id())) {
                targets.push(dest);
            }
        }
        let mut hits = Vec::new();
        for dest in &targets {
            let count = instances::instances(&conn, result.clone(), dest)
                .map_err(|e| format!("Instances failed: {e}"))?
                .len();
            if count == 0 {
                continue;
            }
            if let Some(idx) = self
                .spaces
                .iter()
                .position(|s| s.form_id() == dest.form_id())
            {
                hits.push(InteriorHit {
                    space_idx: idx,
                    display_name: dest.display_name().to_string(),
                    count,
                });
            }
        }
        hits.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        let n = hits.len();
        self.interiors = hits;
        self.interiors_for = editor;
        self.status = format!("Found {n} interior(s) with instances.");
        Ok(())
    }

    /// Switch the current space to `interiors[i]` (a row click in the
    /// interiors section). Invalidates (marks dirty) since the space feeds
    /// the render; out-of-range indexes are a no-op.
    pub fn apply_interior(&mut self, i: usize) {
        let Some(hit) = self.interiors.get(i) else {
            return;
        };
        let (idx, name) = (hit.space_idx, hit.display_name.clone());
        self.space_idx = idx;
        self.invalidate_preview();
        self.status = format!("Switched to {name}.");
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
            items_to_plot: self.items.iter().flat_map(plot_item_to_results).collect(),
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
                result: owned.clone(),
                variants: vec![owned],
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
/// Snapshot of every panel field that feeds [`App::draw_settings`].
/// A struct (not a tuple: tuples stop implementing traits past 12 fields).
#[derive(Debug, Clone, PartialEq)]
struct SettingsFingerprint {
    space_idx: usize,
    background: BackgroundImageType,
    brightness: f32,
    title: String,
    legend_style: LegendStyle,
    legend_halign: LegendHorizontalAlignment,
    legend_valign: LegendVerticalAlignment,
    compass: CompassStyle,
    show_grid: bool,
    show_markers: bool,
    show_infestations: bool,
    plot_mode: PlotMode,
    plot_style: PlotStyleSettings,
}

impl App {
    /// Drop any rendered/stale preview: the panel must never show a map
    /// that no longer matches state. Called after every mutation that
    /// affects the render (items, opened recipe, settings edits). Also
    /// marks the state dirty so auto-render picks it up; the filename
    /// field never calls this (it does not affect the render).
    fn invalidate_preview(&mut self) {
        self.preview_rx = None;
        self.preview = PreviewState::Idle;
        self.dirty = true;
    }

    /// Auto-render hook, called once per frame from `update` after the
    /// settings-fingerprint diff: when the toggle is on and state mutated
    /// since the last spawn, start a preview unless one is already
    /// running. `dirty` is cleared on spawn (not on completion), so a
    /// finished (`Ready`) or failed preview never re-triggers a loop.
    pub fn maybe_auto_render(&mut self) {
        if !self.auto_render || !self.dirty {
            return;
        }
        if matches!(self.preview, PreviewState::Idle | PreviewState::Ready(_)) {
            self.dirty = false;
            self.start_preview();
        }
    }

    /// Snapshot of every panel field that feeds [`App::draw_settings`].
    /// The settings widgets write these fields directly (no setters to
    /// hook), so `update` compares this across the panels and drops a
    /// preview made stale by any settings edit.
    fn settings_fingerprint(&self) -> SettingsFingerprint {
        SettingsFingerprint {
            space_idx: self.space_idx,
            background: self.background,
            brightness: self.brightness,
            title: self.title.clone(),
            legend_style: self.legend_style,
            legend_halign: self.legend_halign,
            legend_valign: self.legend_valign,
            compass: self.compass,
            show_grid: self.show_grid,
            show_markers: self.show_markers,
            show_infestations: self.show_infestations,
            plot_mode: self.plot_mode,
            plot_style: self.plot_style.clone(),
        }
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

/// Convert an items-to-plot entry back to recipe `SearchResult`s, stamping
/// the assigned group + legend text onto every variant (one entry per
/// variant, so reopening loses no instances; each reopens as its own
/// single-variant item sharing the stored group).
fn plot_item_to_results(item: &PlotItem) -> Vec<SearchResult> {
    let stamp = |mut result: SearchResult| {
        let grouped = match &mut result {
            SearchResult::Grouped(g) => g,
            SearchResult::Singular { inner, .. } => inner,
        };
        grouped.legend_group = item.legend_group as i32;
        grouped.legend_text = Some(item.legend.clone());
        result
    };
    if item.variants.is_empty() {
        vec![stamp(item.result.clone())]
    } else {
        item.variants.iter().cloned().map(stamp).collect()
    }
}

/// Expand items-to-plot into one [`Plottable`] per variant, sharing the
/// item's group/icon/legend so no instances are lost.
fn expand_plot_items(items: &[PlotItem]) -> Vec<Plottable> {
    let mut out = Vec::new();
    for it in items {
        if it.variants.is_empty() {
            out.push(Plottable {
                result: it.result.clone(),
                legend_group: it.legend_group as i32,
                legend_text: it.legend.clone(),
                icon: it.icon,
            });
        } else {
            for v in &it.variants {
                out.push(Plottable {
                    result: v.clone(),
                    legend_group: it.legend_group as i32,
                    legend_text: it.legend.clone(),
                    icon: it.icon,
                });
            }
        }
    }
    out
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
    let plottables: Vec<Plottable> = expand_plot_items(&items);
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

/// Merge key for identical-display rows: entity form_id + editor_id +
/// space id + `legend_text(include_count = false)`. Spaces never merge
/// (space id is part of the key); hidden-field differences (quantity,
/// spawn weight, count) stay in the row's variants.
fn merge_key(r: &SearchResult) -> (u32, String, u32, String) {
    (
        r.form_id(),
        r.editor_id().to_string(),
        r.space().form_id(),
        grouped_of(r).legend_text(false),
    )
}

/// Group raw `core::search` output into one [`ResultRow`] per merge key,
/// preserving first-seen order. The row `display` is the first hit with
/// the summed count; `variants` keeps every raw hit.
pub fn merge_results(results: Vec<SearchResult>) -> Vec<ResultRow> {
    let mut order: Vec<(u32, String, u32, String)> = Vec::new();
    let mut groups: std::collections::HashMap<(u32, String, u32, String), Vec<SearchResult>> =
        std::collections::HashMap::new();
    for r in results {
        let key = merge_key(&r);
        groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Vec::new()
        });
        groups.get_mut(&key).expect("just inserted").push(r);
    }
    order
        .into_iter()
        .map(|key| {
            let variants = groups.remove(&key).expect("grouped key");
            let total: i64 = variants.iter().map(|v| v.count()).sum();
            let mut display = variants[0].clone();
            match &mut display {
                SearchResult::Grouped(g) => g.count = total,
                SearchResult::Singular { inner, .. } => inner.count = total,
            }
            ResultRow { display, variants }
        })
        .collect()
}

/// Identity of a merged row, used to dedupe adds and to match items back
/// to the rows they were added from. Operates on rows (the display key),
/// so re-adding a checked row is a no-op.
fn result_key(row: &ResultRow) -> (u32, String, u32, String) {
    merge_key(&row.display)
}

fn item_key(it: &PlotItem) -> (u32, String, u32, String) {
    merge_key(&it.result)
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

/// Primary name for a result row: derived entities (NPC/scrap/flux) show
/// the `DisplayName` (e.g. `Derived Antiseptic Scrap`) which the user
/// recognizes; base entities show the `EditorID`.
fn row_primary_name(row: &ResultRow) -> &str {
    let g = grouped_of(&row.display);
    match row.entity_kind() {
        EntityKind::DerivedNpc | EntityKind::DerivedScrap | EntityKind::DerivedFlux(_) => {
            &g.entity.display_name
        }
        _ => &g.entity.editor_id,
    }
}

/// Row labels: primary `{name} (x{count})` plus secondary disambiguation
/// (`space | label | lock`) so rows that share a name no longer look like
/// clones.
pub fn row_labels(row: &ResultRow) -> (String, String) {
    let primary = format!("{} (x{})", row_primary_name(row), row.count());
    let g = grouped_of(&row.display);
    let mut secondary = row.space().friendly_name();
    if !g.label.trim().is_empty() {
        secondary.push_str(&format!(" | {}", g.label));
    }
    if g.lock_level != LockLevel::None {
        secondary.push_str(&format!(
            " | {}",
            g.lock_level.to_friendly_name_with_context()
        ));
    }
    (primary, secondary)
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
                ui.checkbox(&mut self.search_all_spaces, "All spaces");
                if ui.button("Search").clicked() {
                    if let Err(e) = self.run_search() {
                        self.status = e;
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("Results ({})", self.results.len()));
                    let mut all = self.all_checked();
                    if ui.checkbox(&mut all, "all").changed() {
                        self.set_all_checked(all);
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for i in 0..self.results.len() {
                            let (primary, secondary) = row_labels(&self.results[i]);
                            let current = self.checked[i];
                            let (changed, next) = ui
                                .horizontal(|ui| {
                                    let mut tmp = current;
                                    let changed = ui.checkbox(&mut tmp, primary).changed();
                                    ui.weak(secondary);
                                    (changed, tmp)
                                })
                                .inner;
                            if changed {
                                self.checked[i] = next;
                            }
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Add selected").clicked() {
                        self.add_selected();
                    }
                    if ui.button("Remove checked").clicked() {
                        self.remove_selected();
                    }
                    if ui.button("Clear results").clicked() {
                        self.clear_results();
                    }
                });
                ui.separator();
                if ui.button("Find interiors").clicked() {
                    if let Err(e) = self.find_interiors() {
                        self.status = e;
                    }
                }
                egui::CollapsingHeader::new(format!("Interiors ({})", self.interiors.len())).show(
                    ui,
                    |ui| {
                        if self.interiors.is_empty() {
                            ui.label(format!(
                                "No interiors yet. Check a result and Find interiors{}.",
                                if self.interiors_for.is_empty() {
                                    String::new()
                                } else {
                                    format!(" (last: {})", self.interiors_for)
                                }
                            ));
                        }
                        // Snapshot rows first: the click handler mutates self.
                        let rows: Vec<(usize, String, usize)> = self
                            .interiors
                            .iter()
                            .enumerate()
                            .map(|(i, h)| (i, h.display_name.clone(), h.count))
                            .collect();
                        for (i, name, count) in rows {
                            if ui.button(format!("{name} ({count})")).clicked() {
                                self.apply_interior(i);
                            }
                        }
                    },
                );
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
        // Auto-render: spawn when dirty (no-op while already rendering or
        // when the toggle is off — the Render button stays manual).
        self.maybe_auto_render();

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
            ui.checkbox(&mut self.auto_render, "Auto render");
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
        app.results = vec![single_row(item.result.clone())];
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
        app.results = vec![single_row(item.result.clone())];
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
        let result = SearchResult::Grouped(grouped);
        PlotItem {
            result: result.clone(),
            variants: vec![result],
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

    /// Wrap a raw result as a single-variant merged row.
    fn single_row(r: SearchResult) -> ResultRow {
        ResultRow {
            display: r.clone(),
            variants: vec![r],
        }
    }

    /// Fake grouped hit for DB-free merge tests.
    fn fake_grouped(
        space: &Space,
        editor: &str,
        display_name: &str,
        count: i64,
        label: &str,
    ) -> SearchResult {
        SearchResult::Grouped(Grouped {
            entity: Entity::new(
                0x2000,
                editor,
                display_name,
                Signature::Misc,
                EntityKind::Standard,
            ),
            space: space.clone(),
            count,
            spawn_weight: 1.0,
            label: label.to_string(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        })
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
        app.results = vec![
            single_row(item0.result.clone()),
            single_row(item1.result.clone()),
        ];
        app.checked = vec![false, true];
        app.items = vec![item0, item1];
        app.next_group = 2;
        app.remove_selected();
        assert_eq!(app.items.len(), 1);
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.next_group, 1);
    }

    /// GUI features (1): auto-render toggle.
    #[test]
    fn auto_render_defaults_on_and_clean() {
        let app = settings_app();
        assert!(app.auto_render, "Auto render defaults ON");
        assert!(!app.dirty, "fresh state is clean");
    }

    #[test]
    fn item_mutations_mark_dirty_filename_does_not() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let item = test_plot_item(&space, 0);
        app.results = vec![single_row(item.result.clone())];
        app.checked = vec![true];
        app.add_selected();
        assert!(app.dirty, "add must mark dirty");
        app.dirty = false;
        app.remove_selected();
        assert!(app.dirty, "remove must mark dirty");
        // Filename edits never invalidate: excluded from dirty by design.
        app.dirty = false;
        let before = app.settings_fingerprint();
        app.filename = "custom name".to_string();
        assert_eq!(app.settings_fingerprint(), before);
        assert!(!app.dirty);
    }

    /// Fake paths so `start_preview` spawns a worker (which fails fast
    /// off-thread on the bad DB path) instead of failing synchronously:
    /// proves the spawn path clears `dirty` without the real DB/assets.
    fn spawnable_app() -> App {
        let mut app = settings_app();
        app.db_path = Some("/nonexistent/mappalachia.db".to_string());
        app.assets_root = Some("/nonexistent/assets".to_string());
        app.dirty = true;
        app
    }

    #[test]
    fn maybe_auto_render_spawns_when_dirty_and_idle() {
        let mut app = spawnable_app();
        app.maybe_auto_render();
        assert!(
            matches!(app.preview, PreviewState::Rendering),
            "auto + dirty + idle must spawn"
        );
        assert!(!app.dirty, "dirty cleared on spawn");
        assert!(app.preview_rx.is_some());
    }

    #[test]
    fn maybe_auto_render_off_keeps_idle_and_dirty() {
        let mut app = spawnable_app();
        app.auto_render = false;
        app.maybe_auto_render();
        assert!(matches!(app.preview, PreviewState::Idle));
        assert!(app.dirty, "dirty survives while auto is off");
        assert!(app.preview_rx.is_none());
    }

    #[test]
    fn maybe_auto_render_no_spawn_while_rendering_or_failed() {
        // No second spawn while a worker runs (no render loops).
        let mut app = spawnable_app();
        app.maybe_auto_render();
        app.dirty = true;
        app.maybe_auto_render();
        assert!(matches!(app.preview, PreviewState::Rendering));
        assert!(app.preview_rx.is_some());
        // A failed preview with stale dirty never retries on its own.
        let mut failed = settings_app();
        failed.preview = PreviewState::Failed("boom".to_string());
        failed.dirty = true;
        failed.maybe_auto_render();
        assert!(matches!(failed.preview, PreviewState::Failed(_)));
        assert!(failed.preview_rx.is_none());
    }

    /// GUI features (3): bulk clear.
    #[test]
    fn header_checkbox_toggles_all_results() {
        let mut app = settings_app();
        assert!(!app.all_checked(), "empty list is not 'all checked'");
        let space = app.spaces[app.space_idx].clone();
        app.results = vec![
            single_row(test_plot_item(&space, 0).result),
            single_row(test_plot_item(&space, 1).result),
            single_row(test_plot_item(&space, 2).result),
        ];
        app.checked = vec![false, true, false];
        assert!(!app.all_checked());
        app.set_all_checked(true);
        assert!(app.all_checked());
        assert!(!app.dirty, "checkbox-only state must not mark dirty");
        app.set_all_checked(false);
        assert_eq!(app.checked, vec![false; 3]);
        assert!(!app.all_checked());
    }

    #[test]
    fn remove_checked_removes_all_at_once_with_count() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let made: Vec<PlotItem> = (0..3).map(|g| test_plot_item(&space, g)).collect();
        app.results = made
            .iter()
            .map(|it| single_row(it.result.clone()))
            .collect();
        app.checked = vec![true, false, true];
        app.items = vec![made[0].clone(), made[2].clone()];
        app.remove_selected();
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.checked, vec![false]);
        assert!(app.items.is_empty());
        assert!(
            app.status.contains("Removed 2"),
            "status must carry the count, got: {}",
            app.status
        );
    }

    #[test]
    fn clear_results_empties_list_but_keeps_items() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let item = test_plot_item(&space, 0);
        app.results = vec![single_row(item.result.clone())];
        app.checked = vec![true];
        app.items = vec![item];
        app.clear_results();
        assert!(app.results.is_empty());
        assert!(app.checked.is_empty());
        assert_eq!(app.items.len(), 1, "items-to-plot untouched");
        assert!(!app.dirty, "render unaffected, must stay clean");
        assert!(app.status.contains("Cleared 1"), "got: {}", app.status);
    }

    /// GUI features (2): interiors finder needs a selection first (DB-free:
    /// the selection check runs before any DB open).
    #[test]
    fn find_interiors_without_selection_errors() {
        let mut app = settings_app();
        let err = app
            .find_interiors()
            .expect_err("needs a checked result or item");
        assert!(err.contains("Nothing selected"), "got: {err}");
        assert!(app.interiors.is_empty());
    }

    /// DB-gated interiors finder: probe the first results against the
    /// first connected CELLs until one hits, then run the finder on it.
    #[test]
    fn find_interiors_lists_connected_cells_with_counts() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.run_search().expect("search succeeds");
        let conn =
            db::open_readonly(app.db_path.as_deref().expect("db path")).expect("test DB opens");
        let current = app.spaces[app.space_idx].clone();
        let teleporters = instances::teleporters(&conn, &current).expect("teleporters load");
        assert!(
            !teleporters.is_empty(),
            "current space needs outgoing teleporters"
        );
        // Distinct CELL targets (same dedupe rule as the finder).
        let mut seen = std::collections::HashSet::new();
        let mut targets = Vec::new();
        for t in &teleporters {
            if let Some(dest) = &t.teleports_to {
                if !dest.is_worldspace && seen.insert((t.space.form_id(), dest.form_id())) {
                    targets.push(dest.clone());
                }
            }
        }
        assert!(
            !targets.is_empty(),
            "current space needs connected CELL spaces"
        );
        let mut picked: Option<usize> = None;
        'outer: for (i, row) in app.results.iter().take(8).enumerate() {
            for dest in targets.iter().take(4) {
                let n = instances::instances(&conn, row.display.clone(), dest)
                    .expect("instances resolve")
                    .len();
                if n > 0 {
                    picked = Some(i);
                    break 'outer;
                }
            }
        }
        let Some(i) = picked else {
            eprintln!("SKIP: no probed result occurs in a connected CELL");
            return;
        };
        let editor = app.results[i].editor_id().to_string();
        app.checked[i] = true;
        app.find_interiors().expect("finder succeeds");
        assert!(!app.interiors.is_empty(), "probed hit must surface");
        assert_eq!(app.interiors_for, editor);
        for h in &app.interiors {
            assert!(h.count > 0);
            assert!(h.space_idx < app.spaces.len());
            assert!(
                !app.spaces[h.space_idx].is_worldspace,
                "hits must be CELL spaces"
            );
        }
        assert!(
            app.interiors.windows(2).all(|w| w[0].count >= w[1].count),
            "hits sorted by count desc"
        );
        assert!(app.status.contains("interior"), "got: {}", app.status);
        assert!(!app.dirty, "finding must not mark dirty");
        // Row click switches space and invalidates.
        let (target_idx, target_name) = (
            app.interiors[0].space_idx,
            app.interiors[0].display_name.clone(),
        );
        app.apply_interior(0);
        assert_eq!(app.space_idx, target_idx);
        assert_eq!(app.spaces[app.space_idx].display_name(), target_name);
        assert!(app.dirty, "space switch affects the render");
        assert!(matches!(app.preview, PreviewState::Idle));
        // Out-of-range click is a no-op.
        app.apply_interior(999);
        assert_eq!(app.space_idx, target_idx);
    }

    /// Merge (DB-free): identical-display rows merge, counts sum, variants
    /// kept; distinct rows (label / editor / space) stay separate; spaces
    /// never merge.
    #[test]
    fn merge_identical_rows_sum_counts_and_keep_variants() {
        let app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let a = fake_grouped(&space, "MilitaryAmmoBag", "Ammo Bag", 2, "");
        let b = fake_grouped(&space, "MilitaryAmmoBag", "Ammo Bag", 3, "");
        let rows = merge_results(vec![a, b]);
        assert_eq!(rows.len(), 1, "identical-display rows must merge");
        assert_eq!(rows[0].variants.len(), 2);
        assert_eq!(rows[0].count(), 5, "counts must sum");
    }

    #[test]
    fn merge_keeps_distinct_rows_and_never_across_spaces() {
        let app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let other = Space::new(
            0x0000_0010,
            "CELL0002",
            "Test Cell 2",
            false,
            false,
            0.0,
            0.0,
            1000.0,
            0.0,
        );
        let rows = merge_results(vec![
            fake_grouped(&space, "AmmoBag", "Ammo Bag", 1, ""),
            // Different label -> distinct legend -> distinct row.
            fake_grouped(&space, "AmmoBag", "Ammo Bag", 1, "Shelf"),
            // Different editor -> distinct row.
            fake_grouped(&space, "OtherBag", "Other Bag", 1, ""),
            // Same everything but another space -> must NOT merge.
            fake_grouped(&other, "AmmoBag", "Ammo Bag", 1, ""),
        ]);
        assert_eq!(
            rows.len(),
            4,
            "distinct rows and cross-space rows stay separate"
        );
    }

    #[test]
    fn merged_row_adds_once_expands_to_all_variants_and_readd_is_noop() {
        let mut app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        app.results = merge_results(vec![
            fake_grouped(&space, "MilitaryAmmoBag", "Ammo Bag", 2, ""),
            fake_grouped(&space, "MilitaryAmmoBag", "Ammo Bag", 3, ""),
        ]);
        assert_eq!(app.results.len(), 1);
        app.checked = vec![true];
        app.add_selected();
        assert_eq!(app.items.len(), 1, "one row adds one item");
        assert_eq!(app.items[0].variants.len(), 2);
        let expanded = expand_plot_items(&app.items);
        assert_eq!(expanded.len(), 2, "expansion must cover every variant");
        for p in &expanded {
            assert_eq!(p.legend_group, app.items[0].legend_group as i32);
            assert_eq!(p.legend_text, app.items[0].legend);
            assert_eq!(p.icon, app.items[0].icon);
        }
        app.add_selected();
        assert_eq!(app.items.len(), 1, "re-adding the row must be a no-op");
    }

    #[test]
    fn row_labels_show_space_label_lock_and_derived_display_name() {
        let app = settings_app();
        let space = app.spaces[app.space_idx].clone();
        let row =
            &merge_results(vec![fake_grouped(&space, "AmmoBag", "Ammo Bag", 4, "")])[0].clone();
        let (primary, secondary) = row_labels(row);
        assert!(primary.contains("AmmoBag"), "got: {primary}");
        assert!(primary.contains("x4"), "got: {primary}");
        assert!(
            secondary.contains("Appalachia"),
            "secondary must name the space, got: {secondary}"
        );
        let labelled = &merge_results(vec![fake_grouped(
            &space, "AmmoBag", "Ammo Bag", 1, "Shelf",
        )])[0]
            .clone();
        assert!(
            row_labels(labelled).1.contains("Shelf"),
            "secondary must carry the label"
        );
        // Derived entities show the DisplayName, not the bare editor id.
        let derived = ResultRow {
            display: SearchResult::Grouped(Grouped {
                entity: Entity::derived_scrap("Antiseptic"),
                space: space.clone(),
                count: 2,
                spawn_weight: 1.0,
                label: String::new(),
                lock_level: LockLevel::None,
                in_container: false,
                legend_group: 0,
                legend_text: None,
                plot_icon: None,
            }),
            variants: vec![],
        };
        let (primary, _) = row_labels(&derived);
        assert!(
            primary.contains("Derived Antiseptic Scrap"),
            "derived rows must show the DisplayName, got: {primary}"
        );
    }

    #[test]
    fn search_scope_all_spaces_unscopes_and_default_scopes() {
        let mut app = settings_app();
        assert!(!app.search_all_spaces, "scoped search is the default");
        assert_eq!(
            app.search_params().space_form_id,
            Some(app.spaces[app.space_idx].form_id())
        );
        app.search_all_spaces = true;
        assert_eq!(app.search_params().space_form_id, None);
    }

    /// DB-gated: `MilitaryAmmoBag` yields several identical-display rows
    /// that must merge into exactly one row with all variants kept.
    #[test]
    fn search_militaryammobag_merges_to_single_row() {
        let Some(mut app) = search_app() else {
            eprintln!("SKIP: MAPPALACHIA_DB unset");
            return;
        };
        app.search_term = "MilitaryAmmoBag".to_string();
        let conn =
            db::open_readonly(app.db_path.as_deref().expect("db path")).expect("test DB opens");
        let raw = search(&conn, &app.search_params()).expect("raw search succeeds");
        if raw.is_empty() {
            eprintln!("SKIP: no MilitaryAmmoBag rows in this DB");
            return;
        }
        app.run_search().expect("search succeeds");
        assert_eq!(
            app.results.len(),
            1,
            "MilitaryAmmoBag rows must merge to one (raw: {})",
            raw.len()
        );
        let row = &app.results[0];
        assert_eq!(row.variants.len(), raw.len(), "no instances may be lost");
        assert_eq!(
            row.count(),
            raw.iter().map(|r| r.count()).sum::<i64>(),
            "row count must sum the raw counts"
        );
    }
}
