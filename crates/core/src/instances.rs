//! Instance resolution ported verbatim from the C# reference (READ-ONLY).
//!
//! Sources (`Mappalachia/Database.cs` lines 377-853):
//! - `GetInstances` — dispatch over the 8 resolver branches
//! - singular, in-container, region, location, standard, scrap, NPC, flux
//!   resolvers plus `GetNonNukableRegions`
//! - `GetTeleporters`, `GetInfestationLocations`, `GetZRange`
//! - `GetSpaceByFormID` (0 → null), `ReaderExtensions.GetCoord/GetShape`
//!   (`Library/ReaderExtensions.cs`), `GeometryHelper.GetCentroid`
//!   (`Library/GeometryHelper.cs`)
//!
//! # Porting notes
//! - `Instance.cluster` is left as `None`: cluster membership is assigned
//!   later by the clustering pass, which owns the cluster ids
//!   (`Option<usize>` holds the owning cluster id; see [`crate::model`]).
//! - `teleportsToFormID` NULL *or* 0 maps to `None` (read as
//!   `Option<i64>`); any other id resolves via the space list, erroring when
//!   the target space is missing — mirroring C# `GetSpaceByFormID` (null for
//!   0, throw otherwise).
//! - `Region.spaceFormID` is TEXT in the DB, so it is read as `String` and
//!   parsed to `u32` (same as [`crate::search`]).
//! - `GetZRange` returns `(0, 0)` when there is nothing to plot or nothing
//!   resolves, and plots `height_for_topograph` (Z, or the top edge of the
//!   volume).
//! - Flux resolution excludes instances lying inside non-nukable regions
//!   (via [`crate::model::Region::contains_point`]).
//! - String literals interpolated into SQL (labels, names, editor ids) have
//!   `'` doubled. The C# interpolates them raw; the escaping only changes
//!   behaviour for values containing an apostrophe, where the raw C# query
//!   would break.
//! - `quantity = ?` / `componentQuantity = ?` / `spawnWeight = ?` are bound
//!   to the grouped result's `spawn_weight`, as in C# (`$spawnWeight`).
//! - C# `GetLocationInstance` returns a single `Instance`; here the location
//!   resolver returns a one-element `Vec` so the dispatch stays uniform. An
//!   empty cell set yields a `(0, 0)` coord (C# `Average()` would throw).

use std::str::FromStr;

use rusqlite::{Connection, Result};

use crate::enums::{LockLevel, ShapeType, Signature};
use crate::geometry::centroid;
use crate::model::{
    Cell, Coord, Entity, EntityKind, Instance, Location, Region, RegionPoint, Shape, Space,
};
use crate::search::SearchResult;

/// Double single quotes for SQL string literals (see module docs).
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// Mirrors C# `GetSpaceByFormID`: 0 → `None`, otherwise the matching space.
///
/// Errors (`QueryReturnedNoRows`) when a nonzero id has no space, mirroring
/// the C# throw.
fn space_lookup(spaces: &[Space], form_id: u32) -> Result<Option<Space>> {
    if form_id == 0 {
        return Ok(None);
    }
    spaces
        .iter()
        .find(|s| s.form_id() == form_id)
        .cloned()
        .map(Some)
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// Mirrors `GetSpaceByFormID(reader.GetUInt("teleportsToFormID"))`: NULL/0 →
/// `None` (the column is read as `Option<i64>`).
fn teleports_to(spaces: &[Space], raw: Option<i64>) -> Result<Option<Space>> {
    match raw {
        None | Some(0) => Ok(None),
        Some(id) => space_lookup(spaces, id as u32),
    }
}

/// Mirrors `ReaderExtensions.GetCoord` (NULL → 0).
fn coord_of(row: &rusqlite::Row<'_>) -> Result<Coord> {
    Ok(Coord::new_xyz(
        row.get::<_, Option<f64>>("x")?.unwrap_or(0.0),
        row.get::<_, Option<f64>>("y")?.unwrap_or(0.0),
        row.get::<_, Option<f64>>("z")?.unwrap_or(0.0),
    ))
}

/// Mirrors `ReaderExtensions.GetShape` (NULL/empty → `None`).
fn shape_of(row: &rusqlite::Row<'_>) -> Result<Option<Shape>> {
    let raw: Option<String> = row.get("primitiveShape")?;
    match raw {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            let shape_type = ShapeType::from_str(&s).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
            })?;
            Ok(Some(Shape::new(
                shape_type,
                row.get::<_, Option<f64>>("boundX")?.unwrap_or(0.0),
                row.get::<_, Option<f64>>("boundY")?.unwrap_or(0.0),
                row.get::<_, Option<f64>>("boundZ")?.unwrap_or(0.0),
                row.get::<_, Option<f64>>("rotZ")?.unwrap_or(0.0),
            )))
        }
    }
}

