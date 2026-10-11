# mappalachia-rs

A Rust port of [Mappalachia](https://github.com/AHeroicLlama/Mappalachia) — the mapping tool for Fallout 76.
Generates and exports maps of entities in the Fallout 76 game world.

## Crates

| Crate | What |
|---|---|
| `core` | Data model, readonly SQLite search (same SQL as the original), geometry, recipe JSON |
| `render` | Layered 4096px map pipeline (background → plots → legend), Standard mode |
| `cli` | `recipe.json` in → `.png`/`.jpg` out (headless) |
| `egui` | Native GUI: search, settings, preview, export |

## Run

You need the release data (not shipped here): `mappalachia.db` + `img/` + `font/`
from a Mappalachia release zip.

```bash
export MAPPALACHIA_DB=/path/to/mappalachia.db
export MAPPALACHIA_ASSETS=/path/to/release-root   # contains img/, font/, data/
cargo run -p mappalachia-cli -- path/to/recipe.json
cargo run -p mappalachia-egui
```

## Releases

Tagged pushes (`v*`) build Windows (exe) and Linux (AppImage) binaries via GitHub Actions.

## License

GPL-3.0, like the original. See `LICENSE.md`.
Game data/assets belong to Bethesda; see the original repo's disclaimer.
