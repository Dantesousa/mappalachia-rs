//! Recipe JSON load/save with the exact .NET wire shape (READ-WRITE).
//!
//! Sources (C# reference, READ-ONLY):
//! - `Mappalachia/Recipe.cs` — [`Recipe`] (`Space`, `MapSettings`,
//!   `PlotSettings`, `ItemsToPlot`, all PascalCase via the default
//!   `FileIO.JsonSerializerOptions`, i.e. `WriteIndented` only, no naming
//!   policy).
//! - `Mappalachia/GroupedSearchResult.cs` lines 36-87 —
//!   [`Grouped::legend_text`] ports `GenerateLegendText` verbatim.
//! - `Library/Entity.cs` — `Entity` carries
//!   `[JsonPolymorphic(TypeDiscriminatorPropertyName = "EntityChildType")]`
//!   with discriminators `DerivedNPC`/`DerivedScrap`/`DerivedRawFlux`/
//!   `Region`/`Location`; `DisplayName` and `Signature` are `[JsonIgnore]`.
//! - `Library/Space.cs` — every `Space`-specific property is `[JsonIgnore]`.
//! - `Mappalachia/GroupedSearchResult.cs` —
//!   `[JsonPolymorphic(TypeDiscriminatorPropertyName = "SearchResultType")]`
//!   with `SingularSearchResult` (+ `InstanceFormID`); grouped results
//!   serialize flat (no tag).
//! - `Mappalachia/JsonConverterColor.cs` — colors as HTML `#rrggbb`
//!   (`ColorTranslator.ToHtml`); see [`html_color`]/[`parse_html_color`].
//!
//! # Porting notes
//! - `model::{Entity, Space}` and `search::{Grouped, SearchResult}` keep
//!   their existing (snake_case) derives; this module performs the
//!   PascalCase mapping manually so .NET readers/writers interop.
//! - `Entity` serializes WITHOUT the `EntityChildType` tag when
//!   [`EntityKind::Standard`](crate::model::EntityKind::Standard) —
//!   emitting `"EntityChildType":"Standard"` would break .NET readers,
//!   which only know the five derived discriminators. It is still
//!   *accepted* on read (tolerant reader).
//! - `DisplayName`/`Signature` are skipped on write (C# `[JsonIgnore]`)
//!   but accepted on read (default `""` / kind-implied signature), since
//!   hand-written recipe files (and the Task 7 brief fixture) carry them.
//! - Enums serialize as numbers (System.Text.Json default, no
//!   `JsonStringEnumConverter` in `FileIO`), but the reader also accepts
//!   the C# variant-name strings (`"None"`, `"MISC"`, …).
//! - `Map`/`PlotSettings` are `serde_json::Value`-backed stubs (full
//!   settings arrive with the render crate); the raw value round-trips
//!   untouched and typed accessors expose `Mode`/`Brightness`/`Title`.
//! - `Region`/`Location` entities keep only `FormID`/`EditorID`/tag: their
//!   C# payloads (`Points`/`Cells`/`Space`) have no counterpart in the
//!   flat [`Grouped`](crate::search::Grouped) model, so they cannot
//!   round-trip through a recipe.

use std::path::Path;

use serde_json::{Map, Value};

use crate::enums::{FluxColor, LockLevel, Signature};
use crate::model::{Entity, EntityKind, Space};
use crate::search::{Grouped, SearchResult};

// ---------------------------------------------------------------------------
// Tables (index order == C# declaration order, i.e. the STJ numeric form)
// ---------------------------------------------------------------------------

const SIGNATURES: [Signature; 35] = [
    Signature::Lvli,
    Signature::Flora,
    Signature::Npc,
    Signature::Misc,
    Signature::Book,
    Signature::Alch,
    Signature::Container,
    Signature::Stat,
    Signature::Scol,
    Signature::Mstt,
    Signature::Furn,
    Signature::Note,
    Signature::Term,
    Signature::Armo,
    Signature::Weapon,
    Signature::Ammo,
    Signature::Proj,
    Signature::Currency,
    Signature::Key,
    Signature::Region,
    Signature::Location,
    Signature::Acti,
    Signature::Tact,
    Signature::Trap,
    Signature::Door,
    Signature::Hazd,
    Signature::Idlm,
    Signature::Light,
    Signature::Soun,
    Signature::Sech,
    Signature::Aspc,
    Signature::Txst,
    Signature::Bnds,
    Signature::World,
    Signature::Cell,
];

