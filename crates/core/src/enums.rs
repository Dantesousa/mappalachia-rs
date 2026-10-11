//! Domain enums ported verbatim from the C# reference (READ-ONLY).
//!
//! Sources:
//! - `Library/Entity.cs` — [`Signature`] (variant order = UI relevance order)
//! - `Library/BaseInstance.cs` — [`LockLevel`]
//! - `Library/Common.cs` — [`FluxColor`] (`Common.FluxColor`)
//! - `Library/Shape.cs` — [`ShapeType`]
//! - `Mappalachia/DataTranslator.cs` (lines 36-175) — friendly names,
//!   descriptions, `IsRecommendedSelection`, `IsLockable`, `CanBeSearchedFor`
//! - `Mappalachia/Database.cs` (`ToStringForQuery`) — [`LockLevel::to_query_string`]
//! - `Library/ReaderExtensions.cs` (`GetLockLevel`) — parse documented on
//!   [`LockLevel::from_str`]
//!
//! NOTE: the C# `Signature` enum has 35 variants including `SOUN`; all 35 are
//! ported here (see report for the Task 2 brief's 34-count discrepancy).

use std::fmt;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Signature
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Signature`. Order matches the C# declaration order,
/// which the C# comment notes defines perceived relevance / UI sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Signature {
    Lvli,
    Flora,
    Npc,
    Misc,
    Book,
    Alch,
    Container,
    Stat,
    Scol,
    Mstt,
    Furn,
    Note,
    Term,
    Armo,
    Weapon,
    Ammo,
    Proj,
    Currency,
    Key,
    Region,
    Location,
    Acti,
    Tact,
    Trap,
    Door,
    Hazd,
    Idlm,
    Light,
    Soun,
    Sech,
    Aspc,
    Txst,
    Bnds,
    World,
    Cell,
}

impl FromStr for Signature {
    type Err = String;

    /// Accepts the exact C# enum names (`"NPC_"`, `"LVLI"`, …).
    /// Mirrors C# `Enum.Parse<Signature>` — anything else is an error.
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "LVLI" => Self::Lvli,
            "FLOR" => Self::Flora,
            "NPC_" => Self::Npc,
            "MISC" => Self::Misc,
            "BOOK" => Self::Book,
            "ALCH" => Self::Alch,
            "CONT" => Self::Container,
            "STAT" => Self::Stat,
            "SCOL" => Self::Scol,
            "MSTT" => Self::Mstt,
            "FURN" => Self::Furn,
            "NOTE" => Self::Note,
            "TERM" => Self::Term,
            "ARMO" => Self::Armo,
            "WEAP" => Self::Weapon,
            "AMMO" => Self::Ammo,
            "PROJ" => Self::Proj,
            "CNCY" => Self::Currency,
            "KEYM" => Self::Key,
            "REGN" => Self::Region,
            "LCTN" => Self::Location,
            "ACTI" => Self::Acti,
            "TACT" => Self::Tact,
            "TRAP" => Self::Trap,
            "DOOR" => Self::Door,
            "HAZD" => Self::Hazd,
            "IDLM" => Self::Idlm,
            "LIGH" => Self::Light,
            "SOUN" => Self::Soun,
            "SECH" => Self::Sech,
            "ASPC" => Self::Aspc,
            "TXST" => Self::Txst,
            "BNDS" => Self::Bnds,
            "WRLD" => Self::World,
            "CELL" => Self::Cell,
            _ => return Err(format!("Unexpected signature {s}")),
        })
    }
}

impl Signature {
    /// The exact C# enum name (what the DB stores and [`FromStr`] parses).
    pub fn as_csharp_name(&self) -> &'static str {
        match self {
            Self::Lvli => "LVLI",
            Self::Flora => "FLOR",
            Self::Npc => "NPC_",
            Self::Misc => "MISC",
            Self::Book => "BOOK",
            Self::Alch => "ALCH",
            Self::Container => "CONT",
            Self::Stat => "STAT",
            Self::Scol => "SCOL",
            Self::Mstt => "MSTT",
            Self::Furn => "FURN",
            Self::Note => "NOTE",
            Self::Term => "TERM",
            Self::Armo => "ARMO",
            Self::Weapon => "WEAP",
            Self::Ammo => "AMMO",
            Self::Proj => "PROJ",
            Self::Currency => "CNCY",
            Self::Key => "KEYM",
            Self::Region => "REGN",
            Self::Location => "LCTN",
            Self::Acti => "ACTI",
            Self::Tact => "TACT",
            Self::Trap => "TRAP",
            Self::Door => "DOOR",
            Self::Hazd => "HAZD",
            Self::Idlm => "IDLM",
            Self::Light => "LIGH",
            Self::Soun => "SOUN",
            Self::Sech => "SECH",
            Self::Aspc => "ASPC",
            Self::Txst => "TXST",
            Self::Bnds => "BNDS",
            Self::World => "WRLD",
            Self::Cell => "CELL",
        }
    }

    /// Mirrors `DataTranslator.ToFriendlyName(Signature)` verbatim.
    pub fn friendly_name(&self) -> &'static str {
        match self {
            Self::Acti => "Activator",
            Self::Alch => "Aid",
            Self::Ammo => "Ammo",
            Self::Armo => "Armor/Apparel",
            Self::Aspc => "Acoustics",
            Self::Bnds => "Spline",
            Self::Book => "Note/Plan",
            Self::Cell => "Cell",
            Self::Currency => "Currency",
            Self::Container => "Container",
            Self::Door => "Door",
            Self::Flora => "Flora",
            Self::Furn => "Furniture/Animation",
            Self::Hazd => "Hazard",
            Self::Idlm => "Idle marker",
            Self::Key => "Key",
            Self::Light => "Light",
            Self::Lvli => "Loot",
            Self::Misc => "Junk/Scrap",
            Self::Mstt => "Moveable static",
            Self::Note => "Holotape",
            Self::Npc => "NPC",
            Self::Proj => "Mine/Projectile",
            Self::Region => "Region",
            Self::Location => "Location",
            Self::Scol => "Static collection",
            Self::Sech => "Echo",
            Self::Soun => "Sound",
            Self::Stat => "Static object",
            Self::Tact => "Voice activator",
            Self::Term => "Terminal",
            Self::Trap => "Trap",
            Self::Txst => "Decal",
            Self::Weapon => "Weapon",
            Self::World => "World",
        }
    }

    /// Mirrors `DataTranslator.GetDescription(Signature)` verbatim.
    /// Returns empty string where the C# returns `string.Empty`
    /// (name already obvious).
    pub fn description(&self) -> &'static str {
        match self {
            Self::Acti => "Defined trigger volume, invisible in-game.\nActivators can mark out designated areas, trigger events, or act as 'hit-boxes' for interaction.",
            Self::Alch => "Food, drink, chems etc.",
            Self::Ammo => "",
            Self::Armo => "",
            Self::Aspc => "Applies specific environmental acoustics.",
            Self::Bnds => "A curved line shape. Usually used for power lines.",
            Self::Book => "Note, Plan or Recipe.",
            Self::Cell => "Another location separate from the main outside world.",
            Self::Currency => "",
            Self::Container => "Anything which can hold items.",
            Self::Door => "",
            Self::Flora => "Collectable natural resource.",
            Self::Furn => "Object or position which a character can use to enter into an animation.\nIncludes workbenches, instruments, or enemy ambush positions.",
            Self::Hazd => "Area of space which can harm the player (fire/radiation).",
            Self::Idlm => "Allows an NPC to enter an idle animation.",
            Self::Key => "",
            Self::Light => "",
            Self::Lvli => "A lootable object or set of potential objects, occasionally spawning conditionally or with variable odds.",
            Self::Misc => "Junk, Scrap or Mod.",
            Self::Mstt => "Environmental scenery which animates or responds to physics.",
            Self::Note => "",
            Self::Npc => "Non-player character.",
            Self::Proj => "An 'armed' weapon such as a mine.",
            Self::Region => "A defined bounded area on the map.",
            Self::Location => "A named location defined by associated grid squares (cells).",
            Self::Scol => "A grouped set of static objects.",
            Self::Sech => "Trigger for echo sound effect.",
            Self::Soun => "Trigger for sound effect.",
            Self::Stat => "Environmental scenery which does not move and cannot be interacted with.",
            Self::Tact => "A trigger/activator which causes a voice line to be played.",
            Self::Term => "",
            Self::Trap => "",
            Self::Txst => "A decal applied to a surface such as paint or dirt.",
            Self::Weapon => "",
            Self::World => "The main outside world.",
        }
    }

    /// Mirrors `DataTranslator.IsRecommendedSelection` verbatim.
    pub fn is_recommended_selection(&self) -> bool {
        matches!(
            self,
            Self::Acti
                | Self::Alch
                | Self::Ammo
                | Self::Armo
                | Self::Book
                | Self::Currency
                | Self::Container
                | Self::Flora
                | Self::Furn
                | Self::Key
                | Self::Lvli
                | Self::Misc
                | Self::Note
                | Self::Npc
                | Self::Proj
                | Self::Region
                | Self::Location
                | Self::Term
                | Self::Trap
                | Self::Weapon
        )
    }

    /// Mirrors `DataTranslator.IsLockable` verbatim.
    pub fn is_lockable(&self) -> bool {
        matches!(self, Self::Container | Self::Door | Self::Term)
    }

    /// Mirrors `DataTranslator.CanBeSearchedFor` verbatim.
    pub fn can_be_searched(&self) -> bool {
        !matches!(self, Self::Cell | Self::World)
    }

    /// Default selected signatures (used by Task 5 tests).
    /// Exact list per the Task 2 brief.
    pub fn recommended_default() -> Vec<Signature> {
        [
            Signature::Lvli,
            Signature::Flora,
            Signature::Npc,
            Signature::Misc,
            Signature::Book,
            Signature::Alch,
            Signature::Ammo,
            Signature::Armo,
            Signature::Currency,
            Signature::Container,
            Signature::Furn,
            Signature::Key,
            Signature::Region,
            Signature::Location,
            Signature::Term,
            Signature::Trap,
            Signature::Proj,
            Signature::Note,
            Signature::Weapon,
        ]
        .to_vec()
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.friendly_name())
    }
}

// ---------------------------------------------------------------------------
// LockLevel
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.LockLevel`. Order matches the C# declaration order
/// (perceived relevance / UI sort order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LockLevel {
    None,
    Level0,
    Level1,
    Level2,
    Level3,
    RequiresTerminal,
    RequiresKey,
    Chained,
    Barred,
    Inaccessible,
    Unknown,
}

