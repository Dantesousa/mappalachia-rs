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

use std::path::{Path, PathBuf};
use std::process::Command;

use mappalachia_core::db;
use mappalachia_core::enums::LockLevel;
use mappalachia_core::model::{Entity, EntityKind, Space};
use mappalachia_core::search::{Grouped, SearchResult};
use mappalachia_render::assets::{Assets, BackgroundKind};
use mappalachia_render::canvas::straight_pixel;
use mappalachia_render::draw::{draw, prepare_items, DrawInput};
use mappalachia_render::settings::{BackgroundImageType, MapSettings, PlotSettings};

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
