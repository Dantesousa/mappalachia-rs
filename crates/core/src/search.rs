//! The 9 search queries ported verbatim from the C# reference (READ-ONLY).
//!
//! Sources (`Mappalachia/Database.cs` lines 32-375, plus helpers):
//! - `Search` — fan-out over the 9 fns + `SearchInInstancesOnly` filter
//! - `StandardSearch`, `ContainerSearch`, `ScrapSearch`, `NPCSearch`,
//!   `FluxSearch`, `RegionSearch`, `LocationSearch`, `TeleportsToSearch`,
//!   `InstanceSearch` — SQL copied verbatim (only the already-escaped
//!   `searchTerm` / `spaceClause` / `IN` lists are interpolated, as in C#)
//! - `ProcessSearchString` (line 869), `ToStringForQuery` (line 878)
//! - `SearchSettings.ShouldSearchFor*` (`Settings/SearchSettings.cs`)
//! - `ReaderExtensions.GetLockLevel/GetSignature/GetFluxColor`
//!   (`Library/ReaderExtensions.cs`)
//! - `CommonDatabase.ToSqliteCollection` (`Library/CommonDatabase.cs`)
//!
//! # Porting notes
//! - C# `GroupedSearchResult` / `SingularSearchResult` become
//!   [`SearchResult::Grouped`] / [`SearchResult::Singular`]; the flat
//!   [`Grouped`] struct holds what C# spreads across `BaseInstance` +
//!   `GroupedSearchResult` (`entity`, `space`, `count`, `spawnWeight`,
//!   `label`, `lockLevel`, `inContainer`).
//! - `Region`/`Location` entities map to [`EntityKind::Region`] /
//!   [`EntityKind::Location`] with the C# hardcoded fields (`Region`:
//!   empty display name, `REGN`; `Location`: `LCTN`).
//! - `Region.spaceFormID` is TEXT in the DB (decimal int as string), so it
//!   is read as `String` and parsed to `u32`.
//! - NULL/empty `lockLevel` maps to [`LockLevel::None`]; NULL
//!   `displayName`/`label` map to `""`.
//! - The substitute-word regexes (`derived|scrap` etc., case-insensitive)
//!   are reimplemented without the `regex` crate (plain ASCII alternations;
//!   left-to-right alternation-order scan mirrors `Regex.Replace`).

use std::str::FromStr;

use rusqlite::{Connection, Result};

use crate::common::{hex_to_int, is_hex_form_id};
use crate::enums::{FluxColor, LockLevel, Signature};
use crate::model::{Entity, EntityKind, Space};

/// Mirrors the C# `SearchSettings` inputs consumed by `Database.Search`.
#[derive(Debug, Clone)]
pub struct SearchParams {
    pub search_term: String,
    pub space_form_id: Option<u32>,
    pub advanced: bool,
    pub search_in_instances_only: bool,
    pub signatures: Vec<Signature>,
    pub lock_levels: Vec<LockLevel>,
}

/// Mirrors C# `GroupedSearchResult` (via `BaseInstance`).
#[derive(Debug, Clone)]
pub struct Grouped {
    pub entity: Entity,
    pub space: Space,
    pub count: i64,
    pub spawn_weight: f64,
    pub label: String,
    pub lock_level: LockLevel,
    pub in_container: bool,
    /// Mirrors C# `GroupedSearchResult.LegendGroup` (default 0).
    pub legend_group: i32,
    /// Mirrors C# `GroupedSearchResult.LegendText`: `None` when the recipe
    /// carries no stored text (minimal hand-written recipes); callers fall
    /// back to [`Grouped::legend_text`].
    pub legend_text: Option<String>,
    /// Mirrors C# `GroupedSearchResult.PlotIcon` (serialized without the
    /// image): `None` when the recipe carries no stored icon; callers
    /// fresh-assign via the `FormMain` math.
    pub plot_icon: Option<StoredPlotIcon>,
}

/// The serializable half of C# `PlotIcon` (`PlotIcon.cs`): every field is
/// optional so minimal hand-written recipes (no `PlotIcon` object, or a
/// partial one) still parse. Missing fields resolve via the `FormMain`
/// fresh-assignment math at draw time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPlotIcon {
    pub do_not_use_icon: Option<bool>,
    pub base_icon_index: Option<usize>,
    pub size: Option<u32>,
    /// RGB triple (`#rrggbb` on the wire, cf. `JsonConverterColor`).
    pub color: Option<(u8, u8, u8)>,
}

