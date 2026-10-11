//! Readonly SQLite access ported from the C# reference (READ-ONLY).
//!
//! Sources:
//! - `Library/CommonDatabase.cs` — `GetNewConnection` (readonly mode),
//!   `GetGameVersion`, `GetSpaces`, `GetAllSpaces`, `GetMapMarkers`.
//! - `Mappalachia/Database.cs` — `GetMapMarkersForSpace`
//!   (`SELECT * FROM MapMarker WHERE SpaceFormID = … ORDER BY Y`),
//!   `GetBlastRadius` (`EN07_NukeBlastRadius`), `GetInfestationRadius`
//!   (`HTO_LCP_HostileTakeOver_TargetRadius_Location`), `GetGameVersion`.

use rusqlite::{Connection, Result};

use crate::model::{Coord, MapMarker, Space};

/// Mirrors C# `CommonDatabase.GetNewConnection(path)` (readonly).
pub fn open_readonly(path: &str) -> Result<Connection> {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// Mirrors C# `CommonDatabase.GetGameVersion`
/// (`SELECT value FROM Meta WHERE Key = 'GameVersion'`).
pub fn game_version(c: &Connection) -> Result<String> {
    c.query_row(
        "SELECT value FROM Meta WHERE key = 'GameVersion'",
        [],
        |r| r.get(0),
    )
}

/// Mirrors C# `Database.Initialize`
/// (`SELECT * FROM Space ORDER BY isWorldspace DESC, spaceDisplayName ASC`),
/// returning fully-populated [`Space`] values (cf. C# `PopulateFromData`).
pub fn all_spaces(c: &Connection) -> Result<Vec<Space>> {
    let mut s = c.prepare(
        "SELECT spaceFormID, spaceEditorID, spaceDisplayName, isWorldspace, isInstanceable, centerX, centerY, maxRange, northAngle FROM Space ORDER BY isWorldspace DESC, spaceDisplayName ASC",
    )?;
    let rows = s.query_map([], |r| {
        Ok(Space::new(
            r.get::<_, i64>(0)? as u32,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, i64>(3)? != 0,
            r.get::<_, i64>(4)? != 0,
            r.get::<_, f64>(5)?,
            r.get::<_, f64>(6)?,
            r.get::<_, f64>(7)?,
            r.get::<_, f64>(8)?,
        ))
    })?;
    rows.collect()
}

/// Mirrors C# `Database.GetMapMarkersForSpace`
/// (`SELECT * FROM MapMarker WHERE SpaceFormID = … ORDER BY Y`).
pub fn map_markers_for_space(c: &Connection, space_form_id: u32) -> Result<Vec<MapMarker>> {
    let mut s =
        c.prepare("SELECT x, y, label, icon FROM MapMarker WHERE spaceFormID = ? ORDER BY y")?;
    let rows = s.query_map([space_form_id], |r| {
        Ok(MapMarker::new(
            r.get::<_, String>(3)?,
            r.get::<_, String>(2)?,
            space_form_id,
            Coord::new(r.get::<_, f64>(0)?, r.get::<_, f64>(1)?),
        ))
    })?;
    rows.collect()
}

fn global(c: &Connection, key: &str) -> Result<f64> {
    c.query_row("SELECT value FROM Global WHERE key = ?", [key], |r| {
        r.get(0)
    })
}

/// Mirrors C# `Database.GetBlastRadius` (`EN07_NukeBlastRadius`).
pub fn blast_radius(c: &Connection) -> Result<f64> {
    global(c, "EN07_NukeBlastRadius")
}

/// Mirrors C# `Database.PopulateFromData` (`Database.cs:858-861`): resolve
/// a (typically recipe-deserialized, `max_range == 0`) space against the
/// database, matching by entity equality (`form_id + editor_id`), and error
/// (`QueryReturnedNoRows`) when no row matches — mirroring the C# throw.
pub fn populate_from_data(conn: &Connection, space: &Space) -> Result<Space> {
    let spaces = all_spaces(conn)?;
    spaces
        .into_iter()
        .find(|s| *s == *space)
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// Mirrors C# `Database.GetInfestationRadius`
/// (`HTO_LCP_HostileTakeOver_TargetRadius_Location`).
pub fn infestation_radius(c: &Connection) -> Result<f64> {
    global(c, "HTO_LCP_HostileTakeOver_TargetRadius_Location")
}
