//! Text rendering: Futura Condensed Bold via fontdue, mirroring the GDI+
//! text calls in `Map.cs`.
//!
//! - `GetFont(size)` (`Map.cs:1321`) -> [`Font::from_bytes`] (bytes from
//!   `Assets::font_bytes`); the `px` size is passed per call like C#'s
//!   `new Font(family, size, GraphicsUnit.Pixel)`.
//! - `DrawStringWithDropShadow` (`Map.cs:1228`) -> [`Font::draw`]
//!   (shadow pass at +2,+2 in `(128,0,0,0)`, then the main pass).
//! - `DrawStringCentered` (`Map.cs:1213`) -> [`Font::draw_centered`]
//!   (measure with optional wrap width, center on the coord, top-align or
//!   vertically center).
//! - The seven named `StringFormat`s (`Map.cs:101-113`) -> [`Align`].
//! - Grid x-labels (`Map.cs:307-315`, `RotateTransform(-90)`) ->
//!   [`Font::draw_rotated`] (rasterize horizontal to a temp pixmap, blit
//!   rotated via `canvas::rotate_blit`).

use crate::color::Rgba;
use tiny_skia::Pixmap;

/// Drop-shadow offset, mirroring `Map.DropShadowOffset` (`Map.cs:73`).
pub const SHADOW_OFFSET: f32 = 2.0;

/// Drop-shadow color, mirroring `Map.DropShadowColor`
/// (`Map.cs:91`: `Color.FromArgb(128, 0, 0, 0)`).
pub const SHADOW_COLOR: Rgba = Rgba {
    r: 0,
    g: 0,
    b: 0,
    a: 128,
};

/// Mirrors the seven named `StringFormat`s (`Map.cs:101-113`): horizontal
/// `Alignment` (Near/Center/Far) x vertical `LineAlignment`, minus the
/// unused TopCenter/BottomCenter combos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    TopLeft,
    TopRight,
    Center,
    CenterLeft,
    CenterRight,
    BottomLeft,
    BottomRight,
}

impl Align {
    fn horizontal(self) -> u8 {
        match self {
            Align::TopLeft | Align::CenterLeft | Align::BottomLeft => 0,
            Align::Center => 1,
            Align::TopRight | Align::CenterRight | Align::BottomRight => 2,
        }
    }

    fn vertical(self) -> u8 {
        match self {
            Align::TopLeft | Align::TopRight => 0,
            Align::Center | Align::CenterLeft | Align::CenterRight => 1,
            Align::BottomLeft | Align::BottomRight => 2,
        }
    }

    /// Top-left origin of a (`bw`,`bh`) text block inside `rect`
    /// (`x`,`y`,`w`,`h`), mirroring `DrawString` with the matching
    /// `StringFormat`: each line follows the horizontal alignment within
    /// the rect, the block as a whole follows the vertical one.
    pub fn origin(self, rect: (f32, f32, f32, f32), block: (f32, f32)) -> (f32, f32) {
        let (rx, ry, rw, rh) = rect;
        let (bw, bh) = block;
        let x = match self.horizontal() {
            0 => rx,
            1 => rx + (rw - bw) / 2.0,
            _ => rx + rw - bw,
        };
        let y = match self.vertical() {
            0 => ry,
            1 => ry + (rh - bh) / 2.0,
            _ => ry + rh - bh,
        };
        (x, y)
    }

    /// Per-line x offset of a `line_w`-wide line inside a `block_w`-wide
    /// block. Combined with [`Align::origin`], this centers/right-aligns
    /// each line exactly as GDI+ does within the layout rect.
    fn line_offset(self, block_w: f32, line_w: f32) -> f32 {
        match self.horizontal() {
            0 => 0.0,
            1 => (block_w - line_w) / 2.0,
            _ => block_w - line_w,
        }
    }
}

/// Laid-out text: wrapped lines with per-line widths plus the block size
/// (mirrors `MeasureString`, unbounded or bounded by width).
struct TextLayout {
    lines: Vec<(String, f32)>,
    width: f32,
    height: f32,
}

