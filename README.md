# mappalachia-rs

[![release](https://github.com/Dantesousa/mappalachia-rs/actions/workflows/release.yml/badge.svg)](https://github.com/Dantesousa/mappalachia-rs/actions/workflows/release.yml)
[![latest release](https://img.shields.io/github/v/release/Dantesousa/mappalachia-rs)](https://github.com/Dantesousa/mappalachia-rs/releases/latest)
[![license](https://img.shields.io/github/license/Dantesousa/mappalachia-rs)](LICENSE.md)

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

## Spotlight (opt-in hires tiles)

Spotlight swaps the drawn map background for true top-down game tiles —
off by default. The tile set is a ~3 GB download (two 7z volumes, game
version 2.0.5.2) installed into `img/spotlight/` next to the binary.

In the GUI, open the map settings panel → **Spotlight**: press
**Download** (progress bars show download → extract → verify; **Cancel**
aborts), then tick the **Spotlight** checkbox. Enabling flips the
background to Render (restored on disable); **Size** (1–16 tiles, capped
per space) and **Location** with **Reset to map center** tune the view.
Without installed tiles the toggle stays off with a status hint.

From source / dev: instead of downloading, symlink a tile tree into the
asset root — `ln -s /path/to/spotlight <assets>/img/spotlight` — and point
`MAPPALACHIA_ASSETS` at that root.

## Releases

Tagged pushes (`v*`) build Windows (exe) and Linux (AppImage) binaries via GitHub Actions.

## License

GPL-3.0, like the original. See `LICENSE.md`.
Game data/assets belong to Bethesda; see the original repo's disclaimer.
