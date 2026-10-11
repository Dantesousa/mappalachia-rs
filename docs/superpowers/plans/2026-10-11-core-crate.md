# Core Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `crates/core` — model, readonly DB search (same SQL as C#), geometry, recipe JSON — tested against the real release DB.

**Architecture:** Rust library crate, sync rusqlite (bundled), serde JSON shaped identically to `System.Text.Json` output (PascalCase keys, `#rrggbb` colors, `EntityChildType`/`SearchResultType` discriminators). No rendering, no GUI.

**Tech Stack:** rustc 1.99, edition 2021, rusqlite 0.32 (bundled), serde 1, serde_json 1.

## Global Constraints

- Workspace root is `/home/dante/Documents/mappalachia-rs/`, resolver 2.
- Real test DB at `/home/dante/Documents/mappalachia-data/data/mappalachia.db` — open read-only, never write, never copy into repo.
- Integration tests read env `MAPPALACHIA_DB`, skip with message when unset/missing.
- Same-SQL parity: SQL strings ported verbatim from `Mappalachia/Database.cs`; LIKE terms go through `process_search_string`.
- `Region.spaceFormID` is TEXT in SQLite — read as String, parse to u32.
- Frequent commits, one per task minimum.

---

### Task 1: Scaffold crate + constants + FormID helpers

**Files:**
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `crates/core/src/common.rs`
- Test: `crates/core/src/common.rs` (unit tests inline)

**Interfaces:**
- Consumes: nothing.
- Produces: `common::{MAP_IMAGE_RESOLUTION, CELL_SIZE, TILE_WIDTH, TILE_RADIUS, SPOTLIGHT_MAX_SIZE, SPOTLIGHT_MIN_TILE_RESOLUTION, SPOTLIGHT_TILE_SIZE, SPOTLIGHT_SCALE}`, `common::{is_hex_form_id, hex_to_int, to_hex_u32, without_whitespace}` used by Tasks 2–7.

- [ ] **Step 1: Write Cargo.toml + lib.rs + failing test**

```toml
# crates/core/Cargo.toml
[package]
name = "mappalachia-core"
version = "0.1.0"
edition = "2021"

[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
// crates/core/src/lib.rs
pub mod common;
```

```rust
// crates/core/src/common.rs (append at end)
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_form_id_detection() {
        assert!(is_hex_form_id("00094FC1"));
        assert!(is_hex_form_id("0x94fc1"));
        assert!(!is_hex_form_id("Nuka Cola"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mappalachia-core hex_form_id_detection`
Expected: FAIL with "cannot find function `is_hex_form_id`".

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/core/src/common.rs
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
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    !h.is_empty() && h.len() <= 8 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Mirrors Common.HexToInt: parses optional-0x hex, panics on invalid like Convert.ToUInt32.
pub fn hex_to_int(hex: &str) -> u32 {
    let h = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    u32::from_str_radix(h, 16).expect("invalid hex FormID")
}

/// Mirrors uint.ToString("X8").
pub fn to_hex_u32(id: u32) -> String {
    format!("{:08X}", id)
}

pub fn without_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mappalachia-core`
Expected: PASS (1 test), plus add assertions `hex_to_int("00094FC1") == 610241` and `to_hex_u32(610241) == "00094FC1"` to the test and re-run.

- [ ] **Step 5: Commit**

```bash
git add crates/core/Cargo.toml crates/core/src/lib.rs crates/core/src/common.rs
git commit -m "feat(core): scaffold crate with constants and FormID helpers"
```

---

### Task 2: Domain enums with parse/display/friendly names

**Files:**
- Create: `crates/core/src/enums.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod enums;`)
- Test: inline in `enums.rs`

**Interfaces:**
- Consumes: nothing from Task 1 (pure).
- Produces: `enums::{Signature, LockLevel, FluxColor, ShapeType}` with `parse`, `Display`/friendly names, `Signature::{is_recommended_selection, is_lockable, can_be_searched}`, `LockLevel::to_query_string` (None→"" else variant name) used by Tasks 3–7.

- [ ] **Step 1: Write the failing test**

```rust
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mappalachia-core signature_names_and_flags`
Expected: FAIL, `enums.rs` doesn't exist.

- [ ] **Step 3: Write minimal implementation**

Port `Library/Entity.cs` (Signature, 34 variants), `Library/BaseInstance.cs` (LockLevel, 11 variants), `Library/Common.cs` (FluxColor: Crimson, Cobalt, Fluorescent, Violet, Yellowcake), `Library/Shape.cs` (ShapeType: Box, Sphere, Line, Plane, Ellipsoid, Cylinder), and all `DataTranslator.cs` mappings verbatim. Rust variant names must be valid identifiers: `NPC_`→`Npc`, `LVLI`→`Lvli`, etc. `parse` accepts the exact C# enum names (`"NPC_"`, `"LVLI"`, `"RequiresTerminal"`…). LockLevel friendly names verbatim ("Level 0", "Requires Key", "Not Locked" for `to_friendly_name_with_context` on None). Signature friendly names + descriptions verbatim from DataTranslator (34 entries each).

```rust
// crates/core/src/enums.rs (shape; fill all 34 signatures + 11 lock levels)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Signature { Lvli, Flora, Npc, Misc, Book, Alch, Container, Stat, Scol, Mstt, Furn, Note, Term, Armo, Weapon, Ammo, Proj, Currency, Key, Region, Location, Acti, Tact, Trap, Door, Hazd, Idlm, Light, Sech, Aspc, Txst, Bnds, World, Cell }

