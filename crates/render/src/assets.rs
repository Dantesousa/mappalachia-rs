//! Asset loading: backgrounds, water masks, compass, SVG markers, plot icons.
//!
//! Mirrors C# `Mappalachia/FileIO.cs` lines 105-270 + `Mappalachia/Paths.cs`:
//! - `GetBackgroundImage`: `img/{wrld|cell}/{EditorID}[_menu|_military].jpg`.
//! - `GetWaterMask`: `img/wrld/{EditorID}_waterMask.png`, worldspace only.
//! - `CompassRose`: `img/CompassRose.png`.
//! - `GetMapMarkerImage` / `GetDoorMarker` / `GetInfestationMarker`:
//!   SVG rasterized via resvg at `Map.IconScale` (1.5).
//! - `GetPlotIconImage` / `GetAvailablePlotIconCount`: `img/icon/*.png`
//!   sorted alphabetically (deterministic; C# `GetFiles` order is
//!   platform-defined), out-of-range index falls back to index 0.
//! - Font: `font/futura_condensed_bold.otf`.
//!
//! Missing optional images (`None` kind, absent file) yield `None` — the
//! caller paints empty — mirroring C#'s `EmptyMapImage` return.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mappalachia_core::model::Space;

/// Mirrors C# `Map.IconScale` (`Map.cs`): SVG icons rasterize scaled by 1.5.
pub const ICON_SCALE: f32 = 1.5;

/// Mirrors C# `BackgroundImageType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundKind {
    Render,
    Menu,
    Military,
    None,
}

/// Asset root handle with raster-image and SVG caches.
///
/// Mirrors the static caches on C# `FileIO` (`BackgroundImageCache`,
/// `MapMarkerIconImageCache`, `PlotIconImageCache`).
pub struct Assets {
    root: PathBuf,
    cache: HashMap<String, image::RgbaImage>,
    svg_cache: HashMap<String, tiny_skia::Pixmap>,
    plot_icons: Vec<PathBuf>,
}