impl FromStr for LockLevel {
    type Err = String;

    /// Accepts the exact C# variant names (`"Level0"`, `"RequiresTerminal"`, …).
    /// Mirrors C# `Enum.Parse<LockLevel>`: the DB stores variant names and
    /// `GetLockLevel` strips whitespace before parsing, so `"Level 0"` in the
    /// DB would fail in C# too. Empty/NULL → `None` is handled by the caller.
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "None" => Self::None,
            "Level0" => Self::Level0,
            "Level1" => Self::Level1,
            "Level2" => Self::Level2,
            "Level3" => Self::Level3,
            "RequiresTerminal" => Self::RequiresTerminal,
            "RequiresKey" => Self::RequiresKey,
            "Chained" => Self::Chained,
            "Barred" => Self::Barred,
            "Inaccessible" => Self::Inaccessible,
            "Unknown" => Self::Unknown,
            _ => return Err(format!("Unexpected lock level {s}")),
        })
    }
}

impl LockLevel {
    /// The exact C# variant name (what the DB stores and [`FromStr`] parses).
    pub fn as_csharp_name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Level0 => "Level0",
            Self::Level1 => "Level1",
            Self::Level2 => "Level2",
            Self::Level3 => "Level3",
            Self::RequiresTerminal => "RequiresTerminal",
            Self::RequiresKey => "RequiresKey",
            Self::Chained => "Chained",
            Self::Barred => "Barred",
            Self::Inaccessible => "Inaccessible",
            Self::Unknown => "Unknown",
        }
    }

    /// Mirrors `DataTranslator.ToFriendlyName(LockLevel)` verbatim.
    pub fn friendly_name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Level0 => "Level 0",
            Self::Level1 => "Level 1",
            Self::Level2 => "Level 2",
            Self::Level3 => "Level 3",
            Self::RequiresKey => "Requires Key",
            Self::Inaccessible => "Inaccessible",
            Self::RequiresTerminal => "Requires Terminal",
            Self::Barred => "Barred",
            Self::Chained => "Chained",
            Self::Unknown => "Unknown",
        }
    }

    /// Mirrors `DataTranslator.ToFriendlyNameWithContext(LockLevel)` verbatim:
    /// `None` → `"Not Locked"`, else the plain friendly name.
    pub fn to_friendly_name_with_context(&self) -> &'static str {
        match self {
            Self::None => "Not Locked",
            _ => self.friendly_name(),
        }
    }

    /// Mirrors `Database.ToStringForQuery`: None → `""`, else variant name.
    pub fn to_query_string(&self) -> &'static str {
        match self {
            Self::None => "",
            _ => self.as_csharp_name(),
        }
    }

    /// Mirrors `Enum.GetValues<LockLevel>().ToList()` — all 11 variants.
    /// (Used by Task 5 tests.)
    pub fn all() -> Vec<LockLevel> {
        [
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
        ]
        .to_vec()
    }
}

