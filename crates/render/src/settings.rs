//! Typed map/plot settings ported from the C# reference (READ-WRITE).
//!
//! Sources (C# reference, READ-ONLY):
//! - `Mappalachia/Settings/Settings.cs` — root settings (not ported: the
//!   `Space` setter is DB-coupled; the CLI resolves spaces explicitly via
//!   [`mappalachia_core::db::populate_from_data`]).
//! - `Mappalachia/Settings/MapSettings.cs` — [`MapSettings`] (20 fields).
//! - `Mappalachia/Settings/PlotSettings.cs` — [`PlotSettings`],
//!   [`PlotMode`], [`VolumeDrawMode`].
//! - `Mappalachia/Settings/PlotStyleSettings.cs` — [`PlotStyleSettings`].
//! - `Mappalachia/Settings/FontSettings.cs` — [`FontSettings`] (9 sizes).
//! - `Mappalachia/Settings/ClusterSettings.cs` — [`ClusterSettings`].
//! - `Mappalachia/Settings/HeatmapSettings.cs` — [`HeatmapSettings`].
//! - `Mappalachia/Settings/SearchSettings.cs` — [`SearchSettings`]
//!   (typed for the future egui frontend; unused by the draw pipeline).
//! - `Mappalachia/Map.cs` — [`BackgroundImageType`], [`LegendStyle`],
//!   [`LegendHorizontalAlignment`], [`LegendVerticalAlignment`],
//!   [`CompassStyle`], [`CoordinateGridPrecision`].
//!
//! # Porting notes
//! - Fields use `#[serde(rename_all = "PascalCase")]`, so e.g. `brightness`
//!   reads/writes `"Brightness"`, matching the .NET property names (the
//!   default `FileIO.JsonSerializerOptions` apply no naming policy).
//! - Missing fields fall back to the exact C# defaults (including
//!   `BackgroundImage = Menu`), except `Brightness`, which defaults to
//!   `1.0` as a deliberate product deviation from the C# `0.5`
//!   (see `default_brightness`).
//! - Enums serialize as PascalCase strings (the variant names, which already
//!   are PascalCase) and deserialize from either the string or the STJ
//!   numeric index (declaration order), since .NET writes enums as numbers
//!   (no `JsonStringEnumConverter` in `FileIO`). Unknown strings/indices are
//!   errors.
//! - Palettes serialize as `"#rrggbb"` via
//!   [`mappalachia_core::recipe::html_color`] (cf. `JsonConverterColorList`)
//!   and deserialize via [`crate::color::parse_color`] (hex or CSS named).
//!   An empty/absent palette falls back like the C# setter (`[White]`).
//! - `Brightness` is clamped to `0.1..=2.0` on read, mirroring the C#
//!   setter (`Math.Clamp`).
//! - `SpotlightLocation` reads/writes `{X, Y, Z}` like the C# `Coord`.
//! - `SearchSettings.SearchTerm` defaults to `""` (C# picks a random hint;
//!   deterministic here so renders/tests are reproducible).

use std::str::FromStr;

use mappalachia_core::enums::{LockLevel, Signature};
use mappalachia_core::model::Coord;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::color::{parse_color, Rgba};

// ---------------------------------------------------------------------------
// int-or-string enums (PascalCase strings; STJ numeric indices on read)
// ---------------------------------------------------------------------------

macro_rules! pascal_enum {
    ($name:ident, [$($variant:ident),* $(,)?]) => {
        /// Mirrors the same-named C# enum (declaration order preserved: it
        /// doubles as the STJ numeric form).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),*
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(match self {
                    $(Self::$variant => stringify!($variant)),*
                })
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str(concat!("a ", stringify!($name), " name or index"))
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$name, E> {
                        match v {
                            $(stringify!($variant) => Ok($name::$variant),)*
                            _ => Err(E::unknown_variant(v, &[$(stringify!($variant)),*])),
                        }
                    }
                    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<$name, E> {
                        let all = [$($name::$variant),*];
                        all.get(v as usize).copied().ok_or_else(|| {
                            E::invalid_value(
                                serde::de::Unexpected::Unsigned(v),
                                &"a valid enum index",
                            )
                        })
                    }
                    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<$name, E> {
                        u64::try_from(v)
                            .map_err(|_| E::invalid_value(serde::de::Unexpected::Signed(v), &"a valid enum index"))
                            .and_then(|u| self.visit_u64(u))
                    }
                }
                d.deserialize_any(V)
            }
        }
    };
}