fn parse_signature(raw: String) -> Result<Signature> {
    Signature::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

/// Load the points of one region, mirroring the C# point loop verbatim.
fn load_region_points(conn: &Connection, region: &mut Region, space: &Space) -> Result<()> {
    let mut s = conn.prepare(
        "SELECT x, y, subRegionIndex, coordIndex FROM RegionPoints WHERE regionFormID = ?",
    )?;
    let rows = s.query_map([region.entity.form_id], |row| {
        Ok((
            row.get::<_, f64>("x")?,
            row.get::<_, f64>("y")?,
            row.get::<_, i64>("subRegionIndex")? as u32,
            row.get::<_, i64>("coordIndex")? as u32,
        ))
    })?;
    for r in rows {
        let (x, y, sub_region_index, coord_index) = r?;
        region.add_point(RegionPoint::new(
            region.entity.clone(),
            space.clone(),
            Coord::new(x, y),
            sub_region_index,
            coord_index,
        ));
    }
    Ok(())
}

/// Mirrors C# `GetRegion(editorID, space)`: the region's instance at the
/// first subregion's centroid, or `None` when the region row is missing.
fn region_instance(conn: &Connection, editor_id: &str, space: &Space) -> Result<Option<Instance>> {
    let query = format!(
        "SELECT minLevel, maxLevel, regionFormID FROM Region WHERE regionEditorID = '{}' AND spaceFormID = {}",
        esc(editor_id),
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let row: Option<(u32, u32, u32)> = s
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)? as u32,
                r.get::<_, i64>(1)? as u32,
                r.get::<_, i64>(2)? as u32,
            ))
        })?
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .next();
    let Some((min_level, max_level, region_id)) = row else {
        return Ok(None);
    };
    let mut region = Region::new(region_id, editor_id, space.clone(), min_level, max_level);
    load_region_points(conn, &mut region, space)?;
    // Mirrors `region.GetAllSubRegions().First().GetCentroid()`.
    let first = region
        .all_sub_regions()
        .into_iter()
        .next()
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let center = centroid(&first.iter().map(|p| p.coord()).collect::<Vec<_>>());
    Ok(Some(Instance::new(
        region.entity.clone(),
        space.clone(),
        center,
        0,
        "",
        None,
        LockLevel::None,
        None,
        1.0,
        false,
    )))
}

/// Mirrors C# `GetNonNukableRegions(space)`.
fn non_nukable_regions(conn: &Connection, space: &Space) -> Result<Vec<Region>> {
    let query = format!(
        "SELECT minLevel, maxLevel, regionFormID, regionEditorId FROM Region WHERE nukable = 0 AND spaceFormID = {}",
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)? as u32,
            r.get::<_, i64>(1)? as u32,
            r.get::<_, i64>(2)? as u32,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (min_level, max_level, region_id, editor_id) = r?;
        let mut region = Region::new(region_id, editor_id, space.clone(), min_level, max_level);
        load_region_points(conn, &mut region, space)?;
        out.push(region);
    }
    Ok(out)
}

