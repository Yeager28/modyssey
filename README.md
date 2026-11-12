# Modyssey

**Multi-game modpack installer for Windows** — installs Wabbajack-format
modlist archives (`.wabbajack`), focused on Dark Souls II: Scholar of the
First Sin and extensible to more games.

This is a rebranded fork of [CLF3](https://github.com/SulfurNitride/CLF3)
(MIT) — see [ATTRIBUTION.md](ATTRIBUTION.md). Original code, MIT-licensed;
the owner may distribute derived builds under any terms MIT permits.

## ⚠️ Hard rule for all contributors

**Never paste, port, or transliterate Wabbajack C# source code (or any other
GPL-licensed implementation) into this repository.** This codebase's MIT
license is only valid because it is an independent implementation of the
documented `.wabbajack` format. Reference behavioral/format documentation
(the Wabbajack wiki, format specs) only — never the source of
`wabbajack-tools/wabbajack`, `modde-rs`, or any other GPL project. If you
find yourself opening one of those codebases "just to check", stop and restate
the behavior from documentation instead.

## Features

- **Modlist installation**: parses `.wabbajack` modlists, downloads archives
  (Nexus, HTTP, Mega, MediaFire, Google Drive, LoversLab, Steam game files,
  manual sources), hash-verifies everything, and applies install directives
  (FromArchive, PatchedFromArchive with OctoDiff deltas, Inline/RemappedInline
  files, TransformedTexture with GPU DDS re-encode, CreateBSA).
- **Game detection**: registry, Steam library manifests, and Heroic/GOG
  discovery for known games (Dark Souls II SOTFS included).
- **GUI**: egui modlist browser + collections UI (`modyssey browser`).
- **CLI**: full install pipeline, modlist info, updates, Nexus collection
  planning, MOMW (Modding-OpenMW) installs (Linux-only tooling), Steam login,
  game file index.

## Build

```powershell
cargo build --release
```

Binary: `target/release/modyssey.exe`. Rust stable (MSVC toolchain) with
Visual Studio Build Tools (MSVC + Windows SDK) required.

## Usage

```text
modyssey                       # GUI modlist browser
modyssey info LIST.wabbajack   # inspect a modlist
modyssey install LIST.wabbajack --downloads DIR --output DIR [--game DIR]
modyssey help                  # everything else
```

## License

MIT — see [LICENSE](LICENSE). Copyright © SulfurNitride (CLF3 upstream) and
Bonkfyre (this fork).