pascal_enum!(BackgroundImageType, [Render, Menu, Military, None]);
pascal_enum!(LegendStyle, [Extended, Compact, Hidden]);
pascal_enum!(LegendHorizontalAlignment, [Left, Right]);
pascal_enum!(LegendVerticalAlignment, [Top, Center, Bottom]);
pascal_enum!(CompassStyle, [Off, WhenUseful, Always]);
pascal_enum!(CoordinateGridPrecision, [Fine, Medium, Large]);
pascal_enum!(PlotMode, [Standard, Topographic, Heatmap, Cluster]);
pascal_enum!(VolumeDrawMode, [Fill, Border, Both]);

// ---------------------------------------------------------------------------
// Field helpers
// ---------------------------------------------------------------------------

/// Product default for a missing `Brightness`.
///
/// Deliberate deviation from the C# reference (`MapSettings.Brightness =
/// 0.5`): fresh maps / UI state start at full brightness `1.0` (no dimming),
/// which the golden fixture also pins explicitly, so golden dHashes are
/// unaffected by this default. (The C# 0.5 default is still accepted on
/// read, and any value is clamped to `0.1..=2.0` like the C# setter.)
fn default_brightness() -> f32 {
    1.0
}

fn palette_to_html(palette: &[Rgba]) -> Vec<String> {
    palette
        .iter()
        .map(|c| mappalachia_core::recipe::html_color(c.r, c.g, c.b))
        .collect()
}

fn palette_from_html<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Rgba>, D::Error> {
    let raw = Vec::<String>::deserialize(d)?;
    raw.into_iter()
        .map(|s| {
            parse_color(&s).ok_or_else(|| serde::de::Error::custom(format!("bad color {s:?}")))
        })
        .collect()
}

fn default_palette() -> Vec<Rgba> {
    // PlotStyleSettings.Palette initializer.
    [
        (255, 69, 0),   // OrangeRed
        (0, 255, 255),  // Cyan
        (255, 255, 0),  // Yellow
        (255, 0, 255),  // Magenta
        (0, 255, 0),    // Lime
        (65, 105, 225), // RoyalBlue
        (255, 127, 80), // Coral
        (0, 128, 0),    // Green
    ]
    .into_iter()
    .map(|(r, g, b)| Rgba::new(r, g, b, 255))
    .collect()
}

fn default_secondary_palette() -> Vec<Rgba> {
    // PlotStyleSettings.SecondaryPalette initializer.
    [(0, 255, 255), (255, 69, 0), (255, 255, 255)]
        .into_iter()
        .map(|(r, g, b)| Rgba::new(r, g, b, 255))
        .collect()
}

fn fallback_palette(v: Vec<Rgba>) -> Vec<Rgba> {
    // Mirrors the C# setter guard: null/empty -> [White].
    if v.is_empty() {
        vec![Rgba::new(255, 255, 255, 255)]
    } else {
        v
    }
}

mod coord_pascal {
    //! `{X, Y, Z}` serde for [`Coord`] (mirrors the C# struct shape).

    use super::Coord;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Shadow {
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default)]
        z: f64,
    }

    pub fn serialize<S: Serializer>(c: &Coord, s: S) -> Result<S::Ok, S::Error> {
        Shadow {
            x: c.x,
            y: c.y,
            z: c.z,
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Coord, D::Error> {
        let sh = Shadow::deserialize(d)?;
        Ok(Coord::new_xyz(sh.x, sh.y, sh.z))
    }
}

fn default_spotlight_location() -> Coord {
    Coord::new(0.0, 0.0)
}

// ---------------------------------------------------------------------------
// FontSettings (9 sizes)
// ---------------------------------------------------------------------------

/// Mirrors C# `FontSettings` (all pixel sizes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct FontSettings {
    pub size_title: i32,
    pub size_legend: i32,
    pub size_items_in_other_spaces: i32,
    pub size_map_marker_label: i32,
    pub size_watermark: i32,
    pub size_cluster_label: i32,
    pub size_instance_form_id: i32,
    pub size_region_level: i32,
    pub size_coordinate_grid: i32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            size_title: 72,
            size_legend: 40,
            size_items_in_other_spaces: 20,
            size_map_marker_label: 20,
            size_watermark: 60,
            size_cluster_label: 32,
            size_instance_form_id: 32,
            size_region_level: 32,
            size_coordinate_grid: 32,
        }
    }
}

// ---------------------------------------------------------------------------
// HeatmapSettings / ClusterSettings / PlotStyleSettings
// ---------------------------------------------------------------------------

/// Mirrors C# `HeatmapSettings` (range in pixels).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct HeatmapSettings {
    pub range: i32,
    pub intensity: i32,
    pub live_update: bool,
}