impl std::str::FromStr for Signature {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "LVLI" => Self::Lvli, "FLOR" => Self::Flora, "NPC_" => Self::Npc,
            "MISC" => Self::Misc, "BOOK" => Self::Book, "ALCH" => Self::Alch,
            "CONT" => Self::Container, "STAT" => Self::Stat, "SCOL" => Self::Scol,
            "MSTT" => Self::Mstt, "FURN" => Self::Furn, "NOTE" => Self::Note,
            "TERM" => Self::Term, "ARMO" => Self::Armo, "WEAP" => Self::Weapon,
            "AMMO" => Self::Ammo, "PROJ" => Self::Proj, "CNCY" => Self::Currency,
            "KEYM" => Self::Key, "REGN" => Self::Region, "LCTN" => Self::Location,
            "ACTI" => Self::Acti, "TACT" => Self::Tact, "TRAP" => Self::Trap,
            "DOOR" => Self::Door, "HAZD" => Self::Hazd, "IDLM" => Self::Idlm,
            "LIGH" => Self::Light, "SECH" => Self::Sech, "ASPC" => Self::Aspc,
            "TXST" => Self::Txst, "BNDS" => Self::Bnds, "WRLD" => Self::World,
            "CELL" => Self::Cell, _ => return Err(format!("Unexpected signature {s}")),
        })
    }
}
// + as_csharp_name(), friendly_name(), description(), is_recommended_selection(),
//   is_lockable(), can_be_searched() per DataTranslator.cs verbatim
//   (~/Documents/Mappalachia/Mappalachia/DataTranslator.cs lines 36-175).
// + these two constructors (used by Task 5 tests):
impl Signature {
    /// Mirrors `Enum.GetValues<Signature>().Where(s => s.IsRecommendedSelection())`.
    pub fn recommended_default() -> Vec<Signature> {
        [Signature::Lvli, Signature::Flora, Signature::Npc, Signature::Misc, Signature::Book,
         Signature::Alch, Signature::Ammo, Signature::Armo, Signature::Currency, Signature::Container,
         Signature::Furn, Signature::Key, Signature::Region, Signature::Location, Signature::Term,
         Signature::Trap, Signature::Proj, Signature::Note, Signature::Weapon].to_vec()
    }
}
impl LockLevel {
    /// Mirrors `Enum.GetValues<LockLevel>().ToList()` — all 11 variants.
    pub fn all() -> Vec<LockLevel> {
        [LockLevel::None, LockLevel::Level0, LockLevel::Level1, LockLevel::Level2, LockLevel::Level3,
         LockLevel::RequiresTerminal, LockLevel::RequiresKey, LockLevel::Chained, LockLevel::Barred,
         LockLevel::Inaccessible, LockLevel::Unknown].to_vec()
    }
    /// Mirrors ToStringForQuery: None -> "", else variant name.
    pub fn to_query_string(&self) -> &'static str { todo!() }
}
```

LockLevel: same pattern (`"Level0"` parses to Level0 — note DB stores variant names; `GetLockLevel` strips whitespace then `Enum.Parse`, so `"Level 0"` in DB would fail in C# too; parse exact variant names, empty/NULL→None handled by caller).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mappalachia-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/enums.rs crates/core/src/lib.rs
git commit -m "feat(core): domain enums with C# verbatim mappings"
```