/// Mirrors C# `GroupedSearchResult` / `SingularSearchResult`.
#[derive(Debug, Clone)]
pub enum SearchResult {
    Grouped(Grouped),
    Singular {
        inner: Grouped,
        instance_form_id: u32,
    },
}

impl SearchResult {
    pub fn entity_kind(&self) -> &EntityKind {
        match self {
            SearchResult::Grouped(g) => &g.entity.kind,
            SearchResult::Singular { inner, .. } => &inner.entity.kind,
        }
    }

    /// Mirrors C# `GroupedSearchResult.Count` (always 1 for singular results).
    pub fn count(&self) -> i64 {
        match self {
            SearchResult::Grouped(g) => g.count,
            SearchResult::Singular { .. } => 1,
        }
    }

    pub fn form_id(&self) -> u32 {
        match self {
            SearchResult::Grouped(g) => g.entity.form_id,
            SearchResult::Singular { inner, .. } => inner.entity.form_id,
        }
    }

    pub fn editor_id(&self) -> &str {
        match self {
            SearchResult::Grouped(g) => &g.entity.editor_id,
            SearchResult::Singular { inner, .. } => &inner.entity.editor_id,
        }
    }

    pub fn space(&self) -> &Space {
        match self {
            SearchResult::Grouped(g) => &g.space,
            SearchResult::Singular { inner, .. } => &inner.space,
        }
    }
}

/// Mirrors C# `ProcessSearchString`: trim, escape `'`, prefix `_`/`%`
/// with the escape char, wildcard on space.
pub fn process_search_string(input: &str) -> String {
    input
        .trim()
        .replace('\'', "''")
        .replace('_', "`_")
        .replace('%', "`%")
        .replace(' ', "%")
}

