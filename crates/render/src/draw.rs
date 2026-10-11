//! Standard-map render pipeline ported from the C# reference (READ-WRITE).
//!
//! Sources (C# reference, READ-ONLY):
//! - `Mappalachia/Map.cs:122-209` — [`draw`] ports `Map.Draw` step order.
//! - `Mappalachia/Map.cs:440-508` — standard plots (`topographic = false`).
//! - `Mappalachia/Map.cs:838-928` — region/location volume draws.
//! - `Mappalachia/Map.cs:930-992` — primitive-shape draws.
//! - `Mappalachia/Map.cs:994-1204` — title, watermark, map markers, legend.
//! - `Mappalachia/Map.cs:211-236` — infestations.
//! - `Mappalachia/Map.cs:238-342` — coordinate grid + compass rose.
//! - `Mappalachia/Map.cs:403-437,732-813` — instance FormIDs + connecting
//!   spaces (door markers).
//!
//! # Porting notes
//! - Standard mode only: [`draw`] returns
//!   [`DrawError::UnsupportedPlotMode`] for any other [`PlotMode`].
//! - Spotlight tiles are skipped (see [`draw_spotlight_tiles_stub`]): the
//!   high-res tile image set is not in scope, and the pipeline always renders
//!   at full-canvas scale (`transform::View::for_space`).
//! - Door and infestation SVGs are scale-1 in C# (`Map.cs:804` default
//!   `DoorIconScale = 1`; `GetInfestationMarker` passes no scale) while
//!   [`Assets::svg_marker`](crate::assets::Assets::svg_marker) bakes 1.5x,
//!   so both are downscaled to 2/3 (times `door_icon_scale` for doors) when
//!   drawn.
//! - Strokes use the GDI+ default miter limit 10 (see `canvas::stroke_of`).
//! - Region/location geometry is re-loaded at draw time via
//!   [`instances::load_region`]/[`instances::load_location`]: the flat
//!   [`Instance`] carries only the resolved centroid, since Rust cannot nest
//!   the volume inside the entity the way the C# object graph does.
//! - Map-marker labels always draw with a drop shadow (C#
//!   `DrawStringCentered` delegates to `DrawStringWithDropShadow`); the
//!   `!MapMarkerIcons` flag controls vertical centering, not the shadow.
//! - An infestation location with no cells is skipped (C# `Average()` would
//!   throw; crashing the whole render is not a faithful improvement).

use std::collections::{HashMap, HashSet};

use mappalachia_core::common::{to_hex_u32, CELL_SIZE, MAP_IMAGE_RESOLUTION};
use mappalachia_core::db;
use mappalachia_core::instances;
use mappalachia_core::model::{Coord, EntityKind, Space};
use mappalachia_core::search::SearchResult;
use rusqlite::Connection;

use crate::assets::{Assets, BackgroundKind};
use crate::canvas::{
    blit, draw_pixmap_centered, fill_ellipse, fill_ellipse_xform, fill_polygon, fill_rect,
    fill_rect_xform, line, pixmap_from_image, rotate_about, rotate_blit, stroke_ellipse,
    stroke_ellipse_xform, stroke_polygon, stroke_rect_xform,
};
use crate::color::{apply_brightness_grayscale, with_alpha, Rgba};
use crate::icon::{self, PlotIcon};
use crate::settings::{
    BackgroundImageType, CompassStyle, CoordinateGridPrecision, LegendHorizontalAlignment,
    LegendStyle, LegendVerticalAlignment, MapSettings, PlotMode, PlotSettings, VolumeDrawMode,
};
use crate::text::{Align, Font};
use crate::transform::{image_rect_of_world, world_len_to_image, world_to_image, View};