const LOCK_LEVELS: [LockLevel; 11] = [
    LockLevel::None,
    LockLevel::Level0,
    LockLevel::Level1,
    LockLevel::Level2,
    LockLevel::Level3,
    LockLevel::RequiresTerminal,
    LockLevel::RequiresKey,
    LockLevel::Chained,
    LockLevel::Barred,
    LockLevel::Inaccessible,
    LockLevel::Unknown,
];

const FLUX_COLORS: [FluxColor; 5] = [
    FluxColor::Crimson,
    FluxColor::Cobalt,
    FluxColor::Fluorescent,
    FluxColor::Violet,
    FluxColor::Yellowcake,
];

fn signature_from_index(i: u64) -> Result<Signature, String> {
    SIGNATURES
        .get(i as usize)
        .copied()
        .ok_or_else(|| format!("Unexpected signature index {i}"))
}

fn lock_level_index(level: LockLevel) -> u64 {
    LOCK_LEVELS.iter().position(|l| *l == level).unwrap_or(0) as u64
}

fn lock_level_from_index(i: u64) -> Result<LockLevel, String> {
    LOCK_LEVELS
        .get(i as usize)
        .copied()
        .ok_or_else(|| format!("Unexpected lock level index {i}"))
}

fn flux_color_index(color: FluxColor) -> u64 {
    FLUX_COLORS.iter().position(|c| *c == color).unwrap_or(0) as u64
}

fn flux_color_from_index(i: u64) -> Result<FluxColor, String> {
    FLUX_COLORS
        .get(i as usize)
        .copied()
        .ok_or_else(|| format!("Unexpected flux color index {i}"))
}

// ---------------------------------------------------------------------------
// Colors (#rrggbb, cf. JsonConverterColor / ColorTranslator.ToHtml)
// ---------------------------------------------------------------------------