impl Assets {
    /// Open the asset tree at `root` (e.g. the release folder holding
    /// `img/` and `font/`). Plot icons are enumerated eagerly so
    /// [`Assets::plot_icon_count`] works on `&self`.
    pub fn open(root: &str) -> Self {
        let root = PathBuf::from(root);
        let mut plot_icons = Vec::new();
        let icon_dir = root.join("img/icon");
        if let Ok(entries) = std::fs::read_dir(&icon_dir) {
            let mut names: Vec<_> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
                })
                .collect();
            names.sort();
            plot_icons = names;
        }
        Self {
            root,
            cache: HashMap::new(),
            svg_cache: HashMap::new(),
            plot_icons,
        }
    }

    /// Join `rel` onto the asset root (uses `/` separators like C# `Paths`).
    fn img(&self, rel: &str) -> PathBuf {
        self.root.join(rel.replace('\\', "/"))
    }

    /// Resolve `rel` to an existing file, falling back to a
    /// case-insensitive directory match. The release tree mixes case
    /// (`APPALACHIA.jpg` vs `Appalachia_menu.jpg`) and C# on Windows is
    /// case-insensitive, so an exact-only lookup would miss files on Linux.
    fn resolve(&self, rel: &str) -> Option<PathBuf> {
        let path = self.img(rel);
        if path.is_file() {
            return Some(path);
        }
        let name = path.file_name()?.to_str()?;
        let parent = path.parent()?;
        std::fs::read_dir(parent)
            .ok()?
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .map(|e| e.path())
    }

    /// Load (and cache) a raster image, or `None` when absent/unreadable.
    fn load_cached(&mut self, path: &Path) -> Option<&image::RgbaImage> {
        let key = path.to_string_lossy().into_owned();
        if !self.cache.contains_key(&key) {
            let img = image::open(path).ok()?.to_rgba8();
            self.cache.insert(key.clone(), img);
        }
        self.cache.get(&key)
    }

    /// Mirrors `GetBackgroundImage`: `img/{wrld|cell}/{EditorID}[_menu|_military].jpg`.
    /// `None` kind or a missing file yields `None` (caller paints empty).
    pub fn background(
        &mut self,
        space: &Space,
        kind: &BackgroundKind,
    ) -> Option<&image::RgbaImage> {
        if *kind == BackgroundKind::None {
            return None;
        }
        let dir = if space.is_worldspace { "wrld" } else { "cell" };
        let suffix = match kind {
            BackgroundKind::Render => String::new(),
            BackgroundKind::Menu => "_menu".to_owned(),
            BackgroundKind::Military => "_military".to_owned(),
            BackgroundKind::None => unreachable!(),
        };
        let rel = format!("img/{dir}/{}{suffix}.jpg", space.editor_id());
        let path = self.resolve(&rel)?;
        self.load_cached(&path)
    }

    /// Mirrors `GetWaterMask`: `img/wrld/{EditorID}_waterMask.png`,
    /// worldspace only; otherwise `None`.
    pub fn water_mask(&mut self, space: &Space) -> Option<&image::RgbaImage> {
        if !space.is_worldspace {
            return None;
        }
        let rel = format!("img/wrld/{}_waterMask.png", space.editor_id());
        let path = self.resolve(&rel)?;
        self.load_cached(&path)
    }

    /// `img/CompassRose.png`. Panics when absent — it is a required asset.
    pub fn compass(&mut self) -> &image::RgbaImage {
        let path = self
            .resolve("img/CompassRose.png")
            .expect("missing required asset img/CompassRose.png");
        self.load_cached(&path)
            .expect("unreadable required asset img/CompassRose.png")
    }

    /// Rasterize an SVG marker via resvg at [`ICON_SCALE`] (mirrors
    /// `LoadSVGIcon(path, 1.5)`).
    ///
    /// Accepts a bare icon name (`"VaultMarker"`), a file name
    /// (`"VaultMarker.svg"`), or a root-relative path
    /// (`"mapmarker/VaultMarker.svg"`, `"DoorMarker.svg"`). Bare names
    /// resolve under `img/mapmarker/` (mirrors `GetMapMarkerImage`:
    /// `Paths.MapMarkersPath + icon + ".svg"`), except `DoorMarker` and
    /// `InfestationMarker` which live directly under `img/`.
    pub fn svg_marker(&mut self, rel: &str) -> tiny_skia::Pixmap {
        let rel = Self::normalize_marker(rel);
        if let Some(hit) = self.svg_cache.get(&rel) {
            return hit.clone();
        }
        let path = self
            .resolve(&rel)
            .unwrap_or_else(|| panic!("missing required asset {rel}"));
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("cannot read asset {}: {e}", path.display()));
        let pix = Self::rasterize_svg(&bytes);
        self.svg_cache.insert(rel, pix.clone());
        pix
    }

    /// Mirrors `GetMapMarkerImage`: `img/mapmarker/{icon}.svg`.
    pub fn map_marker_icon(&mut self, icon: &str) -> tiny_skia::Pixmap {
        self.svg_marker(icon)
    }

    /// Mirrors `GetDoorMarker` (`img/DoorMarker.svg`).
    pub fn door_marker(&mut self) -> tiny_skia::Pixmap {
        self.svg_marker("DoorMarker.svg")
    }

    /// Mirrors `GetInfestationMarker` (`img/InfestationMarker.svg`).
    pub fn infestation_marker(&mut self) -> tiny_skia::Pixmap {
        self.svg_marker("InfestationMarker.svg")
    }

    /// Mirrors `GetPlotIconImage(index)`: `img/icon/*.png` sorted
    /// alphabetically; out-of-range index falls back to index 0 like C#.
    pub fn plot_icon_png(&mut self, index: usize) -> &image::RgbaImage {
        assert!(
            !self.plot_icons.is_empty(),
            "no plot icon files found under img/icon"
        );
        let path = self
            .plot_icons
            .get(index)
            .unwrap_or(&self.plot_icons[0])
            .clone();
        self.load_cached(&path).expect("unreadable plot icon image")
    }

    /// Mirrors `GetAvailablePlotIconCount` (release ships 7: `A.png..G.png`).
    pub fn plot_icon_count(&self) -> usize {
        self.plot_icons.len()
    }

    /// Raw bytes of `font/futura_condensed_bold.otf` (mirrors the
    /// `PrivateFontCollection.AddFontFile(Paths.FontPath)` startup load).
    pub fn font_bytes(&self) -> Vec<u8> {
        let path = self.img("font/futura_condensed_bold.otf");
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("missing required asset {}: {e}", path.display()))
    }

    fn normalize_marker(rel: &str) -> String {
        let rel = rel.replace('\\', "/");
        let trimmed = rel.strip_prefix("img/").unwrap_or(&rel).to_owned();
        if trimmed.contains('/') {
            return format!("img/{trimmed}");
        }
        let stem = trimmed.strip_suffix(".svg").unwrap_or(&trimmed);
        if stem.eq_ignore_ascii_case("DoorMarker") || stem.eq_ignore_ascii_case("InfestationMarker")
        {
            return format!("img/{stem}.svg");
        }
        format!("img/mapmarker/{stem}.svg")
    }

    fn rasterize_svg(bytes: &[u8]) -> tiny_skia::Pixmap {
        let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())
            .expect("invalid SVG asset");
        let size = tree
            .size()
            .to_int_size()
            .scale_by(ICON_SCALE)
            .unwrap_or_else(|| {
                let s = tree.size();
                tiny_skia::IntSize::from_wh(
                    (s.width() * ICON_SCALE).ceil() as u32,
                    (s.height() * ICON_SCALE).ceil() as u32,
                )
                .expect("SVG asset has zero size")
            });
        let mut pixmap =
            tiny_skia::Pixmap::new(size.width(), size.height()).expect("SVG asset has zero size");
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(ICON_SCALE, ICON_SCALE),
            &mut pixmap.as_mut(),
        );
        pixmap
    }
}
