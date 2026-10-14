//! Golden tests for the Standard render pipeline + CLI
//! (`crates/render/src/draw.rs`, `crates/cli/src/main.rs`).
//!
//! Requires `MAPPALACHIA_ASSETS` (asset root) and `MAPPALACHIA_DB` (release
//! `mappalachia.db`); every test SKIPs when either is unset or missing.
//!
//! 1. `background_only_matches_source_center` — background-only Appalachia
//!    render is 4096x4096 and its center pixel matches the source jpg center
//!    (tolerance 8/channel; bicubic resample path).
//! 2. `cli_draws_minimal_recipe` — the CLI on
//!    `crates/cli/tests/data/nuka_appalachia.json` produces a 4096x4096
//!    image whose dHash matches the recorded constant (drift fails).
//! 3. `legend_smoke_right_side_nonempty` — 2 items with a right-side Compact
//!    legend leave non-empty pixels in the legend strip.
//! 4. `spotlight_size2_at_map_center_matches_tile` — Appalachia Render
//!    background with spotlight enabled (size 2, location at the space
//!    center) renders 4096x4096 through the zoomed spotlight projection;
//!    an inset patch of installed tile (-1,-1) matches its source-box mean
//!    and the tile-center pixel matches its ~2x2 source-footprint mean
//!    (tile layer paints tiles at their world rects), and the full-image
//!    dHash is pinned (drift fails).
//! 5. `spotlight_size2_grid_on_paints_grid` — same spotlight render with
//!    the coordinate grid on: 4096x4096, dark-gray grid strokes present,
//!    dHash pinned.

use std::path::{Path, PathBuf};
use std::process::Command;

use mappalachia_core::db;
use mappalachia_core::enums::LockLevel;
use mappalachia_core::model::{Coord, Entity, EntityKind, Space, SpotlightTile};
use mappalachia_core::search::{Grouped, SearchResult};
use mappalachia_render::assets::{Assets, BackgroundKind};
use mappalachia_render::canvas::straight_pixel;
use mappalachia_render::draw::{draw, prepare_items, DrawInput};
use mappalachia_render::settings::{BackgroundImageType, MapSettings, PlotSettings};
use mappalachia_render::transform::{image_rect_of_world, spot_view, world_to_image};

fn asset_root() -> Option<String> {
    let root = std::env::var("MAPPALACHIA_ASSETS").ok()?;
    if Path::new(&root).is_dir() {
        Some(root)
    } else {
        None
    }
}

fn db_conn() -> Option<rusqlite::Connection> {
    let path = std::env::var("MAPPALACHIA_DB").ok()?;
    if !Path::new(&path).exists() {
        return None;
    }
    Some(db::open_readonly(&path).unwrap())
}

fn requires() -> Option<(String, rusqlite::Connection)> {
    match (asset_root(), db_conn()) {
        (Some(a), Some(c)) => Some((a, c)),
        _ => {
            eprintln!("SKIP: set MAPPALACHIA_ASSETS and MAPPALACHIA_DB");
            None
        }
    }
}

