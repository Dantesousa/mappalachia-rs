//! Domain model structs ported from the C# reference (READ-ONLY).
//!
//! Sources (all under `Library/` in the C# repo):
//! - `Coord.cs` — [`Coord`]
//! - `Shape.cs` — [`Shape`] ([`ShapeType`](crate::enums::ShapeType) lives in
//!   [`crate::enums`])
//! - `Entity.cs` (+`DerivedNPC.cs`, `DerivedScrap.cs`, `DerivedRawFlux.cs`) —
//!   [`Entity`] with the C# subclasses folded into [`EntityKind`]
//! - `Space.cs` — [`Space`]
//! - `Instance.cs` (+`BaseInstance.cs` for [`LockLevel`](crate::enums::LockLevel))
//!   — [`Instance`]
//! - `Cluster.cs` — [`Cluster`]
//! - `MapMarker.cs` — [`MapMarker`]
//! - `Region.cs` (+`RegionPoint.cs`) — [`Region`], [`RegionPoint`]
//! - `Location.cs` — [`Location`]
//! - `Cell.cs` (+`CellNeighbors.cs`) — [`Cell`], [`CellNeighbors`]
//! - `SpotlightTile.cs` — [`SpotlightTile`]
//!
//! # Porting notes (C# inheritance → Rust composition)
//! - C# `Space : Entity`, `Instance : BaseInstance`, `RegionPoint : Instance`,
//!   `Cell : Instance`, `Region : Entity`, `Location : Entity` become flat
//!   structs holding owned clones (`entity: Entity`, `space: Space`,
//!   `instance: Instance`). Cloning avoids reference cycles (`Region` owns its
//!   `RegionPoint`s, each of which owns a copy of the region's `Entity`).
//! - `Instance.cluster` is `Option<usize>` (owning cluster id assigned by the
//!   clustering pass) instead of the C# object back-pointer, which would be a
//!   reference cycle in Rust. [`Cluster`] therefore takes an `id` in addition
//!   to the C# `Cluster(origin)` parameter.
//! - C# `Entity.Equals` is `FormID + EditorID` only, `Instance.Equals` is
//!   `InstanceFormID` only, and `Space` inherits `Entity` equality; the
//!   manual `PartialEq`/`Eq`/`Hash` impls below mirror that exactly (so a
//!   recipe-deserialized `Space` with `DisplayName ""` still equals the DB
//!   `Space`, matching `PopulateFromData`).
//! - `Region` min/max level are `#[serde(skip)]`, matching the C#
//!   `[JsonIgnore]` attributes.

use crate::common::{
    to_hex_u32, CELL_SIZE, SPOTLIGHT_MIN_TILE_RESOLUTION, TILE_RADIUS, TILE_WIDTH,
};
use crate::enums::{FluxColor, LockLevel, ShapeType, Signature};

