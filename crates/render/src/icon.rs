//! PlotIcon assignment + compositing.
//!
//! Mirrors C# `Mappalachia/PlotIcon.cs` (constructor math and
//! `GenerateIconImage`) and the call sites in
//! `Mappalachia/Form/FormMain.cs:1418-1431`:
//! `new PlotIcon(offset = LegendGroup, palette, size = 50,
//! doNotUseIcon = entity is Region or Location)`.

use crate::canvas;
use crate::color::{tint_keep_alpha, Rgba};

/// Mirrors C# `Map.DropShadowOffset` (`Map.cs:73`).
pub const DROP_SHADOW_OFFSET: u32 = 2;

/// Mirrors C# `Map.DropShadowColor` (`Map.cs:91`): argb(128, 0, 0, 0).
pub const DROP_SHADOW_COLOR: Rgba = Rgba {
    r: 0,
    g: 0,
    b: 0,
    a: 128,
};

/// Which base icon + palette color a plotted group gets.
/// Mirrors the `PlotIcon` constructor inputs after `FileIO` resolution
/// (the raw image itself is loaded separately via
/// [`crate::assets::Assets::plot_icon_png`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlotIcon {
    pub do_not_use_icon: bool,
    pub base_index: usize,
    pub size: u32,
    pub color: Rgba,
}

impl PlotIcon {
    /// Mirrors `new PlotIcon(offset, palette, size, doNotUseIcon)`:
    /// `color = palette[offset % palette.len()]`;
    /// `base_index = (offset / palette.len()) % icon_count`
    /// (`icon_count` = `FileIO.GetAvailablePlotIconCount`; the release
    /// tree ships 7: `A.png..G.png`).
    pub fn assign(
        offset: usize,
        palette: &[Rgba],
        size: u32,
        do_not_use_icon: bool,
        icon_count: usize,
    ) -> Self {
        assert!(!palette.is_empty(), "palette must not be empty");
        assert!(icon_count > 0, "icon_count must not be zero");
        Self {
            do_not_use_icon,
            base_index: (offset / palette.len()) % icon_count,
            size,
            color: palette[offset % palette.len()],
        }
    }

    /// Canvas edge: `Size + DropShadowOffset * 2` (`PlotIcon.cs:131`).
    pub fn canvas_size(&self) -> u32 {
        self.size + DROP_SHADOW_OFFSET * 2
    }
}

/// Tint a copy of `base`, mirroring `SetColor` (RGB replaced, alpha kept).
pub fn tint_image(base: &image::RgbaImage, color: Rgba) -> image::RgbaImage {
    let mut out = base.clone();
    for px in out.pixels_mut() {
        let tinted = tint_keep_alpha(Rgba::new(px[0], px[1], px[2], px[3]), color);
        *px = image::Rgba([tinted.r, tinted.g, tinted.b, tinted.a]);
    }
    out
}

