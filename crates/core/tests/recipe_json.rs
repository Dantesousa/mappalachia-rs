use mappalachia_core::enums::{FluxColor, LockLevel, Signature};
use mappalachia_core::model::{Entity, EntityKind, Space};
use mappalachia_core::recipe::{html_color, load_recipe_file, parse_html_color, Recipe};
use mappalachia_core::search::{Grouped, SearchResult};

fn appalachia() -> Space {
    Space::new(
        2480661,
        "APPALACHIA",
        "Appalachia",
        true,
        true,
        0.0,
        0.0,
        0.0,
        0.0,
    )
}

fn standard_nuka(space: &Space) -> Grouped {
    Grouped {
        entity: Entity::new(
            610241,
            "BMO_BunkerKey03_NukaCola_01",
            "Nuka-Cola",
            Signature::Misc,
            EntityKind::Standard,
        ),
        space: space.clone(),
        count: 5,
        spawn_weight: 1.0,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
    }
}

#[test]
fn recipe_round_trip() {
    let json = r#"{"Space":{"FormID":2480661,"EditorID":"APPALACHIA","DisplayName":"Appalachia","Signature":"WRLD"},
"MapSettings":{"Brightness":0.5,"LegendStyle":"Compact","Title":"test"},
"PlotSettings":{"Mode":"Standard"},
"ItemsToPlot":[{"Entity":{"FormID":610241,"EditorID":"BMO_BunkerKey03_NukaCola_01","DisplayName":"Nuka-Cola","Signature":"MISC"},
"Space":{"FormID":2480661,"EditorID":"APPALACHIA","DisplayName":"Appalachia","Signature":"WRLD"},
"Count":5,"SpawnWeight":1.0,"Label":"","LockLevel":"None","InContainer":false}]}"#;
    let r: Recipe = serde_json::from_str(json).unwrap();
    assert_eq!(r.space.editor_id(), "APPALACHIA");
    assert_eq!(r.items_to_plot.len(), 1);
    let back = serde_json::to_string(&r).unwrap();
    let r2: Recipe = serde_json::from_str(&back).unwrap();
    // NOTE: brief spells this `.count`; `SearchResult` exposes it as `count()`.
    assert_eq!(r2.items_to_plot[0].count(), 5);
}

#[test]
fn legend_count_prefix_rules() {
    let space = appalachia();
    let g = standard_nuka(&space);
    // count prefix "5x " when IncludeCountInLegend && count != 1.
    assert_eq!(
        g.legend_text(true),
        "5x BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\")"
    );
    // No prefix when the flag is off.
    assert_eq!(
        g.legend_text(false),
        "BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\")"
    );
    // No prefix when count == 1 even with the flag on.
    let mut one = g.clone();
    one.count = 1;
    assert_eq!(
        one.legend_text(true),
        "BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\")"
    );
}

#[test]
fn legend_count_uses_thousands_separator() {
    let space = appalachia();
    let mut g = standard_nuka(&space);
    g.count = 1234;
    assert_eq!(
        g.legend_text(true),
        "1,234x BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\")"
    );
}

#[test]
fn legend_derived_npc_uses_display_name_and_percent() {
    let space = appalachia();
    let g = Grouped {
        entity: Entity::derived_npc("Opossum"),
        space,
        count: 4,
        spawn_weight: 0.5,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
    };
    assert_eq!(g.legend_text(true), "4x Derived Opossum Spawn [50%]");
    assert_eq!(g.legend_text(false), "Derived Opossum Spawn [50%]");
}

#[test]
fn legend_derived_npc_with_label_and_lock_renders_context() {
    let space = appalachia();
    let g = Grouped {
        entity: Entity::derived_npc("Opossum"),
        space,
        count: 1,
        spawn_weight: 0.5,
        label: "camp".to_string(),
        lock_level: LockLevel::Level1,
        in_container: false,
    };
    assert_eq!(
        g.legend_text(false),
        "Derived Opossum Spawn (camp, Level 1) [50%]"
    );
}

#[test]
fn legend_derived_scrap_appends_quantity() {
    let space = appalachia();
    let g = Grouped {
        entity: Entity::derived_scrap("Steel"),
        space,
        count: 3,
        spawn_weight: 2.0,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
    };
    assert_eq!(g.legend_text(true), "3x Derived Steel Scrap [x2]");
}

