pub const MAP_IMAGE_RESOLUTION: i32 = 4096;
pub const SPOTLIGHT_MAX_SIZE: i32 = 16;
pub const SPOTLIGHT_MIN_TILE_RESOLUTION: f64 = 2.0;
pub const SPOTLIGHT_TILE_SIZE: i32 = 4096;
pub const SPOTLIGHT_SCALE: i32 = 2;
pub const TILE_WIDTH: i32 = SPOTLIGHT_TILE_SIZE * SPOTLIGHT_SCALE;
pub const TILE_RADIUS: i32 = TILE_WIDTH / 2;
pub const CELL_SIZE: i32 = 4096;

/// Mirrors Common.FormID regex `^(0[Xx])?([0-9A-Fa-f]{1,8})$`.
pub fn is_hex_form_id(s: &str) -> bool {
    let h = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    !h.is_empty() && h.len() <= 8 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Mirrors Common.HexToInt: parses optional-0x hex, panics on invalid like Convert.ToUInt32.
pub fn hex_to_int(hex: &str) -> u32 {
    let h = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    u32::from_str_radix(h, 16).expect("invalid hex FormID")
}

/// Mirrors uint.ToString("X8").
pub fn to_hex_u32(id: u32) -> String {
    format!("{:08X}", id)
}

pub fn without_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_form_id_detection() {
        assert!(is_hex_form_id("00094FC1"));
        assert!(is_hex_form_id("0x94fc1"));
        assert!(!is_hex_form_id("Nuka Cola"));
        assert_eq!(hex_to_int("00094FC1"), 610241);
        assert_eq!(to_hex_u32(610241), "00094FC1");
    }
}
