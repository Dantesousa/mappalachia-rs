use mappalachia_core::{
    common::{MAP_IMAGE_RESOLUTION, TILE_WIDTH},
    model::Space,
};

/// Shared world<->image projection: a center in world coords plus the
/// world-to-image factor. Mirrors the C# pair `GetWorldCenter` /
/// `GetWorldToImageFactor`, whose spotlight branches swap in the spotlight
/// location and `SpotlightSize * TileWidth` range while keeping every
/// formula identical.
pub trait Projection {
    fn center_x(&self) -> f64;
    fn center_y(&self) -> f64;
    fn factor(&self) -> f64;
}

pub struct View {
    pub center_x: f64,
    pub center_y: f64,
    pub factor: f64,
}
impl View {
    pub fn for_space(space: &Space) -> Self {
        Self {
            center_x: space.center_x,
            center_y: space.center_y,
            factor: space.max_range / MAP_IMAGE_RESOLUTION as f64,
        }
    }
}
impl Projection for View {
    fn center_x(&self) -> f64 {
        self.center_x
    }
    fn center_y(&self) -> f64 {
        self.center_y
    }
    fn factor(&self) -> f64 {
        self.factor
    }
}

/// Spotlight projection: plain data struct (consumed by later tasks).
/// Center is the spotlight location, factor the C# spotlight branch
/// `(spotlight_size_tiles * TILE_WIDTH) / 4096` (`Map.cs:1241-1254`).
pub struct SpotView {
    pub center_x: f64,
    pub center_y: f64,
    pub factor: f64,
}
impl Projection for SpotView {
    fn center_x(&self) -> f64 {
        self.center_x
    }
    fn center_y(&self) -> f64 {
        self.center_y
    }
    fn factor(&self) -> f64 {
        self.factor
    }
}

pub fn spot_view(location: (f64, f64), size_tiles: f64) -> SpotView {
    SpotView {
        center_x: location.0,
        center_y: location.1,
        factor: size_tiles * f64::from(TILE_WIDTH) / f64::from(MAP_IMAGE_RESOLUTION),
    }
}

fn half() -> f64 {
    MAP_IMAGE_RESOLUTION as f64 / 2.0
}

/// Mirrors AsImagePoint: x = (wx-cx)/f + half; y = ((-wy+cy)/f) + half (Y flip).
pub fn world_to_image(v: &impl Projection, x: f64, y: f64) -> (f32, f32) {
    (
        ((x - v.center_x()) / v.factor() + half()) as f32,
        (((-y + v.center_y()) / v.factor()) + half()) as f32,
    )
}

/// Mirrors AsWorldCoord (exact inverse).
#[allow(clippy::neg_multiply)] // keep C# `* -1` formula shape verbatim
pub fn image_to_world(v: &impl Projection, px: f32, py: f32) -> (f64, f64) {
    (
        ((px as f64 - half()) * v.factor()) + v.center_x(),
        ((((py as f64 - half()) * v.factor()) - v.center_y()) * -1.0),
    )
}

/// Mirrors AsImageLength (double form; int form is a truncation at the call site).
pub fn world_len_to_image(v: &impl Projection, len: f64) -> f32 {
    (len / v.factor()) as f32
}

/// Mirrors AsImageRectangle (abs widths).
pub fn image_rect_of_world(
    v: &impl Projection,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
) -> (f32, f32, f32, f32) {
    let (tlx, tly) = world_to_image(v, left, top);
    let (brx, bry) = world_to_image(v, left + width, top + height);
    (tlx, tly, (brx - tlx).abs(), (bry - tly).abs())
}

/// Mirrors AsWorldRectangle (abs widths).
pub fn world_rect_of_image(
    v: &impl Projection,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
) -> (f64, f64, f64, f64) {
    let (tlx, tly) = image_to_world(v, left, top);
    let (brx, bry) = image_to_world(v, left + width, top + height);
    (tlx, tly, (brx - tlx).abs(), (tly - bry).abs())
}