impl Default for HeatmapSettings {
    fn default() -> Self {
        Self {
            range: 128,
            intensity: 32,
            live_update: true,
        }
    }
}

/// Mirrors C# `ClusterSettings` (range in game units).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ClusterSettings {
    pub range: i32,
    pub min_weight: i32,
    pub live_update: bool,
    pub cluster_per_legend_group: bool,
}

impl Default for ClusterSettings {
    fn default() -> Self {
        Self {
            range: 10000,
            min_weight: 3,
            live_update: true,
            cluster_per_legend_group: false,
        }
    }
}

/// Mirrors C# `PlotStyleSettings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlotStyleSettings {
    pub size: i32,
    #[serde(
        serialize_with = "ser_palette",
        deserialize_with = "de_palette",
        default = "default_palette"
    )]
    pub palette: Vec<Rgba>,
    #[serde(
        serialize_with = "ser_palette",
        deserialize_with = "de_palette",
        default = "default_secondary_palette"
    )]
    pub secondary_palette: Vec<Rgba>,
}

fn ser_palette<S: Serializer>(p: &[Rgba], s: S) -> Result<S::Ok, S::Error> {
    palette_to_html(p).serialize(s)
}

fn de_palette<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Rgba>, D::Error> {
    palette_from_html(d).map(fallback_palette)
}

impl Default for PlotStyleSettings {
    fn default() -> Self {
        Self {
            size: 50,
            palette: default_palette(),
            secondary_palette: default_secondary_palette(),
        }
    }
}

// ---------------------------------------------------------------------------
// PlotSettings
// ---------------------------------------------------------------------------

/// Mirrors C# `PlotSettings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlotSettings {
    pub mode: PlotMode,
    pub draw_instance_form_id: bool,
    pub volume_draw_mode: VolumeDrawMode,
    pub heatmap_settings: HeatmapSettings,
    pub cluster_settings: ClusterSettings,
    pub plot_style_settings: PlotStyleSettings,
    pub auto_find_plots_in_connected_spaces: bool,
    pub show_region_levels: bool,
    pub include_count_in_legend: bool,
}

impl Default for PlotSettings {
    fn default() -> Self {
        Self {
            mode: PlotMode::Standard,
            draw_instance_form_id: false,
            volume_draw_mode: VolumeDrawMode::Both,
            heatmap_settings: HeatmapSettings::default(),
            cluster_settings: ClusterSettings::default(),
            plot_style_settings: PlotStyleSettings::default(),
            auto_find_plots_in_connected_spaces: false,
            show_region_levels: true,
            include_count_in_legend: false,
        }
    }
}

// ---------------------------------------------------------------------------
// MapSettings (20 fields)
// ---------------------------------------------------------------------------

/// Mirrors C# `MapSettings` (constructor takes the root settings; the
/// `RootSettings` back-pointer itself is `[JsonIgnore]` and not ported).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MapSettings {
    pub font_settings: FontSettings,
    pub background_image: BackgroundImageType,
    #[serde(default = "default_brightness", deserialize_with = "de_brightness")]
    pub brightness: f32,
    #[serde(with = "coord_pascal", default = "default_spotlight_location")]
    pub spotlight_location: Coord,
    pub spotlight_size: f64,
    pub grayscale_background: bool,
    pub highlight_water: bool,
    pub map_marker_icons: bool,
    pub map_marker_labels: bool,
    pub map_marker_grayscale: bool,
    pub door_icon_scale: f64,
    pub legend_style: LegendStyle,
    pub legend_horizontal_alignment: LegendHorizontalAlignment,
    pub legend_vertical_alignment: LegendVerticalAlignment,
    pub title: String,
    pub compass_style: CompassStyle,
    pub show_coordinate_grid: bool,
    pub show_infestations: bool,
    pub coordinate_grid_precision: CoordinateGridPrecision,
    pub spotlight_enabled: bool,
}

fn de_brightness<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    // Accepts a missing/null field (product default 1.0; deliberate
    // deviation from the C# 0.5) or a present value (clamped).
    let opt = Option::<f32>::deserialize(d)?;
    Ok(opt.unwrap_or(1.0).clamp(0.1, 2.0))
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            font_settings: FontSettings::default(),
            background_image: BackgroundImageType::Menu,
            // Deliberate product deviation: C# defaults to 0.5; we start
            // at full brightness 1.0 (see `default_brightness`).
            brightness: 1.0,
            spotlight_location: Coord::new(0.0, 0.0),
            spotlight_size: 2.0,
            grayscale_background: false,
            highlight_water: false,
            map_marker_icons: false,
            map_marker_labels: false,
            map_marker_grayscale: false,
            door_icon_scale: 1.0,
            legend_style: LegendStyle::Compact,
            legend_horizontal_alignment: LegendHorizontalAlignment::Right,
            legend_vertical_alignment: LegendVerticalAlignment::Center,
            title: String::new(),
            compass_style: CompassStyle::WhenUseful,
            show_coordinate_grid: false,
            show_infestations: false,
            coordinate_grid_precision: CoordinateGridPrecision::Medium,
            spotlight_enabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// SearchSettings (typed for the future egui frontend)
