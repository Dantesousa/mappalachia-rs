//! CLI entry point, mirroring `Mappalachia/Mappalachia.cs:25-73`.
//!
//! For every `.json` recipe argument: load the recipe, resolve its space via
//! [`populate_from_data`](mappalachia_core::db::populate_from_data) (like
//! `Recipe.AsSettings` populating the recipe and item spaces), draw it in
//! Standard mode, and save beside the recipe. The image format mirrors
//! `FileIO.GetImageFileTypeRecommendation`: PNG when the space is not a
//! worldspace or the background is `None`, otherwise JPEG.
//!
//! The database comes from `MAPPALACHIA_DB` and the asset tree from
//! `MAPPALACHIA_ASSETS` (both must be set; there is no GUI fallback).

use std::path::PathBuf;

use clap::Parser;
use mappalachia_core::db;
use mappalachia_core::recipe::load_recipe_file;
use mappalachia_render::assets::Assets;
use mappalachia_render::canvas::straight_pixel;
use mappalachia_render::draw::{draw, prepare_items, DrawInput};
use mappalachia_render::settings::{map_from_value, plot_from_value, BackgroundImageType};

#[derive(Parser)]
struct Args {
    /// Recipe files (`.json`) to render.
    recipes: Vec<PathBuf>,
}

/// Straight-alpha canvas pixels to an [`image::RgbaImage`] for saving.
fn to_rgba_image(pix: &tiny_skia::Pixmap) -> image::RgbaImage {
    let (w, h) = (pix.width(), pix.height());
    let mut out = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let c = straight_pixel(pix, x, y);
            out.put_pixel(x, y, image::Rgba([c.r, c.g, c.b, c.a]));
        }
    }
    out
}

fn run_recipe(
    conn: &rusqlite::Connection,
    assets: &mut Assets,
    arg: &PathBuf,
) -> Result<(), String> {
    let recipe = load_recipe_file(arg)?;
    let space = db::populate_from_data(conn, &recipe.space)
        .map_err(|e| format!("space {} not in database: {e}", recipe.space.editor_id()))?;
    let map = map_from_value(recipe.map_settings.raw())?;
    let plot = plot_from_value(recipe.plot_settings.raw())?;

    // `Recipe.AsSettings` populates every item space too.
    let mut items = Vec::with_capacity(recipe.items_to_plot.len());
    for result in recipe.items_to_plot {
        let resolved = db::populate_from_data(conn, result.space()).map_err(|e| {
            format!(
                "item space {} not in database: {e}",
                result.space().editor_id()
            )
        })?;
        let mut owned = result;
        match &mut owned {
            mappalachia_core::search::SearchResult::Grouped(g) => g.space = resolved,
            mappalachia_core::search::SearchResult::Singular { inner, .. } => {
                inner.space = resolved;
            }
        }
        items.push(owned);
    }

    let icon_count = assets.plot_icon_count();
    let items = prepare_items(&plot, icon_count, items);
    let worldspace = space.is_worldspace;
    let background_none = map.background_image == BackgroundImageType::None;
    let img = draw(DrawInput {
        conn,
        assets,
        space,
        map,
        plot,
        items,
    })
    .map_err(|e| e.to_string())?;

    // Mirrors `GetImageFileTypeRecommendation`.
    let jpeg = worldspace && !background_none;
    let ext = if jpeg { "jpg" } else { "png" };
    let out = arg.with_extension(ext);
    let rgba = to_rgba_image(&img);
    if jpeg {
        // JPEG has no alpha: GDI+ encodes the raw RGB (transparent reads as
        // black), so flatten over black.
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
        rgb.save(&out).map_err(|e| e.to_string())?;
    } else {
        rgba.save(&out).map_err(|e| e.to_string())?;
    }

    println!(
        "Recipe {} drawn as image file {}.",
        arg.display(),
        out.display()
    );
    Ok(())
}

fn main() {
    let args = Args::parse();
    let (db_path, assets_root) = match (
        std::env::var("MAPPALACHIA_DB").ok(),
        std::env::var("MAPPALACHIA_ASSETS").ok(),
    ) {
        (Some(db), Some(assets)) => (db, assets),
        _ => match mappalachia_render::assets::exe_adjacent() {
            Some((db, assets)) => (
                db.to_string_lossy().into_owned(),
                assets.to_string_lossy().into_owned(),
            ),
            None => {
                eprintln!(
                    "error: set MAPPALACHIA_DB and MAPPALACHIA_ASSETS, or place data/ img/ font/ next to the executable"
                );
                std::process::exit(1);
            }
        },
    };
    let conn = match db::open_readonly(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot open database {db_path}: {e}");
            std::process::exit(1);
        }
    };
    let mut assets = Assets::open(&assets_root);
    let mut failed = false;
    for arg in &args.recipes {
        if arg
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            if !arg.is_file() {
                continue;
            }
            if let Err(e) = run_recipe(&conn, &mut assets, arg) {
                eprintln!("error: {}: {e}", arg.display());
                failed = true;
            }
        } else if arg.to_string_lossy().eq_ignore_ascii_case("dedication") {
            println!("Dedicated to Molly.");
        }
    }
    if failed {
        std::process::exit(1);
    }
}
