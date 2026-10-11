use mappalachia_core::db;
use mappalachia_core::{
    enums::{LockLevel, Signature},
    model::EntityKind,
    search::{search, SearchParams, SearchResult},
};

fn conn() -> Option<rusqlite::Connection> {
    match std::env::var("MAPPALACHIA_DB") {
        Ok(p) if std::path::Path::new(&p).exists() => Some(db::open_readonly(&p).unwrap()),
        _ => {
            eprintln!("SKIP: set MAPPALACHIA_DB to the release mappalachia.db");
            None
        }
    }
}

#[test]
fn meta_and_spaces() {
    let Some(c) = conn() else { return };
    assert_eq!(db::game_version(&c).unwrap(), "1.7.26.13");
    let spaces = db::all_spaces(&c).unwrap();
    assert_eq!(spaces.len(), 205);
    let app = spaces
        .iter()
        .find(|s| s.editor_id() == "APPALACHIA")
        .unwrap();
    assert_eq!(app.form_id(), 2480661);
}

#[test]
fn markers_and_globals() {
    let Some(c) = conn() else { return };
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM MapMarker", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        458
    );
    assert_eq!(db::blast_radius(&c).unwrap(), 20460.0);
    assert_eq!(db::infestation_radius(&c).unwrap(), 20000.0);
}

fn all_params(term: &str) -> SearchParams {
    SearchParams {
        search_term: term.into(),
        space_form_id: None,
        advanced: term == "00094FC1",
        search_in_instances_only: false,
        signatures: Signature::recommended_default(),
        lock_levels: LockLevel::all(),
    }
}
#[test]
fn search_nuka_cola() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("Nuka Cola")).unwrap();
    // Verbatim C# Search with recommended signatures + all locks:
    // 339 standard + 8 container + 1 location ("Nuka%Cola").
    // (The brief's 586 is the raw standard SQL without the signature IN
    // filter — reproduced via sqlite3 with all 35 signatures.)
    assert_eq!(r.len(), 348);
}
#[test]
fn search_scrap_steel() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("Steel")).unwrap();
    let total: i64 = r
        .iter()
        .filter(|x| matches!(x.entity_kind(), EntityKind::DerivedScrap))
        .map(|x| x.count())
        .sum();
    assert_eq!(total, 21795);
}
#[test]
fn search_npc_scorched() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("Scorched")).unwrap();
    let total: i64 = r
        .iter()
        .filter(|x| matches!(x.entity_kind(), EntityKind::DerivedNpc))
        .map(|x| x.count())
        .sum();
    assert_eq!(total, 1731);
}
#[test]
fn search_flux_crimson() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("Crimson")).unwrap();
    let total: i64 = r
        .iter()
        .filter(|x| matches!(x.entity_kind(), EntityKind::DerivedFlux(_)))
        .map(|x| x.count())
        .sum();
    // Verbatim FluxSearch keeps only nukable (Appalachia) rows: 5677.
    // (The brief's 6497 is the raw flux SQL summed over ALL spaces —
    // reproduced via sqlite3; 820 placements live in other spaces.)
    assert_eq!(total, 5677);
}
#[test]
fn search_form_id_instance() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("00094FC1")).unwrap(); // = entity 610241, advanced on
    assert!(r
        .iter()
        .any(|x| matches!(x, SearchResult::Singular { .. }) || x.form_id() == 610241));
}
#[test]
fn search_region_and_location() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("Nuke")).unwrap();
    assert!(r.iter().any(|x| x.editor_id() == "NukeRegionA"));
    let r = search(&c, &all_params("Flatwoods")).unwrap();
    assert!(r
        .iter()
        .any(|x| x.editor_id() == "LocForestFlatwoodsLocation"));
}

#[test]
fn resolve_nuka_instances() {
    let Some(c) = conn() else { return };
    let r = search(&c, &all_params("Nuka Cola")).unwrap();
    let g = r
        .iter()
        .find(|x| matches!(x, SearchResult::Grouped(_)))
        .unwrap()
        .clone();
    let space = g.space().clone();
    let inst = mappalachia_core::instances::instances(&c, g, &space).unwrap();
    assert!(!inst.is_empty());
    assert!(inst.iter().all(|i| i.space.form_id() == space.form_id()));
}
#[test]
fn teleporters_and_zrange() {
    let Some(c) = conn() else { return };
    let spaces = mappalachia_core::db::all_spaces(&c).unwrap();
    let app = spaces
        .iter()
        .find(|s| s.editor_id() == "APPALACHIA")
        .unwrap();
    let t = mappalachia_core::instances::teleporters(&c, app).unwrap();
    assert!(!t.is_empty());
    let r = search(&c, &all_params("Nuka Cola")).unwrap();
    let grouped: Vec<_> = r
        .into_iter()
        .filter(|x| matches!(x, SearchResult::Grouped(_)))
        .take(3)
        .collect();
    let (lo, hi) = mappalachia_core::instances::z_range(&c, &grouped, app).unwrap();
    assert!(hi >= lo);
}