const CANVAS: f32 = MAP_IMAGE_RESOLUTION as f32;
/// `Map.CompassSize = MapImageResolution / 8`.
const COMPASS_SIZE: f32 = 512.0;
/// `Map.LegendWidth = MapImageResolution / 7` (integer division).
const LEGEND_WIDTH: f32 = 585.0;
const LEGEND_X_PADDING: f32 = 5.0;
const LEGEND_Y_PADDING: f32 = 5.0;
/// `Map.VolumeEdgeThickness`.
const VOLUME_EDGE: f32 = 5.0;
/// `Map.VolumeFillAlpha`.
const VOLUME_FILL_ALPHA: u8 = 64;
/// `Map.CoordinateGridTextMargin = MapImageResolution / 25` (integer division).
const GRID_MARGIN: f32 = 163.0;
/// C# `Color.DarkGray` (`#A9A9A9`).
const DARK_GRAY: Rgba = Rgba {
    r: 169,
    g: 169,
    b: 169,
    a: 255,
};
const WHITE: Rgba = Rgba {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};
/// `Map.BrushGenericTransparent`: argb(128, 255, 255, 255).
const TRANSPARENT_WHITE: Rgba = Rgba {
    r: 255,
    g: 255,
    b: 255,
    a: 128,
};
/// `Map.InfestationZoneInnerColor`: argb(32, 255, 120, 100).
const INFEST_INNER: Rgba = Rgba {
    r: 255,
    g: 120,
    b: 100,
    a: 32,
};
/// `Map.InfestationZoneEdgeColor`: argb(128, 255, 120, 100).
const INFEST_EDGE: Rgba = Rgba {
    r: 255,
    g: 120,
    b: 100,
    a: 128,
};

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// One search result prepared for plotting: legend grouping, legend text
/// and plot icon assigned. Mirrors the `FormMain` add-to-plot path
/// (`LegendGroup` per item, `PlotIcon(offset = LegendGroup, palette, size,
/// doNotUseIcon = entity is Region or Location)`).
#[derive(Debug, Clone)]
pub struct Plottable {
    pub result: SearchResult,
    pub legend_group: usize,
    pub legend_text: String,
    pub icon: PlotIcon,
}

fn grouped_of(result: &SearchResult) -> &mappalachia_core::search::Grouped {
    match result {
        SearchResult::Grouped(g) => g,
        SearchResult::Singular { inner, .. } => inner,
    }
}

fn is_volume(kind: &EntityKind) -> bool {
    matches!(kind, EntityKind::Region | EntityKind::Location)
}

/// Assign legend groups (position order), legend text and plot icons to raw
/// search results. `icon_count` is `Assets::plot_icon_count`.
pub fn prepare_items(
    plot: &PlotSettings,
    icon_count: usize,
    items: Vec<SearchResult>,
) -> Vec<Plottable> {
    let palette = &plot.plot_style_settings.palette;
    let size = plot.plot_style_settings.size.max(0) as u32;
    items
        .into_iter()
        .enumerate()
        .map(|(i, result)| {
            let g = grouped_of(&result);
            let legend_text = g.legend_text(plot.include_count_in_legend);
            let icon = PlotIcon::assign(i, palette, size, is_volume(&g.entity.kind), icon_count);
            Plottable {
                result,
                legend_group: i,
                legend_text,
                icon,
            }
        })
        .collect()
}

/// Everything [`draw`] needs.
pub struct DrawInput<'a> {
    pub conn: &'a Connection,
    pub assets: &'a mut Assets,
    pub space: Space,
    pub map: MapSettings,
    pub plot: PlotSettings,
    pub items: Vec<Plottable>,
}

/// Draw failures: database errors, or a non-Standard plot mode (only
/// Standard is ported).
#[derive(Debug)]
pub enum DrawError {
    Db(rusqlite::Error),
    UnsupportedPlotMode(String),
}

impl std::fmt::Display for DrawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(e) => write!(f, "database error: {e}"),
            Self::UnsupportedPlotMode(m) => {
                write!(f, "only Standard plot mode is supported, got {m}")
            }
        }
    }
}

impl std::error::Error for DrawError {}

impl From<rusqlite::Error> for DrawError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// C# `double.ToString()`: integral values print without a decimal point.
fn grid_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Unpremultiply one premultiplied pixel (mirrors `canvas::straight_pixel`).
fn unpremultiply(px: &[u8]) -> Rgba {
    let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
    if a == 0 || a == 255 {
        return Rgba::new(r, g, b, a);
    }
    let un = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)) as u8;
    Rgba::new(un(r), un(g), un(b), a)
}

fn premultiply(c: Rgba) -> [u8; 4] {
    let p = tiny_skia::ColorU8::from_rgba(c.r, c.g, c.b, c.a).premultiply();
    [p.red(), p.green(), p.blue(), p.alpha()]
}

/// Apply brightness/grayscale to every canvas pixel in place. Linear ops
/// commute with (un)premultiplication, so this matches the C# color matrix
/// up to rounding. Skipped entirely when `brightness == 1.0 && !grayscale`
/// (mirrors `AdjustBrightnessOrGrayscale`'s early exit).
fn adjust_canvas(pix: &mut tiny_skia::Pixmap, brightness: f32, grayscale: bool) {
    if brightness == 1.0 && !grayscale {
        return;
    }
    for px in pix.data_mut().as_chunks_mut::<4>().0 {
        let out = apply_brightness_grayscale(unpremultiply(px), brightness, grayscale);
        px.copy_from_slice(&premultiply(out));
    }
}