fn appalachia(conn: &rusqlite::Connection) -> Space {
    db::all_spaces(conn)
        .unwrap()
        .into_iter()
        .find(|s| s.editor_id() == "APPALACHIA")
        .expect("Appalachia space in DB")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn background_only_matches_source_center() {
    let Some((assets_root, conn)) = requires() else {
        return;
    };
    let space = appalachia(&conn);
    let mut assets = Assets::open(&assets_root);
    let map = MapSettings {
        brightness: 1.0,
        background_image: BackgroundImageType::Render,
        ..Default::default()
    };
    let plot = PlotSettings::default();
    let input = DrawInput {
        conn: &conn,
        assets: &mut assets,
        space: space.clone(),
        map,
        plot,
        items: Vec::new(),
    };
    let img = draw(input).unwrap();
    assert_eq!((img.width(), img.height()), (4096, 4096));

    let src = assets
        .background(&space, &BackgroundKind::Render)
        .expect("Appalachia render background present")
        .clone();
    let (w, h) = src.dimensions();
    let center_src = src.get_pixel(w / 2, h / 2);
    let center_got = straight_pixel(&img, 2048, 2048);
    for (got, want) in [center_got.r, center_got.g, center_got.b].into_iter().zip([
        center_src[0],
        center_src[1],
        center_src[2],
    ]) {
        assert!(
            (i16::from(got) - i16::from(want)).abs() <= 8,
            "center pixel drifted: got ({}, {}, {}) want ({}, {}, {})",
            center_got.r,
            center_got.g,
            center_got.b,
            center_src[0],
            center_src[1],
            center_src[2]
        );
    }
}

/// dHash of the CLI output for the minimal Nuka recipe, recorded on the
/// first green run; any render drift fails this test.
const EXPECTED_DHASH: u64 = 0x88e84c464e4e0a13;

fn dhash(img: &image::RgbaImage) -> u64 {
    let luma = image::imageops::grayscale(img);
    let small = image::imageops::resize(&luma, 9, 8, image::imageops::FilterType::Triangle);
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            if small.get_pixel(x + 1, y)[0] > small.get_pixel(x, y)[0] {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    hash
}

fn cli_binary() -> PathBuf {
    let root = workspace_root();
    let status = Command::new("cargo")
        .arg("build")
        .arg("--quiet")
        .arg("-p")
        .arg("mappalachia-cli")
        .current_dir(&root)
        .status()
        .expect("cargo build -p mappalachia-cli runs");
    assert!(status.success(), "CLI must build");
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let mut bin = root.join(target).join("debug").join("mappalachia-cli");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    assert!(bin.is_file(), "CLI binary at {}", bin.display());
    bin
}

#[test]
fn cli_draws_minimal_recipe() {
    let Some(_) = requires() else {
        return;
    };
    let bin = cli_binary();
    let work = std::env::temp_dir().join(format!("mappalachia-golden-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let fixture = workspace_root().join("crates/cli/tests/data/nuka_appalachia.json");
    let recipe_copy = work.join("nuka_appalachia.json");
    std::fs::copy(&fixture, &recipe_copy).unwrap();

    let out = Command::new(&bin)
        .arg(&recipe_copy)
        .output()
        .expect("CLI runs");
    assert!(
        out.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("drawn as image file"),
        "CLI prints the C# message, got: {stdout}"
    );

    // Worldspace + background -> JPEG (mirrors
    // `GetImageFileTypeRecommendation`).
    let jpg = work.join("nuka_appalachia.jpg");
    assert!(jpg.is_file(), "CLI output beside the recipe");
    let img = image::open(&jpg).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (4096, 4096));
    let hash = dhash(&img);
    eprintln!("dHash = {hash:#018x}");
    assert_eq!(
        hash, EXPECTED_DHASH,
        "render drift: dHash changed (first green run fills EXPECTED_DHASH)"
    );
    std::fs::remove_dir_all(&work).ok();
}

fn fake_grouped(form_id: u32, editor_id: &str, space: Space) -> SearchResult {
    SearchResult::Grouped(Grouped {
        entity: Entity::new(
            form_id,
            editor_id,
            "",
            mappalachia_core::enums::Signature::Misc,
            EntityKind::Standard,
        ),
        space,
        count: 1,
        spawn_weight: 1.0,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
        legend_group: 0,
        legend_text: None,
        plot_icon: None,
    })
}

fn fake_stored(
    form_id: u32,
    editor_id: &str,
    space: Space,
    legend_group: i32,
    legend_text: &str,
    color: (u8, u8, u8),
    base_icon_index: usize,
) -> SearchResult {
    SearchResult::Grouped(Grouped {
        entity: Entity::new(
            form_id,
            editor_id,
            "",
            mappalachia_core::enums::Signature::Misc,
            EntityKind::Standard,
        ),
        space,
        count: 1,
        spawn_weight: 1.0,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
        legend_group,
        legend_text: Some(legend_text.to_string()),
        plot_icon: Some(mappalachia_core::search::StoredPlotIcon {
            do_not_use_icon: Some(false),
            base_icon_index: Some(base_icon_index),
            size: Some(50),
            color: Some(color),
        }),
    })
}

#[test]
fn shared_legend_group_renders_same_icon_color() {
    // Two items sharing a stored legend group keep the stored icon color
    // (the CLI plots stored ItemsToPlot as-is); no assets or DB needed.
    let plot = PlotSettings::default();
    let space = Space::new(1, "T", "", false, false, 0.0, 0.0, 0.0, 0.0);
    let items = prepare_items(
        &plot,
        7,
        vec![
            fake_stored(
                1,
                "Alpha2001",
                space.clone(),
                4,
                "shared text",
                (10, 20, 30),
                2,
            ),
            fake_stored(
                2,
                "Beta2002",
                space.clone(),
                4,
                "shared text",
                (10, 20, 30),
                2,
            ),
            fake_stored(
                3,
                "Gamma2003",
                space.clone(),
                5,
                "other group",
                (200, 10, 10),
                2,
            ),
        ],
    );
    assert_eq!(items.len(), 3);
    // Stored groups/text survive preparation.
    assert_eq!(items[0].legend_group, 4);
    assert_eq!(items[1].legend_group, 4);
    assert_eq!(items[0].legend_text, "shared text");
    // Same group -> the SAME icon color and base icon.
    assert_eq!(items[0].icon.color, items[1].icon.color);
    assert_eq!(items[0].icon.base_index, items[1].icon.base_index);
    assert_eq!(items[0].icon.color.r, 10);
    assert_eq!(items[0].icon.color.g, 20);
    assert_eq!(items[0].icon.color.b, 30);
    // A different group plots a different stored color.
    assert_ne!(items[0].icon.color, items[2].icon.color);
}

#[test]
fn missing_legend_fields_fall_back_to_fresh_assignment() {
    // Minimal recipes (no stored legend data) keep the old behavior: each
    // item gets its position group, computed legend text, and the FormMain
    // palette math.
    let plot = PlotSettings::default();
    let space = Space::new(1, "T", "", false, false, 0.0, 0.0, 0.0, 0.0);
    let items = prepare_items(
        &plot,
        7,
        vec![
            fake_grouped(1, "Alpha2001", space.clone()),
            fake_grouped(2, "Beta2002", space.clone()),
        ],
    );
    assert_eq!(items[0].legend_group, 0);
    assert_eq!(items[1].legend_group, 1);
    assert_eq!(items[0].legend_text, "Alpha2001");
    assert_eq!(items[1].legend_text, "Beta2002");
    assert_ne!(items[0].icon.color, items[1].icon.color);
}

#[test]
fn legend_smoke_right_side_nonempty() {
    let Some((assets_root, conn)) = requires() else {
        return;
    };
    let space = appalachia(&conn);
    let mut assets = Assets::open(&assets_root);
    let map = MapSettings {
        background_image: BackgroundImageType::None,
        ..Default::default()
    };
    let plot = PlotSettings::default();
    let icon_count = assets.plot_icon_count();
    let items = prepare_items(
        &plot,
        icon_count,
        vec![
            fake_grouped(1, "Alpha2001", space.clone()),
            fake_grouped(2, "Beta2002", space.clone()),
        ],
    );
    let input = DrawInput {
        conn: &conn,
        assets: &mut assets,
        space,
        map,
        plot,
        items,
    };
    let img = draw(input).unwrap();
    assert_eq!((img.width(), img.height()), (4096, 4096));
    // Right-side Compact legend strip (LegendWidth = 4096/7 = 585).
    let mut nonempty = 0u32;
    let mut y = 0u32;
    while y < 4096 {
        let mut x = 4096 - 585;
        while x < 4096 {
            if straight_pixel(&img, x, y).a > 0 {
                nonempty += 1;
            }
            x += 7;
        }
        y += 7;
    }
    assert!(
        nonempty > 100,
        "legend strip must be painted, got {nonempty}"
    );
}

/// dHash of the spotlight render below, recorded on the first green run;
/// any tile-layer drift fails this test.
const EXPECTED_SPOTLIGHT_DHASH: u64 = 0x752c5a5272c144cd;

#[test]
fn spotlight_size2_at_map_center_matches_tile() {
    let Some((assets_root, conn)) = requires() else {
        return;
    };
    let space = appalachia(&conn);
    let mut assets = Assets::open(&assets_root);
    let map = MapSettings {
        brightness: 1.0,
        background_image: BackgroundImageType::Render,
        spotlight_enabled: true,
        spotlight_size: 2.0,
        spotlight_location: Coord::new(space.center_x, space.center_y),
        ..Default::default()
    };
    let plot = PlotSettings::default();
    let input = DrawInput {
        conn: &conn,
        assets: &mut assets,
        space: space.clone(),
        map,
        plot,
        items: Vec::new(),
    };
    let img = draw(input).unwrap();
    assert_eq!((img.width(), img.height()), (4096, 4096));

    // Dev-data note: the 2.0.5.2 tile set pre-renders only playable space,
    // and no tile covers the space center itself (that would be tile
    // (-1, 0)). The tile-content assertion therefore targets the nearest
    // installed tile, (-1, -1), whose center maps left-of-center through
    // the zoomed spotlight projection.
    let tile = SpotlightTile::new(
        space.clone(),
        SpotlightTile::snap_to_tile_center(-4096.0),
        SpotlightTile::snap_to_tile_center(-4096.0),
    );
    assert_eq!((tile.x_id(), tile.y_id()), (-1, -1));
    let src = assets
        .spotlight_tile_uncached(space.editor_id(), -1, -1)
        .expect("spotlight tile (-1,-1) installed under img/spotlight/APPALACHIA");
    assert_eq!(src.dimensions(), (4096, 4096));

    let spot = spot_view((space.center_x, space.center_y), 2.0);
    // The tile is minified 2:1 through the spotlight projection (8192 world
    // units over 2048 canvas px at factor 4.0), so each canvas pixel
    // integrates a ~2x2 source footprint (tiny-skia Bicubic). Both
    // assertions below therefore compare against source-box MEANS located
    // through the same transform math (no hardcoded pixels).
    let (tx, ty, tw, th) = tile.rectangle();
    let (ix, iy, iw, ih) = image_rect_of_world(
        &spot,
        f64::from(tx),
        f64::from(ty),
        f64::from(tw),
        f64::from(th),
    );
    eprintln!("tile canvas rect = ({ix}, {iy}, {iw}, {ih})");
    let src_per_px = 4096.0 / f64::from(iw);
    let clamp = |v: f64| v.round().clamp(0.0, 4095.0) as u32;
    let src_box_mean = |ax: u32, ay: u32, bx: u32, by: u32| {
        let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
        // Canvas pixel (ax, ay)..=(bx, by) covers source pixels from the
        // box edges mapped through the tile rect.
        let (sx0, sy0) = (
            clamp((f64::from(ax) - f64::from(ix)) * src_per_px),
            clamp((f64::from(ay) - f64::from(iy)) * src_per_px),
        );
        let (sx1, sy1) = (
            clamp((f64::from(bx + 1) - f64::from(ix)) * src_per_px),
            clamp((f64::from(by + 1) - f64::from(iy)) * src_per_px),
        );
        eprintln!("src box = ({sx0}, {sy0})..({sx1}, {sy1})");
        let mut y = sy0;
        while y <= sy1 {
            let mut x = sx0;
            while x <= sx1 {
                let p = src.get_pixel(x, y);
                sr += u64::from(p[0]);
                sg += u64::from(p[1]);
                sb += u64::from(p[2]);
                n += 1;
                x += 1;
            }
            y += 1;
        }
        [sr / n, sg / n, sb / n]
    };

    // Single canvas pixel at the tile center (floor: the true center maps
    // to a fractional pixel, so floor keeps the footprint straddling it).
    let (fx, fy) = world_to_image(&spot, -4096.0, -4096.0);
    let (px, py) = (fx.floor() as u32, fy.floor() as u32);
    assert!(px < 4096 && py < 4096, "tile center maps on canvas");
    eprintln!("tile center -> canvas ({fx}, {fy}), probing ({px}, {py})");
    let got = straight_pixel(&img, px, py);
    let single_mean = src_box_mean(px, py, px, py);
    eprintln!(
        "single-pixel: got ({}, {}, {}) want footprint-mean ({}, {}, {})",
        got.r, got.g, got.b, single_mean[0], single_mean[1], single_mean[2]
    );

    // Inset patch (pure tile interior, clear of edge blending) vs its
    // matching source box. The zoomed tile rect can extend past the canvas
    // edge, so clamp the probe box to the canvas.
    let (x0, x1) = (
        (ix + 2.0).max(2.0) as u32,
        (ix + iw - 2.0).min(4094.0) as u32,
    );
    let (y0, y1) = (
        (iy + 2.0).max(2.0) as u32,
        (iy + ih - 2.0).min(4094.0) as u32,
    );
    let (mut pr, mut pg, mut pb, mut pn) = (0u64, 0u64, 0u64, 0u64);
    let mut y = y0;
    while y <= y1 {
        let mut x = x0;
        while x <= x1 {
            let p = straight_pixel(&img, x, y);
            pr += u64::from(p.r);
            pg += u64::from(p.g);
            pb += u64::from(p.b);
            pn += 1;
            x += 1;
        }
        y += 1;
    }
    let patch_mean = [pr / pn, pg / pn, pb / pn];
    let want_patch = src_box_mean(x0, y0, x1, y1);
    eprintln!(
        "patch-mean: got ({}, {}, {}) want ({}, {}, {}) over {pn} canvas px",
        patch_mean[0], patch_mean[1], patch_mean[2], want_patch[0], want_patch[1], want_patch[2]
    );
    // Full-image dHash, computed before the assertions so a content failure
    // still reports it.
    let raw = image::RgbaImage::from_raw(4096, 4096, img.data().to_vec())
        .expect("canvas bytes form an RGBA image");
    let hash = dhash(&raw);
    eprintln!("spotlight dHash = {hash:#018x}");
    drop(raw);
    for (got_c, want_c) in [got.r, got.g, got.b].into_iter().zip([
        single_mean[0] as u8,
        single_mean[1] as u8,
        single_mean[2] as u8,
    ]) {
        // Bicubic minification vs box-mean: measured spread (7, 9, 8).
        assert!(
            (i16::from(got_c) - i16::from(want_c)).abs() <= 20,
            "tile pixel drifted: got ({}, {}, {}) want footprint-mean ({}, {}, {})",
            got.r,
            got.g,
            got.b,
            single_mean[0],
            single_mean[1],
            single_mean[2]
        );
    }
    for (got_c, want_c) in patch_mean.into_iter().zip(want_patch) {
        // Means over ~3000 canvas px agree across resamplers; measured exact.
        assert!(
            (got_c as i64 - want_c as i64).abs() <= 8,
            "tile patch drifted: got ({}, {}, {}) want ({}, {}, {})",
            patch_mean[0],
            patch_mean[1],
            patch_mean[2],
            want_patch[0],
            want_patch[1],
            want_patch[2]
        );
    }

    assert_eq!(
        hash, EXPECTED_SPOTLIGHT_DHASH,
        "render drift: spotlight dHash changed (first green run fills EXPECTED_SPOTLIGHT_DHASH)"
    );
}

/// dHash of the spotlight+grid render below, recorded on the first green
/// run; any grid-branch drift fails this test.
const EXPECTED_SPOTLIGHT_GRID_DHASH: u64 = 0x75285a52f2c144cc;

#[test]
fn spotlight_size2_grid_on_paints_grid() {
    let Some((assets_root, conn)) = requires() else {
        return;
    };
    let space = appalachia(&conn);
    let mut assets = Assets::open(&assets_root);
    let map = MapSettings {
        brightness: 1.0,
        background_image: BackgroundImageType::Render,
        spotlight_enabled: true,
        spotlight_size: 2.0,
        spotlight_location: Coord::new(space.center_x, space.center_y),
        show_coordinate_grid: true,
        ..Default::default()
    };
    let plot = PlotSettings::default();
    let input = DrawInput {
        conn: &conn,
        assets: &mut assets,
        space: space.clone(),
        map,
        plot,
        items: Vec::new(),
    };
    let img = draw(input).unwrap();
    assert_eq!((img.width(), img.height()), (4096, 4096));

    // Grid strokes are C# `Color.DarkGray` (#A9A9A9) painted opaque over the
    // map, so their bytes are exact in the (premultiplied) canvas.
    let mut grid_px = 0u64;
    for px in img.data().as_chunks::<4>().0 {
        if *px == [169, 169, 169, 255] {
            grid_px += 1;
        }
    }
    eprintln!("spotlight grid-on exact-dark-gray pixels = {grid_px}");
    assert!(
        grid_px > 10_000,
        "grid strokes must be painted through the spotlight projection, got {grid_px}"
    );

    let raw = image::RgbaImage::from_raw(4096, 4096, img.data().to_vec())
        .expect("canvas bytes form an RGBA image");
    let hash = dhash(&raw);
    eprintln!("spotlight grid-on dHash = {hash:#018x}");
    drop(raw);
    assert_eq!(
        hash, EXPECTED_SPOTLIGHT_GRID_DHASH,
        "render drift: spotlight grid dHash changed (first green run fills EXPECTED_SPOTLIGHT_GRID_DHASH)"
    );
}