/// Application font (Futura Condensed Bold), loaded like C#'s
/// `PrivateFontCollection.AddFontFile(Paths.FontPath)` startup load.
pub struct Font {
    inner: fontdue::Font,
}

impl Font {
    /// Load from raw font bytes (see `Assets::font_bytes`).
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let inner = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .expect("valid font bytes");
        Self { inner }
    }

    /// `(ascent, line_height)` at `px` from the font's horizontal metrics
    /// (`ascent`, `new_line_size = ascent - descent + line_gap`).
    fn vertical(&self, px: f32) -> (f32, f32) {
        match self.inner.horizontal_line_metrics(px) {
            Some(m) => (m.ascent, m.new_line_size),
            None => (px * 0.8, px * 1.2),
        }
    }

    /// Line height at `px`: `ascent - descent + line_gap`.
    pub fn line_height(&self, px: f32) -> f32 {
        self.vertical(px).1
    }

    fn advance(&self, ch: char, px: f32) -> f32 {
        self.inner.metrics(ch, px).advance_width
    }

    fn line_width(&self, line: &str, px: f32) -> f32 {
        line.chars().map(|c| self.advance(c, px)).sum()
    }

    /// Wrap one paragraph (no hard breaks) by words. A word wider than
    /// `max_w` takes its own line and overflows, like fontdue's `Layout`.
    fn wrap_paragraph(&self, para: &str, px: f32, max_w: f32) -> Vec<(String, f32)> {
        let space_w = self.advance(' ', px);
        let mut lines: Vec<(String, f32)> = Vec::new();
        let mut cur = String::new();
        let mut cur_w = 0.0f32;
        for word in para.split_whitespace() {
            let ww: f32 = word.chars().map(|c| self.advance(c, px)).sum();
            if cur.is_empty() {
                // First word of the line: take it even if it overflows.
                cur.push_str(word);
                cur_w = ww;
            } else if cur_w + space_w + ww > max_w {
                lines.push((std::mem::take(&mut cur), cur_w));
                cur.push_str(word);
                cur_w = ww;
            } else {
                cur.push(' ');
                cur.push_str(word);
                cur_w += space_w + ww;
            }
        }
        if lines.is_empty() && cur.is_empty() {
            // Blank paragraph (or blank line): zero-width, keeps its height.
            lines.push((String::new(), 0.0));
        } else {
            lines.push((cur, cur_w));
        }
        lines
    }

    /// Lay out `text` (`\n` hard breaks; word wrap when `wrap_width` is a
    /// positive width), mirroring `MeasureString`'s unbounded/bounded forms.
    fn layout(&self, text: &str, px: f32, wrap_width: Option<f32>) -> TextLayout {
        if text.is_empty() || px <= 0.0 {
            return TextLayout {
                lines: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }
        let mut lines = Vec::new();
        for para in text.split('\n') {
            match wrap_width {
                Some(max_w) if max_w > 0.0 => lines.extend(self.wrap_paragraph(para, px, max_w)),
                _ => lines.push((para.to_owned(), self.line_width(para, px))),
            }
        }
        let width = lines.iter().map(|(_, w)| *w).fold(0.0f32, f32::max);
        let height = self.line_height(px) * lines.len() as f32;
        TextLayout {
            lines,
            width,
            height,
        }
    }

    /// Mirrors `MeasureString` (unbounded form with `wrap_width == None`,
    /// bounded form otherwise). Returns `(width, height)`.
    pub fn measure(&self, text: &str, px: f32, wrap_width: Option<f32>) -> (f32, f32) {
        let layout = self.layout(text, px, wrap_width);
        (layout.width, layout.height)
    }

    /// Src-over blend of one glyph coverage bitmap (`0` = transparent,
    /// `255` = full) tinted with `color` onto the premultiplied canvas.
    fn blend_glyph(dst: &mut Pixmap, bmp: &[u8], gw: usize, gx: i32, gy: i32, color: Rgba) {
        if color.a == 0 || bmp.is_empty() {
            return;
        }
        let dw = dst.width() as i32;
        let dh = dst.height() as i32;
        let sa_base = f32::from(color.a) / 255.0;
        let sr = f32::from(color.r) / 255.0;
        let sg = f32::from(color.g) / 255.0;
        let sb = f32::from(color.b) / 255.0;
        let stride = dst.width() as usize;
        let data = dst.data_mut();
        for (i, &cov) in bmp.iter().enumerate() {
            if cov == 0 {
                continue;
            }
            let x = gx + (i % gw) as i32;
            let y = gy + (i / gw) as i32;
            if x < 0 || y < 0 || x >= dw || y >= dh {
                continue;
            }
            let sa = f32::from(cov) / 255.0 * sa_base;
            let inv = 1.0 - sa;
            let idx = (y as usize * stride + x as usize) * 4;
            let dr = f32::from(data[idx]) / 255.0;
            let dg = f32::from(data[idx + 1]) / 255.0;
            let db = f32::from(data[idx + 2]) / 255.0;
            let da = f32::from(data[idx + 3]) / 255.0;
            data[idx] = ((sr * sa + dr * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
            data[idx + 1] = ((sg * sa + dg * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
            data[idx + 2] = ((sb * sa + db * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
            data[idx + 3] = ((sa + da * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Paint laid-out lines: `line_x` maps a line width to its origin x,
    /// `y0` is the block top. With `shadow`, a first pass at
    /// (+2,+2) in [`SHADOW_COLOR`] precedes the main pass, mirroring
    /// `DrawStringWithDropShadow` (shadow first, text over it).
    // Arity mirrors the C# call shape (brief-mandated API).
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        canvas: &mut Pixmap,
        layout: &TextLayout,
        px: f32,
        color: Rgba,
        line_x: impl Fn(f32) -> f32,
        y0: f32,
        shadow: bool,
    ) {
        if layout.lines.is_empty() {
            return;
        }
        let (ascent, line_h) = self.vertical(px);
        for pass in 0..=1 {
            let (ox, oy, col) = if pass == 0 && shadow {
                (SHADOW_OFFSET, SHADOW_OFFSET, SHADOW_COLOR)
            } else if pass == 1 {
                (0.0, 0.0, color)
            } else {
                continue;
            };
            for (i, (line, lw)) in layout.lines.iter().enumerate() {
                let baseline = y0 + oy + ascent + i as f32 * line_h;
                let mut pen = line_x(*lw) + ox;
                for ch in line.chars() {
                    let (m, bmp) = self.inner.rasterize(ch, px);
                    if m.width > 0 && m.height > 0 && !bmp.is_empty() {
                        // Y-down placement, mirroring fontdue's own
                        // `Layout` engine (`CoordinateSystem::PositiveYDown`):
                        // bitmap top-left at
                        // (pen + xmin, baseline - height - ymin).
                        let gx = (pen + m.bounds.xmin).floor() as i32;
                        let gy = (baseline - m.bounds.height - m.bounds.ymin).floor() as i32;
                        Self::blend_glyph(canvas, &bmp, m.width, gx, gy, col);
                    }
                    pen += m.advance_width;
                }
            }
        }
    }

    /// Draw left-aligned at (`x`,`y`) with drop shadow (+2,+2,
    /// (128,0,0,0)); mirrors `DrawStringWithDropShadow`.
    pub fn draw(&self, canvas: &mut Pixmap, text: &str, px: f32, color: Rgba, x: f32, y: f32) {
        let layout = self.layout(text, px, None);
        self.paint(canvas, &layout, px, color, |_| x, y, true);
    }

    /// Mirrors `DrawStringCentered`: center on (`cx`,`cy`), vertically
    /// centered (`center_vert`) or top-aligned, optional wrap width. Each
    /// wrapped line is centered, like GDI+ `Center` alignment.
    // Arity mirrors DrawStringCentered (brief-mandated API).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_centered(
        &self,
        canvas: &mut Pixmap,
        text: &str,
        px: f32,
        color: Rgba,
        cx: f32,
        cy: f32,
        center_vert: bool,
        wrap_width: Option<f32>,
    ) {
        let layout = self.layout(text, px, wrap_width);
        let y0 = if center_vert {
            cy - layout.height / 2.0
        } else {
            cy
        };
        self.paint(canvas, &layout, px, color, |lw| cx - lw / 2.0, y0, true);
    }

    /// Draw within `rect` (`x`,`y`,`w`,`h`) under an [`Align`] format,
    /// with drop shadow; mirrors `DrawString` with the named formats plus
    /// the shadow pass used for map labels.
    // Arity mirrors the DrawString(rect, format) shape (brief-mandated API).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_aligned(
        &self,
        canvas: &mut Pixmap,
        text: &str,
        px: f32,
        color: Rgba,
        rect: (f32, f32, f32, f32),
        align: Align,
        wrap_width: Option<f32>,
    ) {
        let layout = self.layout(text, px, wrap_width);
        let (ox, oy) = align.origin(rect, (layout.width, layout.height));
        self.paint(
            canvas,
            &layout,
            px,
            color,
            |lw| ox + align.line_offset(layout.width, lw),
            oy,
            true,
        );
    }

    /// Rotated label: rasterize horizontal (no shadow, like the C# grid
    /// labels drawn with plain `DrawString`) to a temp pixmap, then
    /// `canvas::rotate_blit` centered on (`cx`,`cy`). Mirrors the grid
    /// x-labels (`Map.cs:307-315`, angle `-90`).
    // Arity mirrors the rotated-label call shape (brief-mandated API).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rotated(
        &self,
        canvas: &mut Pixmap,
        text: &str,
        px: f32,
        color: Rgba,
        cx: f32,
        cy: f32,
        angle_deg: f32,
        wrap_width: Option<f32>,
    ) {
        let layout = self.layout(text, px, wrap_width);
        if layout.lines.is_empty() {
            return;
        }
        let w = layout.width.ceil().max(1.0) as u32;
        let h = layout.height.ceil().max(1.0) as u32;
        let mut tmp = Pixmap::new(w, h).expect("text block has nonzero size");
        self.paint(
            &mut tmp,
            &layout,
            px,
            color,
            |lw| (w as f32 - lw) / 2.0,
            0.0,
            false,
        );
        crate::canvas::rotate_blit(canvas, &tmp, cx, cy, w as f32, h as f32, angle_deg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{fill_rect, straight_pixel};

    fn asset_root() -> Option<String> {
        let root = std::env::var("MAPPALACHIA_ASSETS").ok()?;
        std::path::Path::new(&root).is_dir().then_some(root)
    }

    fn load_font() -> Option<Font> {
        let root = asset_root()?;
        let bytes = std::fs::read(format!("{root}/font/futura_condensed_bold.otf")).ok()?;
        Some(Font::from_bytes(&bytes))
    }

    fn ink_count(px: &Pixmap) -> u32 {
        let mut n = 0;
        for y in 0..px.height() {
            for x in 0..px.width() {
                if straight_pixel(px, x, y).a != 0 {
                    n += 1;
                }
            }
        }
        n
    }

    const WHITE: Rgba = Rgba {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    #[test]
    fn futura_loads_and_measures() {
        let Some(font) = load_font() else { return };
        let (w, h) = font.measure("Nuka-Cola", 40.0, None);
        assert!(w > 0.0, "width {w}");
        assert!(h > 0.0, "height {h}");
        assert_eq!(font.measure("", 40.0, None), (0.0, 0.0));
    }

    #[test]
    fn bounded_measure_wraps_to_narrower_taller_block() {
        let Some(font) = load_font() else { return };
        let (w_free, h_free) = font.measure("Nuka Cola World Tour", 40.0, None);
        let (w_wrap, h_wrap) = font.measure("Nuka Cola World Tour", 40.0, Some(60.0));
        assert!(w_wrap <= w_free, "{w_wrap} <= {w_free}");
        assert!(h_wrap > h_free, "{h_wrap} > {h_free}");
    }

    #[test]
    fn draw_produces_ink() {
        let Some(font) = load_font() else { return };
        let mut canvas = Pixmap::new(200, 100).expect("canvas");
        font.draw(&mut canvas, "Nuka-Cola", 40.0, WHITE, 10.0, 10.0);
        assert!(ink_count(&canvas) > 50, "draw leaves ink");
    }

    #[test]
    fn draw_shadow_shows_on_matching_background() {
        let Some(font) = load_font() else { return };
        // White text on a white field: only the (128,0,0,0) shadow pass
        // (at +2,+2) can leave non-white pixels.
        let mut canvas = Pixmap::new(200, 100).expect("canvas");
        fill_rect(&mut canvas, 0.0, 0.0, 200.0, 100.0, WHITE);
        font.draw(&mut canvas, "Nuka-Cola", 40.0, WHITE, 10.0, 10.0);
        let mut off_white = 0;
        for y in 0..100 {
            for x in 0..200 {
                if straight_pixel(&canvas, x, y) != WHITE {
                    off_white += 1;
                }
            }
        }
        assert!(off_white > 20, "shadow pass leaves gray ink");
    }

    #[test]
    fn centered_text_spans_both_halves() {
        let Some(font) = load_font() else { return };
        let mut canvas = Pixmap::new(400, 200).expect("canvas");
        font.draw_centered(
            &mut canvas,
            "Nuka-Cola",
            40.0,
            WHITE,
            200.0,
            100.0,
            true,
            None,
        );
        let mut left = 0;
        let mut right = 0;
        for y in 0..200 {
            for x in 0..400 {
                if straight_pixel(&canvas, x, y).a != 0 {
                    if x < 200 {
                        left += 1;
                    } else {
                        right += 1;
                    }
                }
            }
        }
        assert!(left > 20, "ink left of center: {left}");
        assert!(right > 20, "ink right of center: {right}");
    }

    #[test]
    fn align_origin_math() {
        assert_eq!(
            Align::TopLeft.origin((10.0, 20.0, 100.0, 60.0), (30.0, 10.0)),
            (10.0, 20.0)
        );
        assert_eq!(
            Align::Center.origin((0.0, 0.0, 100.0, 100.0), (20.0, 10.0)),
            (40.0, 45.0)
        );
        assert_eq!(
            Align::BottomRight.origin((0.0, 0.0, 100.0, 100.0), (20.0, 10.0)),
            (80.0, 90.0)
        );
        assert_eq!(
            Align::TopRight.origin((0.0, 0.0, 100.0, 100.0), (20.0, 10.0)),
            (80.0, 0.0)
        );
        assert_eq!(
            Align::CenterLeft.origin((0.0, 0.0, 100.0, 100.0), (20.0, 10.0)),
            (0.0, 45.0)
        );
    }

    #[test]
    fn aligned_center_draws_ink() {
        let Some(font) = load_font() else { return };
        let mut canvas = Pixmap::new(400, 200).expect("canvas");
        font.draw_aligned(
            &mut canvas,
            "Nuka-Cola",
            40.0,
            WHITE,
            (0.0, 0.0, 400.0, 200.0),
            Align::Center,
            None,
        );
        assert!(ink_count(&canvas) > 50, "aligned draw leaves ink");
    }

    #[test]
    fn rotated_label_leaves_vertical_ink() {
        let Some(font) = load_font() else { return };
        let mut canvas = Pixmap::new(200, 200).expect("canvas");
        font.draw_rotated(&mut canvas, "1234", 40.0, WHITE, 100.0, 100.0, -90.0, None);
        let mut top = 0;
        let mut bottom = 0;
        for y in 0..200 {
            for x in 0..200 {
                if straight_pixel(&canvas, x, y).a != 0 {
                    if y < 100 {
                        top += 1;
                    } else {
                        bottom += 1;
                    }
                }
            }
        }
        assert!(top > 10, "rotated ink above center: {top}");
        assert!(bottom > 10, "rotated ink below center: {bottom}");
    }
}