/// Grayscale a marker pixmap in place (the `MapMarkerGrayscale` copy path).
fn grayscale_in_place(pix: &mut tiny_skia::Pixmap) {
    for px in pix.data_mut().as_chunks_mut::<4>().0 {
        let out = apply_brightness_grayscale(unpremultiply(px), 1.0, true);
        px.copy_from_slice(&premultiply(out));
    }
}

/// Downscale a pixmap by `factor` (HQ bicubic). Used to compensate the baked
/// 1.5x on scale-1 C# markers (door, infestation).
fn downscale(src: &tiny_skia::Pixmap, factor: f32) -> tiny_skia::Pixmap {
    let dw = ((src.width() as f32 * factor).round().max(1.0)) as u32;
    let dh = ((src.height() as f32 * factor).round().max(1.0)) as u32;
    let mut out = tiny_skia::Pixmap::new(dw, dh).expect("scaled marker has nonzero size");
    blit(&mut out, src, 0.0, 0.0, dw as f32, dh as f32);
    out
}

fn draws_fill(mode: VolumeDrawMode) -> bool {
    matches!(mode, VolumeDrawMode::Fill | VolumeDrawMode::Both)
}

fn draws_border(mode: VolumeDrawMode) -> bool {
    matches!(mode, VolumeDrawMode::Border | VolumeDrawMode::Both)
}

/// Spotlight tiles intentionally skipped: the high-res tile image set is out
/// of scope, so there is nothing to composite (`Map.Draw` calls
/// `DrawSpotlightTiles` right after the background; every transform here
/// uses the full-canvas scale instead).
fn draw_spotlight_tiles_stub() {}