impl fmt::Display for LockLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.friendly_name())
    }
}

// ---------------------------------------------------------------------------
// FluxColor
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Common.FluxColor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FluxColor {
    Crimson,
    Cobalt,
    Fluorescent,
    Violet,
    Yellowcake,
}

impl FromStr for FluxColor {
    type Err = String;

    /// Accepts the exact C# enum names.
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "Crimson" => Self::Crimson,
            "Cobalt" => Self::Cobalt,
            "Fluorescent" => Self::Fluorescent,
            "Violet" => Self::Violet,
            "Yellowcake" => Self::Yellowcake,
            _ => return Err(format!("Unexpected flux color {s}")),
        })
    }
}

impl FluxColor {
    /// The exact C# enum name.
    pub fn as_csharp_name(&self) -> &'static str {
        match self {
            Self::Crimson => "Crimson",
            Self::Cobalt => "Cobalt",
            Self::Fluorescent => "Fluorescent",
            Self::Violet => "Violet",
            Self::Yellowcake => "Yellowcake",
        }
    }
}

impl fmt::Display for FluxColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_csharp_name())
    }
}

// ---------------------------------------------------------------------------
// ShapeType
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.ShapeType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ShapeType {
    Box,
    Sphere,
    Line,
    Plane,
    Ellipsoid,
    Cylinder,
}

