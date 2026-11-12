# Modyssey

**Multi-game modpack installer for Windows** — installs Wabbajack-format
modlist archives (`.wabbajack`), focused on Dark Souls II: Scholar of the
First Sin and extensible to more games.

This is a fork of [CLF3](https://github.com/SulfurNitride/CLF3) by
SulfurNitride (MIT — see [LICENSE-MIT-CLF3](LICENSE-MIT-CLF3) and
[ATTRIBUTION.md](ATTRIBUTION.md)) rebranded as **Modyssey**, with a Windows
port and a Dark Souls II game entry on top. The project as a whole is
**GPL-3.0-or-later**.

## Wabbajack reference policy

[Wabbajack](https://github.com/wabbajack-tools/wabbajack) (GPLv3) is the
reference implementation whose behavior Modyssey ports (the compiler, most
importantly). **Reading and porting Wabbajack C# source is permitted here**
because this project is GPL-3.0-or-later — derivatives of a GPL work stay
GPL. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full policy, including
the one hard rule that stays: **never automate or scrape Nexus free-tier
downloads** (browser-open + local-folder watch only).

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

GPL-3.0-or-later — see [LICENSE](LICENSE). The engine seed is MIT-licensed
CLF3 code (© SulfurNitride), preserved verbatim in
[LICENSE-MIT-CLF3](LICENSE-MIT-CLF3); see [ATTRIBUTION.md](ATTRIBUTION.md)
for the full lineage and [MODLIST-POLICY.md](MODLIST-POLICY.md) for the
modlist-data policy.