fn settings_background_kind(kind: BackgroundImageType) -> BackgroundKind {
    match kind {
        BackgroundImageType::Render => BackgroundKind::Render,
        BackgroundImageType::Menu => BackgroundKind::Menu,
        BackgroundImageType::Military => BackgroundKind::Military,
        BackgroundImageType::None => BackgroundKind::None,
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Port of `Map.Draw` (`Map.cs:122-209`) for Standard mode
/// (`DrawStandardPlots(topographic = false)`).
pub fn draw(input: DrawInput<'_>) -> Result<tiny_skia::Pixmap, DrawError> {
    let DrawInput {
        conn,
        assets,
        space,
        map,
        plot,
        items,
    } = input;
    if plot.mode != PlotMode::Standard {
        return Err(DrawError::UnsupportedPlotMode(format!("{:?}", plot.mode)));
    }

    let view = View::for_space(&space);
    let font = Font::from_bytes(&assets.font_bytes());

    // Empty 4096 canvas (transparent; mirrors `new Bitmap(EmptyMapImage)`).
    let mut img = tiny_skia::Pixmap::new(MAP_IMAGE_RESOLUTION as u32, MAP_IMAGE_RESOLUTION as u32)
        .expect("valid canvas size");

    // Background stretched to the full rect (missing -> leave empty).
    if map.background_image != BackgroundImageType::None {
        let kind = settings_background_kind(map.background_image);
        let bg = assets.background(&space, &kind).map(pixmap_from_image);
        if let Some(bg) = bg {
            blit(&mut img, &bg, 0.0, 0.0, CANVAS, CANVAS);
        }
    }

    draw_spotlight_tiles_stub();

    // Compass rose (512px canvas, rotated, top-right).
    let compass_useful = map.compass_style == CompassStyle::Always
        || (map.compass_style == CompassStyle::WhenUseful && space.north_angle > 0.01);
    if compass_useful {
        let rose = pixmap_from_image(assets.compass());
        rotate_blit(
            &mut img,
            &rose,
            CANVAS - COMPASS_SIZE / 2.0,
            COMPASS_SIZE / 2.0,
            COMPASS_SIZE,
            COMPASS_SIZE,
            space.north_angle as f32,
        );
    }

    // Brightness/grayscale over the whole canvas.
    adjust_canvas(&mut img, map.brightness, map.grayscale_background);

    // Water mask overlay.
    if map.highlight_water && space.is_worldspace {
        let mask = assets.water_mask(&space).map(pixmap_from_image);
        if let Some(mask) = mask {
            blit(&mut img, &mask, 0.0, 0.0, CANVAS, CANVAS);
        }
    }

    if map.show_coordinate_grid {
        draw_coordinate_grid(&mut img, &font, &map, &space, &view);
    }

    if map.show_infestations {
        draw_infestations(conn, &mut img, assets, &space, &view)?;
    }

    if map.map_marker_icons || map.map_marker_labels {
        draw_map_markers(conn, &mut img, &font, assets, &map, &space, &view)?;
    }

    let mut items = items;
    items.sort_by_key(|i| i.legend_group);

    if !items.is_empty() {
        draw_standard_plots(
            conn, &mut img, assets, &font, &map, &plot, &space, &view, &items,
        )?;

        if plot.draw_instance_form_id {
            draw_instance_form_ids(conn, &mut img, &font, &map, &space, &view, &items)?;
        }

        draw_connecting_space_plots(
            conn, &mut img, &font, assets, &map, &plot, &space, &view, &items,
        )?;
    }

    draw_watermark(conn, &mut img, &font, &map, &space)?;
    draw_title(&mut img, &font, &map);

    // Final step (may widen the image): the legend.
    if !items.is_empty() && map.legend_style != LegendStyle::Hidden {
        img = draw_legend(&img, &font, assets, &map, &plot, &items);
    }

    Ok(img)
}

// ---------------------------------------------------------------------------
// Compass/grid/infestations/markers
// ---------------------------------------------------------------------------

fn draw_coordinate_grid(
    img: &mut tiny_skia::Pixmap,
    font: &Font,
    map: &MapSettings,
    space: &Space,
    view: &View,
) {
    // Spotlight branch omitted (spotlight is out of scope).
    let target_intervals = match map.coordinate_grid_precision {
        CoordinateGridPrecision::Fine => 40.0,
        CoordinateGridPrecision::Medium => 20.0,
        CoordinateGridPrecision::Large => 10.0,
    };
    let view_range = space.max_range;
    let grid_size = 2f64.powf((view_range / target_intervals).log2().round());
    let min_x = space.min_x() - (grid_size + (space.min_x() % grid_size));
    let max_x = space.max_x() + (grid_size - (space.max_x() % grid_size));
    let min_y = space.min_y() - (grid_size + (space.min_y() % grid_size));
    let max_y = space.max_y() + (grid_size - (space.max_y() % grid_size));

    let px = (map.font_settings.size_coordinate_grid) as f32;

    let mut y = min_y;
    while y <= max_y {
        let major = (y / grid_size) % 4.0 == 0.0;
        let width = if major { 3.0 } else { 1.0 };
        let (lx, ly) = world_to_image(view, min_x, y);
        let (rx, ry) = world_to_image(view, max_x, y);
        line(img, lx, ly, rx, ry, DARK_GRAY, width);
        let text = grid_num(y);
        let (w, h) = font.measure(&text, px, None);
        font.draw_plain(img, &text, px, WHITE, 0.0, ly - h);
        font.draw_plain(img, &text, px, WHITE, CANVAS - w, ly - h);
        y += grid_size;
    }

    let mut x = min_x;
    while x <= max_x {
        let major = (x / grid_size) % 4.0 == 0.0;
        let width = if major { 3.0 } else { 1.0 };
        let (bx, _) = world_to_image(view, x, min_y);
        let (tx, ty) = world_to_image(view, x, max_y);
        line(
            img,
            bx,
            world_to_image(view, x, min_y).1,
            tx,
            ty,
            DARK_GRAY,
            width,
        );
        let text = grid_num(x);
        let (tw, th) = font.measure(&text, px, None);
        // Net placements derived from the C# translate/rotate(-90)/draw
        // sequence (see module notes): right edge at bx, strips tw tall.
        font.draw_rotated(img, &text, px, WHITE, bx - th / 2.0, tw / 2.0, -90.0, None);
        font.draw_rotated(
            img,
            &text,
            px,
            WHITE,
            bx - th / 2.0,
            CANVAS - tw / 2.0,
            -90.0,
            None,
        );
        x += grid_size;
    }
}

fn draw_infestations(
    conn: &Connection,
    img: &mut tiny_skia::Pixmap,
    assets: &mut Assets,
    space: &Space,
    view: &View,
) -> Result<(), DrawError> {
    let radius = db::infestation_radius(conn)? as f32;
    for location in instances::infestation_locations(conn, space)? {
        if location.cells.is_empty() {
            continue;
        }
        let coords: Vec<Coord> = location.cells.iter().map(|c| c.coord()).collect();
        let center = mappalachia_core::geometry::centroid(&coords);
        // `trueCentroid = centroid + (CellSize/2, -CellSize/2)` (Map.cs:224).
        let tx = center.x + f64::from(CELL_SIZE) / 2.0;
        let ty = center.y - f64::from(CELL_SIZE) / 2.0;
        let r = f64::from(radius);
        let (ix, iy, iw, ih) = image_rect_of_world(view, tx - r, ty + r, r * 2.0, r * 2.0);
        fill_ellipse(img, ix, iy, iw, ih, INFEST_INNER);
        stroke_ellipse(img, ix, iy, iw, ih, INFEST_EDGE, 4.0);
        let (cx, cy) = world_to_image(view, tx, ty);
        let marker = downscale(&assets.infestation_marker(), 2.0 / 3.0);
        draw_pixmap_centered(img, &marker, cx, cy);
    }
    Ok(())
}

fn draw_map_markers(
    conn: &Connection,
    img: &mut tiny_skia::Pixmap,
    font: &Font,
    assets: &mut Assets,
    map: &MapSettings,
    space: &Space,
    view: &View,
) -> Result<(), DrawError> {
    let px = (map.font_settings.size_map_marker_label) as f32;
    for marker in db::map_markers_for_space(conn, space.form_id())? {
        let (x, y) = world_to_image(view, marker.coord.x, marker.coord.y);
        let mut label_offset = 0.0;
        if map.map_marker_icons {
            let mut icon = assets.map_marker_icon(&marker.icon);
            if map.map_marker_grayscale {
                grayscale_in_place(&mut icon);
            }
            label_offset = (icon.height()) as f32 / 2.0;
            draw_pixmap_centered(img, &icon, x, y);
        }
        if map.map_marker_labels {
            font.draw_centered(
                img,
                &marker.label,
                px,
                WHITE,
                x,
                y + label_offset,
                !map.map_marker_icons,
                None,
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Standard plots
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_standard_plots(
    conn: &Connection,
    img: &mut tiny_skia::Pixmap,
    assets: &mut Assets,
    font: &Font,
    map: &MapSettings,
    plot: &PlotSettings,
    space: &Space,
    view: &View,
    items: &[Plottable],
) -> Result<(), DrawError> {
    for item in items {
        if item.result.space() != space {
            continue;
        }
        let item_space = item.result.space().clone();
        let resolved = instances::instances(conn, item.result.clone(), &item_space)?;
        // `PlotIcon.GetImage()` composite, cached per item like the C# cache.
        let composed = if is_volume(&grouped_of(&item.result).entity.kind) {
            None
        } else {
            let base = assets.plot_icon_png(item.icon.base_index).clone();
            Some(icon::composite(&base, &item.icon))
        };
        for instance in &resolved {
            let color = item.icon.color;
            match &instance.entity.kind {
                EntityKind::Region => {
                    draw_region(img, font, instance, item, map, plot, view, conn)?;
                }
                EntityKind::Location => {
                    draw_location(img, instance, item, plot, view, conn)?;
                }
                _ if instance.primitive_shape.is_some() => {
                    draw_primitive_shape(img, instance, color, plot.volume_draw_mode, view);
                }
                _ => {
                    let (x, y) = world_to_image(view, instance.coord.x, instance.coord.y);
                    draw_pixmap_centered(
                        img,
                        composed.as_ref().expect("icon composite for point plots"),
                        x,
                        y,
                    );
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_region(
    img: &mut tiny_skia::Pixmap,
    font: &Font,
    instance: &mappalachia_core::model::Instance,
    item: &Plottable,
    map: &MapSettings,
    plot: &PlotSettings,
    view: &View,
    conn: &Connection,
) -> Result<(), DrawError> {
    let item_space = item.result.space().clone();
    let Some(region) = instances::load_region(conn, &instance.entity.editor_id, &item_space)?
    else {
        return Ok(());
    };
    let edge = item.icon.color;
    let fill = with_alpha(edge, VOLUME_FILL_ALPHA);
    let px = (map.font_settings.size_region_level) as f32;
    for subregion in region.all_sub_regions() {
        let points: Vec<(f32, f32)> = subregion
            .iter()
            .map(|p| world_to_image(view, p.coord().x, p.coord().y))
            .collect();
        if draws_fill(plot.volume_draw_mode) {
            fill_polygon(img, &points, fill);
        }
        if draws_border(plot.volume_draw_mode) {
            stroke_polygon(img, &points, edge, VOLUME_EDGE);
        }
        if plot.show_region_levels {
            if region.min_level == 0 && region.max_level == 0 {
                continue;
            }
            let level = if region.min_level <= 1 && region.max_level != 0 {
                format!("≤{}", region.max_level)
            } else if region.min_level != 0 && region.max_level == 0 {
                format!("≥{}", region.min_level)
            } else {
                format!("{}-{}", region.min_level, region.max_level)
            };
            let coords: Vec<Coord> = subregion.iter().map(|p| p.coord()).collect();
            let c = mappalachia_core::geometry::centroid(&coords);
            let (x, y) = world_to_image(view, c.x, c.y);
            font.draw_centered(img, &level, px, edge, x, y, true, None);
        }
    }
    Ok(())
}

fn draw_location(
    img: &mut tiny_skia::Pixmap,
    instance: &mappalachia_core::model::Instance,
    item: &Plottable,
    plot: &PlotSettings,
    view: &View,
    conn: &Connection,
) -> Result<(), DrawError> {
    let item_space = item.result.space().clone();
    let location = instances::load_location(
        conn,
        instance.entity.form_id,
        &instance.entity.editor_id,
        &instance.entity.display_name,
        &item_space,
    )?;
    let edge = item.icon.color;
    let fill = with_alpha(edge, VOLUME_FILL_ALPHA);
    let half = VOLUME_EDGE / 2.0;
    let cell = f64::from(CELL_SIZE);
    for c in &location.cells {
        let (ix, iy, iw, ih) = image_rect_of_world(view, c.coord().x, c.coord().y, cell, cell);
        if draws_fill(plot.volume_draw_mode) {
            fill_rect(img, ix, iy, iw, ih, fill);
        }
        if draws_border(plot.volume_draw_mode) {
            let n = c.neighbors_in(&location.cells);
            if !n.up {
                line(img, ix, iy + half, ix + iw, iy + half, edge, VOLUME_EDGE);
            }
            if !n.down {
                line(
                    img,
                    ix,
                    iy + ih - half,
                    ix + iw,
                    iy + ih - half,
                    edge,
                    VOLUME_EDGE,
                );
            }
            if !n.left {
                line(img, ix + half, iy + ih, ix + half, iy, edge, VOLUME_EDGE);
            }
            if !n.right {
                line(
                    img,
                    ix + iw - half,
                    iy + ih,
                    ix + iw - half,
                    iy,
                    edge,
                    VOLUME_EDGE,
                );
            }
        }
    }
    Ok(())
}

fn draw_primitive_shape(
    img: &mut tiny_skia::Pixmap,
    instance: &mappalachia_core::model::Instance,
    color: Rgba,
    mode: VolumeDrawMode,
    view: &View,
) {
    let shape = instance.primitive_shape.expect("shape branch has a shape");
    let (cx, cy) = world_to_image(view, instance.coord.x, instance.coord.y);
    let (tx, ty) = world_to_image(
        view,
        instance.coord.x - shape.bound_x / 2.0,
        instance.coord.y + shape.bound_y / 2.0,
    );
    let w = world_len_to_image(view, shape.bound_x);
    let h = world_len_to_image(view, shape.bound_y);
    // `TranslateTransform(c); RotateTransform(r); TranslateTransform(-c)`.
    let xf = rotate_about(shape.rot_z as f32, cx, cy);
    let fill = with_alpha(color, VOLUME_FILL_ALPHA);
    use mappalachia_core::enums::ShapeType;
    match shape.shape_type {
        ShapeType::Box | ShapeType::Line | ShapeType::Plane => {
            if draws_fill(mode) {
                fill_rect_xform(img, tx, ty, w, h, fill, xf);
            }
            if draws_border(mode) {
                stroke_rect_xform(img, tx, ty, w, h, color, VOLUME_EDGE, xf);
            }
        }
        ShapeType::Sphere | ShapeType::Ellipsoid | ShapeType::Cylinder => {
            if draws_fill(mode) {
                fill_ellipse_xform(img, tx, ty, w, h, fill, xf);
            }
            if draws_border(mode) {
                stroke_ellipse_xform(img, tx, ty, w, h, color, VOLUME_EDGE, xf);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_instance_form_ids(
    conn: &Connection,
    img: &mut tiny_skia::Pixmap,
    font: &Font,
    map: &MapSettings,
    space: &Space,
    view: &View,
    items: &[Plottable],
) -> Result<(), DrawError> {
    let _ = space;
    let px = (map.font_settings.size_instance_form_id) as f32;
    for item in items {
        if is_volume(&grouped_of(&item.result).entity.kind) {
            continue;
        }
        let item_space = item.result.space().clone();
        for instance in instances::instances(conn, item.result.clone(), &item_space)? {
            let (x, y) = world_to_image(view, instance.coord.x, instance.coord.y);
            font.draw_centered(
                img,
                &to_hex_u32(instance.instance_form_id),
                px,
                item.icon.color,
                x,
                y + (item.icon.size) as f32 / 2.0,
                false,
                None,
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_connecting_space_plots(
    conn: &Connection,
    img: &mut tiny_skia::Pixmap,
    font: &Font,
    assets: &mut Assets,
    map: &MapSettings,
    plot: &PlotSettings,
    space: &Space,
    view: &View,
    items: &[Plottable],
) -> Result<(), DrawError> {
    let selected_teleporters = instances::teleporters(conn, space)?;
    let mut connections: HashMap<Space, i32> = HashMap::new();
    let px = (map.font_settings.size_items_in_other_spaces) as f32;
    for item in items {
        let item_space = item.result.space().clone();
        if item_space == *space && !plot.auto_find_plots_in_connected_spaces {
            continue;
        }
        let mut teleporters = instances::teleporters(conn, &item_space)?;
        teleporters.extend(selected_teleporters.iter().cloned());
        // `DistinctBy(i => (i.Space, i.TeleportsTo))`, first wins.
        let mut seen = HashSet::new();
        teleporters.retain(|t| {
            seen.insert((
                t.space.form_id(),
                t.teleports_to.as_ref().map(|s| s.form_id()),
            ))
        });
        for teleporter in &teleporters {
            if !plot.auto_find_plots_in_connected_spaces
                && teleporter.teleports_to.as_ref() != Some(space)
                && teleporter.teleports_to.as_ref() != Some(&item_space)
            {
                continue;
            }
            if item_space != teleporter.space
                && teleporter.teleports_to.as_ref() != Some(&item_space)
            {
                continue;
            }
            if teleporter.space != *space {
                continue;
            }
            let Some(to_space) = teleporter.teleports_to.clone() else {
                continue;
            };
            if matches!(
                grouped_of(&item.result).entity.kind,
                EntityKind::DerivedFlux(_)
            ) && !to_space.is_nukable()
            {
                continue;
            }
            let found = instances::instances(conn, item.result.clone(), &to_space)?;
            if found.is_empty() {
                continue;
            }
            let n = {
                let entry = connections.entry(to_space.clone()).or_insert(0);
                *entry += 1;
                *entry
            };
            let (x, y) = world_to_image(view, teleporter.coord.x, teleporter.coord.y);
            let door = downscale(
                &assets.door_marker(),
                map.door_icon_scale as f32 * 2.0 / 3.0,
            );
            draw_pixmap_centered(img, &door, x, y);
            let label = format!(
                "{}: {} ({})",
                to_space.display_name(),
                item.legend_text,
                found.len()
            );
            font.draw_centered(
                img,
                &label,
                px,
                item.icon.color,
                x,
                y + 20.0 + 25.0 * (n) as f32,
                false,
                None,
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Watermark / title / legend
// ---------------------------------------------------------------------------

fn draw_watermark(
    conn: &Connection,
    img: &mut tiny_skia::Pixmap,
    font: &Font,
    map: &MapSettings,
    space: &Space,
) -> Result<(), DrawError> {
    let px = (map.font_settings.size_watermark) as f32;
    let left = map.legend_horizontal_alignment == LegendHorizontalAlignment::Left;
    let mut text = format!("{} ({})", space.display_name(), space.editor_id());
    if space.is_instanceable {
        text.push_str(" (Instanced)");
    }
    if map.show_coordinate_grid {
        text = format!(
            "Range: {}\nCenter: {}, {}\n{text}",
            space.max_range as i32, space.center_x as i32, space.center_y as i32
        );
    }
    text.push_str(&format!(
        "\nGame Version {} | Made with Mappalachia: github.com/AHeroicLlama/Mappalachia",
        db::game_version(conn)?
    ));
    let position = if map.show_coordinate_grid {
        -GRID_MARGIN
    } else {
        0.0
    };
    let rect = (
        if left { position } else { -position },
        position,
        CANVAS,
        CANVAS,
    );
    font.draw_aligned(
        img,
        &text,
        px,
        TRANSPARENT_WHITE,
        rect,
        if left {
            Align::BottomRight
        } else {
            Align::BottomLeft
        },
        None,
    );
    Ok(())
}

fn draw_title(img: &mut tiny_skia::Pixmap, font: &Font, map: &MapSettings) {
    if map.title.trim().is_empty() {
        return;
    }
    let px = (map.font_settings.size_title) as f32;
    let left = map.legend_horizontal_alignment == LegendHorizontalAlignment::Left;
    let padding = if map.show_coordinate_grid {
        GRID_MARGIN
    } else {
        0.0
    };
    let rect = (
        if left { -padding } else { padding },
        padding,
        CANVAS,
        CANVAS,
    );
    font.draw_aligned(
        img,
        &map.title,
        px,
        WHITE,
        rect,
        if left {
            Align::TopRight
        } else {
            Align::TopLeft
        },
        None,
    );
}

fn draw_legend(
    img: &tiny_skia::Pixmap,
    font: &Font,
    assets: &mut Assets,
    map: &MapSettings,
    _plot: &PlotSettings,
    items: &[Plottable],
) -> tiny_skia::Pixmap {
    let left = map.legend_horizontal_alignment == LegendHorizontalAlignment::Left;
    let extended = map.legend_style == LegendStyle::Extended;
    let mut out = if extended {
        let mut big = tiny_skia::Pixmap::new(
            MAP_IMAGE_RESOLUTION as u32 + LEGEND_WIDTH as u32,
            MAP_IMAGE_RESOLUTION as u32,
        )
        .expect("valid legend canvas");
        blit(
            &mut big,
            img,
            if left { LEGEND_WIDTH } else { 0.0 },
            0.0,
            CANVAS,
            CANVAS,
        );
        big
    } else {
        img.clone()
    };

    // `DistinctBy(item => (item.LegendGroup, item.LegendText))`, order kept.
    let mut seen = HashSet::new();
    let rows: Vec<&Plottable> = items
        .iter()
        .filter(|i| seen.insert((i.legend_group, i.legend_text.clone())))
        .collect();

    let px = (map.font_settings.size_legend) as f32;
    let format = if left {
        Align::CenterLeft
    } else {
        Align::CenterRight
    };
    let ext = if extended { LEGEND_WIDTH } else { 0.0 };

    let mut measured = Vec::with_capacity(rows.len());
    let mut total = 0.0f32;
    for item in &rows {
        let wrap = LEGEND_WIDTH - (item.icon.size) as f32 - LEGEND_X_PADDING * 2.0;
        let (w, h) = font.measure(&item.legend_text, px, Some(wrap));
        let row_h = h.max((item.icon.size) as f32 + LEGEND_Y_PADDING);
        total += row_h;
        measured.push((*item, wrap, w, h, row_h));
    }

    let mut y = match map.legend_vertical_alignment {
        LegendVerticalAlignment::Top => 0.0,
        LegendVerticalAlignment::Center => CANVAS / 2.0 - total / 2.0,
        LegendVerticalAlignment::Bottom => CANVAS - total,
    };

    for (item, wrap, bw, bh, row_h) in measured {
        let half = row_h / 2.0;
        let mid = y + half;
        let icon_offset = LEGEND_X_PADDING + (item.icon.size) as f32 / 2.0;
        let (icon_x, text_x) = if left {
            (icon_offset, icon_offset * 2.0)
        } else {
            (
                CANVAS - icon_offset + ext,
                CANVAS - icon_offset * 2.0 - bw + ext,
            )
        };
        font.draw_aligned(
            &mut out,
            &item.legend_text,
            px,
            item.icon.color,
            (text_x, mid - bh / 2.0, bw, bh),
            format,
            Some(wrap),
        );
        if !is_volume(&grouped_of(&item.result).entity.kind) {
            let base = assets.plot_icon_png(item.icon.base_index).clone();
            let composed = icon::composite(&base, &item.icon);
            draw_pixmap_centered(&mut out, &composed, icon_x, mid);
        }
        y += half * 2.0;
    }

    if total > CANVAS {
        let (w, h) = font.measure("(continued)", px, None);
        let x = if left {
            LEGEND_WIDTH
        } else {
            CANVAS - LEGEND_WIDTH - w
        };
        font.draw_aligned(
            &mut out,
            "(continued)",
            px,
            WHITE,
            (x, CANVAS - h, w, h),
            Align::BottomLeft,
            None,
        );
    }

    out
}
