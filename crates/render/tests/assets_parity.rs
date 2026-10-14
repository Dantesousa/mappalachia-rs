//! Parity tests for [`mappalachia_render::assets::Assets`] against the
//! release asset tree (mirrors C# `FileIO.cs` lines 105-270 + `Paths.cs`).
//!
//! Requires `MAPPALACHIA_ASSETS` to point at the asset root (e.g.
//! `/mnt/hd/mappalachia`); all tests SKIP when unset or missing.

use mappalachia_core::model::Space;
use mappalachia_render::assets::{Assets, BackgroundKind};

fn asset_root() -> Option<String> {
    let root = std::env::var("MAPPALACHIA_ASSETS").ok()?;
    if std::path::Path::new(&root).is_dir() {
        Some(root)
    } else {
        None
    }
}

fn appalachia() -> Space {
    Space::new(
        0,
        "APPALACHIA",
        "Appalachia",
        true,
        true,
        0.0,
        0.0,
        1.0,
        0.0,
    )
}

fn missing_cell() -> Space {
    Space::new(
        0,
        "MADEUPCELL123NOTREAL",
        "Made-up cell",
        false,
        false,
        0.0,
        0.0,
        1.0,
        0.0,
    )
}

#[test]
fn render_background_loads() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    let img = assets
        .background(&appalachia(), &BackgroundKind::Render)
        .expect("APPALACHIA render background should load");
    assert!(img.width() > 0 && img.height() > 0);
}

#[test]
fn menu_and_military_backgrounds_load() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    for kind in [BackgroundKind::Menu, BackgroundKind::Military] {
        let img = assets
            .background(&appalachia(), &kind)
            .expect("APPALACHIA menu/military background should load");
        assert!(img.width() > 0 && img.height() > 0);
    }
}

#[test]
fn water_mask_loads() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    let img = assets
        .water_mask(&appalachia())
        .expect("APPALACHIA water mask should load");
    assert!(img.width() > 0 && img.height() > 0);
}

#[test]
fn compass_loads() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    let img = assets.compass();
    assert!(img.width() > 0 && img.height() > 0);
}

#[test]
fn plot_icon_count_is_seven() {
    let Some(root) = asset_root() else { return };
    let assets = Assets::open(&root);
    assert_eq!(assets.plot_icon_count(), 7);
}

#[test]
fn plot_icon_png_loads() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    let img = assets.plot_icon_png(0);
    assert!(img.width() > 0 && img.height() > 0);
}

#[test]
fn vault_marker_svg_rasterizes() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    let pix = assets.map_marker_icon("VaultMarker");
    assert!(pix.width() > 0 && pix.height() > 0);
    assert!(
        pix.data().iter().any(|&b| b != 0),
        "rasterized VaultMarker should contain non-empty pixels"
    );
}

#[test]
fn missing_cell_background_is_none() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    assert!(assets
        .background(&missing_cell(), &BackgroundKind::Render)
        .is_none());
    assert!(assets.water_mask(&missing_cell()).is_none());
    assert!(assets
        .background(&appalachia(), &BackgroundKind::None)
        .is_none());
}

#[test]
fn font_bytes_non_empty() {
    let Some(root) = asset_root() else { return };
    let assets = Assets::open(&root);
    assert!(!assets.font_bytes().is_empty());
}

#[test]
fn spotlight_tile_loads() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    let img = assets
        .spotlight_tile("APPALACHIA", -1, -1)
        .expect("APPALACHIA tile (-1,-1) should load from dev data");
    assert!(img.width() > 0 && img.height() > 0);
}

#[test]
fn missing_spotlight_tile_is_none() {
    let Some(root) = asset_root() else { return };
    let mut assets = Assets::open(&root);
    assert!(assets.spotlight_tile("APPALACHIA", 9999, 9999).is_none());
    assert!(assets
        .spotlight_tile("MADEUPCELL123NOTREAL", 0, 0)
        .is_none());
}

#[test]
fn spotlight_installed_on_dev_data() {
    let Some(root) = asset_root() else { return };
    assert!(Assets::open(&root).is_spotlight_installed());
}

#[test]
fn spotlight_not_installed_without_tiles() {
    // Pure tempdir test: no data required (mirrors `IsSpotlightInstalled`).
    let dir = std::env::temp_dir().join(format!(
        "mappal-spotlight-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create tmpdir");
    // No spotlight dir at all.
    assert!(!Assets::open(dir.to_str().expect("tmpdir utf8")).is_spotlight_installed());
    // Subfolder with only a 0-byte jpg still counts as NOT installed.
    let sub = dir.join("img/spotlight/APPALACHIA");
    std::fs::create_dir_all(&sub).expect("create tile subdir");
    std::fs::write(sub.join("0.0.jpg"), b"").expect("write empty jpg");
    assert!(!Assets::open(dir.to_str().expect("tmpdir utf8")).is_spotlight_installed());
    // One non-0-byte jpg flips the check.
    std::fs::write(sub.join("0.0.jpg"), b"x").expect("write nonzero jpg");
    assert!(Assets::open(dir.to_str().expect("tmpdir utf8")).is_spotlight_installed());
    let _ = std::fs::remove_dir_all(&dir);
}