// ---------------------------------------------------------------------------

fn ser_signatures<S: Serializer>(v: &[Signature], s: S) -> Result<S::Ok, S::Error> {
    v.iter()
        .map(Signature::as_csharp_name)
        .collect::<Vec<_>>()
        .serialize(s)
}

fn de_signatures<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Signature>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|name| {
            Signature::from_str(&name)
                .map_err(|e| serde::de::Error::custom(format!("bad signature {name:?}: {e}")))
        })
        .collect()
}

fn ser_lock_levels<S: Serializer>(v: &[LockLevel], s: S) -> Result<S::Ok, S::Error> {
    v.iter()
        .map(LockLevel::as_csharp_name)
        .collect::<Vec<_>>()
        .serialize(s)
}

fn de_lock_levels<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<LockLevel>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|name| {
            LockLevel::from_str(&name)
                .map_err(|e| serde::de::Error::custom(format!("bad lock level {name:?}: {e}")))
        })
        .collect()
}

/// Mirrors C# `SearchSettings`. Canonical form uses the C# variant-name
/// strings (`"NPC_"`, `"Level0"`); the UI-only `SearchTerm` hint default is
/// `""` instead of C#'s random hint so settings round-trip deterministically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SearchSettings {
    pub search_in_all_spaces: bool,
    pub advanced: bool,
    pub search_term: String,
    pub search_in_instances_only: bool,
    #[serde(serialize_with = "ser_signatures", deserialize_with = "de_signatures")]
    pub selected_signatures: Vec<Signature>,
    #[serde(
        serialize_with = "ser_lock_levels",
        deserialize_with = "de_lock_levels"
    )]
    pub selected_lock_levels: Vec<LockLevel>,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            search_in_all_spaces: false,
            advanced: false,
            search_term: String::new(),
            search_in_instances_only: false,
            selected_signatures: Signature::recommended_default(),
            selected_lock_levels: LockLevel::all(),
        }
    }
}

// ---------------------------------------------------------------------------
// Recipe-stub conversion
// ---------------------------------------------------------------------------

/// Parse typed [`MapSettings`] from a raw recipe `MapSettings` value
/// (missing fields take the C# defaults).
pub fn map_from_value(v: &serde_json::Value) -> Result<MapSettings, String> {
    serde_json::from_value(v.clone()).map_err(|e| e.to_string())
}