/// Formats an RGB color as lowercase HTML hex (`#rrggbb`).
/// Hex-only output; named colors are never produced.
pub fn html_color(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Parses an HTML hex color (`#rrggbb`, case-insensitive); `None` on failure.
/// Hex-only: named colors like "Red" are not accepted.
pub fn parse_html_color(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

// ---------------------------------------------------------------------------
// Small Value helpers
// ---------------------------------------------------------------------------

fn get_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn get_u32(v: &Value, key: &str) -> Result<u32, String> {
    let n = v
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Missing or invalid {key}"))?;
    u32::try_from(n).map_err(|_| format!("{key} out of range: {n}"))
}

fn get_f64(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn get_bool(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

/// The `EntityChildType` discriminator, or `None` for base entities (the
/// tag is omitted on write so .NET readers keep working).
fn entity_child_type(entity: &Entity) -> Option<&'static str> {
    match &entity.kind {
        EntityKind::Standard => None,
        EntityKind::DerivedNpc => Some("DerivedNPC"),
        EntityKind::DerivedScrap => Some("DerivedScrap"),
        EntityKind::DerivedFlux(_) => Some("DerivedRawFlux"),
        EntityKind::Region => Some("Region"),
        EntityKind::Location => Some("Location"),
    }
}

/// The signature implied by the entity kind (used when the payload carries
/// no explicit `Signature`, which .NET never writes).
fn implied_signature(kind: &EntityKind) -> Signature {
    match kind {
        EntityKind::Standard => Signature::Lvli,
        EntityKind::DerivedNpc => Signature::Npc,
        EntityKind::DerivedScrap => Signature::Misc,
        EntityKind::DerivedFlux(_) => Signature::Alch,
        EntityKind::Region => Signature::Region,
        EntityKind::Location => Signature::Location,
    }
}

fn parse_signature_value(v: &Value) -> Result<Option<Signature>, String> {
    match v.get("Signature") {
        None => Ok(None),
        Some(Value::String(s)) => s.parse::<Signature>().map(Some),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| "Invalid Signature number".to_string())
            .and_then(|i| signature_from_index(i).map(Some)),
        Some(_) => Err("Invalid Signature".to_string()),
    }
}

fn parse_flux_color_value(v: &Value) -> Result<Option<FluxColor>, String> {
    match v.get("Color") {
        None => Ok(None),
        Some(Value::String(s)) => s.parse::<FluxColor>().map(Some),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| "Invalid Color number".to_string())
            .and_then(|i| flux_color_from_index(i).map(Some)),
        Some(_) => Err("Invalid Color".to_string()),
    }
}

fn entity_to_json(entity: &Entity) -> Value {
    let mut m = Map::new();
    // STJ polymorphic writers emit the discriminator first.
    if let Some(tag) = entity_child_type(entity) {
        m.insert(
            "EntityChildType".to_string(),
            Value::String(tag.to_string()),
        );
    }
    m.insert("FormID".to_string(), Value::from(entity.form_id));
    m.insert(
        "EditorID".to_string(),
        Value::String(entity.editor_id.clone()),
    );
    // DisplayName + Signature skipped: C# [JsonIgnore].
    if let EntityKind::DerivedFlux(color) = entity.kind {
        m.insert("Color".to_string(), Value::from(flux_color_index(color)));
    }
    Value::Object(m)
}

fn entity_from_json(v: &Value) -> Result<Entity, String> {
    let form_id = get_u32(v, "FormID")?;
    let editor_id = get_str(v, "EditorID");
    let display_name = get_str(v, "DisplayName");
    let signature = parse_signature_value(v)?;

    let kind = match v.get("EntityChildType").and_then(Value::as_str) {
        // Absent (or "Standard", which .NET never writes but we tolerate).
        None | Some("Standard") => EntityKind::Standard,
        Some("DerivedNPC") => EntityKind::DerivedNpc,
        Some("DerivedScrap") => EntityKind::DerivedScrap,
        Some("DerivedRawFlux") => {
            let color = match editor_id.parse::<FluxColor>() {
                Ok(c) => c,
                Err(_) => parse_flux_color_value(v)?.ok_or_else(|| {
                    format!("DerivedRawFlux with unparseable color EditorID {editor_id:?}")
                })?,
            };
            EntityKind::DerivedFlux(color)
        }
        Some("Region") => EntityKind::Region,
        Some("Location") => EntityKind::Location,
        Some(other) => return Err(format!("Unexpected EntityChildType {other}")),
    };
    let signature = signature.unwrap_or_else(|| implied_signature(&kind));
    // DisplayName falls back to the derived constructors' computed names so
    // a bare .NET payload still yields usable legend text.
    let mut entity = match &kind {
        EntityKind::DerivedNpc => Entity::derived_npc(editor_id),
        EntityKind::DerivedScrap => Entity::derived_scrap(editor_id),
        EntityKind::DerivedFlux(color) => Entity::derived_flux(*color),
        _ => Entity::new(form_id, editor_id, display_name, signature, kind),
    };
    if !matches!(entity.kind, EntityKind::Standard) {
        entity.form_id = form_id;
        entity.signature = signature;
        if v.get("DisplayName").and_then(Value::as_str).is_some() {
            entity.display_name = get_str(v, "DisplayName");
        }
    }
    Ok(entity)
}

// ---------------------------------------------------------------------------
// Space (serializes as bare FormID + EditorID; everything else JsonIgnore)
// ---------------------------------------------------------------------------

fn space_to_json(space: &Space) -> Value {
    let mut m = Map::new();
    m.insert("FormID".to_string(), Value::from(space.form_id()));
    m.insert(
        "EditorID".to_string(),
        Value::String(space.editor_id().to_string()),
    );
    Value::Object(m)
}

fn space_from_json(v: &Value) -> Result<Space, String> {
    let form_id = get_u32(v, "FormID")?;
    let editor_id = get_str(v, "EditorID");
    let display_name = get_str(v, "DisplayName");
    // is_worldspace mirrors the signature; absent (the .NET case) falls
    // back to the C# default `false`. Only form_id + editor_id matter for
    // equality / PopulateFromData anyway.
    let is_worldspace = match parse_signature_value(v)? {
        Some(Signature::World) => true,
        Some(Signature::Cell) => false,
        Some(_) => false,
        None => false,
    };
    Ok(Space::new(
        form_id,
        editor_id,
        display_name,
        is_worldspace,
        false,
        0.0,
        0.0,
        0.0,
        0.0,
    ))
}

// ---------------------------------------------------------------------------
// Grouped / SearchResult
// ---------------------------------------------------------------------------

fn parse_lock_level_value(v: &Value) -> Result<LockLevel, String> {
    match v.get("LockLevel") {
        None => Ok(LockLevel::None),
        Some(Value::String(s)) if s.is_empty() => Ok(LockLevel::None),
        Some(Value::String(s)) => s.parse::<LockLevel>(),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| "Invalid LockLevel number".to_string())
            .and_then(lock_level_from_index),
        Some(_) => Err("Invalid LockLevel".to_string()),
    }
}