// ---------------------------------------------------------------------------
// Coord
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Coord` (a value type; `Copy` here too).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Coord {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Coord {
    /// Mirrors `new Coord(x, y, z = 0)`.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y, z: 0.0 }
    }

    pub fn new_xyz(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl std::fmt::Display for Coord {
    /// Mirrors C# `Coord.ToString()` → `{X=.., Y=.., Z=..}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{X={}, Y={}, Z={}}}", self.x, self.y, self.z)
    }
}

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Shape` (a readonly struct).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Shape {
    pub shape_type: ShapeType,
    pub bound_x: f64,
    pub bound_y: f64,
    pub bound_z: f64,
    /// Rotation of the shape in degrees around the Z axis.
    pub rot_z: f64,
}

impl Shape {
    pub fn new(
        shape_type: ShapeType,
        bound_x: f64,
        bound_y: f64,
        bound_z: f64,
        rot_z: f64,
    ) -> Self {
        Self {
            shape_type,
            bound_x,
            bound_y,
            bound_z,
            rot_z,
        }
    }
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

/// The C# `Entity` subclasses folded into one enum:
/// `DerivedNPC` → [`EntityKind::DerivedNpc`], `DerivedScrap` →
/// [`EntityKind::DerivedScrap`], `DerivedRawFlux(color)` →
/// [`EntityKind::DerivedFlux`]`(color)`, `Region` → [`EntityKind::Region`],
/// `Location` → [`EntityKind::Location`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EntityKind {
    Standard,
    DerivedNpc,
    DerivedScrap,
    DerivedFlux(FluxColor),
    Region,
    Location,
}

/// Mirrors C# `Library.Entity`.
///
/// Equality mirrors C# `Entity.Equals`/`GetHashCode`: `form_id + editor_id`
/// only (`display_name`, `signature`, `kind` are ignored).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Entity {
    pub form_id: u32,
    pub editor_id: String,
    pub display_name: String,
    pub signature: Signature,
    pub kind: EntityKind,
}

impl PartialEq for Entity {
    fn eq(&self, other: &Self) -> bool {
        self.form_id == other.form_id && self.editor_id == other.editor_id
    }
}

impl Eq for Entity {}

impl std::hash::Hash for Entity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.form_id.hash(state);
        self.editor_id.hash(state);
    }
}

impl Entity {
    pub fn new(
        form_id: u32,
        editor_id: impl Into<String>,
        display_name: impl Into<String>,
        signature: Signature,
        kind: EntityKind,
    ) -> Self {
        Self {
            form_id,
            editor_id: editor_id.into(),
            display_name: display_name.into(),
            signature,
            kind,
        }
    }

    /// Mirrors C# `DerivedNPC : Entity(0, editorID, $"Derived {editorID} Spawn", NPC_)`.
    pub fn derived_npc(editor_id: impl Into<String>) -> Self {
        let editor_id = editor_id.into();
        Self {
            form_id: 0,
            editor_id: editor_id.clone(),
            display_name: format!("Derived {editor_id} Spawn"),
            signature: Signature::Npc,
            kind: EntityKind::DerivedNpc,
        }
    }

    /// Mirrors C# `DerivedScrap : Entity(0, editorID, $"Derived {editorID} Scrap", MISC)`.
    pub fn derived_scrap(editor_id: impl Into<String>) -> Self {
        let editor_id = editor_id.into();
        Self {
            form_id: 0,
            editor_id: editor_id.clone(),
            display_name: format!("Derived {editor_id} Scrap"),
            signature: Signature::Misc,
            kind: EntityKind::DerivedScrap,
        }
    }

    /// Mirrors C# `DerivedRawFlux(color) : Entity(0, color, $"Derived Raw {color} Flux", ALCH)`.
    pub fn derived_flux(color: FluxColor) -> Self {
        Self {
            form_id: 0,
            editor_id: color.to_string(),
            display_name: format!("Derived Raw {color} Flux"),
            signature: Signature::Alch,
            kind: EntityKind::DerivedFlux(color),
        }
    }

    /// Mirrors C# `Entity.GetFriendlyFormID` (`FormID.ToHex()`, i.e. `X8`);
    /// every derived entity returns `"N/A"`.
    pub fn friendly_form_id(&self) -> String {
        match self.kind {
            EntityKind::Standard => to_hex_u32(self.form_id),
            _ => "N/A".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Space
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Space` (a Worldspace or a Cell space).
///
/// Equality mirrors C#: `Space` does not override `Equals`, so it inherits
/// `Entity` equality (`form_id + editor_id` only).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Space {
    pub entity: Entity,
    pub is_worldspace: bool,
    pub is_instanceable: bool,
    pub center_x: f64,
    pub center_y: f64,
    pub max_range: f64,
    pub north_angle: f64,
}

impl PartialEq for Space {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Eq for Space {}

impl std::hash::Hash for Space {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.entity.hash(state);
    }
}