/// Mirrors C# `GetSingularInstance`.
fn singular_instance(
    conn: &Connection,
    result: &crate::search::Grouped,
    instance_form_id: u32,
    space: &Space,
    spaces: &[Space],
) -> Result<Vec<Instance>> {
    let query = format!(
        "SELECT x, y, z, teleportsToFormID, primitiveShape, boundX, boundY, boundZ, rotZ, spaceFormID FROM Position WHERE instanceFormID = {instance_form_id} AND spaceFormID = {}",
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let mut rows = s.query([])?;
    if let Some(row) = rows.next()? {
        Ok(vec![Instance::new(
            result.entity.clone(),
            space.clone(),
            coord_of(row)?,
            instance_form_id,
            result.label.clone(),
            teleports_to(spaces, row.get::<_, Option<i64>>("teleportsToFormID")?)?,
            result.lock_level,
            shape_of(row)?,
            1.0,
            false,
        )])
    } else {
        Ok(Vec::new())
    }
}

/// Mirrors C# `GetInContainerInstances`.
fn in_container_instances(
    conn: &Connection,
    result: &crate::search::Grouped,
    space: &Space,
) -> Result<Vec<Instance>> {
    let query = format!(
        "SELECT x, y, z, instanceFormID FROM Position JOIN Container ON Container.containerFormID = Position.referenceFormID WHERE contentFormID = {} AND quantity = ? AND lockLevel = '{}' AND spaceFormID = {}",
        result.entity.form_id,
        result.lock_level.to_query_string(),
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([result.spawn_weight], |row| {
        Ok((
            row.get::<_, Option<f64>>("x")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("y")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("z")?.unwrap_or(0.0),
            row.get::<_, i64>("instanceFormID")? as u32,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (x, y, z, id) = r?;
        out.push(Instance::new(
            result.entity.clone(),
            space.clone(),
            Coord::new_xyz(x, y, z),
            id,
            result.label.clone(),
            None,
            result.lock_level,
            None,
            result.spawn_weight,
            true,
        ));
    }
    Ok(out)
}

/// Mirrors C# `GetLocationInstance` (single instance at the cells' centroid).
fn location_instances(
    conn: &Connection,
    result: &crate::search::Grouped,
    space: &Space,
) -> Result<Vec<Instance>> {
    let query = format!(
        "SELECT x, y FROM Cell WHERE locationFormId = '{}' AND spaceFormID = {}",
        result.entity.form_id,
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([], |row| {
        Ok((
            row.get::<_, i64>("x")? as i32,
            row.get::<_, i64>("y")? as i32,
        ))
    })?;
    let mut location = Location::new(
        result.entity.form_id,
        result.entity.editor_id.clone(),
        result.entity.display_name.clone(),
        space.clone(),
    );
    for r in rows {
        let (x, y) = r?;
        let cell = Cell::new(location.entity.clone(), space.clone(), x, y);
        location.add_cell(cell);
    }
    let coords: Vec<Coord> = location.cells.iter().map(|c| c.coord()).collect();
    let center = if coords.is_empty() {
        Coord::new(0.0, 0.0)
    } else {
        centroid(&coords)
    };
    Ok(vec![Instance::new(
        location.entity.clone(),
        space.clone(),
        center,
        0,
        "",
        None,
        LockLevel::None,
        None,
        1.0,
        false,
    )])
}

/// Mirrors C# `GetStandardInstances`.
fn standard_instances(
    conn: &Connection,
    result: &crate::search::Grouped,
    space: &Space,
    spaces: &[Space],
) -> Result<Vec<Instance>> {
    let query = format!(
        "SELECT x, y, z, instanceFormID, teleportsToFormID, primitiveShape, boundX, boundY, boundZ, rotZ FROM Position WHERE referenceFormID = {} AND spaceFormID = {} AND lockLevel = '{}' AND label = '{}'",
        result.entity.form_id,
        space.form_id(),
        result.lock_level.to_query_string(),
        esc(&result.label),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([], |row| {
        Ok((
            coord_of(row)?,
            row.get::<_, i64>("instanceFormID")? as u32,
            row.get::<_, Option<i64>>("teleportsToFormID")?,
            shape_of(row)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (coord, id, teleports_raw, shape) = r?;
        out.push(Instance::new(
            result.entity.clone(),
            space.clone(),
            coord,
            id,
            result.label.clone(),
            teleports_to(spaces, teleports_raw)?,
            result.lock_level,
            shape,
            1.0,
            false,
        ));
    }
    Ok(out)
}

/// Mirrors C# `GetScrapInstances`.
fn scrap_instances(
    conn: &Connection,
    result: &crate::search::Grouped,
    space: &Space,
) -> Result<Vec<Instance>> {
    let query = format!(
        "SELECT x, y, z, Position.instanceFormID FROM Position JOIN Scrap ON Scrap.junkFormID = Position.referenceFormID WHERE component = '{}' AND Scrap.componentQuantity = ? AND Position.spaceFormID = {}",
        esc(&result.entity.editor_id),
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([result.spawn_weight], |row| {
        Ok((
            row.get::<_, Option<f64>>("x")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("y")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("z")?.unwrap_or(0.0),
            row.get::<_, i64>("instanceFormID")? as u32,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (x, y, z, id) = r?;
        out.push(Instance::new(
            result.entity.clone(),
            space.clone(),
            Coord::new_xyz(x, y, z),
            id,
            result.label.clone(),
            None,
            LockLevel::None,
            None,
            result.spawn_weight,
            false,
        ));
    }
    Ok(out)
}

/// Mirrors C# `GetNPCInstances`.
fn npc_instances(
    conn: &Connection,
    result: &crate::search::Grouped,
    space: &Space,
) -> Result<Vec<Instance>> {
    let query = format!(
        "SELECT x, y, z, Position.instanceFormID FROM Position JOIN NPC ON npc.instanceFormID = Position.instanceFormID WHERE npcName = '{}' AND npc.spawnWeight = ? AND npc.spaceFormID = {}",
        esc(&result.entity.editor_id),
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([result.spawn_weight], |row| {
        Ok((
            row.get::<_, Option<f64>>("x")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("y")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("z")?.unwrap_or(0.0),
            row.get::<_, i64>("instanceFormID")? as u32,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (x, y, z, id) = r?;
        out.push(Instance::new(
            result.entity.clone(),
            space.clone(),
            Coord::new_xyz(x, y, z),
            id,
            result.label.clone(),
            None,
            LockLevel::None,
            None,
            result.spawn_weight,
            false,
        ));
    }
    Ok(out)
}

/// Mirrors C# `GetFluxInstances`, including the non-nukable exclusion.
fn flux_instances(
    conn: &Connection,
    result: &crate::search::Grouped,
    space: &Space,
) -> Result<Vec<Instance>> {
    let color = match &result.entity.kind {
        EntityKind::DerivedFlux(color) => color.to_string(),
        _ => result.entity.editor_id.clone(),
    };
    let query = format!(
        "SELECT x, y, z, Position.instanceFormID FROM Position JOIN Flux ON Flux.referenceFormID = Position.referenceFormID WHERE Flux.Color = '{}' AND spaceFormID = {}",
        esc(&color),
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([], |row| {
        Ok((
            row.get::<_, Option<f64>>("x")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("y")?.unwrap_or(0.0),
            row.get::<_, Option<f64>>("z")?.unwrap_or(0.0),
            row.get::<_, i64>("instanceFormID")? as u32,
        ))
    })?;
    let mut instances = Vec::new();
    for r in rows {
        let (x, y, z, id) = r?;
        instances.push(Instance::new(
            result.entity.clone(),
            space.clone(),
            Coord::new_xyz(x, y, z),
            id,
            result.label.clone(),
            None,
            LockLevel::None,
            None,
            result.spawn_weight,
            false,
        ));
    }
    // Exclude instances lying within non-nukable regions.
    for region in non_nukable_regions(conn, space)? {
        instances.retain(|i| !region.contains_point(i.coord));
    }
    Ok(instances)
}

/// Mirrors C# `Database.GetInstances`: resolves every [`Instance`] of a
/// grouped (or singular) search result in the given space (defaults to the
/// result's own space when the caller passes it through).
pub fn instances(conn: &Connection, result: SearchResult, space: &Space) -> Result<Vec<Instance>> {
    let spaces = crate::db::all_spaces(conn)?;
    match &result {
        SearchResult::Singular {
            inner,
            instance_form_id,
        } => singular_instance(conn, inner, *instance_form_id, space, &spaces),
        SearchResult::Grouped(g) if g.in_container => in_container_instances(conn, g, space),
        SearchResult::Grouped(g) if matches!(g.entity.kind, EntityKind::Region) => {
            Ok(region_instance(conn, &g.entity.editor_id.clone(), space)?
                .into_iter()
                .collect())
        }
        SearchResult::Grouped(g) if matches!(g.entity.kind, EntityKind::Location) => {
            location_instances(conn, g, space)
        }
        SearchResult::Grouped(g) if matches!(g.entity.kind, EntityKind::Standard) => {
            standard_instances(conn, g, space, &spaces)
        }
        SearchResult::Grouped(g) if matches!(g.entity.kind, EntityKind::DerivedScrap) => {
            scrap_instances(conn, g, space)
        }
        SearchResult::Grouped(g) if matches!(g.entity.kind, EntityKind::DerivedNpc) => {
            npc_instances(conn, g, space)
        }
        SearchResult::Grouped(g) if matches!(g.entity.kind, EntityKind::DerivedFlux(_)) => {
            flux_instances(conn, g, space)
        }
        // Unreachable: `EntityKind` is exhaustive above; kept total so a
        // future variant fails loudly instead of silently resolving nothing.
        SearchResult::Grouped(_) => Ok(Vec::new()),
    }
}

/// Mirrors C# `Database.GetTeleporters`: every instance in the space whose
/// teleporter target is another space.
pub fn teleporters(conn: &Connection, space: &Space) -> Result<Vec<Instance>> {
    let spaces = crate::db::all_spaces(conn)?;
    let query = format!(
        "SELECT x, y, z, instanceFormID, teleportsToFormID, primitiveShape, boundX, boundY, boundZ, rotZ, referenceFormID, editorID, displayName, signature FROM Position JOIN Entity ON Entity.entityFormID = Position.referenceFormID WHERE spaceFormID = {} AND teleportsToFormID IS NOT NULL AND teleportsToFormID != {}",
        space.form_id(),
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([], |row| {
        Ok((
            coord_of(row)?,
            row.get::<_, i64>("instanceFormID")? as u32,
            row.get::<_, Option<i64>>("teleportsToFormID")?,
            shape_of(row)?,
            row.get::<_, i64>("referenceFormID")? as u32,
            row.get::<_, String>("editorID")?,
            row.get::<_, Option<String>>("displayName")?
                .unwrap_or_default(),
            row.get::<_, String>("signature")?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (coord, id, teleports_raw, shape, ref_id, editor_id, display_name, sig_raw) = r?;
        out.push(Instance::new(
            Entity::new(
                ref_id,
                editor_id,
                display_name,
                parse_signature(sig_raw)?,
                EntityKind::Standard,
            ),
            space.clone(),
            coord,
            id,
            "",
            teleports_to(&spaces, teleports_raw)?,
            LockLevel::None,
            shape,
            1.0,
            false,
        ));
    }
    Ok(out)
}

/// Mirrors C# `Database.GetInfestationLocations`: every infestation-enabled
/// location in the space, with its cells populated.
pub fn infestation_locations(conn: &Connection, space: &Space) -> Result<Vec<Location>> {
    let query = format!(
        "SELECT locationFormId, locationEditorId, locationDisplayName FROM Location WHERE infestation = 1 AND spaceFormID = {}",
        space.form_id(),
    );
    let mut s = conn.prepare(&query)?;
    let rows = s.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (form_id, editor_id, display_name) = r?;
        let mut location = Location::new(form_id, editor_id, display_name, space.clone());
        let cell_query = format!("SELECT x, y FROM Cell WHERE locationFormId = {form_id}");
        let mut cs = conn.prepare(&cell_query)?;
        let cells = cs.query_map([], |row| {
            Ok((row.get::<_, i64>(0)? as i32, row.get::<_, i64>(1)? as i32))
        })?;
        for c in cells {
            let (x, y) = c?;
            let cell = Cell::new(location.entity.clone(), space.clone(), x, y);
            location.add_cell(cell);
        }
        out.push(location);
    }
    Ok(out)
}

/// Mirrors C# `Database.GetZRange`: the min/max `height_for_topograph` over
/// every resolved instance of the items, or `(0, 0)` when empty.
pub fn z_range(conn: &Connection, items: &[SearchResult], space: &Space) -> Result<(f64, f64)> {
    if items.is_empty() {
        return Ok((0.0, 0.0));
    }
    let mut all: Vec<Instance> = Vec::new();
    for item in items {
        all.extend(instances(conn, item.clone(), space)?);
    }
    if all.is_empty() {
        return Ok((0.0, 0.0));
    }
    let min = all
        .iter()
        .map(Instance::height_for_topograph)
        .fold(f64::INFINITY, f64::min);
    let max = all
        .iter()
        .map(Instance::height_for_topograph)
        .fold(f64::NEG_INFINITY, f64::max);
    Ok((min, max))
}
