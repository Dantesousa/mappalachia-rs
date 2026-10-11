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

One download per platform — game data ships inside the bundle, no setup:

- **Windows:** download `mappalachia-rs-windows.zip` from the
  [latest release](https://github.com/Dantesousa/mappalachia-rs/releases/latest),
  extract it, and run `mappalachia-egui.exe` (GUI) or `mappalachia-cli.exe`
  (headless). The zip contains both binaries plus `data/`, `img/`, `font/`.
- **Linux:** download `mappalachia-rs-linux.tar.gz` from the
  [latest release](https://github.com/Dantesousa/mappalachia-rs/releases/latest),
  extract it (`tar -xzf mappalachia-rs-linux.tar.gz`), and run the AppImage
  (`./mappalachia-rs-x86_64.AppImage`) or the CLI
  (`./mappalachia-cli-linux-x86_64`). The tarball contains both binaries
  plus `data/`, `img/`, `font/` — keep them next to the binary you run.

From source you still need game data (not shipped in git): download
`mappalachia-rs-data.zip` from the
[latest release](https://github.com/Dantesousa/mappalachia-rs/releases/latest)
(`data/mappalachia.db` + `img/` + `font/`) and extract it next to the binaries —
or point `MAPPALACHIA_DB` / `MAPPALACHIA_ASSETS` at any Mappalachia data tree.

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