impl Space {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        form_id: u32,
        editor_id: impl Into<String>,
        display_name: impl Into<String>,
        is_worldspace: bool,
        is_instanceable: bool,
        center_x: f64,
        center_y: f64,
        max_range: f64,
        north_angle: f64,
    ) -> Self {
        Self {
            entity: Entity::new(
                form_id,
                editor_id,
                display_name,
                if is_worldspace {
                    Signature::World
                } else {
                    Signature::Cell
                },
                EntityKind::Standard,
            ),
            is_worldspace,
            is_instanceable,
            center_x,
            center_y,
            max_range,
            north_angle,
        }
    }

    /// Mirrors C# `MinX = CenterX - (MaxRange / 2d)` (and Max/Min Y likewise).
    pub fn min_x(&self) -> f64 {
        self.center_x - (self.max_range / 2.0)
    }

    pub fn max_x(&self) -> f64 {
        self.center_x + (self.max_range / 2.0)
    }

    pub fn min_y(&self) -> f64 {
        self.center_y - (self.max_range / 2.0)
    }

    pub fn max_y(&self) -> f64 {
        self.center_y + (self.max_range / 2.0)
    }

    /// Accessed via variable name in the UI.
    pub fn friendly_name(&self) -> String {
        format!("{} ({})", self.entity.display_name, self.entity.editor_id)
    }

    pub fn is_appalachia(&self) -> bool {
        self.entity.editor_id.eq_ignore_ascii_case("APPALACHIA")
    }

    /// A space can be nuked iff it is Appalachia.
    pub fn is_nukable(&self) -> bool {
        self.is_appalachia()
    }

    pub fn center(&self) -> Coord {
        Coord::new(self.center_x, self.center_y)
    }

    /// Mirrors C# `GetRectangle()` → `(left, top, width, height)`.
    pub fn rectangle(&self) -> (f64, f64, f64, f64) {
        (
            self.min_x(),
            self.max_y(),
            (self.max_x() - self.min_x()).abs(),
            (self.max_y() - self.min_y()).abs(),
        )
    }

    /// Mirrors C# `GetTiles()`.
    pub fn tiles(&self) -> Vec<SpotlightTile> {
        let (left, top, width, height) = self.rectangle();
        SpotlightTile::tiles_in_rect(left, top, width, height, self)
    }

    /// Maximum spotlight size the space could utilise before the view gets
    /// smaller than the non-spotlight perspective.
    pub fn max_spotlight_benefit(&self) -> f64 {
        self.max_range / f64::from(TILE_WIDTH)
    }

    /// The space is large enough to meet the minimum threshold to suitably
    /// benefit from spotlight.
    pub fn is_suitable_for_spotlight(&self) -> bool {
        self.max_spotlight_benefit() >= SPOTLIGHT_MIN_TILE_RESOLUTION
    }
}

// ---------------------------------------------------------------------------
// Instance
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Instance` (+ the shared `BaseInstance` fields).
///
/// Equality mirrors C# `Instance.Equals`/`GetHashCode`: `instance_form_id`
/// only (all other fields are ignored).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Instance {
    pub entity: Entity,
    pub space: Space,
    pub coord: Coord,
    /// The instance FormID in base-10; should be unique.
    pub instance_form_id: u32,
    pub label: String,
    /// Where applicable (e.g. a loading trigger door), the space teleported to.
    pub teleports_to: Option<Space>,
    pub lock_level: LockLevel,
    pub primitive_shape: Option<Shape>,
    /// Expected quantity of the spawn (default 1).
    pub spawn_weight: f64,
    pub in_container: bool,
    /// Owning cluster id (see [`Cluster`]); `None` = not in a cluster.
    pub cluster: Option<usize>,
}

impl Instance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entity: Entity,
        space: Space,
        coord: Coord,
        instance_form_id: u32,
        label: impl Into<String>,
        teleports_to: Option<Space>,
        lock_level: LockLevel,
        primitive_shape: Option<Shape>,
        spawn_weight: f64,
        in_container: bool,
    ) -> Self {
        Self {
            entity,
            space,
            coord,
            instance_form_id,
            label: label.into(),
            teleports_to,
            lock_level,
            primitive_shape,
            spawn_weight,
            in_container,
            cluster: None,
        }
    }

    /// The height as Topograph plots would see it: the Z coord, or the top
    /// edge of the volume.
    pub fn height_for_topograph(&self) -> f64 {
        match self.primitive_shape {
            Some(shape) => self.coord.z + (shape.bound_z / 2.0),
            None => self.coord.z,
        }
    }

    pub fn is_member_of_cluster(&self) -> bool {
        self.cluster.is_some()
    }
}

impl PartialEq for Instance {
    fn eq(&self, other: &Self) -> bool {
        self.instance_form_id == other.instance_form_id
    }
}

impl Eq for Instance {}

impl std::hash::Hash for Instance {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.instance_form_id.hash(state);
    }
}

// ---------------------------------------------------------------------------
// Cluster
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Cluster`: a collection of instances forming a cluster.
/// `id` has no C# counterpart — it backs [`Instance::cluster`] since Rust has
/// no object identity for a back-pointer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cluster {
    pub id: usize,
    pub origin: Instance,
    pub members: Vec<Instance>,
}

