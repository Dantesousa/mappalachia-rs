//! Pixmap primitives mirroring the GDI+ calls used by `Map.cs`.
//!
//! - `DrawImage(src, destRect)` (HQ bicubic stretch) -> [`blit`]
//! - `DrawImageCentered` (`Map.cs:1207`) -> [`draw_pixmap_centered`]
//! - Compass rotation (`Map.cs:336`, `RotateTransform(NorthAngle)`) and
//!   shape rotation (`Map.cs:948-950`, translate/rotate/translate about the
//!   shape center) -> [`rotate_about`] / [`rotate_blit`]
//! - `FillRectangle` / `DrawRectangle` (`Map.cs:834,900,961,966`) ->
//!   [`fill_rect`] / [`stroke_rect`]
//! - `FillEllipse` / `DrawEllipse` (`Map.cs:232-233,976,981`) ->
//!   [`fill_ellipse`] / [`stroke_ellipse`]
//! - `DrawLine` (`Map.cs:289,305,909-924`) -> [`line`]
//! - `FillPolygon(winding)` / `DrawPolygon` (`Map.cs:723,851,856`) ->
//!   [`fill_polygon`] / [`stroke_polygon`]
//!
//! Anti-aliasing mirrors GDI+ HQ defaults: tiny-skia strokes/fills are
//! anti-aliased by default (`Paint::default().anti_alias == true`), and
//! stretch blits use [`FilterQuality::Bicubic`] to match
//! `GraphicsFromImageHQ` (`HighQualityBicubic`).

use crate::color::Rgba;
use tiny_skia::{
    FillRule, FilterQuality, IntSize, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};

/// Solid-color paint (straight-alpha `Rgba` -> premultiplied internally).
fn solid_paint(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint
}

fn stroke_of(width: f32) -> Stroke {
    Stroke {
        width,
        // GDI+ `Pen` default miter limit (tiny-skia's own default is 4.0);
        // acute polygon joins must match, e.g. region outlines.
        miter_limit: 10.0,
        ..Default::default()
    }
}

/// Convert a straight-alpha [`image::RgbaImage`] to a premultiplied
/// [`Pixmap`], using tiny-skia's own rounding.
pub fn pixmap_from_image(img: &image::RgbaImage) -> Pixmap {
    let (w, h) = img.dimensions();
    let size = IntSize::from_wh(w, h).expect("image dimensions must be non-zero");
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for px in img.pixels() {
        let a = px[3];
        if a == 255 || a == 0 {
            // Opaque copies over exactly; transparent collapses to zero
            // (RGB content under full transparency is unobservable).
            data.extend_from_slice(if a == 255 { &px.0 } else { &[0, 0, 0, 0] });
        } else {
            let prem = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], a).premultiply();
            data.extend_from_slice(&[prem.red(), prem.green(), prem.blue(), prem.alpha()]);
        }
    }
    Pixmap::from_vec(data, size).expect("buffer length matches dimensions")
}

/// Read a canvas pixel back as straight-alpha [`Rgba`] (unpremultiplied).
/// Used by pixel-probe tests.
pub fn straight_pixel(pix: &Pixmap, x: u32, y: u32) -> Rgba {
    let p = pix.pixel(x, y).expect("pixel in bounds");
    let (r, g, b, a) = (p.red(), p.green(), p.blue(), p.alpha());
    if a == 0 || a == 255 {
        return Rgba::new(r, g, b, a);
    }
    let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32) as u8;
    Rgba::new(un(r), un(g), un(b), a)
}

/// Exact solid pixmap (no blending; mirrors `Graphics.Clear(color)`).
pub fn solid_pixmap(w: u32, h: u32, color: Rgba) -> Pixmap {
    let size = IntSize::from_wh(w, h).expect("dimensions must be non-zero");
    let px = pixmap_from_image(&image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([color.r, color.g, color.b, color.a]),
    ));
    let bytes = px.data();
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        data.extend_from_slice(bytes);
    }
    Pixmap::from_vec(data, size).expect("buffer length matches dimensions")
}

