#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
impl Rgba {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// Mirrors ImageHelper.LerpColors(Color, Color, range): (int) truncation per channel.
pub fn lerp(x: Rgba, y: Rgba, range: f64) -> Rgba {
    Rgba::new(
        (x.r as f64 + ((y.r as f64 - x.r as f64) * range)) as u8,
        (x.g as f64 + ((y.g as f64 - x.g as f64) * range)) as u8,
        (x.b as f64 + ((y.b as f64 - x.b as f64) * range)) as u8,
        255,
    )
}

/// Mirrors ImageHelper.LerpColors(Color[], range): single->itself, empty->Black, range 0/1 edges,
/// position = range*(n-1), floor/frac segment lerp. Panics outside [0,1] like C#.
pub fn lerp_palette(colors: &[Rgba], range: f64) -> Rgba {
    if colors.len() == 1 {
        return colors[0];
    }
    if colors.is_empty() {
        return Rgba::new(0, 0, 0, 255);
    }
    assert!((0.0..=1.0).contains(&range), "range out of [0,1]");
    if range == 0.0 {
        return colors[0];
    }
    if range == 1.0 {
        return colors[colors.len() - 1];
    }
    let pos = range * (colors.len() - 1) as f64;
    let prev = pos.floor() as usize;
    lerp(colors[prev], colors[prev + 1], pos - prev as f64)
}

pub fn with_alpha(c: Rgba, alpha: u8) -> Rgba {
    Rgba::new(c.r, c.g, c.b, alpha)
}

/// Mirrors GenerateColorMatrix: grayscale ? 0.299/0.587/0.114 rows * brightness : brightness diagonal.
/// Alpha channel untouched.
pub fn apply_brightness_grayscale(px: Rgba, brightness: f32, grayscale: bool) -> Rgba {
    if !grayscale {
        return Rgba::new(
            (px.r as f32 * brightness) as u8,
            (px.g as f32 * brightness) as u8,
            (px.b as f32 * brightness) as u8,
            px.a,
        );
    }
    let v = (0.299 * px.r as f32 + 0.587 * px.g as f32 + 0.114 * px.b as f32) * brightness;
    let v = v as u8;
    Rgba::new(v, v, v, px.a)
}

/// Mirrors SetColor: RGB replaced, alpha preserved.
pub fn tint_keep_alpha(px: Rgba, color: Rgba) -> Rgba {
    Rgba::new(color.r, color.g, color.b, px.a)
}

/// Mirrors ReplaceBlackWithTransparency: opaque black -> transparent.
pub fn black_to_transparent(px: Rgba) -> Rgba {
    if px.a != 0 && px.r == 0 && px.g == 0 && px.b == 0 {
        Rgba::new(0, 0, 0, 0)
    } else {
        px
    }
}

/// Parse `#rrggbb`/`#rgb` or CSS named color (ColorTranslator.FromHtml/ToHtml compat).
pub fn parse_color(s: &str) -> Option<Rgba> {
    csscolorparser::parse(s).ok().map(|c| {
        let [r, g, b, a] = c.to_rgba8();
        Rgba::new(r, g, b, a)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lerp_midpoint() {
        assert_eq!(
            lerp(Rgba::new(0, 0, 0, 255), Rgba::new(100, 200, 50, 255), 0.5),
            Rgba::new(50, 100, 25, 255)
        );
    }
    #[test]
    fn grayscale_red() {
        let g = apply_brightness_grayscale(Rgba::new(255, 0, 0, 255), 1.0, true);
        assert_eq!((g.r, g.g, g.b), (76, 76, 76)); // 0.299*255 = 76.245 -> (int)76
    }
}
