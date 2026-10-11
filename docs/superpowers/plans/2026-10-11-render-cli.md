# Render + CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `crates/render` (layered 4096px map pipeline: background → compass → brightness → water → grid → infestations → markers → Standard plots → watermark → title → legend) and `crates/cli` (recipe.json → .png/.jpg), tested against release assets.

**Architecture:** `render` owns typed settings (parsed from core's recipe `Value` stubs), tiny-skia Pixmap canvas, resvg SVG raster, fontdue text, image JPEG/PNG IO. `cli` (clap) loads recipe → `populate_from_data` → draw → save beside recipe (PNG unless worldspace+background, then JPEG). No egui, no other plot modes (next plan), no spotlight tiles (data unavailable — documented stub).

**Tech Stack:** tiny-skia 0.11, resvg 0.45 (must match tiny-skia version), image 0.25, fontdue 0.9, csscolorparser 0.7, clap 4, serde/serde_json (via core).

## Global Constraints

- Workspace root `/home/dante/Documents/mappalachia-rs/`, resolver 2, edition 2021.
- Asset root env `MAPPALACHIA_ASSETS` = `/mnt/hd/mappalachia` (full release extract: `img/{wrld,cell,mapmarker,icon}`, `font/`, `data/`). Tests skip with message when unset/missing. Never write into it.
- Test DB env `MAPPALACHIA_DB` (same skip pattern as core).
- Canvas is always 4096×4096 (`MAP_IMAGE_RESOLUTION`); Extended legend widens to 4096+585.
- Draw order follows `Map.Draw` (`~/Documents/Mappalachia/Mappalachia/Map.cs` lines 122-209) exactly.
- Same-value constants from `Map.cs` lines 53-100 (listed per task).
- Frequent commits, one per task minimum.

---

### Task 1: Scaffold render + color math

**Files:**
- Create: `crates/render/Cargo.toml`
- Create: `crates/render/src/lib.rs`
- Create: `crates/render/src/color.rs`
- Modify: `Cargo.toml` (members add `crates/render`, `crates/cli`)
- Test: inline in `color.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `color::{Rgba, lerp, lerp_palette, with_alpha, apply_brightness_grayscale, tint_keep_alpha, black_to_transparent}` used by Tasks 3–6.

- [ ] **Step 1: Write Cargo.toml + failing test**

```toml
# crates/render/Cargo.toml
[package]
name = "mappalachia-render"
version = "0.1.0"
edition = "2021"

[dependencies]
mappalachia-core = { path = "../core" }
tiny-skia = "0.11"
resvg = "0.45"
image = "0.25"
fontdue = "0.9"
csscolorparser = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lerp_midpoint() {
        assert_eq!(lerp(Rgba::new(0,0,0,255), Rgba::new(100,200,50,255), 0.5), Rgba::new(50,100,25,255));
    }
    #[test]
    fn grayscale_red() {
        let g = apply_brightness_grayscale(Rgba::new(255,0,0,255), 1.0, true);
        assert_eq!((g.r, g.g, g.b), (76,76,76)); // 0.299*255 = 76.245 -> (int)76
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mappalachia-render lerp_midpoint`
Expected: FAIL (crate doesn't exist; then unresolved `color`).

- [ ] **Step 3: Write minimal implementation**

Port `ImageHelper.cs` pixel ops onto a plain struct (tiny-skia interop in later tasks):

```rust
// crates/render/src/color.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba { pub r: u8, pub g: u8, pub b: u8, pub a: u8 }
impl Rgba { pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self { Self { r, g, b, a } } }

/// Mirrors ImageHelper.LerpColors(Color, Color, range): (int) truncation per channel.
pub fn lerp(x: Rgba, y: Rgba, range: f64) -> Rgba {
    Rgba::new((x.r as f64 + ((y.r as f64 - x.r as f64) * range)) as u8,
              (x.g as f64 + ((y.g as f64 - x.g as f64) * range)) as u8,
              (x.b as f64 + ((y.b as f64 - x.b as f64) * range)) as u8, 255)
}

/// Mirrors ImageHelper.LerpColors(Color[], range): 1->first, 0->Black, range 0/1 edges,
/// position = range*(n-1), floor/frac segment lerp. Panics outside [0,1] like C#.
pub fn lerp_palette(colors: &[Rgba], range: f64) -> Rgba {
    if colors.len() == 1 { return colors[0]; }
    if colors.is_empty() { return Rgba::new(0,0,0,255); }
    assert!((0.0..=1.0).contains(&range), "range out of [0,1]");
    if range == 0.0 { return colors[0]; }
    if range == 1.0 { return colors[colors.len()-1]; }
    let pos = range * (colors.len() - 1) as f64;
    let prev = pos.floor() as usize;
    lerp(colors[prev], colors[prev+1], pos - prev as f64)
}

pub fn with_alpha(c: Rgba, alpha: u8) -> Rgba { Rgba::new(c.r, c.g, c.b, alpha) }

/// Mirrors GenerateColorMatrix: grayscale ? 0.299/0.587/0.114 rows * brightness : brightness diagonal.
/// Alpha channel untouched.
pub fn apply_brightness_grayscale(px: Rgba, brightness: f32, grayscale: bool) -> Rgba {
    if !grayscale {
        return Rgba::new((px.r as f32 * brightness) as u8, (px.g as f32 * brightness) as u8, (px.b as f32 * brightness) as u8, px.a);
    }
    let v = (0.299 * px.r as f32 + 0.587 * px.g as f32 + 0.114 * px.b as f32) * brightness;
    let v = v as u8;
    Rgba::new(v, v, v, px.a)
}

/// Mirrors SetColor: RGB replaced, alpha preserved.
pub fn tint_keep_alpha(px: Rgba, color: Rgba) -> Rgba { Rgba::new(color.r, color.g, color.b, px.a) }

/// Mirrors ReplaceBlackWithTransparency: opaque black -> transparent.
pub fn black_to_transparent(px: Rgba) -> Rgba {
    if px.a != 0 && px.r == 0 && px.g == 0 && px.b == 0 { Rgba::new(0,0,0,0) } else { px }
}

/// Parse `#rrggbb`/`#rgb` or CSS named color (ColorTranslator.FromHtml/ToHtml compat).
pub fn parse_color(s: &str) -> Option<Rgba> {
    csscolorparser::parse(s).ok().map(|c| { let [r,g,b,a] = c.to_rgba8(); Rgba::new(r,g,b,a) })
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mappalachia-render`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/render Cargo.toml
git commit -m "feat(render): scaffold crate with color math"
```

---

### Task 2: World↔image coordinate transform

**Files:**
- Create: `crates/render/src/transform.rs`
- Modify: `crates/render/src/lib.rs` (add `pub mod transform;`)
- Test: inline

**Interfaces:**
- Consumes: `MAP_IMAGE_RESOLUTION`, core `Space`/`Coord`.
- Produces: `transform::{world_to_image, image_to_world, world_len_to_image, image_rect_of_world, world_rect_of_image, background_rect}` used by Tasks 4–6.

Port `Map.cs` lines 1240-1318, spotlight disabled (no spotlight in this plan):

```rust
pub struct View { pub center_x: f64, pub center_y: f64, pub factor: f64 }
impl View {
    pub fn for_space(space: &mappalachia_core::model::Space) -> Self {
        Self { center_x: space.center_x, center_y: space.center_y,
               factor: space.max_range / mappalachia_core::common::MAP_IMAGE_RESOLUTION as f64 }
    }
}
```

/// Mirrors AsImagePoint: x = (wx-cx)/f + 2048; y = ((-wy+cy)/f) + 2048 (Y flip).

Write it plainly:

```rust
pub fn world_to_image(v: &View, x: f64, y: f64) -> (f32, f32) {
    (((x - v.center_x) / v.factor + 2048.0) as f32,
     (((-y + v.center_y) / v.factor) + 2048.0) as f32)
}
/// Mirrors AsWorldCoord (exact inverse).
pub fn image_to_world(v: &View, px: f32, py: f32) -> (f64, f64) {
    (((px as f64 - 2048.0) * v.factor) + v.center_x,
     ((((py as f64 - 2048.0) * v.factor) - v.center_y) * -1.0))
}
/// Mirrors AsImageLength (double + int forms).
pub fn world_len_to_image(v: &View, len: f64) -> f32 { (len / v.factor) as f32 }
```

Rect helpers mirror `AsImageRectangle`/`AsWorldRectangle` (abs widths), `background_rect` returns full `(0,0,4096,4096)` (spotlight disabled).

Tests (exact, APPALACHIA center (-500,135) maxRange 582550 → factor 142.22412109375):

```rust
#[test]
fn center_maps_to_center() {
    let v = View { center_x: -500.0, center_y: 135.0, factor: 582550.0/4096.0 };
    assert_eq!(world_to_image(&v, -500.0, 135.0), (2048.0, 2048.0));
    // one factor east -> +1px; Y flip: larger world y -> smaller image y
    assert_eq!(world_to_image(&v, -500.0 + v.factor, 135.0), (2049.0, 2048.0));
    let (px, py) = world_to_image(&v, 1000.0, 2000.0);
    let (wx, wy) = image_to_world(&v, px, py);
    assert!((wx - 1000.0).abs() < 200.0 && (wy - 2000.0).abs() < 200.0); // f32 round-trip tolerance
}
```

Steps 2/4/5 same shape as Task 1 (fail → pass → commit `feat(render): world-image transform`).

---

### Task 3: Asset loading (backgrounds, masks, compass, SVG markers, icons)

**Files:**
- Create: `crates/render/src/assets.rs`
- Modify: `lib.rs`
- Test: `crates/render/tests/assets_parity.rs` (skip without `MAPPALACHIA_ASSETS`)

**Interfaces:**
- Consumes: `color`, core `Space`/`MapMarker`.
- Produces: `assets::Assets::open(root)`, `background(space, kind)`, `water_mask(space)`, `compass()`, `door_marker()`, `infestation_marker()`, `map_marker achieving icon(name)`, `plot_icon_png(index)`, `plot_icon_count()`, `font_bytes()` used by Tasks 4–6.

Port `FileIO.cs` lines 105-270 + `Paths.cs`:

```rust
pub struct Assets { root: std::path::PathBuf, cache: std::collections::HashMap<String, image::RgbaImage> }

pub enum BackgroundKind { Render, Menu, Military, None } // mirrors BackgroundImageType

impl Assets {
    pub fn open(root: &str) -> Self { ... }
    fn img(&self, rel: &str) -> &str { ... } // join helper
    /// Mirrors GetBackgroundImage: img/{wrld|cell}/{EditorID}[_menu|_military].jpg; None|missing -> None (caller paints empty).
    pub fn background(&mut self, space: &Space, kind: &BackgroundKind) -> Option<&image::RgbaImage>;
    /// Mirrors GetWaterMask: img/wrld/{EditorID}_waterMask.png, worldspace only.
    pub fn water_mask(&mut self, space: &Space) -> Option<&image::RgbaImage>;
    pub fn compass(&mut self) -> &image::RgbaImage; // img/CompassRose.png
    /// SVG markers rasterized via resvg at Map.IconScale 1.5 (mirrors LoadSVGIcon(path, 1.5)).
    pub fn svg_marker(&mut self, rel: &str) -> tiny_skia::Pixmap; // img/mapmarker/{icon}.svg, DoorMarker.svg, InfestationMarker.svg
    /// Plot base icons img/icon/*.png sorted alphabetically (deterministic; C# GetFiles order is platform-defined).
    pub fn plot_icon_png(&mut self, index: usize) -> &image::RgbaImage; // fallback index 0 like C#
    pub fn plot_icon_count(&self) -> usize; // release has 7 (A.png..G.png)
    pub fn font_bytes(&self) -> Vec<u8>; // font/futura_condensed_bold.otf
}
```

Tests (with `MAPPALACHIA_ASSETS=/mnt/hd/mappalachia`): APPALACHIA Render background loads (30,292,814-byte jpg → dims >0), `_menu`/`_military` variants load, water mask loads, compass loads, `plot_icon_count()==7`, marker svg `VaultMarker` rasterizes non-empty, Cell-confined space missing jpg → None (e.g. made-up EditorID), font bytes non-empty.

---

### Task 4: PlotIcon assignment + compositing + canvas primitives

**Files:**
- Create: `crates/render/src/icon.rs` (PlotIcon math + tint/shadow composite)
- Create: `crates/render/src/canvas.rs` (Pixmap helpers: blit, fill_rect, stroke_rect, ellipse, line, polygon fill/stroke, draw_pixmap_centered, rotate_blit for compass/shapes)
- Modify: `lib.rs`
- Test: inline + pixel probes

Port `PlotIcon.cs` + `FormMain.cs:1418-1431`:

```rust
pub struct PlotIcon { pub do_not_use_icon: bool, pub base_index: usize, pub size: u32, pub color: Rgba }
/// Mirrors new PlotIcon(offset, palette, size, doNotUseIcon):
/// color = palette[offset % len]; base_index = (offset / len) % icon_count.
pub fn assign(offset: usize, palette: &[Rgba], size: u32, do_not_use_icon: bool, icon_count: usize) -> PlotIcon;
/// Mirrors GenerateIconImage: canvas (size+4)²; drop shadow = base tinted (128,0,0,0) at (4,4);
/// icon tinted `color` at (2,2). do_not_use_icon -> solid size×size color square.
pub fn composite(base: &image::RgbaImage, icon: &PlotIcon) -> tiny_skia::Pixmap;
```

Tint mirrors `SetColor` (RGB replace, alpha keep) via `tint_keep_alpha`. Canvas ops mirror GDI+ calls used by Map.cs: HQ bicubic stretch blit (tiny-skia default filter is bilinear — set `FilterQuality::Bicubic` on draw to match `HighQualityBicubic`; anti-aliased strokes via tiny-skia defaults). Rotation (compass NorthAngle, shape RotZ): compose `Transform::from_row` translate→rotate→translate; verify against tiny-skia 0.11 API at implementation time and pin with a test (rotated compass differs from unrotated; 0° is identity).

Tests: `assign(0, palette8, 50, false, 7)` → color OrangeRed #FF4500, base 0; `assign(8, …)` → base 1 color OrangeRed; `assign(1,…)` → Cyan; composite center pixel == tint color where base icon opaque (probe A.png center alpha>0 first, then assert); do_not_use solid square all-equal.

---

### Task 5: Text (font load, measure, draw, shadow, centered)

**Files:**
- Create: `crates/render/src/text.rs`
- Modify: `lib.rs`
- Test: inline + `assets` font

Port `Map.cs` `GetFont`/`DrawStringCentered`/`DrawStringWithDropShadow`:

```rust
pub struct Font { inner: fontdue::Font }
impl Font {
    pub fn from_bytes(bytes: &[u8]) -> Self;
    /// Mirrors MeasureString (unbounded + bounded width forms).
    pub fn measure(&self, text: &str, px: f32, wrap_width: Option<f32>) -> (f32, f32);
    /// Draw left-aligned at (x,y) with drop shadow (+2,+2, (128,0,0,0)); mirrors DrawStringWithDropShadow.
    pub fn draw(&self, canvas: &mut tiny_skia::Pixmap, text: &str, px: f32, color: Rgba, x: f32, y: f32);
    /// Mirrors DrawStringCentered: center on (cx, cy), optional top-align, optional wrap width.
    pub fn draw_centered(&self, canvas: &mut tiny_skia::Pixmap, text: &str, px: f32, color: Rgba, cx: f32, cy: f32, center_vert: bool, wrap_width: Option<f32>);
}
```

Layout metrics: fontdue `metrics()` per char + `line_gap`/advance for height; wrap by words for bounded form (legend). Alignment variants (TopLeft/TopRight/Center/CenterLeft/CenterRight/BottomLeft/BottomRight) as an enum with anchor math. Rotated labels (grid x-labels, -90°) rendered via `canvas.rs::rotate_blit`: rasterize horizontal to temp pixmap, blit rotated.

Tests: futura loads; measure("Nuka-Cola", 40) width>0 height>0; draw produces non-empty pixels; centered math: text drawn at canvas center has ink on both halves.

---

### Task 6: Pipeline draw() + typed settings + populate + CLI + golden tests

**Files:**
- Create: `crates/render/src/settings.rs` (typed Map/Plot/Heatmap/Cluster/PlotStyle/Font/Search settings + enums, serde PascalCase, palettes via `parse_color`)
- Create: `crates/render/src/draw.rs` (the pipeline)
- Create: `crates/cli/Cargo.toml` + `crates/cli/src/main.rs`
- Create: `crates/render/tests/golden.rs`
- Modify: `crates/core/src/db.rs` (add `populate_from_data(conn, &Space) -> Result<Space>` per final-review Issue 1: match by entity equality, error when missing — mirrors `Database.cs:858-861`)
- Test: golden + CLI run

Port `Map.Draw` order (Map.cs 122-209), Standard mode only (`DrawStandardPlots(topographic=false)`), spotlight tiles skipped (documented: data not in scope):

```
empty 4096 canvas (transparent, mirrors new Bitmap) 
→ background stretched to full rect (missing → leave empty)
→ compass (WhenUseful && northAngle>0.01; 512px canvas rotated, top-right)
→ brightness/grayscale whole canvas (skip when 1.0/false)
→ water mask overlay (HighlightWater && worldspace)
→ coord grid (Fine 40/Medium 20/Large 10 intervals; minor/major pens; edge labels; x-labels -90°)
→ infestations (fill (32,255,120,100) + edge (128,255,120,100) w4 + infestation SVG centered; centroid +(2048,-2048) per Map.cs:224)
→ map markers (icons at 1.5×, optional grayscale; labels below, shadowed iff no icons)
NOTE: door + infestation SVGs are scale-1 in C# (Map.cs:804 default DoorIconScale=1; GetInfestationMarker scale=1) but `svg_marker` bakes 1.5× — downscale to 2/3 when drawing these two so output matches.
→ standard plots (items sorted by legend_group; skip other spaces; region/location/shape/icon branches; VolumeDrawMode Fill/Border/Both; region levels ≤/≥/range strings)
NOTE: strokes use miter_limit 10.0 (GDI+ Pen default; tiny-skia Stroke::default is 4.0) so acute polygon joins match.
→ watermark (space name + instanced + grid range/center + game version + credit; transparent white, bottom, grid margin)
→ title (top, margin when grid)
→ legend (skip Hidden/empty; distinct (group,text); Extended widens +585; rows icon+colored text; (continued) overflow)
```

Typed settings cover every field in `Settings/*.cs` (MapSettings 20 fields incl. spotlight stub fields, PlotSettings incl. nested, FontSettings 8 sizes, SearchSettings for future egui). `populate_from_data` resolves recipe spaces (max_range==0) before transform.

CLI mirrors `Mappalachia.cs:25-73`:

```toml
# crates/cli/Cargo.toml
[package]
name = "mappalachia-cli"
version = "0.1.0"
edition = "2021"

[dependencies]
mappalachia-core = { path = "../core" }
mappalachia-render = { path = "../render" }
clap = { version = "4", features = ["derive"] }
image = "0.25"
```

```rust
// crates/cli/src/main.rs (clap)
#[derive(Parser)] struct Args { recipes: Vec<PathBuf> }
// per recipe: load (core), resolve space via populate_from_data, draw Standard,
// format = (!worldspace || background None) ? PNG : JPEG (mirrors GetImageFileTypeRecommendation),
// save beside recipe with that extension, print "Recipe {arg} drawn as image file {out}."
```

Golden tests (`render/tests/golden.rs`, need ASSETS+DB): 
1. background-only Appalachia render → 4096×4096, center pixel ≈ source jpg center (tolerance 8/channel, bicubic resample).
2. CLI on a checked-in minimal recipe (`crates/cli/tests/data/nuka_appalachia.json`, 1 Nuka item, Standard, Compact legend) → output exists, dims 4096×4096, dHash recorded in-test (first green run fills it; drift fails).
3. legend smoke: 2 items → right-side legend pixels non-empty.

Steps per task: fail → pass → commit (`feat(render/cli): …`).