/// HQ-bicubic stretch blit: draw `src` scaled to `dw` x `dh` at (`dx`,`dy`).
/// Mirrors `graphics.DrawImage(image, destRect)` under `GraphicsFromImageHQ`;
/// note tiny-skia's default filter is bilinear, so [`FilterQuality::Bicubic`]
/// is set explicitly.
pub fn blit(dst: &mut Pixmap, src: &Pixmap, dx: f32, dy: f32, dw: f32, dh: f32) {
    let sx = dw / src.width() as f32;
    let sy = dh / src.height() as f32;
    let transform = Transform::from_translate(dx, dy).pre_scale(sx, sy);
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..Default::default()
    };
    dst.draw_pixmap(0, 0, src.as_ref(), &paint, transform, None);
}

/// 1:1 blit centered on (`cx`,`cy`). Mirrors `DrawImageCentered`
/// (`Map.cs:1207`: `coord - image.Size / 2`).
pub fn draw_pixmap_centered(dst: &mut Pixmap, src: &Pixmap, cx: f32, cy: f32) {
    let x = (cx - src.width() as f32 / 2.0).round() as i32;
    let y = (cy - src.height() as f32 / 2.0).round() as i32;
    dst.draw_pixmap(
        x,
        y,
        src.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Rotation about a pivot in degrees, clockwise-positive (matches C#
/// `RotateTransform` in y-down screen space; verified against tiny-skia
/// 0.11 `Transform::from_rotate`, which takes degrees).
///
/// Composed translate -> rotate -> translate from the `from_row`
/// constructors, mirroring the `Map.cs:948-950` sequence
/// (`TranslateTransform(c); RotateTransform(r); TranslateTransform(-c)`).
pub fn rotate_about(angle_deg: f32, cx: f32, cy: f32) -> Transform {
    Transform::from_translate(cx, cy)
        .pre_concat(Transform::from_rotate(angle_deg))
        .pre_translate(-cx, -cy)
}

/// Rotated stretch blit: draw `src` scaled to `dw` x `dh`, centered on
/// (`cx`,`cy`), rotated `angle_deg` (clockwise-positive) about the center.
/// Mirrors the compass rose draw (`Map.cs:331-338`: `CompassSize` canvas,
/// `RotateTransform(NorthAngle)`, full-canvas `DrawImage`) and shape draws
/// (`Map.cs:948-950`).
pub fn rotate_blit(
    dst: &mut Pixmap,
    src: &Pixmap,
    cx: f32,
    cy: f32,
    dw: f32,
    dh: f32,
    angle_deg: f32,
) {
    let sx = dw / src.width() as f32;
    let sy = dh / src.height() as f32;
    let transform = Transform::from_translate(cx, cy)
        .pre_concat(Transform::from_rotate(angle_deg))
        .pre_translate(-dw / 2.0, -dh / 2.0)
        .pre_scale(sx, sy);
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..Default::default()
    };
    dst.draw_pixmap(0, 0, src.as_ref(), &paint, transform, None);
}

/// Solid rectangle fill. Mirrors `FillRectangle`.
pub fn fill_rect(dst: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
    fill_rect_xform(dst, x, y, w, h, color, Transform::identity());
}

/// Solid rectangle fill under `transform`. Mirrors the shape draws
/// (`Map.cs:948-950`: translate/rotate/translate about the shape center).
pub fn fill_rect_xform(
    dst: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Rgba,
    transform: Transform,
) {
    let rect = Rect::from_xywh(x, y, w, h).expect("valid rect");
    dst.fill_rect(rect, &solid_paint(color), transform, None);
}

/// Rectangle outline. Mirrors `DrawRectangle(pen, rect)`.
pub fn stroke_rect(dst: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Rgba, width: f32) {
    stroke_rect_xform(dst, x, y, w, h, color, width, Transform::identity());
}

/// Rectangle outline under `transform` (see [`fill_rect_xform`]).
#[allow(clippy::too_many_arguments)]
pub fn stroke_rect_xform(
    dst: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Rgba,
    width: f32,
    transform: Transform,
) {
    let rect = Rect::from_xywh(x, y, w, h).expect("valid rect");
    let path = PathBuilder::from_rect(rect);
    dst.stroke_path(
        &path,
        &solid_paint(color),
        &stroke_of(width),
        transform,
        None,
    );
}

/// Solid ellipse bounded by the rect. Mirrors `FillEllipse`.
pub fn fill_ellipse(dst: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
    fill_ellipse_xform(dst, x, y, w, h, color, Transform::identity());
}

/// Solid ellipse under `transform` (see [`fill_rect_xform`]).
pub fn fill_ellipse_xform(
    dst: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Rgba,
    transform: Transform,
) {
    let rect = Rect::from_xywh(x, y, w, h).expect("valid rect");
    let path = PathBuilder::from_oval(rect).expect("valid oval");
    dst.fill_path(
        &path,
        &solid_paint(color),
        FillRule::Winding,
        transform,
        None,
    );
}

/// Ellipse outline. Mirrors `DrawEllipse(pen, rect)`.
pub fn stroke_ellipse(dst: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Rgba, width: f32) {
    stroke_ellipse_xform(dst, x, y, w, h, color, width, Transform::identity());
}

/// Ellipse outline under `transform` (see [`fill_rect_xform`]).
#[allow(clippy::too_many_arguments)]
pub fn stroke_ellipse_xform(
    dst: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Rgba,
    width: f32,
    transform: Transform,
) {
    let rect = Rect::from_xywh(x, y, w, h).expect("valid rect");
    let path = PathBuilder::from_oval(rect).expect("valid oval");
    dst.stroke_path(
        &path,
        &solid_paint(color),
        &stroke_of(width),
        transform,
        None,
    );
}

/// Line segment. Mirrors `DrawLine(pen, a, b)` (default flat-cap pen).
pub fn line(dst: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32, color: Rgba, width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    let path = pb.finish().expect("line path is non-empty");
    dst.stroke_path(
        &path,
        &solid_paint(color),
        &stroke_of(width),
        Transform::identity(),
        None,
    );
}

fn polygon_path(points: &[(f32, f32)]) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    let mut iter = points.iter();
    let first = iter.next().expect("polygon needs at least one point");
    pb.move_to(first.0, first.1);
    for p in iter {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    pb.finish().expect("polygon path is non-empty")
}

/// Solid polygon (winding fill). Mirrors
/// `FillPolygon(brush, points, FillMode.Winding)` (`Map.cs:851`).
pub fn fill_polygon(dst: &mut Pixmap, points: &[(f32, f32)], color: Rgba) {
    let path = polygon_path(points);
    dst.fill_path(
        &path,
        &solid_paint(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// Polygon outline. Mirrors `DrawPolygon(pen, points)` (`Map.cs:723,856`).
pub fn stroke_polygon(dst: &mut Pixmap, points: &[(f32, f32)], color: Rgba, width: f32) {
    let path = polygon_path(points);
    dst.stroke_path(
        &path,
        &solid_paint(color),
        &stroke_of(width),
        Transform::identity(),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = Rgba {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };

    fn blank(w: u32, h: u32) -> Pixmap {
        Pixmap::new(w, h).expect("valid size")
    }

    #[test]
    fn fill_rect_paints_interior_only() {
        let mut px = blank(8, 8);
        fill_rect(&mut px, 2.0, 2.0, 4.0, 4.0, RED);
        assert_eq!(straight_pixel(&px, 4, 4), RED);
        assert_eq!(straight_pixel(&px, 0, 0).a, 0);
        assert_eq!(straight_pixel(&px, 1, 1).a, 0);
    }

    #[test]
    fn stroke_rect_leaves_interior_empty() {
        let mut px = blank(10, 10);
        stroke_rect(&mut px, 1.0, 1.0, 8.0, 8.0, RED, 2.0);
        assert_eq!(straight_pixel(&px, 5, 5).a, 0);
        assert_eq!(straight_pixel(&px, 1, 5), RED);
    }

    #[test]
    fn blit_stretches_uniform_source_exactly() {
        let src = solid_pixmap(2, 2, RED);
        let mut dst = blank(4, 4);
        blit(&mut dst, &src, 0.0, 0.0, 4.0, 4.0);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(straight_pixel(&dst, x, y), RED, "at ({x},{y})");
            }
        }
    }

    #[test]
    fn centered_blit_rounds_to_nearest_pixel() {
        let src = solid_pixmap(3, 3, RED);
        let mut dst = blank(9, 9);
        draw_pixmap_centered(&mut dst, &src, 4.0, 4.0);
        // 3x3 at top-left (3,3)..=(5,5); DrawImageCentered rounds halves.
        assert_eq!(straight_pixel(&dst, 2, 4).a, 0);
        assert_eq!(straight_pixel(&dst, 3, 4), RED);
        assert_eq!(straight_pixel(&dst, 5, 5), RED);
        assert_eq!(straight_pixel(&dst, 6, 4).a, 0);
    }

    #[test]
    fn rotate_about_zero_is_identity() {
        assert!(rotate_about(0.0, 37.0, 91.0).is_identity());
    }

    #[test]
    fn rotate_about_quarter_turn_is_clockwise() {
        // C# RotateTransform(+90) in y-down space moves east -> south.
        let t = rotate_about(90.0, 10.0, 10.0);
        let mut p = tiny_skia::Point { x: 11.0, y: 10.0 };
        t.map_point(&mut p);
        assert!(
            (p.x - 10.0).abs() < 1e-4 && (p.y - 11.0).abs() < 1e-4,
            "{p:?}"
        );
    }

    #[test]
    fn rotated_blit_differs_from_unrotated() {
        // Asymmetric source: red left half, transparent right half.
        let mut img = image::RgbaImage::new(8, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let src = pixmap_from_image(&img);
        let mut plain = blank(16, 16);
        rotate_blit(&mut plain, &src, 8.0, 8.0, 8.0, 4.0, 0.0);
        let mut rotated = blank(16, 16);
        rotate_blit(&mut rotated, &src, 8.0, 8.0, 8.0, 4.0, 90.0);
        assert_ne!(plain.data(), rotated.data());
        // 0-degree rotate_blit matches the axis-aligned blit over the
        // destination footprint.
        let mut direct = blank(16, 16);
        blit(&mut direct, &src, 4.0, 6.0, 8.0, 4.0);
        assert_eq!(plain.data(), direct.data());
    }

    #[test]
    fn polygon_fill_covers_triangle_interior() {
        let mut px = blank(10, 10);
        fill_polygon(&mut px, &[(1.0, 1.0), (8.0, 1.0), (1.0, 8.0)], RED);
        assert_eq!(straight_pixel(&px, 2, 2), RED);
        assert_eq!(straight_pixel(&px, 8, 8).a, 0);
    }

    #[test]
    fn line_and_ellipse_mark_expected_pixels() {
        let mut px = blank(12, 12);
        line(&mut px, 0.0, 6.0, 11.0, 6.0, RED, 2.0);
        assert_eq!(straight_pixel(&px, 5, 6), RED);
        let mut el = blank(12, 12);
        stroke_ellipse(&mut el, 1.0, 1.0, 10.0, 10.0, RED, 2.0);
        assert_eq!(straight_pixel(&el, 6, 6).a, 0); // hollow
        let mut fel = blank(12, 12);
        fill_ellipse(&mut fel, 1.0, 1.0, 10.0, 10.0, RED);
        assert_eq!(straight_pixel(&fel, 6, 6), RED);
    }
}