fn grouped_to_json(g: &Grouped) -> Value {
    let mut m = Map::new();
    m.insert("Entity".to_string(), entity_to_json(&g.entity));
    m.insert("Space".to_string(), space_to_json(&g.space));
    m.insert("Count".to_string(), Value::from(g.count));
    m.insert("SpawnWeight".to_string(), Value::from(g.spawn_weight));
    m.insert("Label".to_string(), Value::String(g.label.clone()));
    m.insert(
        "LockLevel".to_string(),
        Value::from(lock_level_index(g.lock_level)),
    );
    m.insert("InContainer".to_string(), Value::Bool(g.in_container));
    Value::Object(m)
}

fn grouped_from_json(v: &Value) -> Result<Grouped, String> {
    let entity = entity_from_json(v.get("Entity").ok_or("Missing Entity")?)?;
    let space = space_from_json(v.get("Space").ok_or("Missing Space")?)?;
    Ok(Grouped {
        entity,
        space,
        count: v
            .get("Count")
            .and_then(Value::as_i64)
            .ok_or("Missing or invalid Count")?,
        spawn_weight: get_f64(v, "SpawnWeight", 1.0),
        label: get_str(v, "Label"),
        lock_level: parse_lock_level_value(v)?,
        in_container: get_bool(v, "InContainer"),
    })
}

fn search_result_to_json(r: &SearchResult) -> Value {
    match r {
        SearchResult::Grouped(g) => grouped_to_json(g),
        SearchResult::Singular {
            inner,
            instance_form_id,
        } => {
            let mut m = grouped_to_json(inner)
                .as_object()
                .cloned()
                .expect("grouped serializes as object");
            m.insert(
                "SearchResultType".to_string(),
                Value::String("SingularSearchResult".to_string()),
            );
            m.insert("InstanceFormID".to_string(), Value::from(*instance_form_id));
            Value::Object(m)
        }
    }
}

fn search_result_from_json(v: &Value) -> Result<SearchResult, String> {
    match v.get("SearchResultType").and_then(Value::as_str) {
        None => grouped_from_json(v).map(SearchResult::Grouped),
        Some("SingularSearchResult") => {
            let inner = grouped_from_json(v)?;
            let instance_form_id = get_u32(v, "InstanceFormID")?;
            Ok(SearchResult::Singular {
                inner,
                instance_form_id,
            })
        }
        Some(other) => Err(format!("Unexpected SearchResultType {other}")),
    }
}

// ---------------------------------------------------------------------------
// Settings stubs (Value-backed; full settings arrive with the render crate)
// ---------------------------------------------------------------------------

/// `MapSettings` stub: preserves the raw JSON value; typed accessors expose
/// the fields the recipe flow needs (`Brightness`, `Title`).
#[derive(Debug, Clone, PartialEq)]
pub struct MapSettingsStub {
    value: Value,
}

impl MapSettingsStub {
    pub fn brightness(&self) -> f64 {
        self.value
            .get("Brightness")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    }

    pub fn title(&self) -> &str {
        self.value
            .get("Title")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn legend_style(&self) -> &str {
        self.value
            .get("LegendStyle")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn raw(&self) -> &Value {
        &self.value
    }
}

impl From<Value> for MapSettingsStub {
    fn from(value: Value) -> Self {
        Self { value }
    }
}

/// `PlotSettings` stub: preserves the raw JSON value; typed accessors
/// expose the fields the recipe flow needs (`Mode`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlotSettingsStub {
    value: Value,
}

impl PlotSettingsStub {
    /// The `PlotMode` name (`Standard`/`Topographic`/`Heatmap`/`Cluster`).
    pub fn mode(&self) -> &str {
        self.value.get("Mode").and_then(Value::as_str).unwrap_or("")
    }

    pub fn include_count_in_legend(&self) -> bool {
        self.value
            .get("IncludeCountInLegend")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn raw(&self) -> &Value {
        &self.value
    }
}

impl From<Value> for PlotSettingsStub {
    fn from(value: Value) -> Self {
        Self { value }
    }
}

// ---------------------------------------------------------------------------
// Recipe
// ---------------------------------------------------------------------------

/// Mirrors C# `Mappalachia.Recipe`.
#[derive(Debug, Clone)]
pub struct Recipe {
    pub space: Space,
    pub map_settings: MapSettingsStub,
    pub plot_settings: PlotSettingsStub,
    pub items_to_plot: Vec<SearchResult>,
}

impl Recipe {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("Space".to_string(), space_to_json(&self.space));
        m.insert("MapSettings".to_string(), self.map_settings.raw().clone());
        m.insert("PlotSettings".to_string(), self.plot_settings.raw().clone());
        m.insert(
            "ItemsToPlot".to_string(),
            Value::Array(
                self.items_to_plot
                    .iter()
                    .map(search_result_to_json)
                    .collect(),
            ),
        );
        Value::Object(m)
    }