/// Mirrors `SelectedLockLevels.Select(l => l.ToStringForQuery()).ToSqliteCollection()`.
fn lock_list(levels: &[LockLevel]) -> String {
    format!(
        "({})",
        levels
            .iter()
            .map(|l| format!("'{}'", l.to_query_string()))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Mirrors `SelectedSignatures.ToSqliteCollection()`.
fn sig_list(sigs: &[Signature]) -> String {
    format!(
        "({})",
        sigs.iter()
            .map(|s| format!("'{}'", s.as_csharp_name()))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Mirrors `ReaderExtensions.GetLockLevel`: NULL/empty → `None`, else
/// `Enum.Parse` after stripping whitespace.
fn parse_lock_level(raw: Option<String>) -> Result<LockLevel> {
    match raw {
        None => Ok(LockLevel::None),
        Some(s) if s.is_empty() => Ok(LockLevel::None),
        Some(s) => {
            let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            LockLevel::from_str(&stripped).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
            })
        }
    }
}

fn parse_signature(raw: String) -> Result<Signature> {
    Signature::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

fn parse_flux_color(raw: String) -> Result<FluxColor> {
    FluxColor::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

/// Mirrors `SubstituteScrap/SubstituteNPC/SubstituteFlux.Replace(term, "").Trim()`.
/// The C# patterns are plain case-insensitive ASCII alternations; this
/// left-to-right, alternation-order scan reproduces `Regex.Replace`.
fn strip_words(term: &str, words: &[&str]) -> String {
    let mut out = String::with_capacity(term.len());
    let mut i = 0;
    while i < term.len() {
        let mut skip = 0;
        for w in words {
            if term.len() - i >= w.len()
                && term
                    .get(i..i + w.len())
                    .map(|s| s.eq_ignore_ascii_case(w))
                    .unwrap_or(false)
            {
                skip = w.len();
                break;
            }
        }
        if skip > 0 {
            i += skip;
        } else {
            let ch = term[i..].chars().next().expect("char boundary");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out.trim().to_string()
}

/// Mirrors C# `GetSpaceByFormID` (throws when missing).
fn space_by_id(spaces: &[Space], form_id: u32) -> Result<Space> {
    spaces
        .iter()
        .find(|s| s.form_id() == form_id)
        .cloned()
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn standard_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    search_for_form_id: bool,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let form_id_or = if search_for_form_id {
        format!("OR referenceFormID = '{}'", hex_to_int(term))
    } else {
        String::new()
    };
    let query = format!(
        "SELECT referenceFormID, editorID, displayName, signature, spaceFormID, count, label, lockLevel \
        FROM Position_PreGrouped \
        JOIN Entity ON Entity.entityFormID = Position_PreGrouped.referenceFormID \
        WHERE (label LIKE '%{term}%' ESCAPE '`' \
        OR editorID LIKE '%{term}%' ESCAPE '`' \
        OR displayName LIKE '%{term}%' ESCAPE '`' \
        {form_id_or}) \
        {space_clause}\
        AND Position_PreGrouped.lockLevel IN {} \
        AND Entity.signature IN {};",
        lock_list(&params.lock_levels),
        sig_list(&params.signatures)
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)? as u32,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (form_id, editor_id, display_name, sig_raw, space_id, count, label, lock_raw) = r?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::new(
                form_id,
                editor_id,
                display_name,
                parse_signature(sig_raw)?,
                EntityKind::Standard,
            ),
            space: space_by_id(spaces, space_id)?,
            count,
            spawn_weight: 1.0,
            label,
            lock_level: parse_lock_level(lock_raw)?,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn container_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    search_for_form_id: bool,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let form_id_or = if search_for_form_id {
        format!("OR contentFormID = '{}'", hex_to_int(term))
    } else {
        String::new()
    };
    let query = format!(
        "SELECT contentFormID, editorID, displayName, signature, spaceFormID, SUM(count) as count, lockLevel, quantity \
        FROM Container \
        JOIN Entity ON Container.contentFormID = Entity.entityFormID \
        JOIN Position_PreGrouped ON Position_PreGrouped.referenceFormID = Container.containerFormID \
        WHERE (label LIKE '%{term}%' ESCAPE '`' OR editorID LIKE '%{term}%' ESCAPE '`' OR displayName LIKE '%{term}%' ESCAPE '`' {form_id_or}) \
        {space_clause}\
        AND Position_PreGrouped.lockLevel IN {} \
        AND Entity.signature IN {} \
        GROUP BY editorID, contentFormID, lockLevel, quantity, spaceFormID;",
        lock_list(&params.lock_levels),
        sig_list(&params.signatures)
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)? as u32,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, i64>(7)? as f64,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (form_id, editor_id, display_name, sig_raw, space_id, count, lock_raw, quantity) = r?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::new(
                form_id,
                editor_id,
                display_name,
                parse_signature(sig_raw)?,
                EntityKind::Standard,
            ),
            space: space_by_id(spaces, space_id)?,
            count,
            spawn_weight: quantity,
            label: String::new(),
            lock_level: parse_lock_level(lock_raw)?,
            in_container: true,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn scrap_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    // Mirrors ShouldSearchForScrap: MISC + None.
    if !(params.signatures.contains(&Signature::Misc)
        && params.lock_levels.contains(&LockLevel::None))
    {
        return Ok(out);
    }
    let substitute = strip_words(term, &["derived", "scrap"]);
    let query = format!(
        "SELECT component, spaceFormID, componentQuantity, SUM(count) AS properCount \
        FROM Scrap \
        JOIN Position_PreGrouped ON Position_PreGrouped.referenceFormID = Scrap.junkFormID \
        WHERE (component LIKE '%{term}%' ESCAPE '`' \
        OR component LIKE '%{substitute}%' ESCAPE '`') \
        {space_clause}\
        GROUP BY component, spaceFormID, componentQuantity;"
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u32,
            row.get::<_, f64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (component, space_id, component_quantity, proper_count) = r?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::derived_scrap(component),
            space: space_by_id(spaces, space_id)?,
            count: proper_count,
            spawn_weight: component_quantity,
            label: String::new(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn npc_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    // Mirrors ShouldSearchForNPC: NPC_ + None.
    if !(params.signatures.contains(&Signature::Npc)
        && params.lock_levels.contains(&LockLevel::None))
    {
        return Ok(out);
    }
    let substitute = strip_words(term, &["derived", "spawn"]);
    let query = format!(
        "SELECT npcName, spaceFormID, spawnWeight, count(*) AS count \
        FROM NPC \
        WHERE (npcName LIKE '%{term}%' ESCAPE '`' \
        OR npcName LIKE '%{substitute}%' ESCAPE '`') \
        {space_clause}\
        GROUP BY NPC.spaceFormID, npcName, spawnWeight;"
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u32,
            row.get::<_, f64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (npc_name, space_id, spawn_weight, count) = r?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::derived_npc(npc_name),
            space: space_by_id(spaces, space_id)?,
            count,
            spawn_weight,
            label: String::new(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn flux_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    // Mirrors ShouldSearchForRawFlux: ALCH + None.
    if !(params.signatures.contains(&Signature::Alch)
        && params.lock_levels.contains(&LockLevel::None))
    {
        return Ok(out);
    }
    let substitute = strip_words(term, &["derived", "raw", "flux"]);
    let query = format!(
        "SELECT color, spaceFormID, SUM(count) AS properCount \
        FROM Flux \
        JOIN Position_PreGrouped ON Position_PreGrouped.referenceFormID = Flux.referenceFormID \
        WHERE (color LIKE '%{term}%' ESCAPE '`' \
        OR color LIKE '%{substitute}%' ESCAPE '`') \
        {space_clause}\
        GROUP BY spaceFormID, color;"
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u32,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for r in rows {
        let (color_raw, space_id, proper_count) = r?;
        let space = space_by_id(spaces, space_id)?;
        // Mirrors `results.Where(r => r.Space.IsNukable())`.
        if !space.is_nukable() {
            continue;
        }
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::derived_flux(parse_flux_color(color_raw)?),
            space,
            count: proper_count,
            spawn_weight: 1.0,
            label: String::new(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn region_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    search_for_form_id: bool,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    // Mirrors ShouldSearchForRegion: REGN + None.
    if !(params.signatures.contains(&Signature::Region)
        && params.lock_levels.contains(&LockLevel::None))
    {
        return Ok(out);
    }
    let form_id_or = if search_for_form_id {
        format!("OR regionFormID = '{}'", hex_to_int(term))
    } else {
        String::new()
    };
    let query = format!(
        "SELECT regionFormID, regionEditorID, spaceFormID \
        FROM Region \
        WHERE (regionEditorID LIKE '%{term}%' ESCAPE '`' \
        OR regionFormID = '{term}' \
        {form_id_or}) \
        {space_clause}\
        GROUP BY regionFormID;"
    );
    let mut stmt = conn.prepare(&query)?;
    // NOTE: Region.spaceFormID is TEXT — read as String, parse to u32.
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for r in rows {
        let (region_id, region_editor_id, space_raw) = r?;
        let space_id: u32 = space_raw.parse::<u32>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
        })?;
        let space = space_by_id(spaces, space_id)?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::new(
                region_id,
                region_editor_id,
                String::new(),
                Signature::Region,
                EntityKind::Region,
            ),
            space,
            count: 1,
            spawn_weight: 1.0,
            label: String::new(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn location_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    search_for_form_id: bool,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    // Mirrors ShouldSearchForLocation: LCTN + None.
    if !(params.signatures.contains(&Signature::Location)
        && params.lock_levels.contains(&LockLevel::None))
    {
        return Ok(out);
    }
    let form_id_or = if search_for_form_id {
        format!("OR locationFormID = '{}'", hex_to_int(term))
    } else {
        String::new()
    };
    let query = format!(
        "SELECT locationFormID, locationEditorID, locationDisplayName, spaceFormID \
        FROM Location \
        WHERE (locationEditorID LIKE '%{term}%' ESCAPE '`' \
        OR locationDisplayName LIKE '%{term}%' ESCAPE '`' \
        OR locationFormID = '{term}' \
        {form_id_or}) \
        {space_clause}\
        GROUP BY locationFormID;"
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)? as u32,
        ))
    })?;
    for r in rows {
        let (location_id, editor_id, display_name, space_id) = r?;
        let space = space_by_id(spaces, space_id)?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::new(
                location_id,
                editor_id,
                display_name,
                Signature::Location,
                EntityKind::Location,
            ),
            space,
            count: 1,
            spawn_weight: 1.0,
            label: String::new(),
            lock_level: LockLevel::None,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn teleports_to_search(
    conn: &Connection,
    params: &SearchParams,
    term: &str,
    space_clause: &str,
    search_for_form_id: bool,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    if !search_for_form_id {
        return Ok(out);
    }
    let query = format!(
        "SELECT referenceFormID, editorID, displayName, signature, spaceFormID, label, lockLevel, count(*) AS count \
        FROM Position \
        JOIN Entity ON Entity.entityFormID = Position.referenceFormID \
        WHERE ((teleportsToFormID = '{}') \
        {space_clause}\
        AND lockLevel IN {} \
        AND Entity.signature IN {}) \
        GROUP BY spaceFormID, Position.referenceFormID, teleportsToFormID, lockLevel, label;",
        hex_to_int(term),
        lock_list(&params.lock_levels),
        sig_list(&params.signatures)
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)? as u32,
            row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            row.get::<_, Option<String>>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;
    for r in rows {
        let (form_id, editor_id, display_name, sig_raw, space_id, label, lock_raw, count) = r?;
        out.push(SearchResult::Grouped(Grouped {
            entity: Entity::new(
                form_id,
                editor_id,
                display_name,
                parse_signature(sig_raw)?,
                EntityKind::Standard,
            ),
            space: space_by_id(spaces, space_id)?,
            count,
            spawn_weight: 1.0,
            label,
            lock_level: parse_lock_level(lock_raw)?,
            in_container: false,
            legend_group: 0,
            legend_text: None,
            plot_icon: None,
        }));
    }
    Ok(out)
}

fn instance_search(
    conn: &Connection,
    term: &str,
    search_for_form_id: bool,
    spaces: &[Space],
) -> Result<Vec<SearchResult>> {
    let mut out = Vec::new();
    if !search_for_form_id {
        return Ok(out);
    }
    let query = format!(
        "SELECT instanceFormID, referenceFormID, editorID, displayName, signature, spaceFormID, label, lockLevel \
        FROM Position \
        JOIN Entity ON Entity.entityFormID = Position.referenceFormID \
        WHERE instanceFormID = '{}';",
        hex_to_int(term)
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u32,
            row.get::<_, i64>(1)? as u32,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)? as u32,
            row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    for r in rows {
        let (instance_id, form_id, editor_id, display_name, sig_raw, space_id, label, lock_raw) =
            r?;
        // Mirrors C# `SingularSearchResult` (count 1, spawnWeight 1).
        out.push(SearchResult::Singular {
            inner: Grouped {
                entity: Entity::new(
                    form_id,
                    editor_id,
                    display_name,
                    parse_signature(sig_raw)?,
                    EntityKind::Standard,
                ),
                space: space_by_id(spaces, space_id)?,
                count: 1,
                spawn_weight: 1.0,
                label,
                lock_level: parse_lock_level(lock_raw)?,
                in_container: false,
                legend_group: 0,
                legend_text: None,
                plot_icon: None,
            },
            instance_form_id: instance_id,
        });
    }
    Ok(out)
}

/// Mirrors C# `Database.Search`: runs the 9 queries and applies the
/// `SearchInInstancesOnly` filter (Singular results or instanceable spaces).
pub fn search(conn: &Connection, params: &SearchParams) -> Result<Vec<SearchResult>> {
    let spaces = crate::db::all_spaces(conn)?;
    let term = process_search_string(&params.search_term);
    let search_for_form_id = params.advanced && is_hex_form_id(&term);
    let space_clause = match params.space_form_id {
        Some(id) => format!("AND spaceFormID = {id} "),
        None => String::new(),
    };
    let mut out: Vec<SearchResult> = Vec::new();
    out.extend(standard_search(
        conn,
        params,
        &term,
        &space_clause,
        search_for_form_id,
        &spaces,
    )?);
    out.extend(container_search(
        conn,
        params,
        &term,
        &space_clause,
        search_for_form_id,
        &spaces,
    )?);
    out.extend(scrap_search(conn, params, &term, &space_clause, &spaces)?);
    out.extend(npc_search(conn, params, &term, &space_clause, &spaces)?);
    out.extend(flux_search(conn, params, &term, &space_clause, &spaces)?);
    out.extend(region_search(
        conn,
        params,
        &term,
        &space_clause,
        search_for_form_id,
        &spaces,
    )?);
    out.extend(location_search(
        conn,
        params,
        &term,
        &space_clause,
        search_for_form_id,
        &spaces,
    )?);
    out.extend(teleports_to_search(
        conn,
        params,
        &term,
        &space_clause,
        search_for_form_id,
        &spaces,
    )?);
    out.extend(instance_search(conn, &term, search_for_form_id, &spaces)?);
    if params.search_in_instances_only {
        out.retain(|r| matches!(r, SearchResult::Singular { .. }) || r.space().is_instanceable);
    }
    Ok(out)
}