/// Mirrors `GenerateIconImage(color)` (`PlotIcon.cs:112-152`):
/// a `(size+4)^2` canvas holding the base icon stretched to `size x size`,
/// tinted with the drop-shadow color at (4,4), overdrawn with the same icon
/// tinted `icon.color` at (2,2) — both stretches HQ-bicubic. Strokes and the
/// pattern use tiny-skia defaults (anti-aliased), matching
/// `GraphicsFromImageHQ`.
///
/// `do_not_use_icon` (Region/Location entries) mirrors the early return
/// (`PlotIcon.cs:114-121`): a solid `size x size` square of the color.
pub fn composite(base: &image::RgbaImage, icon: &PlotIcon) -> tiny_skia::Pixmap {
    if icon.do_not_use_icon {
        return canvas::solid_pixmap(icon.size, icon.size, icon.color);
    }
    let dim = icon.canvas_size();
    let mut out = tiny_skia::Pixmap::new(dim, dim).expect("valid canvas size");
    let shadow = canvas::pixmap_from_image(&tint_image(base, DROP_SHADOW_COLOR));
    let tinted = canvas::pixmap_from_image(&tint_image(base, icon.color));
    let o = DROP_SHADOW_OFFSET as f32;
    let size = icon.size as f32;
    canvas::blit(&mut out, &shadow, o * 2.0, o * 2.0, size, size);
    canvas::blit(&mut out, &tinted, o, o, size, size);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default `PlotStyleSettings.Palette` (8 entries) + `Size = 50`.
    fn default_palette() -> Vec<Rgba> {
        vec![
            Rgba::new(255, 69, 0, 255),   // OrangeRed #FF4500
            Rgba::new(0, 255, 255, 255),  // Cyan
            Rgba::new(255, 255, 0, 255),  // Yellow
            Rgba::new(255, 0, 255, 255),  // Magenta
            Rgba::new(0, 255, 0, 255),    // Lime
            Rgba::new(65, 105, 225, 255), // RoyalBlue
            Rgba::new(255, 127, 80, 255), // Coral
            Rgba::new(0, 128, 0, 255),    // Green
        ]
    }

    const ORANGE_RED: Rgba = Rgba {
        r: 255,
        g: 69,
        b: 0,
        a: 255,
    };
    const CYAN: Rgba = Rgba {
        r: 0,
        g: 255,
        b: 255,
        a: 255,
    };

    #[test]
    fn assign_first_group_gets_orange_red_base_zero() {
        let icon = PlotIcon::assign(0, &default_palette(), 50, false, 7);
        assert_eq!(icon.color, ORANGE_RED);
        assert_eq!(icon.base_index, 0);
        assert_eq!(icon.size, 50);
        assert!(!icon.do_not_use_icon);
    }

    #[test]
    fn assign_wraps_palette_then_advances_base() {
        // offset 8: color wraps (8 % 8 == 0 -> OrangeRed), base advances
        // ((8 / 8) % 7 == 1).
        let icon = PlotIcon::assign(8, &default_palette(), 50, false, 7);
        assert_eq!(icon.color, ORANGE_RED);
        assert_eq!(icon.base_index, 1);
    }

    #[test]
    fn assign_second_group_gets_cyan() {
        let icon = PlotIcon::assign(1, &default_palette(), 50, false, 7);
        assert_eq!(icon.color, CYAN);
        assert_eq!(icon.base_index, 0);
    }

    #[test]
    fn do_not_use_composites_to_solid_square() {
        let base = image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]));
        let icon = PlotIcon::assign(3, &default_palette(), 50, true, 7);
        let px = composite(&base, &icon);
        assert_eq!((px.width(), px.height()), (50, 50));
        for y in [0, 25, 49] {
            for x in [0, 25, 49] {
                assert_eq!(canvas::straight_pixel(&px, x, y), icon.color);
            }
        }
    }

    fn asset_root() -> Option<String> {
        let root = std::env::var("MAPPALACHIA_ASSETS").ok()?;
        std::path::Path::new(&root).is_dir().then_some(root)
    }

    #[test]
    fn composite_center_probe_matches_tint_where_base_opaque() {
        let Some(root) = asset_root() else { return };
        let base = image::open(format!("{root}/img/icon/A.png"))
            .expect("A.png loads")
            .to_rgba8();
        // Find an interior opaque pixel (5x5 fully-opaque neighborhood) so
        // the bicubic resample stays exactly the tint color.
        let (w, h) = base.dimensions();
        let mut probe = None;
        'outer: for y in 2..h - 2 {
            for x in 2..w - 2 {
                let interior = (y - 2..=y + 2)
                    .all(|ny| (x - 2..=x + 2).all(|nx| base.get_pixel(nx, ny)[3] == 255));
                if interior {
                    probe = Some((x, y));
                    break 'outer;
                }
            }
        }
        let (sx, sy) = probe.expect("A.png has an interior opaque region");
        let icon = PlotIcon::assign(0, &default_palette(), 50, false, 7);
        let px = composite(&base, &icon);
        assert_eq!((px.width(), px.height()), (54, 54), "canvas is (size+4)^2");
        // Base (256x256 here) is stretched to 50x50 at offset (2,2).
        let dx = (2.0 + (sx as f32 + 0.5) * 50.0 / w as f32).floor() as u32;
        let dy = (2.0 + (sy as f32 + 0.5) * 50.0 / h as f32).floor() as u32;
        assert_eq!(canvas::straight_pixel(&px, dx, dy), ORANGE_RED);
    }
}