    fn from_json(v: &Value) -> Result<Self, String> {
        let space = space_from_json(v.get("Space").ok_or("Missing Space")?)?;
        let map_settings =
            MapSettingsStub::from(v.get("MapSettings").cloned().unwrap_or(Value::Null));
        let plot_settings =
            PlotSettingsStub::from(v.get("PlotSettings").cloned().unwrap_or(Value::Null));
        let items = v
            .get("ItemsToPlot")
            .and_then(Value::as_array)
            .ok_or("Missing or invalid ItemsToPlot")?;
        let items_to_plot = items
            .iter()
            .map(search_result_from_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            space,
            map_settings,
            plot_settings,
            items_to_plot,
        })
    }

    /// Writes the recipe file (indented, like `FileIO` `WriteIndented`).
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(|e| e.to_string())?;
        std::fs::write(path.as_ref(), text).map_err(|e| e.to_string())
    }
}

impl serde::Serialize for Recipe {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Recipe {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(deserializer)?;
        Self::from_json(&v).map_err(serde::de::Error::custom)
    }
}

/// Mirrors C# `Recipe.LoadFromFile` (which returns null + notifies on
/// failure; here the error is returned as a string).
pub fn load_recipe_file(path: impl AsRef<Path>) -> Result<Recipe, String> {
    let text = std::fs::read_to_string(path.as_ref()).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Grouped::legend_text (ports GroupedSearchResult.GenerateLegendText)
// ---------------------------------------------------------------------------

/// Formats a count with thousands separators (`Count.ToString("N0")`).
/// Hardcodes the en-US comma separator; C# `N0` is culture-dependent.
fn format_n0(count: i64) -> String {
    let digits: Vec<char> = count.unsigned_abs().to_string().chars().collect();
    let mut out = String::new();
    for (i, ch) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*ch);
    }
    if count < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// C# `Math.Round(value, 2)` (banker's rounding) for the NPC percent suffix.
fn round_half_even_2(v: f64) -> f64 {
    let scaled = v * 100.0;
    let floor = scaled.floor();
    let frac = scaled - floor;
    let rounded = if frac > 0.5 || (frac == 0.5 && floor % 2.0 != 0.0) {
        floor + 1.0
    } else {
        floor
    };
    rounded / 100.0
}

impl Grouped {
    /// Ports `GroupedSearchResult.GenerateLegendText(settings)` verbatim:
    /// `"{count}x "` prefix, derived entities use `DisplayName` while base
    /// entities use `EditorID` + `("DisplayName")`, then label + lock
    /// context in parentheses, then the NPC `" [x%]"` / scrap `" [xN]"`
    /// suffixes. `include_count` mirrors
    /// `settings.PlotSettings.IncludeCountInLegend`.
    pub fn legend_text(&self, include_count: bool) -> String {
        let mut builder = String::new();
        if include_count && self.count != 1 {
            builder.push_str(&format!("{}x ", format_n0(self.count)));
        }

        let derived_name = matches!(
            self.entity.kind,
            EntityKind::DerivedNpc | EntityKind::DerivedScrap | EntityKind::DerivedFlux(_)
        );
        let mut additional = Vec::new();
        if derived_name {
            builder.push_str(&self.entity.display_name);
        } else {
            builder.push_str(&self.entity.editor_id);
            if !self.entity.display_name.trim().is_empty() {
                additional.push(format!("\"{}\"", self.entity.display_name));
            }
        }
        if !self.label.trim().is_empty() {
            additional.push(self.label.clone());
        }
        if self.lock_level_relevant() {
            additional.push(self.lock_level.to_friendly_name_with_context().to_string());
        }
        if !additional.is_empty() {
            builder.push_str(&format!(" ({})", additional.join(", ")));
        }

        match self.entity.kind {
            EntityKind::DerivedNpc => {
                builder.push_str(&format!(
                    " [{}%]",
                    round_half_even_2(self.spawn_weight * 100.0)
                ));
            }
            EntityKind::DerivedScrap => {
                builder.push_str(&format!(" [x{}]", self.spawn_weight));
            }
            _ => {}
        }
        builder
    }

    /// Mirrors the C# `LockLevelRelevant` local: lockable signature,
    /// in-container, or a non-`None` lock level.
    fn lock_level_relevant(&self) -> bool {
        self.entity.signature.is_lockable()
            || self.in_container
            || self.lock_level != LockLevel::None
    }
}