impl Cluster {
    pub fn new(id: usize, origin: Instance) -> Self {
        let mut cluster = Self {
            id,
            origin: origin.clone(),
            members: Vec::new(),
        };
        cluster.add_member(origin);
        cluster
    }

    pub fn add_member(&mut self, mut instance: Instance) {
        instance.cluster = Some(self.id);
        if self.members.is_empty() {
            self.origin = instance.clone();
        }
        self.members.push(instance);
    }

    /// Removes the member with the given instance FormID, clearing its
    /// cluster link, and returns it (C# `RemoveMember`).
    pub fn remove_member(&mut self, instance_form_id: u32) -> Option<Instance> {
        let pos = self
            .members
            .iter()
            .position(|m| m.instance_form_id == instance_form_id)?;
        let mut removed = self.members.remove(pos);
        removed.cluster = None;
        Some(removed)
    }

    pub fn contains_instance(&self, instance_form_id: u32) -> bool {
        self.members
            .iter()
            .any(|m| m.instance_form_id == instance_form_id)
    }

    pub fn weight(&self) -> f64 {
        self.members.iter().map(|m| m.spawn_weight).sum()
    }
}

// ---------------------------------------------------------------------------
// MapMarker
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.MapMarker`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MapMarker {
    pub icon: String,
    pub label: String,
    pub space_form_id: u32,
    pub coord: Coord,
}

impl MapMarker {
    pub fn new(
        icon: impl Into<String>,
        label: impl Into<String>,
        space_form_id: u32,
        coord: Coord,
    ) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
            space_form_id,
            coord,
        }
    }
}

// ---------------------------------------------------------------------------
// Region + RegionPoint
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Region`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Region {
    pub entity: Entity,
    /// All region points forming this region (across every subregion).
    pub points: Vec<RegionPoint>,
    pub space: Space,
    #[serde(skip)]
    pub min_level: u32,
    #[serde(skip)]
    pub max_level: u32,
}

impl Region {
    pub fn new(
        form_id: u32,
        editor_id: impl Into<String>,
        space: Space,
        min_level: u32,
        max_level: u32,
    ) -> Self {
        Self {
            entity: Entity::new(
                form_id,
                editor_id,
                "",
                Signature::Region,
                EntityKind::Region,
            ),
            points: Vec::new(),
            space,
            min_level,
            max_level,
        }
    }

    pub fn add_point(&mut self, point: RegionPoint) {
        self.points.push(point);
    }

    pub fn sub_region_count(&self) -> usize {
        let mut indexes: Vec<u32> = self.points.iter().map(|p| p.sub_region_index).collect();
        indexes.sort_unstable();
        indexes.dedup();
        indexes.len()
    }

    /// Points of one subregion polygon. Mirrors the C# guard verbatim
    /// (`subRegionIndex > GetSubRegionCount()` throws — i.e. `== count` yields
    /// an empty list rather than an error).
    pub fn sub_region(&self, sub_region_index: u32) -> Result<Vec<RegionPoint>, String> {
        if sub_region_index > self.sub_region_count() as u32 {
            return Err(format!("No SubRegion with index {sub_region_index}"));
        }
        Ok(self
            .points
            .iter()
            .filter(|p| p.sub_region_index == sub_region_index)
            .cloned()
            .collect())
    }

    pub fn all_sub_regions(&self) -> Vec<Vec<RegionPoint>> {
        (0..self.sub_region_count() as u32)
            .filter_map(|i| self.sub_region(i).ok())
            .collect()
    }

    /// Ray casting algorithm over each subregion, ported verbatim from C#.
    pub fn contains_point(&self, n: Coord) -> bool {
        for sub_region in self.all_sub_regions() {
            let count = sub_region.len();
            if count == 0 {
                continue;
            }
            let mut inside = false;
            // Iterate all adjacent pairs of points, wrapping around.
            let mut j = count - 1;
            for i in 0..count {
                let a = sub_region[i].instance.coord;
                let b = sub_region[j].instance.coord;
                // If n is not between a and b in y dimension, skip.
                if (a.y > n.y) == (b.y > n.y) {
                    j = i;
                    continue;
                }
                let intersection_x = a.x + ((b.x - a.x) * ((n.y - a.y) / (b.y - a.y)));
                // X is left of line, flip 'inside'.
                if n.x < intersection_x {
                    inside = !inside;
                }
                j = i;
            }
            // Inside this subregion, no need to check others.
            if inside {
                return true;
            }
        }
        // Not inside any subregion.
        false
    }
}