/// Mirrors GetScaledMapBackgroundRect (`Map.cs:1310-1318`): full canvas when
/// spotlight is off (`None`), else the space rect mapped through the
/// spotlight projection.
pub fn background_rect(space: &Space, spotlight: Option<&SpotView>) -> (f32, f32, f32, f32) {
    match spotlight {
        None => (
            0.0,
            0.0,
            MAP_IMAGE_RESOLUTION as f32,
            MAP_IMAGE_RESOLUTION as f32,
        ),
        Some(spot) => {
            let (left, top, width, height) = space.rectangle();
            image_rect_of_world(spot, left, top, width, height)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn center_maps_to_center() {
        let v = View {
            center_x: -500.0,
            center_y: 135.0,
            factor: 582550.0 / 4096.0,
        };
        assert_eq!(world_to_image(&v, -500.0, 135.0), (2048.0, 2048.0));
        // one factor east -> +1px; Y flip: larger world y -> smaller image y
        assert_eq!(
            world_to_image(&v, -500.0 + v.factor, 135.0),
            (2049.0, 2048.0)
        );
        let (px, py) = world_to_image(&v, 1000.0, 2000.0);
        let (wx, wy) = image_to_world(&v, px, py);
        assert!((wx - 1000.0).abs() < 200.0 && (wy - 2000.0).abs() < 200.0); // f32 round-trip tolerance
    }
    #[test]
    fn rects_and_background() {
        let v = View {
            center_x: -500.0,
            center_y: 135.0,
            factor: 582550.0 / 4096.0,
        };
        let space = Space::new(
            2480661,
            "APPALACHIA",
            "Appalachia",
            true,
            true,
            -500.0,
            135.0,
            582550.0,
            0.0,
        );
        let (x, y, w, h) = image_rect_of_world(&v, -500.0, 135.0, v.factor, v.factor);
        assert_eq!((x, y, w, h), (2048.0, 2048.0, 1.0, 1.0));
        let (wx, wy, ww, wh) = world_rect_of_image(&v, 2048.0, 2048.0, 1.0, 1.0);
        assert!((wx + 500.0).abs() < 1.0 && (wy - 135.0).abs() < 1.0);
        assert!((ww - v.factor).abs() < 1.0 && (wh - v.factor).abs() < 1.0);
        assert_eq!(background_rect(&space, None), (0.0, 0.0, 4096.0, 4096.0));
    }
    #[test]
    fn spot_factor_size_two() {
        // C# GetWorldToImageFactor spotlight branch: size_tiles * TILE_WIDTH / 4096.
        let spot = spot_view((1000.0, 2000.0), 2.0);
        assert_eq!(spot.factor, 2.0 * 8192.0 / 4096.0);
        assert_eq!(spot.factor, 4.0);
        assert_eq!((spot.center_x, spot.center_y), (1000.0, 2000.0));
    }
    #[test]
    fn spot_center_maps_to_center() {
        let spot = spot_view((1000.0, 2000.0), 2.0);
        assert_eq!(world_to_image(&spot, 1000.0, 2000.0), (2048.0, 2048.0));
        // one spot factor east -> +1px, same Y-flip rule as the full-canvas view
        assert_eq!(
            world_to_image(&spot, 1000.0 + spot.factor, 2000.0),
            (2049.0, 2048.0)
        );
        let (px, py) = world_to_image(&spot, 1500.0, 2500.0);
        let (wx, wy) = image_to_world(&spot, px, py);
        assert!((wx - 1500.0).abs() < 1.0 && (wy - 2500.0).abs() < 1.0);
    }
    #[test]
    fn spot_background_rect_appalachia_zoomed() {
        // Appalachia (DB row 2480661): center (-500, 135), range 582550.
        let space = Space::new(
            2480661,
            "APPALACHIA",
            "Appalachia",
            true,
            true,
            -500.0,
            135.0,
            582550.0,
            0.0,
        );
        // Spotlight on the space center, 2 tiles wide -> factor 4.0.
        let spot = spot_view((-500.0, 135.0), 2.0);
        // Hand-computed AsImageRectangle of the space rect
        // (left -291775, top 291410, 582550 x 582550) at factor 4.0:
        // x = (-291775 + 500)/4 + 2048 = -70770.75, w = 582550/4 = 145637.5.
        let (x, y, w, h) = background_rect(&space, Some(&spot));
        assert!((x + 70770.75).abs() < 0.01, "x = {x}");
        assert!((y + 70770.75).abs() < 0.01, "y = {y}");
        assert!((w - 145637.5).abs() < 0.01, "w = {w}");
        assert!((h - 145637.5).abs() < 0.01, "h = {h}");
    }
}
