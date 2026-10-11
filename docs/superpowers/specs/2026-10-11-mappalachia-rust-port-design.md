# Mappalachia Rust Port — Design Spec (2026-10-11)

## 1. Reverse-engineering findings (source: AHeroicLlama/Mappalachia @ 2.0.5.2)

- 6 projects, ~14k lines C# (.NET 10): `Library` (1.9k), `Mappalachia` WinForms GUI (10.3k),
  `Preprocessor` (1.6k), `BackgroundRenderer`, `MapIconProcessor`, `Tests`.
- Deps: `Microsoft.Data.Sqlite`, `Svg.NET`, `KGySoft.CoreLibraries` (imaging), WinForms.
- Pipeline: FO76Edit Pascal scripts → raw CSVs → Preprocessor builds `mappalachia.db`
  (SQLite STRICT tables) → GUI searches DB → `Map.Draw` renders 4096×4096 → PNG/JPG/SVG export.
- CLI mode exists: `Mappalachia.exe recipe.json` renders headless (parity oracle for us).
- Real DB (release 2.0.5.2, game 1.7.26.13): 15 tables. `Position` 4.49M rows,
  `Position_PreGrouped` 208k, `Entity` 51k, `Space` 205, `NPC` 14k, `Region` 316,
  `RegionPoints` 3.3k, `Location` 672, `Cell` 12.5k, `Scrap` 1273, `Container` 704,
  `Flux` 73, `MapMarker` 458, `Global` 2, `Meta` 1.
- Search = 9 query fns in `Database.cs`: Standard, Container, Scrap, NPC, Flux, Region,
  Location, TeleportsTo, Instance. Grouped results via `Position_PreGrouped`,
  singular via `Position` (instanceFormID PK).
- Model (`Library/`): `Entity` (+`DerivedNPC/Scrap/RawFlux`, `Region`, `Location`),
  `Space` (WRLD/CELL, center/maxRange/northAngle), `Coord`, `Instance`/`BaseInstance`
  (lockLevel, spawnWeight, primitiveShape, teleportsTo), `Cluster`, `MapMarker`,
  `SpotlightTile`, `GeometryHelper` (monotone-chain convex hull, segment intersection).
- Plot modes (`PlotSettings.Mode`): Standard, Topographic, Heatmap, Cluster.
- Render layers (`Map.cs`, 1326 lines): background JPG (wrld/cell) → regions/volumes →
  plots → legend (Extended/Compact/Hidden) + compass + coord grid + title/watermark.
- Assets (release zip): `data/mappalachia.db` (481MB), `img/{wrld,cell,mapmarker,icon}`,
  `font/futura_condensed_bold.otf`. Recipes: JSON (`Recipe.cs`), forward-compatible target.

## 2. Scope

IN: `core` lib (model + readonly DB search + geometry + recipes), `render` lib
(4 plot modes, 4096px, layered pipeline), `cli` (recipe→image, parity harness),
`egui` GUI via eframe (search, space select, plot settings, preview, export).
Test data: `~/Documents/mappalachia-data/data/mappalachia.db` (read-only, never copied
into repo).

OUT: Preprocessor, BackgroundRenderer, MapIconProcessor (need FO76 + Windows/FO76Edit),
spotlight tiles, in-game map mod, recipe editor UI (v1: load/run only).

## 3. Architecture

```text
mappalachia-rs/ (cargo workspace, resolver 2, edition 2021)
├── crates/core    # model, db (rusqlite readonly), geometry, recipe (serde_json)
├── crates/render  # tiny-skia + resvg + image; layered Map::draw
├── crates/cli     # recipe.json → .png/.jpg (headless, exit code on error)
└── crates/egui    # eframe: search panel, preview, settings, export
```

Dependency direction: `egui → render → core`, `cli → render → core`. No cycles.

Key deps: `rusqlite` (bundled), `serde/serde_json`, `tiny-skia`, `resvg`,
`image`, `eframe` (= egui), `anyhow`, `clap`.

## 4. Data flow

`open DB (mode=ro)` → `search()` (same SQL as Database.cs, 9 fns) →
`GroupedSearchResult` → resolve `Instance`s via `Position` → optional
cluster/heatmap aggregation → `render::draw(items, space, map/plot settings)`
layers in order → encode PNG/JPG/SVG to `Saved_Maps/`-equivalent out dir.

## 5. C# → Rust mapping

| C# | Rust |
|---|---|
| `System.Drawing` bitmap/pen/brush | `tiny-skia` Pixmap/Paint |
| `Svg.NET` | `resvg` → rasterize to tiny-skia |
| WinForms (`FormMain` + 10 dialogs) | single `eframe` app, docked panels |
| `Microsoft.Data.Sqlite` async | `rusqlite` sync (search is fast indexed SQL; no async needed v1) |
| `System.Text.Json` polymorphic recipes | `serde` tagged enum, same JSON shape |
| `KGySoft` imaging utils | `image` crate |

Coordinate spaces and the 4096 constants (`MapImageResolution`, `CellSize`,
`SpotlightTileSize`) are ported verbatim from `Library/Common.cs`.

## 6. Parity strategy

1. Same-SQL search: identical row counts vs .NET for a fixed battery of
   search terms run against the release DB (recorded in `tests/parity_search.rs`).
2. CLI render of reference recipes compared against .NET output by dHash
   (hamming distance ≤ 5; exact pixels not required v1: font rasterization
   differs on Linux), implemented with the `image` crate in tests.
3. `core` unit tests: geometry (hull/intersection/centroid), FormID hex helpers,
   recipe round-trip.

## 7. Milestones

1. `core`: model + open DB + 9 searches + tests (no rendering).
2. `render`: background + Standard plots + legend/compass/grid + PNG out.
3. `cli`: recipe in → image out; parity battery green.
4. `render`: Topographic, Heatmap, Cluster modes.
5. `egui`: search + preview + settings + export.