/// Mirrors C# `Library.RegionPoint : Instance`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegionPoint {
    pub instance: Instance,
    /// Subregion polygon this vertex belongs to (most regions use only 0).
    pub sub_region_index: u32,
    /// Index of this vertex within its subregion, ascending clockwise.
    pub coord_index: u32,
}

impl RegionPoint {
    pub fn new(
        region_entity: Entity,
        space: Space,
        coord: Coord,
        sub_region_index: u32,
        coord_index: u32,
    ) -> Self {
        Self {
            instance: Instance::new(
                region_entity,
                space,
                coord,
                0,
                "",
                None,
                LockLevel::None,
                None,
                1.0,
                false,
            ),
            sub_region_index,
            coord_index,
        }
    }

    pub fn coord(&self) -> Coord {
        self.instance.coord
    }
}

// ---------------------------------------------------------------------------
// Location + Cell
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.Location`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Location {
    pub entity: Entity,
    pub space: Space,
    pub cells: Vec<Cell>,
}

impl Location {
    pub fn new(
        form_id: u32,
        editor_id: impl Into<String>,
        display_name: impl Into<String>,
        space: Space,
    ) -> Self {
        Self {
            entity: Entity::new(
                form_id,
                editor_id,
                display_name,
                Signature::Location,
                EntityKind::Location,
            ),
            space,
            cells: Vec::new(),
        }
    }

    pub fn add_cell(&mut self, cell: Cell) {
        self.cells.push(cell);
    }
}

/// Mirrors C# `Library.CellNeighbors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct CellNeighbors {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

/// Mirrors C# `Library.Cell` (a cell of a worldspace, not an interior cell).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub instance: Instance,
    pub x: i32,
    pub y: i32,
}

impl Cell {
    /// The cell coord is `(x * CellSize, y * CellSize + CellSize)`.
    pub fn new(location_entity: Entity, space: Space, x: i32, y: i32) -> Self {
        let coord = Coord::new(
            f64::from(x) * f64::from(CELL_SIZE),
            f64::from(y) * f64::from(CELL_SIZE) + f64::from(CELL_SIZE),
        );
        Self {
            instance: Instance::new(
                location_entity,
                space,
                coord,
                0,
                "",
                None,
                LockLevel::None,
                None,
                1.0,
                false,
            ),
            x,
            y,
        }
    }

    pub fn coord(&self) -> Coord {
        self.instance.coord
    }

    /// Which orthogonal neighbours exist among `siblings` (the cells of the
    /// parent location, mirroring the C# scan of `parentLocation.Cells`).
    pub fn neighbors_in(&self, siblings: &[Cell]) -> CellNeighbors {
        let mut neighbors = CellNeighbors::default();
        for cell in siblings {
            if cell.x == self.x && cell.y == self.y + 1 {
                neighbors.up = true;
            } else if cell.x == self.x && cell.y == self.y - 1 {
                neighbors.down = true;
            } else if cell.x == self.x - 1 && cell.y == self.y {
                neighbors.left = true;
            } else if cell.x == self.x + 1 && cell.y == self.y {
                neighbors.right = true;
            }
        }
        neighbors
    }
}

// ---------------------------------------------------------------------------
// SpotlightTile
// ---------------------------------------------------------------------------

/// Mirrors C# `Library.SpotlightTile`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpotlightTile {
    pub space: Space,
    pub x_center: i32,
    pub y_center: i32,
}

impl SpotlightTile {
    pub fn new(space: Space, x_center: i32, y_center: i32) -> Self {
        Self {
            space,
            x_center,
            y_center,
        }
    }