#[test]
fn legend_derived_flux_has_no_suffix() {
    let space = appalachia();
    let g = Grouped {
        entity: Entity::derived_flux(FluxColor::Violet),
        space,
        count: 1,
        spawn_weight: 1.0,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
    };
    assert_eq!(g.legend_text(false), "Derived Raw Violet Flux");
}

#[test]
fn legend_lock_suffix_rules() {
    let space = appalachia();
    // Lockable signature (CONT) with LockLevel::None still shows context.
    let mut g = standard_nuka(&space);
    g.entity.signature = Signature::Container;
    assert_eq!(
        g.legend_text(false),
        "BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\", Not Locked)"
    );
    // Non-lockable signature with a real lock level.
    let mut g2 = standard_nuka(&space);
    g2.lock_level = LockLevel::Level1;
    assert_eq!(
        g2.legend_text(false),
        "BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\", Level 1)"
    );
    // In-container forces the lock context even when None + non-lockable.
    let mut g3 = standard_nuka(&space);
    g3.in_container = true;
    assert_eq!(
        g3.legend_text(false),
        "BMO_BunkerKey03_NukaCola_01 (\"Nuka-Cola\", Not Locked)"
    );
    // Label joins the parenthesised info; empty display name is skipped.
    let mut g4 = standard_nuka(&space);
    g4.entity.display_name = String::new();
    g4.entity.signature = Signature::Misc;
    g4.label = "desk".to_string();
    g4.lock_level = LockLevel::Level2;
    assert_eq!(
        g4.legend_text(false),
        "BMO_BunkerKey03_NukaCola_01 (desk, Level 2)"
    );
}

#[test]
fn legend_region_uses_editor_id_branch() {
    let space = appalachia();
    let g = Grouped {
        entity: Entity::new(99, "TestRegion", "", Signature::Region, EntityKind::Region),
        space,
        count: 1,
        spawn_weight: 1.0,
        label: String::new(),
        lock_level: LockLevel::None,
        in_container: false,
    };
    assert_eq!(g.legend_text(false), "TestRegion");
}

#[test]
fn entity_child_type_omitted_for_standard_only() {
    let space = appalachia();
    let recipe = Recipe {
        space: space.clone(),
        map_settings: serde_json::json!({}).into(),
        plot_settings: serde_json::json!({}).into(),
        items_to_plot: vec![
            SearchResult::Grouped(standard_nuka(&space)),
            SearchResult::Grouped(Grouped {
                entity: Entity::derived_npc("Opossum"),
                space: space.clone(),
                count: 2,
                spawn_weight: 0.5,
                label: String::new(),
                lock_level: LockLevel::None,
                in_container: false,
            }),
        ],
    };
    let back = serde_json::to_string(&recipe).unwrap();
    let v: serde_json::Value = serde_json::from_str(&back).unwrap();
    let items = v["ItemsToPlot"].as_array().unwrap();
    // Writing "EntityChildType":"Standard" would break .NET readers -> omit it.
    assert!(items[0]["Entity"].get("EntityChildType").is_none());
    assert!(items[0]["Entity"].get("DisplayName").is_none());
    assert!(items[0]["Entity"].get("Signature").is_none());
    assert_eq!(items[1]["Entity"]["EntityChildType"], "DerivedNPC");
    // Grouped serializes flat (no tag).
    assert!(items[0].get("SearchResultType").is_none());
}

#[test]
fn singular_adds_tag_and_instance_form_id() {
    let space = appalachia();
    let recipe = Recipe {
        space: space.clone(),
        map_settings: serde_json::json!({}).into(),
        plot_settings: serde_json::json!({}).into(),
        items_to_plot: vec![SearchResult::Singular {
            inner: standard_nuka(&space),
            instance_form_id: 0x1234,
        }],
    };
    let back = serde_json::to_string(&recipe).unwrap();
    let v: serde_json::Value = serde_json::from_str(&back).unwrap();
    assert_eq!(
        v["ItemsToPlot"][0]["SearchResultType"],
        "SingularSearchResult"
    );
    assert_eq!(v["ItemsToPlot"][0]["InstanceFormID"], 0x1234);
    let r2: Recipe = serde_json::from_str(&back).unwrap();
    match &r2.items_to_plot[0] {
        SearchResult::Singular {
            inner,
            instance_form_id,
        } => {
            assert_eq!(*instance_form_id, 0x1234);
            assert_eq!(inner.count, 5);
        }
        SearchResult::Grouped(_) => panic!("expected singular"),
    }
    assert_eq!(r2.items_to_plot[0].count(), 1);
}

