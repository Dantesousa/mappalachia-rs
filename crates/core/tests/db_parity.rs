use mappalachia_core::db;

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