impl FromStr for ShapeType {
    type Err = String;

    /// Accepts the exact C# enum names (mirrors `Enum.Parse<ShapeType>`
    /// in `ReaderExtensions.GetShape`).
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "Box" => Self::Box,
            "Sphere" => Self::Sphere,
            "Line" => Self::Line,
            "Plane" => Self::Plane,
            "Ellipsoid" => Self::Ellipsoid,
            "Cylinder" => Self::Cylinder,
            _ => return Err(format!("Unexpected shape type {s}")),
        })
    }
}

impl ShapeType {
    /// The exact C# enum name.
    pub fn as_csharp_name(&self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Sphere => "Sphere",
            Self::Line => "Line",
            Self::Plane => "Plane",
            Self::Ellipsoid => "Ellipsoid",
            Self::Cylinder => "Cylinder",
        }
    }
}

impl fmt::Display for ShapeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_csharp_name())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_names_and_flags() {
        assert_eq!(Signature::Npc.friendly_name(), "NPC");
        assert_eq!(Signature::Misc.friendly_name(), "Junk/Scrap");
        assert!(Signature::Flora.is_recommended_selection());
        assert!(!Signature::World.is_recommended_selection());
        assert!(Signature::Container.is_lockable());
        assert!(!Signature::World.can_be_searched());
        assert_eq!(LockLevel::None.to_query_string(), "");
        assert_eq!(LockLevel::RequiresKey.to_query_string(), "RequiresKey");
        assert_eq!("NPC_".parse::<Signature>().unwrap(), Signature::Npc);
    }

    #[test]
    fn verbatim_mappings_round_trip() {
        // SOUN (35th C# variant): parse, C# name, friendly name, description.
        assert_eq!("SOUN".parse::<Signature>().unwrap(), Signature::Soun);
        assert_eq!(Signature::Soun.as_csharp_name(), "SOUN");
        assert_eq!(Signature::Soun.friendly_name(), "Sound");
        assert_eq!(Signature::Soun.description(), "Trigger for sound effect.");
        // LockLevel friendly names verbatim, incl. context variant for None.
        assert_eq!(LockLevel::Level0.friendly_name(), "Level 0");
        assert_eq!(
            LockLevel::None.to_friendly_name_with_context(),
            "Not Locked"
        );
        assert_eq!(
            "RequiresTerminal".parse::<LockLevel>().unwrap(),
            LockLevel::RequiresTerminal
        );
        assert!("Level 0".parse::<LockLevel>().is_err());
        assert_eq!(LockLevel::all().len(), 11);
        assert_eq!(Signature::recommended_default().len(), 19);
        // FluxColor / ShapeType parse + display.
        assert_eq!("Violet".parse::<FluxColor>().unwrap(), FluxColor::Violet);
        assert_eq!(FluxColor::Yellowcake.to_string(), "Yellowcake");
        assert_eq!(
            "Cylinder".parse::<ShapeType>().unwrap(),
            ShapeType::Cylinder
        );
        assert_eq!(ShapeType::Box.to_string(), "Box");
        // Signature Display shows the friendly name.
        assert_eq!(Signature::Misc.to_string(), "Junk/Scrap");
        assert_eq!(LockLevel::RequiresKey.to_string(), "Requires Key");
    }
}