#[test]
fn settings_stubs_preserve_values_and_accessors() {
    let json = r#"{"Space":{"FormID":2480661,"EditorID":"APPALACHIA"},
"MapSettings":{"Brightness":0.5,"LegendStyle":"Compact","Title":"test"},
"PlotSettings":{"Mode":"Standard"},"ItemsToPlot":[]}"#;
    let r: Recipe = serde_json::from_str(json).unwrap();
    assert_eq!(r.map_settings.brightness(), 0.5);
    assert_eq!(r.map_settings.title(), "test");
    assert_eq!(r.plot_settings.mode(), "Standard");
    let back = serde_json::to_string(&r).unwrap();
    let v: serde_json::Value = serde_json::from_str(&back).unwrap();
    assert_eq!(v["MapSettings"]["Brightness"], 0.5);
    assert_eq!(v["MapSettings"]["Title"], "test");
    assert_eq!(v["PlotSettings"]["Mode"], "Standard");
}

#[test]
fn lock_level_accepts_string_and_number() {
    for lock in ["\"None\"", "\"Level1\"", "0", "2"] {
        let json = format!(
            "{{\"Space\":{{\"FormID\":2480661,\"EditorID\":\"APPALACHIA\"}},\
            \"MapSettings\":{{}},\"PlotSettings\":{{}},\"ItemsToPlot\":[\
            {{\"Entity\":{{\"FormID\":1,\"EditorID\":\"E\"}},\
            \"Space\":{{\"FormID\":2480661,\"EditorID\":\"APPALACHIA\"}},\
            \"Count\":1,\"SpawnWeight\":1.0,\"Label\":\"\",\
            \"LockLevel\":{lock},\"InContainer\":false}}]}}"
        );
        let r: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(r.items_to_plot.len(), 1);
    }
    let json = "{\"Space\":{\"FormID\":2480661,\"EditorID\":\"APPALACHIA\"},\
        \"MapSettings\":{},\"PlotSettings\":{},\"ItemsToPlot\":[\
        {\"Entity\":{\"FormID\":1,\"EditorID\":\"E\"},\
        \"Space\":{\"FormID\":2480661,\"EditorID\":\"APPALACHIA\"},\
        \"Count\":1,\"SpawnWeight\":1.0,\"Label\":\"\",\
        \"LockLevel\":\"Level3\",\"InContainer\":false}]}";
    let r: Recipe = serde_json::from_str(json).unwrap();
    assert_eq!(r.items_to_plot[0].count(), 1);
    match &r.items_to_plot[0] {
        SearchResult::Grouped(g) => assert_eq!(g.lock_level, LockLevel::Level3),
        SearchResult::Singular { .. } => panic!("expected grouped"),
    }
}

#[test]
fn html_colors_round_trip_as_rrggbb() {
    assert_eq!(html_color(255, 0, 128), "#ff0080");
    assert_eq!(html_color(0, 0, 0), "#000000");
    assert_eq!(parse_html_color("#ff0080"), Some((255, 0, 128)));
    assert_eq!(parse_html_color("#FF0080"), Some((255, 0, 128)));
    assert_eq!(parse_html_color("not-a-color"), None);
    assert_eq!(parse_html_color("#fff"), None);
}

#[test]
fn load_recipe_file_reads_from_disk() {
    let json = r#"{"Space":{"FormID":2480661,"EditorID":"APPALACHIA","DisplayName":"Appalachia","Signature":"WRLD"},
"MapSettings":{"Brightness":0.5,"LegendStyle":"Compact","Title":"test"},
"PlotSettings":{"Mode":"Standard"},"ItemsToPlot":[]}"#;
    let path = std::env::temp_dir().join("mappalachia-task7-recipe-test.json");
    std::fs::write(&path, json).unwrap();
    let r = load_recipe_file(&path).unwrap();
    assert_eq!(r.space.editor_id(), "APPALACHIA");
    assert_eq!(r.plot_settings.mode(), "Standard");
    std::fs::remove_file(&path).ok();
    assert!(load_recipe_file(&path).is_err());
}