/// Parse typed [`PlotSettings`] from a raw recipe `PlotSettings` value
/// (missing fields take the C# defaults).
pub fn plot_from_value(v: &serde_json::Value) -> Result<PlotSettings, String> {
    serde_json::from_value(v.clone()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_csharp() {
        let map = MapSettings::default();
        // Deliberate product deviation from the C# 0.5 (see
        // `default_brightness`): fresh maps start at full brightness.
        assert_eq!(map.brightness, 1.0);
        assert_eq!(map.background_image, BackgroundImageType::Menu);
        assert_eq!(map.legend_style, LegendStyle::Compact);
        assert_eq!(
            map.legend_horizontal_alignment,
            LegendHorizontalAlignment::Right
        );
        assert_eq!(
            map.legend_vertical_alignment,
            LegendVerticalAlignment::Center
        );
        assert_eq!(map.compass_style, CompassStyle::WhenUseful);
        assert_eq!(
            map.coordinate_grid_precision,
            CoordinateGridPrecision::Medium
        );
        assert_eq!(map.door_icon_scale, 1.0);
        assert_eq!(map.spotlight_size, 2.0);
        assert_eq!(map.font_settings.size_title, 72);
        assert_eq!(map.font_settings.size_coordinate_grid, 32);

        let plot = PlotSettings::default();
        assert_eq!(plot.mode, PlotMode::Standard);
        assert_eq!(plot.volume_draw_mode, VolumeDrawMode::Both);
        assert!(plot.show_region_levels);
        assert!(!plot.include_count_in_legend);
        assert_eq!(plot.plot_style_settings.size, 50);
        assert_eq!(plot.plot_style_settings.palette.len(), 8);
        assert_eq!(plot.heatmap_settings.range, 128);
        assert_eq!(plot.cluster_settings.range, 10000);
    }

    #[test]
    fn empty_object_takes_all_defaults() {
        let map: MapSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(map, MapSettings::default());
        let plot: PlotSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(plot, PlotSettings::default());
    }

    #[test]
    fn enums_accept_strings_and_stj_indices() {
        // Canonical PascalCase strings.
        let map: MapSettings =
            serde_json::from_value(json!({"BackgroundImage": "Render", "Mode": 0})).unwrap();
        assert_eq!(map.background_image, BackgroundImageType::Render);
        // STJ numeric indices (what .NET actually writes).
        let map: MapSettings = serde_json::from_value(json!({"BackgroundImage": 1})).unwrap();
        assert_eq!(map.background_image, BackgroundImageType::Menu);
        let plot: PlotSettings = serde_json::from_value(json!({"Mode": 0})).unwrap();
        assert_eq!(plot.mode, PlotMode::Standard);
        let plot: PlotSettings =
            serde_json::from_value(json!({"Mode": "Heatmap", "VolumeDrawMode": 2})).unwrap();
        assert_eq!(plot.mode, PlotMode::Heatmap);
        assert_eq!(plot.volume_draw_mode, VolumeDrawMode::Both);
        // Out-of-range rejects.
        assert!(serde_json::from_value::<PlotSettings>(json!({"Mode": 99})).is_err());
        assert!(serde_json::from_value::<PlotSettings>(json!({"Mode": "Sideways"})).is_err());
    }

    #[test]
    fn enums_serialize_pascal_case() {
        let plot = PlotSettings::default();
        let v = serde_json::to_value(&plot).unwrap();
        assert_eq!(v["Mode"], json!("Standard"));
        assert_eq!(v["VolumeDrawMode"], json!("Both"));
        let map = MapSettings::default();
        let v = serde_json::to_value(&map).unwrap();
        assert_eq!(v["BackgroundImage"], json!("Menu"));
        assert_eq!(v["Brightness"], json!(1.0));
        assert_eq!(v["Title"], json!(""));
    }

    #[test]
    fn default_brightness_is_full_not_csharp_half() {
        // Deliberate product deviation from the C# 0.5 default.
        assert_eq!(MapSettings::default().brightness, 1.0);
        // A missing field takes the product default...
        let map: MapSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(map.brightness, 1.0);
        // ...while an explicit C# 0.5 still reads back as 0.5.
        let map: MapSettings = serde_json::from_value(json!({"Brightness": 0.5})).unwrap();
        assert_eq!(map.brightness, 0.5);
    }

    #[test]
    fn brightness_clamps_like_csharp_setter() {
        let map: MapSettings = serde_json::from_value(json!({"Brightness": 9.0})).unwrap();
        assert_eq!(map.brightness, 2.0);
        let map: MapSettings = serde_json::from_value(json!({"Brightness": 0.0})).unwrap();
        assert_eq!(map.brightness, 0.1);
    }

    #[test]
    fn palettes_round_trip_html_and_reject_empty() {
        let plot = PlotSettings::default();
        let v = serde_json::to_value(&plot).unwrap();
        assert_eq!(v["PlotStyleSettings"]["Palette"][0], json!("#ff4500"));
        let back: PlotSettings = serde_json::from_value(v).unwrap();
        assert_eq!(back, plot);
        // Empty palette falls back to [White] like the C# setter.
        let plot: PlotSettings =
            serde_json::from_value(json!({"PlotStyleSettings": {"Palette": []}})).unwrap();
        assert_eq!(
            plot.plot_style_settings.palette,
            vec![Rgba::new(255, 255, 255, 255)]
        );
        // Named CSS colors parse too.
        let plot: PlotSettings =
            serde_json::from_value(json!({"PlotStyleSettings": {"Palette": ["Red"]}})).unwrap();
        assert_eq!(
            plot.plot_style_settings.palette,
            vec![Rgba::new(255, 0, 0, 255)]
        );
    }

    #[test]
    fn fixture_values_parse() {
        let map = map_from_value(&json!({
            "Title": "Nuka-Cola Appalachia",
            "Brightness": 1.0,
            "BackgroundImage": "Render",
            "LegendStyle": "Compact",
        }))
        .unwrap();
        assert_eq!(map.title, "Nuka-Cola Appalachia");
        assert_eq!(map.brightness, 1.0);
        assert_eq!(map.background_image, BackgroundImageType::Render);
        let plot = plot_from_value(&json!({"Mode": "Standard"})).unwrap();
        assert_eq!(plot.mode, PlotMode::Standard);
    }
}