---

### Task 3: Model structs + geometry

**Files:**
- Create: `crates/core/src/model.rs`
- Create: `crates/core/src/geometry.rs`
- Modify: `crates/core/src/lib.rs` (add both modules)
- Test: inline unit tests in both files

**Interfaces:**
- Consumes: `common::{CELL_SIZE, TILE_WIDTH, TILE_RADIUS}`, `enums::*`.
- Produces: `model::{Coord, Shape, Entity, EntityKind, Space, Instance, Cluster, MapMarker, Region, RegionPoint, Location, Cell}` and `geometry::{lines_intersect, convex_hull, centroid}` used by Tasks 4–7.

- [ ] **Step 1: Write the failing test**

```rust
// geometry.rs tests
#[test]
fn hull_and_intersection() {
    use crate::model::Coord;
    let a = Coord { x: 0.0, y: 0.0, z: 0.0 };
    let b = Coord { x: 2.0, y: 2.0, z: 0.0 };
    let c = Coord { x: 0.0, y: 2.0, z: 0.0 };
    let d = Coord { x: 2.0, y: 0.0, z: 0.0 };
    assert!(lines_intersect(a, b, c, d));
    assert!(!lines_intersect(a, c, b, d));
    let pts = vec![a, b, c, d, Coord { x: 1.0, y: 1.0, z: 0.0 }];
    assert_eq!(convex_hull(&pts).len(), 4);
}
// model.rs tests
#[test]
fn region_contains_point() {
    // square (0,0)-(10,10) subregion 0 contains (5,5), not (15,5)
}
#[test]
fn cell_coord_math() {
    // Cell(Location, x=2, y=3) coord == (2*4096, 3*4096+4096)
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mappalachia-core hull_and_intersection`
Expected: FAIL, modules don't exist.

- [ ] **Step 3: Write minimal implementation**

Port `Library/Coord.cs`, `Shape.cs`, `Entity.cs` (+DerivedNPC/Scrap/RawFlux as `EntityKind`), `Space.cs` (incl. `min/max`, `is_appalachia`, `is_nukable`, `max_spotlight_benefit`, `is_suitable_for_spotlight`), `Instance.cs` (+`height_for_topograph`, `cluster: Option`), `Cluster.cs`, `MapMarker.cs`, `Region.cs` (+`contains_point` ray casting verbatim), `Location.cs`, `Cell.cs` (coord `(x*4096, y*4096+4096)`) + neighbors, `GeometryHelper.cs` (orientation+`lines_intersect`, monotone-chain `convex_hull` on (x,y), `pythagoras`, `centroid`), `SpotlightTile.cs` (`snap_to_tile_center`, `tiles_in_rect`, `contains_point`).

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EntityKind { Standard, DerivedNpc, DerivedScrap, DerivedFlux(FluxColor), Region, Location }