    /// All tiles existing within the rectangle of world coordinates
    /// `(left, top, width, height)`, mirroring `GetTilesInRect(RectangleF)`.
    pub fn tiles_in_rect(
        left: f64,
        top: f64,
        width: f64,
        height: f64,
        space: &Space,
    ) -> Vec<SpotlightTile> {
        // Mirrors `rect.Height *= -1`: the C# rect's Y is the top edge.
        let bottom = top - height;
        let right = left + width;
        let mut tiles = Vec::new();
        let mut x = Self::snap_to_tile_center(left);
        while x <= Self::snap_to_tile_center(right) {
            let mut y = Self::snap_to_tile_center(bottom);
            while y <= Self::snap_to_tile_center(top) {
                tiles.push(SpotlightTile::new(space.clone(), x, y));
                y += TILE_WIDTH;
            }
            x += TILE_WIDTH;
        }
        tiles
    }

    /// The center coord of the tile (or rather lines of tiles) at the given
    /// coordinate dimension, mirroring `SnapToTileCenter` verbatim.
    pub fn snap_to_tile_center(position: f64) -> i32 {
        let width = f64::from(TILE_WIDTH);
        let radius = f64::from(TILE_RADIUS);
        (position - (position % width) + (radius * if position < 0.0 { -1.0 } else { 1.0 })) as i32
    }

    pub fn x_id(&self) -> i32 {
        (self.x_center - TILE_RADIUS) / TILE_WIDTH
    }

    pub fn y_id(&self) -> i32 {
        (self.y_center - TILE_RADIUS) / TILE_WIDTH
    }

    pub fn contains_point(&self, coord: Coord) -> bool {
        let radius = f64::from(TILE_RADIUS);
        let (cx, cy) = (f64::from(self.x_center), f64::from(self.y_center));
        coord.x > cx - radius
            && coord.x < cx + radius
            && coord.y > cy - radius
            && coord.y < cy + radius
    }

    /// The rectangle in world coordinates defining this tile, mirroring
    /// `GetRectangle()` → `(left, top, width, height)`.
    pub fn rectangle(&self) -> (i32, i32, i32, i32) {
        (
            self.x_center - TILE_RADIUS,
            self.y_center + TILE_RADIUS,
            TILE_WIDTH,
            TILE_WIDTH,
        )
    }

