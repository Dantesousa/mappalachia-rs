use mappalachia_core::{common::MAP_IMAGE_RESOLUTION, model::Space};

pub struct View { pub center_x: f64, pub center_y: f64, pub factor: f64 }
impl View {
    pub fn for_space(space: &Space) -> Self {
        Self { center_x: space.center_x, center_y: space.center_y,
               factor: space.max_range / MAP_IMAGE_RESOLUTION as f64 }
    }
}

fn half() -> f64 { MAP_IMAGE_RESOLUTION as f64 / 2.0 }

/// Mirrors AsImagePoint: x = (wx-cx)/f + half; y = ((-wy+cy)/f) + half (Y flip).
pub fn world_to_image(v: &View, x: f64, y: f64) -> (f32, f32) {
    (((x - v.center_x) / v.factor + half()) as f32,
     (((-y + v.center_y) / v.factor) + half()) as f32)
}

/// Mirrors AsWorldCoord (exact inverse).
pub fn image_to_world(v: &View, px: f32, py: f32) -> (f64, f64) {
    (((px as f64 - half()) * v.factor) + v.center_x,
     ((((py as f64 - half()) * v.factor) - v.center_y) * -1.0))
}

/// Mirrors AsImageLength (double form; int form is a truncation at the call site).
pub fn world_len_to_image(v: &View, len: f64) -> f32 { (len / v.factor) as f32 }

/// Mirrors AsImageRectangle (abs widths).
pub fn image_rect_of_world(v: &View, left: f64, top: f64, width: f64, height: f64) -> (f32, f32, f32, f32) {
    let (tlx, tly) = world_to_image(v, left, top);
    let (brx, bry) = world_to_image(v, left + width, top + height);
    (tlx, tly, (brx - tlx).abs(), (bry - tly).abs())
}

/// Mirrors AsWorldRectangle (abs widths).
pub fn world_rect_of_image(v: &View, left: f32, top: f32, width: f32, height: f32) -> (f64, f64, f64, f64) {
    let (tlx, tly) = image_to_world(v, left, top);
    let (brx, bry) = image_to_world(v, left + width, top + height);
    (tlx, tly, (brx - tlx).abs(), (tly - bry).abs())
}

/// Mirrors GetScaledMapBackgroundRect with spotlight disabled: full canvas.
pub fn background_rect() -> (f32, f32, f32, f32) {
    (0.0, 0.0, MAP_IMAGE_RESOLUTION as f32, MAP_IMAGE_RESOLUTION as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn rects_and_background() {
        let v = View { center_x: -500.0, center_y: 135.0, factor: 582550.0/4096.0 };
        let (x, y, w, h) = image_rect_of_world(&v, -500.0, 135.0, v.factor, v.factor);
        assert_eq!((x, y, w, h), (2048.0, 2048.0, 1.0, 1.0));
        let (wx, wy, ww, wh) = world_rect_of_image(&v, 2048.0, 2048.0, 1.0, 1.0);
        assert!((wx + 500.0).abs() < 1.0 && (wy - 135.0).abs() < 1.0);
        assert!((ww - v.factor).abs() < 1.0 && (wh - v.factor).abs() < 1.0);
        assert_eq!(background_rect(), (0.0, 0.0, 4096.0, 4096.0));
    }
}