#[derive(Debug, Clone, PartialEq)]
pub struct Entity { pub form_id: u32, pub editor_id: String, pub display_name: String, pub signature: Signature, pub kind: EntityKind }
impl Entity {
    pub fn friendly_form_id(&self) -> String {
        match self.kind { EntityKind::Standard => crate::common::to_hex_u32(self.form_id), _ => "N/A".into() }
    }
}
```

Region deserialization note: `Region` min/max level are `#[serde(skip)]` in C# (JsonIgnore) — same in Rust.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mappalachia-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/model.rs crates/core/src/geometry.rs crates/core/src/lib.rs
git commit -m "feat(core): model structs and geometry port"
```

---

### Task 4: Readonly DB open + spaces + markers + globals

**Files:**
- Create: `crates/core/src/db.rs`
- Create: `crates/core/tests/db_parity.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod db;`)
- Modify: `crates/core/Cargo.toml` (add `[dev-dependencies]`? none needed)

**Interfaces:**
- Consumes: `model::{Space, MapMarker}`, rusqlite.
- Produces: `db::{open_readonly(path) -> rusqlite::Result<Connection>, game_version, all_spaces, map_markers_for_space, blast_radius, infestation_radius}` used by Tasks 5–6.

- [ ] **Step 1: Write the failing integration test**

```rust
// crates/core/tests/db_parity.rs
use mappalachia_core::db;
fn conn() -> rusqlite::Connection {
    let p = std::env::var("MAPPALACHIA_DB").expect("set MAPPALACHIA_DB to release mappalachia.db");
    db::open_readonly(&p).unwrap()
}
#[test]
fn meta_and_spaces() {
    let c = conn();
    assert_eq!(db::game_version(&c).unwrap(), "1.7.26.13");
    let spaces = db::all_spaces(&c).unwrap();
    assert_eq!(spaces.len(), 205);
    let app = spaces.iter().find(|s| s.editor_id == "APPALACHIA").unwrap();
    assert_eq!(app.form_id, 2480661);
}
#[test]
fn markers_and_globals() {
    let c = conn();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM MapMarker", [], |r| r.get::<_, i64>(0)).unwrap(), 458);
    assert_eq!(db::blast_radius(&c).unwrap(), 20460.0);
    assert_eq!(db::infestation_radius(&c).unwrap(), 20000.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `MAPPALACHIA_DB=/home/dante/Documents/mappalachia-data/data/mappalachia.db cargo test -p mappalachia-core --test db_parity`
Expected: FAIL, `db` module doesn't exist.

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/core/src/db.rs
use rusqlite::{Connection, Result};
use crate::model::{MapMarker, Space};
use crate::common as _; // (unused ok to drop)

pub fn open_readonly(path: &str) -> Result<Connection> {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
}
pub fn game_version(c: &Connection) -> Result<String> {
    c.query_row("SELECT value FROM Meta WHERE key = 'GameVersion'", [], |r| r.get(0))
}
pub fn all_spaces(c: &Connection) -> Result<Vec<Space>> {
    let mut s = c.prepare("SELECT spaceFormID, spaceEditorID, spaceDisplayName, isWorldspace, isInstanceable, centerX, centerY, maxRange, northAngle FROM Space ORDER BY isWorldspace DESC, spaceDisplayName ASC")?;
    let rows = s.query_map([], |r| Ok(Space {
        form_id: r.get::<_, i64>(0)? as u32, editor_id: r.get(1)?, display_name: r.get(2)?,
        is_worldspace: r.get::<_, i64>(3)? != 0, is_instanceable: r.get::<_, i64>(4)? != 0,
        center_x: r.get(5)?, center_y: r.get(6)?, max_range: r.get(7)?, north_angle: r.get(8)?,
    }))?;
    rows.collect()
}
// map_markers_for_space: SELECT x, y, label, icon FROM MapMarker WHERE spaceFormID = ? ORDER BY y
// blast_radius/infestation_radius: SELECT value FROM Global WHERE key = ?
```

Note: `isWorldspace` stored INTEGER 0/1 — read as i64, not bool (rusqlite bool read of INTEGER works actually, but i64 is explicit and safe). `displayName` may be NULL → use `r.get::<_, Option<String>>(2)?.unwrap_or_default()`.

- [ ] **Step 4: Run test to verify it passes**

Run: same command as Step 2. Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/db.rs crates/core/src/lib.rs crates/core/tests/db_parity.rs
git commit -m "feat(core): readonly DB open, spaces, markers, globals"
```

---

### Task 5: The 9 search queries

**Files:**
- Modify: `crates/core/src/db.rs` (add search module `pub mod search` or new file `search.rs` + `pub mod search;`)
- Create: `crates/core/src/search.rs`
- Modify: `crates/core/tests/db_parity.rs` (append search tests)

**Interfaces:**
- Consumes: `db::open_readonly`, `model::*`, `enums::*`, `common::{is_hex_form_id, hex_to_int}`.
- Produces: `search::{SearchParams, SearchResult::{Grouped, Singular}, search(conn, params) -> Vec<SearchResult>}` used by Tasks 6–7 and later crates.

- [ ] **Step 1: Write the failing tests (append to db_parity.rs)**

```rust
use mappalachia_core::{search::{SearchParams, search}, enums::Signature};
fn all_params(term: &str) -> SearchParams {
    SearchParams { search_term: term.into(), space_form_id: None, advanced: term == "00094FC1",
        search_in_instances_only: false, signatures: Signature::recommended_default(), lock_levels: LockLevel::all() }
}
#[test]
fn search_nuka_cola() {
    let c = conn();
    let r = search(&c, &all_params("Nuka Cola")).unwrap();
    assert_eq!(r.len(), 586); // standard rows for "Nuka%Cola", all spaces
}
#[test]
fn search_scrap_steel() {
    let c = conn();
    let r = search(&c, &all_params("Steel")).unwrap();
    let total: i64 = r.iter().filter(|x| matches!(x.entity_kind(), EntityKind::DerivedScrap)).map(|x| x.count()).sum();
    assert_eq!(total, 21795);
}
#[test]
fn search_npc_scorched() {
    let c = conn();
    let r = search(&c, &all_params("Scorched")).unwrap();
    let total: i64 = r.iter().filter(|x| matches!(x.entity_kind(), EntityKind::DerivedNpc)).map(|x| x.count()).sum();
    assert_eq!(total, 1731);
}
#[test]
fn search_flux_crimson() {
    let c = conn();
    let r = search(&c, &all_params("Crimson")).unwrap();
    let total: i64 = r.iter().filter(|x| matches!(x.entity_kind(), EntityKind::DerivedFlux(_))).map(|x| x.count()).sum();
    assert_eq!(total, 6497);
}
#[test]
fn search_form_id_instance() {
    let c = conn();
    let r = search(&c, &all_params("00094FC1")).unwrap(); // = entity 610241, advanced on
    assert!(r.iter().any(|x| matches!(x, SearchResult::Singular { .. }) || x.form_id() == 610241));
}
#[test]
fn search_region_and_location() {
    let c = conn();
    let r = search(&c, &all_params("Nuke")).unwrap();
    assert!(r.iter().any(|x| x.editor_id() == "NukeRegionA"));
    let r = search(&c, &all_params("Flatwoods")).unwrap();
    assert!(r.iter().any(|x| x.editor_id() == "LocForestFlatwoodsLocation"));
}
```

Expected counts measured 2026-10-11 against release DB (game 1.7.26.13): standard rows 586, scrap Steel 21795, NPC Scorched 1731, flux Crimson 6497.

- [ ] **Step 2: Run tests to verify they fail**

Run: `MAPPALACHIA_DB=... cargo test -p mappalachia-core --test db_parity`
Expected: FAIL, no `search` module.

- [ ] **Step 3: Write minimal implementation**

Port `Database.Search` + 9 fns verbatim (`~/Documents/Mappalachia/Mappalachia/Database.cs` lines 32-375). `process_search_string`: trim, `'`→`''`, `_`→`` `_ ``, `%`→`` `% ``, space→`%`. `ESCAPE '`'`. `search_for_form_id = is_hex && advanced`. Gating: scrap needs MISC+None; npc needs NPC_+None; flux needs ALCH+None; region needs REGN+None; location needs LCTN+None; teleportsTo+instance need `search_for_form_id`. Flux results filtered to nukable spaces (`is_appalachia`). `search_in_instances_only` filters to Singular or instanceable spaces.

```rust
// crates/core/src/search.rs
use rusqlite::Connection;
use crate::{enums::*, model::*};

#[derive(Debug, Clone)]
pub struct SearchParams {
    pub search_term: String, pub space_form_id: Option<u32>, pub advanced: bool,
    pub search_in_instances_only: bool, pub signatures: Vec<Signature>, pub lock_levels: Vec<LockLevel>,
}

#[derive(Debug, Clone)]
pub struct Grouped { pub entity: Entity, pub space: Space, pub count: i64, pub spawn_weight: f64, pub label: String, pub lock_level: LockLevel, pub in_container: bool }

#[derive(Debug, Clone)]
pub enum SearchResult { Grouped(Grouped), Singular { inner: Grouped, instance_form_id: u32 } }

impl SearchResult {
    pub fn entity_kind(&self) -> &EntityKind {
        match self { SearchResult::Grouped(g) => &g.entity.kind, SearchResult::Singular { inner, .. } => &inner.entity.kind }
    }
    pub fn count(&self) -> i64 {
        match self { SearchResult::Grouped(g) => g.count, SearchResult::Singular { .. } => 1 }
    }
    pub fn form_id(&self) -> u32 {
        match self { SearchResult::Grouped(g) => g.entity.form_id, SearchResult::Singular { inner, .. } => inner.entity.form_id }
    }
    pub fn editor_id(&self) -> &str {
        match self { SearchResult::Grouped(g) => &g.entity.editor_id, SearchResult::Singular { inner, .. } => &inner.entity.editor_id }
    }
    pub fn space(&self) -> &Space {
        match self { SearchResult::Grouped(g) => &g.space, SearchResult::Singular { inner, .. } => &inner.space }
    }
}

pub fn process_search_string(input: &str) -> String {
    input.trim().replace('\'', "''").replace('_', "`_").replace('%', "`%").replace(' ', "%")
}

fn lock_list(levels: &[LockLevel]) -> String {
    format!("({})", levels.iter().map(|l| format!("'{}'", l.to_query_string())).collect::<Vec<_>>().join(","))
}
fn sig_list(sigs: &[Signature]) -> String {
    format!("({})", sigs.iter().map(|s| format!("'{}'", s.as_csharp_name())).collect::<Vec<_>>().join(","))
}

pub fn search(conn: &Connection, params: &SearchParams) -> rusqlite::Result<Vec<SearchResult>> {
    let spaces = crate::db::all_spaces(conn)?;
    let by_id = |id: u32| spaces.iter().find(|s| s.form_id == id).cloned()
        .ok_or(rusqlite::Error::QueryReturnedNoRows);
    let term = process_search_string(&params.search_term);
    let for_id = params.advanced && crate::common::is_hex_form_id(&term);
    let space_clause = match params.space_form_id { Some(id) => format!("AND spaceFormID = {} ", id), None => String::new() };
    let mut out: Vec<SearchResult> = Vec::new();
    // 9 query fns, SQL copied verbatim from Database.cs (interpolating term/space_clause/lists).
    // standard_search, container_search, scrap_search (gated), npc_search (gated),
    // flux_search (gated + nukable filter), region_search (gated), location_search (gated),
    // teleports_to_search (needs for_id), instance_search (needs for_id -> Singular).
    // Region rows: spaceFormID is TEXT -> read String, parse u32.
    // Null displayName/label -> unwrap_or_default. Null/empty lockLevel -> LockLevel::None.
    if params.search_in_instances_only {
        out.retain(|r| matches!(r, SearchResult::Singular { .. }) || r.space().is_instanceable);
    }
    Ok(out)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `MAPPALACHIA_DB=/home/dante/Documents/mappalachia-data/data/mappalachia.db cargo test -p mappalachia-core --test db_parity`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/search.rs crates/core/src/lib.rs crates/core/tests/db_parity.rs
git commit -m "feat(core): 9 search queries with parity counts"
```

---

### Task 6: Instance resolution (8 branches)

**Files:**
- Modify: `crates/core/src/db.rs` (add `instances`, `teleporters`, `infestation_locations`, `z_range`) or new file `crates/core/src/instances.rs` + `pub mod instances;`
- Modify: `crates/core/tests/db_parity.rs` (append)

**Interfaces:**
- Consumes: `search::SearchResult`, `model::*`, `db::all_spaces`.
- Produces: `instances::instances(conn, result, space) -> Vec<Instance>` (+ teleporters/infestations/z_range) used by Task 7 tests and later `render` crate.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn resolve_nuka_instances() {
    let c = conn();
    let r = search(&c, &all_params("Nuka Cola")).unwrap();
    let g = r.iter().find(|x| matches!(x, SearchResult::Grouped(_))).unwrap().clone();
    let space = g.space().clone();
    let inst = mappalachia_core::instances::instances(&c, g, &space).unwrap();
    assert!(!inst.is_empty());
    assert!(inst.iter().all(|i| i.space.form_id == space.form_id));
}
#[test]
fn teleporters_and_zrange() {
    let c = conn();
    let spaces = mappalachia_core::db::all_spaces(&c).unwrap();
    let app = spaces.iter().find(|s| s.editor_id == "APPALACHIA").unwrap();
    let t = mappalachia_core::instances::teleporters(&c, app).unwrap();
    assert!(!t.is_empty());
    let r = search(&c, &all_params("Nuka Cola")).unwrap();
    let grouped: Vec<_> = r.into_iter().filter(|x| matches!(x, SearchResult::Grouped(_))).take(3).collect();
    let (lo, hi) = mappalachia_core::instances::z_range(&c, &grouped, app).unwrap();
    assert!(hi >= lo);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `MAPPALACHIA_DB=... cargo test -p mappalachia-core --test db_parity resolve_`
Expected: FAIL, no `instances` module.

- [ ] **Step 3: Write minimal implementation**

Port `Database.GetInstances` dispatch + 8 resolvers verbatim (`Database.cs` lines 377-853): singular (by instanceFormID), in-container (join Container, `quantity = ?` bound to spawn_weight), region (Region+RegionPoints by editorID, instance at first-subregion centroid), location (Cells → centroid instance), standard (referenceFormID+space+lock+label), scrap (join Scrap, componentQuantity bound), npc (join NPC, spawnWeight bound), flux (join Flux by color, exclude non-nukable regions via `contains_point`). Plus `GetTeleporters`, `GetInfestationLocations`, `GetZRange` (min/max `height_for_topograph`, (0,0) when empty). `teleportsToFormID` NULL/0 → None (read as Option<i64>).

- [ ] **Step 4: Run tests to verify they pass**

Run: `MAPPALACHIA_DB=... cargo test -p mappalachia-core --test db_parity`
Expected: PASS (10 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/instances.rs crates/core/src/lib.rs crates/core/tests/db_parity.rs
git commit -m "feat(core): instance resolution, teleporters, z-range"
```

---

### Task 7: Recipe JSON + legend text

**Files:**
- Create: `crates/core/src/recipe.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod recipe;`)
- Test: `crates/core/tests/recipe_json.rs`

**Interfaces:**
- Consumes: `model::*`, `enums::*`, serde.
- Produces: `recipe::{Recipe, load_recipe_file}` + `Grouped::legend_text()` used later by `cli`/`egui`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/tests/recipe_json.rs
use mappalachia_core::recipe::Recipe;
#[test]
fn recipe_round_trip() {
    let json = r#"{"Space":{"FormID":2480661,"EditorID":"APPALACHIA","DisplayName":"Appalachia","Signature":"WRLD"},
"MapSettings":{"Brightness":0.5,"LegendStyle":"Compact","Title":"test"},
"PlotSettings":{"Mode":"Standard"},
"ItemsToPlot":[{"Entity":{"FormID":610241,"EditorID":"BMO_BunkerKey03_NukaCola_01","DisplayName":"Nuka-Cola","Signature":"MISC"},
"Space":{"FormID":2480661,"EditorID":"APPALACHIA","DisplayName":"Appalachia","Signature":"WRLD"},
"Count":5,"SpawnWeight":1.0,"Label":"","LockLevel":"None","InContainer":false}]}"#;
    let r: Recipe = serde_json::from_str(json).unwrap();
    assert_eq!(r.space.editor_id, "APPALACHIA");
    assert_eq!(r.items_to_plot.len(), 1);
    let back = serde_json::to_string(&r).unwrap();
    let r2: Recipe = serde_json::from_str(&back).unwrap();
    assert_eq!(r2.items_to_plot[0].count, 5);
}
#[test]
fn legend_text_rules() {
    // count prefix "5x " when IncludeCountInLegend && count != 1; derived uses DisplayName;
    // lock suffix when lockable/in-container/locked; NPC appends "[x%]", scrap "[xN]".
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mappalachia-core --test recipe_json`
Expected: FAIL, no `recipe` module.

- [ ] **Step 3: Write minimal implementation**

```rust
// Field names PascalCase to match System.Text.Json output (no naming policy in FileIO).
// Entity: manual serde — discriminator "EntityChildType" present ONLY for derived
// (DerivedNPC/DerivedScrap/DerivedRawFlux/Region/Location values), absent for base.
// Writing "EntityChildType":"Standard" would break .NET readers -> omit it.
#[derive(Debug, Clone)]
pub struct EntitySer { /* form_id, editor_id, display_name, signature, kind */ }
// SearchResult: Grouped serializes flat (no tag); Singular adds
// "SearchResultType":"SingularSearchResult" + "InstanceFormID".
// Colors serialize as "#rrggbb" (ColorTranslator.ToHtml).
// Grouped::legend_text(include_count: bool) ports GenerateLegendText verbatim:
// "{count}x " prefix, derived->DisplayName else EditorID + ("DisplayName") + label +
// lock context, NPC " [{:.2}%]", scrap " [x{}]".
```

Recipe struct: `{ space: Space, map_settings: MapSettingsStub, plot_settings: PlotSettingsStub, items_to_plot: Vec<SearchResult> }` where Map/PlotSettings are `serde_json::Value`-backed minimal structs for now (full settings arrive with render crate; recipe must still parse/preserve them — store as `serde_json::Value` + typed accessors for `Mode`/`Brightness`/`Title`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mappalachia-core --test recipe_json && cargo test -p mappalachia-core`
Expected: PASS all.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/recipe.rs crates/core/src/lib.rs crates/core/tests/recipe_json.rs
git commit -m "feat(core): recipe JSON load/save with .NET-compatible shape"
```