    pub fn center(&self) -> Coord {
        Coord::new(f64::from(self.x_center), f64::from(self.y_center))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CELL_SIZE;

    fn test_space() -> Space {
        Space::new(
            0xABC,
            "APPALACHIA",
            "Appalachia",
            true,
            true,
            0.0,
            0.0,
            100_000.0,
            0.0,
        )
    }

    #[test]
    fn region_contains_point() {
        // Square (0,0)-(10,10), single subregion 0.
        let space = test_space();
        let mut region = Region::new(1, "TestRegion", space.clone(), 0, 0);
        for (i, (x, y)) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .iter()
            .enumerate()
        {
            region.add_point(RegionPoint::new(
                region.entity.clone(),
                space.clone(),
                Coord::new(*x, *y),
                0,
                i as u32,
            ));
        }
        assert_eq!(region.sub_region_count(), 1);
        assert!(region.contains_point(Coord::new(5.0, 5.0)));
        assert!(!region.contains_point(Coord::new(15.0, 5.0)));
    }

    #[test]
    fn cell_coord_math() {
        // Cell(Location, x=2, y=3) coord == (2*4096, 3*4096+4096).
        let space = test_space();
        let location = Location::new(1, "TestLoc", "Test Location", space.clone());
        let cell = Cell::new(location.entity.clone(), space, 2, 3);
        assert_eq!(cell.coord().x, 2.0 * f64::from(CELL_SIZE));
        assert_eq!(
            cell.coord().y,
            3.0 * f64::from(CELL_SIZE) + f64::from(CELL_SIZE)
        );
    }

    #[test]
    fn entity_friendly_form_id() {
        let standard = Entity::new(
            0x94FC1,
            "SomeEditor",
            "Some Display",
            Signature::Flora,
            EntityKind::Standard,
        );
        assert_eq!(standard.friendly_form_id(), "00094FC1");
        assert_eq!(Entity::derived_npc("Opossum").friendly_form_id(), "N/A");
        assert_eq!(Entity::derived_scrap("Steel").friendly_form_id(), "N/A");
        assert_eq!(
            Entity::derived_flux(FluxColor::Violet).friendly_form_id(),
            "N/A"
        );
        let flux = Entity::derived_flux(FluxColor::Cobalt);
        assert_eq!(flux.editor_id, "Cobalt");
        assert_eq!(flux.signature, Signature::Alch);
    }

    #[test]
    fn spotlight_tile_math() {
        // Snap rounds down to the tile edge then back to the center, away from zero.
        assert_eq!(SpotlightTile::snap_to_tile_center(0.0), TILE_RADIUS);
        assert_eq!(
            SpotlightTile::snap_to_tile_center(f64::from(TILE_WIDTH)),
            TILE_WIDTH + TILE_RADIUS
        );
        let space = test_space();
        let tile = SpotlightTile::new(space, TILE_RADIUS, TILE_RADIUS);
        assert_eq!(tile.x_id(), 0);
        assert!(tile.contains_point(Coord::new(4096.0, 4096.0)));
        assert!(!tile.contains_point(Coord::new(8193.0, 4096.0)));
        // Cell neighbours: only +x neighbour present.
        let loc = Location::new(1, "L", "L", tile.space.clone());
        let self_cell = Cell::new(loc.entity.clone(), loc.space.clone(), 0, 0);
        let siblings = vec![
            self_cell.clone(),
            Cell::new(loc.entity.clone(), loc.space.clone(), 1, 0),
        ];
        let n = self_cell.neighbors_in(&siblings);
        assert!(n.right && !n.left && !n.up && !n.down);
        // Space helpers mirror the C# derivations.
        assert!(test_space().is_appalachia());
        assert!(test_space().is_nukable());
        assert!(test_space().is_suitable_for_spotlight());
    }

    #[test]
    fn entity_equality_is_form_id_plus_editor_id() {
        let base = Entity::new(
            0x94FC1,
            "SomeEditor",
            "Some Display",
            Signature::Flora,
            EntityKind::Standard,
        );
        // Different display_name/kind still equal.
        let variant = Entity::new(
            0x94FC1,
            "SomeEditor",
            "Other Display",
            Signature::Npc,
            EntityKind::DerivedNpc,
        );
        assert_eq!(base, variant);
        // Different form_id unequal.
        let other_form = Entity::new(
            0x94FC2,
            "SomeEditor",
            "Some Display",
            Signature::Flora,
            EntityKind::Standard,
        );
        assert_ne!(base, other_form);
        // Different editor_id unequal.
        let other_editor = Entity::new(
            0x94FC1,
            "OtherEditor",
            "Some Display",
            Signature::Flora,
            EntityKind::Standard,
        );
        assert_ne!(base, other_editor);
    }

    #[test]
    fn instance_equality_is_instance_form_id_only() {
        let space = test_space();
        let entity = Entity::new(
            0x94FC1,
            "SomeEditor",
            "Some Display",
            Signature::Flora,
            EntityKind::Standard,
        );
        let a = Instance::new(
            entity.clone(),
            space.clone(),
            Coord::new(1.0, 2.0),
            12345,
            "Label A",
            None,
            LockLevel::None,
            None,
            1.0,
            false,
        );
        // Different coords/label (and everything but the id) still equal.
        let b = Instance::new(
            Entity::new(
                0xDEAD,
                "Other",
                "Other",
                Signature::Npc,
                EntityKind::DerivedNpc,
            ),
            space.clone(),
            Coord::new_xyz(9.0, 8.0, 7.0),
            12345,
            "Label B",
            None,
            LockLevel::Level1,
            None,
            2.0,
            true,
        );
        assert_eq!(a, b);
        // Different instance_form_id unequal.
        let c = Instance::new(
            entity,
            space,
            Coord::new(1.0, 2.0),
            54321,
            "Label A",
            None,
            LockLevel::None,
            None,
            1.0,
            false,
        );
        assert_ne!(a, c);
    }

    #[test]
    fn recipe_space_equals_db_appalachia() {
        // Full DB Appalachia space vs the recipe-deserialized stub, whose
        // DisplayName is "" (C# `[JsonIgnore]`).
        let db = Space::new(
            2480661,
            "APPALACHIA",
            "Appalachia",
            true,
            true,
            0.0,
            0.0,
            100_000.0,
            0.0,
        );
        let recipe = Space::new(2480661, "APPALACHIA", "", true, true, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(recipe, db);
    }
}
